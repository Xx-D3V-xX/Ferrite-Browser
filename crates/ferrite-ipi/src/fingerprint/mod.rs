//! Predict-phase fingerprinting: turns a user prompt, alone, into the
//! *expected* capability set a task should need — before any web content is
//! fetched or any tool runs (`docs/REBUILD_DIRECTIVE.md` §6/A4,
//! `docs/TO-DO.md` T-104).
//!
//! # Shape
//!
//! A [`Fingerprint`] has two layers, matching the pre-rebuild hybrid design
//! (`docs/AUDIT.md`'s reference material, `crates/ferrite-ipi/src/tool_decision/`)
//! but retyped against the closed taxonomy in [`ferrite_core`] instead of a
//! stringly `ToolId`:
//!
//! - [`rules::rule_based_must_use`] — a deterministic keyword layer.
//!   Capabilities the prompt text unambiguously implies. Pure, offline, no
//!   model call.
//! - [`engine::generate_fingerprint`] — calls a [`ferrite_model::ModelProvider`]
//!   to predict `may_use`: capabilities the task *might* plausibly need,
//!   beyond what the rules already pinned down.
//!
//! # The closed allowlist
//!
//! A model's prediction is a list of strings — untrusted, attacker-adjacent
//! text per §10.4. It is filtered against [`ferrite_core::Capability::ALL`]
//! (`Capability::as_str()` equality, nothing fuzzier): a label that is not
//! one of the seven closed capability names is dropped at the boundary, not
//! merely rejected after being provisionally believed. This is what makes
//! the adversarial-label tests in `engine`'s test module pass by
//! construction rather than by the model happening to behave.
//!
//! # `js.execute` is structurally absent from every fingerprint
//!
//! [`Fingerprint::must_use`] and [`Fingerprint::may_use`] are
//! `BTreeSet<Capability>`, and [`ferrite_core::Capability`] is the closed
//! seven-member enum from `docs/DECISIONS.md` ADR-001/ADR-003: there is no
//! `Capability` variant belonging to [`ferrite_core::ActionClass::Execute`],
//! because no capability's realization is allowed to authorize `js.execute`
//! at any scope (`ferrite_core::taxonomy`'s own
//! `no_capabilitys_expected_realization_can_contain_js_execute` test proves
//! this at the taxonomy level). This module does not re-implement that
//! guarantee — it inherits it for free by reusing [`ferrite_core::Capability`]
//! rather than inventing a parallel "predicted tool" enum. See
//! [`engine::tests::a_fingerprint_can_never_express_js_execute`] for the
//! proof restated at this module's boundary, and the module's `compile_fail`
//! doctest below for the same fact ADR-003 requires.
//!
//! ```compile_fail
//! // `Capability` has no `JsExecute` (or any `Execute`-class) variant to
//! // predict in the first place — there is nothing for a `Fingerprint` to
//! // hold that would lower to `js.execute`.
//! let _ = ferrite_core::Capability::JsExecute;
//! ```
//!
//! while the same path with a real variant compiles, showing the failure
//! above is the missing variant and not a mistyped path:
//!
//! ```
//! let _ = ferrite_core::Capability::WebRead;
//! ```
//!
//! # Fail to empty, never a bypass
//!
//! Every provider failure — transport error, timeout, rate limit, malformed
//! or oversized body, an empty body — degrades `may_use` to the empty set.
//! `engine::predict_may_use`'s control flow (not a scattered `if let Err`)
//! is what makes this the only reachable outcome of failure: see that
//! function's doc comment for exactly how.
//!
//! # Disjointness
//!
//! `must_use` and `may_use` are disjoint by construction: [`Fingerprint`]'s
//! only non-empty constructor subtracts `must_use` from the model's
//! candidate `may_use` before storing either, so there is no public way to
//! build a `Fingerprint` with a capability in both sets. See
//! `engine::tests::prop_must_use_and_may_use_are_always_disjoint` for the
//! property test over a range of synthetic prompts and adversarial model
//! outputs.

pub mod engine;
pub mod rules;

pub use engine::{generate_fingerprint, Fingerprint};
pub use rules::rule_based_must_use;
