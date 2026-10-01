//! Every error this crate can produce.
//!
//! There is deliberately no crate-level aggregate error enum: nothing in
//! `ferrite-core` returns one, and an aggregate with no caller is exactly the
//! dead machinery R9 exists to prevent. A downstream crate that needs to mix
//! these adds its own `#[from]` variants.

use thiserror::Error;

use crate::taxonomy::Capability;

/// An expected-capability set violated the taxonomy's invariants.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TaxonomyError {
    /// The same capability was listed twice. Each capability carries exactly
    /// one [`OriginScope`](crate::scope::OriginScope) per task, so two
    /// entries for one capability would make attribution ambiguous — which of
    /// the two scopes justified the action? — and that ambiguity is precisely
    /// what D1's per-capability scoping exists to remove.
    #[error(
        "capability {capability} appears more than once in an expected set; \
         each capability carries exactly one origin scope per task"
    )]
    DuplicateCapability {
        /// The capability that was listed twice.
        capability: Capability,
    },
}

/// An [`OriginScope`](crate::scope::OriginScope) or
/// [`DomainSuffix`](crate::scope::DomainSuffix) was handed a value that
/// violates ADR-004's authoring rules.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScopeError {
    /// An `Exact` or `DomainSuffix` scope was built with no entries. Such a
    /// scope admits nothing, which is never what an author meant — it is the
    /// silent way to write a capability that can never be attributed.
    #[error("an origin scope of kind {kind} must list at least one entry")]
    Empty {
        /// The scope variant that was empty.
        kind: &'static str,
    },

    /// A `TaskOpen` scope was built without a written rationale. ADR-004
    /// requires one, because task-open is the weak-scope tier and its use has
    /// to be justifiable and reportable rather than a default.
    #[error("a task-open origin scope requires a written rationale (ADR-004)")]
    MissingRationale,

    /// A domain suffix was not a usable domain.
    #[error("domain suffix {value:?} is unusable: {reason}")]
    InvalidDomainSuffix {
        /// The rejected input.
        value: String,
        /// Why it was rejected.
        reason: &'static str,
    },
}

/// A newtype identifier was handed a value that violates its invariants.
///
/// `kind` is the identifier type's name (`"CaseId"`, `"PrincipalId"`, …) so
/// one enum can serve every identifier without a variant each.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    /// The value was empty, or whitespace-only.
    #[error("{kind} must not be empty")]
    Empty {
        /// The identifier type that rejected the value.
        kind: &'static str,
    },

    /// The value exceeded the identifier's length ceiling.
    #[error("{kind} is {len} bytes, over the {max}-byte maximum")]
    TooLong {
        /// The identifier type that rejected the value.
        kind: &'static str,
        /// The rejected value's length, in bytes.
        len: usize,
        /// The ceiling, in bytes.
        max: usize,
    },

    /// The value contained a character outside the identifier's alphabet.
    #[error("{kind} contains {ch:?}, which is outside its alphabet ({allowed})")]
    DisallowedCharacter {
        /// The identifier type that rejected the value.
        kind: &'static str,
        /// The first offending character.
        ch: char,
        /// A human-readable description of the permitted alphabet.
        allowed: &'static str,
    },

    /// An [`Origin`](crate::ids::Origin) was not parseable as an absolute URL.
    #[error("origin {value:?} is not an absolute URL: {source}")]
    OriginNotAUrl {
        /// The rejected input.
        value: String,
        /// The underlying parse failure.
        #[source]
        source: url::ParseError,
    },

    /// An [`Origin`](crate::ids::Origin) had a scheme that carries no tuple
    /// origin, so nothing about it can be scoped.
    #[error(
        "origin {value:?} has scheme {scheme:?}; only http and https carry a \
         tuple origin that an OriginScope can admit"
    )]
    OriginUnsupportedScheme {
        /// The rejected input.
        value: String,
        /// The scheme that was found.
        scheme: String,
    },

    /// An [`Origin`](crate::ids::Origin) parsed but carried no host.
    #[error("origin {value:?} has no host")]
    OriginMissingHost {
        /// The rejected input.
        value: String,
    },
}
