//! `just models` — hits `/api/tags` and prints what the configured Ollama
//! endpoint actually serves.
//!
//! Deliberately outside the test suite (R7): this makes a real network call
//! and needs `OLLAMA_API_KEY` (or the OS keyring) unless
//! `FERRITE_OLLAMA_BASE_URL` points at a local endpoint. Cannot be verified
//! in an offline sandbox — see `docs/handoffs/a3.md`.

use ferrite_model::backends::shared_ollama;
use ferrite_model::{ModelConfig, ModelTier, OsKeyring, SystemEnv};

#[tokio::main]
async fn main() {
    let config = match ModelConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(1);
        }
    };

    // Tier is irrelevant to listing tags — Small is arbitrary.
    let provider = match shared_ollama(&config, ModelTier::Small, &SystemEnv, &OsKeyring) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not build an Ollama provider: {e}");
            std::process::exit(1);
        }
    };

    match provider.list_tags().await {
        Ok(tags) if tags.is_empty() => println!("(no tags served)"),
        Ok(tags) => {
            for tag in tags {
                println!("{tag}");
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
