//! Origin scoping: where a capability is authorized to act.
//!
//! Per ADR-004, `expected_origins` is authored per case as a *typed* scope
//! rather than a flat allowlist, because a flat list either over-restricts
//! (false positives on legitimate result pages) or gets padded wide enough to
//! admit the attack it was meant to exclude.
//!
//! # Specificity is the admission rank
//!
//! [`Specificity`]'s [`Ord`] implementation **is** the admission rank D2/T-002
//! is about. The pre-rebuild code computed an `admission_rank()` that nothing
//! consumed; here the rank is the return value of [`OriginScope::admits`], so
//! a caller cannot ask "does this scope admit this origin?" without also
//! being handed the rank to attribute by. A7's comparator picks the admitting
//! capability with the maximum [`Specificity`] (`Exact > DomainSuffix >
//! TaskOpen`), which is least privilege: the narrowest capability that admits
//! an origin defines the blast radius attributed to it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ScopeError;
use crate::ids::Origin;

/// How tightly a scope admitted an origin — the admission rank.
///
/// Ordered `TaskOpen < DomainSuffix < Exact`, so `max` picks the narrowest
/// justification, per ADR-004's precedence `exact > domain_suffix >
/// task_open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Specificity {
    /// Admitted by a `task_open` scope — the weak tier, reported separately
    /// in stratified containment results.
    TaskOpen,
    /// Admitted by a `domain_suffix` scope — a bounded family of hosts.
    DomainSuffix,
    /// Admitted by an `exact` scope — a specific named origin, the tightest.
    Exact,
}

/// A validated domain suffix, e.g. `example.com`, matching that domain and
/// any subdomain of it.
///
/// This is a newtype rather than a bare `String` on purpose: an unnormalized
/// suffix is the one way a scope can fail *open* (`".com"`, `"Example.Com"`
/// or `"*.example.com"` silently matching more or less than the author
/// intended), so normalization happens once, at construction, and a
/// hand-written Rust literal cannot bypass it. This is the same lesson as
/// defect D6 applied before it can become a defect here.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DomainSuffix(String);

/// Where one capability is authorized to act (ADR-004).
///
/// Construct via [`OriginScope::exact`], [`OriginScope::domain_suffix`] or
/// [`OriginScope::task_open`], all of which validate. The variants are public
/// so the comparator and the consent UI can match on them, and their payload
/// element types ([`Origin`], [`DomainSuffix`]) are themselves validated, so
/// the only invariant a hand-built literal can break is emptiness — which
/// fails *closed* (an empty scope admits nothing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", try_from = "OriginScopeWire")]
pub enum OriginScope {
    /// Specific known origins; the tightest scope.
    Exact(Vec<Origin>),
    /// A bounded family of hosts (`example.com` admits `mail.example.com`).
    DomainSuffix(Vec<DomainSuffix>),
    /// Genuinely open browsing. Requires a written rationale and is reported
    /// as weak-scope in stratified results.
    TaskOpen {
        /// Why this task cannot be bounded to named origins.
        rationale: String,
    },
}

/// Deserialization shadow of [`OriginScope`], so that JSON-authored scopes go
/// through the same validation as Rust-constructed ones.
///
/// Its shape must stay identical to `OriginScope`'s `Serialize` output; the
/// golden-fixture tests in `tests/schema_stability.rs` are what hold that
/// true.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OriginScopeWire {
    Exact(Vec<Origin>),
    DomainSuffix(Vec<DomainSuffix>),
    TaskOpen { rationale: String },
}

impl DomainSuffix {
    /// Normalizes and validates a domain suffix.
    ///
    /// Accepts the authoring spellings `example.com`, `.example.com` and
    /// `*.example.com`, all of which normalize to `example.com`, and
    /// lowercases the result.
    ///
    /// # Errors
    ///
    /// [`ScopeError::InvalidDomainSuffix`] if, after normalization, the value
    /// is empty, has an empty label, or contains a character that cannot
    /// appear in a hostname.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, ScopeError> {
        let raw = value.as_ref();
        let invalid = |reason: &'static str| ScopeError::InvalidDomainSuffix {
            value: raw.to_owned(),
            reason,
        };

        let normalized = raw
            .trim()
            .trim_start_matches("*.")
            .trim_start_matches('.')
            .trim_end_matches('.')
            .to_ascii_lowercase();

        if normalized.is_empty() {
            return Err(invalid("empty after normalization"));
        }
        if normalized
            .chars()
            .any(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-' | '.'))
        {
            return Err(invalid(
                "only ASCII letters, digits, '-' and '.' may appear in a host",
            ));
        }
        if normalized.split('.').any(str::is_empty) {
            return Err(invalid("contains an empty label"));
        }
        // T-212: a suffix that is itself a public suffix (`com`, `co.uk`,
        // `github.io`, `localhost`) admits every site registered beneath it.
        // That is the one way a scope can fail open by authoring error, so it
        // is refused here, where every route (Rust literal, JSON corpus, serde)
        // passes. An all-numeric last label is an IP address, never a suffix.
        if normalized
            .rsplit('.')
            .next()
            .is_some_and(|last| last.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(invalid(
                "a numeric (IP-address-like) value is not a domain suffix",
            ));
        }
        if psl::domain_str(&normalized).is_none() {
            return Err(invalid(
                "is a public suffix, which would admit every site beneath it; name a registrable domain such as example.com",
            ));
        }

        Ok(Self(normalized))
    }

    /// The normalized suffix, e.g. `example.com`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `host` is this domain or a subdomain of it.
    ///
    /// Matches on label boundaries, so `example.com` admits `example.com` and
    /// `mail.example.com` but **not** `notexample.com`. `host` is expected in
    /// the normalized form [`Origin::host`] produces.
    #[must_use]
    pub fn matches(&self, host: &str) -> bool {
        let suffix = self.as_str();
        if host == suffix {
            return true;
        }
        host.len() > suffix.len()
            && host.ends_with(suffix)
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
    }
}

impl OriginScope {
    /// A scope admitting exactly the listed origins.
    ///
    /// # Errors
    ///
    /// [`ScopeError::Empty`] if no origins were given.
    pub fn exact(origins: impl IntoIterator<Item = Origin>) -> Result<Self, ScopeError> {
        let origins: Vec<Origin> = origins.into_iter().collect();
        if origins.is_empty() {
            return Err(ScopeError::Empty { kind: "exact" });
        }
        Ok(Self::Exact(origins))
    }

    /// A scope admitting the listed domains and their subdomains.
    ///
    /// # Errors
    ///
    /// [`ScopeError::Empty`] if no suffixes were given.
    pub fn domain_suffix(
        suffixes: impl IntoIterator<Item = DomainSuffix>,
    ) -> Result<Self, ScopeError> {
        let suffixes: Vec<DomainSuffix> = suffixes.into_iter().collect();
        if suffixes.is_empty() {
            return Err(ScopeError::Empty {
                kind: "domain_suffix",
            });
        }
        Ok(Self::DomainSuffix(suffixes))
    }

    /// A scope admitting any origin, justified by a written rationale.
    ///
    /// # Errors
    ///
    /// [`ScopeError::MissingRationale`] if the rationale is blank.
    pub fn task_open(rationale: impl Into<String>) -> Result<Self, ScopeError> {
        let rationale = rationale.into();
        if rationale.trim().is_empty() {
            return Err(ScopeError::MissingRationale);
        }
        Ok(Self::TaskOpen { rationale })
    }

    /// Whether this scope admits `origin`, and if so how tightly.
    ///
    /// The returned [`Specificity`] is the admission rank: the comparator
    /// attributes an event to the admitting capability with the greatest one
    /// (T-002/D2 — the rank is not computable without also being consumed).
    #[must_use]
    pub fn admits(&self, origin: &Origin) -> Option<Specificity> {
        match self {
            Self::Exact(origins) => origins
                .iter()
                .any(|o| o == origin)
                .then_some(Specificity::Exact),
            Self::DomainSuffix(suffixes) => suffixes
                .iter()
                .any(|s| s.matches(origin.host()))
                .then_some(Specificity::DomainSuffix),
            Self::TaskOpen { .. } => Some(Specificity::TaskOpen),
        }
    }

    /// The tightness tier this scope can grant at best, regardless of origin.
    ///
    /// Used for stratified reporting (ADR-004: containment reported by scope
    /// tightness), where the scope's tier matters even for origins it does
    /// not admit.
    #[must_use]
    pub const fn tier(&self) -> Specificity {
        match self {
            Self::Exact(_) => Specificity::Exact,
            Self::DomainSuffix(_) => Specificity::DomainSuffix,
            Self::TaskOpen { .. } => Specificity::TaskOpen,
        }
    }
}

impl TryFrom<OriginScopeWire> for OriginScope {
    type Error = ScopeError;

    fn try_from(wire: OriginScopeWire) -> Result<Self, Self::Error> {
        match wire {
            OriginScopeWire::Exact(origins) => Self::exact(origins),
            OriginScopeWire::DomainSuffix(suffixes) => Self::domain_suffix(suffixes),
            OriginScopeWire::TaskOpen { rationale } => Self::task_open(rationale),
        }
    }
}

impl TryFrom<String> for DomainSuffix {
    type Error = ScopeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<DomainSuffix> for String {
    fn from(value: DomainSuffix) -> Self {
        value.0
    }
}

impl std::str::FromStr for DomainSuffix {
    type Err = ScopeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for DomainSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for DomainSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DomainSuffix({:?})", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(s: &str) -> Origin {
        Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn suffix(s: &str) -> DomainSuffix {
        DomainSuffix::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn specificity_orders_exact_above_suffix_above_task_open() {
        assert!(Specificity::Exact > Specificity::DomainSuffix);
        assert!(Specificity::DomainSuffix > Specificity::TaskOpen);
        assert_eq!(
            [
                Specificity::TaskOpen,
                Specificity::Exact,
                Specificity::DomainSuffix
            ]
            .into_iter()
            .max(),
            Some(Specificity::Exact),
            "max() is how the comparator picks the narrowest justification"
        );
    }

    #[test]
    fn domain_suffix_normalizes_authoring_spellings() {
        for input in [
            "example.com",
            "*.example.com",
            ".example.com",
            "Example.COM",
        ] {
            assert_eq!(suffix(input).as_str(), "example.com", "normalizing {input}");
        }
    }

    #[test]
    fn domain_suffix_rejects_unusable_values() {
        for bad in ["", "   ", "*.", "exa mple.com", "example..com", "ex/ample"] {
            assert!(
                matches!(
                    DomainSuffix::parse(bad),
                    Err(ScopeError::InvalidDomainSuffix { .. })
                ),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn domain_suffix_matches_on_label_boundaries_only() {
        let s = suffix("example.com");
        assert!(s.matches("example.com"), "the domain itself");
        assert!(s.matches("mail.example.com"), "a subdomain");
        assert!(s.matches("a.b.example.com"), "a nested subdomain");
        assert!(
            !s.matches("notexample.com"),
            "a lookalike host must not be admitted by a suffix scope"
        );
        assert!(!s.matches("example.com.evil.test"), "suffix, not infix");
        assert!(!s.matches("example.co"));
    }

    #[test]
    fn exact_scope_admits_only_the_listed_origins() {
        let scope = OriginScope::exact([origin("https://mail.example.com")]).expect("non-empty");
        assert_eq!(
            scope.admits(&origin("https://mail.example.com/inbox")),
            Some(Specificity::Exact),
            "path is not part of an origin"
        );
        assert_eq!(scope.admits(&origin("https://other.example.com")), None);
        assert_eq!(
            scope.admits(&origin("http://mail.example.com")),
            None,
            "scheme is part of the origin tuple"
        );
    }

    #[test]
    fn domain_suffix_scope_admits_subdomains_at_suffix_rank() {
        let scope = OriginScope::domain_suffix([suffix("example.com")]).expect("non-empty");
        assert_eq!(
            scope.admits(&origin("https://mail.example.com")),
            Some(Specificity::DomainSuffix)
        );
        assert_eq!(scope.admits(&origin("https://notexample.com")), None);
    }

    #[test]
    fn task_open_scope_admits_every_origin_at_the_weakest_rank() {
        let scope = OriginScope::task_open("open web search for the user's query").expect("valid");
        for o in [
            "https://example.com",
            "http://localhost:3000",
            "https://attacker.test",
        ] {
            assert_eq!(scope.admits(&origin(o)), Some(Specificity::TaskOpen));
        }
    }

    #[test]
    fn scope_constructors_reject_scopes_that_could_admit_nothing_or_nothing_justified() {
        assert_eq!(
            OriginScope::exact([]),
            Err(ScopeError::Empty { kind: "exact" })
        );
        assert_eq!(
            OriginScope::domain_suffix([]),
            Err(ScopeError::Empty {
                kind: "domain_suffix"
            })
        );
        assert_eq!(
            OriginScope::task_open("   "),
            Err(ScopeError::MissingRationale),
            "ADR-004 requires task-open to be justified in writing"
        );
    }

    #[test]
    fn tier_reports_the_scope_kind_independently_of_any_origin() {
        assert_eq!(
            OriginScope::exact([origin("https://a.test")])
                .expect("valid")
                .tier(),
            Specificity::Exact
        );
        assert_eq!(
            OriginScope::domain_suffix([suffix("a.test")])
                .expect("valid")
                .tier(),
            Specificity::DomainSuffix
        );
        assert_eq!(
            OriginScope::task_open("why").expect("valid").tier(),
            Specificity::TaskOpen
        );
    }

    // ── Monotonicity ────────────────────────────────────────────────────
    //
    // The property: widening a scope never turns an admitted origin into a
    // non-admitted one, and never *raises* its admission rank.
    //
    // Checked by exhaustive enumeration over a small closed universe rather
    // than by random sampling. Exhaustive enumeration is strictly stronger
    // here (it proves the property over the whole domain instead of
    // sampling it), it is deterministic by construction as R8 requires, and
    // it costs no new dependency — §7.3 asks for 40 lines of our own code
    // over a dependency tree, and a property-test framework is a large one
    // for a domain this small.

    /// Every origin the widening universe is checked against.
    fn universe_of_origins() -> Vec<Origin> {
        [
            "https://example.com",
            "http://example.com",
            "https://example.com:8443",
            "https://mail.example.com",
            "https://a.b.example.com",
            "https://notexample.com",
            "https://example.org",
            "http://localhost:3000",
        ]
        .into_iter()
        .map(origin)
        .collect()
    }

    /// Scopes paired with a strictly wider scope, each pair annotated with
    /// why the second is a widening of the first.
    fn widening_pairs() -> Vec<(&'static str, OriginScope, OriginScope)> {
        let ex = |o: &str| OriginScope::exact([origin(o)]).expect("non-empty");
        let ex2 = |a: &str, b: &str| OriginScope::exact([origin(a), origin(b)]).expect("non-empty");
        let ds = |s: &str| OriginScope::domain_suffix([suffix(s)]).expect("non-empty");
        let ds2 = |a: &str, b: &str| {
            OriginScope::domain_suffix([suffix(a), suffix(b)]).expect("non-empty")
        };
        let open = || OriginScope::task_open("widening target").expect("valid");

        vec![
            (
                "adding an origin to an exact scope",
                ex("https://example.com"),
                ex2("https://example.com", "https://mail.example.com"),
            ),
            (
                "adding a suffix to a domain-suffix scope",
                ds("example.com"),
                ds2("example.com", "example.org"),
            ),
            (
                "shortening a suffix to a parent domain",
                ds("mail.example.com"),
                ds("example.com"),
            ),
            (
                "generalizing an exact origin to its domain suffix",
                ex("https://mail.example.com"),
                ds("mail.example.com"),
            ),
            (
                "generalizing an exact origin to a parent domain suffix",
                ex("https://mail.example.com"),
                ds("example.com"),
            ),
            (
                "opening an exact scope to task-open",
                ex("https://example.com"),
                open(),
            ),
            (
                "opening a domain-suffix scope to task-open",
                ds("example.com"),
                open(),
            ),
            (
                "task-open is its own widest form",
                open(),
                OriginScope::task_open("a different rationale").expect("valid"),
            ),
        ]
    }

    #[test]
    fn prop_widening_a_scope_never_revokes_an_admission() {
        for (why, narrow, wide) in widening_pairs() {
            for o in universe_of_origins() {
                if narrow.admits(&o).is_some() {
                    assert!(
                        wide.admits(&o).is_some(),
                        "{why}: {o} was admitted by {narrow:?} but not by the wider {wide:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn prop_widening_a_scope_never_raises_the_admission_rank() {
        for (why, narrow, wide) in widening_pairs() {
            for o in universe_of_origins() {
                if let (Some(narrow_rank), Some(wide_rank)) = (narrow.admits(&o), wide.admits(&o)) {
                    assert!(
                        wide_rank <= narrow_rank,
                        "{why}: {o} ranked {wide_rank:?} under the wider {wide:?} but only \
                         {narrow_rank:?} under {narrow:?} — a wider scope must never be a \
                         tighter justification"
                    );
                }
            }
        }
    }

    #[test]
    fn prop_admission_is_deterministic() {
        for (_, narrow, wide) in widening_pairs() {
            for o in universe_of_origins() {
                for scope in [&narrow, &wide] {
                    assert_eq!(scope.admits(&o), scope.admits(&o));
                }
            }
        }
    }
}
