//! `just probe` — one live Ollama Cloud round-trip, by hand (A3's exit gate,
//! `docs/REBUILD_DIRECTIVE.md` §6/A3).
//!
//! Deliberately outside the test suite (R7) and not wrapped in any
//! cache/throttle/budget decorator: a probe is meant to prove the raw wire
//! path works at all, before anything is layered on top of it. Needs
//! `OLLAMA_API_KEY` (or the OS keyring) and `FERRITE_MODEL_SMALL` set to a
//! tag the endpoint actually serves (`just models` lists them). Cannot be
//! run in this offline sandbox — see `docs/handoffs/a3.md`.

use ferrite_model::backends::shared_ollama;
use ferrite_model::{
    CompletionRequest, Message, ModelConfig, ModelProvider, ModelTier, OsKeyring, SystemEnv,
};

#[tokio::main]
async fn main() {
    let config = match ModelConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(1);
        }
    };

    let provider = match shared_ollama(&config, ModelTier::Small, &SystemEnv, &OsKeyring) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not build an Ollama provider: {e}");
            std::process::exit(1);
        }
    };

    let req = CompletionRequest::new(
        config.tag(ModelTier::Small),
        ModelTier::Small,
        vec![Message::user("Reply with exactly one word: hello")],
    );

    match provider.complete(req).await {
        Ok(response) => {
            println!("provider: {}", response.provenance.provider);
            println!("tag:      {}", response.provenance.model_tag);
            println!(
                "tokens:   {} prompt + {} completion",
                response.usage.prompt_eval_count, response.usage.eval_count
            );
            println!("content:  {}", response.content);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
