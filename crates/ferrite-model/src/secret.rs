//! API-key resolution — §10.1's "and nowhere else".
//!
//! > The API key comes from the `OLLAMA_API_KEY` environment variable or the
//! > OS keyring, and nowhere else. Not in `config.toml`, not in a `.env`
//! > that is committed, not in a default, not in a test fixture, not in a
//! > log line.
//!
//! This is a deliberate break from the pre-rebuild Gemini integration, which
//! fell back to a `gemini_key.txt` sitting next to the executable. A key in
//! a file beside the binary is a key that gets committed, shipped, or
//! screenshotted; the keyring is the OS's answer to the same problem and it
//! does not produce an artifact anyone can accidentally `git add`.
//!
//! [`Token`]'s [`Debug`] is redacted, so the one remaining way a key reaches
//! a log line — someone printing a struct that holds it — is closed by the
//! type rather than by remembering.

use std::collections::BTreeMap;

use crate::config::EnvSource;
use crate::error::ModelError;

/// The keyring service name every Ferrite secret is stored under.
pub const KEYRING_SERVICE: &str = "ferrite";

/// An API key.
///
/// Holds the value, prints nothing. There is no [`Display`](std::fmt::Display)
/// impl and no `Deref<Target = str>`: reading the secret takes an explicit
/// [`Token::expose`], which is a word a reviewer can grep for.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wraps a secret value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself. Every call site is a place a key could leak, so
    /// the name is deliberately conspicuous.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not even the length: that is information about the key.
        f.write_str("Token(<redacted>)")
    }
}

/// Where secrets are stored when they are not in the environment.
pub trait SecretStore: std::fmt::Debug + Send + Sync {
    /// The secret for `service`/`account`, or `None`.
    fn get(&self, service: &str, account: &str) -> Option<String>;
}

/// The operating system's keyring.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsKeyring;

impl SecretStore for OsKeyring {
    fn get(&self, service: &str, account: &str) -> Option<String> {
        // Any keyring failure — no entry, a locked keychain, no backend on
        // this machine — is "no key here", not a hard error: the caller's
        // next step is the same either way, and the resulting
        // `MissingApiKey` already explains both places to put one.
        let entry = keyring::Entry::new(service, account).ok()?;
        let value = entry.get_password().ok()?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

/// An in-memory store, for tests. Nothing here touches the real keyring —
/// a test suite that wrote to a developer's login keychain would be a
/// considerably worse citizen than one that makes a network call.
#[derive(Debug, Clone, Default)]
pub struct MapSecretStore(BTreeMap<(String, String), String>);

impl MapSecretStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a secret.
    #[must_use]
    pub fn with(
        mut self,
        service: impl Into<String>,
        account: impl Into<String>,
        secret: impl Into<String>,
    ) -> Self {
        self.0
            .insert((service.into(), account.into()), secret.into());
        self
    }
}

impl SecretStore for MapSecretStore {
    fn get(&self, service: &str, account: &str) -> Option<String> {
        self.0
            .get(&(service.to_string(), account.to_string()))
            .cloned()
    }
}

/// A store that never has anything — the explicit way to say "environment
/// only", used by the CLI's offline paths so no code path can reach a real
/// keyring by omission.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSecretStore;

impl SecretStore for NoSecretStore {
    fn get(&self, _service: &str, _account: &str) -> Option<String> {
        None
    }
}

/// Resolves a key: environment first, then the OS keyring, then an error
/// that names both places.
///
/// Environment first because that is what a CI runner and a one-off shell
/// override both use, and the keyring is the persistent default a developer
/// sets once.
///
/// # Errors
///
/// [`ModelError::MissingApiKey`] when neither source has it.
pub fn resolve(
    env: &dyn EnvSource,
    store: &dyn SecretStore,
    env_var: &'static str,
) -> Result<Token, ModelError> {
    if let Some(value) = env.get(env_var) {
        return Ok(Token::new(value));
    }
    if let Some(value) = store.get(KEYRING_SERVICE, env_var) {
        return Ok(Token::new(value));
    }
    Err(ModelError::MissingApiKey {
        env_var,
        keyring_service: KEYRING_SERVICE,
        keyring_account: env_var.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MapEnv;

    const VAR: &str = "OLLAMA_API_KEY";

    #[test]
    fn a_token_never_prints_its_value() {
        let token = Token::new("sk-live-do-not-print-me");
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
        assert!(!format!("{token:?}").contains("sk-live"));
        assert_eq!(token.expose(), "sk-live-do-not-print-me");
    }

    #[test]
    fn a_struct_holding_a_token_cannot_leak_it_through_derived_debug() {
        #[derive(Debug)]
        struct Holder {
            auth: Option<Token>,
        }
        let held = Holder {
            auth: Some(Token::new("sk-secret")),
        };
        assert!(!format!("{held:?}").contains("sk-secret"), "{held:?}");
        assert_eq!(
            held.auth.as_ref().map(Token::expose),
            Some("sk-secret"),
            "the value is still there — it is only the rendering that is redacted"
        );
    }

    #[test]
    fn the_environment_is_consulted_first() {
        let env = MapEnv::new().with(VAR, "from-env");
        let store = MapSecretStore::new().with(KEYRING_SERVICE, VAR, "from-keyring");
        assert_eq!(
            resolve(&env, &store, VAR).expect("found").expose(),
            "from-env"
        );
    }

    #[test]
    fn the_keyring_is_the_fallback() {
        let env = MapEnv::new();
        let store = MapSecretStore::new().with(KEYRING_SERVICE, VAR, "from-keyring");
        assert_eq!(
            resolve(&env, &store, VAR).expect("found").expose(),
            "from-keyring"
        );
    }

    #[test]
    fn neither_source_is_an_error_naming_both() {
        let err = resolve(&MapEnv::new(), &NoSecretStore, VAR).expect_err("no key anywhere");
        let message = err.to_string();
        assert!(message.contains(VAR));
        assert!(message.contains(KEYRING_SERVICE));
        assert!(
            matches!(err, ModelError::MissingApiKey { .. }),
            "and it must be typed, not a string: {err:?}"
        );
    }

    #[test]
    fn an_empty_environment_variable_falls_through_to_the_keyring() {
        // `OLLAMA_API_KEY=` in a shell profile is a common way to "unset" a
        // key; it must not shadow a perfectly good keyring entry.
        let env = MapEnv::new().with(VAR, "   ");
        let store = MapSecretStore::new().with(KEYRING_SERVICE, VAR, "from-keyring");
        assert_eq!(
            resolve(&env, &store, VAR).expect("found").expose(),
            "from-keyring"
        );
    }

    #[test]
    fn the_null_store_never_returns_anything() {
        assert_eq!(NoSecretStore.get(KEYRING_SERVICE, VAR), None);
    }
}
