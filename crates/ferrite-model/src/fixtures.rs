//! Record/replay fixtures — §10.3's second rate-limit mechanism.
//!
//! > A `just record` mode writes every live response into
//! > `tests/fixtures/model/<hash>.json`. These **are committed**.
//! > `ReplayProvider` serves them in integration tests and in CI, so the
//! > suite is free, deterministic, and offline — satisfying R7 while still
//! > exercising real model output rather than hand-written mocks.
//!
//! A fixture stores the provider's **raw wire body**, not a pre-digested
//! response. That is what makes the last clause true: replaying a fixture
//! runs the same parser the live backend runs, so the integration suite
//! exercises the wire format rather than a summary of it, and a parser
//! regression fails a test instead of passing one.
//!
//! The filename is the same [`CacheKey`] the response cache uses, computed
//! against the provider that *recorded* it. A fixture and a cache entry for
//! the same call therefore have the same name, which is deliberate: the two
//! mechanisms cannot disagree about what "the same request" means.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use ferrite_core::Clock;
use serde::{Deserialize, Serialize};

use crate::cache_key::CacheKey;
use crate::error::ModelError;
use crate::provider::{ModelTier, ProviderId};
use crate::request::CompletionRequest;

/// Where committed fixtures live.
///
/// Resolved from `CARGO_MANIFEST_DIR` (a compile-time constant Cargo always
/// sets to this crate's own directory) rather than a relative string from
/// the workspace root. A relative path here is CWD-dependent: `cargo test`
/// runs a package's test binaries with the *package* directory as the
/// working directory, not the workspace root, so a plain
/// `"crates/ferrite-model/tests/fixtures/model"` silently resolves to a
/// nonexistent nested `crates/ferrite-model/crates/ferrite-model/...` path
/// the moment a test is run the ordinary way. Caught empirically while
/// generating the fixtures this directory now holds.
#[must_use]
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model")
}

/// One recorded round-trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    /// The content-addressed key — also the filename stem.
    pub key: String,
    /// Which live backend produced it.
    pub provider: ProviderId,
    /// The exact tag that answered.
    pub model_tag: String,
    /// Which tier the tag was configured as.
    pub tier: ModelTier,
    /// When it was recorded, from the injected [`Clock`].
    pub recorded_at: DateTime<Utc>,
    /// The request, stored for human auditability. It is *not* what the key
    /// is recomputed from at replay time — the lookup goes the other way —
    /// but a directory of opaque hashes that nobody can read is a directory
    /// nobody maintains.
    pub request: CompletionRequest,
    /// The provider's response body, verbatim.
    pub wire_response: serde_json::Value,
}

/// The path a fixture with `key` lives at.
#[must_use]
pub fn path(dir: &Path, key: &CacheKey) -> PathBuf {
    dir.join(format!("{key}.json"))
}

/// Loads the fixture for `key`.
///
/// # Errors
///
/// [`ModelError::ReplayMiss`] if there is no such fixture — §10.3: "a replay
/// miss is a hard failure with the missing key printed, never a silent
/// fallthrough to the network." Also a miss if the file is unreadable or
/// malformed: a fixture that cannot be parsed is not a fixture, and the fix
/// (re-record it) is the same.
pub fn load(dir: &Path, key: &CacheKey) -> Result<Fixture, ModelError> {
    let miss = || ModelError::ReplayMiss {
        key: key.as_str().to_string(),
        fixture_dir: dir.to_path_buf(),
    };
    let raw = std::fs::read_to_string(path(dir, key)).map_err(|_| miss())?;
    let fixture: Fixture = serde_json::from_str(&raw).map_err(|_| miss())?;
    if fixture.key != key.as_str() {
        return Err(miss());
    }
    Ok(fixture)
}

/// Writes a fixture for `req` as answered by `provider`, returning its path.
///
/// This is what `just record` calls once per live response.
///
/// # Errors
///
/// [`ModelError::Cache`] if the directory or file cannot be written.
pub fn write(
    dir: &Path,
    provider: ProviderId,
    req: &CompletionRequest,
    wire_response: serde_json::Value,
    clock: &dyn Clock,
) -> Result<PathBuf, ModelError> {
    let key = CacheKey::compute(provider, req);
    let fixture = Fixture {
        key: key.as_str().to_string(),
        provider,
        model_tag: req.model_tag.clone(),
        tier: req.tier,
        recorded_at: clock.now(),
        request: req.clone(),
        wire_response,
    };
    std::fs::create_dir_all(dir).map_err(|e| ModelError::Cache {
        path: dir.to_path_buf(),
        detail: e.to_string(),
    })?;
    let target = path(dir, &key);
    let body = serde_json::to_string_pretty(&fixture).map_err(|e| ModelError::Cache {
        path: target.clone(),
        detail: e.to_string(),
    })?;
    std::fs::write(&target, body).map_err(|e| ModelError::Cache {
        path: target.clone(),
        detail: e.to_string(),
    })?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use ferrite_core::FixedClock;

    use super::*;
    use crate::request::Message;
    use crate::testing::TempDir;

    fn req() -> CompletionRequest {
        CompletionRequest::new("tag:1b", ModelTier::Small, vec![Message::user("hi")])
    }

    #[test]
    fn fixture_dir_is_absolute_regardless_of_the_process_working_directory() {
        // A relative path here previously resolved wrong under `cargo test`,
        // which runs a package's test binaries from the package directory,
        // not the workspace root — a fixture written to what looked like
        // the right relative path landed in a nonexistent nested
        // `crates/ferrite-model/crates/ferrite-model/...` instead. Pinning
        // "absolute, and ends where the committed fixtures actually live"
        // is what would have caught that before it shipped.
        let dir = fixture_dir();
        assert!(dir.is_absolute(), "{dir:?} is not absolute");
        assert!(
            dir.ends_with("crates/ferrite-model/tests/fixtures/model"),
            "{dir:?} does not end where committed fixtures live"
        );
    }

    #[test]
    fn a_written_fixture_loads_back_identically() {
        let dir = TempDir::new("fixtures-roundtrip");
        let wire = serde_json::json!({"message": {"content": "[]"}});
        let written = write(
            dir.path(),
            ProviderId::Ollama,
            &req(),
            wire.clone(),
            &FixedClock::at_epoch(),
        )
        .expect("writes");

        let key = CacheKey::compute(ProviderId::Ollama, &req());
        assert_eq!(written, path(dir.path(), &key));

        let loaded = load(dir.path(), &key).expect("loads");
        assert_eq!(loaded.wire_response, wire);
        assert_eq!(loaded.provider, ProviderId::Ollama);
        assert_eq!(loaded.model_tag, "tag:1b");
        assert_eq!(loaded.recorded_at, DateTime::UNIX_EPOCH);
        assert_eq!(loaded.request, req());
    }

    #[test]
    fn a_missing_fixture_is_a_replay_miss_that_prints_the_key() {
        let dir = TempDir::new("fixtures-miss");
        let key = CacheKey::compute(ProviderId::Ollama, &req());
        let err = load(dir.path(), &key).expect_err("nothing recorded");
        match &err {
            ModelError::ReplayMiss { key: printed, .. } => assert_eq!(printed, key.as_str()),
            other => panic!("expected ReplayMiss, got {other:?}"),
        }
        assert!(err.to_string().contains(key.as_str()));
    }

    #[test]
    fn a_malformed_fixture_is_a_miss_not_a_crash() {
        let dir = TempDir::new("fixtures-corrupt");
        let key = CacheKey::compute(ProviderId::Ollama, &req());
        std::fs::write(path(dir.path(), &key), "{ not json").expect("write");
        let err = load(dir.path(), &key).expect_err("unparseable");
        assert!(matches!(err, ModelError::ReplayMiss { .. }), "{err:?}");
    }

    #[test]
    fn a_fixture_renamed_to_the_wrong_key_is_rejected() {
        // Same reasoning as the cache's filename check: a file copied to the
        // wrong name must be a miss, never a confident wrong answer.
        let dir = TempDir::new("fixtures-mismatch");
        write(
            dir.path(),
            ProviderId::Ollama,
            &req(),
            serde_json::json!({}),
            &FixedClock::at_epoch(),
        )
        .expect("writes");

        let real = CacheKey::compute(ProviderId::Ollama, &req());
        let wrong = CacheKey::from_hex("f".repeat(64));
        std::fs::rename(path(dir.path(), &real), path(dir.path(), &wrong)).expect("rename");

        assert!(matches!(
            load(dir.path(), &wrong).expect_err("mismatched"),
            ModelError::ReplayMiss { .. }
        ));
    }

    #[test]
    fn two_providers_recording_the_same_request_get_two_fixtures() {
        let dir = TempDir::new("fixtures-per-provider");
        for provider in [ProviderId::Ollama, ProviderId::Gemini] {
            write(
                dir.path(),
                provider,
                &req(),
                serde_json::json!({}),
                &FixedClock::at_epoch(),
            )
            .expect("writes");
        }
        let count = std::fs::read_dir(dir.path()).expect("readable").count();
        assert_eq!(
            count, 2,
            "the same prompt asked of two backends is two answers"
        );
    }
}
