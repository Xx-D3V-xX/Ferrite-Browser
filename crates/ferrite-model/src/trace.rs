//! Model-activity trace: what was sent to the LLM and to Laya, what came back,
//! and how long it took.
//!
//! Every provider call (through the [`Trace`](crate::decorators::Trace)
//! decorator) and every Laya request (inside [`LayaClient`](crate::laya::LayaClient))
//! is recorded as a [`TraceEvent`] in a bounded in-memory ring
//! ([`TraceLog`]) and, once [`TraceLog::set_file`] has been called, appended to
//! a JSON-lines file. The UI reads the ring to show the activity view and the
//! per-stage timing table ([`StageStats`]) that answers "is Laya actually
//! faster than the LLM here?".
//!
//! **Privacy.** A trace holds the prompts and page text the models received, so
//! the file is as sensitive as the pages themselves. It is written only under
//! the user's own data directory, never sent anywhere, and every text field is
//! cut to [`MAX_FIELD_CHARS`]. Nothing here influences a decision: recording
//! is best-effort and a failure to write is ignored.
//!
//! In tests the log is memory-only (no file is set), so no test touches disk.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest request/response text kept per event (characters).
pub const MAX_FIELD_CHARS: usize = 6_000;
/// Events kept in memory.
pub const RING_CAPACITY: usize = 500;
/// The JSONL file is rotated to `<name>.1` once it passes this size.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Which component an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceBackend {
    /// A language-model provider (Ollama, Gemini, ...).
    Llm,
    /// The local Laya server.
    Laya,
    /// The agent itself: run start/finish, actions it executed, consent
    /// decisions. Not a model call; recorded so the timeline reads end to end.
    Agent,
}

impl TraceBackend {
    /// Short display name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Llm => "LLM",
            Self::Laya => "Laya",
            Self::Agent => "Agent",
        }
    }
}

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Position in this process's sequence (assigned by [`TraceLog::record`]).
    pub seq: u64,
    /// Unix time in milliseconds (assigned by [`TraceLog::record`]).
    pub at_ms: u64,
    /// LLM or Laya.
    pub backend: TraceBackend,
    /// What the call was for, e.g. `"agent step"`, `"fingerprint"`, `"fast lane"`.
    pub stage: String,
    /// The model tag (or Laya checkpoint) that answered.
    pub model: String,
    /// What was sent (bounded).
    pub request: String,
    /// What came back, or the error text (bounded).
    pub response: String,
    /// Wall time of the call.
    pub latency_ms: u64,
    /// Whether the call itself succeeded (a Laya answer that the gates then
    /// rejected is still `ok`; the verdict is in `note`).
    pub ok: bool,
    /// Extra facts: cache hit, the gate verdict, the error class.
    pub note: String,
    /// Prompt tokens, when the backend reports them.
    pub tokens_in: Option<u32>,
    /// Completion tokens, when the backend reports them.
    pub tokens_out: Option<u32>,
    /// Served from the on-disk response cache (no model ran).
    pub cached: bool,
}

impl TraceEvent {
    /// A new event with empty text fields; fill in what applies.
    #[must_use]
    pub fn new(backend: TraceBackend, stage: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            seq: 0,
            at_ms: 0,
            backend,
            stage: stage.into(),
            model: model.into(),
            request: String::new(),
            response: String::new(),
            latency_ms: 0,
            ok: true,
            note: String::new(),
            tokens_in: None,
            tokens_out: None,
            cached: false,
        }
    }
}

/// Timing summary for one `(backend, stage)` pair.
#[derive(Debug, Clone, PartialEq)]
pub struct StageStats {
    /// LLM or Laya.
    pub backend: TraceBackend,
    /// The stage label.
    pub stage: String,
    /// Calls recorded (cache hits excluded from the latency figures).
    pub calls: usize,
    /// Calls that failed.
    pub failures: usize,
    /// Calls answered from the cache.
    pub cached: usize,
    /// Mean latency of the calls that ran, in ms.
    pub mean_ms: u64,
    /// Median latency of the calls that ran, in ms.
    pub median_ms: u64,
    /// Slowest call, in ms.
    pub max_ms: u64,
}

#[derive(Default)]
struct Inner {
    ring: VecDeque<TraceEvent>,
    next_seq: u64,
    file: Option<PathBuf>,
}

/// A bounded, thread-safe trace of model calls.
#[derive(Default)]
pub struct TraceLog {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for TraceLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceLog")
            .field("recorded", &self.total_recorded())
            .finish_non_exhaustive()
    }
}

impl TraceLog {
    /// An empty, memory-only log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while holding the lock must not silence tracing for good.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Also append every event to `path` as one JSON object per line.
    pub fn set_file(&self, path: PathBuf) {
        self.lock().file = Some(path);
    }

    /// Records `event`, assigning its sequence number and timestamp.
    pub fn record(&self, mut event: TraceEvent) {
        event.request = truncate(&event.request);
        event.response = truncate(&event.response);
        event.at_ms = now_ms();
        let mut inner = self.lock();
        event.seq = inner.next_seq;
        inner.next_seq += 1;
        if let Some(path) = inner.file.clone() {
            append_line(&path, &event);
        }
        if inner.ring.len() == RING_CAPACITY {
            inner.ring.pop_front();
        }
        inner.ring.push_back(event);
    }

    /// Forgets the events held in memory (the file, if any, is left alone).
    pub fn clear(&self) {
        self.lock().ring.clear();
    }

    /// The events currently held, oldest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<TraceEvent> {
        self.lock().ring.iter().cloned().collect()
    }

    /// Number of events recorded since the process started.
    #[must_use]
    pub fn total_recorded(&self) -> u64 {
        self.lock().next_seq
    }

    /// Per-`(backend, stage)` timing over the events currently held.
    #[must_use]
    pub fn stats(&self) -> Vec<StageStats> {
        stats_of(&self.snapshot())
    }
}

/// Timing summary over `events` (see [`TraceLog::stats`]).
#[must_use]
pub fn stats_of(events: &[TraceEvent]) -> Vec<StageStats> {
    let mut groups: BTreeMap<(TraceBackend, String), Vec<&TraceEvent>> = BTreeMap::new();
    for event in events {
        groups
            .entry((event.backend, event.stage.clone()))
            .or_default()
            .push(event);
    }
    groups
        .into_iter()
        .map(|((backend, stage), group)| {
            let mut ran: Vec<u64> = group
                .iter()
                .filter(|e| !e.cached)
                .map(|e| e.latency_ms)
                .collect();
            ran.sort_unstable();
            let mean_ms = if ran.is_empty() {
                0
            } else {
                ran.iter().sum::<u64>() / ran.len() as u64
            };
            StageStats {
                backend,
                stage,
                calls: group.len(),
                failures: group.iter().filter(|e| !e.ok).count(),
                cached: group.iter().filter(|e| e.cached).count(),
                mean_ms,
                median_ms: ran.get(ran.len() / 2).copied().unwrap_or(0),
                max_ms: ran.last().copied().unwrap_or(0),
            }
        })
        .collect()
}

/// The process-wide log every recorder in this workspace writes to.
#[must_use]
pub fn global() -> Arc<TraceLog> {
    static LOG: OnceLock<Arc<TraceLog>> = OnceLock::new();
    LOG.get_or_init(|| Arc::new(TraceLog::new())).clone()
}

/// `text` cut to [`MAX_FIELD_CHARS`] characters, saying how much was dropped.
#[must_use]
pub fn truncate(text: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_FIELD_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_FIELD_CHARS).collect();
    format!(
        "{kept}\n… [{} more characters not kept]",
        total - MAX_FIELD_CHARS
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn append_line(path: &PathBuf, event: &TraceEvent) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
        let mut rotated = path.clone().into_os_string();
        rotated.push(".1");
        let _ = std::fs::rename(path, rotated);
    }
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(backend: TraceBackend, stage: &str, ms: u64) -> TraceEvent {
        let mut e = TraceEvent::new(backend, stage, "m");
        e.latency_ms = ms;
        e
    }

    #[test]
    fn record_assigns_increasing_sequence_numbers_and_a_timestamp() {
        let log = TraceLog::new();
        log.record(event(TraceBackend::Llm, "a", 1));
        log.record(event(TraceBackend::Llm, "a", 2));
        let events = log.snapshot();
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[1].seq, 1);
        assert!(events[0].at_ms > 0);
        assert_eq!(log.total_recorded(), 2);
    }

    #[test]
    fn the_ring_is_bounded_and_drops_the_oldest() {
        let log = TraceLog::new();
        for i in 0..(RING_CAPACITY as u64 + 10) {
            log.record(event(TraceBackend::Llm, "a", i));
        }
        let events = log.snapshot();
        assert_eq!(events.len(), RING_CAPACITY);
        assert_eq!(events[0].seq, 10);
        assert_eq!(log.total_recorded(), RING_CAPACITY as u64 + 10);
    }

    #[test]
    fn clear_empties_the_ring_but_keeps_counting() {
        let log = TraceLog::new();
        log.record(event(TraceBackend::Llm, "a", 1));
        log.clear();
        assert!(log.snapshot().is_empty());
        log.record(event(TraceBackend::Llm, "a", 1));
        assert_eq!(log.snapshot()[0].seq, 1, "sequence numbers do not restart");
    }

    #[test]
    fn long_text_is_cut_and_says_how_much_was_dropped() {
        let log = TraceLog::new();
        let mut e = event(TraceBackend::Llm, "a", 1);
        e.request = "x".repeat(MAX_FIELD_CHARS + 25);
        log.record(e);
        let request = &log.snapshot()[0].request;
        assert!(request.contains("25 more characters not kept"));
        assert!(request.chars().count() < MAX_FIELD_CHARS + 60);
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let text = "é".repeat(MAX_FIELD_CHARS + 3);
        let cut = truncate(&text);
        assert!(cut.starts_with('é') && cut.contains("3 more characters"));
    }

    #[test]
    fn stats_group_by_backend_and_stage_and_ignore_cache_hits_for_timing() {
        let mut cached = event(TraceBackend::Llm, "step", 0);
        cached.cached = true;
        let mut failed = event(TraceBackend::Llm, "step", 400);
        failed.ok = false;
        let events = vec![
            event(TraceBackend::Llm, "step", 1000),
            event(TraceBackend::Llm, "step", 3000),
            cached,
            failed,
            event(TraceBackend::Laya, "fast lane", 20),
            event(TraceBackend::Laya, "fast lane", 40),
        ];
        let stats = stats_of(&events);
        let llm = stats
            .iter()
            .find(|s| s.backend == TraceBackend::Llm)
            .unwrap();
        assert_eq!((llm.calls, llm.failures, llm.cached), (4, 1, 1));
        assert_eq!(llm.mean_ms, (400 + 1000 + 3000) / 3);
        assert_eq!(llm.median_ms, 1000);
        assert_eq!(llm.max_ms, 3000);
        let laya = stats
            .iter()
            .find(|s| s.backend == TraceBackend::Laya)
            .unwrap();
        assert_eq!((laya.calls, laya.mean_ms, laya.max_ms), (2, 30, 40));
    }

    #[test]
    fn a_file_receives_one_json_line_per_event() {
        let dir = std::env::temp_dir().join(format!("ferrite-trace-{}", std::process::id()));
        let path = dir.join("nested").join("activity.jsonl");
        let log = TraceLog::new();
        log.set_file(path.clone());
        log.record(event(TraceBackend::Laya, "fast lane", 7));
        log.record(event(TraceBackend::Llm, "step", 9));
        let text = std::fs::read_to_string(&path).expect("file written");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let back: TraceEvent = serde_json::from_str(lines[0]).expect("valid json");
        assert_eq!(back.backend, TraceBackend::Laya);
        assert_eq!(back.latency_ms, 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_directory_or_unwritable_path_never_panics() {
        let log = TraceLog::new();
        log.set_file(PathBuf::from("/proc/definitely/not/writable/x.jsonl"));
        log.record(event(TraceBackend::Llm, "a", 1));
        assert_eq!(log.snapshot().len(), 1);
    }
}
