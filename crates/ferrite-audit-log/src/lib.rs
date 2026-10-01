//! Hash-chained audit log.
//!
//! ## What this fixes (T-005 / T-108)
//!
//! The pre-A8 hash preimage was
//! `format!("{}{}{:?}{}{}", sequence, timestamp.to_rfc3339(), kind,
//! principal_id, prev_hash)` — it omitted `capability` and `url` entirely,
//! so those two columns (where `EvalExecutionRecorded` entries carry their
//! `exec_id`/`case_id` payload — see [`AuditEventKind::EvalExecutionRecorded`]'s
//! doc comment) could be rewritten directly in SQLite with
//! [`AuditLog::verify_chain`] still reporting the chain intact. That
//! directly contradicted the audit log's reason for existing: making the
//! containment decision verifiable after the fact.
//!
//! The fix has three parts, all inside [`compute_entry_hash`] — the single
//! function [`AuditLog::append`] and [`AuditLog::verify_chain`] both call,
//! so the two can never drift apart the way the old duplicated `format!`
//! strings implicitly could:
//!
//! 1. **Full coverage.** Every field persisted in the `audit_entries` table
//!    — including `entry_id` and the new `schema_version` — is part of the
//!    preimage. Nothing independently mutable in SQLite sits outside the
//!    hash.
//! 2. **Canonical serialization, not `Debug` formatting.** [`HashPreimage`]
//!    is serialized with `serde_json`, field-by-field, in the struct's
//!    fixed declared order (never a `HashMap`, so there is no key-ordering
//!    ambiguity). This replaces naive `format!("{}{}{:?}...", ...)`
//!    concatenation, which had two real ambiguity risks: (a) `{:?}` on an
//!    enum is not a stable wire format across Rust/derive versions, and (b)
//!    concatenating `Option<String>` fields with no separator or
//!    length-prefixing can make two different `(capability, url)` pairs
//!    hash identically once either field can be empty or absent — nothing
//!    in the old scheme made `None` and `Some(String::new())`
//!    distinguishable other than incidental `Display` behavior. JSON fixes
//!    this structurally: it distinguishes `null` from `""` and
//!    length-delimits every string, so `None` and `Some(String::new())` for
//!    `capability`/`url` are byte-distinguishable in the preimage, and
//!    always will be.
//! 3. **Domain separation + schema versioning.** The preimage is prefixed
//!    with the fixed constant [`HASH_DOMAIN`] (`b"ferrite-audit-v1"`), so a
//!    SHA-256 collision/reuse attack against some other part of the
//!    codebase that also hashes similar-shaped JSON cannot be replayed
//!    here. [`AUDIT_SCHEMA_VERSION`] is itself hashed, so a future change to
//!    what gets hashed changes what an old, correctly-verifying entry
//!    *means* — it does not silently reinterpret old entries under new
//!    rules, and a verifier can branch on the entry's own stored
//!    `schema_version` byte instead of every historical entry going dark
//!    with no explanation.
//!
//! ## What this does not fix
//!
//! Per `docs/AUDIT.md`'s D5 entry, this session fixes the *preimage*, not
//! the chain *structure* — SHA-256, sequence-linking, and SQLite
//! persistence all carry forward unchanged. One structural gap the tamper
//! matrix surfaced anyway: a hash chain, on its own, cannot prove
//! *completeness* — nothing in entry N's hash can prove entry N+1 once
//! existed and was deleted. [`PersistentAuditLog`] closes this with a
//! second, small persisted checkpoint (`audit_chain_head`, a single-row
//! table holding the next sequence number and last entry hash, written in
//! the same transaction as every append) that [`PersistentAuditLog::load`]
//! cross-checks against the entries actually present. This is a
//! best-effort watermark, not a cryptographic guarantee: an attacker with
//! full read/write access to the SQLite file can update both the entries
//! table and the checkpoint row consistently (see
//! `tests::truncation_undetected_if_attacker_also_forges_the_checkpoint_head`
//! for exactly that boundary, made explicit rather than left to be
//! discovered). Closing that gap for real needs an external anchor (a
//! separately-stored or periodically-published copy of the latest hash) —
//! out of this charter's scope, filed as `T-218`.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

/// Domain-separation prefix for every hash this crate computes. Prevents a
/// SHA-256 preimage computed here from being reinterpreted as a hash
/// computed elsewhere in the codebase over similar-shaped JSON.
const HASH_DOMAIN: &[u8] = b"ferrite-audit-v1";

/// The current version of what [`compute_entry_hash`] hashes. Bump this —
/// and add a matching branch in `compute_entry_hash` keyed on the entry's
/// own stored `schema_version` — the next time the preimage's field set
/// changes. Never reuse a version number for a different preimage shape.
pub const AUDIT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditEventKind {
    CapabilityGranted,
    CapabilityDenied,
    CapabilityExercised,
    ContentBlocked,
    /// An evaluation-harness execution record was produced (ferrite-eval). Recorded
    /// so eval runs are themselves verifiable artifacts in the hash chain. The
    /// `capability` field carries the exec_id; `url` carries the case_id.
    EvalExecutionRecorded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub entry_id: Uuid,
    /// Which version of [`compute_entry_hash`]'s preimage shape produced
    /// `entry_hash`. Hashed itself (see module docs) so an old entry cannot
    /// be silently reinterpreted under a newer hashing rule.
    pub schema_version: u8,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub kind: AuditEventKind,
    pub principal_id: Uuid,
    pub capability: Option<String>,
    pub url: Option<String>,
    pub prev_hash: String,
    pub entry_hash: String,
}

/// The exact, canonically-ordered set of fields covered by the hash. A
/// purpose-built type rather than reusing [`AuditEntry`] directly: this
/// deliberately excludes `entry_hash` (an entry never hashes itself) and
/// makes the field list the hash covers visible in one place, independent
/// of the persistence-oriented `AuditEntry`.
#[derive(Serialize)]
struct HashPreimage<'a> {
    schema_version: u8,
    entry_id: Uuid,
    sequence: u64,
    /// RFC3339 string, not the `DateTime` value — pins the exact bytes
    /// hashed to a stable textual representation rather than `chrono`'s
    /// internal repr, which is not part of this crate's stability contract.
    timestamp: String,
    kind: &'a AuditEventKind,
    principal_id: Uuid,
    capability: &'a Option<String>,
    url: &'a Option<String>,
    prev_hash: &'a str,
}

/// Computes the domain-separated, schema-versioned entry hash. The single
/// function [`AuditLog::append`] and [`AuditLog::verify_chain`] both call —
/// see the module docs for why that matters.
#[allow(clippy::too_many_arguments)]
fn compute_entry_hash(
    schema_version: u8,
    entry_id: Uuid,
    sequence: u64,
    timestamp: &DateTime<Utc>,
    kind: &AuditEventKind,
    principal_id: Uuid,
    capability: &Option<String>,
    url: &Option<String>,
    prev_hash: &str,
) -> String {
    let preimage = HashPreimage {
        schema_version,
        entry_id,
        sequence,
        timestamp: timestamp.to_rfc3339(),
        kind,
        principal_id,
        capability,
        url,
        prev_hash,
    };
    let canonical =
        serde_json::to_vec(&preimage).expect("HashPreimage contains no non-serializable types");

    let mut hasher = Sha256::new();
    hasher.update(HASH_DOMAIN);
    hasher.update(&canonical);
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone)]
pub struct AuditLog {
    pub entries: Vec<AuditEntry>,
    pub sequence_counter: u64,
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("Hash mismatch: entry {sequence} hash {expected} does not match computed {actual}")]
    HashMismatch {
        sequence: u64,
        expected: String,
        actual: String,
    },
    #[error(
        "Chain broken at entry {sequence}: prev_hash {actual_prev} != previous entry_hash {expected_prev}"
    )]
    ChainBroken {
        sequence: u64,
        expected_prev: String,
        actual_prev: String,
    },
    #[error("Database error: {0}")]
    Sql(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Chain broken upon load")]
    ChainBrokenLoad,
    /// The persisted checkpoint (`audit_chain_head`) does not match the
    /// entries actually present after loading — the tail of the chain was
    /// removed (or the checkpoint row itself was deleted while entries
    /// remained). See the module docs for what this can and cannot prove.
    #[error(
        "Chain truncated: checkpoint expects {expected_next_sequence} entries with head hash {expected_last_hash:?}, but the loaded chain has {actual_next_sequence} with head hash {actual_last_hash:?}"
    )]
    TruncatedChain {
        expected_next_sequence: u64,
        expected_last_hash: String,
        actual_next_sequence: u64,
        actual_last_hash: String,
    },
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            sequence_counter: 0,
        }
    }

    pub fn append(
        &mut self,
        kind: AuditEventKind,
        principal_id: Uuid,
        capability: Option<String>,
        url: Option<String>,
    ) {
        let sequence = self.sequence_counter;
        let timestamp = Utc::now();
        let entry_id = Uuid::new_v4();
        let prev_hash = self
            .entries
            .last()
            .map(|e| e.entry_hash.clone())
            .unwrap_or_default();

        let entry_hash = compute_entry_hash(
            AUDIT_SCHEMA_VERSION,
            entry_id,
            sequence,
            &timestamp,
            &kind,
            principal_id,
            &capability,
            &url,
            &prev_hash,
        );

        let entry = AuditEntry {
            entry_id,
            schema_version: AUDIT_SCHEMA_VERSION,
            sequence,
            timestamp,
            kind,
            principal_id,
            capability,
            url,
            prev_hash,
            entry_hash,
        };

        self.entries.push(entry);
        self.sequence_counter += 1;
    }

    pub fn verify_chain(&self) -> bool {
        let mut expected_prev = String::new();

        for entry in &self.entries {
            if entry.prev_hash != expected_prev {
                return false;
            }

            let computed_hash = compute_entry_hash(
                entry.schema_version,
                entry.entry_id,
                entry.sequence,
                &entry.timestamp,
                &entry.kind,
                entry.principal_id,
                &entry.capability,
                &entry.url,
                &entry.prev_hash,
            );

            if computed_hash != entry.entry_hash {
                return false;
            }
            expected_prev = entry.entry_hash.clone();
        }

        true
    }
}

/// SQLite stores integers as signed 64-bit; rusqlite 0.38 no longer converts
/// `u64` implicitly. A sequence number never comes near `i64::MAX`, so saturating
/// on write and rejecting a negative value on read are both only ever
/// defensive.
fn sequence_to_sql(sequence: u64) -> i64 {
    i64::try_from(sequence).unwrap_or(i64::MAX)
}

fn sequence_from_sql(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

pub struct PersistentAuditLog {
    pub log: AuditLog,
    pub conn: Connection,
}

impl PersistentAuditLog {
    pub fn new(db_path: &str) -> Result<Self, AuditError> {
        let conn = Connection::open(db_path).map_err(|e| AuditError::Sql(e.to_string()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit_entries (
                entry_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                sequence INTEGER,
                timestamp TEXT,
                kind TEXT,
                principal_id TEXT,
                capability TEXT,
                url TEXT,
                prev_hash TEXT,
                entry_hash TEXT
            )",
            [],
        )
        .map_err(|e| AuditError::Sql(e.to_string()))?;

        // Single-row watermark of the chain's tail, updated transactionally
        // with every append. Lets `load` detect a truncated tail, which the
        // hash chain alone cannot (see module docs).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit_chain_head (
                id INTEGER PRIMARY KEY CHECK (id = 0),
                next_sequence INTEGER NOT NULL,
                last_entry_hash TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| AuditError::Sql(e.to_string()))?;

        Ok(Self {
            log: AuditLog::new(),
            conn,
        })
    }

    pub fn append(
        &mut self,
        kind: AuditEventKind,
        principal_id: Uuid,
        capability: Option<String>,
        url: Option<String>,
    ) -> Result<(), AuditError> {
        self.log
            .append(kind.clone(), principal_id, capability.clone(), url.clone());
        let entry = self
            .log
            .entries
            .last()
            .expect("append just pushed an entry");

        let kind_str = serde_json::to_string(&entry.kind)
            .map_err(|e| AuditError::Serialization(e.to_string()))?;

        let tx = self
            .conn
            .transaction()
            .map_err(|e| AuditError::Sql(e.to_string()))?;

        tx.execute(
            "INSERT INTO audit_entries (entry_id, schema_version, sequence, timestamp, kind, principal_id, capability, url, prev_hash, entry_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                entry.entry_id.to_string(),
                entry.schema_version,
                sequence_to_sql(entry.sequence),
                entry.timestamp.to_rfc3339(),
                kind_str,
                entry.principal_id.to_string(),
                entry.capability,
                entry.url,
                entry.prev_hash,
                entry.entry_hash,
            ],
        ).map_err(|e| AuditError::Sql(e.to_string()))?;

        // Upsert the single watermark row in the same transaction as the
        // entry insert, so the two can never observably diverge from a
        // caller's point of view (a crash between them just loses the
        // whole transaction, not half of it).
        tx.execute(
            "INSERT INTO audit_chain_head (id, next_sequence, last_entry_hash) VALUES (0, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET next_sequence = excluded.next_sequence, last_entry_hash = excluded.last_entry_hash",
            params![sequence_to_sql(entry.sequence + 1), entry.entry_hash],
        ).map_err(|e| AuditError::Sql(e.to_string()))?;

        tx.commit().map_err(|e| AuditError::Sql(e.to_string()))?;

        Ok(())
    }

    pub fn load(db_path: &str) -> Result<Self, AuditError> {
        let conn = Connection::open(db_path).map_err(|e| AuditError::Sql(e.to_string()))?;

        let entries = {
            let mut stmt = conn.prepare("SELECT entry_id, schema_version, sequence, timestamp, kind, principal_id, capability, url, prev_hash, entry_hash FROM audit_entries ORDER BY sequence ASC").map_err(|e| AuditError::Sql(e.to_string()))?;

            let entry_iter = stmt
                .query_map([], |row| {
                    let entry_id_str: String = row.get(0)?;
                    let schema_version: u8 = row.get(1)?;
                    let sequence = sequence_from_sql(row.get(2)?, 2)?;
                    let timestamp_str: String = row.get(3)?;
                    let kind_str: String = row.get(4)?;
                    let principal_id_str: String = row.get(5)?;
                    let capability: Option<String> = row.get(6)?;
                    let url: Option<String> = row.get(7)?;
                    let prev_hash: String = row.get(8)?;
                    let entry_hash: String = row.get(9)?;

                    Ok((
                        entry_id_str,
                        schema_version,
                        sequence,
                        timestamp_str,
                        kind_str,
                        principal_id_str,
                        capability,
                        url,
                        prev_hash,
                        entry_hash,
                    ))
                })
                .map_err(|e| AuditError::Sql(e.to_string()))?;

            let mut entries = Vec::new();
            for row_result in entry_iter {
                let (
                    entry_id_str,
                    schema_version,
                    sequence,
                    timestamp_str,
                    kind_str,
                    principal_id_str,
                    capability,
                    url,
                    prev_hash,
                    entry_hash,
                ) = row_result.map_err(|e| AuditError::Sql(e.to_string()))?;

                let entry_id =
                    Uuid::from_str(&entry_id_str).map_err(|e| AuditError::Parse(e.to_string()))?;
                let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
                    .map_err(|e| AuditError::Parse(e.to_string()))?
                    .with_timezone(&Utc);
                let kind: AuditEventKind = serde_json::from_str(&kind_str)
                    .map_err(|e| AuditError::Serialization(e.to_string()))?;
                let principal_id = Uuid::from_str(&principal_id_str)
                    .map_err(|e| AuditError::Parse(e.to_string()))?;

                entries.push(AuditEntry {
                    entry_id,
                    schema_version,
                    sequence,
                    timestamp,
                    kind,
                    principal_id,
                    capability,
                    url,
                    prev_hash,
                    entry_hash,
                });
            }
            entries
        };

        let sequence_counter = entries.last().map(|e| e.sequence + 1).unwrap_or(0);

        let log = AuditLog {
            entries,
            sequence_counter,
        };

        if !log.verify_chain() {
            return Err(AuditError::ChainBrokenLoad);
        }

        // Truncation check: a hash chain alone cannot prove nothing was
        // removed from its tail, so cross-reference the persisted
        // watermark against what's actually present.
        let head: Option<(u64, String)> = conn
            .query_row(
                "SELECT next_sequence, last_entry_hash FROM audit_chain_head WHERE id = 0",
                [],
                |row| Ok((sequence_from_sql(row.get(0)?, 0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| AuditError::Sql(e.to_string()))?;

        let actual_next_sequence = sequence_counter;
        let actual_last_hash = log
            .entries
            .last()
            .map(|e| e.entry_hash.clone())
            .unwrap_or_default();

        match head {
            Some((expected_next_sequence, expected_last_hash)) => {
                if expected_next_sequence != actual_next_sequence
                    || expected_last_hash != actual_last_hash
                {
                    return Err(AuditError::TruncatedChain {
                        expected_next_sequence,
                        expected_last_hash,
                        actual_next_sequence,
                        actual_last_hash,
                    });
                }
            }
            None if actual_next_sequence != 0 => {
                // Entries exist but no checkpoint was ever written for
                // them. `append` always writes one transactionally, so
                // this means the checkpoint row itself was removed —
                // treat it the same as a detected truncation rather than
                // silently trusting the entries table alone.
                return Err(AuditError::TruncatedChain {
                    expected_next_sequence: 0,
                    expected_last_hash: String::new(),
                    actual_next_sequence,
                    actual_last_hash,
                });
            }
            None => {}
        }

        Ok(Self { log, conn })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ferrite-audit-log-test-{label}-{}.db",
            Uuid::new_v4()
        ))
    }

    fn fixed_uuid(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    // ---- basic sanity ----

    #[test]
    fn append_produces_a_verifying_chain() {
        let mut log = AuditLog::new();
        log.append(
            AuditEventKind::CapabilityGranted,
            Uuid::new_v4(),
            Some("web.read".to_string()),
            Some("https://example.com".to_string()),
        );
        log.append(
            AuditEventKind::CapabilityExercised,
            Uuid::new_v4(),
            None,
            None,
        );
        assert!(log.verify_chain());
    }

    #[test]
    fn empty_log_verifies() {
        assert!(AuditLog::new().verify_chain());
    }

    // ---- T-005 direct regression: the two originally-named fields ----

    #[test]
    fn tampering_capability_after_the_fact_breaks_verification() {
        let mut log = AuditLog::new();
        log.append(
            AuditEventKind::EvalExecutionRecorded,
            Uuid::new_v4(),
            Some("exec-1".to_string()),
            Some("case-1".to_string()),
        );
        log.entries[0].capability = Some("exec-FORGED".to_string());
        assert!(!log.verify_chain());
    }

    #[test]
    fn tampering_url_after_the_fact_breaks_verification() {
        let mut log = AuditLog::new();
        log.append(
            AuditEventKind::EvalExecutionRecorded,
            Uuid::new_v4(),
            Some("exec-1".to_string()),
            Some("case-1".to_string()),
        );
        log.entries[0].url = Some("case-FORGED".to_string());
        assert!(!log.verify_chain());
    }

    // ---- tamper matrix: every field, not just the two the bug report named ----

    fn base_log() -> AuditLog {
        let mut log = AuditLog::new();
        log.append(
            AuditEventKind::EvalExecutionRecorded,
            fixed_uuid(0x11),
            Some("exec-1".to_string()),
            Some("case-1".to_string()),
        );
        log.append(
            AuditEventKind::CapabilityExercised,
            fixed_uuid(0x22),
            Some("web.read".to_string()),
            Some("https://example.com".to_string()),
        );
        log
    }

    #[test]
    fn tamper_matrix_capability() {
        let mut log = base_log();
        log.entries[0].capability = Some("forged".to_string());
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_url() {
        let mut log = base_log();
        log.entries[0].url = Some("https://attacker.example".to_string());
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_principal_id() {
        let mut log = base_log();
        log.entries[0].principal_id = fixed_uuid(0x99);
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_kind() {
        let mut log = base_log();
        log.entries[0].kind = AuditEventKind::ContentBlocked;
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_timestamp() {
        let mut log = base_log();
        log.entries[0].timestamp += chrono::Duration::seconds(1);
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_sequence() {
        let mut log = base_log();
        log.entries[0].sequence = 999;
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_entry_id() {
        let mut log = base_log();
        log.entries[0].entry_id = fixed_uuid(0x88);
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_prev_hash() {
        let mut log = base_log();
        log.entries[1].prev_hash = "0".repeat(64);
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_schema_version() {
        let mut log = base_log();
        log.entries[0].schema_version = 2;
        assert!(!log.verify_chain());
    }

    #[test]
    fn tamper_matrix_entry_hash_rewritten_directly() {
        let mut log = base_log();
        log.entries[0].entry_hash = "0".repeat(64);
        assert!(!log.verify_chain());
    }

    // ---- truncation, reordering, insertion attacks via a real SQLite file ----

    #[test]
    fn truncation_attack_dropping_the_tail_is_detected_on_load() {
        let db_path = temp_db_path("truncation");
        let db_path_str = db_path.to_str().unwrap();
        {
            let mut audit = PersistentAuditLog::new(db_path_str).unwrap();
            for i in 0..4u32 {
                audit
                    .append(
                        AuditEventKind::CapabilityExercised,
                        Uuid::new_v4(),
                        Some(format!("cap-{i}")),
                        None,
                    )
                    .unwrap();
            }
        }

        // Drop the last entry directly, exactly as an attacker with file
        // access would, without touching the checkpoint row.
        {
            let conn = Connection::open(db_path_str).unwrap();
            conn.execute(
                "DELETE FROM audit_entries WHERE sequence = (SELECT MAX(sequence) FROM audit_entries)",
                [],
            )
            .unwrap();
        }

        let result = PersistentAuditLog::load(db_path_str).map(|_| ());
        assert!(
            matches!(result, Err(AuditError::TruncatedChain { .. })),
            "expected TruncatedChain, got {result:?}"
        );

        std::fs::remove_file(db_path).ok();
    }

    #[test]
    fn truncation_undetected_if_attacker_also_forges_the_checkpoint_head() {
        // Documents the honest limit stated in the module docs: the
        // checkpoint is a best-effort watermark, not a cryptographic
        // guarantee. An attacker with full read/write access to the SQLite
        // file can keep the checkpoint consistent with a truncated tail.
        let db_path = temp_db_path("truncation-forged-head");
        let db_path_str = db_path.to_str().unwrap();
        let mut last_remaining_hash = String::new();
        {
            let mut audit = PersistentAuditLog::new(db_path_str).unwrap();
            for i in 0..3u32 {
                audit
                    .append(
                        AuditEventKind::CapabilityExercised,
                        Uuid::new_v4(),
                        Some(format!("cap-{i}")),
                        None,
                    )
                    .unwrap();
                if i == 1 {
                    last_remaining_hash = audit.log.entries.last().unwrap().entry_hash.clone();
                }
            }
        }

        {
            let conn = Connection::open(db_path_str).unwrap();
            conn.execute(
                "DELETE FROM audit_entries WHERE sequence = (SELECT MAX(sequence) FROM audit_entries)",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE audit_chain_head SET next_sequence = 2, last_entry_hash = ?1",
                params![last_remaining_hash],
            )
            .unwrap();
        }

        let result = PersistentAuditLog::load(db_path_str).map(|_| ());
        assert!(
            result.is_ok(),
            "expected load to (mis)report success: {result:?}"
        );

        std::fs::remove_file(db_path).ok();
    }

    #[test]
    fn reordering_attack_swapping_sequence_values_is_detected_on_load() {
        let db_path = temp_db_path("reorder");
        let db_path_str = db_path.to_str().unwrap();
        {
            let mut audit = PersistentAuditLog::new(db_path_str).unwrap();
            for i in 0..3u32 {
                audit
                    .append(
                        AuditEventKind::CapabilityExercised,
                        Uuid::new_v4(),
                        Some(format!("cap-{i}")),
                        None,
                    )
                    .unwrap();
            }
        }

        // Swap the sequence values of entries 1 and 2, leaving their
        // stored hashes untouched — exactly what an attacker who doesn't
        // know the hash scheme would do.
        {
            let conn = Connection::open(db_path_str).unwrap();
            conn.execute(
                "UPDATE audit_entries SET sequence = 99 WHERE sequence = 1",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE audit_entries SET sequence = 1 WHERE sequence = 2",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE audit_entries SET sequence = 2 WHERE sequence = 99",
                [],
            )
            .unwrap();
        }

        let result = PersistentAuditLog::load(db_path_str).map(|_| ());
        assert!(
            matches!(result, Err(AuditError::ChainBrokenLoad)),
            "expected ChainBrokenLoad, got {result:?}"
        );

        std::fs::remove_file(db_path).ok();
    }

    #[test]
    fn insertion_attack_splicing_a_self_consistent_forged_entry_is_detected_on_load() {
        let db_path = temp_db_path("insertion");
        let db_path_str = db_path.to_str().unwrap();
        let entry_a_hash;
        {
            let mut audit = PersistentAuditLog::new(db_path_str).unwrap();
            audit
                .append(
                    AuditEventKind::CapabilityExercised,
                    fixed_uuid(0x01),
                    Some("cap-a".to_string()),
                    None,
                )
                .unwrap();
            entry_a_hash = audit.log.entries[0].entry_hash.clone();
            audit
                .append(
                    AuditEventKind::CapabilityExercised,
                    fixed_uuid(0x02),
                    Some("cap-b".to_string()),
                    None,
                )
                .unwrap();
        }

        // Build a forged entry whose own hash is internally valid (an
        // attacker who has read the source could compute this), chained
        // from the real entry A, but never woven into B's prev_hash or
        // reflected in the checkpoint. sequence collides with B's — SQLite
        // has no UNIQUE constraint on `sequence`, so this insert succeeds.
        let forged_kind_json = serde_json::to_string(&AuditEventKind::CapabilityGranted).unwrap();
        let forged_id = fixed_uuid(0xfe);
        let forged_principal = fixed_uuid(0xff);
        let forged_capability = Some("cap-FORGED".to_string());
        let forged_url: Option<String> = None;
        let forged_timestamp = Utc::now();
        let forged_hash = compute_entry_hash(
            AUDIT_SCHEMA_VERSION,
            forged_id,
            1,
            &forged_timestamp,
            &AuditEventKind::CapabilityGranted,
            forged_principal,
            &forged_capability,
            &forged_url,
            &entry_a_hash,
        );

        {
            let conn = Connection::open(db_path_str).unwrap();
            conn.execute(
                "INSERT INTO audit_entries (entry_id, schema_version, sequence, timestamp, kind, principal_id, capability, url, prev_hash, entry_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    forged_id.to_string(),
                    AUDIT_SCHEMA_VERSION,
                    1,
                    forged_timestamp.to_rfc3339(),
                    forged_kind_json,
                    forged_principal.to_string(),
                    forged_capability,
                    forged_url,
                    entry_a_hash,
                    forged_hash,
                ],
            )
            .unwrap();
        }

        let result = PersistentAuditLog::load(db_path_str).map(|_| ());
        assert!(
            matches!(result, Err(AuditError::ChainBrokenLoad)),
            "expected the splice to break the chain link, got {result:?}"
        );

        std::fs::remove_file(db_path).ok();
    }

    // ---- golden chain fixture: pins the preimage so an accidental change is loud ----

    #[test]
    fn golden_chain_fixture_pins_entry_hashes() {
        let fixed_timestamp: DateTime<Utc> = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let entry_id_a = Uuid::parse_str("00000000-0000-0000-0000-00000000000a").unwrap();
        let entry_id_b = Uuid::parse_str("00000000-0000-0000-0000-00000000000b").unwrap();
        let principal = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        let hash_a = compute_entry_hash(
            AUDIT_SCHEMA_VERSION,
            entry_id_a,
            0,
            &fixed_timestamp,
            &AuditEventKind::EvalExecutionRecorded,
            principal,
            &Some("exec-42".to_string()),
            &Some("case-7".to_string()),
            "",
        );

        let hash_b = compute_entry_hash(
            AUDIT_SCHEMA_VERSION,
            entry_id_b,
            1,
            &fixed_timestamp,
            &AuditEventKind::CapabilityExercised,
            principal,
            &None,
            &None,
            &hash_a,
        );

        assert_eq!(
            hash_a, "c1b4ff371b8ed37e43a3f6fb61a4a88f95c457cfd3219e0214d743929b0de11a",
            "golden hash for entry A changed — an accidental preimage change, or an intentional \
             schema_version bump that forgot to update this pinned fixture"
        );
        assert_eq!(
            hash_b, "66e21de3ecb7547adc3bcf6a901031fe694b3a3cd73c37e26b4d263e4c282671",
            "golden hash for entry B changed — an accidental preimage change, or an intentional \
             schema_version bump that forgot to update this pinned fixture"
        );
    }

    // ---- persistence round-trip sanity, covering schema_version/checkpoint plumbing ----

    #[test]
    fn persisted_log_round_trips_and_verifies() {
        let db_path = temp_db_path("roundtrip");
        let db_path_str = db_path.to_str().unwrap();
        {
            let mut audit = PersistentAuditLog::new(db_path_str).unwrap();
            audit
                .append(
                    AuditEventKind::EvalExecutionRecorded,
                    Uuid::new_v4(),
                    Some("exec-1".to_string()),
                    Some("case-1".to_string()),
                )
                .unwrap();
            audit
                .append(
                    AuditEventKind::CapabilityExercised,
                    Uuid::new_v4(),
                    None,
                    None,
                )
                .unwrap();
        }

        let loaded = PersistentAuditLog::load(db_path_str).unwrap();
        assert_eq!(loaded.log.entries.len(), 2);
        assert!(
            loaded
                .log
                .entries
                .iter()
                .all(|e| e.schema_version == AUDIT_SCHEMA_VERSION)
        );
        assert!(loaded.log.verify_chain());

        std::fs::remove_file(db_path).ok();
    }

    #[test]
    fn persisted_tamper_of_capability_column_is_detected_on_load() {
        // The exact T-005 scenario: rewrite a persisted column directly in
        // SQLite, bypassing the API entirely, and confirm `load` now
        // refuses to report the chain intact.
        let db_path = temp_db_path("persisted-tamper");
        let db_path_str = db_path.to_str().unwrap();
        {
            let mut audit = PersistentAuditLog::new(db_path_str).unwrap();
            audit
                .append(
                    AuditEventKind::EvalExecutionRecorded,
                    Uuid::new_v4(),
                    Some("exec-1".to_string()),
                    Some("case-1".to_string()),
                )
                .unwrap();
        }

        {
            let conn = Connection::open(db_path_str).unwrap();
            conn.execute("UPDATE audit_entries SET capability = 'exec-FORGED'", [])
                .unwrap();
        }

        let result = PersistentAuditLog::load(db_path_str);
        assert!(matches!(result, Err(AuditError::ChainBrokenLoad)));

        std::fs::remove_file(db_path).ok();
    }
}
