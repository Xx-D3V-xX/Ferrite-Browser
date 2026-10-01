//! `just cache-stats` — reports the on-disk response cache's size and the
//! hit rate flushed by the last run (§10.3: "a low hit rate is a bug,
//! investigate it").
//!
//! Needs no model configuration or network access — it only reads
//! `~/.cache/ferrite-model/` (or `FERRITE_MODEL_CACHE_DIR`), so it works
//! offline and is safe to run in CI or this sandbox, unlike `models`/
//! `probe`/`record`.

use ferrite_model::config::{EnvSource, SystemEnv, default_cache_dir};
use ferrite_model::decorators::cache_report;

fn main() {
    let dir = match SystemEnv.get("FERRITE_MODEL_CACHE_DIR") {
        Some(explicit) => explicit.into(),
        None => match default_cache_dir() {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
    };

    match cache_report(&dir) {
        Ok(report) => {
            println!("cache dir: {}", report.dir.display());
            println!("entries:   {}", report.entries);
            println!("size:      {} bytes", report.bytes);
            match report.last_session {
                Some(stats) => {
                    println!(
                        "last run:  {} hits, {} misses, {} uncacheable",
                        stats.hits, stats.misses, stats.uncacheable
                    );
                    match stats.hit_rate() {
                        Some(rate) => println!("hit rate:  {:.1}%", rate * 100.0),
                        None => println!("hit rate:  n/a (no cacheable calls yet)"),
                    }
                }
                None => println!("last run:  no run has flushed stats yet"),
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
