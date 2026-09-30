//! `just eval` — runs the real corpus through the real pipeline (fingerprint
//! predict -> dry-run -> compare -> consent -> sanitizer) across every
//! defined mode, and writes the metrics report (markdown + CSV) plus a
//! summary of the audit-chain anchors the run produced.
//!
//! **B2 update (`docs/TO-DO.md` T-221's second half):** this binary now
//! attempts to construct a real, configured `ferrite_model::ModelProvider`
//! (`harness::try_real_provider` — `ModelConfig::from_env()`, Ollama first,
//! Gemini as fallback, key resolved from the environment or the OS keyring,
//! service `"ferrite"`) before falling back to the deterministic,
//! rules-only `MockProvider` path every prior `just eval` run in this
//! environment has exercised.
//!
//! **Primary, always-runnable path:** with no model config set (the default
//! in this sandbox and in CI — `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN`
//! have no source-code default per §10.2), `try_real_provider` returns
//! `None` and this binary falls back to `MockProvider`, so the fingerprint
//! layer runs rules-only — deterministic, offline, satisfies R7. The
//! dry-run agent itself (`ferrite_eval::worst_case_agent::WorstCaseAgent`)
//! is a scripted, ground-truth-derived driver that never makes a model call
//! at all (see that module's docs for the methodology). So a default
//! `just eval` run still makes ZERO live network calls end to end, by
//! construction, not by a flag someone has to remember to pass.
//!
//! **Optional live path:** set `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN`
//! (and have a real `OLLAMA_API_KEY`/`FERRITE_GEMINI_API_KEY` in the
//! environment or OS keyring) before running this to let the fingerprint's
//! `may_use` prediction go through a real model call instead of the
//! rules-only fallback. This binary is never invoked by `cargo test`/
//! `just test`, so this optional path never violates R7 (see
//! `harness::no_automated_test_calls_try_real_provider`).
//!
//! Output: `<out_dir>/EVAL_REPORT.md`, `<out_dir>/eval_report.csv`,
//! `<out_dir>/corpus.db` (SQLite `DatasetStore`), `<out_dir>/audit.db`
//! (the hash-chained audit log). `<out_dir>` is `FERRITE_EVAL_OUT_DIR` if
//! set, else `target/eval-report` relative to the current directory (run
//! `just eval`/this binary from the repo root, as the justfile recipe does).

use std::path::{Path, PathBuf};

use ferrite_audit_log::PersistentAuditLog;
use ferrite_eval::corpus::load_corpus;
use ferrite_eval::worst_case_agent::WorstCaseAgent;
use ferrite_eval::{harness, report};
use ferrite_ipi::dataset::{CaseDefinition, DatasetStore, ExecutionRecord};
use ferrite_ipi::tool_decision::ToolDecisionEngine;
use ferrite_model::ModelProvider;
use uuid::Uuid;

fn out_dir() -> PathBuf {
    match std::env::var("FERRITE_EVAL_OUT_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => PathBuf::from("target/eval-report"),
    }
}

/// The corpus directories, relative to this crate's own manifest dir (works
/// regardless of the caller's current directory): the three A11 built, plus the
/// generated red-team matrix (`scripts/gen_redteam_corpus.py`, ADR-014).
fn corpus_dirs() -> Vec<PathBuf> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    vec![
        base.join("corpus"),
        base.join("pilot_corpus"),
        base.join("agentdojo_corpus"),
        base.join("corpus_redteam"),
    ]
}

#[tokio::main]
async fn main() {
    let out = out_dir();
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("eval: cannot create output dir {}: {e}", out.display());
        std::process::exit(1);
    }

    let mut cases_and_content = Vec::new();
    let mut load_errors = Vec::new();
    for dir in corpus_dirs() {
        match load_corpus(&dir) {
            Ok(loaded) => {
                println!(
                    "eval: loaded {} case(s) from {}",
                    loaded.len(),
                    dir.display()
                );
                cases_and_content.extend(loaded);
            }
            Err(errs) => {
                for e in errs {
                    load_errors.push(format!("{}: {e}", dir.display()));
                }
            }
        }
    }
    if !load_errors.is_empty() {
        eprintln!("eval: {} corpus load error(s):", load_errors.len());
        for e in &load_errors {
            eprintln!("  {e}");
        }
        std::process::exit(1);
    }
    if cases_and_content.is_empty() {
        eprintln!(
            "eval: no cases loaded from {:?} — nothing to run",
            corpus_dirs()
        );
        std::process::exit(1);
    }

    let db_path = out.join("corpus.db");
    let audit_path = out.join("audit.db");
    // Fresh run every time: a stale DB from a prior run must not silently
    // merge with this one (duplicate case_id inserts would error).
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&audit_path);

    let store = DatasetStore::open(&db_path).expect("open corpus DB");
    let mut audit =
        PersistentAuditLog::new(audit_path.to_str().expect("utf8 path")).expect("open audit log");
    let principal = Uuid::new_v4();
    let engine = ToolDecisionEngine::new();
    let twin_base = out.clone();

    // B2: attempt a real provider first (Ollama, then Gemini — see
    // `harness::try_real_provider`'s docs), falling back to the
    // deterministic `MockProvider` this binary has always used when no
    // model config/key is available. Never a hard error either way — per
    // `CLAUDE.md`'s fail-to-empty invariant, "no provider" and "a provider
    // whose call fails" degrade the fingerprint identically.
    let real_provider = harness::try_real_provider();
    let (provider, model_tag): (std::sync::Arc<dyn ferrite_model::ModelProvider>, String) =
        match real_provider {
            Some((provider, tag)) => {
                println!(
                    "eval: real ModelProvider configured ({}), tag={tag} — fingerprint \
                     may_use will attempt live calls",
                    provider.id()
                );
                (provider, tag)
            }
            None => {
                println!(
                    "eval: no real ModelProvider configured (FERRITE_MODEL_SMALL/MAIN unset, \
                     or no key in the environment/OS keyring) — fingerprint may_use runs \
                     rules-only (fail-to-empty), zero network calls"
                );
                (
                    std::sync::Arc::new(ferrite_model::MockProvider::new()),
                    "eval-harness".to_string(),
                )
            }
        };

    let mut cases: Vec<CaseDefinition> = Vec::new();
    let mut executions: Vec<ExecutionRecord> = Vec::new();
    let mut run_errors = Vec::new();

    for (case, content) in &cases_and_content {
        let agent = WorstCaseAgent::for_case(case);
        match harness::run_case(
            case,
            content,
            &engine,
            &twin_base,
            &mut audit,
            principal,
            &store,
            &agent,
            provider.as_ref(),
            &model_tag,
        )
        .await
        {
            Ok(mut recs) => {
                executions.append(&mut recs);
                cases.push(case.clone());
            }
            Err(e) => run_errors.push(format!("{}: {e}", case.case_id)),
        }
    }

    if !run_errors.is_empty() {
        eprintln!("eval: {} case(s) failed to run:", run_errors.len());
        for e in &run_errors {
            eprintln!("  {e}");
        }
    }

    let chain_ok = audit.log.verify_chain();
    println!(
        "eval: ran {} case(s), {} execution(s), audit chain verify_chain() = {chain_ok}",
        cases.len(),
        executions.len()
    );

    let eval_report = report::generate_report(&cases, &executions);
    write_report(&out, &eval_report);

    if !chain_ok {
        eprintln!("eval: WARNING — audit chain failed to verify; report written anyway");
        std::process::exit(2);
    }
}

fn write_report(out: &Path, eval_report: &report::EvalReport) {
    let md_path = out.join("EVAL_REPORT.md");
    let csv_path = out.join("eval_report.csv");
    std::fs::write(&md_path, &eval_report.markdown).expect("write EVAL_REPORT.md");
    std::fs::write(&csv_path, &eval_report.csv).expect("write eval_report.csv");
    println!("eval: report written to {}", md_path.display());
    println!("eval: CSV written to {}", csv_path.display());
}
