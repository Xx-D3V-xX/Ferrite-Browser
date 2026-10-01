//! [`ExpectedFingerprint`] — the expected side of a [`compare`](super::compare)
//! call, and the resolution of the per-capability-`OriginScope`-sourcing gap
//! documented at length in the [parent module's docs](super).
//!
//! `ferrite_core::ExpectedCapabilitySet` (A2) already **is** "capabilities,
//! each carrying its own `OriginScope`" — the exact shape T-001/D1 asks for.
//! This type is a thin wrapper around it plus a constructor that answers the
//! question A2/A4 left open: *where does a live task's per-capability scope
//! actually come from, today?* See [`ExpectedFingerprint::from_fingerprint`]
//! and the module docs for the full reasoning.
//!
//! B1 (`docs/TO-DO.md` T-221) deleted the second such bridge,
//! `from_legacy_tool_fingerprint`, alongside the `tool_decision::ToolFingerprint`
//! type it existed only to convert — every real caller
//! (`ferrite-eval::harness::run_one`, `ferrite-ui`) now calls
//! `tool_decision::ToolDecisionEngine`'s replacement fingerprint method,
//! which returns [`Fingerprint`] directly, and goes through
//! [`ExpectedFingerprint::from_fingerprint`] like every other caller of the
//! real, tested predictor.

use ferrite_core::scope::OriginScope;
use ferrite_core::{
    Capability, ExpectedCapability, ExpectedCapabilitySet, Origin, ScopablePrimitive,
};

use crate::fingerprint::Fingerprint;

/// The provisional rationale recorded on every `task_open` scope
/// [`ExpectedFingerprint::from_fingerprint`] has to fall back to, so it is
/// self-documenting wherever it is read back (a consent UI, an audit entry,
/// a `dataset::ExpectedRealization` dump) rather than a bare `"task_open"`
/// tag with no indication *why* the task was left unscoped.
const PROVISIONAL_NO_CONTEXT_URL_RATIONALE: &str =
    "provisional (A7, T-001): no per-task OriginScope-authoring mechanism exists yet for \
     fingerprint::Fingerprint, and this task carried no context URL to narrow it to — see \
     ferrite_ipi::comparator module docs, \"Where a per-capability OriginScope comes from\"";

/// The expected capability set for one `compare()` call, each capability
/// carrying its own [`OriginScope`] (T-001/D1).
///
/// A wrapper around [`ExpectedCapabilitySet`] rather than a type alias for
/// it, so this module can host the scope-sourcing policy
/// ([`ExpectedFingerprint::from_fingerprint`],
/// [`ExpectedFingerprint::from_legacy_tool_fingerprint`]) next to the type it
/// produces, without implying every future caller must go through one of
/// those two provisional bridges — [`ExpectedFingerprint::from_capabilities`]
/// is the direct, no-defaulting constructor for a caller (a real per-task
/// author, or `ferrite-eval`'s dataset once it is migrated) that already has
/// a genuine per-capability scope for every entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedFingerprint {
    capabilities: ExpectedCapabilitySet,
}

impl ExpectedFingerprint {
    /// Wraps an already-built [`ExpectedCapabilitySet`] directly — the
    /// preferred constructor for any caller that has authored a real
    /// per-capability scope for every entry, with no defaulting involved.
    #[must_use]
    pub fn from_capabilities(capabilities: ExpectedCapabilitySet) -> Self {
        Self { capabilities }
    }

    /// The fail-to-empty expected fingerprint: no capability, so
    /// [`ExpectedFingerprint::lowered`] yields nothing and every observed
    /// event becomes a deviation once compared.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            capabilities: ExpectedCapabilitySet::empty(),
        }
    }

    /// Builds an [`ExpectedFingerprint`] from A4's [`Fingerprint`] (a
    /// `BTreeSet<Capability>` `must_use`/`may_use` pair with **no** scope of
    /// its own — A4 deferred that explicitly, see `fingerprint`'s module
    /// docs) plus the one piece of scope context a live task actually has
    /// today: its declared context URL.
    ///
    /// # The default policy, spelled out
    ///
    /// Every capability in `fp.must_use()` and `fp.may_use()` gets the
    /// *same* scope:
    /// - `context_url` is `Some(origin)` → [`OriginScope::exact`] on that one
    ///   origin. This is the tightest scope any current caller can honestly
    ///   justify without a real per-task authoring step: the task's own
    ///   declared starting point is the one origin we know it needs, and
    ///   nothing else is known to be needed. It is deliberately *not*
    ///   differentiated per capability (e.g. `scoped.read` narrowed to a
    ///   mail host, `web.read` left open for search results) — that would be
    ///   fabricating precision no signal in a bare prompt + context URL
    ///   actually supports. [`ExpectedFingerprint::from_capabilities`] is
    ///   the escape hatch once a caller *does* have that signal (see
    ///   `docs/TO-DO.md`'s new row for wiring `dataset::CaseDefinition`'s
    ///   scope-rationale field, or a per-task authoring UI, into this path).
    /// - `context_url` is `None` → [`OriginScope::task_open`], the weakest
    ///   tier, carrying [`PROVISIONAL_NO_CONTEXT_URL_RATIONALE`] so anyone
    ///   reading the scope back (consent UI, audit entry, `PROGRESS.md`
    ///   citation) sees *why* the task went unscoped rather than a bare tag.
    ///   `task_open` is stratified as weak-scope by ADR-004's own contract,
    ///   so a fingerprint built this way is honestly reported as
    ///   low-confidence containment, not silently folded into the same
    ///   bucket as an authored, deliberate `task_open`.
    ///
    /// # Why this is not "everything is `task_open`"
    ///
    /// The reflex regression this function must never collapse into is
    /// "since we don't have real per-capability scopes yet, just make
    /// everything `task_open` and let the loop run open". `task_open` is the
    /// *weakest* [`ferrite_core::scope::Specificity`] tier — it admits every
    /// origin — so defaulting to it universally would make every event
    /// admitted, which makes T-001's whole per-capability-scope rewrite
    /// pointless: the comparator would never flag an out-of-scope origin
    /// again, for any task that has a context URL. This function only
    /// reaches `task_open` when there is *no* context URL to be tighter
    /// with; whenever one exists, every capability is `exact`-scoped to it,
    /// which does admit-vs-flag real work (`tests::from_fingerprint_with_a_context_url_still_flags_a_different_origin`
    /// is the regression test that would fail if this ever silently
    /// degraded back to always-`task_open`).
    #[must_use]
    pub fn from_fingerprint(fp: &Fingerprint, context_url: Option<&Origin>) -> Self {
        let scope_for_each = || -> OriginScope {
            match context_url {
                Some(origin) => {
                    OriginScope::exact([origin.clone()]).expect("one origin is never empty")
                }
                None => OriginScope::task_open(PROVISIONAL_NO_CONTEXT_URL_RATIONALE)
                    .expect("the rationale constant is non-blank"),
            }
        };

        let entries = fp
            .must_use()
            .iter()
            .chain(fp.may_use())
            .map(|capability| ExpectedCapability::new(*capability, scope_for_each()));

        // must_use/may_use are disjoint by construction (`Fingerprint::new`),
        // so this can never actually hit the duplicate-capability error —
        // but fail to empty rather than panic if that invariant is ever
        // broken upstream, per this crate's own "never a bypass" discipline.
        let capabilities =
            ExpectedCapabilitySet::new(entries).unwrap_or_else(|_| ExpectedCapabilitySet::empty());

        Self { capabilities }
    }

    /// The underlying per-capability-scoped set.
    #[must_use]
    pub fn capabilities(&self) -> &ExpectedCapabilitySet {
        &self.capabilities
    }

    /// Lowers to `(primitive, scope, capability)` triples — see
    /// [`ExpectedCapabilitySet::lowered`], which this delegates to directly
    /// rather than re-deriving.
    #[must_use]
    pub fn lowered(&self) -> Vec<(ScopablePrimitive, &OriginScope, Capability)> {
        self.capabilities.lowered()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_core::scope::Specificity;
    use ferrite_model::MockProvider;

    fn origin(s: &str) -> Origin {
        Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[tokio::test]
    async fn from_fingerprint_with_a_context_url_scopes_every_capability_exact_to_it() {
        let mock = MockProvider::new().push_content("[]");
        let fp =
            crate::fingerprint::generate_fingerprint(&mock, "test-tag", "check my inbox").await;
        assert!(
            !fp.must_use().is_empty(),
            "the rule layer must fire on 'inbox'"
        );

        let ctx = origin("https://mail.example.com");
        let expected = ExpectedFingerprint::from_fingerprint(&fp, Some(&ctx));

        for (_, scope, _) in expected.lowered() {
            assert_eq!(
                scope.admits(&ctx),
                Some(Specificity::Exact),
                "every capability should be exact-scoped to the context URL"
            );
        }
    }

    #[tokio::test]
    async fn from_fingerprint_with_no_context_url_falls_back_to_a_documented_task_open() {
        let mock = MockProvider::new().push_content("[]");
        let fp =
            crate::fingerprint::generate_fingerprint(&mock, "test-tag", "check my inbox").await;
        let expected = ExpectedFingerprint::from_fingerprint(&fp, None);

        let (_, scope, _) = expected
            .lowered()
            .into_iter()
            .next()
            .expect("must_use is non-empty for this prompt");
        assert_eq!(scope.tier(), Specificity::TaskOpen);
        assert!(
            matches!(scope, OriginScope::TaskOpen { rationale } if rationale.contains("provisional"))
        );
    }

    /// The regression this design gap must never collapse back into: if
    /// `from_fingerprint` silently defaulted every capability to `task_open`
    /// regardless of `context_url` (the "everything is TaskOpen" failure the
    /// A7 charter calls out by name), this test would fail, because
    /// `task_open` admits every origin and an attacker origin would never be
    /// flagged.
    #[tokio::test]
    async fn from_fingerprint_with_a_context_url_still_flags_a_different_origin() {
        let mock = MockProvider::new().push_content("[]");
        let fp =
            crate::fingerprint::generate_fingerprint(&mock, "test-tag", "check my inbox").await;
        let ctx = origin("https://mail.example.com");
        let expected = ExpectedFingerprint::from_fingerprint(&fp, Some(&ctx));

        let attacker = origin("https://attacker.example");
        for (_, scope, _) in expected.lowered() {
            assert_eq!(
                scope.admits(&attacker),
                None,
                "an exact-scoped capability must not admit an unrelated origin — if this \
                 fails, from_fingerprint has regressed to defaulting every capability to \
                 task_open regardless of context_url"
            );
        }
    }

    #[test]
    fn empty_lowers_to_nothing() {
        assert!(ExpectedFingerprint::empty().lowered().is_empty());
    }
}
