//! The content-addressed response cache — §10.3's first and biggest
//! rate-limit mechanism.
//!
//! The arithmetic it exists to fix: ~900 executions per eval run at 3–5
//! calls each is 3,000–4,500 live calls, which exhausts the quota on the
//! first attempt. The fingerprint call is identical across all four defense
//! modes for a given case, so 900 of them collapse to ~360; a second run of
//! an unchanged corpus costs zero calls.
//!
//! **On disk, not in memory.** A process-local map would give a hit rate of
//! zero on the second run, which is the run that matters. Each entry is one
//! file named by its [`CacheKey`] under `~/.cache/ferrite-model/`, written
//! by rename so a killed process leaves either the old entry or the new one
//! and never a half-written file.
//!
//! **Sound, not a fudge.** At `temperature = 0` a response is a pure
//! function of the key. Above it, it is one draw from a distribution, and
//! memoizing one draw is not caching — so an uncacheable request passes
//! straight through and is counted separately.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrite_core::Clock;
use serde::{Deserialize, Serialize};

use crate::cache_key::CacheKey;
use crate::error::ModelError;
use crate::provider::{ModelProvider, ProviderCapabilities, ProviderId};
use crate::request::CompletionRequest;
use crate::response::CompletionResponse;

/// One cached response, as it sits on disk.
///
/// The key is stored inside the entry as well as being the filename so that
/// a directory of fixtures can be audited without recomputing anything, and
/// so a file renamed by hand is detectably wrong rather than silently
/// serving the wrong answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The content-addressed key this entry answers.
    pub key: String,
    /// When it was written, from the injected [`Clock`] (R8 — never
    /// `Utc::now()` reached for directly).
    pub created_at: DateTime<Utc>,
    /// The response.
    pub response: CompletionResponse,
}

/// Cumulative counters for a cache, reported by `just cache-stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheStatsSnapshot {
    /// Requests served from disk.
    pub hits: u64,
    /// Requests that had to go to the wrapped provider.
    pub misses: u64,
    /// Entries written.
    pub writes: u64,
    /// Requests that bypassed the cache because `temperature != 0`.
    pub uncacheable: u64,
}

impl CacheStatsSnapshot {
    /// Hits as a fraction of cacheable requests, or `None` when there were
    /// none. §10.3: "a low hit rate is a bug, investigate it."
    #[must_use]
    pub fn hit_rate(&self) -> Option<f64> {
        let considered = self.hits + self.misses;
        if considered == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        Some(self.hits as f64 / considered as f64)
    }
}

/// Live counters.
#[derive(Debug, Default)]
struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    writes: AtomicU64,
    uncacheable: AtomicU64,
}

impl CacheStats {
    fn snapshot(&self) -> CacheStatsSnapshot {
        CacheStatsSnapshot {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            uncacheable: self.uncacheable.load(Ordering::Relaxed),
        }
    }
}

/// The filename `just cache-stats` reads. Prefixed so it can never collide
/// with a 64-hex-character entry name.
pub const STATS_FILENAME: &str = "_stats.json";

/// What `just cache-stats` reports about a cache directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheDirReport {
    /// Where the cache lives.
    pub dir: PathBuf,
    /// How many cached responses are on disk.
    pub entries: u64,
    /// Their total size.
    pub bytes: u64,
    /// The counters last flushed by a run, if any run has flushed them.
    pub last_session: Option<CacheStatsSnapshot>,
}

/// Inspects a cache directory without needing a provider — what the CLI's
/// `cache-stats` subcommand calls.
///
/// # Errors
///
/// [`ModelError::Cache`] if the directory exists but cannot be read.
pub fn report(dir: &Path) -> Result<CacheDirReport, ModelError> {
    let mut entries = 0;
    let mut bytes = 0;
    if dir.is_dir() {
        let read = std::fs::read_dir(dir).map_err(|e| cache_err(dir, &e))?;
        for entry in read {
            let entry = entry.map_err(|e| cache_err(dir, &e))?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && path.file_name().is_some_and(|n| n != STATS_FILENAME)
            {
                entries += 1;
                bytes += entry.metadata().map_err(|e| cache_err(&path, &e))?.len();
            }
        }
    }
    let last_session = std::fs::read_to_string(dir.join(STATS_FILENAME))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(CacheDirReport {
        dir: dir.to_path_buf(),
        entries,
        bytes,
        last_session,
    })
}

fn cache_err(path: &Path, e: &dyn std::fmt::Display) -> ModelError {
    ModelError::Cache {
        path: path.to_path_buf(),
        detail: e.to_string(),
    }
}

/// Wraps any provider with the on-disk response cache.
#[derive(Debug)]
pub struct Cache<P: ModelProvider> {
    inner: P,
    dir: PathBuf,
    clock: Arc<dyn Clock>,
    stats: CacheStats,
    tmp_counter: AtomicU64,
}

impl<P: ModelProvider> Cache<P> {
    /// Wraps `inner`, storing entries under `dir`.
    #[must_use]
    pub fn new(inner: P, dir: impl Into<PathBuf>, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner,
            dir: dir.into(),
            clock,
            stats: CacheStats::default(),
            tmp_counter: AtomicU64::new(0),
        }
    }

    /// This process's counters so far.
    #[must_use]
    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }

    /// The cache directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes this run's counters where `just cache-stats` can find them.
    ///
    /// Called at exit rather than on every operation: a read-modify-write
    /// per call would cost more IO than the cache saves, and last-writer-wins
    /// across concurrent runs is acceptable for a diagnostic counter in a way
    /// it would never be for an entry.
    ///
    /// # Errors
    ///
    /// [`ModelError::Cache`] if the file cannot be written.
    pub fn flush_stats(&self) -> Result<(), ModelError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| cache_err(&self.dir, &e))?;
        let path = self.dir.join(STATS_FILENAME);
        let body = serde_json::to_string_pretty(&self.stats()).map_err(|e| cache_err(&path, &e))?;
        std::fs::write(&path, body).map_err(|e| cache_err(&path, &e))
    }

    fn entry_path(&self, key: &CacheKey) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// A hit, or `None`. A corrupt entry counts as a miss and is removed:
    /// the cache is a derived artifact, so the repair for an unreadable one
    /// is to re-derive it, not to fail the run.
    fn read(&self, key: &CacheKey) -> Option<CompletionResponse> {
        let path = self.entry_path(key);
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<CacheEntry>(&raw) {
            Ok(entry) if entry.key == key.as_str() => Some(entry.response),
            _ => {
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Writes by rename, so a reader never sees a partial file.
    fn write(&self, key: &CacheKey, response: &CompletionResponse) -> Result<(), ModelError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| cache_err(&self.dir, &e))?;
        let entry = CacheEntry {
            key: key.as_str().to_string(),
            created_at: self.clock.now(),
            response: response.clone(),
        };
        let body = serde_json::to_string_pretty(&entry).map_err(|e| cache_err(&self.dir, &e))?;

        let seq = self.tmp_counter.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .dir
            .join(format!(".{key}.{}.{seq}.tmp", std::process::id()));
        std::fs::write(&tmp, body).map_err(|e| cache_err(&tmp, &e))?;
        let final_path = self.entry_path(key);
        std::fs::rename(&tmp, &final_path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            cache_err(&final_path, &e)
        })?;
        self.stats.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[async_trait]
impl<P: ModelProvider> ModelProvider for Cache<P> {
    fn id(&self) -> ProviderId {
        // The wrapped provider's identity, not a synthetic "cache" one: a
        // decorator must be transparent, and §10.2's provenance requirement
        // is about which *model* answered.
        self.inner.id()
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        if !req.is_cacheable() {
            self.stats.uncacheable.fetch_add(1, Ordering::Relaxed);
            return self.inner.complete(req).await;
        }

        let key = CacheKey::compute(self.inner.id(), &req);
        if let Some(hit) = self.read(&key) {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            let mut hit = hit;
            hit.provenance.cache_hit = true;
            return Ok(hit);
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        let response = self.inner.complete(req).await?;
        // A cache write failure is logged in the returned value's absence
        // from disk, not by failing the call: the caller already has the
        // answer it asked for, and losing a cache entry is not losing data.
        let _ = self.write(&key, &response);
        Ok(response)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
}

#[cfg(test)]
mod tests {
    use ferrite_core::FixedClock;

    use super::*;
    use crate::backends::MockProvider;
    use crate::provider::ModelTier;
    use crate::request::{Message, SamplingOptions};
    use crate::testing::TempDir;

    fn req() -> CompletionRequest {
        CompletionRequest::new("tag:1b", ModelTier::Small, vec![Message::user("hello")])
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(FixedClock::at_epoch())
    }

    #[tokio::test]
    async fn a_second_identical_call_hits_disk_and_the_provider_is_called_once() {
        // A3's exit gate, literally: "the response cache demonstrably
        // returns a hit on the second identical call (test asserts the
        // underlying HTTP client was called once)".
        let dir = TempDir::new("cache-hit");
        let inner = Arc::new(MockProvider::new().always_content("the answer"));
        let cache = Cache::new(Arc::clone(&inner), dir.path(), clock());

        let first = cache.complete(req()).await.expect("miss then call");
        let second = cache.complete(req()).await.expect("hit");

        assert_eq!(
            inner.call_count(),
            1,
            "the second call must not reach the provider"
        );
        assert_eq!(first.content, second.content);
        assert!(!first.provenance.cache_hit);
        assert!(second.provenance.cache_hit);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().writes, 1);
    }

    #[tokio::test]
    async fn the_cache_survives_a_process_restart() {
        // The whole point of being on disk (§10.3: "a second full run of an
        // unchanged corpus costs zero calls"). Two independent Cache values
        // over one directory stand in for two runs.
        let dir = TempDir::new("cache-restart");
        let first_run = Arc::new(MockProvider::new().always_content("persisted"));
        Cache::new(Arc::clone(&first_run), dir.path(), clock())
            .complete(req())
            .await
            .expect("populates");

        let second_run = Arc::new(MockProvider::new());
        let cache = Cache::new(Arc::clone(&second_run), dir.path(), clock());
        let hit = cache.complete(req()).await.expect("served from disk");

        assert_eq!(hit.content, "persisted");
        assert_eq!(
            second_run.call_count(),
            0,
            "an unchanged second run must cost zero calls"
        );
    }

    #[tokio::test]
    async fn a_different_request_is_a_different_entry() {
        let dir = TempDir::new("cache-distinct");
        let inner = Arc::new(MockProvider::new().always_content("x"));
        let cache = Cache::new(Arc::clone(&inner), dir.path(), clock());

        cache.complete(req()).await.expect("first");
        let mut other = req();
        other.messages = vec![Message::user("a different question")];
        cache.complete(other).await.expect("second");

        assert_eq!(inner.call_count(), 2);
        assert_eq!(report(dir.path()).expect("readable").entries, 2);
    }

    #[tokio::test]
    async fn a_sampled_request_bypasses_the_cache_entirely() {
        let dir = TempDir::new("cache-uncacheable");
        let inner = Arc::new(MockProvider::new().always_content("a draw"));
        let cache = Cache::new(Arc::clone(&inner), dir.path(), clock());

        let hot = req().with_options(SamplingOptions {
            temperature: 0.9,
            ..SamplingOptions::default()
        });
        cache.complete(hot.clone()).await.expect("first");
        cache.complete(hot).await.expect("second");

        assert_eq!(inner.call_count(), 2, "memoizing one draw is not caching");
        assert_eq!(cache.stats().uncacheable, 2);
        assert_eq!(report(dir.path()).expect("readable").entries, 0);
    }

    #[tokio::test]
    async fn a_failed_call_is_not_cached() {
        let dir = TempDir::new("cache-error");
        let inner = Arc::new(
            MockProvider::new()
                .push_error(ModelError::EmptyResponse {
                    provider: ProviderId::Mock,
                })
                .always_content("recovered"),
        );
        let cache = Cache::new(Arc::clone(&inner), dir.path(), clock());

        cache.complete(req()).await.expect_err("first call fails");
        let second = cache.complete(req()).await.expect("retry succeeds");

        assert_eq!(second.content, "recovered");
        assert_eq!(inner.call_count(), 2, "a failure must not be memoized");
    }

    #[tokio::test]
    async fn a_corrupt_entry_is_treated_as_a_miss_and_repaired() {
        let dir = TempDir::new("cache-corrupt");
        let inner = Arc::new(MockProvider::new().always_content("fresh"));
        let cache = Cache::new(Arc::clone(&inner), dir.path(), clock());
        cache.complete(req()).await.expect("populates");

        let key = CacheKey::compute(ProviderId::Mock, &req());
        std::fs::write(dir.path().join(format!("{key}.json")), "{ not json").expect("corrupt it");

        let recovered = cache.complete(req()).await.expect("re-derived");
        assert_eq!(recovered.content, "fresh");
        assert_eq!(inner.call_count(), 2);
    }

    #[tokio::test]
    async fn an_entry_whose_filename_does_not_match_its_key_is_rejected() {
        // Guards against a fixture copied to the wrong name serving a
        // confidently wrong answer.
        let dir = TempDir::new("cache-mismatch");
        let inner = Arc::new(MockProvider::new().always_content("correct"));
        let cache = Cache::new(Arc::clone(&inner), dir.path(), clock());
        cache.complete(req()).await.expect("populates");

        let key = CacheKey::compute(ProviderId::Mock, &req());
        let path = dir.path().join(format!("{key}.json"));
        let mut entry: CacheEntry =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        entry.key = "0".repeat(64);
        std::fs::write(&path, serde_json::to_string(&entry).expect("ser")).expect("write");

        cache.complete(req()).await.expect("re-derived");
        assert_eq!(inner.call_count(), 2);
    }

    #[tokio::test]
    async fn no_temporary_file_is_left_behind() {
        let dir = TempDir::new("cache-tmp");
        let cache = Cache::new(MockProvider::new().always_content("x"), dir.path(), clock());
        cache.complete(req()).await.expect("ok");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("readable")
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[tokio::test]
    async fn the_entry_timestamp_comes_from_the_injected_clock() {
        let dir = TempDir::new("cache-clock");
        let fixed = Arc::new(FixedClock::new(
            DateTime::UNIX_EPOCH + chrono::TimeDelta::days(9),
        ));
        let cache = Cache::new(
            MockProvider::new().always_content("x"),
            dir.path(),
            fixed as Arc<dyn Clock>,
        );
        cache.complete(req()).await.expect("ok");

        let key = CacheKey::compute(ProviderId::Mock, &req());
        let raw = std::fs::read_to_string(dir.path().join(format!("{key}.json"))).expect("read");
        let entry: CacheEntry = serde_json::from_str(&raw).expect("parse");
        assert_eq!(
            entry.created_at,
            DateTime::UNIX_EPOCH + chrono::TimeDelta::days(9),
            "R8: no wall clock reaches a stored artifact"
        );
    }

    #[test]
    fn a_report_over_a_directory_that_does_not_exist_is_empty_not_an_error() {
        let report = report(Path::new("/nonexistent/ferrite-model-cache")).expect("not an error");
        assert_eq!(report.entries, 0);
        assert_eq!(report.bytes, 0);
        assert_eq!(report.last_session, None);
    }

    #[tokio::test]
    async fn flushed_stats_are_readable_by_a_later_report() {
        let dir = TempDir::new("cache-stats");
        let cache = Cache::new(MockProvider::new().always_content("x"), dir.path(), clock());
        cache.complete(req()).await.expect("miss");
        cache.complete(req()).await.expect("hit");
        cache.flush_stats().expect("flushes");

        let report = report(dir.path()).expect("readable");
        let session = report.last_session.expect("a run flushed its counters");
        assert_eq!(session.hits, 1);
        assert_eq!(session.misses, 1);
        assert_eq!(
            report.entries, 1,
            "the stats file itself is not counted as a cached response"
        );
    }

    #[test]
    fn a_hit_rate_is_reported_only_once_there_is_something_to_rate() {
        assert_eq!(CacheStatsSnapshot::default().hit_rate(), None);
        let half = CacheStatsSnapshot {
            hits: 3,
            misses: 1,
            ..CacheStatsSnapshot::default()
        };
        assert_eq!(half.hit_rate(), Some(0.75));
    }
}
