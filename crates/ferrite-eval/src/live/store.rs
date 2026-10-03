//! The append-only result store: JSONL, one [`LiveRecord`] per line.
//!
//! # Crash safety
//!
//! A record is written whole, then flushed and synced, before the runner starts
//! the next case, so a crash (or a Ctrl-C, or a laptop closing) loses at most the
//! case in flight. A process killed in the middle of a write can leave a torn last
//! line; reading skips any line that does not parse (and says how many), and the
//! next append starts on a fresh line so the fragment cannot swallow a good record.
//!
//! # Resume
//!
//! The store is append-only: a retried case adds a line, it never rewrites one.
//! [`Results`] keeps the **latest** line per [`run_key`](super::record::run_key),
//! so a case redone with `--retry-failed` replaces its failed predecessor in every
//! report without anyone editing a file.
//!
//! # Layout
//!
//! `<out>/results/<provider>--<small>--<main>.jsonl`: one file per provider and
//! model pair, so two runs against different models never share a file (and
//! never interleave appends). Names are slugged; the identity of a line is the
//! fields inside it, not the file name.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use super::record::LiveRecord;

/// What reading a result file found.
#[derive(Debug, Clone, Default)]
pub struct Results {
    /// The latest record per run key.
    pub by_key: BTreeMap<String, LiveRecord>,
    /// Lines that did not parse (a torn write, or a file from another tool).
    pub unreadable_lines: usize,
    /// Lines from an older schema, skipped rather than misread.
    pub other_schema_lines: usize,
}

impl Results {
    /// Whether a result (success or failure) is stored for `key`.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    /// Whether the stored result for `key` is a success.
    #[must_use]
    pub fn succeeded(&self, key: &str) -> bool {
        self.by_key.get(key).is_some_and(LiveRecord::succeeded)
    }

    /// How many stored results are failures.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.by_key.values().filter(|r| !r.succeeded()).count()
    }

    /// Absorbs `other`, keeping the later of two lines for a key (`other` wins a tie
    /// only if its start time is not earlier).
    pub fn merge(&mut self, other: Results) {
        self.unreadable_lines += other.unreadable_lines;
        self.other_schema_lines += other.other_schema_lines;
        for (key, record) in other.by_key {
            match self.by_key.get(&key) {
                Some(existing) if existing.started_at > record.started_at => {}
                _ => {
                    self.by_key.insert(key, record);
                }
            }
        }
    }
}

/// `provider--small--main.jsonl`, with anything outside `[A-Za-z0-9._-]` in a tag
/// replaced by `_` so a tag like `models/x:1b` cannot escape the directory.
#[must_use]
pub fn file_name(provider: &str, small: &str, main: &str) -> String {
    let slug = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!("{}--{}--{}.jsonl", slug(provider), slug(small), slug(main))
}

/// The directory a store lives in under `out`.
#[must_use]
pub fn results_dir(out: &Path) -> PathBuf {
    out.join("results")
}

/// Reads one result file. A missing file is an empty result set.
///
/// # Errors
///
/// An I/O error other than the file not existing.
pub fn read_file(path: &Path) -> std::io::Result<Results> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Results::default()),
        Err(e) => return Err(e),
    };
    let mut results = Results::default();
    for line in BufReader::new(file).split(b'\n') {
        let bytes = line?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<LiveRecord>(&bytes) {
            Ok(record) if record.schema == super::record::SCHEMA_VERSION => {
                results.by_key.insert(record.run_key.clone(), record);
            }
            Ok(_) => results.other_schema_lines += 1,
            Err(_) => results.unreadable_lines += 1,
        }
    }
    Ok(results)
}

/// Reads every `*.jsonl` under `<out>/results`.
///
/// # Errors
///
/// An I/O error reading the directory or a file.
pub fn read_all(out: &Path) -> std::io::Result<Results> {
    let dir = results_dir(out);
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    files.sort();
    let mut all = Results::default();
    for file in files {
        all.merge(read_file(&file)?);
    }
    Ok(all)
}

/// An open result file, appended to.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    file: File,
}

impl Store {
    /// Opens (creating directories and the file as needed) the result file for a
    /// provider and model pair, and reads what is already in it.
    ///
    /// # Errors
    ///
    /// Any I/O error creating or reading the file.
    pub fn open(out: &Path, provider: &str, small: &str, main: &str) -> std::io::Result<(Self, Results)> {
        let dir = results_dir(out);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file_name(provider, small, main));
        let existing = read_file(&path)?;
        let ends_with_newline = match std::fs::read(&path) {
            Ok(bytes) => bytes.last().is_none_or(|b| *b == b'\n'),
            Err(_) => true,
        };
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        if !ends_with_newline {
            // A torn last line: end it, so the next record starts on its own line.
            file.write_all(b"\n")?;
        }
        Ok((Self { path, file }, existing))
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one record and makes it durable before returning.
    ///
    /// # Errors
    ///
    /// Any I/O or serialization error. The caller must treat one as fatal: a
    /// result that cannot be stored is a result that will be paid for twice.
    pub fn append(&mut self, record: &LiveRecord) -> std::io::Result<()> {
        let mut line = record
            .to_line()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        // One write call for the whole line, so a crash tears at most this line.
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.file.sync_data()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::config::{AgentKind, LiveMode, PredictorKind};
    use crate::live::record::{CallCounts, Outcome, SCHEMA_VERSION};

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrite-live-store-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn record(key: &str, started: &str, error: bool) -> LiveRecord {
        LiveRecord {
            schema: SCHEMA_VERSION,
            run_key: key.to_string(),
            case_id: key.split('|').next().unwrap().to_string(),
            suite: "s".to_string(),
            source: "src".to_string(),
            kind: "attack".to_string(),
            ground_truth: "deviation".to_string(),
            attack_category: None,
            carrier: "tool_output".to_string(),
            provider: "mock".to_string(),
            small_model: "s".to_string(),
            main_model: "m".to_string(),
            mode: LiveMode::Guard,
            agent: AgentKind::Llm,
            predictor: PredictorKind::Llm,
            config_hash: "h".to_string(),
            started_at: started.to_string(),
            prediction: None,
            ideal_capabilities: None,
            attack_capabilities: None,
            actions: Vec::new(),
            stop_reason: "finished".to_string(),
            final_answer: None,
            outcome: Outcome::default(),
            latency_ms: 1,
            calls: CallCounts::default(),
            error: error.then(|| crate::live::record::RecordError {
                class: "rate_limited".to_string(),
                message: "slow down".to_string(),
                retryable: true,
            }),
        }
    }

    #[test]
    fn an_appended_record_reads_back_and_a_reopen_sees_it() {
        let out = temp("roundtrip");
        let (mut store, existing) = Store::open(&out, "mock", "s", "m").unwrap();
        assert!(existing.by_key.is_empty());
        store.append(&record("c1|k", "2026-01-01T00:00:00Z", false)).unwrap();
        store.append(&record("c2|k", "2026-01-01T00:00:01Z", false)).unwrap();
        drop(store);
        let (_, again) = Store::open(&out, "mock", "s", "m").unwrap();
        assert_eq!(again.by_key.len(), 2);
        assert!(again.succeeded("c1|k"));
    }

    #[test]
    fn a_retried_case_replaces_its_failure_without_rewriting_the_file() {
        let out = temp("retry");
        let (mut store, _) = Store::open(&out, "mock", "s", "m").unwrap();
        store.append(&record("c1|k", "2026-01-01T00:00:00Z", true)).unwrap();
        let before = std::fs::read_to_string(store.path()).unwrap();
        store.append(&record("c1|k", "2026-01-01T00:05:00Z", false)).unwrap();
        let after = std::fs::read_to_string(store.path()).unwrap();
        assert!(after.starts_with(&before), "append-only: the old line is untouched");
        let results = read_file(store.path()).unwrap();
        assert_eq!(results.by_key.len(), 1);
        assert!(results.succeeded("c1|k"), "the latest line wins");
        assert_eq!(results.failed(), 0);
    }

    #[test]
    fn a_torn_last_line_loses_only_that_line_and_does_not_poison_the_next() {
        let out = temp("torn");
        let (mut store, _) = Store::open(&out, "mock", "s", "m").unwrap();
        store.append(&record("c1|k", "2026-01-01T00:00:00Z", false)).unwrap();
        let path = store.path().to_path_buf();
        drop(store);
        // A crash mid-write: half a JSON object, no newline.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"schema":1,"run_key":"c2|k","case_id":"c2","sui"#).unwrap();
        drop(f);

        let read = read_file(&path).unwrap();
        assert_eq!(read.by_key.len(), 1, "the good record survives");
        assert_eq!(read.unreadable_lines, 1);

        let (mut store, existing) = Store::open(&out, "mock", "s", "m").unwrap();
        assert_eq!(existing.by_key.len(), 1);
        store.append(&record("c3|k", "2026-01-01T00:00:09Z", false)).unwrap();
        let read = read_file(&path).unwrap();
        assert!(read.has("c1|k") && read.has("c3|k"), "the next record is intact");
        assert!(!read.has("c2|k"));
    }

    #[test]
    fn a_line_from_another_schema_is_skipped_not_misread() {
        let out = temp("schema");
        let (mut store, _) = Store::open(&out, "mock", "s", "m").unwrap();
        let mut old = record("c1|k", "2026-01-01T00:00:00Z", false);
        old.schema = SCHEMA_VERSION + 1;
        store.append(&old).unwrap();
        let read = read_file(store.path()).unwrap();
        assert!(read.by_key.is_empty());
        assert_eq!(read.other_schema_lines, 1);
    }

    #[test]
    fn file_names_cannot_escape_the_results_directory() {
        let name = file_name("ollama", "../../etc/passwd", "models/x:1b");
        assert!(!name.contains('/') && !name.contains(".."), "{name}");
        assert!(name.ends_with(".jsonl"));
    }

    #[test]
    fn two_models_never_share_a_file_and_read_all_merges_them() {
        let out = temp("two");
        let (mut a, _) = Store::open(&out, "mock", "s", "m1").unwrap();
        let (mut b, _) = Store::open(&out, "mock", "s", "m2").unwrap();
        assert_ne!(a.path(), b.path());
        a.append(&record("c1|a", "2026-01-01T00:00:00Z", false)).unwrap();
        b.append(&record("c1|b", "2026-01-01T00:00:00Z", false)).unwrap();
        assert_eq!(read_all(&out).unwrap().by_key.len(), 2);
    }

    #[test]
    fn reading_a_missing_directory_is_empty_not_an_error() {
        let out = temp("missing").join("nope");
        assert!(read_all(&out).unwrap().by_key.is_empty());
    }
}
