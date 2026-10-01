//! Synthetic-identity generation for the dry run, encrypted at rest with a
//! resolved key (`docs/REBUILD_DIRECTIVE.md` §6/A6, closing `docs/TO-DO.md`
//! **T-008** / defect **D8**).
//!
//! # What changed from the pre-rebuild `twin.rs`
//!
//! The old module worked (`SyntheticTwin::generate`, AES-256-GCM at rest,
//! TTL rotation) but encrypted every twin with a literal 32-byte constant
//! compiled into the binary:
//! `const DEV_KEY: &[u8; 32] = b"ferrite-ipi-twin-dev-key-32byte!";`. Every
//! build of Ferrite, everywhere, encrypted with the same key — indistinguishable
//! from no encryption at all to anyone who has the source or the binary,
//! which is everyone. [`key::resolve_twin_key`] replaces it: the OS keyring
//! first, the `FERRITE_TWIN_KEY` environment variable second, and a typed,
//! actionable [`key::TwinKeyError`] — never a panic, never another
//! constant — when neither is configured.
//!
//! **Where that error goes:** [`manager::TwinManager::load_or_generate`]
//! propagates it as a real `Result::Err` — directly tested, see that
//! module's tests. `crate::dry_run::DryRunOrchestrator::run` (this crate's
//! only caller of `load_or_generate`, and itself called by
//! `ferrite-eval`/`ferrite-ui`, neither editable by this session) chooses
//! *not* to let that error abort the dry run: it logs a loud warning and
//! generates an unpersisted twin for that call instead, the same shape of
//! degrade `tool_decision`'s Gemini-key-optional fallback already uses
//! elsewhere in this crate. See that method's doc comment for the full
//! reasoning — the short version is that the twin key gates a caching
//! convenience, not any part of the actual containment/sanitizer/consent
//! security mechanism, so losing the cache is the right failure mode, not
//! losing the whole dry run.
//!
//! # What this actually protects (read before assuming more)
//!
//! [`manager::SyntheticTwin`] is **fake data**: a generated name, email,
//! password, card number, SSN and address, invented fresh for one dry run
//! so the sanitizer/comparator/consent loop has something plausible to
//! react to when a scripted attack tries to exfiltrate "the user's" data.
//! None of it is real. Encrypting the on-disk cache file
//! (`TwinManager`'s `storage_path`, reused across calls within a TTL window
//! so a dry run doesn't regenerate an identity every single time) is
//! ordinary at-rest hygiene for a file that is *shaped* like a secrets
//! dump — it is not a claim that this module protects real user data, and
//! it must never be read as evidence for that stronger claim. If Ferrite
//! ever routes real user credentials through a twin-shaped cache, that is a
//! new decision with a new threat model, not an extension of this one.
//!
//! # Shape
//!
//! - [`key`] — resolution only: [`key::resolve_twin_key`] returns a raw
//!   [`ferrite_model::secret::Token`], reusing `ferrite-model`'s
//!   [`ferrite_model::secret::SecretStore`]/[`ferrite_model::secret::OsKeyring`]/
//!   [`ferrite_model::secret::Token`] and
//!   [`ferrite_model::config::EnvSource`] rather than reimplementing a
//!   keyring wrapper — see that module's docs for exactly what is and
//!   is not reused, and why the resolution order (keyring, then env) is
//!   the reverse of `ferrite-model`'s own.
//! - [`crypto`] — [`crypto::derive_key`] turns resolved secret material
//!   into a 32-byte AES key (SHA-256, no KDF — see that module's docs for
//!   why that is sufficient here), and
//!   [`crypto::encrypt_twin`]/[`crypto::decrypt_twin`] take that key as a
//!   parameter instead of reading a constant.
//! - [`manager`] — [`manager::SyntheticTwin`] (unchanged generator, ported
//!   from the pre-rebuild module) and [`manager::TwinManager`], which
//!   resolves a key on every [`manager::TwinManager::load_or_generate`]
//!   call rather than caching one, so a key added mid-session is picked up
//!   without a restart.

pub mod crypto;
pub mod key;
pub mod manager;

pub use crypto::{decrypt_twin, derive_key, encrypt_twin, TwinKey};
pub use key::{resolve_twin_key, TwinKeyError, TWIN_KEYRING_ACCOUNT, TWIN_KEY_ENV_VAR};
pub use manager::{SyntheticTwin, TwinManager};
