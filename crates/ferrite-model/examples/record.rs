//! `just record` — writes one live response into the committed fixture
//! directory (§10.3: "A `just record` mode writes every live response into
//! `tests/fixtures/model/<hash>.json`. These are committed.").
//!
//! Usage: `cargo run -p ferrite-model --example record -- ollama|gemini "<prompt>"`
//!
//! Deliberately outside the test suite (R7): makes a real network call, and
//! writes to the crate's own source tree (`fixtures::fixture_dir()`), not a
//! temp dir — that is the whole point of a fixture being *committed*.
//! Cannot be run in this offline sandbox — see `docs/handoffs/a3.md`. The
//! two fixtures already committed under `tests/fixtures/model/` were
//! generated with `fixtures::write` directly against literal, realistic
//! wire JSON (this crate's tests read them back), which is what stood in
//! for a real recording this session.

use ferrite_core::SystemClock;
use ferrite_model::backends::shared_ollama;
use ferrite_model::{
    CompletionRequest, GeminiProvider, Message, ModelConfig, ModelTier, OsKeyring, SystemEnv,
    fixtures,
};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let provider_name = args.next().unwrap_or_default();
    let prompt = args
        .next()
        .unwrap_or_else(|| "Reply with exactly one word: hello".to_string());

    let config = match ModelConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(1);
        }
    };

    let req = CompletionRequest::new(
        config.tag(ModelTier::Small),
        ModelTier::Small,
        vec![Message::user(prompt)],
    );

    let (id, wire) = match provider_name.as_str() {
        "ollama" => {
            let provider = match shared_ollama(&config, ModelTier::Small, &SystemEnv, &OsKeyring) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("could not build an Ollama provider: {e}");
                    std::process::exit(1);
                }
            };
            match provider.fetch_wire(&req).await {
                Ok(wire) => (ferrite_model::ProviderId::Ollama, wire),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        "gemini" => {
            let provider = match GeminiProvider::from_config(
                &config,
                ModelTier::Small,
                &SystemEnv,
                &OsKeyring,
            ) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("could not build a Gemini provider: {e}");
                    std::process::exit(1);
                }
            };
            match provider.fetch_wire(&req).await {
                Ok(wire) => (ferrite_model::ProviderId::Gemini, wire),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("usage: record <ollama|gemini> [prompt] (got {other:?})");
            std::process::exit(1);
        }
    };

    match fixtures::write(&fixtures::fixture_dir(), id, &req, wire, &SystemClock) {
        Ok(path) => println!("wrote {}", path.display()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
