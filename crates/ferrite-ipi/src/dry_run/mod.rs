//! The dry run: execute the agent's plan against synthetic data instead of
//! the real browser, and hand back an ordered record of what it actually
//! did (`docs/REBUILD_DIRECTIVE.md` §6/A6, rebuilt on `ferrite_engine::BrowserEngine`
//! per B1, `docs/TO-DO.md` T-221).
//!
//! This is the middle third of the `predict → dry-run → compare → consent`
//! loop: [`crate::fingerprint`] predicts what the agent *should* need;
//! this module runs the agent's plan for real against a
//! [`content::DryRunContent`] the case author scripted, so
//! [`crate::comparator`] has something concrete to compare the prediction
//! against.
//!
//! # Shape
//!
//! - [`record`] — [`record::ToolEvent`]/[`record::DryRunRecord`]: the
//!   ordered event log itself. `ToolEvent` carries a real
//!   [`ferrite_core::Primitive`] directly — see that module's docs for why
//!   this is a genuine simplification over the pre-B1 stringly `ToolId`
//!   bridge, not a cosmetic rename.
//! - [`content`] — [`content::DryRunContent`]/[`content::ReplyChannel`]:
//!   per-origin ordered reply queues, so a second read from the same origin
//!   can be scripted to return different content than the first (delayed
//!   payload / redirect chain). **Unchanged by B1** — this is a stable,
//!   external contract `ferrite-eval::corpus`'s JSON-corpus loader
//!   constructs directly.
//! - [`engine`] — [`engine::DryRunEngine`]: the synthetic
//!   [`ferrite_engine::BrowserEngine`] implementation that serves scripted
//!   content, records every call, and never performs real I/O. See its
//!   module docs for the full architectural reasoning (why `BrowserEngine`,
//!   why not `MockEngine` or `browser_loop::run_agent_loop`) and the
//!   containment argument (T-007/D7, ported from the pre-B1
//!   `RecordingExecutor`).
//! - [`orchestrator`] — [`orchestrator::DryRunOrchestrator`]/[`orchestrator::DryRunDriver`]:
//!   wires a twin, content, and a [`engine::DryRunEngine`] around one dry
//!   run, with a whole-turn timeout that returns the partial record rather
//!   than discarding it. `DryRunDriver` is B1's replacement for
//!   `ferrite_agent::AgentRuntime` as the "what decides the plan" seam — see
//!   `docs/handoffs/b01.md` for exactly how `ferrite-eval`/`ferrite-ui`
//!   bridge their existing `AgentRuntime` implementors onto it.
//!
//! # Containment is by construction (closes `docs/TO-DO.md` T-007 / D7)
//!
//! The pre-rebuild crate had a `containment` module: an application-layer
//! request interceptor plus a Linux-only network-namespace creator, both
//! activated around every dry run. D7 named the actual state of things
//! precisely: the interceptor's `intercept_request()` was unreachable
//! outside its own unit test, because the executor never made a real
//! network call for it to intercept in the first place — containment was
//! already achieved by the executor never touching the network, not by
//! anything the deleted module did. A6 deleted that module outright; B1's
//! `DryRunEngine` inherits the same property and the same grep-backed test
//! proving it (`engine::containment_tests::this_modules_source_names_no_network_capable_dependency`).

pub mod content;
pub mod engine;
pub mod orchestrator;
pub mod record;

pub use content::{DryRunContent, DryRunReply, ReplyChannel};
pub use engine::DryRunEngine;
pub use orchestrator::{DryRunDriver, DryRunOrchestrator};
pub use record::{DryRunRecord, FindingCarrier, RecordedFinding, ToolEvent};
