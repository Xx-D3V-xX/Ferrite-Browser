//! The comparator: what the agent actually did, compared against what its
//! task said it might need (`docs/REBUILD_DIRECTIVE.md` §6/A7, "the
//! highest-value fix in the rebuild").
//!
//! This module replaces the old flat `comparator.rs`, whose `compare()` took
//! **one** `OriginScope` for the whole task — a signature that could not
//! express a fingerprint mixing a narrow `scoped.read` on `mail.example.com`
//! with a wide `web.read` for open search results (D1/T-001), and that left
//! `OriginScope::admission_rank()` computed and never consumed (D2/T-002).
//! The old file's local `OriginScope`/`ScopeType` types are gone; every type
//! this module's `compare` reasons about origin-scoping with is
//! [`ferrite_core::scope::OriginScope`]/[`ferrite_core::scope::Specificity`].
//!
//! # New contract
//!
//! ```ignore
//! fn compare(expected: &ExpectedFingerprint, actual: &DryRunRecord) -> FingerprintDiff;
//! ```
//!
//! [`ExpectedFingerprint`] wraps [`ferrite_core::ExpectedCapabilitySet`] —
//! capabilities, each carrying its **own** [`ferrite_core::scope::OriginScope`] —
//! which is already exactly the shape the directive asks for (A2 built it;
//! see [`expected`]'s docs for how a live task's fingerprint gets one today).
//!
//! # Where a per-capability `OriginScope` comes from — the design gap
//!
//! This is the single most important design decision in this charter, so it
//! is spelled out here at length rather than in a commit message no one
//! reads later.
//!
//! `fingerprint::Fingerprint` (A4) does not carry a per-capability scope:
//! `must_use`/`may_use` are plain `BTreeSet<Capability>`. A4's own module
//! docs say so explicitly: an authored `OriginScope` per capability "isn't
//! derivable from a prompt... left for A7's comparator integration." (B1,
//! `docs/TO-DO.md` T-221, deleted the pre-rebuild `tool_decision::ToolFingerprint`
//! this section used to also name here — every real caller now produces a
//! [`Fingerprint`] directly, so there is exactly one fingerprint shape to
//! reason about, not two.)
//!
//! Meanwhile every *caller* of the old `compare()` already had exactly
//! **one** `OriginScope` to give it — `dataset::CaseDefinition::expected_origins`
//! for the eval harness, or a hardcoded `OriginScope::task_open()` in
//! `ferrite-ui` (see that call site's own `TODO(Task 19)` comment, now
//! resolved — read on). So the gap is not "no scope exists anywhere"; it is
//! "no one authors a *different* scope per capability yet — only ever one
//! per task."
//!
//! Resolution chosen (option (a) from the A7 charter): [`ExpectedFingerprint::from_fingerprint`]
//! applies a single scope to every capability in a fingerprint that has none
//! of its own, sourced from the one piece of context a live task actually
//! carries: its declared context URL (`exact([context_url])` when present,
//! `task_open` with a stated rationale when it is not). Its doc comment
//! spells out the policy and its honesty limits in full, **and names it
//! "provisional"** in its own scope rationale text — this is a stand-in for
//! a real per-task authoring mechanism that does not exist yet, not a claim
//! that per-capability precision has been achieved.
//! [`ExpectedFingerprint::from_capabilities`] is the direct, no-defaulting
//! constructor for any caller — present or future — that *does* have a
//! genuine per-capability scope to give, which is what makes this a real fix
//! rather than a relabeled global scope: the type can express
//! per-capability precision the moment something upstream starts authoring
//! it, without another comparator rewrite.
//!
//! **The regression this must never collapse into**: defaulting every
//! capability to `task_open` unconditionally, which would readmit every
//! origin and make the whole T-001 rewrite pointless. This is why
//! `from_fingerprint` only reaches `task_open` when there is *no* context
//! URL — see `expected::tests::from_fingerprint_with_a_context_url_still_flags_a_different_origin`,
//! which is written to fail specifically if that default were ever widened.
//! The live `ferrite-ui` call site (`crates/ferrite-ui/src/lib.rs`) is a
//! second instance of exactly this fix: it already threads the active tab's
//! URL into `AgentTask::context_url`, but its old `compare()` call ignored
//! that and hardcoded `OriginScope::task_open()` — a real instance of "just
//! default to TaskOpen" already living in production. This session's
//! minimal edit to that call site passes the task's own `context_url`
//! through `from_fingerprint`'s bridge instead, so the live path gets real
//! origin narrowing for free rather than staying an open TODO.
//!
//! # `js.execute` — unconditional, first branch, no exceptions (ADR-003)
//!
//! `compare`'s very first check on every event is whether its primitive's
//! [`ferrite_core::ActionClass`] is scopable at all
//! (`ActionClass::is_scopable`). `js.execute` is the one primitive whose
//! class is not, and that check runs *before* any origin is even parsed —
//! so there is no origin, however permissively scoped, that can ever reach
//! the attribution logic for it. `tests::js_execute_is_flagged_even_under_a_scope_that_would_admit_any_origin`
//! is the test that proves the "no exceptions" rule really has none in this
//! code, not just in the ones that were convenient to write.
//!
//! # Attribution and the T-206 double-flag fix
//!
//! For each recorded event whose primitive *is* scopable and whose origin
//! *does* parse as a real [`ferrite_core::Origin`]:
//!
//! 1. Every `(primitive, scope, capability)` triple from
//!    [`ExpectedFingerprint::lowered`] whose primitive matches is a
//!    candidate.
//! 2. Among candidates, `scope.admits(origin)` is called — this is the exact
//!    call D2/T-002 was about: [`ferrite_core::scope::Specificity`]'s `Ord`
//!    is the admission rank, and `compare` picks the candidate with the
//!    *maximum* one (`exact > domain_suffix > task_open`), recording which
//!    capability that was in [`Attribution`]. `tests::admission_rank_is_actually_consumed_not_just_computed`
//!    is written so that reverting to "first match wins" instead of "highest
//!    rank wins" fails it.
//! 3. If no candidate admits the origin: was the primitive a candidate at
//!    all (some capability names it, just not at this origin)? If so, this
//!    is an **`OutOfScopeOrigin`** — the origin string goes into
//!    `out_of_scope_origins`. If *no* capability's realization ever
//!    contained the primitive, this is an **`ExtraPrimitive`** — the tool id
//!    goes into `extra_primitives`.
//!
//! Step 3 is deliberately an `if`/`else`, not two independent checks — this
//! is the T-206 fix. The old `compare()` checked "is the origin admitted?"
//! and, independently, "is the primitive in the flat expected union?",
//! which could set *both* `out_of_scope_origins` and `extra_primitives` for
//! the same event when both questions answered "no". Here, "primitive named
//! by some capability" and "primitive named by no capability" partition the
//! primitive space **exactly** — a primitive is in exactly one of those two
//! states with respect to the whole expected set, independent of any
//! origin — so the two outcomes are mutually exclusive by construction, not
//! by remembering not to double-insert. `tests::t206_regression_primitive_not_expected_and_origin_not_admitted_lands_in_exactly_one_bucket`
//! is the direct regression test.
//!
//! **Why there is no `Both` variant.** The charter asks whether `Both`
//! should exist as a third classification. Having built the fix, the answer
//! is no: `Both` would only ever fire for the exact case T-206 already
//! names — a primitive nobody expects, at an origin nobody admits — and
//! that is fully and correctly described by `ExtraPrimitive` alone (the
//! primitive being unexpected is already sufficient reason to flag it; the
//! origin question is moot once no capability could have admitted the
//! primitive in the first place, at any origin). Introducing `Both` would
//! either (a) duplicate information already implied by `ExtraPrimitive`, or
//! (b) resurrect the double-counting bug under a new name by trying to
//! independently populate two buckets for one event. The classification
//! this module implements is a true partition of "successfully attributed /
//! out-of-scope-origin / extra-primitive" with no overlap, which is the
//! stronger property.
//!
//! # T-211 — opaque and missing origins: ratified, not left open
//!
//! `ferrite_core::ids::Origin::parse` accepts only `http`/`https` by design
//! — its own doc comment states plainly that every other scheme has an
//! *opaque* origin, "which by definition no scope can admit", so admitting
//! one into `Origin` "would be modelling something the scope algebra cannot
//! express." Having read that reasoning together with ADR-004 and A6's
//! handoff, this session **ratifies** that stance rather than adding a
//! `ferrite_core::ids::Origin::Opaque` variant: a variant would have to be
//! unconditionally unadmittable by every `OriginScope` construction to mean
//! anything, which is exactly what "fails to parse, therefore unadmittable"
//! already achieves today, with strictly less surface area (no new match
//! arm every future `OriginScope`/`Origin` consumer has to remember to
//! handle correctly). `compare` treats the two cases A6 flagged explicitly:
//!
//! - An origin string that was recorded but does not parse as `http`/`https`
//!   (`about:blank`, `data:...`, `blob:...`) goes into
//!   `out_of_scope_origins` under its literal string — there *is* a
//!   reportable origin, it is simply one nothing can ever admit, which reads
//!   naturally as "this origin is out of scope" to a consent reviewer.
//! - An event with **no** recorded origin at all (`ToolEvent.origin ==
//!   None`, e.g. an action before the first navigation) has nothing to
//!   report as an offending origin, so it is classified as an
//!   `extra_primitives` entry instead — the same bucket `js.execute` and
//!   `Origin::parse` failures on the primitive side land in, i.e. "cannot
//!   ever be justified", which is the honest description of an action taken
//!   with no origin context to check against any scope.
//!
//! `docs/TO-DO.md`'s T-211 row is updated to `done` (ratified), not left as
//! an open design question.
//!
//! # T-216 — `download.file` vs. `Primitive::Download`: closed, not patched
//!
//! Pre-B1, `ferrite-agent::BrowserTool::DownloadFile::tool_id()` emitted the
//! wire string `"download.file"`; `ferrite_core::Primitive::Download::as_str()`
//! is `"download"` (no dot-suffix) — a real mismatch between two
//! independently-authored string vocabularies, patched with a one-entry
//! lookup (`resolve_primitive`) local to this module.
//!
//! B1 (`docs/TO-DO.md` T-221) removed the mismatch at its source instead of
//! patching around it: `dry_run::record::ToolEvent` now carries a real
//! `ferrite_core::Primitive` directly, tagged by
//! `ferrite_engine::Call::primitive()` at the point the dry-run engine
//! records the call — there is no second, independently-authored wire
//! string for `Primitive::Download` (or anything else) to drift out of sync
//! with. `resolve_primitive` and its one-entry patch table are deleted, not
//! carried forward: `compare` reads `event.primitive` below with no
//! intermediate lookup, and `download_file_attributes_to_web_download_at_an_admitted_origin`
//! (this module's tests) is the compile-level proof the bridge is gone —
//! the test still passes by construction, not by consulting a patch table.
//!
//! # How far this actually reaches
//!
//! - `attributions` records the admitting capability and specificity per
//!   successfully-justified event, in recorded order — the shape A8's audit
//!   log needs to cite "which capability justified this" per entry, not
//!   just "something did". It is **not** deduplicated: a repeated
//!   tool/origin pair produces a repeated `Attribution`, one per actual
//!   occurrence, because the audit log's job is recording what happened,
//!   not summarizing it.
//! - `compare` has no notion of *time* beyond the order `actual.tool_events`
//!   already carries — it does not re-derive `events_with_seq`. A future
//!   caller that needs "attribute event N specifically" rather than "was
//!   this tool/origin pair ever justified" would zip `attributions` back
//!   against `events_with_seq()` by matching tool+origin, which is unambiguous only
//!   as long as duplicate calls to the same tool/origin are attributed
//!   identically — true here, since attribution depends only on
//!   `(primitive, origin)`, never on call order.
//! - `extra_primitives`/`out_of_scope_origins` remain `HashSet`s (pre-rebuild
//!   shape, kept for `dataset`/`ferrite-eval`/`ferrite-ui` compatibility —
//!   see `diff`'s module docs), so a diff answers "was tool X ever flagged"
//!   and "was origin Y ever out of scope", not "on which of N calls". The
//!   ordered `attributions` list is the only place call-level granularity
//!   survives.

mod consent;
mod diff;
mod expected;
mod guard;

pub use consent::ConsentDecision;
pub use diff::{Attribution, FingerprintDiff};
pub use expected::ExpectedFingerprint;
pub use guard::{GuardVerdict, RuntimeGuard};

use ferrite_core::scope::Specificity;
use ferrite_core::{Capability, Origin};

use crate::dry_run::DryRunRecord;
use crate::tool_decision::ToolId;

/// How one observed event relates to an expected fingerprint: the single
/// classification both the post-hoc [`compare`] (over a dry-run record) and the
/// live [`RuntimeGuard`] (one action at a time, before it runs) are built on,
/// so the two can never disagree about the same event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventVerdict {
    /// An expected capability's scope admitted the event.
    Justified(Attribution),
    /// Some expected capability names the primitive, but none admits this
    /// origin (including an opaque origin such as `about:blank`, `data:`).
    OutOfScopeOrigin(String),
    /// No expected capability names the primitive at all, or it is
    /// unconditionally unscopable (`js.execute`), or no origin was recorded.
    ExtraPrimitive(ToolId),
}

/// Classifies one event. See the [module docs](self) for the reasoning behind
/// every branch; `compare` and `RuntimeGuard::check` both call only this.
#[must_use]
pub fn classify_event(
    expected: &ExpectedFingerprint,
    primitive: ferrite_core::Primitive,
    origin: Option<&str>,
) -> EventVerdict {
    let tool_id = ToolId::new(primitive.as_str());

    // ADR-003, first branch, no exceptions: any Execute-class primitive
    // is unconditionally a deviation, before any origin is even parsed.
    if !primitive.action_class().is_scopable() {
        return EventVerdict::ExtraPrimitive(tool_id);
    }

    // T-211: no recorded origin at all — nothing to check any scope
    // against, and nothing reportable as an offending origin either.
    let Some(origin_str) = origin else {
        return EventVerdict::ExtraPrimitive(tool_id);
    };

    // T-211: an origin that does not parse as `http`/`https` is opaque —
    // unadmittable by any scope by definition — but it IS a reportable
    // string, so it is an out-of-scope origin under its literal form.
    let Ok(parsed) = Origin::parse(origin_str) else {
        return EventVerdict::OutOfScopeOrigin(origin_str.to_string());
    };

    // A primitive with no expected-side (`ScopablePrimitive`) counterpart at
    // all — js.execute is already handled above, so in practice every
    // primitive reaching this line has one, but the fallible bridge stays
    // explicit rather than assumed.
    let Some(scopable) = primitive.as_scopable() else {
        return EventVerdict::ExtraPrimitive(tool_id);
    };

    // The admission-rank scan: every lowered triple whose primitive matches
    // is a candidate; among candidates that admit `origin`, keep the one with
    // the greatest Specificity (T-002 — this `Ord` comparison is the actual
    // consumption `admission_rank_is_actually_consumed_not_just_computed`
    // checks for).
    let mut best: Option<(Specificity, Capability)> = None;
    let mut primitive_expected_anywhere = false;
    for (lowered_primitive, scope, capability) in expected.lowered() {
        if lowered_primitive != scopable {
            continue;
        }
        primitive_expected_anywhere = true;
        if let Some(rank) = scope.admits(&parsed) {
            if best.is_none_or(|(best_rank, _)| rank > best_rank) {
                best = Some((rank, capability));
            }
        }
    }

    match best {
        Some((specificity, capability)) => EventVerdict::Justified(Attribution {
            tool: tool_id,
            origin: origin_str.to_string(),
            capability,
            specificity,
        }),
        // T-206: these two outcomes are mutually exclusive by construction —
        // `primitive_expected_anywhere` partitions the primitive space
        // independently of `origin`, so exactly one branch below ever runs
        // for a given event.
        None if primitive_expected_anywhere => {
            EventVerdict::OutOfScopeOrigin(origin_str.to_string())
        }
        None => EventVerdict::ExtraPrimitive(tool_id),
    }
}

/// Compares `expected` against what the agent actually did, per the new
/// contract: per-capability origin scoping, admission-rank attribution, and
/// a T-206-safe classification into exactly one bucket per event. See the
/// [module docs](self) for the full reasoning behind every branch
/// ([`classify_event`]).
#[must_use]
pub fn compare(expected: &ExpectedFingerprint, actual: &DryRunRecord) -> FingerprintDiff {
    let mut diff = FingerprintDiff::default();
    for event in &actual.tool_events {
        match classify_event(expected, event.primitive, event.origin.as_deref()) {
            EventVerdict::Justified(attribution) => diff.attributions.push(attribution),
            EventVerdict::OutOfScopeOrigin(origin) => {
                diff.out_of_scope_origins.insert(origin);
            }
            EventVerdict::ExtraPrimitive(tool) => {
                diff.extra_primitives.insert(tool);
            }
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_core::scope::DomainSuffix;
    use ferrite_core::{ExpectedCapability, ExpectedCapabilitySet};

    fn origin(s: &str) -> Origin {
        Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn exact(s: &str) -> ferrite_core::scope::OriginScope {
        ferrite_core::scope::OriginScope::exact([origin(s)]).expect("non-empty")
    }

    fn suffix_scope(s: &str) -> ferrite_core::scope::OriginScope {
        ferrite_core::scope::OriginScope::domain_suffix([DomainSuffix::parse(s).expect("valid")])
            .expect("non-empty")
    }

    fn task_open(rationale: &str) -> ferrite_core::scope::OriginScope {
        ferrite_core::scope::OriginScope::task_open(rationale).expect("valid")
    }

    /// Test-only convenience: `tool` is the primitive's wire string
    /// (`"dom.read"`, `"js.execute"`, ...) so every existing test literal
    /// below reads exactly as it did pre-B1 — this is a test-fixture
    /// lookup, not a production bridge (`compare` itself never does a
    /// string-to-`Primitive` lookup; see the module docs).
    fn record(events: &[(&str, Option<&str>)]) -> DryRunRecord {
        let mut r = DryRunRecord::new(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        for (tool, origin) in events {
            let primitive = ferrite_core::Primitive::ALL
                .iter()
                .copied()
                .find(|p| p.as_str() == *tool)
                .unwrap_or_else(|| panic!("no Primitive with wire string {tool:?}"));
            r.record_tool(primitive, origin.map(str::to_string));
        }
        r
    }

    fn expected_of(entries: Vec<ExpectedCapability>) -> ExpectedFingerprint {
        ExpectedFingerprint::from_capabilities(
            ExpectedCapabilitySet::new(entries).expect("no duplicate capabilities in these tests"),
        )
    }

    // ── Basic clean / flagged behavior ──────────────────────────────────

    #[test]
    fn clean_when_agent_uses_admitted_primitives_on_admitted_origin() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            exact("https://example.com"),
        )]);
        let record = record(&[
            ("dom.read", Some("https://example.com")),
            ("dom.query", Some("https://example.com")),
        ]);
        let diff = compare(&expected, &record);
        assert!(diff.is_clean(), "{diff:?}");
        assert_eq!(diff.attributions.len(), 2);
        assert!(diff
            .attributions
            .iter()
            .all(|a| a.capability == Capability::WebRead));
    }

    #[test]
    fn detects_extra_primitive_outside_any_capabilitys_realization() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::ScopedRead,
            exact("https://mail.example.com"),
        )]);
        let record = record(&[("dom.write", Some("https://mail.example.com"))]);
        let diff = compare(&expected, &record);
        assert!(!diff.is_clean());
        assert!(diff
            .extra_primitives
            .contains(&crate::tool_decision::ToolId::new("dom.write")));
        assert!(diff.out_of_scope_origins.is_empty());
    }

    #[test]
    fn detects_out_of_scope_origin_for_an_otherwise_expected_primitive() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            exact("https://example.com"),
        )]);
        let record = record(&[("dom.read", Some("https://attacker.example"))]);
        let diff = compare(&expected, &record);
        assert!(!diff.is_clean());
        assert!(diff
            .out_of_scope_origins
            .contains("https://attacker.example"));
        assert!(diff.extra_primitives.is_empty());
    }

    #[test]
    fn domain_suffix_scope_admits_subdomains() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            suffix_scope("example.com"),
        )]);
        let record = record(&[("dom.read", Some("https://docs.example.com"))]);
        assert!(compare(&expected, &record).is_clean());
    }

    #[test]
    fn task_open_scope_admits_any_origin() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            task_open("open browse task"),
        )]);
        let record = record(&[("dom.read", Some("https://anywhere.example"))]);
        let diff = compare(&expected, &record);
        assert!(diff.is_clean());
        assert_eq!(diff.attributions[0].specificity, Specificity::TaskOpen);
    }

    #[test]
    fn every_capability_in_the_expected_set_can_admit_regardless_of_must_or_may_use_origin() {
        // ExpectedFingerprint makes no must_use/may_use distinction of its
        // own — `ExpectedFingerprint::from_legacy_tool_fingerprint` lowers
        // both a ToolFingerprint's must_use and may_use layers identically,
        // matching the pre-rebuild "may_use is not flagged when used within
        // its realization" behavior. Proven here directly against two
        // capabilities that would have come from either layer.
        let expected = expected_of(vec![
            ExpectedCapability::new(Capability::WebRead, exact("https://example.com")),
            ExpectedCapability::new(
                Capability::ClipboardWrite,
                task_open("clipboard write, any origin"),
            ),
        ]);
        let record = record(&[
            ("dom.read", Some("https://example.com")),
            ("clipboard.write", Some("https://example.com")),
        ]);
        let diff = compare(&expected, &record);
        assert!(diff.is_clean(), "{diff:?}");
        assert_eq!(diff.attributions.len(), 2);
    }

    // ── js.execute: unconditional, no exceptions ────────────────────────

    #[test]
    fn js_execute_is_always_flagged_regardless_of_capability_or_origin() {
        let expected = expected_of(vec![
            ExpectedCapability::new(Capability::WebRead, exact("https://example.com")),
            ExpectedCapability::new(Capability::WebInteract, exact("https://example.com")),
        ]);
        let record = record(&[("js.execute", Some("https://example.com"))]);
        let diff = compare(&expected, &record);
        assert!(!diff.is_clean());
        assert!(diff
            .extra_primitives
            .contains(&crate::tool_decision::ToolId::new("js.execute")));
        assert!(diff.attributions.is_empty());
    }

    #[test]
    fn js_execute_is_flagged_even_under_a_scope_that_would_admit_any_origin() {
        // The strongest form of "no exceptions": every capability present is
        // task_open (admits literally any origin), and the event's origin is
        // one that scope would happily admit for any other primitive.
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            task_open("wide open browse task"),
        )]);
        let record = record(&[("js.execute", Some("https://example.com"))]);
        let diff = compare(&expected, &record);
        assert!(diff
            .extra_primitives
            .contains(&crate::tool_decision::ToolId::new("js.execute")));
        assert!(
            diff.attributions.is_empty(),
            "js.execute must never be attributed"
        );
    }

    // ── T-206: exactly one bucket, never both ───────────────────────────

    #[test]
    fn t206_regression_primitive_not_expected_and_origin_not_admitted_lands_in_exactly_one_bucket()
    {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            exact("https://example.com"),
        )]);
        // dom.write is not in WebRead's realization, AND the origin isn't
        // admitted either — the exact double-flag scenario T-206 named.
        let record = record(&[("dom.write", Some("https://attacker.example"))]);
        let diff = compare(&expected, &record);

        let flagged_as_extra = diff
            .extra_primitives
            .contains(&crate::tool_decision::ToolId::new("dom.write"));
        let flagged_as_out_of_scope = diff
            .out_of_scope_origins
            .contains("https://attacker.example");

        assert!(flagged_as_extra, "must be flagged somewhere");
        assert!(
            !flagged_as_out_of_scope,
            "primitive was never expected by anyone, so this must be ExtraPrimitive only, not \
             also OutOfScopeOrigin — T-206's double-flag bug"
        );
    }

    #[test]
    fn primitive_expected_elsewhere_but_wrong_origin_is_out_of_scope_only_not_also_extra() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            exact("https://example.com"),
        )]);
        let record = record(&[("dom.read", Some("https://attacker.example"))]);
        let diff = compare(&expected, &record);
        assert!(diff
            .out_of_scope_origins
            .contains("https://attacker.example"));
        assert!(diff.extra_primitives.is_empty());
    }

    // ── T-211: opaque / missing origins ──────────────────────────────────

    #[test]
    fn an_opaque_scheme_origin_is_out_of_scope_under_its_literal_string() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            task_open("would admit any real http(s) origin"),
        )]);
        for opaque in ["about:blank", "data:text/html,hi", "blob:whatever"] {
            let record = record(&[("dom.read", Some(opaque))]);
            let diff = compare(&expected, &record);
            assert!(
                diff.out_of_scope_origins.contains(opaque),
                "{opaque} must be reported as out-of-scope even under a task_open capability, \
                 because it never reaches Origin::parse successfully"
            );
            assert!(diff.attributions.is_empty());
        }
    }

    #[test]
    fn an_event_with_no_recorded_origin_is_an_extra_primitive() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::ClipboardRead,
            task_open("would admit any origin if there were one"),
        )]);
        let record = record(&[("clipboard.read", None)]);
        let diff = compare(&expected, &record);
        assert!(diff
            .extra_primitives
            .contains(&crate::tool_decision::ToolId::new("clipboard.read")));
    }

    // ── T-216: download.file <-> Primitive::Download ────────────────────

    #[test]
    fn download_file_attributes_to_web_download_at_an_admitted_origin() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebDownload,
            exact("https://files.example"),
        )]);
        let record = record(&[("download", Some("https://files.example"))]);
        let diff = compare(&expected, &record);
        assert!(diff.is_clean(), "{diff:?}");
        assert_eq!(diff.attributions.len(), 1);
        assert_eq!(diff.attributions[0].capability, Capability::WebDownload);
    }

    #[test]
    fn download_file_is_out_of_scope_at_an_unexpected_origin() {
        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebDownload,
            exact("https://files.example"),
        )]);
        let record = record(&[("download", Some("https://attacker.example"))]);
        let diff = compare(&expected, &record);
        assert!(diff
            .out_of_scope_origins
            .contains("https://attacker.example"));
    }

    // ── T-002: admission rank is actually consumed ──────────────────────

    #[test]
    fn admission_rank_is_actually_consumed_not_just_computed() {
        // `ExpectedCapabilitySet` allows only one entry per `Capability`, so
        // this proves consumption in two steps: first, that
        // `ExpectedCapability::admits` — the exact call `compare` makes
        // through each `lowered()` triple's scope — really does return a
        // `DomainSuffix` rank here (not just `Some`/`None`); second, that
        // `compare`'s own output carries that specific rank through to the
        // `Attribution`, rather than dropping it or hardcoding a fixed
        // value. A `compare` that stopped calling `OriginScope::admits` (or
        // ignored its `Specificity`) could not produce this exact result.
        let suffix_only = ExpectedCapability::new(Capability::WebRead, suffix_scope("example.com"));
        assert_eq!(
            suffix_only.admits(
                ferrite_core::ScopablePrimitive::DomRead,
                &origin("https://mail.example.com")
            ),
            Some(Specificity::DomainSuffix)
        );

        let expected = expected_of(vec![ExpectedCapability::new(
            Capability::WebRead,
            suffix_scope("example.com"),
        )]);
        let record = record(&[("dom.read", Some("https://mail.example.com"))]);
        let diff = compare(&expected, &record);
        assert_eq!(diff.attributions.len(), 1);
        assert_eq!(diff.attributions[0].specificity, Specificity::DomainSuffix);
    }

    #[test]
    fn admission_rank_picks_the_tighter_of_two_admitting_capabilities() {
        // A pathological but legal setup proving `compare` picks the MAXIMUM
        // Specificity among every admitting candidate, not merely "some"
        // admitting candidate. WebInteract and WebRead realize disjoint
        // primitive sets in the real taxonomy, so to get two DIFFERENT
        // capabilities both realizing dom.read is impossible by construction
        // (lowering_table_covers_every_scopable_primitive_exactly_once) —
        // which is exactly why this is checked directly against
        // ExpectedCapability::admits (the primitive compare() calls) instead
        // of trying to fabricate an impossible ExpectedCapabilitySet.
        let exact_cap =
            ExpectedCapability::new(Capability::WebRead, exact("https://mail.example.com"));
        let suffix_cap = ExpectedCapability::new(Capability::WebRead, suffix_scope("example.com"));
        let target = origin("https://mail.example.com");
        let ranks = [
            exact_cap.admits(ferrite_core::ScopablePrimitive::DomRead, &target),
            suffix_cap.admits(ferrite_core::ScopablePrimitive::DomRead, &target),
        ];
        assert_eq!(
            ranks.into_iter().flatten().max(),
            Some(Specificity::Exact),
            "max() over both candidates' ranks must pick Exact — the same reduction compare() \
             performs over `lowered()`'s candidates"
        );
    }

    // ── Multi-capability fixture from the A7 charter itself ─────────────

    #[test]
    fn narrow_scoped_read_and_wide_web_read_in_the_same_fingerprint_attribute_independently() {
        // The exact fixture the old single-global-scope signature could not
        // express: scoped.read narrowly scoped to a mail host, alongside
        // web.read wide open for search results, in ONE fingerprint.
        let expected = expected_of(vec![
            ExpectedCapability::new(Capability::ScopedRead, exact("https://mail.example.com")),
            ExpectedCapability::new(Capability::WebRead, task_open("open web search results")),
        ]);

        let record = record(&[
            ("cookie.read", Some("https://mail.example.com")),
            ("dom.read", Some("https://search.example")),
            // Wrong pairing: cookie.read (ScopedRead-only) at the WIDE
            // origin web.read was scoped for — ScopedRead's narrow scope
            // must not admit it just because some OTHER capability is wide
            // open.
            ("cookie.read", Some("https://search.example")),
        ]);
        let diff = compare(&expected, &record);

        assert_eq!(diff.attributions.len(), 2, "{diff:?}");
        assert!(diff
            .attributions
            .iter()
            .any(|a| a.capability == Capability::ScopedRead
                && a.origin == "https://mail.example.com"
                && a.specificity == Specificity::Exact));
        assert!(diff
            .attributions
            .iter()
            .any(|a| a.capability == Capability::WebRead
                && a.origin == "https://search.example"
                && a.specificity == Specificity::TaskOpen));
        assert!(
            diff.out_of_scope_origins.contains("https://search.example"),
            "cookie.read at search.example must be flagged — ScopedRead's own scope doesn't \
             admit it, and it must not borrow web.read's wide scope just because they share a \
             fingerprint"
        );
    }

    // ── Property: determinism (R8) ──────────────────────────────────────

    #[test]
    fn prop_compare_is_deterministic() {
        let expected = expected_of(vec![
            ExpectedCapability::new(Capability::ScopedRead, exact("https://mail.example.com")),
            ExpectedCapability::new(Capability::WebRead, suffix_scope("example.com")),
        ]);
        let record = record(&[
            ("cookie.read", Some("https://mail.example.com")),
            ("dom.read", Some("https://docs.example.com")),
            ("dom.write", Some("https://attacker.example")),
            ("js.execute", Some("https://mail.example.com")),
            ("dom.read", None),
            ("dom.read", Some("data:text/html,hi")),
        ]);
        let a = compare(&expected, &record);
        let b = compare(&expected, &record);
        assert_eq!(a, b);
    }

    // ── Property: monotonicity ───────────────────────────────────────────
    //
    // Widening a capability's scope must never turn a previously-admitted
    // (unflagged) event into a flagged one. Checked exhaustively over a
    // small closed universe, matching `ferrite_core::scope`'s own style —
    // exhaustive enumeration proves the property over the whole tested
    // domain rather than sampling it, deterministically (R8), at no new
    // dependency cost.

    fn widening_pairs() -> Vec<(
        &'static str,
        ferrite_core::scope::OriginScope,
        ferrite_core::scope::OriginScope,
    )> {
        vec![
            (
                "exact -> matching domain_suffix",
                exact("https://mail.example.com"),
                suffix_scope("mail.example.com"),
            ),
            (
                "exact -> parent domain_suffix",
                exact("https://mail.example.com"),
                suffix_scope("example.com"),
            ),
            (
                "domain_suffix -> task_open",
                suffix_scope("example.com"),
                task_open("widened for the test"),
            ),
            (
                "exact -> task_open",
                exact("https://mail.example.com"),
                task_open("widened for the test"),
            ),
        ]
    }

    fn universe_of_origins() -> Vec<&'static str> {
        vec![
            "https://mail.example.com",
            "https://docs.example.com",
            "https://example.com",
            "https://notexample.com",
            "https://attacker.example",
        ]
    }

    #[test]
    fn prop_widening_a_capabilitys_scope_never_turns_an_admitted_event_into_a_flagged_one() {
        for (why, narrow, wide) in widening_pairs() {
            for origin_str in universe_of_origins() {
                let narrow_expected = expected_of(vec![ExpectedCapability::new(
                    Capability::WebRead,
                    narrow.clone(),
                )]);
                let wide_expected = expected_of(vec![ExpectedCapability::new(
                    Capability::WebRead,
                    wide.clone(),
                )]);

                let rec = record(&[("dom.read", Some(origin_str))]);
                let narrow_diff = compare(&narrow_expected, &rec);
                let wide_diff = compare(&wide_expected, &rec);

                let was_admitted = !narrow_diff.out_of_scope_origins.contains(origin_str)
                    && narrow_diff.extra_primitives.is_empty();
                if was_admitted {
                    let still_admitted = !wide_diff.out_of_scope_origins.contains(origin_str)
                        && wide_diff.extra_primitives.is_empty();
                    assert!(
                        still_admitted,
                        "{why}: {origin_str} was admitted under {narrow:?} but flagged under \
                         the wider {wide:?}"
                    );
                }
            }
        }
    }

    // ── Exhaustive small-case enumeration ────────────────────────────────
    //
    // 2 capabilities x 3 origins x 4 primitives, every combination checked
    // against a hand-verified expected classification. This is the fixture
    // from the A7 charter's own example (scoped.read on mail.example.com,
    // web.read on a wider suffix), extended to also exercise js.execute and
    // a wholly-unexpected primitive.

    #[derive(Debug, PartialEq)]
    enum Bucket {
        Attributed(Capability, Specificity),
        OutOfScope,
        Extra,
    }

    fn classify(diff: &FingerprintDiff, tool: &str, origin_str: &str) -> Bucket {
        if let Some(a) = diff
            .attributions
            .iter()
            .find(|a| a.tool == crate::tool_decision::ToolId::new(tool) && a.origin == origin_str)
        {
            return Bucket::Attributed(a.capability, a.specificity);
        }
        if diff.out_of_scope_origins.contains(origin_str) {
            return Bucket::OutOfScope;
        }
        if diff
            .extra_primitives
            .contains(&crate::tool_decision::ToolId::new(tool))
        {
            return Bucket::Extra;
        }
        panic!("event ({tool}, {origin_str}) landed in no bucket at all — compare() dropped it");
    }

    #[test]
    fn exhaustive_two_capability_three_origin_four_primitive_enumeration() {
        // Capabilities: ScopedRead exact on mail.example.com; WebRead
        // domain_suffix on search.test (the charter's own fixture shape).
        let expected = expected_of(vec![
            ExpectedCapability::new(Capability::ScopedRead, exact("https://mail.example.com")),
            ExpectedCapability::new(Capability::WebRead, suffix_scope("search.test")),
        ]);

        let origins = [
            "https://mail.example.com", // admitted by ScopedRead only
            "https://sub.search.test",  // admitted by WebRead only
            "https://attacker.example", // admitted by neither
        ];
        // cookie.read -> ScopedRead only; dom.read -> WebRead only;
        // js.execute -> never, unconditionally; click -> WebInteract, named
        // by neither capability here.
        let primitives = ["cookie.read", "dom.read", "js.execute", "click"];

        // Hand-verified table: (primitive, origin) -> expected bucket.
        let table: Vec<((&str, &str), Bucket)> = vec![
            (
                ("cookie.read", "https://mail.example.com"),
                Bucket::Attributed(Capability::ScopedRead, Specificity::Exact),
            ),
            (
                ("cookie.read", "https://sub.search.test"),
                Bucket::OutOfScope,
            ),
            (
                ("cookie.read", "https://attacker.example"),
                Bucket::OutOfScope,
            ),
            (("dom.read", "https://mail.example.com"), Bucket::OutOfScope),
            (
                ("dom.read", "https://sub.search.test"),
                Bucket::Attributed(Capability::WebRead, Specificity::DomainSuffix),
            ),
            (("dom.read", "https://attacker.example"), Bucket::OutOfScope),
            (("js.execute", "https://mail.example.com"), Bucket::Extra),
            (("js.execute", "https://sub.search.test"), Bucket::Extra),
            (("js.execute", "https://attacker.example"), Bucket::Extra),
            (("click", "https://mail.example.com"), Bucket::Extra),
            (("click", "https://sub.search.test"), Bucket::Extra),
            (("click", "https://attacker.example"), Bucket::Extra),
        ];
        assert_eq!(table.len(), origins.len() * primitives.len());

        for ((primitive, origin_str), expected_bucket) in table {
            let rec = record(&[(primitive, Some(origin_str))]);
            let diff = compare(&expected, &rec);
            let actual_bucket = classify(&diff, primitive, origin_str);
            assert_eq!(
                actual_bucket, expected_bucket,
                "({primitive}, {origin_str})"
            );
        }
    }
}
