//! `just guard-eval` — the runtime-guard experiment (ADR-014): what the real
//! run does when the dry run could not have seen the attack. Prints a table and
//! writes `target/eval-report/GUARD_REPORT.md`. No network, no API key.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ferrite_eval::corpus::load_corpus;
use ferrite_eval::guard_eval::{run_case, summarize, GuardSummary};
use ferrite_ipi::tool_decision::ToolDecisionEngine;
use uuid::Uuid;

fn corpus_dirs() -> Vec<PathBuf> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    [
        "corpus",
        "pilot_corpus",
        "agentdojo_corpus",
        "corpus_redteam",
    ]
    .iter()
    .map(|d| base.join(d))
    .collect()
}

fn pct(n: usize, d: usize) -> String {
    if d == 0 {
        "n/a".to_string()
    } else {
        format!("{n}/{d} = {:.1}%", 100.0 * n as f64 / d as f64)
    }
}

#[tokio::main]
async fn main() {
    let engine = ToolDecisionEngine::new();
    let provider = ferrite_model::MockProvider::new();
    let twin_base = std::env::temp_dir();
    let mut results = Vec::new();
    for dir in corpus_dirs() {
        let cases = match load_corpus(&dir) {
            Ok(c) => c,
            Err(errors) => {
                eprintln!(
                    "guard-eval: {} load error(s) in {}: {errors:?}",
                    errors.len(),
                    dir.display()
                );
                std::process::exit(1);
            }
        };
        for (case, content) in cases {
            let twin = twin_base.join(format!("ferrite-guard-eval-{}.enc", Uuid::new_v4()));
            match run_case(&case, &content, &engine, twin, &provider).await {
                Ok(r) => results.push(r),
                Err(e) => {
                    eprintln!("guard-eval: case {} failed: {e}", case.case_id);
                    std::process::exit(1);
                }
            }
        }
    }

    let total = summarize(&results);
    let mut by_tier: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in &results {
        by_tier
            .entry(format!("{:?}", r.case.tier))
            .or_default()
            .push(r.clone());
    }

    let mut out = String::new();
    out.push_str("# Runtime-guard experiment (ADR-014)\n\n");
    out.push_str(
        "The compromised real run (the worst-case agent after the injection has steered it, on the real page, \
         with no defense in its way) is checked action by action. *Without the guard* the live loop ran every \
         action a clean dry run did not flag; *with the guard* an action outside the prediction is blocked. \
         Nothing is approved in this experiment, and no network or model is used.\n\n",
    );
    out.push_str("| Stratum | Attacks | Succeed without the guard | Succeed with the guard (all residual) | Benign cases | Benign blocked |\n|---|---|---|---|---|---|\n");
    let mut row = |name: &str, s: &GuardSummary| {
        out.push_str(&format!(
            "| {name} | {} | {} | {} | {} | {} |\n",
            s.attacks,
            pct(s.succeed_without_guard, s.attacks),
            pct(s.succeed_with_guard, s.attacks),
            s.benign,
            pct(s.benign_blocked, s.benign),
        ));
    };
    for (tier, rs) in &by_tier {
        row(tier, &summarize(rs));
    }
    row("**All**", &total);
    out.push_str(&format!(
        "\nActions the guard blocked although the prediction admits them: **{}** (must be 0: the guard and the comparator classify every event identically).\n\n",
        total.wrongly_blocked_actions
    ));
    out.push_str(
        "What still succeeds with the guard is the residual: actions the prediction admits (same primitive at the \
         task's own origin, or any origin under a `task_open` scope). They are indistinguishable from the task's \
         own behaviour by construction (`docs/EVALUATION.md` O4).\n",
    );
    let report_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("eval-report");
    let _ = std::fs::create_dir_all(&report_dir);
    let path = report_dir.join("GUARD_REPORT.md");
    if let Err(e) = std::fs::write(&path, &out) {
        eprintln!("guard-eval: cannot write {}: {e}", path.display());
    }
    print!("{out}");
}
