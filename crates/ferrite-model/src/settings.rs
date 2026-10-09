//! What a person configures in the app's Settings screen, and how it becomes
//! the same [`ModelConfig`] the environment variables always produced.
//!
//! A packaged app has no shell to export `FERRITE_MODEL_SMALL` from, so the
//! Settings screen needs somewhere to keep a provider choice and two model
//! names. This module is that somewhere, and it is deliberately *not* a second
//! configuration system:
//!
//! * **Non-secret choices** (which provider, which models, a local server's
//!   address) live in one small JSON file, [`ModelSettings`]. It is translated
//!   into the environment-variable vocabulary by [`ModelSettings::to_env`] and
//!   layered *under* the real environment by [`LayeredEnv`], so
//!   [`ModelConfig::load`] and every provider constructor run unchanged. An
//!   exported variable still wins (a CI runner, a one-off shell override), and
//!   [`ModelSettings::env_overrides`] says when that is happening so the
//!   Settings screen can tell the person why their choice is not in effect.
//! * **API keys are never in that file.** They go to the OS keyring through
//!   [`SecretVault`](crate::secret::SecretVault), under the account names the
//!   providers already read (`OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY`).
//!   §10.1's rule — environment or keyring "and nowhere else" — is unchanged.
//! * **No model name appears in Rust source** (§10.2). The names in the file
//!   are the person's own choice from a list the provider itself served
//!   ([`list_models`]); an unset one is a clear "choose a model", never a
//!   default that a provider may since have retired.
//!
//! Nothing here touches the network except [`list_models`], which a person
//! starts by pressing a button, and nothing here is reachable from a test
//! without an injected [`EnvSource`] and [`SecretStore`].

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backends::{
    ANTHROPIC_API_KEY_VAR, AnthropicProvider, GEMINI_API_KEY_VAR, GeminiProvider,
    OLLAMA_API_KEY_VAR, OPENAI_API_KEY_VAR, OllamaProvider, OpenAiProvider,
};
use crate::config::{
    DEFAULT_ANTHROPIC_BASE_URL, DEFAULT_GEMINI_BASE_URL, DEFAULT_MAX_RESPONSE_BYTES,
    DEFAULT_OLLAMA_BASE_URL, DEFAULT_OPENAI_BASE_URL, EnvSource, LOCAL_OLLAMA_BASE_URL, MapEnv,
    ModelConfig, is_local_url,
};
use crate::decorators::Trace;
use crate::error::ModelError;
use crate::provider::{ModelProvider, ModelTier, ProviderId};
use crate::secret::{KEYRING_SERVICE, SecretStore, Token, resolve};

/// The file's name inside the data directory.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// How long [`list_models`] waits. A person pressed a button and is looking at
/// a spinner: long enough for a slow cloud endpoint, short enough that a dead
/// one is reported rather than waited on.
pub const LIST_MODELS_TIMEOUT: Duration = Duration::from_secs(20);

/// The longest model name accepted. Real tags are far shorter; the bound
/// exists so a pasted blob is refused instead of stored.
const MAX_MODEL_NAME_LEN: usize = 200;

/// Which service answers the agent's model calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderChoice {
    /// Ollama's hosted service. Needs an API key.
    OllamaCloud,
    /// An Ollama server on this computer. Takes no key, and is sent none.
    OllamaLocal,
    /// Google's Gemini API. Needs an API key.
    Gemini,
    /// Anthropic's Claude models. Needs an API key.
    Anthropic,
    /// OpenAI, or any server that speaks its chat-completions format
    /// (OpenRouter, Groq, vLLM, LM Studio, ...). Needs a key unless the
    /// server is on this computer.
    OpenAiCompatible,
}

impl ProviderChoice {
    /// Every choice, in the order the Settings screen lists them.
    pub const ALL: [Self; 5] = [
        Self::OllamaCloud,
        Self::OllamaLocal,
        Self::Gemini,
        Self::Anthropic,
        Self::OpenAiCompatible,
    ];

    /// The name shown to a person.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OllamaCloud => "Ollama Cloud",
            Self::OllamaLocal => "Ollama (this computer)",
            Self::Gemini => "Google Gemini",
            Self::Anthropic => "Anthropic Claude",
            Self::OpenAiCompatible => "OpenAI or compatible",
        }
    }

    /// One plain sentence on what picking this means.
    #[must_use]
    pub const fn blurb(self) -> &'static str {
        match self {
            Self::OllamaCloud => "Models hosted by Ollama. Needs an Ollama API key.",
            Self::OllamaLocal => {
                "Models running on your own machine through Ollama. No key, and nothing leaves your computer."
            }
            Self::Gemini => "Models hosted by Google. Needs a Gemini API key.",
            Self::Anthropic => "Claude models hosted by Anthropic. Needs an Anthropic API key.",
            Self::OpenAiCompatible => {
                "OpenAI, or any server with the same API (OpenRouter, Groq, vLLM, LM Studio). \
                 Needs a key unless the server is on this computer."
            }
        }
    }

    /// The environment variable (and keyring account) holding this choice's
    /// key, or `None` for the choice that takes none.
    #[must_use]
    pub const fn key_var(self) -> Option<&'static str> {
        match self {
            Self::OllamaCloud => Some(OLLAMA_API_KEY_VAR),
            Self::OllamaLocal => None,
            Self::Gemini => Some(GEMINI_API_KEY_VAR),
            Self::Anthropic => Some(ANTHROPIC_API_KEY_VAR),
            Self::OpenAiCompatible => Some(OPENAI_API_KEY_VAR),
        }
    }

    /// Which backend answers.
    #[must_use]
    pub const fn provider_id(self) -> ProviderId {
        match self {
            Self::OllamaCloud | Self::OllamaLocal => ProviderId::Ollama,
            Self::Gemini => ProviderId::Gemini,
            Self::Anthropic => ProviderId::Anthropic,
            Self::OpenAiCompatible => ProviderId::OpenAi,
        }
    }
}

impl std::fmt::Display for ProviderChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The two models a provider is configured with: the fast one that predicts
/// what a task needs (`FERRITE_MODEL_SMALL`) and the one that drives the
/// browser (`FERRITE_MODEL_MAIN`). Empty means "not chosen yet".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPair {
    /// The fast model's name.
    pub small: String,
    /// The agent model's name.
    pub main: String,
}

/// Where a provider's key is coming from right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// This choice takes no key.
    NotNeeded,
    /// An exported environment variable (it wins over the keyring).
    Environment(&'static str),
    /// The OS keyring.
    Keyring,
    /// Nowhere: a key must be entered.
    Missing,
}

/// The persisted, non-secret model choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSettings {
    /// File format version, for the day it has to change.
    pub version: u32,
    /// The chosen provider; `None` until a person chooses one, in which case
    /// the environment and keyring alone decide (see [`connect`]).
    pub provider: Option<ProviderChoice>,
    /// Models chosen for [`ProviderChoice::OllamaCloud`].
    pub ollama_cloud: ModelPair,
    /// Models chosen for [`ProviderChoice::OllamaLocal`].
    pub ollama_local: ModelPair,
    /// Models chosen for [`ProviderChoice::Gemini`].
    pub gemini: ModelPair,
    /// Models chosen for [`ProviderChoice::Anthropic`].
    pub anthropic: ModelPair,
    /// Models chosen for [`ProviderChoice::OpenAiCompatible`].
    pub openai: ModelPair,
    /// The local Ollama server's address.
    pub ollama_local_url: String,
    /// The OpenAI-compatible server's API root, version included
    /// (`https://api.openai.com/v1`, `http://localhost:1234/v1`).
    pub openai_url: String,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            version: 1,
            provider: None,
            ollama_cloud: ModelPair::default(),
            ollama_local: ModelPair::default(),
            gemini: ModelPair::default(),
            anthropic: ModelPair::default(),
            openai: ModelPair::default(),
            ollama_local_url: LOCAL_OLLAMA_BASE_URL.to_string(),
            openai_url: DEFAULT_OPENAI_BASE_URL.to_string(),
        }
    }
}

impl ModelSettings {
    /// The models stored for `choice`.
    #[must_use]
    pub fn pair(&self, choice: ProviderChoice) -> &ModelPair {
        match choice {
            ProviderChoice::OllamaCloud => &self.ollama_cloud,
            ProviderChoice::OllamaLocal => &self.ollama_local,
            ProviderChoice::Gemini => &self.gemini,
            ProviderChoice::Anthropic => &self.anthropic,
            ProviderChoice::OpenAiCompatible => &self.openai,
        }
    }

    /// Mutable access to the models stored for `choice`.
    pub fn pair_mut(&mut self, choice: ProviderChoice) -> &mut ModelPair {
        match choice {
            ProviderChoice::OllamaCloud => &mut self.ollama_cloud,
            ProviderChoice::OllamaLocal => &mut self.ollama_local,
            ProviderChoice::Gemini => &mut self.gemini,
            ProviderChoice::Anthropic => &mut self.anthropic,
            ProviderChoice::OpenAiCompatible => &mut self.openai,
        }
    }

    /// The server address for `choice`: an exported `FERRITE_*_BASE_URL`
    /// first (the same override the environment always offered), else this
    /// file's value for the local server, else the provider's own endpoint.
    #[must_use]
    pub fn base_url(&self, choice: ProviderChoice, env: &dyn EnvSource) -> String {
        match choice {
            ProviderChoice::OllamaCloud => env
                .get("FERRITE_OLLAMA_BASE_URL")
                .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_string()),
            ProviderChoice::OllamaLocal => env
                .get("FERRITE_OLLAMA_BASE_URL")
                .unwrap_or_else(|| self.local_url()),
            ProviderChoice::Gemini => env
                .get("FERRITE_GEMINI_BASE_URL")
                .unwrap_or_else(|| DEFAULT_GEMINI_BASE_URL.to_string()),
            ProviderChoice::Anthropic => env
                .get("FERRITE_ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| DEFAULT_ANTHROPIC_BASE_URL.to_string()),
            ProviderChoice::OpenAiCompatible => env
                .get("FERRITE_OPENAI_BASE_URL")
                .unwrap_or_else(|| self.openai_root()),
        }
    }

    /// Whether `choice` needs a key with these settings: every provider that
    /// takes one does, except an OpenAI-compatible server on this computer.
    #[must_use]
    pub fn key_required(&self, choice: ProviderChoice, env: &dyn EnvSource) -> bool {
        choice.key_var().is_some()
            && !(choice == ProviderChoice::OpenAiCompatible
                && is_local_url(&self.base_url(choice, env)))
    }

    fn openai_root(&self) -> String {
        let url = self.openai_url.trim().trim_end_matches('/');
        if url.is_empty() {
            DEFAULT_OPENAI_BASE_URL.to_string()
        } else {
            url.to_string()
        }
    }

    fn local_url(&self) -> String {
        let url = self.ollama_local_url.trim();
        if url.is_empty() {
            LOCAL_OLLAMA_BASE_URL.to_string()
        } else {
            url.to_string()
        }
    }

    /// Why the active provider's settings cannot be used yet, as a sentence
    /// for a person — or `None` when they can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let choice = self.provider?;
        let pair = self.pair(choice);
        for (what, name) in [("fast", &pair.small), ("agent", &pair.main)] {
            if name.trim().is_empty() {
                return Some(format!("Choose a model for the {what} role."));
            }
            if let Some(why) = bad_model_name(name) {
                return Some(format!("The {what} model name {why}."));
            }
        }
        if choice == ProviderChoice::OpenAiCompatible && !is_http_url(&self.openai_root()) {
            return Some(
                "The server address must be a web address like https://api.openai.com/v1.".into(),
            );
        }
        if choice == ProviderChoice::OllamaLocal {
            let url = self.local_url();
            if !is_http_url(&url) {
                return Some(
                    "The server address must be a web address like http://localhost:11434.".into(),
                );
            }
            if !is_local_url(&url) {
                return Some(
                    "A local server must be on this computer (localhost or 127.0.0.1). \
                     Use Ollama Cloud for a remote one."
                        .into(),
                );
            }
        }
        None
    }

    /// These settings in the environment-variable vocabulary [`ModelConfig`]
    /// reads. Only the active provider contributes, and only what is set.
    #[must_use]
    pub fn to_env(&self) -> MapEnv {
        let Some(choice) = self.provider else {
            return MapEnv::new();
        };
        let pair = self.pair(choice);
        let mut env = MapEnv::new()
            .with(ModelTier::Small.env_var(), pair.small.trim())
            .with(ModelTier::Main.env_var(), pair.main.trim());
        match choice {
            ProviderChoice::OllamaCloud => {
                env = env.with("FERRITE_OLLAMA_BASE_URL", DEFAULT_OLLAMA_BASE_URL);
            }
            ProviderChoice::OllamaLocal => {
                env = env.with("FERRITE_OLLAMA_BASE_URL", self.local_url());
            }
            ProviderChoice::Gemini => {
                env = env.with("FERRITE_GEMINI_BASE_URL", DEFAULT_GEMINI_BASE_URL);
            }
            ProviderChoice::Anthropic => {
                env = env.with("FERRITE_ANTHROPIC_BASE_URL", DEFAULT_ANTHROPIC_BASE_URL);
            }
            ProviderChoice::OpenAiCompatible => {
                env = env.with("FERRITE_OPENAI_BASE_URL", self.openai_root());
            }
        }
        env
    }

    /// The variables this file would set that `env` already sets, and which
    /// therefore win: the reason a saved choice might not be in effect.
    #[must_use]
    pub fn env_overrides(&self, env: &dyn EnvSource) -> Vec<&'static str> {
        let own = self.to_env();
        [
            ModelTier::Small.env_var(),
            ModelTier::Main.env_var(),
            "FERRITE_OLLAMA_BASE_URL",
            "FERRITE_GEMINI_BASE_URL",
            "FERRITE_ANTHROPIC_BASE_URL",
            "FERRITE_OPENAI_BASE_URL",
        ]
        .into_iter()
        .filter(|var| own.get(var).is_some() && env.get(var).is_some())
        .collect()
    }

    /// Reads the file. A missing file is the default settings (a first run),
    /// not an error; a file that is not these settings is.
    ///
    /// # Errors
    ///
    /// [`ModelError::Config`] when the file exists but cannot be read or is
    /// not valid settings JSON.
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                ModelError::Config(format!(
                    "{} is not valid settings ({e}); fix or delete it",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ModelError::Config(format!(
                "cannot read {}: {e}",
                path.display()
            ))),
        }
    }

    /// Writes the file, replacing it whole: to a sibling temporary file first,
    /// then renamed over the real one, so a crash mid-write leaves the old
    /// settings rather than half of the new.
    ///
    /// # Errors
    ///
    /// [`ModelError::Config`] when the directory or file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), ModelError> {
        let fail = |what: &str, e: std::io::Error| {
            ModelError::Config(format!("cannot {what} {}: {e}", path.display()))
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail("create the folder for", e))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| ModelError::Config(format!("settings do not serialize: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| fail("write", e))?;
        std::fs::rename(&tmp, path).map_err(|e| fail("replace", e))
    }
}

fn bad_model_name(name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.len() > MAX_MODEL_NAME_LEN {
        Some("is too long")
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Some("cannot contain spaces")
    } else {
        None
    }
}

fn is_http_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

/// `primary` first, then `fallback`: how the real environment (CI, a shell
/// override) is layered over the values the Settings screen saved.
#[derive(Debug)]
pub struct LayeredEnv<'a> {
    primary: &'a dyn EnvSource,
    fallback: &'a dyn EnvSource,
}

impl<'a> LayeredEnv<'a> {
    /// Layers `primary` over `fallback`.
    #[must_use]
    pub fn new(primary: &'a dyn EnvSource, fallback: &'a dyn EnvSource) -> Self {
        Self { primary, fallback }
    }
}

impl EnvSource for LayeredEnv<'_> {
    fn get(&self, key: &str) -> Option<String> {
        self.primary.get(key).or_else(|| self.fallback.get(key))
    }
}

/// Where `choice`'s key is coming from, without reading its value.
#[must_use]
pub fn key_source(
    choice: ProviderChoice,
    env: &dyn EnvSource,
    store: &dyn SecretStore,
) -> KeySource {
    let Some(var) = choice.key_var() else {
        return KeySource::NotNeeded;
    };
    if env.get(var).is_some() {
        KeySource::Environment(var)
    } else if store.get(KEYRING_SERVICE, var).is_some() {
        KeySource::Keyring
    } else {
        KeySource::Missing
    }
}

/// The key to use for `choice` right now: what the person just typed if they
/// typed one (so models can be listed before anything is saved), else the
/// environment, else the keyring. `None` for a choice that takes no key, or
/// when there is none.
#[must_use]
pub fn resolve_key(
    choice: ProviderChoice,
    typed: &str,
    env: &dyn EnvSource,
    store: &dyn SecretStore,
) -> Option<Token> {
    let var = choice.key_var()?;
    let typed = typed.trim();
    if !typed.is_empty() {
        return Some(Token::new(typed));
    }
    resolve(env, store, var).ok()
}

/// A provider ready to use, and what it was built from.
pub struct Connection {
    /// The provider, already wrapped in the activity [`Trace`].
    pub provider: Arc<dyn ModelProvider>,
    /// The configuration it was built from (tags, cache directory, limits).
    pub config: ModelConfig,
    /// Which choice this is.
    pub choice: ProviderChoice,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn ModelProvider` is not `Debug`; its id is the useful part, and a
        // provider's own secrets are redacted wherever they are held.
        f.debug_struct("Connection")
            .field("choice", &self.choice)
            .field("provider", &self.provider.id())
            .field("config", &self.config)
            .finish()
    }
}

/// Builds the provider the settings (and, above them, the environment) name.
///
/// With a provider chosen, that provider is built. With none chosen — the
/// state before anyone opens Settings, and every developer's setup — it is
/// what the app has always done: Ollama if a key (or a local server) is
/// configured, else Gemini. Either way nothing touches the network here.
///
/// # Errors
///
/// [`ModelError::Config`] when the two model names are not set anywhere,
/// [`ModelError::MissingApiKey`] when the provider needs a key and neither
/// the environment nor `store` has one.
pub fn connect(
    settings: &ModelSettings,
    env: &dyn EnvSource,
    store: &dyn SecretStore,
) -> Result<Connection, ModelError> {
    let own = settings.to_env();
    let layered = LayeredEnv::new(env, &own);
    let config = ModelConfig::load(&layered)?;

    let build_ollama = |config: &ModelConfig| -> Result<Arc<dyn ModelProvider>, ModelError> {
        let provider = OllamaProvider::from_config(config, ModelTier::Small, env, store)?;
        Ok(Arc::new(Trace::new(provider, crate::trace::global())))
    };
    let build_gemini = |config: &ModelConfig| -> Result<Arc<dyn ModelProvider>, ModelError> {
        let provider = GeminiProvider::from_config(config, ModelTier::Small, env, store)?;
        Ok(Arc::new(Trace::new(provider, crate::trace::global())))
    };

    let build_anthropic = |config: &ModelConfig| -> Result<Arc<dyn ModelProvider>, ModelError> {
        let provider = AnthropicProvider::from_config(config, ModelTier::Small, env, store)?;
        Ok(Arc::new(Trace::new(provider, crate::trace::global())))
    };
    let build_openai = |config: &ModelConfig| -> Result<Arc<dyn ModelProvider>, ModelError> {
        let provider = OpenAiProvider::from_config(config, ModelTier::Small, env, store)?;
        Ok(Arc::new(Trace::new(provider, crate::trace::global())))
    };

    let (provider, choice) = match settings.provider {
        Some(ProviderChoice::Anthropic) => (build_anthropic(&config)?, ProviderChoice::Anthropic),
        Some(ProviderChoice::OpenAiCompatible) => {
            (build_openai(&config)?, ProviderChoice::OpenAiCompatible)
        }
        Some(choice @ (ProviderChoice::OllamaCloud | ProviderChoice::OllamaLocal)) => {
            (build_ollama(&config)?, choice)
        }
        Some(ProviderChoice::Gemini) => (build_gemini(&config)?, ProviderChoice::Gemini),
        None => match build_ollama(&config) {
            Ok(provider) => {
                let choice = if config.ollama_is_local() {
                    ProviderChoice::OllamaLocal
                } else {
                    ProviderChoice::OllamaCloud
                };
                (provider, choice)
            }
            Err(ollama_error) => match build_gemini(&config) {
                Ok(provider) => (provider, ProviderChoice::Gemini),
                // The first failure is the useful one: Ollama is tried first,
                // so its missing key is the thing to fix.
                Err(_) => return Err(ollama_error),
            },
        },
    };
    Ok(Connection {
        provider,
        config,
        choice,
    })
}

/// Every model `choice` offers, sorted — what the Settings screen puts in its
/// pickers. Listing also proves the key and address work, before a task spends
/// a call on them. This is the one place this module uses the network, and a
/// person's button press is what starts it.
///
/// # Errors
///
/// [`ModelError::MissingApiKey`] when a key is needed and `key` is `None`;
/// [`ModelError::Timeout`] after [`LIST_MODELS_TIMEOUT`]; otherwise the typed
/// transport, status and parse errors of the backend. A key never appears in
/// one.
pub async fn list_models(
    choice: ProviderChoice,
    base_url: &str,
    key: Option<Token>,
) -> Result<Vec<String>, ModelError> {
    list_models_within(choice, base_url, key, LIST_MODELS_TIMEOUT).await
}

async fn list_models_within(
    choice: ProviderChoice,
    base_url: &str,
    key: Option<Token>,
    timeout: Duration,
) -> Result<Vec<String>, ModelError> {
    let work = async {
        match choice {
            ProviderChoice::OllamaLocal => {
                // No key is sent to a local server, whatever was typed.
                OllamaProvider::new(base_url, None, ModelTier::Small, DEFAULT_MAX_RESPONSE_BYTES)
                    .list_tags()
                    .await
            }
            ProviderChoice::OllamaCloud => {
                let key = key.ok_or_else(|| missing_key(choice))?;
                OllamaProvider::new(
                    base_url,
                    Some(key),
                    ModelTier::Small,
                    DEFAULT_MAX_RESPONSE_BYTES,
                )
                .list_tags()
                .await
            }
            ProviderChoice::Gemini => {
                let key = key.ok_or_else(|| missing_key(choice))?;
                GeminiProvider::new(base_url, key, ModelTier::Small, DEFAULT_MAX_RESPONSE_BYTES)
                    .list_models()
                    .await
            }
            ProviderChoice::Anthropic => {
                let key = key.ok_or_else(|| missing_key(choice))?;
                AnthropicProvider::new(base_url, key, ModelTier::Small, DEFAULT_MAX_RESPONSE_BYTES)
                    .list_models()
                    .await
            }
            ProviderChoice::OpenAiCompatible => {
                // A server on this computer may take no key.
                if key.is_none() && !is_local_url(base_url) {
                    return Err(missing_key(choice));
                }
                OpenAiProvider::new(base_url, key, ModelTier::Small, DEFAULT_MAX_RESPONSE_BYTES)
                    .list_models()
                    .await
            }
        }
    };
    tokio::time::timeout(timeout, work)
        .await
        .unwrap_or(Err(ModelError::Timeout {
            provider: choice.provider_id(),
            after: timeout,
        }))
}

fn missing_key(choice: ProviderChoice) -> ModelError {
    let var = choice.key_var().unwrap_or("");
    ModelError::MissingApiKey {
        env_var: var,
        keyring_service: KEYRING_SERVICE,
        keyring_account: var.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::{MemoryVault, NoSecretStore, SecretVault};
    use crate::testing::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn cloud_with(small: &str, main: &str) -> ModelSettings {
        let mut s = ModelSettings {
            provider: Some(ProviderChoice::OllamaCloud),
            ..ModelSettings::default()
        };
        s.ollama_cloud = ModelPair {
            small: small.into(),
            main: main.into(),
        };
        s
    }

    // ── the file ─────────────────────────────────────────────────────────

    #[test]
    fn a_new_install_has_no_provider_and_the_standard_local_address() {
        let s = ModelSettings::default();
        assert_eq!(s.provider, None);
        assert_eq!(s.ollama_local_url, LOCAL_OLLAMA_BASE_URL);
        assert!(s.problem().is_none(), "nothing chosen is not a problem yet");
        assert!(s.to_env().get(ModelTier::Small.env_var()).is_none());
    }

    #[test]
    fn settings_survive_a_save_and_a_load() {
        let dir = TempDir::new("settings-roundtrip");
        let path = dir.path().join("nested").join(SETTINGS_FILE_NAME);
        let mut s = cloud_with("small-a", "main-b");
        s.gemini.main = "gem-main".into();
        s.save(&path).expect("saves, creating the folder");
        assert_eq!(ModelSettings::load(&path).expect("loads"), s);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "no temp file left"
        );
    }

    #[test]
    fn a_missing_file_is_a_first_run_not_an_error() {
        let dir = TempDir::new("settings-missing");
        let s = ModelSettings::load(&dir.path().join("nope.json")).expect("default");
        assert_eq!(s, ModelSettings::default());
    }

    #[test]
    fn a_corrupt_file_is_an_error_that_names_the_file() {
        let dir = TempDir::new("settings-corrupt");
        let path = dir.path().join(SETTINGS_FILE_NAME);
        std::fs::write(&path, "{ not json").unwrap();
        let err = ModelSettings::load(&path).expect_err("corrupt");
        assert!(err.to_string().contains(SETTINGS_FILE_NAME), "{err}");
    }

    #[test]
    fn a_file_from_a_future_version_with_extra_fields_still_loads() {
        let dir = TempDir::new("settings-future");
        let path = dir.path().join(SETTINGS_FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":2,"provider":"gemini","new_thing":{"a":1}}"#,
        )
        .unwrap();
        let s = ModelSettings::load(&path).expect("tolerant");
        assert_eq!(s.provider, Some(ProviderChoice::Gemini));
    }

    #[test]
    fn the_file_never_holds_a_key() {
        let dir = TempDir::new("settings-nokey");
        let path = dir.path().join(SETTINGS_FILE_NAME);
        cloud_with("a", "b").save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap().to_lowercase();
        assert!(!text.contains("key") && !text.contains("secret") && !text.contains("token"));
    }

    // ── what is a usable choice ─────────────────────────────────────────

    #[test]
    fn a_choice_needs_both_models_and_says_which_is_missing() {
        assert_eq!(
            cloud_with("", "m").problem().as_deref(),
            Some("Choose a model for the fast role.")
        );
        assert_eq!(
            cloud_with("s", "  ").problem().as_deref(),
            Some("Choose a model for the agent role.")
        );
        assert!(cloud_with("s", "m").problem().is_none());
    }

    #[test]
    fn a_model_name_with_spaces_or_a_novel_is_refused() {
        assert!(cloud_with("two words", "m").problem().is_some());
        assert!(
            cloud_with("s", &"x".repeat(MAX_MODEL_NAME_LEN + 1))
                .problem()
                .is_some()
        );
        assert!(
            cloud_with("s", "tag:with/odd-chars_1.5")
                .problem()
                .is_none()
        );
    }

    #[test]
    fn a_local_server_must_be_on_this_computer() {
        let mut s = ModelSettings {
            provider: Some(ProviderChoice::OllamaLocal),
            ..ModelSettings::default()
        };
        s.ollama_local = ModelPair {
            small: "s".into(),
            main: "m".into(),
        };
        assert!(s.problem().is_none());
        for ok in [
            "http://localhost:11434",
            "http://127.0.0.1:9999",
            "http://[::1]:1",
        ] {
            s.ollama_local_url = ok.into();
            assert!(s.problem().is_none(), "{ok}");
        }
        for bad in [
            "http://192.168.1.5:11434",
            "https://ollama.com",
            "http://localhost.evil.example",
        ] {
            s.ollama_local_url = bad.into();
            assert!(s.problem().is_some(), "{bad}");
        }
        s.ollama_local_url = "not a url".into();
        assert!(s.problem().is_some());
        s.ollama_local_url = "  ".into();
        assert!(s.problem().is_none(), "blank means the standard address");
    }

    // ── translation into the environment vocabulary ─────────────────────

    #[test]
    fn settings_translate_into_the_environment_vocabulary() {
        let env = cloud_with(" s ", "m").to_env();
        assert_eq!(env.get("FERRITE_MODEL_SMALL").as_deref(), Some("s"));
        assert_eq!(env.get("FERRITE_MODEL_MAIN").as_deref(), Some("m"));
        assert_eq!(
            env.get("FERRITE_OLLAMA_BASE_URL").as_deref(),
            Some(DEFAULT_OLLAMA_BASE_URL)
        );
        let mut local = ModelSettings {
            provider: Some(ProviderChoice::OllamaLocal),
            ..ModelSettings::default()
        };
        local.ollama_local = ModelPair {
            small: "s".into(),
            main: "m".into(),
        };
        assert_eq!(
            local.to_env().get("FERRITE_OLLAMA_BASE_URL").as_deref(),
            Some(LOCAL_OLLAMA_BASE_URL)
        );
    }

    #[test]
    fn only_the_active_provider_contributes() {
        let mut s = cloud_with("cloud-s", "cloud-m");
        s.gemini = ModelPair {
            small: "g-s".into(),
            main: "g-m".into(),
        };
        assert_eq!(
            s.to_env().get("FERRITE_MODEL_SMALL").as_deref(),
            Some("cloud-s")
        );
        s.provider = Some(ProviderChoice::Gemini);
        assert_eq!(
            s.to_env().get("FERRITE_MODEL_SMALL").as_deref(),
            Some("g-s")
        );
    }

    #[test]
    fn an_exported_variable_beats_the_saved_choice_and_is_reported() {
        let s = cloud_with("saved-s", "saved-m");
        let real = MapEnv::new().with("FERRITE_MODEL_SMALL", "exported-s");
        let own = s.to_env();
        let layered = LayeredEnv::new(&real, &own);
        assert_eq!(
            layered.get("FERRITE_MODEL_SMALL").as_deref(),
            Some("exported-s")
        );
        assert_eq!(
            layered.get("FERRITE_MODEL_MAIN").as_deref(),
            Some("saved-m")
        );
        assert_eq!(s.env_overrides(&real), vec!["FERRITE_MODEL_SMALL"]);
        assert!(s.env_overrides(&MapEnv::new()).is_empty());
    }

    #[test]
    fn the_server_address_follows_env_then_file_then_default() {
        let s = ModelSettings {
            ollama_local_url: "http://localhost:5".into(),
            ..ModelSettings::default()
        };
        let none = MapEnv::new();
        assert_eq!(
            s.base_url(ProviderChoice::OllamaLocal, &none),
            "http://localhost:5"
        );
        assert_eq!(
            s.base_url(ProviderChoice::OllamaCloud, &none),
            DEFAULT_OLLAMA_BASE_URL
        );
        assert_eq!(
            s.base_url(ProviderChoice::Gemini, &none),
            DEFAULT_GEMINI_BASE_URL
        );
        let exported = MapEnv::new().with("FERRITE_OLLAMA_BASE_URL", "http://127.0.0.1:7");
        assert_eq!(
            s.base_url(ProviderChoice::OllamaLocal, &exported),
            "http://127.0.0.1:7"
        );
    }

    // ── keys ─────────────────────────────────────────────────────────────

    #[test]
    fn key_source_names_where_the_key_is_without_reading_it() {
        let vault = MemoryVault::new();
        let none = MapEnv::new();
        assert_eq!(
            key_source(ProviderChoice::OllamaCloud, &none, &vault),
            KeySource::Missing
        );
        assert_eq!(
            key_source(ProviderChoice::OllamaLocal, &none, &vault),
            KeySource::NotNeeded
        );
        vault.set(KEYRING_SERVICE, OLLAMA_API_KEY_VAR, "k").unwrap();
        assert_eq!(
            key_source(ProviderChoice::OllamaCloud, &none, &vault),
            KeySource::Keyring
        );
        let exported = MapEnv::new().with(OLLAMA_API_KEY_VAR, "e");
        assert_eq!(
            key_source(ProviderChoice::OllamaCloud, &exported, &vault),
            KeySource::Environment(OLLAMA_API_KEY_VAR)
        );
    }

    #[test]
    fn a_just_typed_key_is_used_before_anything_is_saved() {
        let vault = MemoryVault::new();
        vault
            .set(KEYRING_SERVICE, GEMINI_API_KEY_VAR, "saved")
            .unwrap();
        let none = MapEnv::new();
        let typed = resolve_key(ProviderChoice::Gemini, "  typed  ", &none, &vault).unwrap();
        assert_eq!(typed.expose(), "typed");
        let saved = resolve_key(ProviderChoice::Gemini, "   ", &none, &vault).unwrap();
        assert_eq!(saved.expose(), "saved");
        assert!(resolve_key(ProviderChoice::OllamaLocal, "typed", &none, &vault).is_none());
        assert!(resolve_key(ProviderChoice::OllamaCloud, "", &none, &NoSecretStore).is_none());
    }

    // ── connect ──────────────────────────────────────────────────────────

    fn sandbox_env() -> MapEnv {
        // The cache directory is a required config value; point it somewhere
        // that is never created (connect() builds providers, not caches).
        MapEnv::new().with("FERRITE_MODEL_CACHE_DIR", "/tmp/ferrite-settings-test")
    }

    #[test]
    fn a_chosen_cloud_provider_is_built_from_the_saved_models_and_keyring_key() {
        let vault = MemoryVault::new();
        vault
            .set(KEYRING_SERVICE, OLLAMA_API_KEY_VAR, "sk-saved")
            .unwrap();
        let conn =
            connect(&cloud_with("s-tag", "m-tag"), &sandbox_env(), &vault).expect("connects");
        assert_eq!(conn.choice, ProviderChoice::OllamaCloud);
        assert_eq!(conn.provider.id(), ProviderId::Ollama);
        assert_eq!(conn.config.tag(ModelTier::Small), "s-tag");
        assert_eq!(conn.config.tag(ModelTier::Main), "m-tag");
        assert!(
            !format!("{conn:?}").contains("sk-saved"),
            "the key must not print"
        );
    }

    #[test]
    fn a_cloud_provider_without_a_key_names_the_two_places_to_put_one() {
        let err =
            connect(&cloud_with("s", "m"), &sandbox_env(), &NoSecretStore).expect_err("no key");
        assert!(matches!(err, ModelError::MissingApiKey { .. }), "{err:?}");
    }

    #[test]
    fn a_local_server_needs_no_key() {
        let mut s = ModelSettings {
            provider: Some(ProviderChoice::OllamaLocal),
            ..ModelSettings::default()
        };
        s.ollama_local = ModelPair {
            small: "s".into(),
            main: "m".into(),
        };
        let conn = connect(&s, &sandbox_env(), &NoSecretStore).expect("keyless");
        assert_eq!(conn.choice, ProviderChoice::OllamaLocal);
        assert!(conn.config.ollama_is_local());
    }

    #[test]
    fn a_chosen_gemini_provider_is_built_from_its_own_models_and_key() {
        let vault = MemoryVault::new();
        vault
            .set(KEYRING_SERVICE, GEMINI_API_KEY_VAR, "g-key")
            .unwrap();
        let mut s = ModelSettings {
            provider: Some(ProviderChoice::Gemini),
            ..ModelSettings::default()
        };
        s.gemini = ModelPair {
            small: "g-s".into(),
            main: "g-m".into(),
        };
        let conn = connect(&s, &sandbox_env(), &vault).expect("connects");
        assert_eq!(conn.provider.id(), ProviderId::Gemini);
        assert_eq!(conn.config.tag(ModelTier::Main), "g-m");
        assert!(connect(&s, &sandbox_env(), &NoSecretStore).is_err());
    }

    #[test]
    fn with_no_choice_the_environment_alone_decides_as_it_always_did() {
        let env = sandbox_env()
            .with("FERRITE_MODEL_SMALL", "e-s")
            .with("FERRITE_MODEL_MAIN", "e-m")
            .with(OLLAMA_API_KEY_VAR, "sk-env");
        let conn = connect(&ModelSettings::default(), &env, &NoSecretStore).expect("ollama");
        assert_eq!(conn.provider.id(), ProviderId::Ollama);

        let gemini_only = sandbox_env()
            .with("FERRITE_MODEL_SMALL", "e-s")
            .with("FERRITE_MODEL_MAIN", "e-m")
            .with(GEMINI_API_KEY_VAR, "g-env");
        let conn =
            connect(&ModelSettings::default(), &gemini_only, &NoSecretStore).expect("falls back");
        assert_eq!(conn.provider.id(), ProviderId::Gemini);
    }

    #[test]
    fn with_nothing_configured_anywhere_connect_says_what_is_missing() {
        let err = connect(&ModelSettings::default(), &sandbox_env(), &NoSecretStore)
            .expect_err("nothing");
        assert!(err.to_string().contains("FERRITE_MODEL_SMALL"), "{err}");
        let tags_but_no_key = sandbox_env()
            .with("FERRITE_MODEL_SMALL", "s")
            .with("FERRITE_MODEL_MAIN", "m");
        let err = connect(&ModelSettings::default(), &tags_but_no_key, &NoSecretStore)
            .expect_err("no key");
        assert!(
            matches!(
                err,
                ModelError::MissingApiKey {
                    env_var: OLLAMA_API_KEY_VAR,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    // ── listing models, against a loopback server ───────────────────────

    /// Answers every request with `status`/`body` after `delay`, and records
    /// the raw requests. Nothing leaves the machine.
    struct Fake {
        url: String,
        seen: Arc<std::sync::Mutex<Vec<String>>>,
    }

    async fn fake(status: u16, body: &str, delay: Duration) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let body = body.to_string();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (log, body) = (Arc::clone(&log), body.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_string());
                    tokio::time::sleep(delay).await;
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        Fake { url, seen }
    }

    const TAGS: &str = r#"{"models":[{"name":"qwen3:8b"},{"name":"gemma3:27b"}]}"#;

    #[tokio::test]
    async fn a_local_server_lists_its_models_and_is_sent_no_key() {
        let server = fake(200, TAGS, Duration::ZERO).await;
        let models = list_models(
            ProviderChoice::OllamaLocal,
            &server.url,
            Some(Token::new("sk-typed")),
        )
        .await
        .expect("lists");
        assert_eq!(models, vec!["gemma3:27b", "qwen3:8b"]);
        let request = server.seen.lock().unwrap()[0].to_lowercase();
        assert!(request.starts_with("get /api/tags"), "{request}");
        assert!(
            !request.contains("authorization") && !request.contains("sk-typed"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn a_cloud_listing_sends_the_key_as_a_bearer_and_needs_one() {
        let server = fake(200, TAGS, Duration::ZERO).await;
        list_models(
            ProviderChoice::OllamaCloud,
            &server.url,
            Some(Token::new("sk-abc")),
        )
        .await
        .expect("lists");
        assert!(server.seen.lock().unwrap()[0].contains("sk-abc"));
        let err = list_models(ProviderChoice::OllamaCloud, &server.url, None)
            .await
            .expect_err("keyless");
        assert!(matches!(err, ModelError::MissingApiKey { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_gemini_listing_filters_to_models_that_can_generate() {
        let body = r#"{"models":[
            {"name":"models/gemini-x","supportedGenerationMethods":["generateContent"]},
            {"name":"models/embed-y","supportedGenerationMethods":["embedContent"]}]}"#;
        let server = fake(200, body, Duration::ZERO).await;
        let models = list_models(
            ProviderChoice::Gemini,
            &server.url,
            Some(Token::new("g-key")),
        )
        .await
        .expect("lists");
        assert_eq!(models, vec!["gemini-x"]);
        assert!(server.seen.lock().unwrap()[0].contains("key=g-key"));
    }

    #[tokio::test]
    async fn a_rejected_key_is_a_typed_client_error_not_an_empty_list() {
        let server = fake(401, r#"{"error":"bad key"}"#, Duration::ZERO).await;
        let err = list_models(
            ProviderChoice::OllamaCloud,
            &server.url,
            Some(Token::new("nope")),
        )
        .await
        .expect_err("rejected");
        assert!(
            matches!(err, ModelError::ClientError { status: 401, .. }),
            "{err:?}"
        );
        assert!(
            !err.to_string().contains("nope"),
            "the key must not appear: {err}"
        );
    }

    #[tokio::test]
    async fn a_gemini_error_never_contains_the_key_even_though_it_is_in_the_url() {
        // Nothing listens on this port, so the transport error is the one
        // reqwest builds from the full URL, query string included.
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let err = list_models(
            ProviderChoice::Gemini,
            &closed,
            Some(Token::new("g-secret-123")),
        )
        .await
        .expect_err("down");
        assert!(!err.to_string().contains("g-secret-123"), "{err}");
    }

    #[tokio::test]
    async fn a_server_that_never_answers_is_a_timeout() {
        let server = fake(200, TAGS, Duration::from_secs(5)).await;
        let err = list_models_within(
            ProviderChoice::OllamaLocal,
            &server.url,
            None,
            Duration::from_millis(100),
        )
        .await
        .expect_err("slow");
        assert!(matches!(err, ModelError::Timeout { .. }), "{err:?}");
    }
}
