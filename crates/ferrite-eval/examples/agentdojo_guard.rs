//! `cargo run -p ferrite-eval --example agentdojo_guard -- --provider P --model M [--out DIR]`:
//! Ferrite's guard as a line-JSON server for AgentDojo's own harness
//! (`scripts/agentdojo_ferrite.py` starts it). See `ferrite_eval::agentdojo_bridge`
//! for the protocol.
//!
//! It takes `live_eval`'s provider flags (`--provider`, `--model`/`--small-model`,
//! `--base-url`, `--pause-ms`, `--max-calls`, `--out` for the response cache) and
//! reaches the network only for `--provider ollama|gemini`. Keys come from the
//! environment or the OS keyring via `ferrite-model`, never from here. Answers go
//! to stdout, one line per request; nothing else is printed there.

use std::sync::Arc;

use ferrite_eval::agentdojo_bridge::{default_manifest_path, Session, ToolMap};
use ferrite_eval::live::config::LiveArgs;
use ferrite_eval::live::provider::connect;
use ferrite_model::testing::TokioSleeper;
use ferrite_model::{OsKeyring, SystemEnv};

fn fail(message: &str) -> ! {
    eprintln!("agentdojo_guard: {message}");
    std::process::exit(2);
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = LiveArgs::parse(&argv, &SystemEnv).unwrap_or_else(|e| fail(&e.to_string()));
    let tags = args.model_tags().unwrap_or_else(|e| fail(&e.to_string()));
    let manifest = std::fs::read_to_string(default_manifest_path())
        .unwrap_or_else(|e| fail(&format!("cannot read the AgentDojo manifest: {e}")));
    let map = ToolMap::from_manifest_json(&manifest).unwrap_or_else(|e| fail(&e));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(async {
        let stack = match connect(
            &args,
            &tags,
            &SystemEnv,
            &OsKeyring,
            Arc::new(ferrite_core::SystemClock),
            Arc::new(TokioSleeper),
        ) {
            Ok(s) => s,
            Err(e) => fail(&format!("cannot reach the model provider: {e}")),
        };
        let mut session = Session::new(&map, &stack, tags.small.clone());
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        match session.serve(stdin.lock(), &mut stdout).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("agentdojo_guard: {e}");
                1
            }
        }
    });
    std::process::exit(code);
}
