//! Ferrite's model provider layer — `docs/REBUILD_DIRECTIVE.md` §10.
//!
//! This crate is the **only** place in the workspace that knows Ollama and
//! Gemini exist. Everything above it takes a [`ModelProvider`] and records a
//! [`ProviderId`]; §10.5 states the rule as "no provider type may appear in
//! a `ferrite-ipi` or `ferrite-agent` signature."
//!
//! # Shape
//!
//! Backends speak one wire format each and do nothing else:
//! [`MockProvider`] (scripted, for unit tests), [`ReplayProvider`]
//! (committed fixtures, for integration tests and CI), [`OllamaProvider`]
//! (cloud *and* local — one struct, one wire protocol, two base URLs),
//! [`GeminiProvider`], [`AnthropicProvider`] and [`OpenAiProvider`] (OpenAI and
//! every server that speaks its chat-completions format).
//!
//! Everything cross-cutting is a decorator wrapping any provider, so a new
//! backend inherits all of it:
//!
//! ```text
//! Budget<Throttle<Cache<Ollama>>>
//! ```
//!
//! - [`Cache`] — content-addressed on disk at `~/.cache/ferrite-model/`.
//!   Sound because [`SamplingOptions`]'s default `temperature = 0` makes a
//!   response a pure function of its key.
//! - [`Throttle`] — in-flight semaphore, token bucket, exponential backoff
//!   with full jitter on 429/5xx, per-request timeout, `Retry-After`.
//! - [`Budget`] — a hard per-process call ceiling that aborts with a
//!   partial-results artifact rather than quietly spending the rest of a
//!   quota.
//!
//! # Why the arithmetic drives the design
//!
//! §10.3: ~900 executions per full eval run × 3–5 calls each is
//! 3,000–4,500 live calls, which exhausts the quota on the first attempt.
//! The cache is what collapses that — identical fingerprint calls across all
//! four defense modes hit the same key — and a second run of an unchanged
//! corpus costs zero calls. A low cache hit rate is a bug, which is why
//! `just cache-stats` exists.
//!
//! # Trust boundary
//!
//! Model output is never trusted as text (§10.4). Every backend's bytes go
//! through one guard that bounds their size, rejects an empty body, and
//! parses a structured response into a typed value or a typed error. What
//! this crate deliberately does *not* do is decide what a parsed label is
//! allowed to *mean*: filtering against the closed 7-capability allowlist is
//! A4's policy, and it consumes
//! `CompletionResponse::parse_structured`.

#![deny(dead_code)]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod backends;
pub mod cache_key;
pub mod config;
pub mod decorators;
pub mod error;
pub mod fixtures;
mod guard;
pub mod laya;
pub mod laya_governor;
pub mod provider;
pub mod request;
pub mod response;
pub mod secret;
pub mod settings;
pub mod testing;
pub mod trace;

pub use backends::{
    AnthropicProvider, GeminiProvider, MockProvider, MockStep, OllamaProvider, OpenAiProvider,
    ReplayProvider,
};
pub use cache_key::CacheKey;
pub use config::{EnvSource, MapEnv, ModelConfig, SystemEnv};
pub use decorators::{Budget, Cache, Throttle, ThrottleConfig, Trace};
pub use error::ModelError;
pub use laya_governor::{Gate, LaneGovernor, Paused};
pub use provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId, TextSink};
pub use request::{CompletionRequest, Message, Role, SamplingOptions};
pub use response::{CompletionResponse, Provenance, TokenUsage};
pub use secret::{MemoryVault, OsKeyring, SecretStore, SecretVault, Token};
pub use settings::{Connection, KeySource, LayeredEnv, ModelPair, ModelSettings, ProviderChoice};
