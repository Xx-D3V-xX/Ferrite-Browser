//! The hard call budget — §10.3's fourth mechanism.
//!
//! `FERRITE_MODEL_CALL_BUDGET` (default 500 per process). On exhaustion the
//! run **aborts with a clear error and a partial-results file** rather than
//! silently continuing or quietly burning through the rest of the quota.
//!
//! The partial-results artifact is the part that makes the abort survivable:
//! a run that dies at call 500 of 900 has still produced 500 real results,
//! and throwing them away because the process ended badly would mean paying
//! for them twice. It is a JSON ledger of every call made, in order, with
//! its provenance and token counts — enough to resume, and enough to see
//! *where* the quota went when the budget turns out to be too small.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrite_core::Clock;
use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, TokenUsage};

/// One call, as recorded in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRecord {
    /// 1-based position in the run.
    pub seq: u32,
    /// When it was made, from the injected [`Clock`].
    pub at: DateTime<Utc>,
    /// Which backend.
    pub provider: ProviderId,
    /// Which tag.
    pub model_tag: String,
    /// Which tier.
    pub tier: ModelTier,
    /// Whether the cache served it (a cache hit costs no quota but is worth
    /// recording, since the hit rate is the thing §10.3 asks to be watched).
    pub cache_hit: bool,
    /// Token counts, absent when the call failed.
    pub usage: Option<TokenUsage>,
    /// `None` on success, or the error's `Display` form — bounded, because
    /// the errors it renders already bound their provider-controlled parts.
    pub error: Option<String>,
}

/// What a run spent, printed at exit (§10.3) and written beside the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSummary {
    /// The configured ceiling.
    pub budget: u32,
    /// Calls that reached [`ModelProvider::complete`] on this decorator.
    pub calls_used: u32,
    /// How many of those the cache served.
    pub cache_hits: u32,
    /// Prompt tokens across the run.
    pub prompt_tokens: u64,
    /// Completion tokens across the run.
    pub eval_tokens: u64,
}

impl std::fmt::Display for BudgetSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "calls-used {}/{} · cache-hits {} · tokens {} prompt + {} eval",
            self.calls_used, self.budget, self.cache_hits, self.prompt_tokens, self.eval_tokens
        )
    }
}

/// The artifact written when the budget runs out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialResults {
    /// What the run spent.
    pub summary: BudgetSummary,
    /// When it aborted.
    pub aborted_at: DateTime<Utc>,
    /// Every call, in order.
    pub calls: Vec<CallRecord>,
}

/// Wraps any provider with a hard per-process call ceiling.
#[derive(Debug)]
pub struct Budget<P: ModelProvider> {
    inner: P,
    limit: u32,
    used: AtomicU32,
    ledger: Mutex<Vec<CallRecord>>,
    partial_results_path: PathBuf,
    clock: Arc<dyn Clock>,
}

impl<P: ModelProvider> Budget<P> {
    /// Wraps `inner` with a ceiling of `limit` calls, writing the
    /// partial-results artifact to `partial_results_path` if it is hit.
    #[must_use]
    pub fn new(
        inner: P,
        limit: u32,
        partial_results_path: impl Into<PathBuf>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner,
            limit,
            used: AtomicU32::new(0),
            ledger: Mutex::new(Vec::new()),
            partial_results_path: partial_results_path.into(),
            clock,
        }
    }

    /// Where the partial-results artifact is written.
    #[must_use]
    pub fn partial_results_path(&self) -> &Path {
        &self.partial_results_path
    }

    /// What has been spent so far — the line a run prints at exit.
    #[must_use]
    pub fn summary(&self) -> BudgetSummary {
        let ledger = self.ledger.lock().expect("budget ledger poisoned");
        let mut summary = BudgetSummary {
            budget: self.limit,
            calls_used: self.used.load(Ordering::SeqCst),
            cache_hits: 0,
            prompt_tokens: 0,
            eval_tokens: 0,
        };
        for record in ledger.iter() {
            if record.cache_hit {
                summary.cache_hits += 1;
            }
            if let Some(usage) = record.usage {
                summary.prompt_tokens += u64::from(usage.prompt_eval_count);
                summary.eval_tokens += u64::from(usage.eval_count);
            }
        }
        summary
    }

    /// Every call recorded so far, in order.
    #[must_use]
    pub fn ledger(&self) -> Vec<CallRecord> {
        self.ledger.lock().expect("budget ledger poisoned").clone()
    }

    /// Writes the partial-results artifact now, without waiting for the
    /// budget to run out — what a run calls on any abort, not just this one.
    ///
    /// # Errors
    ///
    /// [`ModelError::Cache`] if the file cannot be written. The path is
    /// reported so the caller can say where the results did *not* land.
    pub fn write_partial_results(&self) -> Result<(), ModelError> {
        let artifact = PartialResults {
            summary: self.summary(),
            aborted_at: self.clock.now(),
            calls: self.ledger(),
        };
        if let Some(parent) = self.partial_results_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ModelError::Cache {
                path: parent.to_path_buf(),
                detail: e.to_string(),
            })?;
        }
        let body = serde_json::to_string_pretty(&artifact).map_err(|e| ModelError::Cache {
            path: self.partial_results_path.clone(),
            detail: e.to_string(),
        })?;
        std::fs::write(&self.partial_results_path, body).map_err(|e| ModelError::Cache {
            path: self.partial_results_path.clone(),
            detail: e.to_string(),
        })
    }

    fn record(
        &self,
        seq: u32,
        req: &CompletionRequest,
        outcome: &Result<CompletionResponse, ModelError>,
    ) {
        let record = match outcome {
            Ok(response) => CallRecord {
                seq,
                at: self.clock.now(),
                provider: response.provenance.provider,
                model_tag: response.provenance.model_tag.clone(),
                tier: response.provenance.tier,
                cache_hit: response.provenance.cache_hit,
                usage: Some(response.usage),
                error: None,
            },
            Err(e) => CallRecord {
                seq,
                at: self.clock.now(),
                provider: self.inner.id(),
                model_tag: req.model_tag.clone(),
                tier: req.tier,
                cache_hit: false,
                usage: None,
                error: Some(e.to_string()),
            },
        };
        self.ledger
            .lock()
            .expect("budget ledger poisoned")
            .push(record);
    }
}

#[async_trait]
impl<P: ModelProvider> ModelProvider for Budget<P> {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        // Reserved before the call, not after: two concurrent calls must not
        // both see 499 used and both proceed.
        let seq = self.used.fetch_add(1, Ordering::SeqCst) + 1;
        if seq > self.limit {
            // Roll back so the counter reports what was actually spent
            // rather than climbing with every rejected call.
            self.used.fetch_sub(1, Ordering::SeqCst);
            // Best-effort: if the artifact cannot be written, the abort
            // still has to happen, and the budget error is the more
            // important of the two to surface.
            let _ = self.write_partial_results();
            return Err(ModelError::BudgetExhausted {
                budget: self.limit,
                partial_results_path: self.partial_results_path.clone(),
            });
        }

        let outcome = self.inner.complete(req.clone()).await;
        self.record(seq, &req, &outcome);
        outcome
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
    use crate::request::Message;
    use crate::testing::TempDir;

    fn req() -> CompletionRequest {
        CompletionRequest::new("tag:1b", ModelTier::Small, vec![Message::user("hi")])
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(FixedClock::at_epoch())
    }

    #[tokio::test]
    async fn calls_within_the_budget_pass_through() {
        let dir = TempDir::new("budget-ok");
        let budget = Budget::new(
            MockProvider::new().always_content("ok"),
            3,
            dir.path().join("partial.json"),
            clock(),
        );
        for _ in 0..3 {
            budget.complete(req()).await.expect("within budget");
        }
        assert_eq!(budget.summary().calls_used, 3);
        assert!(
            !dir.path().join("partial.json").exists(),
            "no artifact is written while the run is healthy"
        );
    }

    #[tokio::test]
    async fn exceeding_the_budget_aborts_with_a_clear_error() {
        // A3's exit gate: "the call-budget guard aborts a run when exceeded
        // (test asserts it)".
        let dir = TempDir::new("budget-abort");
        let inner = Arc::new(MockProvider::new().always_content("ok"));
        let budget = Budget::new(
            Arc::clone(&inner),
            2,
            dir.path().join("partial.json"),
            clock(),
        );

        budget.complete(req()).await.expect("1 of 2");
        budget.complete(req()).await.expect("2 of 2");
        let err = budget.complete(req()).await.expect_err("3 of 2 must abort");

        match err {
            ModelError::BudgetExhausted {
                budget: limit,
                partial_results_path,
            } => {
                assert_eq!(limit, 2);
                assert_eq!(partial_results_path, dir.path().join("partial.json"));
            }
            other => panic!("expected BudgetExhausted, got {other:?}"),
        }
        assert_eq!(
            inner.call_count(),
            2,
            "the over-budget call must never reach the provider"
        );
    }

    #[tokio::test]
    async fn exceeding_the_budget_writes_the_partial_results_artifact() {
        let dir = TempDir::new("budget-partial");
        let path = dir.path().join("nested").join("partial.json");
        let budget = Budget::new(
            MockProvider::new().always_content("an answer"),
            1,
            &path,
            clock(),
        );
        budget.complete(req()).await.expect("1 of 1");
        budget.complete(req()).await.expect_err("aborts");

        let raw = std::fs::read_to_string(&path).expect("artifact written, parents created");
        let artifact: PartialResults = serde_json::from_str(&raw).expect("valid JSON");
        assert_eq!(artifact.summary.budget, 1);
        assert_eq!(artifact.summary.calls_used, 1);
        assert_eq!(artifact.calls.len(), 1, "the work already paid for is kept");
        assert_eq!(artifact.calls[0].seq, 1);
        assert_eq!(artifact.calls[0].model_tag, "tag:1b");
        assert_eq!(
            artifact.aborted_at,
            DateTime::UNIX_EPOCH,
            "R8: the artifact's timestamp comes from the injected clock"
        );
    }

    #[tokio::test]
    async fn the_counter_does_not_climb_past_the_budget_on_repeated_rejections() {
        let dir = TempDir::new("budget-rollback");
        let budget = Budget::new(
            MockProvider::new().always_content("ok"),
            1,
            dir.path().join("partial.json"),
            clock(),
        );
        budget.complete(req()).await.expect("1 of 1");
        for _ in 0..5 {
            budget.complete(req()).await.expect_err("aborts");
        }
        assert_eq!(
            budget.summary().calls_used,
            1,
            "calls-used must report what was spent, not how often it was refused"
        );
    }

    #[tokio::test]
    async fn a_failed_call_still_counts_against_the_budget_and_is_recorded() {
        // It cost a request either way. Not counting failures is how a
        // retry loop silently doubles the real spend.
        let dir = TempDir::new("budget-failure");
        let budget = Budget::new(
            MockProvider::new().push_error(ModelError::EmptyResponse {
                provider: ProviderId::Mock,
            }),
            5,
            dir.path().join("partial.json"),
            clock(),
        );
        budget.complete(req()).await.expect_err("the call fails");

        assert_eq!(budget.summary().calls_used, 1);
        let ledger = budget.ledger();
        assert_eq!(ledger.len(), 1);
        assert!(ledger[0].error.is_some());
        assert_eq!(ledger[0].usage, None);
    }

    #[tokio::test]
    async fn the_summary_totals_tokens_and_cache_hits() {
        let dir = TempDir::new("budget-summary");
        let budget = Budget::new(
            MockProvider::new().push_content_with_usage(
                "ok",
                TokenUsage {
                    prompt_eval_count: 10,
                    eval_count: 3,
                },
            ),
            5,
            dir.path().join("partial.json"),
            clock(),
        );
        budget.complete(req()).await.expect("ok");

        let summary = budget.summary();
        assert_eq!(summary.prompt_tokens, 10);
        assert_eq!(summary.eval_tokens, 3);
        assert_eq!(summary.cache_hits, 0);
        assert!(
            summary.to_string().contains("calls-used 1/5"),
            "§10.3 asks every run to print this line: {summary}"
        );
    }

    #[tokio::test]
    async fn a_cache_hit_is_recorded_as_one() {
        let dir = TempDir::new("budget-cachehit");
        let cache_dir = TempDir::new("budget-cachehit-store");
        let cached = super::super::Cache::new(
            MockProvider::new().always_content("ok"),
            cache_dir.path(),
            clock(),
        );
        let budget = Budget::new(cached, 5, dir.path().join("partial.json"), clock());

        budget.complete(req()).await.expect("miss");
        budget.complete(req()).await.expect("hit");

        assert_eq!(budget.summary().cache_hits, 1);
        assert_eq!(
            budget.summary().calls_used,
            2,
            "the budget counts requests it was asked to make; the cache is what makes them cheap"
        );
    }

    #[tokio::test]
    async fn partial_results_can_be_written_before_the_budget_runs_out() {
        let dir = TempDir::new("budget-early");
        let path = dir.path().join("partial.json");
        let budget = Budget::new(
            MockProvider::new().always_content("ok"),
            100,
            &path,
            clock(),
        );
        budget.complete(req()).await.expect("ok");
        budget.write_partial_results().expect("writes on demand");

        let artifact: PartialResults =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(artifact.calls.len(), 1);
    }

    #[tokio::test]
    async fn the_ledger_is_ordered_by_sequence() {
        let dir = TempDir::new("budget-order");
        let budget = Budget::new(
            MockProvider::new().always_content("ok"),
            10,
            dir.path().join("partial.json"),
            clock(),
        );
        for _ in 0..4 {
            budget.complete(req()).await.expect("ok");
        }
        let seqs: Vec<u32> = budget.ledger().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4]);
    }
}
