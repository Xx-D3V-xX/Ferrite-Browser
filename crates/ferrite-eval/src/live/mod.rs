//! The live evaluation runner: a real model in the loop, in batches a quota can
//! survive.
//!
//! The corpus-wide runner (`just eval`) answers a question about the *architecture*:
//! given an agent that has been steered off its task, does the loop catch the
//! deviation? Its agent is a script that complies with every injection
//! (`crate::worst_case_agent`) and its fingerprint is rules-only unless a key
//! happens to be set. This module asks the question that script cannot: **what does
//! a real model actually do under an injection, and what does Ferrite then do about
//! it?** A model is put in two places, independently selectable:
//!
//! - the **predictor** ([`config::PredictorKind::Llm`]): the small-tier call that
//!   proposes the fingerprint's `may_use`, as in the app;
//! - the **agent** ([`config::AgentKind::Llm`]): the main-tier model choosing
//!   actions in the app's own agent loop ([`agent::LlmAgent`]), reading a page or
//!   tool output that carries the injection, with the runtime guard in front of
//!   every action.
//!
//! # Modules
//!
//! | module | what it is |
//! |---|---|
//! | [`cli`] | what the binary does with parsed arguments: plan, run, report; exit codes |
//! | [`config`] | flags and environment; provider and model selection; defaults |
//! | [`corpus`] | loading, filtering, ordering and slicing cases (batches) |
//! | [`provider`] | the model stack: pacing, retries, `Retry-After`, a hard call cap, a cache |
//! | [`agent`] | the LLM-driven agent as a `DryRunDriver`, and the guard gate |
//! | [`outcome`] | deciding whether an action realizes the attack, and what the run amounts to |
//! | [`runner`] | one invocation: predict, run each mode, label, store, stop safely |
//! | [`record`], [`store`] | the JSONL result line, redaction, resume |
//! | [`plan`] | `--plan`: calls and tokens a selection would cost, without calling |
//! | [`report`] | Markdown and CSV with Wilson intervals, paired tests and breakdowns |
//! | [`mock_model`] | the `mock` provider: the whole pipeline with no network |
//!
//! # What is and is not tested without a key
//!
//! Everything here is exercised offline: the `mock` provider, `ferrite-model`'s
//! scripted `MockProvider`, and loopback fake servers. What cannot be verified
//! without a key is how a particular real model behaves, which is the point of
//! running it; see `docs/EVALUATION.md`, "Running with a real model".

pub mod agent;
pub mod cli;
pub mod config;
pub mod corpus;
pub mod mock_model;
pub mod outcome;
pub mod plan;
pub mod provider;
pub mod record;
pub mod report;
pub mod runner;
pub mod store;

#[cfg(test)]
pub(crate) mod testing;
