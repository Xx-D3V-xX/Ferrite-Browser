//! Twin encryption-key resolution — closes `docs/TO-DO.md` **T-008** (D8):
//! the compiled-in `DEV_KEY` constant (`ferrite-ipi-twin-dev-key-32byte!`)
//! is gone. The key now comes from the OS keyring first, the
//! `FERRITE_TWIN_KEY` environment variable second, and neither source
//! present is a loud, typed [`TwinKeyError`] — never a panic, never a
//! silent fallback to any other constant.
//!
//! # Reuse, not reimplementation
//!
//! `ferrite-ipi` already depends on `ferrite-model` (downward, per
//! `CLAUDE.md`'s `core ← {model, audit, engine} ← ipi ← ...`), and
//! `ferrite-model::secret` already built exactly the keyring wrapper this
//! needs: [`ferrite_model::secret::OsKeyring`] (the real
//! `keyring::Entry::new` call, with "no entry / locked keychain / no
//! backend" all folded into a plain `None` rather than an error), the
//! [`ferrite_model::secret::SecretStore`] trait tests inject a fake against,
//! [`ferrite_model::secret::Token`] (a secret that cannot leak through a
//! derived `Debug`), and [`ferrite_model::config::EnvSource`] (environment
//! access as a trait, so tests configure a map instead of racing
//! `std::env::set_var` across `cargo test`'s parallel threads). All four are
//! reused here verbatim — this module does not open a second `keyring::Entry`
//! call site anywhere in the workspace.
//!
//! **What is *not* reused directly:** `ferrite_model::secret::resolve()`
//! itself. Its `Result` is [`ferrite_model::ModelError`], an enum shaped
//! entirely around model-provider failure modes (`Transport { provider }`,
//! `ModelTagUnavailable`, `RateLimited`, ...). Returning
//! `ModelError::MissingApiKey` for a twin encryption key would be a type
//! lie: a reader matching on `ModelError` would reasonably conclude a model
//! call failed, and the constructor's own field names
//! (`keyring_account: env_var.to_string()`) assume the env var name *is*
//! the keyring account, which is a model-provider convention, not
//! necessarily this crate's. [`TwinKeyError`] below carries the same two
//! named sources and the same actionable message shape (mirroring
//! `ModelError::MissingApiKey`'s wording per the charter), so nothing about
//! `resolve()`'s actual value is lost — only its ~10-line orchestration
//! (env-or-keyring-or-error) is duplicated, not the keyring wrapper itself.
//!
//! # Resolution order: keyring first, then environment
//!
//! Deliberately the *opposite* order from `ferrite_model::secret::resolve`
//! (env first, then keyring). That module's ordering is justified by "a CI
//! runner and a one-off shell override both use the environment" — a
//! reasonable default for a value that legitimately changes per invocation
//! (which model backend to hit). A twin encryption key is not like that: it
//! is a long-lived, at-rest secret an operator sets once, so the keyring
//! (the OS's actual purpose-built secret store) is checked first, with the
//! environment variable as the override/CI/no-keyring-backend fallback.
//! This asymmetry is intentional, not an oversight — flagged here so nobody
//! "fixes" it into matching `ferrite-model`'s order without reading this.

use ferrite_model::config::EnvSource;
use ferrite_model::secret::{SecretStore, Token, KEYRING_SERVICE};

/// The environment variable checked (second) for the twin encryption key.
pub const TWIN_KEY_ENV_VAR: &str = "FERRITE_TWIN_KEY";

/// The keyring account name checked (first), under
/// [`ferrite_model::secret::KEYRING_SERVICE`] ("ferrite") — the same
/// service `ferrite-model` stores `OLLAMA_API_KEY` under, different
/// account, one keyring for the whole application.
pub const TWIN_KEYRING_ACCOUNT: &str = "FERRITE_TWIN_KEY";

/// Neither the keyring nor the environment had a twin encryption key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "no twin encryption key found: checked the OS keyring (service \"{keyring_service}\", \
     account \"{keyring_account}\") and the {env_var} environment variable. Set {env_var}, \
     or store a key for service \"{keyring_service}\" account \"{keyring_account}\" in your \
     OS keyring. It may not go in a config file, a .env, or a default."
)]
pub struct TwinKeyError {
    /// The keyring service that was checked first.
    pub keyring_service: &'static str,
    /// The keyring account that was checked first.
    pub keyring_account: &'static str,
    /// The environment variable that was checked second.
    pub env_var: &'static str,
}

impl TwinKeyError {
    fn missing() -> Self {
        Self {
            keyring_service: KEYRING_SERVICE,
            keyring_account: TWIN_KEYRING_ACCOUNT,
            env_var: TWIN_KEY_ENV_VAR,
        }
    }
}

/// Resolves the twin encryption key material: OS keyring first, then
/// `FERRITE_TWIN_KEY`, then [`TwinKeyError`].
///
/// The returned [`Token`] is raw secret material, not yet a 32-byte AES key
/// — see [`super::crypto::derive_key`] for that step. Keeping resolution and
/// derivation separate means a resolution test never needs to reason about
/// AES at all, and a derivation test never needs a fake [`SecretStore`].
///
/// # Errors
///
/// [`TwinKeyError`] when neither source has a key.
pub fn resolve_twin_key(
    env: &dyn EnvSource,
    store: &dyn SecretStore,
) -> Result<Token, TwinKeyError> {
    if let Some(value) = store.get(KEYRING_SERVICE, TWIN_KEYRING_ACCOUNT) {
        return Ok(Token::new(value));
    }
    if let Some(value) = env.get(TWIN_KEY_ENV_VAR) {
        return Ok(Token::new(value));
    }
    Err(TwinKeyError::missing())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_model::config::MapEnv;
    use ferrite_model::secret::{MapSecretStore, NoSecretStore};

    #[test]
    fn the_keyring_is_consulted_first() {
        let env = MapEnv::new().with(TWIN_KEY_ENV_VAR, "from-env");
        let store =
            MapSecretStore::new().with(KEYRING_SERVICE, TWIN_KEYRING_ACCOUNT, "from-keyring");
        assert_eq!(
            resolve_twin_key(&env, &store).expect("found").expose(),
            "from-keyring"
        );
    }

    #[test]
    fn the_environment_is_the_fallback() {
        let env = MapEnv::new().with(TWIN_KEY_ENV_VAR, "from-env");
        let store = MapSecretStore::new();
        assert_eq!(
            resolve_twin_key(&env, &store).expect("found").expose(),
            "from-env"
        );
    }

    #[test]
    fn neither_source_is_a_typed_error_naming_both_and_never_a_panic() {
        // Not a #[should_panic] — a real Result::Err, actually observed.
        let result = std::panic::catch_unwind(|| resolve_twin_key(&MapEnv::new(), &NoSecretStore));
        let err = result
            .expect("resolve_twin_key must not panic")
            .expect_err("neither source has a key");
        let message = err.to_string();
        assert!(message.contains(TWIN_KEY_ENV_VAR));
        assert!(message.contains(KEYRING_SERVICE));
        assert!(message.contains(TWIN_KEYRING_ACCOUNT));
        assert_eq!(err, TwinKeyError::missing());
    }

    #[test]
    fn an_empty_environment_variable_does_not_shadow_the_keyring_fallback_path() {
        // Symmetric with ferrite-model's equivalent test: EnvSource::get
        // already filters empty/whitespace-only values to None, so this
        // exercises that resolve_twin_key relies on that contract rather
        // than re-checking emptiness itself.
        let env = MapEnv::new().with(TWIN_KEY_ENV_VAR, "   ");
        let store = MapSecretStore::new();
        assert!(resolve_twin_key(&env, &store).is_err());
    }
}
