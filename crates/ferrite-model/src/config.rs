//! Configuration with environment overrides — `docs/TO-DO.md` **T-213**,
//! deferred from A2 because nothing below this layer had a config *value* to
//! load. This is the first real caller.
//!
//! Two rules from §10.2 shape the whole module:
//!
//! - **No model name appears in Rust source.** So [`ModelConfig::small`] and
//!   [`ModelConfig::main`] have no defaults. An unset `FERRITE_MODEL_SMALL`
//!   is a configuration error with an actionable message, not a silent
//!   fallback to a tag that Ollama may since have retired.
//! - **Everything else has a documented default**, because a budget or a
//!   timeout that must be set before anything runs is a tripwire, not a
//!   safety feature.
//!
//! Reading is done through [`EnvSource`] rather than `std::env::var`
//! directly, so tests configure a map instead of mutating process-global
//! state. That is not fastidiousness: `set_var` is racy across the threads
//! `cargo test` runs tests on, and the resulting flake looks like a config
//! bug rather than a test bug.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::ModelError;
use crate::provider::ModelTier;

/// Where configuration values come from.
pub trait EnvSource: std::fmt::Debug {
    /// The value of `key`, or `None` if unset or empty.
    fn get(&self, key: &str) -> Option<String>;
}

/// The process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnv;

impl EnvSource for SystemEnv {
    fn get(&self, key: &str) -> Option<String> {
        match std::env::var(key) {
            Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            _ => None,
        }
    }
}

/// A fixed set of values, for tests and for the CLI's `--set k=v` overrides.
#[derive(Debug, Clone, Default)]
pub struct MapEnv(BTreeMap<String, String>);

impl MapEnv {
    /// An empty environment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a value.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }
}

impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0
            .get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }
}

/// Default Ollama Cloud base URL (§10.1). A URL, not a model name — §10.2's
/// no-hardcoding rule is about *tags*, which Ollama retires; the API
/// endpoint is stable and having it in source is what makes the config
/// optional for the common case.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "https://ollama.com";
/// Default local Ollama base URL — the same wire protocol, no auth header.
pub const LOCAL_OLLAMA_BASE_URL: &str = "http://localhost:11434";
/// Default Gemini base URL, matching the pre-rebuild integration's endpoint.
pub const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// `FERRITE_MODEL_CALL_BUDGET`'s default (§10.3).
pub const DEFAULT_CALL_BUDGET: u32 = 500;
/// Default in-flight cap for the global semaphore (§10.3).
pub const DEFAULT_MAX_IN_FLIGHT: usize = 2;
/// Default per-request timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
/// Default ceiling on a response body. Comfortably above any legitimate
/// `num_predict`-capped completion and far below the 10 MB adversarial case.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Everything the model layer reads from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    /// `FERRITE_MODEL_SMALL` — required, no default (§10.2).
    pub small: String,
    /// `FERRITE_MODEL_MAIN` — required, no default (§10.2).
    pub main: String,
    /// `FERRITE_OLLAMA_BASE_URL`, default [`DEFAULT_OLLAMA_BASE_URL`].
    /// Point it at [`LOCAL_OLLAMA_BASE_URL`] for the offline fallback.
    pub ollama_base_url: String,
    /// `FERRITE_GEMINI_BASE_URL`, default [`DEFAULT_GEMINI_BASE_URL`].
    pub gemini_base_url: String,
    /// `FERRITE_MODEL_CALL_BUDGET`, default [`DEFAULT_CALL_BUDGET`].
    pub call_budget: u32,
    /// `FERRITE_MODEL_MAX_IN_FLIGHT`, default [`DEFAULT_MAX_IN_FLIGHT`].
    pub max_in_flight: usize,
    /// `FERRITE_MODEL_TIMEOUT_SECS`, default [`DEFAULT_TIMEOUT_SECS`].
    pub request_timeout: Duration,
    /// `FERRITE_MODEL_MAX_RESPONSE_BYTES`, default
    /// [`DEFAULT_MAX_RESPONSE_BYTES`].
    pub max_response_bytes: usize,
    /// `FERRITE_MODEL_CACHE_DIR`, default `~/.cache/ferrite-model` (§10.3).
    pub cache_dir: PathBuf,
}

impl ModelConfig {
    /// Loads from the process environment.
    ///
    /// # Errors
    ///
    /// [`ModelError::Config`] if a required tag is unset or a numeric
    /// override does not parse.
    pub fn from_env() -> Result<Self, ModelError> {
        Self::load(&SystemEnv)
    }

    /// Loads from an arbitrary [`EnvSource`].
    ///
    /// # Errors
    ///
    /// [`ModelError::Config`] if a required tag is unset, a numeric
    /// override does not parse, or the home directory cannot be resolved
    /// and no explicit cache dir was given.
    pub fn load(env: &dyn EnvSource) -> Result<Self, ModelError> {
        let small = require_tag(env, ModelTier::Small)?;
        let main = require_tag(env, ModelTier::Main)?;

        let cache_dir = match env.get("FERRITE_MODEL_CACHE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => default_cache_dir()?,
        };

        Ok(Self {
            small,
            main,
            ollama_base_url: env
                .get("FERRITE_OLLAMA_BASE_URL")
                .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_string()),
            gemini_base_url: env
                .get("FERRITE_GEMINI_BASE_URL")
                .unwrap_or_else(|| DEFAULT_GEMINI_BASE_URL.to_string()),
            call_budget: parse_or(env, "FERRITE_MODEL_CALL_BUDGET", DEFAULT_CALL_BUDGET)?,
            max_in_flight: parse_or(env, "FERRITE_MODEL_MAX_IN_FLIGHT", DEFAULT_MAX_IN_FLIGHT)?,
            request_timeout: Duration::from_secs(parse_or(
                env,
                "FERRITE_MODEL_TIMEOUT_SECS",
                DEFAULT_TIMEOUT_SECS,
            )?),
            max_response_bytes: parse_or(
                env,
                "FERRITE_MODEL_MAX_RESPONSE_BYTES",
                DEFAULT_MAX_RESPONSE_BYTES,
            )?,
            cache_dir,
        })
    }

    /// The configured tag for a tier.
    #[must_use]
    pub fn tag(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Small => &self.small,
            ModelTier::Main => &self.main,
        }
    }

    /// Both configured tags, deduplicated and sorted — what the startup
    /// preflight (§10.2) validates against `/api/tags`.
    ///
    /// Sorted because the failure message lists them and R8 forbids output
    /// whose order depends on a hash or on which tier was read first.
    #[must_use]
    pub fn configured_tags(&self) -> Vec<&str> {
        let mut tags = vec![self.small.as_str(), self.main.as_str()];
        tags.sort_unstable();
        tags.dedup();
        tags
    }

    /// Whether the configured Ollama endpoint is the local one, which takes
    /// no `Authorization` header (§10.1).
    /// Matching is on the whole host, not a prefix: `localhost.evil.example.com`
    /// starts with `localhost` and is emphatically not local, and treating it
    /// as such would mean silently dropping the `Authorization` header — or,
    /// worse in the other direction, sending a bearer token to it.
    #[must_use]
    pub fn ollama_is_local(&self) -> bool {
        let rest = self
            .ollama_base_url
            .split_once("://")
            .map_or(self.ollama_base_url.as_str(), |(_, rest)| rest);
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = match authority.strip_prefix('[') {
            // Bracketed IPv6 literal: the host is everything up to the `]`.
            Some(after) => match after.split_once(']') {
                Some((inner, _port)) => format!("[{inner}]"),
                None => return false,
            },
            None => authority.split(':').next().unwrap_or("").to_string(),
        };
        matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
    }
}

fn require_tag(env: &dyn EnvSource, tier: ModelTier) -> Result<String, ModelError> {
    env.get(tier.env_var()).ok_or_else(|| {
        ModelError::Config(format!(
            "{} is not set. Model tags are configuration, never source literals \
             (docs/REBUILD_DIRECTIVE.md §10.2: Ollama retires cloud models). \
             Run `just models` to list the tags your key can actually reach.",
            tier.env_var()
        ))
    })
}

fn parse_or<T>(env: &dyn EnvSource, key: &str, default: T) -> Result<T, ModelError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env.get(key) {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|e| ModelError::Config(format!("{key}={raw:?} is not a valid value: {e}"))),
    }
}

/// `~/.cache/ferrite-model`, matching the justfile's `~/.cache/ferrite-target`
/// convention for the build cache.
///
/// Public (not just used internally by [`ModelConfig::load`]) so `just
/// cache-stats` can resolve the default cache location without first
/// needing a full [`ModelConfig`] — which would otherwise force the two
/// required model tags to be set just to look at a directory that has
/// nothing to do with them.
///
/// # Errors
///
/// [`ModelError::Config`] if the home directory cannot be resolved.
pub fn default_cache_dir() -> Result<PathBuf, ModelError> {
    let home = dirs::home_dir().ok_or_else(|| {
        ModelError::Config(
            "cannot resolve the home directory for the response cache; \
             set FERRITE_MODEL_CACHE_DIR explicitly"
                .to_string(),
        )
    })?;
    Ok(home.join(".cache").join("ferrite-model"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> MapEnv {
        MapEnv::new()
            .with("FERRITE_MODEL_SMALL", "small-tag:1b")
            .with("FERRITE_MODEL_MAIN", "main-tag:31b")
            .with("FERRITE_MODEL_CACHE_DIR", "/tmp/ferrite-model-test")
    }

    #[test]
    fn a_missing_small_tag_is_an_actionable_config_error_not_a_default() {
        let env = MapEnv::new().with("FERRITE_MODEL_MAIN", "m");
        let err = ModelConfig::load(&env).expect_err("no default tag may exist");
        let msg = err.to_string();
        assert!(msg.contains("FERRITE_MODEL_SMALL"), "{msg}");
        assert!(
            msg.contains("just models"),
            "the message should say how to find a valid tag: {msg}"
        );
    }

    #[test]
    fn a_missing_main_tag_is_also_an_error() {
        let env = MapEnv::new().with("FERRITE_MODEL_SMALL", "s");
        let err = ModelConfig::load(&env).expect_err("no default tag may exist");
        assert!(err.to_string().contains("FERRITE_MODEL_MAIN"));
    }

    #[test]
    fn an_empty_or_whitespace_value_counts_as_unset() {
        let env = MapEnv::new()
            .with("FERRITE_MODEL_SMALL", "   ")
            .with("FERRITE_MODEL_MAIN", "m");
        assert!(ModelConfig::load(&env).is_err());
    }

    #[test]
    fn everything_but_the_tags_has_a_documented_default() {
        let cfg = ModelConfig::load(&minimal()).expect("only the two tags are required");
        assert_eq!(cfg.call_budget, DEFAULT_CALL_BUDGET);
        assert_eq!(cfg.max_in_flight, DEFAULT_MAX_IN_FLIGHT);
        assert_eq!(
            cfg.request_timeout,
            Duration::from_secs(DEFAULT_TIMEOUT_SECS)
        );
        assert_eq!(cfg.max_response_bytes, DEFAULT_MAX_RESPONSE_BYTES);
        assert_eq!(cfg.ollama_base_url, DEFAULT_OLLAMA_BASE_URL);
        assert_eq!(cfg.gemini_base_url, DEFAULT_GEMINI_BASE_URL);
    }

    #[test]
    fn every_default_is_overridable_from_the_environment() {
        let env = minimal()
            .with("FERRITE_MODEL_CALL_BUDGET", "7")
            .with("FERRITE_MODEL_MAX_IN_FLIGHT", "5")
            .with("FERRITE_MODEL_TIMEOUT_SECS", "3")
            .with("FERRITE_MODEL_MAX_RESPONSE_BYTES", "99")
            .with("FERRITE_OLLAMA_BASE_URL", LOCAL_OLLAMA_BASE_URL)
            .with("FERRITE_GEMINI_BASE_URL", "https://example.invalid/v1");
        let cfg = ModelConfig::load(&env).expect("loads");
        assert_eq!(cfg.call_budget, 7);
        assert_eq!(cfg.max_in_flight, 5);
        assert_eq!(cfg.request_timeout, Duration::from_secs(3));
        assert_eq!(cfg.max_response_bytes, 99);
        assert_eq!(cfg.ollama_base_url, LOCAL_OLLAMA_BASE_URL);
        assert_eq!(cfg.gemini_base_url, "https://example.invalid/v1");
    }

    #[test]
    fn a_non_numeric_override_is_an_error_naming_the_variable_and_the_value() {
        let env = minimal().with("FERRITE_MODEL_CALL_BUDGET", "lots");
        let err = ModelConfig::load(&env).expect_err("must not silently fall back");
        let msg = err.to_string();
        assert!(msg.contains("FERRITE_MODEL_CALL_BUDGET"), "{msg}");
        assert!(msg.contains("lots"), "{msg}");
    }

    #[test]
    fn tags_are_looked_up_by_tier() {
        let cfg = ModelConfig::load(&minimal()).expect("loads");
        assert_eq!(cfg.tag(ModelTier::Small), "small-tag:1b");
        assert_eq!(cfg.tag(ModelTier::Main), "main-tag:31b");
    }

    #[test]
    fn configured_tags_are_sorted_and_deduplicated() {
        let cfg = ModelConfig::load(&minimal()).expect("loads");
        assert_eq!(cfg.configured_tags(), vec!["main-tag:31b", "small-tag:1b"]);

        let same = MapEnv::new()
            .with("FERRITE_MODEL_SMALL", "one:1b")
            .with("FERRITE_MODEL_MAIN", "one:1b")
            .with("FERRITE_MODEL_CACHE_DIR", "/tmp/x");
        let cfg = ModelConfig::load(&same).expect("loads");
        assert_eq!(
            cfg.configured_tags(),
            vec!["one:1b"],
            "one tag configured twice is one tag to preflight"
        );
    }

    #[test]
    fn the_local_endpoint_is_recognised_so_no_auth_header_is_sent() {
        let local = |url: &str| {
            ModelConfig::load(&minimal().with("FERRITE_OLLAMA_BASE_URL", url))
                .expect("loads")
                .ollama_is_local()
        };
        assert!(local(LOCAL_OLLAMA_BASE_URL));
        assert!(local("http://127.0.0.1:11434"));
        assert!(local("http://localhost:1234"));
        assert!(local("http://[::1]:11434/"));
        assert!(!local(DEFAULT_OLLAMA_BASE_URL));
        assert!(
            !local("https://localhost.evil.example.com"),
            "a host that merely starts with the string is still remote"
        );
    }

    #[test]
    fn the_cache_dir_defaults_under_the_home_directory() {
        let env = MapEnv::new()
            .with("FERRITE_MODEL_SMALL", "s")
            .with("FERRITE_MODEL_MAIN", "m");
        let cfg = ModelConfig::load(&env).expect("home dir resolves on any supported platform");
        assert!(
            cfg.cache_dir.ends_with("ferrite-model"),
            "{}",
            cfg.cache_dir.display()
        );
        assert!(cfg.cache_dir.to_string_lossy().contains(".cache"));
    }
}
