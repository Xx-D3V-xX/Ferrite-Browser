//! Synthetic PII generation and TTL-rotated, encrypted-at-rest storage.
//!
//! [`SyntheticTwin`] itself is a straight port of the pre-rebuild
//! `twin.rs`'s generator — legitimate reference material per
//! `docs/AUDIT.md`, kept as-is. What changed is [`TwinManager`]: it no
//! longer has a compiled-in key to hand `encrypt_twin`/`decrypt_twin`
//! (T-008/D8) — it resolves one via [`super::key::resolve_twin_key`] on
//! every call, and a resolution failure is a real `Err` up through
//! [`TwinManager::load_or_generate`], not a silently-skipped cache step.

use std::path::PathBuf;

use ferrite_model::config::{EnvSource, SystemEnv};
use ferrite_model::secret::{OsKeyring, SecretStore};
use rand::Rng;

use super::crypto::{decrypt_twin, derive_key, encrypt_twin};
use super::key::{resolve_twin_key, TwinKeyError};

/// A fake but realistic set of user credentials used during dry runs.
///
/// All values are plausible but entirely fictitious — see the `twin`
/// module's top-level docs for exactly what encrypting this does and does
/// not protect.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyntheticTwin {
    pub name: String,
    pub email: String,
    pub password: String,
    pub phone: String,
    pub credit_card: String,
    pub ssn: String,
    pub address: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl SyntheticTwin {
    /// Generates a new random synthetic twin.
    /// All values are plausible but fictitious.
    #[must_use]
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let id: u32 = rng.gen_range(1000..9999);
        Self {
            name: format!("Alex Ferrite-{}", id),
            email: format!("user{}@ferrite-test.invalid", id),
            password: format!("Synth!Pass{}#", id),
            phone: format!("+1-555-{:04}-{:04}", id, rng.gen_range(1000u32..9999u32)),
            credit_card: format!("4000-0000-0000-{:04}", id),
            ssn: format!("000-00-{:04}", id),
            address: format!("{} Synthetic Ave, Testville, CA 00000", id),
            created_at: chrono::Utc::now(),
        }
    }

    /// Returns true if the twin is older than `ttl_hours`.
    #[must_use]
    pub fn is_expired(&self, ttl_hours: i64) -> bool {
        let age = chrono::Utc::now() - self.created_at;
        age.num_hours() >= ttl_hours
    }
}

/// Manages the stored synthetic twin, including TTL rotation and key
/// resolution.
///
/// Holds its secret sources as trait objects ([`EnvSource`],
/// [`SecretStore`]) rather than touching `std::env`/the keyring directly,
/// so [`TwinManager::with_secret_source`] can hand tests a fake of each —
/// the same pattern `ferrite-model`'s own tests use, and required here for
/// the same reason (R7: no real keyring access from a test suite).
///
/// The trait objects are bounded `+ Send + Sync` explicitly: `EnvSource`
/// itself (`ferrite-model`, not this crate's to edit) carries no such
/// supertrait, but every caller of `DryRunOrchestrator::run` needs the
/// whole chain — `RecordingExecutor` → `TwinManager` → these trait objects —
/// to be `Send` so it can live across an `.await` inside a spawned task
/// (`ferrite-ui`'s agent loop does exactly this). `SystemEnv`/`MapEnv` and
/// `OsKeyring`/`MapSecretStore`/`NoSecretStore` are all trivially `Send +
/// Sync` (plain data, no interior mutability), so this bound costs nothing
/// real — it only makes explicit what every concrete source already is.
#[derive(Debug)]
pub struct TwinManager {
    storage_path: PathBuf,
    ttl_hours: i64,
    env: Box<dyn EnvSource + Send + Sync>,
    store: Box<dyn SecretStore>,
}

impl TwinManager {
    /// The production constructor: real environment, real OS keyring.
    #[must_use]
    pub fn new(storage_path: PathBuf) -> Self {
        Self::with_secret_source(storage_path, Box::new(SystemEnv), Box::new(OsKeyring))
    }

    /// Builds a manager against injected secret sources — for tests (a
    /// [`ferrite_model::config::MapEnv`]/
    /// [`ferrite_model::secret::MapSecretStore`] pair, never the real
    /// keyring) and for any future caller that wants to supply its own
    /// resolved key material without going through the environment.
    #[must_use]
    pub fn with_secret_source(
        storage_path: PathBuf,
        env: Box<dyn EnvSource + Send + Sync>,
        store: Box<dyn SecretStore>,
    ) -> Self {
        Self {
            storage_path,
            ttl_hours: 24,
            env,
            store,
        }
    }

    /// Loads the stored twin if present, decryptable, and unexpired.
    /// Generates and stores a fresh twin otherwise.
    ///
    /// # Errors
    ///
    /// [`TwinKeyError`] when neither the keyring nor `FERRITE_TWIN_KEY` has
    /// a key. This is checked on every call (not cached), so setting the
    /// key mid-session is picked up on the next dry run without a restart.
    /// A resolvable key that merely fails to *decrypt* the existing cache
    /// file (wrong key, corrupted file) is not an error here — it is
    /// treated the same as a missing cache file and a fresh twin is
    /// generated, matching the pre-rebuild behavior for a stale/corrupt
    /// cache.
    pub fn load_or_generate(&self) -> Result<SyntheticTwin, TwinKeyError> {
        let secret = resolve_twin_key(self.env.as_ref(), self.store.as_ref())?;
        let key = derive_key(&secret);

        if let Ok(data) = std::fs::read(&self.storage_path) {
            if let Ok(twin) = decrypt_twin(&data, &key) {
                if !twin.is_expired(self.ttl_hours) {
                    return Ok(twin);
                }
            }
        }
        let twin = SyntheticTwin::generate();
        if let Ok(enc) = encrypt_twin(&twin, &key) {
            let _ = std::fs::write(&self.storage_path, enc);
        }
        Ok(twin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_model::config::MapEnv;
    use ferrite_model::secret::{MapSecretStore, NoSecretStore};

    fn test_manager(path: PathBuf) -> TwinManager {
        let env = MapEnv::new().with(super::super::key::TWIN_KEY_ENV_VAR, "test-only-twin-key");
        TwinManager::with_secret_source(path, Box::new(env), Box::new(NoSecretStore))
    }

    #[test]
    fn twin_generate_produces_unique_values() {
        let t1 = SyntheticTwin::generate();
        let t2 = SyntheticTwin::generate();
        assert_ne!(t1.email, t2.email);
    }

    #[test]
    fn twin_not_expired_immediately() {
        let twin = SyntheticTwin::generate();
        assert!(!twin.is_expired(24));
    }

    #[test]
    fn manager_with_a_resolvable_key_loads_the_same_twin_on_a_second_call() {
        let path =
            std::env::temp_dir().join(format!("ferrite-twin-test-{}.enc", uuid::Uuid::new_v4()));
        let mgr = test_manager(path.clone());
        let twin = mgr.load_or_generate().expect("resolves and generates");
        assert!(twin.email.contains("ferrite-test.invalid"));
        let twin2 = mgr.load_or_generate().expect("resolves and loads");
        assert_eq!(twin.email, twin2.email);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn manager_without_a_resolvable_key_fails_loudly_not_silently() {
        let path =
            std::env::temp_dir().join(format!("ferrite-twin-test-{}.enc", uuid::Uuid::new_v4()));
        let mgr =
            TwinManager::with_secret_source(path, Box::new(MapEnv::new()), Box::new(NoSecretStore));
        let err = mgr
            .load_or_generate()
            .expect_err("no key anywhere must be a real Err, not a fabricated twin");
        assert!(err.to_string().contains("FERRITE_TWIN_KEY"));
    }

    #[test]
    fn two_managers_with_different_keys_cannot_read_each_others_cache() {
        let path =
            std::env::temp_dir().join(format!("ferrite-twin-test-{}.enc", uuid::Uuid::new_v4()));
        let env_a = MapEnv::new().with(super::super::key::TWIN_KEY_ENV_VAR, "key-a");
        let mgr_a = TwinManager::with_secret_source(
            path.clone(),
            Box::new(env_a),
            Box::new(MapSecretStore::new()),
        );
        let twin_a = mgr_a.load_or_generate().expect("mgr_a resolves a key");

        let env_b = MapEnv::new().with(super::super::key::TWIN_KEY_ENV_VAR, "key-b");
        let mgr_b = TwinManager::with_secret_source(
            path.clone(),
            Box::new(env_b),
            Box::new(MapSecretStore::new()),
        );
        // mgr_b can't decrypt mgr_a's cache file with its own key, so it
        // transparently regenerates rather than erroring or leaking mgr_a's
        // twin — this is the "stale/corrupt cache" path, not a key error.
        let twin_b = mgr_b.load_or_generate().expect("mgr_b resolves a key");
        assert_ne!(twin_a.email, twin_b.email);

        let _ = std::fs::remove_file(path);
    }
}
