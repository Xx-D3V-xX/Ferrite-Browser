//! [`ConsentDecision`]: the user's per-tool approve/reject state after
//! reviewing a [`FingerprintDiff`](super::FingerprintDiff).
//!
//! Unchanged by the T-107 rewrite — A7's charter is the comparison and
//! attribution logic, not consent decisioning (directive §6/A10 owns the
//! consent *UI*; this type is the plain data `ferrite-ui` already builds on
//! top of it). Moved here verbatim from the old flat `comparator.rs` only
//! because that file no longer exists.

use std::collections::HashSet;

use crate::tool_decision::ToolId;

/// The user's per-tool approval/rejection state after reviewing a
/// [`FingerprintDiff`](super::FingerprintDiff).
#[derive(Debug, Default, Clone)]
pub struct ConsentDecision {
    /// Tools the user has approved despite being flagged.
    pub approved: HashSet<ToolId>,
    /// Tools the user has rejected.
    pub rejected: HashSet<ToolId>,
}

impl ConsentDecision {
    /// Returns `true` when every extra primitive in `diff` has been approved
    /// or rejected.
    #[must_use]
    pub fn is_complete(&self, diff: &super::FingerprintDiff) -> bool {
        diff.extra_primitives
            .iter()
            .all(|t| self.approved.contains(t) || self.rejected.contains(t))
    }

    /// Approves `tool`, clearing any prior rejection.
    pub fn approve(&mut self, tool: ToolId) {
        self.rejected.remove(&tool);
        self.approved.insert(tool);
    }

    /// Rejects `tool`, clearing any prior approval.
    pub fn reject(&mut self, tool: ToolId) {
        self.approved.remove(&tool);
        self.rejected.insert(tool);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparator::FingerprintDiff;

    #[test]
    fn consent_is_complete_when_all_decided() {
        let mut diff = FingerprintDiff::default();
        diff.extra_primitives.insert(ToolId::new("js.execute"));
        diff.extra_primitives.insert(ToolId::new("dom.write"));
        let mut decision = ConsentDecision::default();
        assert!(!decision.is_complete(&diff));
        decision.approve(ToolId::new("js.execute"));
        assert!(!decision.is_complete(&diff));
        decision.reject(ToolId::new("dom.write"));
        assert!(decision.is_complete(&diff));
    }

    #[test]
    fn approve_removes_from_rejected() {
        let mut decision = ConsentDecision::default();
        decision.reject(ToolId::new("web.interact"));
        decision.approve(ToolId::new("web.interact"));
        assert!(decision.approved.contains(&ToolId::new("web.interact")));
        assert!(!decision.rejected.contains(&ToolId::new("web.interact")));
    }

    #[test]
    fn reject_removes_from_approved() {
        let mut decision = ConsentDecision::default();
        decision.approve(ToolId::new("web.interact"));
        decision.reject(ToolId::new("web.interact"));
        assert!(decision.rejected.contains(&ToolId::new("web.interact")));
        assert!(!decision.approved.contains(&ToolId::new("web.interact")));
    }
}
