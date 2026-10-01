//! The runtime guard: the predicted fingerprint, enforced while the real run
//! executes (ADR-014).
//!
//! # The gap this closes
//!
//! The dry run executes the agent against *synthetic* pages with no real
//! network reachable, so a page that carries an injected instruction exists
//! only in the real run: nothing the dry run did could reflect it. Before the
//! guard, the live loop compared the dry run's record against the prediction,
//! asked for consent once, and then ran every real action except the ones the
//! user had explicitly rejected. A deviation that appeared only in the real run
//! was never compared against anything. The guard makes the prediction binding
//! on the real run: before each action executes it is classified by exactly the
//! function [`compare`](super::compare) uses ([`classify_event`]), and an
//! action that is neither expected nor approved does not execute.
//!
//! What is allowed is therefore *expected ∪ approved*: the predicted
//! capabilities (with their origin scopes), plus the tools and origins the user
//! approved in this task's consent prompt. Everything else is blocked, which is
//! the same fail-to-empty posture as the rest of the defense: an empty
//! fingerprint blocks everything it has not been given consent for.
//!
//! The guard is pure (no I/O, no clock), so the "same classification as
//! `compare`" property is checked directly by a property test below and the
//! enforcement itself is checked in the eval harness's runtime-guard run.

use std::collections::HashSet;

use ferrite_core::Primitive;

use super::{classify_event, Attribution, EventVerdict, ExpectedFingerprint};
use crate::tool_decision::ToolId;

/// What the guard decided about one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardVerdict {
    /// An expected capability's scope admits it.
    Expected(Attribution),
    /// Outside the prediction, but the user approved exactly this deviation
    /// (the tool, or the origin) in this task's consent prompt.
    Approved(EventVerdict),
    /// Outside the prediction and not approved: the action must not run.
    Block(EventVerdict),
}

impl GuardVerdict {
    /// Whether the action may run.
    #[must_use]
    pub fn allows(&self) -> bool {
        !matches!(self, Self::Block(_))
    }

    /// One plain-English sentence for the log and the agent's observation. It
    /// names the action and the origin only, never anything a page wrote.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Expected(a) => format!(
                "{} at {} is what this task was expected to need",
                a.tool, a.origin
            ),
            Self::Approved(v) => format!("{} was approved when you were asked", describe_event(v)),
            Self::Block(v) => format!(
                "{} is outside what this task was expected to need, and was not approved",
                describe_event(v)
            ),
        }
    }
}

fn describe_event(verdict: &EventVerdict) -> String {
    match verdict {
        EventVerdict::Justified(a) => format!("{} at {}", a.tool, a.origin),
        EventVerdict::OutOfScopeOrigin(origin) => format!("an action at {origin}"),
        EventVerdict::ExtraPrimitive(tool) => format!("the tool {tool}"),
    }
}

/// Normalizes an origin string so the approved set and a live action compare
/// equal however they were spelled (`https://X.example:443/a` and
/// `https://x.example`); an opaque origin (`about:blank`) stays literal.
fn normalized(origin: &str) -> String {
    ferrite_core::Origin::parse(origin)
        .map_or_else(|_| origin.to_string(), |o| o.as_str().to_string())
}

/// The prediction plus the user's approvals, checked action by action.
#[derive(Debug, Clone)]
pub struct RuntimeGuard {
    expected: ExpectedFingerprint,
    approved_tools: HashSet<ToolId>,
    approved_origins: HashSet<String>,
}

impl RuntimeGuard {
    /// A guard that allows only what `expected` admits.
    #[must_use]
    pub fn new(expected: ExpectedFingerprint) -> Self {
        Self {
            expected,
            approved_tools: HashSet::new(),
            approved_origins: HashSet::new(),
        }
    }

    /// Adds what the user approved in the consent prompt: tool ids (for
    /// primitives no capability names) and origins (for origins no scope admits).
    #[must_use]
    pub fn with_approvals(
        mut self,
        tools: impl IntoIterator<Item = ToolId>,
        origins: impl IntoIterator<Item = String>,
    ) -> Self {
        self.approved_tools.extend(tools);
        self.approved_origins
            .extend(origins.into_iter().map(|o| normalized(&o)));
        self
    }

    /// Classifies one action: `primitive` at `origin` (`None` when no origin is
    /// known, which is itself a deviation).
    #[must_use]
    pub fn check(&self, primitive: Primitive, origin: Option<&str>) -> GuardVerdict {
        match classify_event(&self.expected, primitive, origin) {
            EventVerdict::Justified(attribution) => GuardVerdict::Expected(attribution),
            verdict @ EventVerdict::ExtraPrimitive(_) => {
                let EventVerdict::ExtraPrimitive(tool) = &verdict else {
                    unreachable!("matched above")
                };
                if self.approved_tools.contains(tool) {
                    GuardVerdict::Approved(verdict)
                } else {
                    GuardVerdict::Block(verdict)
                }
            }
            verdict @ EventVerdict::OutOfScopeOrigin(_) => {
                let EventVerdict::OutOfScopeOrigin(origin) = &verdict else {
                    unreachable!("matched above")
                };
                if self.approved_origins.contains(&normalized(origin)) {
                    GuardVerdict::Approved(verdict)
                } else {
                    GuardVerdict::Block(verdict)
                }
            }
        }
    }

    /// Checks an action that counts as either of two effects and allows it if
    /// **either** is allowed: a click on a link that leads to the origin the
    /// tab is already at is both "a click here" and "a navigation here", and a
    /// task scoped to navigating that origin needs no separate permission to
    /// click. When neither is allowed the first effect's verdict is returned.
    ///
    /// This only ever relaxes between two effects that are both inside the
    /// same origin, so it admits nothing a `navigate` action to that origin
    /// would not already admit.
    #[must_use]
    pub fn check_either(
        &self,
        first: (Primitive, Option<&str>),
        second: (Primitive, Option<&str>),
    ) -> GuardVerdict {
        let a = self.check(first.0, first.1);
        if a.allows() {
            return a;
        }
        let b = self.check(second.0, second.1);
        if b.allows() {
            b
        } else {
            a
        }
    }

    /// Checks every effect one action has (a click on a cross-origin link is
    /// both a click here and a navigation there) and returns the first
    /// blocking verdict, or the last allowing one when nothing blocks.
    ///
    /// An empty slice is treated as a block (fail closed): every action has at
    /// least one effect, so an empty list means the caller lost track of it.
    #[must_use]
    pub fn check_all(&self, effects: &[(Primitive, Option<String>)]) -> GuardVerdict {
        let mut last = None;
        for (primitive, origin) in effects {
            let verdict = self.check(*primitive, origin.as_deref());
            if !verdict.allows() {
                return verdict;
            }
            last = Some(verdict);
        }
        last.unwrap_or_else(|| {
            GuardVerdict::Block(EventVerdict::ExtraPrimitive(ToolId::new("unknown")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparator::compare;
    use crate::dry_run::DryRunRecord;
    use ferrite_core::scope::{DomainSuffix, OriginScope};
    use ferrite_core::{Capability, ExpectedCapability, ExpectedCapabilitySet, Origin};

    fn origin(s: &str) -> Origin {
        Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn fingerprint(caps: &[(Capability, OriginScope)]) -> ExpectedFingerprint {
        ExpectedFingerprint::from_capabilities(
            ExpectedCapabilitySet::new(
                caps.iter()
                    .map(|(c, s)| ExpectedCapability::new(*c, s.clone())),
            )
            .expect("distinct capabilities"),
        )
    }

    fn exact(s: &str) -> OriginScope {
        OriginScope::exact([origin(s)]).expect("non-empty")
    }

    fn mail_task() -> ExpectedFingerprint {
        fingerprint(&[
            (Capability::ScopedRead, exact("https://mail.example")),
            (Capability::WebNavigate, exact("https://mail.example")),
        ])
    }

    #[test]
    fn expected_actions_run_and_attribution_is_reported() {
        let guard = RuntimeGuard::new(mail_task());
        let v = guard.check(Primitive::Navigate, Some("https://mail.example/inbox"));
        assert!(v.allows());
        assert!(matches!(v, GuardVerdict::Expected(a) if a.capability == Capability::WebNavigate));
    }

    #[test]
    fn an_action_at_an_unexpected_origin_is_blocked() {
        let guard = RuntimeGuard::new(mail_task());
        let v = guard.check(Primitive::Navigate, Some("https://attacker.example"));
        assert!(!v.allows());
        assert_eq!(
            v,
            GuardVerdict::Block(EventVerdict::OutOfScopeOrigin(
                "https://attacker.example".into()
            ))
        );
        assert!(v.describe().contains("attacker.example") && v.describe().contains("not approved"));
    }

    #[test]
    fn an_unexpected_primitive_is_blocked_at_any_origin() {
        let guard = RuntimeGuard::new(mail_task());
        for p in [
            Primitive::Download,
            Primitive::ClipboardRead,
            Primitive::Click,
            Primitive::FormFill,
        ] {
            let v = guard.check(p, Some("https://mail.example"));
            assert!(
                matches!(v, GuardVerdict::Block(EventVerdict::ExtraPrimitive(_))),
                "{p:?}"
            );
        }
    }

    #[test]
    fn js_execute_is_blocked_under_any_scope() {
        let open = OriginScope::task_open("anything goes").expect("rationale");
        let wide = fingerprint(&[
            (Capability::WebRead, open.clone()),
            (Capability::WebInteract, open.clone()),
            (Capability::WebNavigate, open),
        ]);
        let guard = RuntimeGuard::new(wide);
        assert!(!guard
            .check(Primitive::JsExecute, Some("https://mail.example"))
            .allows());
        assert!(!guard.check(Primitive::JsExecute, None).allows());
    }

    #[test]
    fn an_empty_fingerprint_blocks_everything() {
        let guard = RuntimeGuard::new(ExpectedFingerprint::empty());
        for p in [
            Primitive::Navigate,
            Primitive::DomRead,
            Primitive::Click,
            Primitive::Scroll,
        ] {
            assert!(
                !guard.check(p, Some("https://mail.example")).allows(),
                "{p:?}"
            );
        }
    }

    #[test]
    fn a_missing_or_opaque_origin_is_blocked() {
        let guard = RuntimeGuard::new(mail_task());
        assert!(!guard.check(Primitive::DomRead, None).allows());
        for opaque in [
            "about:blank",
            "data:text/html,x",
            "blob:https://mail.example/1",
            "javascript:alert(1)",
        ] {
            assert!(
                !guard.check(Primitive::DomRead, Some(opaque)).allows(),
                "{opaque}"
            );
        }
    }

    #[test]
    fn approvals_allow_exactly_what_was_approved() {
        let guard = RuntimeGuard::new(mail_task()).with_approvals(
            [ToolId::new("download")],
            ["https://files.example:443/x".to_string()],
        );
        // The approved tool passes; a different unexpected tool does not.
        assert!(guard
            .check(Primitive::Download, Some("https://mail.example"))
            .allows());
        assert!(!guard
            .check(Primitive::ClipboardRead, Some("https://mail.example"))
            .allows());
        // The approved origin passes (however spelled); another one does not.
        assert!(matches!(
            guard.check(Primitive::Navigate, Some("https://FILES.example")),
            GuardVerdict::Approved(_)
        ));
        assert!(!guard
            .check(Primitive::Navigate, Some("https://files.example:8443"))
            .allows());
        assert!(!guard
            .check(Primitive::Navigate, Some("https://other.example"))
            .allows());
    }

    #[test]
    fn approving_an_origin_does_not_approve_a_primitive_nobody_expects() {
        let guard = RuntimeGuard::new(mail_task())
            .with_approvals([], ["https://files.example".to_string()]);
        // Click is not expected at all, so it is an ExtraPrimitive verdict, which
        // an *origin* approval does not cover.
        assert!(!guard
            .check(Primitive::Click, Some("https://files.example"))
            .allows());
    }

    #[test]
    fn check_either_allows_when_one_of_the_two_effects_is_admitted() {
        let nav_only = RuntimeGuard::new(fingerprint(&[(
            Capability::WebNavigate,
            exact("https://docs.example"),
        )]));
        let click_only = RuntimeGuard::new(fingerprint(&[(
            Capability::WebInteract,
            exact("https://docs.example"),
        )]));
        let here = Some("https://docs.example");
        for guard in [&nav_only, &click_only] {
            assert!(guard
                .check_either((Primitive::Click, here), (Primitive::Navigate, here))
                .allows());
        }
        // Neither admitted (read-only task): blocked, and the click is what is reported.
        let read_only = RuntimeGuard::new(fingerprint(&[(
            Capability::WebRead,
            exact("https://docs.example"),
        )]));
        let v = read_only.check_either((Primitive::Click, here), (Primitive::Navigate, here));
        assert!(!v.allows());
        assert!(matches!(
            v,
            GuardVerdict::Block(EventVerdict::ExtraPrimitive(_))
        ));
    }

    #[test]
    fn check_either_never_admits_another_origin() {
        let nav = RuntimeGuard::new(fingerprint(&[(
            Capability::WebNavigate,
            exact("https://docs.example"),
        )]));
        let v = nav.check_either(
            (Primitive::Click, Some("https://evil.example")),
            (Primitive::Navigate, Some("https://evil.example")),
        );
        assert!(!v.allows());
    }

    #[test]
    fn check_all_blocks_on_any_blocked_effect() {
        let task = fingerprint(&[
            (Capability::WebInteract, exact("https://shop.example")),
            (Capability::WebNavigate, exact("https://shop.example")),
        ]);
        let guard = RuntimeGuard::new(task);
        // A click on a link that leaves the site is a click here and a navigation there.
        let v = guard.check_all(&[
            (Primitive::Click, Some("https://shop.example".into())),
            (Primitive::Navigate, Some("https://attacker.example".into())),
        ]);
        assert_eq!(
            v,
            GuardVerdict::Block(EventVerdict::OutOfScopeOrigin(
                "https://attacker.example".into()
            ))
        );
        let ok = guard.check_all(&[
            (Primitive::Click, Some("https://shop.example".into())),
            (
                Primitive::Navigate,
                Some("https://shop.example/cart".into()),
            ),
        ]);
        assert!(ok.allows());
    }

    #[test]
    fn domain_suffix_scopes_admit_subdomains_but_not_lookalikes() {
        let scope =
            OriginScope::domain_suffix([DomainSuffix::parse("shop.example").expect("valid")])
                .expect("non-empty");
        let guard = RuntimeGuard::new(fingerprint(&[(Capability::WebNavigate, scope)]));
        assert!(guard
            .check(Primitive::Navigate, Some("https://pay.shop.example"))
            .allows());
        assert!(!guard
            .check(Primitive::Navigate, Some("https://shop.example.evil.com"))
            .allows());
        assert!(!guard
            .check(Primitive::Navigate, Some("https://shop.example@evil.com"))
            .allows());
    }

    /// The guard and the post-hoc comparator must agree on every event: a
    /// deterministic sweep over primitives x origins x fingerprints (a real
    /// property test, without a randomness dependency).
    #[test]
    fn the_guard_and_compare_classify_every_event_identically() {
        let open = OriginScope::task_open("sweep").expect("rationale");
        let suffix = OriginScope::domain_suffix([DomainSuffix::parse("a.example").expect("valid")])
            .expect("non-empty");
        let fingerprints = [
            ExpectedFingerprint::empty(),
            mail_task(),
            fingerprint(&[(Capability::WebRead, open.clone())]),
            fingerprint(&[
                (Capability::WebRead, exact("https://a.example")),
                (Capability::WebInteract, suffix),
            ]),
            fingerprint(&[
                (Capability::WebRead, open.clone()),
                (Capability::WebInteract, open.clone()),
                (Capability::WebNavigate, open.clone()),
                (Capability::WebDownload, open),
            ]),
        ];
        let origins: [Option<&str>; 9] = [
            None,
            Some("https://mail.example"),
            Some("https://a.example"),
            Some("https://sub.a.example"),
            Some("https://attacker.example"),
            Some("about:blank"),
            Some("data:text/html,x"),
            Some("https://mail.example:8443"),
            Some("not a url"),
        ];
        for expected in &fingerprints {
            let guard = RuntimeGuard::new(expected.clone());
            for primitive in Primitive::ALL {
                for origin in origins {
                    let mut record = DryRunRecord::default();
                    record.record_tool(*primitive, origin.map(str::to_string));
                    let diff = compare(expected, &record);
                    let verdict = guard.check(*primitive, origin);
                    assert_eq!(
                        verdict.allows(),
                        diff.is_clean(),
                        "guard and compare disagree on {primitive:?} at {origin:?}"
                    );
                }
            }
        }
    }
}
