//! Cross-cutting behaviour as decorators, not as backend features.
//!
//! §10.5 is explicit: "Cache, throttle, budget and telemetry are
//! **decorators** wrapping any provider — `Budget<Throttle<Cache<Ollama>>>`
//! — not features baked into each backend."
//!
//! The nesting order in that example is the one to use, read outside-in:
//!
//! - [`Budget`] outermost, so a cache hit still counts as a request the run
//!   asked for and the ledger reflects what was actually attempted;
//! - [`Throttle`] next, so retries and pacing apply only to calls that
//!   survived the budget check;
//! - [`Cache`] innermost, so a hit costs neither a retry slot nor a token
//!   from the bucket — which is the entire point of having one.
//!
//! Each decorator forwards [`ModelProvider::id`](crate::ModelProvider::id)
//! and `capabilities()` to what it wraps. A decorator is transparent: the
//! recorded provenance names the model that answered, never the wrapper.

mod budget;
mod cache;
mod throttle;
mod trace;

pub use budget::{Budget, BudgetSummary, CallRecord, PartialResults};
pub use cache::{
    Cache, CacheDirReport, CacheEntry, CacheStatsSnapshot, STATS_FILENAME, report as cache_report,
};
pub use throttle::{BackoffPolicy, RateLimit, Throttle, ThrottleConfig};
pub use trace::Trace;
