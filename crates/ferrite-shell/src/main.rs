use ferrite_servo::session::HeadlessServoSession;
use ferrite_servo::shell::ServoShell;

mod logging;

fn main() {
    // First, before anything can print or panic: a run with no terminal gets
    // a log file (see `logging`).
    let _log = logging::init();
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "ui" => run_ui(),
        "window" => ServoShell::new().run(),
        "jstest" => run_js_compat_test(),
        "agent-smoke" => run_agent_smoke(),
        "smoke" => run_smoke_test(),
        _ => run_ui(),
    }
}

/// Runs the browser, then ends the process outright. Servo's own threads can
/// be wedged by a page that crashed one of them, and waiting for them at exit
/// is what turned a quit into a force quit.
fn run_ui() {
    let result = ferrite_ui::launch();
    if let Err(e) = &result {
        eprintln!("[ferrite] the UI exited with an error: {e}");
    }
    std::process::exit(i32::from(result.is_err()));
}

fn run_js_compat_test() {
    const URLS: &[&str] = &[
        "https://example.com",
        "https://lite.duckduckgo.com",
        "https://doc.rust-lang.org",
    ];

    let mut session = match HeadlessServoSession::new(1280, 800) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[jstest] Failed to create Servo session: {}", e);
            eprintln!("[jstest] Servo feature may not be enabled — build with:");
            eprintln!("  cargo run -p ferrite-shell --features ferrite-servo/servo -- jstest");
            std::process::exit(1);
        }
    };

    let mut results = Vec::new();

    for url in URLS {
        print!("[jstest] Probing {} ... ", url);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let result = session.test_js_compat(url);
        println!(
            "js={} title={:?} errors={}",
            result.js_executed,
            result.page_title.as_deref().unwrap_or("—"),
            result.console_errors.len()
        );
        results.push(result);
    }

    // ── Print results table ────────────────────────────────────────────────
    println!();
    println!("{:<42} {:<12} {:<40} Errors", "URL", "JS Executed", "Title");
    println!("{}", "-".repeat(110));
    for r in &results {
        let title = r.page_title.as_deref().unwrap_or("—");
        let title_truncated = if title.len() > 38 {
            format!("{}…", &title[..37])
        } else {
            title.to_string()
        };
        let errors_summary = if r.console_errors.is_empty() {
            "none".to_string()
        } else {
            format!(
                "{}: {}",
                r.console_errors.len(),
                r.console_errors[0].chars().take(40).collect::<String>()
            )
        };
        println!(
            "{:<42} {:<12} {:<40} {}",
            r.url,
            if r.js_executed { "yes" } else { "no" },
            title_truncated,
            errors_summary,
        );
    }
    println!();
    println!("JS compat baseline complete");

    // ── Save CSV ───────────────────────────────────────────────────────────
    let csv_dir = std::path::Path::new("../paper/data");
    if let Err(e) = std::fs::create_dir_all(csv_dir) {
        eprintln!(
            "[jstest] Warning: could not create {}: {}",
            csv_dir.display(),
            e
        );
        return;
    }
    let csv_path = csv_dir.join("js_compat_baseline.csv");
    let mut csv = String::from("url,js_executed,page_title,error_count,first_error\n");
    for r in &results {
        let title = r.page_title.as_deref().unwrap_or("").replace('"', "\"\"");
        let first_error = r
            .console_errors
            .first()
            .map(|e| e.replace('"', "\"\""))
            .unwrap_or_default();
        csv.push_str(&format!(
            "\"{}\",{},\"{}\",{},\"{}\"\n",
            r.url,
            r.js_executed,
            title,
            r.console_errors.len(),
            first_error,
        ));
    }
    match std::fs::write(&csv_path, &csv) {
        Ok(_) => println!("[jstest] Results saved to {}", csv_path.display()),
        Err(e) => eprintln!("[jstest] Warning: could not write CSV: {}", e),
    }
}

/// Constructs a real, configured `ferrite_model::ModelProvider` — the same
/// pattern `ferrite-eval::harness::try_real_provider()` and
/// `ferrite-ui`'s `try_real_model_provider()` both use (`ModelConfig::
/// from_env()`, Ollama first via the OS keyring, Gemini fallback). Kept
/// local rather than shared: each of the three call sites already
/// duplicates this exact ~15-line pattern by this project's own established
/// convention (see `ferrite-ui/src/lib.rs`'s own copy for the precedent),
/// so a fourth copy here is consistent, not a new smell.
fn try_real_model_provider() -> Option<(
    std::sync::Arc<dyn ferrite_model::ModelProvider>,
    ferrite_model::ModelConfig,
)> {
    let config = ferrite_model::ModelConfig::from_env().ok()?;

    if let Ok(ollama) = ferrite_model::backends::shared_ollama(
        &config,
        ferrite_model::ModelTier::Main,
        &ferrite_model::SystemEnv,
        &ferrite_model::OsKeyring,
    ) {
        let provider: std::sync::Arc<dyn ferrite_model::ModelProvider> = ollama;
        return Some((provider, config));
    }

    if let Ok(gemini) = ferrite_model::GeminiProvider::from_config(
        &config,
        ferrite_model::ModelTier::Main,
        &ferrite_model::SystemEnv,
        &ferrite_model::OsKeyring,
    ) {
        let provider: std::sync::Arc<dyn ferrite_model::ModelProvider> =
            std::sync::Arc::new(gemini);
        return Some((provider, config));
    }

    None
}

/// Demonstrates the new post-B3 agent stack end to end: a real
/// `ferrite_model::ModelProvider` driving `ferrite_agent::browser_loop::
/// run_agent_loop` against `ferrite_engine::MockEngine` (a real, in-process
/// `BrowserEngine` — no live Servo needed for a fast smoke check; the live
/// app itself, `ferrite-ui`, is what exercises the real Servo-backed
/// `BorrowedServoEngine`). Replaces this subcommand's old GeminiAgent/
/// BrowserTool/AgentRuntime/ToolExecutor demonstration, deleted alongside
/// the rest of that vocabulary (`docs/TO-DO.md` T-224).
fn run_agent_smoke() {
    use ferrite_agent::browser_loop::{LoopBudget, LoopStopReason, run_agent_loop};
    use ferrite_engine::MockEngine;
    use ferrite_model::ModelTier;

    let Some((provider, config)) = try_real_model_provider() else {
        eprintln!(
            "[agent-smoke] no ModelProvider configured/reachable (FERRITE_MODEL_SMALL/\
             FERRITE_MODEL_MAIN unset, or no Ollama/Gemini credential found) — skipping"
        );
        std::process::exit(0);
    };
    let model_tag = config.tag(ModelTier::Main).to_string();

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async {
        let mut engine = MockEngine::new();
        let clock = ferrite_core::SystemClock;
        let result = run_agent_loop(
            provider.as_ref(),
            &mut engine,
            &clock,
            &model_tag,
            ModelTier::Main,
            "What is the title of the page at https://example.com?",
            LoopBudget::default(),
        )
        .await;

        println!(
            "[agent-smoke] actions taken: {}",
            result.actions_taken.len()
        );
        match result.stop_reason {
            LoopStopReason::Finished(answer) => {
                println!("[agent-smoke] final_response: {}", answer);
            }
            // The model stopped to ask rather than guess: a clean stop, and
            // the question is the result.
            LoopStopReason::AskedUser(question) => {
                println!("[agent-smoke] asked_user: {}", question);
            }
            other => {
                eprintln!("[agent-smoke] did not finish cleanly: {:?}", other);
                std::process::exit(1);
            }
        }
    });
}

fn run_smoke_test() {
    use ferrite_audit_log::{AuditEventKind, PersistentAuditLog};
    use uuid::Uuid;

    let db_path = std::env::temp_dir()
        .join("ferrite_smoke_test.db")
        .to_string_lossy()
        .into_owned();

    let mut audit_log = PersistentAuditLog::new(&db_path).expect("failed to open audit log");

    let principal_id = Uuid::new_v4();

    audit_log
        .append(
            AuditEventKind::CapabilityGranted,
            principal_id,
            Some("network.fetch".to_string()),
            Some("https://example.com".to_string()),
        )
        .expect("audit append failed");

    audit_log
        .append(
            AuditEventKind::CapabilityDenied,
            principal_id,
            Some("network.fetch".to_string()),
            Some("https://blocked.example".to_string()),
        )
        .expect("audit append failed");

    assert!(
        audit_log.log.verify_chain(),
        "audit chain integrity check failed"
    );

    println!(
        "[ferrite] audit chain verified: {} entries",
        audit_log.log.entries.len()
    );
    println!("Smoke test: ALL CHECKS PASSED");
}
