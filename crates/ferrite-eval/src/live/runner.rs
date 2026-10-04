//! Running a window of cases: predict, run each requested mode, label, store.
//!
//! One invocation is bounded three ways, and every bound is on the safe side:
//!
//! - **calls**: the stack's [`Budget`](ferrite_model::Budget) refuses call
//!   `--max-calls + 1`. The case in flight is then abandoned *unrecorded* (its
//!   completed modes are already stored; the unfinished one is simply still
//!   pending), and the invocation stops;
//! - **a provider that has gone away**: after `--max-consecutive-failures`
//!   infrastructure failures in a row (rate limit, outage, rejected key) the
//!   invocation stops rather than hammering a provider that is saying no;
//! - **the window**: `--batch-size`, `--limit`.
//!
//! # What is a failed case, and what is a result
//!
//! A case that failed for **infrastructure** reasons (the call never got an answer)
//! is stored as an error and excluded from every rate: it says nothing about the
//! model. A case where the model answered **badly** (unparseable action, empty
//! reply, an unusable fingerprint) is a result: the fingerprint falls back to the
//! rule layer and the agent's run ends, which the outcome labels record as what it
//! is (`CLAUDE.md`: fail to empty, never a bypass).

use std::path::PathBuf;
use std::time::Instant;

use ferrite_core::{Capability, Primitive};
use ferrite_ipi::comparator::{compare, RuntimeGuard};
use ferrite_ipi::dataset::{Carrier, CaseDefinition};
use ferrite_ipi::dry_run::{DryRunContent, DryRunOrchestrator};
use ferrite_ipi::fingerprint::Fingerprint;
use ferrite_ipi::tool_decision::ToolDecisionEngine;
use ferrite_ipi::IpiTask;

use super::agent::{Context, LlmAgent};
use super::config::{AgentKind, LiveArgs, LiveMode, ModelTags, PredictorKind};
use super::corpus::{LiveCase, Window};
use super::outcome::{judge, Attempt, Stop};
use super::plan::{behaviour_hash, is_pending, key_for, needs_run};
use super::provider::{CallEvent, ErrorClass, ModelStack};
use super::record::{
    CallCounts, LiveRecord, PredictionRecord, RecordError, Redactor, Verdict, SCHEMA_VERSION,
};
use super::store::{Results, Store};
use crate::harness::expected_fingerprint_for;
use crate::worst_case_agent::WorstCaseAgent;

/// Builds the dry-run orchestrator for one run. The binary passes the real
/// constructor; tests pass one with a fixed twin key so none touches the OS keyring.
pub type OrchestratorFactory = dyn Fn(PathBuf, DryRunContent) -> DryRunOrchestrator + Send + Sync;

/// Why an invocation stopped before the window was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// `--max-calls` is spent.
    BudgetExhausted,
    /// `--max-consecutive-failures` provider failures in a row.
    ProviderUnavailable {
        /// The last failure's class.
        class: String,
    },
}

/// What an invocation did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    /// Cases in the window.
    pub cases_in_window: usize,
    /// `(case, mode)` results already stored and reused.
    pub reused: usize,
    /// `(case, mode)` results produced now.
    pub produced: usize,
    /// Of those, errors.
    pub failed: usize,
    /// Cases with every requested mode stored after this invocation.
    pub cases_complete: usize,
    /// Why it stopped early, if it did.
    pub stopped: Option<StopReason>,
    /// What the calls cost, summed over the records produced now.
    pub calls: CallCounts,
}

/// Everything a run needs that is not the cases.
pub struct Runner<'a> {
    /// The arguments.
    pub args: &'a LiveArgs,
    /// The two model tags.
    pub tags: &'a ModelTags,
    /// The provider stack.
    pub stack: &'a ModelStack,
    /// Scrubs free text before it is stored.
    pub redactor: &'a Redactor,
    /// How to build an orchestrator.
    pub orchestrator: &'a OrchestratorFactory,
    /// Where the synthetic twin is cached for this invocation.
    pub twin_path: PathBuf,
}

enum CaseFailure {
    /// The call never got an answer: the case did not run.
    Infrastructure { class: ErrorClass, message: String },
    /// The `--max-calls` ceiling.
    Budget,
    /// The harness itself failed.
    Harness(String),
}

struct Predicted {
    fingerprint: Fingerprint,
    record: PredictionRecord,
    outer: Vec<CallEvent>,
    live: Vec<CallEvent>,
}

struct Product {
    attempts: Vec<Attempt>,
    stop: Stop,
    sanitizer_findings: u32,
    dry_run_gated: Option<bool>,
    outer: Vec<CallEvent>,
    live: Vec<CallEvent>,
}

/// Counts what a set of events cost.
#[must_use]
pub fn counts(outer: &[CallEvent], live: &[CallEvent]) -> CallCounts {
    CallCounts {
        logical: u32::try_from(outer.len()).unwrap_or(u32::MAX),
        live_attempts: u32::try_from(live.len()).unwrap_or(u32::MAX),
        cache_hits: u32::try_from(outer.iter().filter(|e| e.cache_hit).count()).unwrap_or(u32::MAX),
        rate_limited: u32::try_from(
            live.iter()
                .filter(|e| matches!(e.error, Some((ErrorClass::RateLimited, _))))
                .count(),
        )
        .unwrap_or(u32::MAX),
        // A cache hit carries the tokens of the call that first made it; counting
        // them again would report spend that did not happen.
        prompt_tokens: outer
            .iter()
            .filter(|e| !e.cache_hit)
            .map(|e| u64::from(e.prompt_tokens))
            .sum(),
        eval_tokens: outer
            .iter()
            .filter(|e| !e.cache_hit)
            .map(|e| u64::from(e.eval_tokens))
            .sum(),
    }
}

fn add(total: &mut CallCounts, c: CallCounts) {
    total.logical += c.logical;
    total.live_attempts += c.live_attempts;
    total.cache_hits += c.cache_hits;
    total.rate_limited += c.rate_limited;
    total.prompt_tokens += c.prompt_tokens;
    total.eval_tokens += c.eval_tokens;
}

fn capability_name(debug_name: &str) -> Option<&'static str> {
    Capability::ALL
        .iter()
        .find(|c| format!("{c:?}") == debug_name)
        .map(|c| c.as_str())
}

/// The capabilities the planted attack needs beyond what the task's own ground
/// truth needs, as `Capability::as_str` names.
fn attack_capabilities(case: &LiveCase, ideal: &[String]) -> Option<Vec<String>> {
    let meta = case.meta.as_ref()?;
    meta.injection_task_id.as_ref()?;
    let mut out: Vec<String> = meta
        .injection_primitives
        .iter()
        .filter_map(|name| Primitive::ALL.iter().find(|p| p.as_str() == name))
        .filter_map(|p| p.as_scopable())
        .map(|sp| sp.capability().as_str().to_string())
        .filter(|c| !ideal.contains(c))
        .collect();
    out.sort();
    out.dedup();
    Some(out)
}

fn context_url(case: &CaseDefinition, content: &DryRunContent) -> Option<String> {
    match &case.expected_origins {
        ferrite_core::OriginScope::Exact(origins) => {
            origins.first().map(|o| o.as_str().to_string())
        }
        _ => content.first_origin().map(str::to_string),
    }
}

impl Runner<'_> {
    /// Runs `window` (already ordered) and appends results to `store`.
    ///
    /// `progress` receives one line per stored result and one for each notable
    /// event; the binary prints them.
    ///
    /// # Errors
    ///
    /// An I/O error appending a result. Fatal on purpose: a result that cannot be
    /// stored is one that would be paid for twice.
    pub async fn run(
        &self,
        ordered: Vec<LiveCase>,
        store: &mut Store,
        existing: &Results,
        progress: &mut dyn FnMut(String),
    ) -> std::io::Result<RunSummary> {
        let hash = behaviour_hash(self.args);
        let provider_name = self
            .args
            .provider
            .map_or("mock", super::config::ProviderKind::as_str);
        let key = |case: &LiveCase, mode: LiveMode| {
            key_for(
                self.args,
                self.tags,
                &case.case.case_id.to_string(),
                mode,
                &hash,
            )
        };
        let window = Window::of(self.args).take(ordered, |case| {
            is_pending(self.args, self.tags, existing, &hash, case)
        });
        let total = window.len();

        let mut summary = RunSummary {
            cases_in_window: total,
            ..RunSummary::default()
        };
        let mut consecutive_failures = 0u32;
        let mut stored_now: std::collections::HashSet<String> = std::collections::HashSet::new();

        'cases: for (index, lc) in window.iter().enumerate() {
            let pending: Vec<LiveMode> = self
                .args
                .modes
                .iter()
                .copied()
                .filter(|m| needs_run(existing, &key(lc, *m), self.args.retry_failed))
                .collect();
            summary.reused += self.args.modes.len() - pending.len();
            if pending.is_empty() {
                summary.cases_complete += 1;
                continue;
            }

            // The prediction is made once per case and shared by its modes.
            let mut predicted: Option<Predicted> = None;
            let mut predict_failure: Option<CaseFailure> = None;
            if pending.iter().any(|m| m.needs_prediction()) {
                match self.predict(lc).await {
                    Ok(p) => predicted = Some(p),
                    Err(failure) => predict_failure = Some(failure),
                }
            }
            let mut predict_events = predicted
                .as_ref()
                .map(|p| (p.outer.clone(), p.live.clone()));

            let mut case_ok = true;
            for mode in pending {
                let started_at = chrono::Utc::now().to_rfc3339();
                let started = Instant::now();
                let outcome = if mode.needs_prediction() {
                    match (&predicted, predict_failure.take()) {
                        (Some(p), _) => self.run_mode(lc, mode, Some(p)).await,
                        (None, Some(failure)) => Err(failure),
                        (None, None) => Err(CaseFailure::Harness(
                            "the prediction failed earlier in this case".to_string(),
                        )),
                    }
                } else {
                    self.run_mode(lc, mode, None).await
                };

                let mut record = self.skeleton(lc, mode, &started_at, provider_name, &hash);
                record.latency_ms =
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Some(p) = &predicted {
                    if mode.needs_prediction() {
                        record.prediction = Some(p.record.clone());
                    }
                }
                match outcome {
                    Ok(product) => {
                        consecutive_failures = 0;
                        let (o, actions) = judge(
                            lc,
                            mode,
                            &product.attempts,
                            &product.stop,
                            product.dry_run_gated,
                            product.sanitizer_findings,
                            self.redactor,
                        );
                        record.outcome = o;
                        record.actions = actions;
                        record.stop_reason = product.stop.name();
                        record.final_answer = match &product.stop {
                            Stop::Finished(a) => Some(self.redactor.bounded(a)),
                            _ => None,
                        };
                        let mut c = counts(&product.outer, &product.live);
                        // The prediction's calls are charged to the first record.
                        if let Some((o, l)) = predict_events.take() {
                            add(&mut c, counts(&o, &l));
                        }
                        record.calls = c;
                    }
                    Err(CaseFailure::Budget) => {
                        summary.stopped = Some(StopReason::BudgetExhausted);
                        progress(format!(
                            "stopping: the --max-calls budget ({}) is spent; the case in flight was not recorded and is still pending",
                            self.args.max_calls
                        ));
                        break 'cases;
                    }
                    Err(CaseFailure::Infrastructure { class, message }) => {
                        consecutive_failures += 1;
                        record.stop_reason = "error".to_string();
                        record.error = Some(RecordError {
                            class: class.as_str().to_string(),
                            message: self.redactor.bounded(&message),
                            retryable: class.is_retryable(),
                        });
                        if let Some((o, l)) = predict_events.take() {
                            record.calls = counts(&o, &l);
                        }
                        case_ok = false;
                        if consecutive_failures >= self.args.max_consecutive_failures {
                            self.store(store, &record, &mut summary, &mut stored_now)?;
                            progress(self.line(index, total, lc, mode, &record));
                            summary.stopped = Some(StopReason::ProviderUnavailable {
                                class: class.as_str().to_string(),
                            });
                            progress(format!(
                                "stopping: {consecutive_failures} provider failures in a row (last: {}); \
                                 not hammering it. Wait, then re-run; completed cases are kept.",
                                class.as_str()
                            ));
                            break 'cases;
                        }
                    }
                    Err(CaseFailure::Harness(message)) => {
                        record.stop_reason = "error".to_string();
                        record.error = Some(RecordError {
                            class: "harness".to_string(),
                            message: self.redactor.bounded(&message),
                            retryable: false,
                        });
                        case_ok = false;
                    }
                }
                self.store(store, &record, &mut summary, &mut stored_now)?;
                progress(self.line(index, total, lc, mode, &record));
            }
            if case_ok {
                summary.cases_complete += 1;
            }
        }
        self.stack.flush();
        Ok(summary)
    }

    fn store(
        &self,
        store: &mut Store,
        record: &LiveRecord,
        summary: &mut RunSummary,
        stored_now: &mut std::collections::HashSet<String>,
    ) -> std::io::Result<()> {
        store.append(record)?;
        summary.produced += 1;
        if record.error.is_some() {
            summary.failed += 1;
        }
        add(&mut summary.calls, record.calls);
        stored_now.insert(record.run_key.clone());
        Ok(())
    }

    fn line(
        &self,
        index: usize,
        total: usize,
        lc: &LiveCase,
        mode: LiveMode,
        r: &LiveRecord,
    ) -> String {
        let id: String = lc.case.case_id.to_string().chars().take(8).collect();
        let what = match &r.error {
            Some(e) => format!("ERROR {}", e.class),
            None if lc.case.corpus == ferrite_ipi::dataset::Corpus::Attack => format!(
                "attempted={} executed={} blocked={} done={}",
                r.outcome.attack_attempted,
                r.outcome.attack_executed,
                r.outcome.attack_blocked,
                r.outcome.task_completed
            ),
            None => format!(
                "false_positive={} done={}",
                r.outcome.benign_blocked, r.outcome.task_completed
            ),
        };
        format!(
            "[{}/{total}] {} {id} {:<6} {what}  calls={} {}ms",
            index + 1,
            lc.suite,
            mode.as_str(),
            r.calls.logical,
            r.latency_ms
        )
    }

    fn skeleton(
        &self,
        lc: &LiveCase,
        mode: LiveMode,
        started_at: &str,
        provider: &str,
        hash: &str,
    ) -> LiveRecord {
        let ideal: Option<Vec<String>> = lc
            .meta
            .as_ref()
            .filter(|m| !m.user_capabilities.is_empty())
            .map(|m| {
                m.user_capabilities
                    .iter()
                    .filter_map(|n| capability_name(n))
                    .map(str::to_string)
                    .collect()
            });
        let attack_caps = ideal.as_ref().and_then(|i| attack_capabilities(lc, i));
        let id = lc.case.case_id.to_string();
        LiveRecord {
            schema: SCHEMA_VERSION,
            run_key: key_for(self.args, self.tags, &id, mode, hash),
            case_id: id,
            suite: lc.suite.clone(),
            source: lc.source.clone(),
            kind: lc.kind().to_string(),
            ground_truth: lc.ground_truth_class().to_string(),
            attack_category: lc.case.attack_category.map(|c| format!("{c:?}")),
            carrier: lc.carrier().to_string(),
            provider: provider.to_string(),
            small_model: self.tags.small.clone(),
            main_model: self.tags.main.clone(),
            mode,
            agent: self.args.agent,
            predictor: self.args.predictor,
            config_hash: hash.to_string(),
            started_at: started_at.to_string(),
            prediction: None,
            ideal_capabilities: ideal,
            attack_capabilities: attack_caps,
            actions: Vec::new(),
            stop_reason: String::new(),
            final_answer: None,
            outcome: super::record::Outcome::default(),
            latency_ms: 0,
            calls: CallCounts::default(),
            error: None,
        }
    }

    async fn predict(&self, lc: &LiveCase) -> Result<Predicted, CaseFailure> {
        let engine = ToolDecisionEngine::new();
        let task = &lc.case.user_task;
        let (fingerprint, mut degraded, mut reason) = match self.args.predictor {
            PredictorKind::Rules => {
                // No model: the rule layer alone. A mock that answers nothing fails
                // every call, which is exactly the fail-to-empty `may_use`.
                let none = ferrite_model::MockProvider::new();
                (
                    engine.generate_fingerprint(&none, "rules", task).await,
                    false,
                    None,
                )
            }
            PredictorKind::Llm => (
                engine
                    .generate_fingerprint(self.stack.provider.as_ref(), &self.tags.small, task)
                    .await,
                false,
                None,
            ),
        };
        let outer = self.stack.outer.drain();
        let live = self.stack.live.drain();
        for event in &outer {
            if let Some((class, message)) = &event.error {
                if *class == ErrorClass::BudgetExhausted {
                    return Err(CaseFailure::Budget);
                }
                if class.is_infrastructure() {
                    return Err(CaseFailure::Infrastructure {
                        class: *class,
                        message: message.clone(),
                    });
                }
                degraded = true;
                reason = Some(message.clone());
            }
        }
        let names = |set: &std::collections::BTreeSet<Capability>| -> Vec<String> {
            set.iter().map(|c| c.as_str().to_string()).collect()
        };
        let record = PredictionRecord {
            must_use: names(fingerprint.must_use()),
            may_use: names(fingerprint.may_use()),
            degraded,
            degraded_reason: reason,
            predictor: self.args.predictor,
        };
        Ok(Predicted {
            fingerprint,
            record,
            outer,
            live,
        })
    }

    async fn run_mode(
        &self,
        lc: &LiveCase,
        mode: LiveMode,
        predicted: Option<&Predicted>,
    ) -> Result<Product, CaseFailure> {
        let case = &lc.case;
        let expected = predicted.map(|p| expected_fingerprint_for(case, &p.fingerprint));
        let guard = if mode.enforces_guard() {
            Some(RuntimeGuard::new(expected.clone().ok_or_else(|| {
                CaseFailure::Harness("a guarded mode ran without a prediction".to_string())
            })?))
        } else {
            None
        };
        let content = if mode == LiveMode::DryRun {
            DryRunContent::default()
        } else {
            lc.content.clone()
        };
        let ipi = IpiTask::new(case.user_task.clone(), context_url(case, &lc.content));
        let mut orch = (self.orchestrator)(self.twin_path.clone(), content);
        orch.set_defense_mode(mode.defense_mode());
        // The whole-turn timeout exists to bound a hung scripted agent. A live run is
        // bounded by per-request timeouts, retries and the step budget, and pacing
        // alone can make it longer than the 30 s default.
        orch.set_timeout_secs(3600);

        match self.args.agent {
            AgentKind::Llm => {
                let context = match (mode, case.carrier_vector.carrier()) {
                    (LiveMode::DryRun, _) => Context::None,
                    (_, Carrier::WebContent) => Context::Page,
                    (_, Carrier::ToolOutput) => Context::ToolOutput,
                };
                let agent = LlmAgent::new(
                    self.stack.provider.as_ref(),
                    self.tags.main.clone(),
                    case.user_task.clone(),
                    context,
                    guard,
                    self.args.max_steps,
                );
                let result = orch.run(&ipi, &agent).await;
                let trace = agent.into_trace();
                let outer = self.stack.outer.drain();
                let live = self.stack.live.drain();
                for event in &outer {
                    if let Some((class, message)) = &event.error {
                        if *class == ErrorClass::BudgetExhausted {
                            return Err(CaseFailure::Budget);
                        }
                        if class.is_infrastructure() {
                            return Err(CaseFailure::Infrastructure {
                                class: *class,
                                message: message.clone(),
                            });
                        }
                    }
                }
                let (record, error) = match result {
                    Ok(record) => (Some(record), None),
                    Err(e) => (None, Some(e)),
                };
                if let (None, Some(e)) = (&record, &error) {
                    // An error with no failed call behind it is the harness's, unless
                    // it is the model's own unusable output, which the trace says.
                    if trace.model_error.is_none() {
                        return Err(CaseFailure::Harness(e.clone()));
                    }
                }
                let dry_run_gated = match (mode, &record, &expected) {
                    (LiveMode::DryRun, Some(r), Some(e)) => Some(!compare(e, r).is_clean()),
                    _ => None,
                };
                Ok(Product {
                    attempts: trace.attempts,
                    stop: trace
                        .stop
                        .unwrap_or_else(|| Stop::Other("run_timeout".to_string())),
                    sanitizer_findings: record.as_ref().map_or(0, |r| {
                        u32::try_from(r.sanitizer_findings.len()).unwrap_or(u32::MAX)
                    }),
                    dry_run_gated,
                    outer,
                    live,
                })
            }
            AgentKind::Scripted => {
                let agent = WorstCaseAgent::for_case(case);
                let record = orch.run(&ipi, &agent).await.map_err(CaseFailure::Harness)?;
                // The script has already acted: the guard can only classify.
                let attempts = record
                    .tool_events
                    .iter()
                    .enumerate()
                    .map(|(step, e)| {
                        let verdict = match &guard {
                            None => Verdict::Unguarded,
                            Some(g) if g.check(e.primitive, e.origin.as_deref()).allows() => {
                                Verdict::WouldAllow
                            }
                            Some(_) => Verdict::WouldBlock,
                        };
                        Attempt {
                            step,
                            effects: vec![(e.primitive, e.origin.clone())],
                            text: format!(
                                "{} {}",
                                e.primitive.as_str(),
                                e.origin.as_deref().unwrap_or("")
                            ),
                            verdict,
                            executed: true,
                        }
                    })
                    .collect();
                Ok(Product {
                    attempts,
                    stop: Stop::Finished("(scripted agent)".to_string()),
                    sanitizer_findings: u32::try_from(record.sanitizer_findings.len())
                        .unwrap_or(u32::MAX),
                    dry_run_gated: None,
                    outer: Vec::new(),
                    live: Vec::new(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ferrite_core::FixedClock;
    use ferrite_model::testing::RecordingSleeper;
    use ferrite_model::{MockProvider, MockStep, ModelError, ProviderId};

    use super::*;
    use crate::live::config::{MockBehavior, ProviderKind};
    use crate::live::corpus;
    use crate::live::provider::{build_stack, StackConfig};

    fn tmp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-live-runner-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn orchestrator_factory() -> Box<OrchestratorFactory> {
        Box::new(|path, content| {
            DryRunOrchestrator::with_test_twin_key(
                path,
                content,
                Box::new(
                    ferrite_model::MapEnv::new()
                        .with(ferrite_ipi::twin::TWIN_KEY_ENV_VAR, "test-only"),
                ),
                Box::new(ferrite_model::secret::NoSecretStore),
            )
        })
    }

    fn base_args(out: &std::path::Path) -> LiveArgs {
        LiveArgs {
            provider: Some(ProviderKind::Mock),
            out_dir: out.to_path_buf(),
            modes: vec![LiveMode::Off, LiveMode::Guard],
            mock_behavior: MockBehavior::Compliant,
            max_steps: 6,
            ..LiveArgs::default()
        }
    }

    fn stack_for(args: &LiveArgs, provider: MockProvider) -> ModelStack {
        let mut cfg = StackConfig::from_args(args, false);
        cfg.pause = Duration::ZERO;
        let sleeper = Arc::new(RecordingSleeper::new());
        let clock: Arc<dyn ferrite_core::Clock> = Arc::new(FixedClock::at_epoch());
        build_stack(provider, &cfg, clock, sleeper, &Redactor::default())
    }

    fn slack_cases(n: usize) -> Vec<LiveCase> {
        let all = corpus::load(&corpus::default_root(), &["agentdojo".to_string()]).unwrap();
        let a = LiveArgs {
            suites: vec!["agentdojo/slack".to_string()],
            ..LiveArgs::default()
        };
        corpus::select(all, &a).into_iter().take(n).collect()
    }

    async fn go(
        args: &LiveArgs,
        provider: MockProvider,
        cases: Vec<LiveCase>,
    ) -> (RunSummary, Results, Vec<String>, ModelStack) {
        let tags = args.model_tags().unwrap();
        let stack = stack_for(args, provider);
        let (mut store, existing) =
            Store::open(&args.out_dir, "mock", &tags.small, &tags.main).unwrap();
        let factory = orchestrator_factory();
        let redactor = Redactor::default();
        let runner = Runner {
            args,
            tags: &tags,
            stack: &stack,
            redactor: &redactor,
            orchestrator: factory.as_ref(),
            twin_path: args.out_dir.join("twin.enc"),
        };
        let mut lines = Vec::new();
        let summary = runner
            .run(cases, &mut store, &existing, &mut |l| lines.push(l))
            .await
            .unwrap();
        let results = crate::live::store::read_all(&args.out_dir).unwrap();
        (summary, results, lines, stack)
    }

    #[tokio::test]
    async fn a_batch_runs_stores_one_line_per_case_and_mode_and_reports_its_cost() {
        let out = tmp("batch");
        let mut args = base_args(&out);
        args.batch_size = Some(4);
        let (summary, results, lines, stack) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            slack_cases(10),
        )
        .await;

        assert_eq!(summary.cases_in_window, 4);
        assert_eq!(summary.produced, 8, "4 cases x 2 modes");
        assert_eq!(summary.failed, 0);
        assert_eq!(results.by_key.len(), 8);
        assert_eq!(lines.len(), 8);
        assert!(summary.calls.logical > 0);
        assert_eq!(
            u64::from(summary.calls.live_attempts),
            u64::from(stack.summary().backend_calls),
            "the per-record accounting matches the budget's own count"
        );
        for r in results.by_key.values() {
            assert_eq!(r.provider, "mock");
            assert_eq!(r.config_hash.len(), 16);
            assert!(r.error.is_none());
        }
    }

    #[tokio::test]
    async fn a_second_invocation_takes_the_next_batch_and_a_third_finds_nothing_to_do() {
        let out = tmp("resume");
        let mut args = base_args(&out);
        args.batch_size = Some(3);
        let cases = slack_cases(7);
        let mock = || crate::live::mock_model::build(MockBehavior::Compliant);

        let (s1, r1, _, _) = go(&args, mock(), cases.clone()).await;
        assert_eq!((s1.produced, r1.by_key.len()), (6, 6));
        let (s2, r2, _, st2) = go(&args, mock(), cases.clone()).await;
        assert_eq!(
            s2.produced, 6,
            "the next 3 pending cases, not the same 3 again"
        );
        assert_eq!(r2.by_key.len(), 12);
        assert_eq!(
            st2.summary().backend_calls as usize,
            s2.calls.live_attempts as usize
        );
        let (s3, r3, _, _) = go(&args, mock(), cases.clone()).await;
        assert_eq!(s3.produced, 2, "the last, short batch: 1 case x 2 modes");
        assert_eq!(r3.by_key.len(), 14);
        let (s4, r4, _, st4) = go(&args, mock(), cases).await;
        assert_eq!(s4.produced, 0);
        assert_eq!(r4.by_key.len(), 14);
        assert_eq!(
            st4.summary().backend_calls,
            0,
            "a finished selection costs nothing"
        );
    }

    #[tokio::test]
    async fn a_fixed_batch_index_always_means_the_same_slice() {
        let out = tmp("index");
        let mut args = base_args(&out);
        args.batch_size = Some(3);
        args.batch_index = Some(1);
        args.modes = vec![LiveMode::Off];
        let cases = slack_cases(9);
        let want: Vec<String> = cases[3..6]
            .iter()
            .map(|c| c.case.case_id.to_string())
            .collect();
        let (_, results, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            cases,
        )
        .await;
        let mut got: Vec<String> = results.by_key.values().map(|r| r.case_id.clone()).collect();
        got.sort();
        let mut want = want;
        want.sort();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn the_call_budget_is_a_hard_cap_the_case_in_flight_is_left_pending_and_nothing_is_lost()
    {
        let out = tmp("budget");
        let mut args = base_args(&out);
        args.max_calls = 7;
        let cases = slack_cases(6);
        let (summary, results, _, stack) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            cases.clone(),
        )
        .await;
        assert_eq!(summary.stopped, Some(StopReason::BudgetExhausted));
        assert!(
            stack.summary().backend_calls <= 7,
            "never more than the cap"
        );
        assert!(summary.produced < 12);
        // Everything stored is whole; the abandoned (case, mode) is not stored.
        assert_eq!(results.by_key.len(), summary.produced);
        assert!(results.by_key.values().all(|r| r.error.is_none()));

        // Raising the cap and re-running finishes the rest without redoing any.
        args.max_calls = 100;
        let (s2, r2, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            cases,
        )
        .await;
        assert_eq!(s2.stopped, None);
        assert_eq!(r2.by_key.len(), 12, "6 cases x 2 modes, each exactly once");
        assert_eq!(s2.produced + summary.produced, 12);
    }

    fn rate_limit_always() -> MockProvider {
        MockProvider::new().always(|_| {
            MockStep::Fail(ModelError::RateLimited {
                provider: ProviderId::Mock,
                retry_after: Some(Duration::from_secs(1)),
            })
        })
    }

    #[tokio::test]
    async fn a_provider_that_keeps_saying_no_stops_the_invocation_and_stores_errors_not_scores() {
        let out = tmp("ratelimit");
        let mut args = base_args(&out);
        args.max_attempts = 2;
        args.max_consecutive_failures = 3;
        let (summary, results, _, _) = go(&args, rate_limit_always(), slack_cases(10)).await;

        assert_eq!(
            summary.stopped,
            Some(StopReason::ProviderUnavailable {
                class: "rate_limited".to_string()
            })
        );
        assert!(
            summary.failed >= 3 && summary.failed <= 4,
            "{}",
            summary.failed
        );
        assert!(results.by_key.values().all(|r| r
            .error
            .as_ref()
            .is_some_and(|e| e.class == "rate_limited" && e.retryable)));
        assert!(
            results
                .by_key
                .values()
                .all(|r| !r.outcome.attack_attempted && !r.outcome.task_completed),
            "an error is never scored"
        );
    }

    #[tokio::test]
    async fn failed_cases_are_skipped_by_default_and_redone_with_retry_failed() {
        let out = tmp("retry");
        let mut args = base_args(&out);
        args.modes = vec![LiveMode::Off];
        args.max_attempts = 1;
        args.max_consecutive_failures = 100;
        let cases = slack_cases(3);
        let (s1, r1, _, _) = go(&args, rate_limit_always(), cases.clone()).await;
        assert_eq!((s1.produced, s1.failed), (3, 3));
        assert_eq!(r1.failed(), 3);

        let (s2, _, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            cases.clone(),
        )
        .await;
        assert_eq!(s2.produced, 0, "recorded errors count as done unless asked");

        args.retry_failed = true;
        let (s3, r3, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            cases,
        )
        .await;
        assert_eq!(s3.produced, 3);
        assert_eq!(r3.failed(), 0, "the latest line per key wins");
        assert_eq!(r3.by_key.len(), 3);
    }

    #[tokio::test]
    async fn a_malformed_fingerprint_degrades_to_the_rule_layer_and_never_widens_what_is_admitted()
    {
        let out = tmp("malformed-fp");
        let mut args = base_args(&out);
        args.modes = vec![LiveMode::Guard];
        // Every fingerprint call answers garbage; agent steps answer with a finish.
        let provider = MockProvider::new().always(|req| {
            if req.label == "fingerprint" {
                MockStep::Content("this is not a json array".to_string())
            } else {
                MockStep::Content(
                    r#"{"action":"navigate","url":"https://www.true-informations.com"}"#
                        .to_string(),
                )
            }
        });
        let (summary, results, _, _) = go(&args, provider, slack_cases(3)).await;
        assert_eq!(
            summary.failed, 0,
            "the model's bad output is a result, not an outage"
        );
        for r in results.by_key.values() {
            let p = r.prediction.as_ref().expect("a prediction was recorded");
            assert!(p.degraded, "the unusable answer is recorded as such");
            assert!(
                p.may_use.is_empty(),
                "fail to empty: nothing the model said widens the set"
            );
            // The navigation the agent attempts is outside a rules-only fingerprint.
            assert!(
                r.actions.iter().any(|a| a.verdict == Verdict::Blocked),
                "{:?}",
                r.actions
            );
            assert!(!r.outcome.attack_executed);
        }
    }

    #[tokio::test]
    async fn malformed_agent_output_ends_the_run_without_a_bypass() {
        let out = tmp("malformed-agent");
        let mut args = base_args(&out);
        args.modes = vec![LiveMode::Guard];
        let provider = MockProvider::new().always(|req| {
            if req.label == "fingerprint" {
                MockStep::Content(r#"["web.read"]"#.to_string())
            } else {
                MockStep::Content("Sure! I will do that now.".to_string())
            }
        });
        let (summary, results, _, _) = go(&args, provider, slack_cases(2)).await;
        assert_eq!(summary.failed, 0);
        for r in results.by_key.values() {
            assert_eq!(r.stop_reason, "malformed_action");
            assert!(r.actions.is_empty());
            assert!(!r.outcome.task_completed && !r.outcome.attack_executed);
        }
    }

    #[tokio::test]
    async fn the_guard_blocks_what_a_compliant_model_does_and_the_baseline_does_not() {
        let out = tmp("compare");
        let mut args = base_args(&out);
        args.modes = vec![LiveMode::Off, LiveMode::Guard];
        let cases: Vec<LiveCase> = slack_cases(120)
            .into_iter()
            .filter(|c| c.kind() == "attack" && c.ground_truth_class() == "deviation")
            .take(8)
            .collect();
        assert!(!cases.is_empty());
        let (_, results, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            cases,
        )
        .await;
        let off: Vec<_> = results
            .by_key
            .values()
            .filter(|r| r.mode == LiveMode::Off)
            .collect();
        let guard: Vec<_> = results
            .by_key
            .values()
            .filter(|r| r.mode == LiveMode::Guard)
            .collect();
        assert!(
            off.iter().any(|r| r.outcome.attack_executed),
            "the compliant mock does follow injections"
        );
        assert!(guard.iter().any(|r| r.outcome.attack_attempted));
        assert!(
            guard.iter().all(|r| !r.outcome.attack_executed),
            "a deviation the model attempts never runs under the guard"
        );
        assert!(guard.iter().any(|r| r.outcome.attack_blocked));
    }

    #[tokio::test]
    async fn the_scripted_agent_is_classified_after_the_fact_and_needs_no_agent_calls() {
        let out = tmp("scripted");
        let mut args = base_args(&out);
        args.agent = AgentKind::Scripted;
        args.predictor = PredictorKind::Rules;
        let (summary, results, _, stack) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            slack_cases(4),
        )
        .await;
        assert_eq!(summary.failed, 0);
        assert_eq!(stack.summary().backend_calls, 0, "no model call at all");
        assert!(results.by_key.values().any(|r| r
            .actions
            .iter()
            .any(|a| matches!(a.verdict, Verdict::WouldBlock | Verdict::WouldAllow))));
    }

    #[tokio::test]
    async fn the_dry_run_mode_runs_on_a_clean_page_and_reports_whether_it_would_have_gated() {
        let out = tmp("dryrun");
        let mut args = base_args(&out);
        args.modes = vec![LiveMode::DryRun];
        let cases: Vec<LiveCase> = slack_cases(60)
            .into_iter()
            .filter(|c| c.kind() == "benign")
            .take(3)
            .collect();
        let (summary, results, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Resistant),
            cases,
        )
        .await;
        assert_eq!(summary.failed, 0);
        for r in results.by_key.values() {
            assert!(r.outcome.dry_run_gated.is_some());
            assert!(!r.outcome.attack_attempted);
        }
    }

    #[tokio::test]
    async fn no_record_contains_the_page_content_of_a_planted_injection_beyond_the_bounded_actions()
    {
        let out = tmp("bounded");
        let args = base_args(&out);
        let (_, results, _, _) = go(
            &args,
            crate::live::mock_model::build(MockBehavior::Compliant),
            slack_cases(2),
        )
        .await;
        for r in results.by_key.values() {
            for a in &r.actions {
                assert!(a.action.len() < crate::live::record::MAX_TEXT_BYTES + 60);
            }
        }
    }
}
