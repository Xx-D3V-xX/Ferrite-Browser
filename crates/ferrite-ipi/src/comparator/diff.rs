//! [`FingerprintDiff`]: the result of [`compare`](super::compare) — what the
//! agent actually did, classified against what the task's [`ExpectedFingerprint`](super::ExpectedFingerprint)
//! said it might need.
//!
//! `extra_primitives` and `out_of_scope_origins` keep their pre-rebuild
//! field names and `HashSet` shapes on purpose: `crate::dataset` (A11's
//! schema, off-limits to this session) reads `extra_primitives` directly in
//! its own, non-test `unscopable_primitive_invoked` function, and
//! `ferrite-eval`/`ferrite-ui` construct and read both fields extensively.
//! Renaming or retyping either would not be a "minimal mechanical" compile
//! fix — see `docs/handoffs/a07.md`. [`Attribution`]/`attributions` is new:
//! it is the T-001/A7 addition that lets a consent UI or audit entry cite
//! *which* capability justified a non-flagged action, not just that the
//! action happened.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use ferrite_core::scope::Specificity;
use ferrite_core::Capability;

use crate::tool_decision::ToolId;

/// One successfully-justified event: which expected capability admitted it,
/// and how tightly.
///
/// Only events `compare` could actually attribute appear here — a flagged
/// event (present in [`FingerprintDiff::extra_primitives`] or
/// [`FingerprintDiff::out_of_scope_origins`]) never gets an `Attribution`,
/// by construction of `compare`'s control flow (every branch that inserts
/// into one of those two sets `continue`s before reaching the attribution
/// push).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// The tool/primitive wire id of the recorded event.
    pub tool: ToolId,
    /// The origin the event was recorded at.
    pub origin: String,
    /// The capability whose scope admitted `origin` for `tool`.
    pub capability: Capability,
    /// How tightly — the admission rank `compare` picked the maximum of,
    /// among every capability that could have admitted this event
    /// (T-002/D2: this field is exactly what proves `Specificity` was
    /// consumed, not just computed).
    pub specificity: Specificity,
}

/// Every primitive and origin the agent used beyond what its expected
/// fingerprint justified, plus the successful attributions for what wasn't.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct FingerprintDiff {
    /// Primitives the agent invoked with **no** expected capability naming
    /// them at all (regardless of origin), or that are unconditionally
    /// unscopable (`js.execute`; ADR-003), or whose origin was not even
    /// recorded. See the parent module's docs for the exact classification
    /// rule and why an event can never land in both this set and
    /// [`FingerprintDiff::out_of_scope_origins`] (T-206).
    pub extra_primitives: HashSet<ToolId>,
    /// Origins the agent contacted with a primitive some expected capability
    /// *does* name, but whose scope does not admit this particular origin —
    /// including an origin that was recorded but does not parse as a real
    /// [`ferrite_core::Origin`] (T-211: opaque schemes like `about:blank`,
    /// `data:`, `blob:` are unadmittable by any scope by definition, so they
    /// land here under their literal recorded string).
    pub out_of_scope_origins: HashSet<String>,
    /// Events `compare` could attribute to a specific admitting capability.
    /// Preserves recorded event order; may contain more than one entry for
    /// the same tool/origin pair if the agent repeated the action.
    #[serde(default)]
    pub attributions: Vec<Attribution>,
}

impl FingerprintDiff {
    /// Returns `true` when the agent stayed within its expected footprint.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.extra_primitives.is_empty() && self.out_of_scope_origins.is_empty()
    }

    /// Human-readable description of unexpected activity, suitable for the
    /// consent UI.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return "No unexpected activity detected.".to_string();
        }
        let mut lines =
            vec!["The agent attempted the following actions beyond your request:".to_string()];
        for primitive in &self.extra_primitives {
            lines.push(format!("  - Used tool: {primitive}"));
        }
        for origin in &self.out_of_scope_origins {
            lines.push(format!("  - Contacted: {origin}"));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_clean() {
        assert!(FingerprintDiff::default().is_clean());
    }

    #[test]
    fn dirty_when_either_set_is_non_empty() {
        let mut extra = FingerprintDiff::default();
        extra.extra_primitives.insert(ToolId::new("js.execute"));
        assert!(!extra.is_clean());

        let mut origin = FingerprintDiff::default();
        origin
            .out_of_scope_origins
            .insert("https://attacker.example".to_string());
        assert!(!origin.is_clean());
    }

    #[test]
    fn summary_describes_extras_clearly() {
        let mut diff = FingerprintDiff::default();
        diff.extra_primitives.insert(ToolId::new("js.execute"));
        assert!(diff.summary().contains("js.execute"));
    }

    #[test]
    fn attributions_do_not_affect_cleanliness() {
        // An event that was successfully attributed is not a deviation, even
        // though it is recorded — is_clean must only look at the two flagged
        // sets, never at attributions.
        let diff = FingerprintDiff {
            extra_primitives: HashSet::new(),
            out_of_scope_origins: HashSet::new(),
            attributions: vec![Attribution {
                tool: ToolId::new("dom.read"),
                origin: "https://example.com".to_string(),
                capability: Capability::WebRead,
                specificity: Specificity::Exact,
            }],
        };
        assert!(diff.is_clean());
    }

    #[test]
    fn diff_round_trips_through_serde_json_including_attributions() {
        let diff = FingerprintDiff {
            extra_primitives: [ToolId::new("js.execute")].into_iter().collect(),
            out_of_scope_origins: ["https://attacker.example".to_string()]
                .into_iter()
                .collect(),
            attributions: vec![Attribution {
                tool: ToolId::new("dom.read"),
                origin: "https://example.com".to_string(),
                capability: Capability::WebRead,
                specificity: Specificity::Exact,
            }],
        };
        let json = serde_json::to_string(&diff).unwrap();
        let back: FingerprintDiff = serde_json::from_str(&json).unwrap();
        assert_eq!(diff, back);
    }

    /// `dataset::ExpectedRealization`/serde fixtures authored before
    /// `attributions` existed have no such field — `#[serde(default)]` keeps
    /// them deserializable.
    #[test]
    fn diff_deserializes_without_an_attributions_field() {
        let json = r#"{"extra_primitives":["js.execute"],"out_of_scope_origins":[]}"#;
        let diff: FingerprintDiff = serde_json::from_str(json).unwrap();
        assert!(diff.attributions.is_empty());
    }
}
