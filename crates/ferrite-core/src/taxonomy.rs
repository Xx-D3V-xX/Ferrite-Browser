//! The closed capability/primitive vocabulary and the lowering between them.
//!
//! Per ADR-001 a capability is an **action class** (a grouping of primitives
//! sharing a security character) paired with an **origin scope** (where it is
//! authorized to act). It is deliberately not a fake domain tool: "email" is a
//! property of the origin scope, authored per task, never something the model
//! can invent. The vocabulary is closed — seven capabilities, seventeen
//! primitives — so a label outside it is unrepresentable rather than merely
//! rejected.
//!
//! # `js.execute` is structurally absent from every expected realization
//!
//! `js.execute` can synthesize any other primitive past the executor
//! boundary, so admitting it at any scope would void every other scope check
//! (ADR-003). Pre-rebuild this was `const UNSCOPABLE: &[&str] =
//! &["js.execute"]` consulted at runtime in the comparator — a check someone
//! could forget to make.
//!
//! Here it is a property of the type system, in three layers:
//!
//! 1. [`ActionClass::Execute`] is the one action class that is not scopable,
//!    and [`Primitive::JsExecute`] is the only primitive in it. The rule is
//!    general — *any primitive whose action class is unscopable is
//!    unconditionally a deviation* — not a string special-case.
//! 2. [`ScopablePrimitive`] is a separate enum with no `JsExecute` variant.
//!    Every *expected* realization is made of these, so an expected set
//!    containing `js.execute` is unrepresentable.
//! 3. [`Primitive`] — what the executor actually *records* — does have the
//!    variant, because the recorder must be able to report that it happened.
//!    [`Primitive::as_scopable`] is the only bridge back, and it returns
//!    [`None`] for `js.execute`.
//!
//! So the comparator cannot accidentally admit it: there is no value of the
//! expected-side type that equals it.

use serde::{Deserialize, Serialize};

use crate::error::TaxonomyError;
use crate::ids::Origin;
use crate::scope::{OriginScope, Specificity};

/// Declares a closed enum together with the complete list of its variants.
///
/// `ALL` is generated from the same invocation that declares the variants, so
/// a variant cannot exist without appearing in `ALL` — the list cannot drift
/// out of date the way a hand-maintained one can.
macro_rules! closed_enum {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $( $(#[$variant_meta:meta])* $variant:ident => $wire:literal ),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        pub enum $name {
            $(
                $(#[$variant_meta])*
                #[serde(rename = $wire)]
                $variant,
            )+
        }

        impl $name {
            /// Every variant of this enum, in declaration order.
            ///
            /// Generated alongside the variant list itself, so it is complete
            /// by construction rather than by discipline.
            pub const ALL: &'static [Self] = &[ $( Self::$variant, )+ ];

            /// This variant's stable wire name — the string used in serde
            /// output, authored corpus JSON, and audit entries.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $wire, )+ }
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

closed_enum! {
    /// The security character shared by a group of primitives (ADR-001).
    ///
    /// Scopability is a property of the class, not of any particular
    /// primitive name — see [`ActionClass::is_scopable`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub enum ActionClass {
        /// Observing page or browser state without changing it.
        Read => "read",
        /// Changing page state, or driving the page as a user would.
        Interact => "interact",
        /// Changing *which* origin the agent is acting on.
        Navigate => "navigate",
        /// Pulling bytes out of the browser onto the host.
        Download => "download",
        /// Crossing the browser/OS boundary via the system clipboard.
        Clipboard => "clipboard",
        /// Running arbitrary code in the page. Never scopable — see the
        /// [module docs](self).
        Execute => "execute",
    }
}

closed_enum! {
    /// A capability a *task* may legitimately need — the closed allowlist the
    /// model's predictions are filtered against (ADR-001, directive §8).
    ///
    /// A capability on its own says only *what class of action*; paired with
    /// an [`OriginScope`] in an [`ExpectedCapability`] it says *where*.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub enum Capability {
        /// Read content from admitted origins.
        WebRead => "web.read",
        /// Move between admitted origins, including opening and closing tabs.
        WebNavigate => "web.navigate",
        /// Drive the page as a user would at admitted origins.
        WebInteract => "web.interact",
        /// Download a file from an admitted origin.
        WebDownload => "web.download",
        /// Read per-origin stored state (cookies, web storage) — a read, but
        /// of credential-adjacent data, so it is its own capability rather
        /// than part of `web.read`.
        ScopedRead => "scoped.read",
        /// Read the system clipboard.
        ClipboardRead => "clipboard.read",
        /// Write the system clipboard.
        ClipboardWrite => "clipboard.write",
    }
}

closed_enum! {
    /// A primitive that can appear in a capability's expected realization.
    ///
    /// This enum has **no `js.execute` variant**, and that absence is the
    /// crate's central invariant (see the [module docs](self)): every
    /// expected-side type is built from `ScopablePrimitive`, so no expected
    /// capability set can name `js.execute` at any scope.
    ///
    /// The variant genuinely does not exist:
    ///
    /// ```compile_fail
    /// let _ = ferrite_core::ScopablePrimitive::JsExecute;
    /// ```
    ///
    /// while the same path with a real variant compiles, which is what shows
    /// the failure above is the missing variant and not a mistyped path:
    ///
    /// ```
    /// let _ = ferrite_core::ScopablePrimitive::DomRead;
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub enum ScopablePrimitive {
        /// Load a URL in the current tab.
        Navigate => "navigate",
        /// Read the accessibility-tree snapshot of the current page.
        DomRead => "dom.read",
        /// Resolve a selector to element handles.
        DomQuery => "dom.query",
        /// Mutate page content or type into an element.
        DomWrite => "dom.write",
        /// Fill a form's fields.
        FormFill => "form.fill",
        /// Click an element.
        Click => "click",
        /// Scroll the viewport.
        Scroll => "scroll",
        /// Wait for a selector, for idle, or for a timeout.
        Wait => "wait",
        /// Download a file.
        Download => "download",
        /// Open a new tab.
        TabOpen => "tab.open",
        /// Close a tab.
        TabClose => "tab.close",
        /// Read cookies for an origin.
        CookieRead => "cookie.read",
        /// Read local/session storage for an origin.
        StorageRead => "storage.read",
        /// Read the system clipboard.
        ClipboardRead => "clipboard.read",
        /// Write the system clipboard.
        ClipboardWrite => "clipboard.write",
        /// Capture a screenshot.
        Screenshot => "screenshot",
    }
}

closed_enum! {
    /// A primitive the executor actually performs, as recorded in a dry-run
    /// event log.
    ///
    /// This is the *observed* vocabulary and therefore includes
    /// [`Primitive::JsExecute`] — the recorder has to be able to report that
    /// it happened. It is the expected side ([`ScopablePrimitive`]) that
    /// cannot name it. Convert observed → expected with
    /// [`Primitive::as_scopable`], which returns [`None`] for exactly that
    /// one variant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub enum Primitive {
        /// Load a URL in the current tab.
        Navigate => "navigate",
        /// Read the accessibility-tree snapshot of the current page.
        DomRead => "dom.read",
        /// Resolve a selector to element handles.
        DomQuery => "dom.query",
        /// Mutate page content or type into an element.
        DomWrite => "dom.write",
        /// Fill a form's fields.
        FormFill => "form.fill",
        /// Click an element.
        Click => "click",
        /// Scroll the viewport.
        Scroll => "scroll",
        /// Wait for a selector, for idle, or for a timeout.
        Wait => "wait",
        /// Download a file.
        Download => "download",
        /// Open a new tab.
        TabOpen => "tab.open",
        /// Close a tab.
        TabClose => "tab.close",
        /// Read cookies for an origin.
        CookieRead => "cookie.read",
        /// Read local/session storage for an origin.
        StorageRead => "storage.read",
        /// Read the system clipboard.
        ClipboardRead => "clipboard.read",
        /// Write the system clipboard.
        ClipboardWrite => "clipboard.write",
        /// Capture a screenshot.
        Screenshot => "screenshot",
        /// Run arbitrary JavaScript in the page. Unconditionally unscopable
        /// (ADR-003): always a deviation, always consent-gated.
        JsExecute => "js.execute",
    }
}

// ── The lowering table ──────────────────────────────────────────────────

const WEB_READ: &[ScopablePrimitive] = &[
    ScopablePrimitive::DomRead,
    ScopablePrimitive::DomQuery,
    ScopablePrimitive::Screenshot,
];

const WEB_NAVIGATE: &[ScopablePrimitive] = &[
    ScopablePrimitive::Navigate,
    ScopablePrimitive::TabOpen,
    ScopablePrimitive::TabClose,
];

const WEB_INTERACT: &[ScopablePrimitive] = &[
    ScopablePrimitive::DomWrite,
    ScopablePrimitive::FormFill,
    ScopablePrimitive::Click,
    ScopablePrimitive::Scroll,
    ScopablePrimitive::Wait,
];

const WEB_DOWNLOAD: &[ScopablePrimitive] = &[ScopablePrimitive::Download];

const SCOPED_READ: &[ScopablePrimitive] = &[
    ScopablePrimitive::CookieRead,
    ScopablePrimitive::StorageRead,
];

const CLIPBOARD_READ: &[ScopablePrimitive] = &[ScopablePrimitive::ClipboardRead];

const CLIPBOARD_WRITE: &[ScopablePrimitive] = &[ScopablePrimitive::ClipboardWrite];

/// The capability → primitive lowering, as **data** (directive §8).
///
/// This table is the single source of truth: [`Capability::realization`] is a
/// lookup into it, not a second copy of it. Two independent gates keep it
/// honest, so a primitive cannot be added without being assigned:
///
/// - Adding a [`ScopablePrimitive`] variant makes
///   [`ScopablePrimitive::capability`]'s exhaustive `match` non-exhaustive,
///   which **fails to compile**.
/// - Adding a [`Capability`] variant makes `Capability::index`'s exhaustive
///   `match` non-exhaustive, which also fails to compile; forgetting to then
///   add its row here fails
///   `lowering_table_covers_every_scopable_primitive_exactly_once`.
pub const LOWERING: [(Capability, &[ScopablePrimitive]); 7] = [
    (Capability::WebRead, WEB_READ),
    (Capability::WebNavigate, WEB_NAVIGATE),
    (Capability::WebInteract, WEB_INTERACT),
    (Capability::WebDownload, WEB_DOWNLOAD),
    (Capability::ScopedRead, SCOPED_READ),
    (Capability::ClipboardRead, CLIPBOARD_READ),
    (Capability::ClipboardWrite, CLIPBOARD_WRITE),
];

impl ActionClass {
    /// Whether a primitive of this class can ever be admitted by a scope.
    ///
    /// False for [`ActionClass::Execute`] alone. This is the general form of
    /// ADR-003's rule — the property lives on the action class, so the
    /// comparator never has to compare a primitive against a name.
    #[must_use]
    pub const fn is_scopable(self) -> bool {
        match self {
            Self::Read | Self::Interact | Self::Navigate | Self::Download | Self::Clipboard => true,
            Self::Execute => false,
        }
    }
}

impl Capability {
    /// Position of this capability in [`LOWERING`]; the exhaustive `match` is
    /// what forces a new capability to be given a row.
    const fn index(self) -> usize {
        match self {
            Self::WebRead => 0,
            Self::WebNavigate => 1,
            Self::WebInteract => 2,
            Self::WebDownload => 3,
            Self::ScopedRead => 4,
            Self::ClipboardRead => 5,
            Self::ClipboardWrite => 6,
        }
    }

    /// The primitives this capability authorizes — its realization.
    ///
    /// Every element is a [`ScopablePrimitive`], so this can never contain
    /// `js.execute`.
    ///
    /// # Panics
    ///
    /// If [`LOWERING`] has no row for this capability, which
    /// `lowering_table_covers_every_capability` catches in the test suite.
    #[must_use]
    pub const fn realization(self) -> &'static [ScopablePrimitive] {
        let i = self.index();
        assert!(i < LOWERING.len(), "LOWERING is missing a capability row");
        LOWERING[i].1
    }

    /// The action class this capability belongs to (ADR-001).
    #[must_use]
    pub const fn action_class(self) -> ActionClass {
        match self {
            Self::WebRead | Self::ScopedRead => ActionClass::Read,
            Self::WebNavigate => ActionClass::Navigate,
            Self::WebInteract => ActionClass::Interact,
            Self::WebDownload => ActionClass::Download,
            Self::ClipboardRead | Self::ClipboardWrite => ActionClass::Clipboard,
        }
    }
}

impl ScopablePrimitive {
    /// The one capability whose realization contains this primitive.
    ///
    /// This exhaustive `match` is the compile-time gate directive §8 asks
    /// for: adding a primitive without assigning it to a capability does not
    /// compile.
    #[must_use]
    pub const fn capability(self) -> Capability {
        match self {
            Self::DomRead | Self::DomQuery | Self::Screenshot => Capability::WebRead,
            Self::Navigate | Self::TabOpen | Self::TabClose => Capability::WebNavigate,
            Self::DomWrite | Self::FormFill | Self::Click | Self::Scroll | Self::Wait => {
                Capability::WebInteract
            }
            Self::Download => Capability::WebDownload,
            Self::CookieRead | Self::StorageRead => Capability::ScopedRead,
            Self::ClipboardRead => Capability::ClipboardRead,
            Self::ClipboardWrite => Capability::ClipboardWrite,
        }
    }

    /// This primitive as an observed [`Primitive`].
    #[must_use]
    pub const fn as_primitive(self) -> Primitive {
        match self {
            Self::Navigate => Primitive::Navigate,
            Self::DomRead => Primitive::DomRead,
            Self::DomQuery => Primitive::DomQuery,
            Self::DomWrite => Primitive::DomWrite,
            Self::FormFill => Primitive::FormFill,
            Self::Click => Primitive::Click,
            Self::Scroll => Primitive::Scroll,
            Self::Wait => Primitive::Wait,
            Self::Download => Primitive::Download,
            Self::TabOpen => Primitive::TabOpen,
            Self::TabClose => Primitive::TabClose,
            Self::CookieRead => Primitive::CookieRead,
            Self::StorageRead => Primitive::StorageRead,
            Self::ClipboardRead => Primitive::ClipboardRead,
            Self::ClipboardWrite => Primitive::ClipboardWrite,
            Self::Screenshot => Primitive::Screenshot,
        }
    }

    /// The action class of this primitive — always a scopable one.
    #[must_use]
    pub const fn action_class(self) -> ActionClass {
        self.as_primitive().action_class()
    }
}

impl From<ScopablePrimitive> for Primitive {
    fn from(value: ScopablePrimitive) -> Self {
        value.as_primitive()
    }
}

impl Primitive {
    /// The action class of this primitive (ADR-001).
    #[must_use]
    pub const fn action_class(self) -> ActionClass {
        match self {
            Self::DomRead
            | Self::DomQuery
            | Self::Screenshot
            | Self::CookieRead
            | Self::StorageRead => ActionClass::Read,
            Self::DomWrite | Self::FormFill | Self::Click | Self::Scroll | Self::Wait => {
                ActionClass::Interact
            }
            Self::Navigate | Self::TabOpen | Self::TabClose => ActionClass::Navigate,
            Self::Download => ActionClass::Download,
            Self::ClipboardRead | Self::ClipboardWrite => ActionClass::Clipboard,
            Self::JsExecute => ActionClass::Execute,
        }
    }

    /// This primitive as a [`ScopablePrimitive`], or [`None`] if no scope can
    /// ever admit it.
    ///
    /// This is the only bridge from the observed vocabulary to the expected
    /// one, and it is where `js.execute` is filtered out — a comparator that
    /// goes through this function cannot reach origin logic for it.
    #[must_use]
    pub const fn as_scopable(self) -> Option<ScopablePrimitive> {
        match self {
            Self::Navigate => Some(ScopablePrimitive::Navigate),
            Self::DomRead => Some(ScopablePrimitive::DomRead),
            Self::DomQuery => Some(ScopablePrimitive::DomQuery),
            Self::DomWrite => Some(ScopablePrimitive::DomWrite),
            Self::FormFill => Some(ScopablePrimitive::FormFill),
            Self::Click => Some(ScopablePrimitive::Click),
            Self::Scroll => Some(ScopablePrimitive::Scroll),
            Self::Wait => Some(ScopablePrimitive::Wait),
            Self::Download => Some(ScopablePrimitive::Download),
            Self::TabOpen => Some(ScopablePrimitive::TabOpen),
            Self::TabClose => Some(ScopablePrimitive::TabClose),
            Self::CookieRead => Some(ScopablePrimitive::CookieRead),
            Self::StorageRead => Some(ScopablePrimitive::StorageRead),
            Self::ClipboardRead => Some(ScopablePrimitive::ClipboardRead),
            Self::ClipboardWrite => Some(ScopablePrimitive::ClipboardWrite),
            Self::Screenshot => Some(ScopablePrimitive::Screenshot),
            Self::JsExecute => None,
        }
    }
}

// ── Expected capabilities ───────────────────────────────────────────────

/// One capability the task is expected to need, together with **its own**
/// origin scope.
///
/// Carrying the scope here rather than once per task is the fix for D1/T-001:
/// a fingerprint mixing a narrow `scoped.read` on `mail.example.com` with a
/// wide `web.read` for search results is expressible, which it was not under
/// the pre-rebuild single-global-scope signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedCapability {
    capability: Capability,
    scope: OriginScope,
}

/// The set of capabilities a task is expected to need — the expected side of
/// a fingerprint.
///
/// At most one entry per [`Capability`], so a capability name is an
/// unambiguous attribution target: when the comparator says "this action was
/// justified by `scoped.read`", there is exactly one scope that could have
/// justified it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "Vec<ExpectedCapability>", into = "Vec<ExpectedCapability>")]
pub struct ExpectedCapabilitySet(Vec<ExpectedCapability>);

impl ExpectedCapability {
    /// Pairs a capability with the scope it is authorized at.
    #[must_use]
    pub const fn new(capability: Capability, scope: OriginScope) -> Self {
        Self { capability, scope }
    }

    /// The capability.
    #[must_use]
    pub const fn capability(&self) -> Capability {
        self.capability
    }

    /// Where this capability is authorized to act.
    #[must_use]
    pub const fn scope(&self) -> &OriginScope {
        &self.scope
    }

    /// The primitives this capability authorizes — never `js.execute`.
    #[must_use]
    pub const fn realization(&self) -> &'static [ScopablePrimitive] {
        self.capability.realization()
    }

    /// Whether this capability justifies `primitive` at `origin`, and if so
    /// how tightly.
    ///
    /// The [`Specificity`] is the admission rank the comparator attributes by
    /// (T-002).
    #[must_use]
    pub fn admits(&self, primitive: ScopablePrimitive, origin: &Origin) -> Option<Specificity> {
        if !self.realization().contains(&primitive) {
            return None;
        }
        self.scope.admits(origin)
    }
}

impl ExpectedCapabilitySet {
    /// Builds a set, rejecting duplicate capabilities.
    ///
    /// Entries are stored sorted by capability so that lowering, and anything
    /// derived from it, is deterministic (R8).
    ///
    /// # Errors
    ///
    /// [`TaxonomyError::DuplicateCapability`] if a capability appears twice.
    pub fn new(
        entries: impl IntoIterator<Item = ExpectedCapability>,
    ) -> Result<Self, TaxonomyError> {
        let mut entries: Vec<ExpectedCapability> = entries.into_iter().collect();
        entries.sort_by_key(ExpectedCapability::capability);
        if let Some(pair) = entries
            .windows(2)
            .find(|pair| pair[0].capability == pair[1].capability)
        {
            return Err(TaxonomyError::DuplicateCapability {
                capability: pair[0].capability,
            });
        }
        Ok(Self(entries))
    }

    /// The empty set — the fail-to-empty fingerprint.
    ///
    /// An empty expected set admits nothing, so every observed event is a
    /// deviation and routes through consent. That is the safe degradation
    /// every provider failure is required to produce (directive §10.4).
    #[must_use]
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// The entries, sorted by capability.
    #[must_use]
    pub fn entries(&self) -> &[ExpectedCapability] {
        &self.0
    }

    /// How many capabilities the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the set is empty — see [`ExpectedCapabilitySet::empty`] for
    /// what that means.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Lowers the set to `(primitive, scope, capability)` triples — the shape
    /// the comparator consumes (directive §6/A7).
    ///
    /// Note what this is *not*: a flat union of primitives. Each primitive
    /// arrives paired with the scope of the capability that authorized it and
    /// with that capability's name, which is what lets the comparator
    /// attribute an event to a specific capability and lets the consent panel
    /// and audit entry say which one justified the action.
    ///
    /// The first element is a [`ScopablePrimitive`], so `js.execute` cannot
    /// appear in the output at any scope.
    ///
    /// Deterministically ordered by capability, then by the capability's
    /// realization order (R8).
    #[must_use]
    pub fn lowered(&self) -> Vec<(ScopablePrimitive, &OriginScope, Capability)> {
        self.0
            .iter()
            .flat_map(|entry| {
                entry
                    .realization()
                    .iter()
                    .map(move |primitive| (*primitive, entry.scope(), entry.capability()))
            })
            .collect()
    }
}

impl TryFrom<Vec<ExpectedCapability>> for ExpectedCapabilitySet {
    type Error = TaxonomyError;

    fn try_from(value: Vec<ExpectedCapability>) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExpectedCapabilitySet> for Vec<ExpectedCapability> {
    fn from(value: ExpectedCapabilitySet) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::DomainSuffix;

    fn origin(s: &str) -> Origin {
        Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn exact(s: &str) -> OriginScope {
        OriginScope::exact([origin(s)]).expect("non-empty")
    }

    fn suffix_scope(s: &str) -> OriginScope {
        OriginScope::domain_suffix([DomainSuffix::parse(s).expect("valid suffix")])
            .expect("non-empty")
    }

    // ── The closed vocabularies ─────────────────────────────────────────

    #[test]
    fn capability_vocabulary_is_the_closed_seven_of_adr_001() {
        let names: Vec<&str> = Capability::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "web.read",
                "web.navigate",
                "web.interact",
                "web.download",
                "scoped.read",
                "clipboard.read",
                "clipboard.write",
            ],
            "the allowlist is closed; changing it changes what the model may express"
        );
    }

    #[test]
    fn primitive_vocabulary_is_the_seventeen_of_directive_section_8() {
        let names: Vec<&str> = Primitive::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "navigate",
                "dom.read",
                "dom.query",
                "dom.write",
                "form.fill",
                "click",
                "scroll",
                "wait",
                "download",
                "tab.open",
                "tab.close",
                "cookie.read",
                "storage.read",
                "clipboard.read",
                "clipboard.write",
                "screenshot",
                "js.execute",
            ]
        );
    }

    #[test]
    fn scopable_vocabulary_is_the_observed_one_minus_js_execute() {
        let scopable: Vec<&str> = ScopablePrimitive::ALL.iter().map(|p| p.as_str()).collect();
        let observed: Vec<&str> = Primitive::ALL
            .iter()
            .filter(|p| **p != Primitive::JsExecute)
            .map(|p| p.as_str())
            .collect();
        assert_eq!(scopable, observed);
        assert_eq!(scopable.len(), 16);
        assert!(!scopable.contains(&"js.execute"));
    }

    // ── The lowering table ──────────────────────────────────────────────

    #[test]
    fn lowering_table_covers_every_scopable_primitive_exactly_once() {
        let mut seen: Vec<(ScopablePrimitive, Capability)> = Vec::new();
        for (capability, realization) in LOWERING {
            for primitive in realization {
                assert!(
                    !seen.iter().any(|(p, _)| p == primitive),
                    "{primitive} is assigned to more than one capability"
                );
                seen.push((*primitive, capability));
            }
        }
        for primitive in ScopablePrimitive::ALL {
            assert!(
                seen.iter().any(|(p, _)| p == primitive),
                "{primitive} is in no capability's realization — directive §8 requires \
                 every primitive be assigned to exactly one"
            );
        }
        assert_eq!(seen.len(), ScopablePrimitive::ALL.len());
    }

    #[test]
    fn lowering_table_covers_every_capability_exactly_once() {
        for capability in Capability::ALL {
            let rows = LOWERING.iter().filter(|(c, _)| c == capability).count();
            assert_eq!(rows, 1, "{capability} should have exactly one row");
        }
        assert_eq!(LOWERING.len(), Capability::ALL.len());
    }

    #[test]
    fn realization_lookup_agrees_with_the_lowering_table() {
        for (capability, realization) in LOWERING {
            assert_eq!(capability.realization(), realization, "{capability}");
            assert!(
                !capability.realization().is_empty(),
                "{capability} realizes nothing, so it could never be attributed to"
            );
        }
    }

    #[test]
    fn reverse_index_agrees_with_the_lowering_table() {
        for primitive in ScopablePrimitive::ALL {
            let capability = primitive.capability();
            assert!(
                capability.realization().contains(primitive),
                "{primitive} claims {capability} but is not in its realization"
            );
        }
    }

    #[test]
    fn a_capabilitys_realization_shares_its_action_class() {
        for capability in Capability::ALL {
            for primitive in capability.realization() {
                assert_eq!(
                    primitive.action_class(),
                    capability.action_class(),
                    "{primitive} is in {capability}'s realization but is a different action class \
                     — a capability is an action class paired with a scope (ADR-001)"
                );
            }
        }
    }

    // ── The js.execute invariant ────────────────────────────────────────

    #[test]
    fn js_execute_is_the_only_unscopable_primitive() {
        let unscopable: Vec<&Primitive> = Primitive::ALL
            .iter()
            .filter(|p| !p.action_class().is_scopable())
            .collect();
        assert_eq!(unscopable, [&Primitive::JsExecute]);
        assert_eq!(Primitive::JsExecute.action_class(), ActionClass::Execute);
    }

    #[test]
    fn a_primitive_is_convertible_exactly_when_its_action_class_is_scopable() {
        for primitive in Primitive::ALL {
            assert_eq!(
                primitive.as_scopable().is_some(),
                primitive.action_class().is_scopable(),
                "{primitive}: the type-level split and the action-class rule must agree, or \
                 ADR-003's general rule and its structural encoding could drift apart"
            );
        }
        assert_eq!(Primitive::JsExecute.as_scopable(), None);
    }

    #[test]
    fn no_capabilitys_expected_realization_can_contain_js_execute() {
        for capability in Capability::ALL {
            for primitive in capability.realization() {
                assert_ne!(
                    Primitive::from(*primitive),
                    Primitive::JsExecute,
                    "{capability} realizes js.execute — ADR-003 forbids it at any scope"
                );
            }
        }
    }

    #[test]
    fn an_expected_capability_set_can_never_lower_to_js_execute() {
        // Every capability, each at the widest possible scope, is the most
        // permissive expected set the type system allows anyone to build.
        let set = ExpectedCapabilitySet::new(Capability::ALL.iter().map(|c| {
            ExpectedCapability::new(
                *c,
                OriginScope::task_open("widest scope the taxonomy permits").expect("valid"),
            )
        }))
        .expect("no duplicates");

        for (primitive, _, _) in set.lowered() {
            assert_ne!(Primitive::from(primitive), Primitive::JsExecute);
        }
        assert!(
            set.lowered()
                .iter()
                .all(|(p, _, _)| p.action_class().is_scopable()),
            "even the widest expressible expected set cannot reach an unscopable action class"
        );
    }

    #[test]
    fn js_execute_is_admitted_by_nothing_because_it_cannot_be_asked_about() {
        // The comparator's only route from an observed primitive to the
        // expected side is `as_scopable`, and for js.execute it stops there —
        // origin logic is never reached, which is exactly ADR-003's
        // "first branch, before any origin check".
        let widest = ExpectedCapability::new(
            Capability::WebRead,
            OriginScope::task_open("widest").expect("valid"),
        );
        assert_eq!(Primitive::JsExecute.as_scopable(), None);
        // ...so there is no value to hand to `admits` at all, at any origin:
        assert!(
            Primitive::JsExecute
                .as_scopable()
                .map(|p| widest.admits(p, &origin("https://example.com")))
                .is_none()
        );
    }

    // ── ExpectedCapability / ExpectedCapabilitySet ──────────────────────

    #[test]
    fn expected_capability_admits_only_its_own_primitives_at_admitted_origins() {
        let cap =
            ExpectedCapability::new(Capability::ScopedRead, exact("https://mail.example.com"));
        let admitted = origin("https://mail.example.com");
        let elsewhere = origin("https://other.example.com");

        assert_eq!(
            cap.admits(ScopablePrimitive::CookieRead, &admitted),
            Some(Specificity::Exact)
        );
        assert_eq!(
            cap.admits(ScopablePrimitive::CookieRead, &elsewhere),
            None,
            "right primitive, wrong origin"
        );
        assert_eq!(
            cap.admits(ScopablePrimitive::Click, &admitted),
            None,
            "right origin, primitive outside this capability's realization"
        );
    }

    #[test]
    fn expected_capability_set_rejects_a_duplicate_capability() {
        let err = ExpectedCapabilitySet::new([
            ExpectedCapability::new(Capability::WebRead, exact("https://a.test")),
            ExpectedCapability::new(Capability::WebRead, suffix_scope("b.test")),
        ])
        .expect_err("two scopes for one capability make attribution ambiguous");
        assert_eq!(
            err,
            TaxonomyError::DuplicateCapability {
                capability: Capability::WebRead
            }
        );
    }

    #[test]
    fn expected_capability_set_lowers_to_per_capability_scopes() {
        // The fixture from directive §6/A7 that the pre-rebuild single-global
        // -scope signature could not express at all.
        let narrow = exact("https://mail.example.com");
        let wide = suffix_scope("search.test");
        let set = ExpectedCapabilitySet::new([
            ExpectedCapability::new(Capability::ScopedRead, narrow.clone()),
            ExpectedCapability::new(Capability::WebRead, wide.clone()),
        ])
        .expect("distinct capabilities");

        let lowered = set.lowered();
        for (primitive, scope, capability) in &lowered {
            assert_eq!(primitive.capability(), *capability);
            match capability {
                Capability::ScopedRead => assert_eq!(**scope, narrow),
                Capability::WebRead => assert_eq!(**scope, wide),
                other => panic!("unexpected capability {other}"),
            }
        }
        assert_eq!(
            lowered.len(),
            Capability::ScopedRead.realization().len() + Capability::WebRead.realization().len()
        );
        assert!(
            lowered
                .iter()
                .any(|(p, s, _)| *p == ScopablePrimitive::CookieRead && **s == narrow),
            "cookie.read must carry the narrow scope, not a task-wide union"
        );
        assert!(
            lowered
                .iter()
                .any(|(p, s, _)| *p == ScopablePrimitive::DomRead && **s == wide),
            "dom.read must carry the wide scope"
        );
    }

    #[test]
    fn expected_capability_set_is_deterministically_ordered() {
        let entries = [
            ExpectedCapability::new(Capability::ClipboardWrite, exact("https://a.test")),
            ExpectedCapability::new(Capability::WebRead, exact("https://b.test")),
            ExpectedCapability::new(Capability::WebInteract, exact("https://c.test")),
        ];
        let forwards = ExpectedCapabilitySet::new(entries.clone()).expect("distinct");
        let mut reversed = entries;
        reversed.reverse();
        let backwards = ExpectedCapabilitySet::new(reversed).expect("distinct");

        assert_eq!(
            forwards, backwards,
            "insertion order must not be observable"
        );
        assert_eq!(forwards.lowered(), backwards.lowered());
        assert_eq!(
            forwards
                .entries()
                .iter()
                .map(ExpectedCapability::capability)
                .collect::<Vec<_>>(),
            [
                Capability::WebRead,
                Capability::WebInteract,
                Capability::ClipboardWrite
            ],
            "sorted by the capability vocabulary's declaration order"
        );
    }

    #[test]
    fn the_empty_expected_set_lowers_to_nothing() {
        let empty = ExpectedCapabilitySet::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert!(
            empty.lowered().is_empty(),
            "fail-to-empty must admit nothing, so every event becomes a deviation"
        );
        assert_eq!(empty, ExpectedCapabilitySet::default());
    }
}
