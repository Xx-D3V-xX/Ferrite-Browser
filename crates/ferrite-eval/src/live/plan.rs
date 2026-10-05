//! `--plan`: what a selection would cost, estimated without calling anything.
//!
//! The point is to know *before* a run whether it fits a quota. The counts of
//! calls are exact bounds, not guesses: a fingerprint prediction is one call; an
//! agent run is at least one call (it finishes at once) and at most
//! `3 * max_steps` (every step, each preceded by two unparseable replies the
//! loop's self-correction allows). The **typical** figure is an assumption, stated
//! as one (a model that does a couple of actions and finishes), and the token
//! figures are `characters / 4` over the real prompts the run would send: good
//! to within a factor of two across tokenizers, and nothing more.
//!
//! What it cannot know, and says so: how many calls the response cache will
//! absorb, how many steps a given model really takes, and a provider's real
//! tokenization. All three only ever make the real run cheaper than the upper
//! bound, never dearer than it.

use std::fmt::Write as _;

use ferrite_agent::browser_loop::{MAX_CONSECUTIVE_MALFORMED_STEPS, SYSTEM_PROMPT};
use ferrite_ipi::dry_run::DryRunContent;

use super::config::{AgentKind, LiveArgs, LiveMode, PredictorKind};
use super::corpus::LiveCase;
use super::record::{config_hash, run_key, BehaviourConfig};

/// Steps a typical run is assumed to take (the actions plus the final answer).
pub const TYPICAL_AGENT_CALLS: u64 = 3;
/// Output tokens assumed per agent step (one JSON action).
const OUT_TOKENS_PER_STEP: u64 = 60;
/// Output tokens assumed for the final answer.
const OUT_TOKENS_FINAL: u64 = 150;
/// Tokens each executed step adds to the conversation (the action and its
/// observation).
const HISTORY_TOKENS_PER_STEP: u64 = 150;
/// Prompt and output tokens of one fingerprint call.
const FINGERPRINT_TOKENS: (u64, u64) = (160, 20);

/// A count with a typical value and an upper bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Range {
    /// What a typical run would use.
    pub typical: u64,
    /// The most it can use.
    pub max: u64,
}

impl Range {
    fn add(&mut self, other: Self) {
        self.typical += other.typical;
        self.max += other.max;
    }
}

/// The estimate.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Cases in the window.
    pub cases: usize,
    /// `(mode, runs still to do)`.
    pub pending_runs: Vec<(LiveMode, usize)>,
    /// `(case, mode)` results already stored and reused.
    pub done_runs: usize,
    /// Stored results that are errors and will be redone (`--retry-failed`) or
    /// left (default).
    pub failed_runs: usize,
    /// Fingerprint predictions the runs need.
    pub prediction_calls: Range,
    /// Agent calls.
    pub agent_calls: Range,
    /// Prompt tokens, all calls.
    pub input_tokens: Range,
    /// Completion tokens, all calls.
    pub output_tokens: Range,
    /// Cases per suite in the window.
    pub by_suite: Vec<(String, usize)>,
}

impl Plan {
    /// All model calls.
    #[must_use]
    pub fn calls(&self) -> Range {
        let mut total = self.prediction_calls;
        total.add(self.agent_calls);
        total
    }
}

/// Characters of content a case's dry-run engine would serve.
fn content_chars(content: &DryRunContent) -> usize {
    let mut content = content.clone();
    let mut total = 0;
    let origins: Vec<String> = content
        .read_page
        .origins()
        .into_iter()
        .chain(content.extract_data.origins())
        .map(str::to_string)
        .collect();
    for origin in origins {
        while let Some(reply) = content.read_page.next(Some(&origin)) {
            total += reply_len(&reply);
        }
        while let Some(reply) = content.extract_data.next(Some(&origin)) {
            total += reply_len(&reply);
        }
    }
    total
}

fn reply_len(reply: &ferrite_ipi::dry_run::DryRunReply) -> usize {
    match reply {
        ferrite_ipi::dry_run::DryRunReply::Ok(v) => v.to_string().len(),
        ferrite_ipi::dry_run::DryRunReply::Err(e) => e.len(),
    }
}

/// The behaviour-relevant settings of `args`, hashed with the others the result key
/// carries. Shared by the runner and the plan so they agree on what is "done".
#[must_use]
pub fn behaviour_hash(args: &LiveArgs) -> String {
    config_hash(&BehaviourConfig {
        harness_version: super::record::HARNESS_VERSION,
        schema: super::record::SCHEMA_VERSION,
        predictor: args.predictor,
        agent: args.agent,
        max_steps: args.max_steps,
        agent_prompt_version: ferrite_agent::browser_loop::SYSTEM_PROMPT_VERSION,
        mock_behavior: (args.provider == Some(super::config::ProviderKind::Mock))
            .then(|| format!("{:?}", args.mock_behavior)),
        sampling: "temperature0-seed42".to_string(),
    })
}

/// Whether `(case, mode)` still needs a run, given what is stored.
#[must_use]
pub fn needs_run(existing: &super::store::Results, key: &str, retry_failed: bool) -> bool {
    !existing.has(key) || (retry_failed && !existing.succeeded(key))
}

/// The key of one `(case, mode)` result under `args` and `tags`.
#[must_use]
pub fn key_for(
    args: &LiveArgs,
    tags: &super::config::ModelTags,
    case_id: &str,
    mode: LiveMode,
    hash: &str,
) -> String {
    run_key(
        case_id,
        args.provider
            .map_or("mock", super::config::ProviderKind::as_str),
        &tags.small,
        &tags.main,
        mode,
        hash,
    )
}

/// Whether any requested mode of `case` still needs a run. The runner and the plan
/// both use this, so "the next 25 pending cases" means the same cases to each.
#[must_use]
pub fn is_pending(
    args: &LiveArgs,
    tags: &super::config::ModelTags,
    existing: &super::store::Results,
    hash: &str,
    case: &LiveCase,
) -> bool {
    let id = case.case.case_id.to_string();
    args.modes.iter().any(|m| {
        needs_run(
            existing,
            &key_for(args, tags, &id, *m, hash),
            args.retry_failed,
        )
    })
}

/// Estimates `cases` (already windowed) against what is stored.
#[must_use]
pub fn estimate(
    args: &LiveArgs,
    tags: &super::config::ModelTags,
    cases: &[LiveCase],
    existing: &super::store::Results,
) -> Plan {
    let hash = behaviour_hash(args);
    let system_tokens = (SYSTEM_PROMPT.len() as u64).div_ceil(4);
    let max_calls_per_run =
        args.max_steps as u64 * (1 + u64::from(MAX_CONSECUTIVE_MALFORMED_STEPS));
    let mut plan = Plan {
        cases: cases.len(),
        ..Plan::default()
    };
    let mut by_suite: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut per_mode: std::collections::BTreeMap<LiveMode, usize> =
        std::collections::BTreeMap::new();

    for case in cases {
        *by_suite.entry(case.suite.clone()).or_default() += 1;
        let id = case.case.case_id.to_string();
        let mut needs_prediction = false;
        for mode in &args.modes {
            let key = key_for(args, tags, &id, *mode, &hash);
            if existing.has(&key) {
                plan.done_runs += 1;
                if !existing.succeeded(&key) {
                    plan.failed_runs += 1;
                }
            }
            if !needs_run(existing, &key, args.retry_failed) {
                continue;
            }
            *per_mode.entry(*mode).or_default() += 1;
            needs_prediction |= mode.needs_prediction();

            if args.agent == AgentKind::Llm {
                let seed_tokens = if *mode == LiveMode::DryRun {
                    (case.case.user_task.len() as u64).div_ceil(4)
                } else {
                    ((case.case.user_task.len() + content_chars(&case.content)) as u64).div_ceil(4)
                        + 150
                };
                let first = system_tokens + seed_tokens;
                let typical_in: u64 = (0..TYPICAL_AGENT_CALLS)
                    .map(|i| first + i * HISTORY_TOKENS_PER_STEP)
                    .sum();
                let max_in: u64 = (0..max_calls_per_run)
                    .map(|i| {
                        first
                            + (i / (1 + u64::from(MAX_CONSECUTIVE_MALFORMED_STEPS)))
                                * HISTORY_TOKENS_PER_STEP
                    })
                    .sum();
                plan.agent_calls.add(Range {
                    typical: TYPICAL_AGENT_CALLS,
                    max: max_calls_per_run,
                });
                plan.input_tokens.add(Range {
                    typical: typical_in,
                    max: max_in,
                });
                plan.output_tokens.add(Range {
                    typical: (TYPICAL_AGENT_CALLS - 1) * OUT_TOKENS_PER_STEP + OUT_TOKENS_FINAL,
                    max: max_calls_per_run * OUT_TOKENS_PER_STEP,
                });
            }
        }
        if needs_prediction && args.predictor == PredictorKind::Llm {
            plan.prediction_calls.add(Range { typical: 1, max: 1 });
            plan.input_tokens.add(Range {
                typical: FINGERPRINT_TOKENS.0,
                max: FINGERPRINT_TOKENS.0,
            });
            plan.output_tokens.add(Range {
                typical: FINGERPRINT_TOKENS.1,
                max: FINGERPRINT_TOKENS.1,
            });
        }
    }
    plan.pending_runs = per_mode.into_iter().collect();
    plan.by_suite = by_suite.into_iter().collect();
    plan
}

/// The plan as text.
#[must_use]
pub fn render(
    plan: &Plan,
    args: &LiveArgs,
    tags: &super::config::ModelTags,
    total_in_selection: usize,
) -> String {
    let mut out = String::new();
    let provider = args
        .provider
        .map_or("(none)", super::config::ProviderKind::as_str);
    let _ = writeln!(out, "PLAN (nothing was called)");
    let _ = writeln!(
        out,
        "  provider {provider}   predictor {:?} ({})   agent {:?} ({})",
        args.predictor,
        if args.predictor == PredictorKind::Llm {
            &tags.small
        } else {
            "no model"
        },
        args.agent,
        if args.agent == AgentKind::Llm {
            &tags.main
        } else {
            "no model"
        },
    );
    let _ = writeln!(
        out,
        "  modes {}   max steps {}",
        args.modes
            .iter()
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join(","),
        args.max_steps
    );
    let _ = writeln!(
        out,
        "  window: {} case(s) of {} selected (corpus {})",
        plan.cases,
        total_in_selection,
        args.corpora.join(",")
    );
    for (suite, n) in &plan.by_suite {
        let _ = writeln!(out, "    {suite}: {n}");
    }
    let pending: usize = plan.pending_runs.iter().map(|(_, n)| n).sum();
    let _ = writeln!(
        out,
        "  runs to do: {pending} ({})   already stored: {} ({} of them errors{})",
        plan.pending_runs
            .iter()
            .map(|(m, n)| format!("{} x{n}", m.as_str()))
            .collect::<Vec<_>>()
            .join(", "),
        plan.done_runs,
        plan.failed_runs,
        if args.retry_failed {
            ", will be redone"
        } else {
            ", left alone; --retry-failed redoes them"
        },
    );
    let calls = plan.calls();
    let _ = writeln!(out, "  model calls:");
    let _ = writeln!(
        out,
        "    fingerprint predictions   {:>7}",
        plan.prediction_calls.max
    );
    let _ = writeln!(
        out,
        "    agent steps               {:>7} typical   {:>7} at most",
        plan.agent_calls.typical, plan.agent_calls.max
    );
    let _ = writeln!(
        out,
        "    total                     {:>7} typical   {:>7} at most   (cap this invocation: {})",
        calls.typical, calls.max, args.max_calls
    );
    let _ = writeln!(
        out,
        "  tokens (characters/4, rough): prompt {} typical / {} at most; completion {} typical / {} at most",
        plan.input_tokens.typical, plan.input_tokens.max, plan.output_tokens.typical, plan.output_tokens.max
    );
    if calls.max > u64::from(args.max_calls) {
        let _ = writeln!(
            out,
            "  NOTE: the worst case exceeds --max-calls; a typical run needs about {} invocation(s) at this cap.",
            calls.typical.div_ceil(u64::from(args.max_calls).max(1))
        );
    } else {
        let _ = writeln!(out, "  fits --max-calls even in the worst case.");
    }
    if let Some(size) = args.batch_size {
        let _ = writeln!(
            out,
            "  batching: {size} case(s) per invocation -> {} invocation(s) for the whole selection",
            super::corpus::batch_count(total_in_selection, size)
        );
    }
    let _ = writeln!(
        out,
        "  not known until run: how many calls the response cache absorbs (a re-run of anything already\n  \
         run is free), how many steps this model really takes, and the provider's own tokenizer."
    );
    out
}

#[cfg(test)]
mod tests {
    use ferrite_ipi::dataset::Corpus;

    use super::*;
    use crate::live::config::{ModelTags, ProviderKind};
    use crate::live::store::Results;
    use crate::live::testing::{deviation, live_case, record};

    fn args() -> LiveArgs {
        LiveArgs {
            provider: Some(ProviderKind::Gemini),
            small_model: Some("s".into()),
            main_model: Some("m".into()),
            ..LiveArgs::default()
        }
    }

    fn tags() -> ModelTags {
        ModelTags {
            small: "s".into(),
            main: "m".into(),
        }
    }

    fn cases(n: usize) -> Vec<LiveCase> {
        (0..n)
            .map(|_| live_case(Corpus::Attack, deviation(&["form.fill"], &[]), &[]))
            .collect()
    }

    #[test]
    fn the_call_bounds_follow_from_the_modes_and_the_step_budget() {
        let a = args(); // modes off,guard; llm predictor + agent; 8 steps
        let plan = estimate(&a, &tags(), &cases(10), &Results::default());
        assert_eq!(plan.cases, 10);
        // One prediction per case (guard needs it), two agent runs per case.
        assert_eq!(plan.prediction_calls.max, 10);
        assert_eq!(plan.agent_calls.typical, 10 * 2 * TYPICAL_AGENT_CALLS);
        assert_eq!(
            plan.agent_calls.max,
            10 * 2 * 8 * 3,
            "steps x (1 + 2 retries)"
        );
        assert!(plan.calls().typical < plan.calls().max);
        assert!(plan.input_tokens.typical > 0 && plan.output_tokens.typical > 0);
        assert!(plan.input_tokens.max >= plan.input_tokens.typical);
    }

    #[test]
    fn off_mode_alone_needs_no_prediction_call() {
        let mut a = args();
        a.modes = vec![LiveMode::Off];
        let plan = estimate(&a, &tags(), &cases(4), &Results::default());
        assert_eq!(plan.prediction_calls.max, 0);
    }

    #[test]
    fn a_rules_predictor_and_a_scripted_agent_cost_nothing() {
        let mut a = args();
        a.predictor = PredictorKind::Rules;
        a.agent = AgentKind::Scripted;
        a.modes = vec![LiveMode::Off, LiveMode::Guard];
        let plan = estimate(&a, &tags(), &cases(4), &Results::default());
        assert_eq!(plan.calls(), Range::default());
    }

    #[test]
    fn stored_results_are_reused_and_failures_are_redone_only_on_request() {
        let mut a = args();
        a.modes = vec![LiveMode::Off];
        let cs = cases(3);
        let hash = behaviour_hash(&a);
        let mut existing = Results::default();
        for (i, c) in cs.iter().enumerate() {
            let key = key_for(
                &a,
                &tags(),
                &c.case.case_id.to_string(),
                LiveMode::Off,
                &hash,
            );
            let mut r = record(&key, "2026-01-01T00:00:00Z");
            if i == 0 {
                r.error = Some(crate::live::record::RecordError {
                    class: "rate_limited".into(),
                    message: "x".into(),
                    retryable: true,
                });
            }
            existing.by_key.insert(key, r);
        }
        let plan = estimate(&a, &tags(), &cs, &existing);
        assert_eq!((plan.done_runs, plan.failed_runs), (3, 1));
        assert_eq!(plan.agent_calls.typical, 0, "everything is already stored");
        a.retry_failed = true;
        let plan = estimate(&a, &tags(), &cs, &existing);
        assert_eq!(
            plan.pending_runs,
            vec![(LiveMode::Off, 1)],
            "only the failed one is redone"
        );
    }

    #[test]
    fn a_change_of_behaviour_is_a_different_key_so_nothing_stale_is_reused() {
        let a = args();
        let mut b = args();
        b.max_steps = 12;
        assert_ne!(behaviour_hash(&a), behaviour_hash(&b));
        let mut c = args();
        c.pause = std::time::Duration::from_secs(5);
        c.max_calls = 7;
        assert_eq!(
            behaviour_hash(&a),
            behaviour_hash(&c),
            "pacing is not behaviour"
        );
    }

    #[test]
    fn the_rendered_plan_says_what_it_does_not_know_and_whether_it_fits() {
        let mut a = args();
        a.max_calls = 5;
        a.batch_size = Some(25);
        let cs = cases(30);
        let plan = estimate(&a, &tags(), &cs, &Results::default());
        let text = render(&plan, &a, &tags(), 600);
        assert!(text.starts_with("PLAN (nothing was called)"));
        assert!(text.contains("exceeds --max-calls"), "{text}");
        assert!(
            text.contains("24 invocation(s)"),
            "600 cases in batches of 25: {text}"
        );
        assert!(text.contains("response cache"), "{text}");
        // Never prints a key or a secret-looking thing.
        assert!(!text.contains("API"), "{text}");
    }

    #[test]
    fn the_dry_run_prompt_is_the_task_alone_so_it_is_estimated_smaller() {
        let mut a = args();
        a.modes = vec![LiveMode::DryRun];
        let one = cases(1);
        let dry = estimate(&a, &tags(), &one, &Results::default());
        a.modes = vec![LiveMode::Guard];
        let real = estimate(&a, &tags(), &one, &Results::default());
        assert!(dry.input_tokens.typical < real.input_tokens.typical);
    }
}
