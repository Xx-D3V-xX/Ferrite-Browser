//! Ferrite's core contracts: identifiers, the capability/primitive taxonomy,
//! origin scoping, time, and the error types those need.
//!
//! This crate holds **types and their invariants only** — no business logic.
//! It sits at the bottom of the dependency graph
//! (`core ← {model, audit, engine} ← ipi ← agent ← {ui, eval, cli}`) and must
//! never depend on another `ferrite-*` crate.

#![deny(dead_code)]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod clock;
pub mod error;
pub mod ids;
pub mod scope;
pub mod taxonomy;

pub use clock::{Clock, SystemClock};
pub use error::{IdError, ScopeError, TaxonomyError};
pub use ids::{CaseId, ExecId, Origin, PrincipalId};
pub use scope::{DomainSuffix, OriginScope, Specificity};
pub use taxonomy::{
    ActionClass, Capability, ExpectedCapability, ExpectedCapabilitySet, Primitive,
    ScopablePrimitive,
};

#[cfg(any(test, feature = "test-util"))]
pub use clock::FixedClock;
