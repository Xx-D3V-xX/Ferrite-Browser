//! `cargo run -p ferrite-eval --example live_eval -- ...`: the evaluation with a
//! real model in the loop, in batches a quota can survive. See
//! `docs/EVALUATION.md`, "Running with a real model", and `--help`.
//!
//! This is the one binary in the crate that can reach the network, and only when
//! `--provider gemini` or `--provider ollama` is given. It is never built into or
//! run by `cargo test`; every behaviour it has is tested offline in
//! `ferrite_eval::live` against the `mock` provider and loopback fake servers.
//!
//! Keys come from the environment (`OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY`) or
//! the OS keyring, via `ferrite-model`, and nowhere else. Nothing here reads,
//! prints or stores one.

use std::sync::Arc;

use ferrite_eval::live::cli::{execute, Host, EXIT_USAGE};
use ferrite_eval::live::config::LiveArgs;
use ferrite_ipi::dry_run::DryRunOrchestrator;
use ferrite_model::testing::TokioSleeper;
use ferrite_model::{OsKeyring, SystemEnv};

fn main() {
    // The dry-run engine caches a synthetic twin (fake data) encrypted under a key
    // it looks up in the OS keyring, then in `FERRITE_TWIN_KEY`, and warns on every
    // run when it finds neither. For an evaluation that makes a thousand runs, give
    // this process a random one: nothing real is protected by it, and a key already
    // in the keyring still takes precedence. Done before any thread exists.
    if std::env::var_os(ferrite_ipi::twin::TWIN_KEY_ENV_VAR).is_none() {
        std::env::set_var(
            ferrite_ipi::twin::TWIN_KEY_ENV_VAR,
            format!("live-eval-{}", uuid::Uuid::new_v4()),
        );
    }

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match LiveArgs::parse(&argv, &SystemEnv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("live_eval: {e}");
            std::process::exit(EXIT_USAGE);
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(async {
        let factory = |path, content| DryRunOrchestrator::with_content(path, content);
        let host = Host {
            env: &SystemEnv,
            secrets: &OsKeyring,
            clock: Arc::new(ferrite_core::SystemClock),
            sleeper: Arc::new(TokioSleeper),
            orchestrator: &factory,
        };
        execute(args, &host, &mut std::io::stdout()).await
    });
    std::process::exit(code);
}
