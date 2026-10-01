//! `ferrite-agent`: the engine-agnostic, provider-agnostic agent loop
//! (`browser_loop`) — see that module's own docs for the full design.
//!
//! # History (B3, `docs/TO-DO.md` T-224)
//!
//! This crate used to also hold a second, older agent runtime built before
//! `ferrite_engine::BrowserEngine`/`ferrite_model::ModelProvider` existed:
//! `BrowserTool`/`AgentTask`/`AgentToolCall`/`AgentToolResult`/`AgentTurn`/
//! `AgentError`, the `AgentRuntime`/`ToolExecutor` traits, `GeminiAgent`
//! (its one real implementor, `gemini.rs`, with its own raw `reqwest` HTTP
//! calls and file/env-based API key lookup that predated `ferrite-model`
//! entirely), a `RateLimiter` used only by `GeminiAgent`, and
//! `engine_bridge::EngineToolExecutor` (a bridge B1 added so that old
//! runtime could still drive the new `BrowserEngine`-based dry run).
//!
//! B3 deleted all of it, not deprecated it: `ferrite-ui` (T-224) was its
//! last real caller — the live app now runs `browser_loop::{AgentAction,
//! execute_action}` directly against a live `BrowserEngine` face
//! (`ferrite_engine_servo::BorrowedServoEngine`) and a real
//! `ferrite_model::ModelProvider`, and the dry run drives
//! `browser_loop::run_agent_loop` directly against
//! `ferrite_ipi::dry_run::DryRunEngine` — no bridge needed on either side
//! anymore. `ferrite-shell`'s `agent-smoke` subcommand (its other caller)
//! was migrated onto the same new stack in the same charter. A
//! full-workspace grep confirmed zero remaining references to any of the
//! deleted names before this file was cut down to just this module.

pub mod browser_loop;
pub mod chat;
pub mod context;
pub mod decider;
