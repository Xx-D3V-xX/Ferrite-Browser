//! What `live_eval` does with parsed arguments: plan, run, or report.
//!
//! In the library rather than in the example so the whole command-line behaviour
//! (output, exit codes, files written) is tested offline, end to end, with the
//! `mock` provider.
//!
//! # Exit codes
//!
//! A script that runs batches in a loop needs to know *why* an invocation ended:
//!
//! | code | meaning | what a loop should do |
//! |---|---|---|
//! | 0 | the window finished, every result stored | go on to the next batch |
//! | 1 | an I/O or internal failure | stop and look |
//! | 2 | a usage or configuration problem (flags, missing key or model tag) | stop and fix it |
//! | 3 | `--max-calls` was spent | raise the cap or start a new session; nothing is lost |
//! | 4 | the provider kept failing (rate limit, outage) | wait, then run the same command again |
//! | 5 | the window finished but some cases failed | `--retry-failed` after a pause |

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use ferrite_core::Clock;
use ferrite_model::testing::Sleeper;
use ferrite_model::{EnvSource, SecretStore};

use super::config::{Command, LiveArgs, ModelTags, ProviderKind, HELP};
use super::corpus::{self, Window};
use super::plan;
use super::provider::connect;
use super::record::Redactor;
use super::report::{generate, ReportOptions};
use super::runner::{OrchestratorFactory, RunSummary, Runner, StopReason};
use super::store::{self, Results, Store};

/// The invocation finished cleanly.
pub const EXIT_OK: i32 = 0;
/// An I/O or internal failure.
pub const EXIT_INTERNAL: i32 = 1;
/// Bad flags or configuration.
pub const EXIT_USAGE: i32 = 2;
/// `--max-calls` was spent.
pub const EXIT_BUDGET: i32 = 3;
/// The provider kept failing.
pub const EXIT_PROVIDER: i32 = 4;
/// The window finished, with failed cases among its results.
pub const EXIT_SOME_FAILED: i32 = 5;

/// Everything the commands need from the outside world, injectable for tests.
pub struct Host<'a> {
    /// Where configuration variables come from.
    pub env: &'a dyn EnvSource,
    /// Where keys come from besides the environment (the OS keyring).
    pub secrets: &'a dyn SecretStore,
    /// Time.
    pub clock: Arc<dyn Clock>,
    /// Waiting.
    pub sleeper: Arc<dyn Sleeper>,
    /// How to build a dry-run orchestrator.
    pub orchestrator: &'a OrchestratorFactory,
}

fn say(out: &mut dyn Write, text: &str) {
    let _ = writeln!(out, "{text}");
}

fn provider_name(args: &LiveArgs) -> &'static str {
    args.provider.map_or("mock", ProviderKind::as_str)
}

/// Runs the command `args` selects; returns the process exit code.
pub async fn execute(args: LiveArgs, host: &Host<'_>, out: &mut dyn Write) -> i32 {
    match args.command {
        Command::Help => {
            say(out, HELP);
            EXIT_OK
        }
        Command::Report => report(&args, out),
        Command::Plan | Command::Run => plan_or_run(args, host, out).await,
    }
}

fn report(args: &LiveArgs, out: &mut dyn Write) -> i32 {
    let results = match store::read_all(&args.out_dir) {
        Ok(r) => r,
        Err(e) => {
            say(
                out,
                &format!("cannot read results under {}: {e}", args.out_dir.display()),
            );
            return EXIT_INTERNAL;
        }
    };
    let report = generate(
        &results,
        &ReportOptions {
            compare: args.compare.clone(),
        },
    );
    if report.groups == 0 {
        say(out, &report.markdown);
        return EXIT_OK;
    }
    let md = args.out_dir.join("REPORT.md");
    let csv = args.out_dir.join("report.csv");
    if let Err(e) =
        std::fs::write(&md, &report.markdown).and_then(|()| std::fs::write(&csv, &report.csv))
    {
        say(out, &format!("cannot write the report: {e}"));
        return EXIT_INTERNAL;
    }
    say(
        out,
        &format!(
            "report over {} run group(s), {} stored result(s):\n  {}\n  {}",
            report.groups,
            results.by_key.len(),
            md.display(),
            csv.display()
        ),
    );
    if results.unreadable_lines > 0 {
        say(
            out,
            &format!(
                "  note: {} unreadable line(s) were skipped (a torn write from an interrupted run)",
                results.unreadable_lines
            ),
        );
    }
    EXIT_OK
}

fn existing_results(args: &LiveArgs, tags: &ModelTags) -> std::io::Result<Results> {
    let path = store::results_dir(&args.out_dir).join(store::file_name(
        provider_name(args),
        &tags.small,
        &tags.main,
    ));
    store::read_file(&path)
}

async fn plan_or_run(args: LiveArgs, host: &Host<'_>, out: &mut dyn Write) -> i32 {
    if args.provider.is_none() {
        say(out, "choose a provider: --provider gemini|ollama|mock (or FERRITE_LIVE_PROVIDER). See --help.");
        return EXIT_USAGE;
    }
    let tags = match args.model_tags() {
        Ok(t) => t,
        Err(e) => {
            say(out, &format!("error: {e}"));
            return EXIT_USAGE;
        }
    };
    let root: PathBuf = args
        .corpus_root
        .clone()
        .unwrap_or_else(corpus::default_root);
    let cases = match corpus::load(&root, &args.corpora) {
        Ok(c) => corpus::select(c, &args),
        Err(e) => {
            say(out, &format!("error: {e}"));
            return EXIT_USAGE;
        }
    };
    if cases.is_empty() {
        say(
            out,
            "no case matches the selection (check --corpus, --suite, --only)",
        );
        return EXIT_USAGE;
    }
    let total = cases.len();
    let existing = match existing_results(&args, &tags) {
        Ok(r) => r,
        Err(e) => {
            say(out, &format!("cannot read stored results: {e}"));
            return EXIT_INTERNAL;
        }
    };
    let hash = plan::behaviour_hash(&args);

    if args.command == Command::Plan {
        let windowed = Window::of(&args).take(cases, |c| {
            plan::is_pending(&args, &tags, &existing, &hash, c)
        });
        let estimate = plan::estimate(&args, &tags, &windowed, &existing);
        say(out, &plan::render(&estimate, &args, &tags, total));
        return EXIT_OK;
    }

    // ── run ──
    let stack = match connect(
        &args,
        &tags,
        host.env,
        host.secrets,
        host.clock.clone(),
        host.sleeper.clone(),
    ) {
        Ok(s) => s,
        Err(e) => {
            // `ModelError`'s text never contains a key (ferrite-model redacts and
            // bounds everything it formats), and the redactor below is the net.
            let redactor = Redactor::from_env(host.env);
            say(out, &format!("error: {}", redactor.scrub(&e.to_string())));
            return EXIT_USAGE;
        }
    };
    let (mut store, existing) =
        match Store::open(&args.out_dir, provider_name(&args), &tags.small, &tags.main) {
            Ok(pair) => pair,
            Err(e) => {
                say(
                    out,
                    &format!(
                        "cannot open the result file under {}: {e}",
                        args.out_dir.display()
                    ),
                );
                return EXIT_INTERNAL;
            }
        };
    let redactor = Redactor::from_env(host.env);
    banner(out, &args, &tags, total, store.path());
    if existing.unreadable_lines > 0 {
        say(
            out,
            &format!("note: {} unreadable line(s) in the result file were skipped (an interrupted earlier write)", existing.unreadable_lines),
        );
    }
    let twin_path = args
        .out_dir
        .join(format!("twin-{}.enc", uuid::Uuid::new_v4()));
    let runner = Runner {
        args: &args,
        tags: &tags,
        stack: &stack,
        redactor: &redactor,
        orchestrator: host.orchestrator,
        twin_path: twin_path.clone(),
    };
    // Progress goes out as it happens, so an interrupted run shows how far it got.
    let result = runner
        .run(cases.clone(), &mut store, &existing, &mut |line| {
            say(out, &line)
        })
        .await;
    let _ = std::fs::remove_file(&twin_path);
    let summary = match result {
        Ok(s) => s,
        Err(e) => {
            say(out, &format!("could not store a result: {e}. Stopping: a result that cannot be saved would be paid for twice."));
            return EXIT_INTERNAL;
        }
    };
    let after = existing_results(&args, &tags).unwrap_or_default();
    let remaining = remaining(&args, &tags, &after, &hash, &cases);
    finish(out, &args, &stack, &summary, remaining, store.path())
}

fn remaining(
    args: &LiveArgs,
    tags: &ModelTags,
    existing: &Results,
    hash: &str,
    cases: &[corpus::LiveCase],
) -> usize {
    cases
        .iter()
        .filter(|c| plan::is_pending(args, tags, existing, hash, c))
        .count()
}

fn banner(
    out: &mut dyn Write,
    args: &LiveArgs,
    tags: &ModelTags,
    total: usize,
    results: &std::path::Path,
) {
    let live = |on: bool, tag: &str| {
        if on {
            tag.to_string()
        } else {
            "none (not a model)".to_string()
        }
    };
    say(out, &format!("provider  {}", provider_name(args)));
    say(
        out,
        &format!(
            "predictor {:?}: {}",
            args.predictor,
            live(
                args.predictor == super::config::PredictorKind::Llm,
                &tags.small
            )
        ),
    );
    say(
        out,
        &format!(
            "agent     {:?}: {}",
            args.agent,
            live(args.agent == super::config::AgentKind::Llm, &tags.main)
        ),
    );
    say(
        out,
        &format!(
            "modes     {}   selection {total} case(s) from {}",
            args.modes
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(","),
            args.corpora.join(",")
        ),
    );
    say(
        out,
        &format!(
            "limits    max {} model call(s) this invocation, {} attempt(s) per call, pause {} ms, cache {}",
            args.max_calls,
            args.max_attempts,
            args.pause.as_millis(),
            if args.no_cache || args.provider == Some(ProviderKind::Mock) { "off" } else { "on" }
        ),
    );
    say(out, &format!("results   {}", results.display()));
    if args.provider == Some(ProviderKind::Mock) {
        say(out, "NOTE      the mock provider is a fixed policy with no network: it exercises the machinery and says nothing about any model");
    }
}

fn finish(
    out: &mut dyn Write,
    args: &LiveArgs,
    stack: &super::provider::ModelStack,
    s: &RunSummary,
    remaining: usize,
    results: &std::path::Path,
) -> i32 {
    let budget = stack.summary();
    say(out, "");
    say(out, "== summary ==");
    say(
        out,
        &format!(
            "cases in this window: {}   results written: {} ({} failed)   reused from earlier: {}",
            s.cases_in_window, s.produced, s.failed, s.reused
        ),
    );
    say(
        out,
        &format!(
            "backend requests: {} of {} allowed (retries included)   answered from the cache: {}   rate-limit answers: {}",
            budget.backend_calls, args.max_calls, budget.cache_hits, s.calls.rate_limited
        ),
    );
    say(
        out,
        &format!(
            "tokens (as the backend reported): {} prompt + {} completion",
            s.calls.prompt_tokens, s.calls.eval_tokens
        ),
    );
    say(out, &format!("results file: {}", results.display()));
    match &s.stopped {
        Some(StopReason::BudgetExhausted) => {
            say(out, "STOPPED: the call budget is spent. Nothing was lost; the case in flight is still pending.");
            say(
                out,
                "          Raise --max-calls, or run the same command again later.",
            );
            return EXIT_BUDGET;
        }
        Some(StopReason::ProviderUnavailable { class }) => {
            say(out, &format!("STOPPED: the provider kept failing ({class}). Wait, then run the same command again; stored results are kept."));
            return EXIT_PROVIDER;
        }
        None => {}
    }
    if remaining == 0 {
        say(out, "the selection is complete. Next: --report");
    } else {
        say(out, &format!("{remaining} case(s) of the selection still have work to do: run the same command again for the next batch."));
    }
    if s.failed > 0 {
        say(out, &format!("{} result(s) are errors and are excluded from every rate; --retry-failed redoes them.", s.failed));
        return EXIT_SOME_FAILED;
    }
    EXIT_OK
}

#[cfg(test)]
mod tests {
    use ferrite_core::FixedClock;
    use ferrite_ipi::dry_run::DryRunOrchestrator;
    use ferrite_model::testing::RecordingSleeper;
    use ferrite_model::MapEnv;

    use super::*;
    use crate::live::config::LiveArgs;

    struct Out(Vec<u8>);

    impl Out {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0).to_string()
        }
    }

    impl Write for Out {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn tmp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ferrite-live-cli-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn factory() -> Box<OrchestratorFactory> {
        Box::new(|path, content| {
            DryRunOrchestrator::with_test_twin_key(
                path,
                content,
                Box::new(MapEnv::new().with(ferrite_ipi::twin::TWIN_KEY_ENV_VAR, "test-only")),
                Box::new(ferrite_model::secret::NoSecretStore),
            )
        })
    }

    async fn run(argv: &[&str], env: &MapEnv) -> (i32, String) {
        let owned: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
        let args = LiveArgs::parse(&owned, env).expect("parses");
        let secrets = ferrite_model::secret::NoSecretStore;
        let f = factory();
        let host = Host {
            env,
            secrets: &secrets,
            clock: Arc::new(FixedClock::at_epoch()),
            sleeper: Arc::new(RecordingSleeper::new()),
            orchestrator: f.as_ref(),
        };
        let mut out = Out(Vec::new());
        let code = execute(args, &host, &mut out).await;
        (code, out.text())
    }

    #[tokio::test]
    async fn help_prints_the_flags_and_exits_zero() {
        let (code, text) = run(&["--help"], &MapEnv::new()).await;
        assert_eq!(code, EXIT_OK);
        assert!(text.contains("--batch-size") && text.contains("--max-calls"));
    }

    #[tokio::test]
    async fn a_plan_calls_nothing_needs_no_key_and_writes_no_results() {
        let out = tmp("plan");
        // A real provider with no key anywhere: a plan must still work.
        let (code, text) = run(
            &[
                "--plan",
                "--provider",
                "gemini",
                "--model",
                "any-tag",
                "--batch-size",
                "25",
                "--suite",
                "agentdojo/slack",
                "--out",
                out.to_str().unwrap(),
            ],
            &MapEnv::new(),
        )
        .await;
        assert_eq!(code, EXIT_OK, "{text}");
        assert!(text.starts_with("PLAN (nothing was called)"), "{text}");
        assert!(text.contains("window: 25 case(s)"), "{text}");
        assert!(!out.join("results").exists(), "planning stores nothing");
    }

    #[tokio::test]
    async fn a_missing_provider_or_model_is_a_usage_error_not_a_default() {
        let (code, text) = run(&["--plan"], &MapEnv::new()).await;
        assert_eq!(code, EXIT_USAGE);
        assert!(text.contains("choose a provider"), "{text}");
        let (code, text) = run(&["--plan", "--provider", "gemini"], &MapEnv::new()).await;
        assert_eq!(code, EXIT_USAGE);
        assert!(text.contains("--small-model"), "{text}");
    }

    #[tokio::test]
    async fn a_real_provider_with_no_key_fails_with_the_documented_message_and_never_a_key() {
        let out = tmp("nokey");
        let (code, text) = run(
            &[
                "--provider",
                "gemini",
                "--model",
                "t",
                "--batch-size",
                "2",
                "--suite",
                "agentdojo/slack",
                "--out",
                out.to_str().unwrap(),
            ],
            &MapEnv::new(),
        )
        .await;
        assert_eq!(code, EXIT_USAGE);
        assert!(
            text.contains("FERRITE_GEMINI_API_KEY"),
            "names the variable: {text}"
        );
        assert!(text.contains("keyring"), "{text}");
        assert!(
            !out.join("results").exists()
                || std::fs::read_dir(out.join("results")).unwrap().count() == 0
        );
    }

    #[tokio::test]
    async fn run_then_resume_then_report_end_to_end_with_the_mock() {
        let out = tmp("e2e");
        let base = [
            "--provider",
            "mock",
            "--mock-behavior",
            "compliant",
            "--suite",
            "agentdojo/slack",
            "--batch-size",
            "10",
            "--seed",
            "3",
            "--modes",
            "off,guard",
            "--out",
            out.to_str().unwrap(),
        ];
        let (code, text) = run(&base, &MapEnv::new()).await;
        assert_eq!(code, EXIT_OK, "{text}");
        assert!(text.contains("== summary =="));
        assert!(
            text.contains("results written: 20"),
            "10 cases x 2 modes: {text}"
        );
        assert!(text.contains("still have work to do"), "{text}");
        assert!(
            text.contains("MOCK") || text.contains("mock provider"),
            "{text}"
        );

        // Same command again: the next 10, not the same 10.
        let (code, text) = run(&base, &MapEnv::new()).await;
        assert_eq!(code, EXIT_OK, "{text}");
        assert!(text.contains("results written: 20"), "{text}");
        let results = store::read_all(&out).unwrap();
        assert_eq!(results.by_key.len(), 40);

        let (code, text) = run(
            &["--report", "--out", out.to_str().unwrap()],
            &MapEnv::new(),
        )
        .await;
        assert_eq!(code, EXIT_OK, "{text}");
        let md = std::fs::read_to_string(out.join("REPORT.md")).unwrap();
        assert!(md.contains("MOCK PROVIDER"));
        assert!(md.contains("agentdojo/slack"));
        let csv = std::fs::read_to_string(out.join("report.csv")).unwrap();
        assert!(csv.starts_with("group,section,mode,key,metric,k,n,rate,ci_low,ci_high,note"));
    }

    #[tokio::test]
    async fn the_call_cap_exits_with_the_budget_code_and_a_rerun_finishes_the_work() {
        let out = tmp("cap");
        let base = [
            "--provider",
            "mock",
            "--suite",
            "agentdojo/slack",
            "--batch-size",
            "6",
            "--seed",
            "1",
            "--modes",
            "off,guard",
            "--out",
            out.to_str().unwrap(),
        ];
        let mut capped = base.to_vec();
        capped.extend(["--max-calls", "5"]);
        let (code, text) = run(&capped, &MapEnv::new()).await;
        assert_eq!(code, EXIT_BUDGET, "{text}");
        assert!(text.contains("call budget is spent"), "{text}");
        let stored = store::read_all(&out).unwrap().by_key.len();
        assert!(stored < 12);

        // The same command again takes the next 6 cases that still have work to do:
        // the unfinished one first, then new ones. Nothing stored is lost or redone.
        let (code, _) = run(&base, &MapEnv::new()).await;
        assert_eq!(code, EXIT_OK);
        let after = store::read_all(&out).unwrap();
        assert!(after.by_key.len() >= 12);
        let mut modes_per_case: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        for r in after.by_key.values() {
            *modes_per_case.entry(r.case_id.as_str()).or_default() += 1;
        }
        assert!(
            modes_per_case.values().all(|n| *n == 2),
            "every case that was started is now complete in both modes: {modes_per_case:?}"
        );
    }

    #[tokio::test]
    async fn a_report_with_nothing_stored_says_so_and_exits_zero() {
        let out = tmp("empty");
        let (code, text) = run(
            &["--report", "--out", out.to_str().unwrap()],
            &MapEnv::new(),
        )
        .await;
        assert_eq!(code, EXIT_OK);
        assert!(text.contains("No stored results"));
    }

    #[tokio::test]
    async fn no_output_ever_contains_a_key_even_one_in_the_environment() {
        let key = "AIzaFAKEKEYFORTESTSONLY0123456789abc";
        let env = MapEnv::new().with("FERRITE_GEMINI_API_KEY", key);
        let out = tmp("secret");
        // A real provider with a key set: the connect fails or runs against nothing;
        // either way nothing printed or stored may contain the key.
        let (_, text) = run(
            &[
                "--provider",
                "gemini",
                "--model",
                "t",
                "--base-url",
                "http://127.0.0.1:9",
                "--batch-size",
                "1",
                "--max-attempts",
                "1",
                "--suite",
                "agentdojo/slack",
                "--out",
                out.to_str().unwrap(),
            ],
            &env,
        )
        .await;
        assert!(!text.contains(key), "{text}");
        for entry in walk(&out) {
            let body = std::fs::read_to_string(&entry).unwrap_or_default();
            assert!(!body.contains(key), "{}", entry.display());
        }
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    files.extend(walk(&p));
                } else {
                    files.push(p);
                }
            }
        }
        files
    }
}
