//! `BrowserEngine`: the full agentic browser action surface
//! (`docs/REBUILD_DIRECTIVE.md` §6/A9, T-109), and this crate's own
//! [`MockEngine`] — deterministic, no feature flag, always built, backing
//! every test in the workspace that needs a `BrowserEngine`.
//!
//! The real embedding, `ServoEngine` (wrapping
//! `ferrite_servo::session::HeadlessServoSession`), lives in the sibling
//! `ferrite-engine-servo` crate behind the `engine-servo` Cargo feature —
//! kept out of this crate entirely so the default build, and every test that
//! does not need a live Servo, never pulls in `libservo`'s dependency tree
//! (directive §7.1: "not building Servo for 95% of the work" is the single
//! biggest build/disk lever).
//!
//! # Origin tracking is unavoidable, not optional
//!
//! Every action method returns `Result<(T, Origin), EngineError>` rather
//! than `T` alone. The origin the action executed against travels with the
//! result, so a caller (the dry-run recorder, a future live executor) can
//! never forget to separately query it after the fact — this is the
//! directive's own stated reason for the shape, not a convenience.
//!
//! Actions are also tagged, internally, with the
//! [`ferrite_core::Primitive`] they realize (see [`Call::primitive`]) —
//! reusing A2's closed wire vocabulary rather than inventing a second,
//! independent tool-id string the way the pre-rebuild `BrowserTool::tool_id`
//! and `Primitive::as_str()` once drifted apart on exactly one case
//! (T-216). There is deliberately no second vocabulary here to drift.
//!
//! # Opaque origins are a real, first-class error, not a panic
//!
//! `ferrite_core::Origin` only represents `http`/`https` origins (ADR-004,
//! ratified at T-211): every other scheme is opaque and, by construction, no
//! [`OriginScope`](ferrite_core::OriginScope) can ever admit it. A page whose
//! current URL has an opaque scheme (`about:blank`, a `data:` URL, a `blob:`
//! URL) genuinely has no `Origin` to report. Rather than fabricate one, every
//! `BrowserEngine` action attempted while the active page has no
//! representable origin fails with [`EngineError::OpaqueOrigin`] — a real,
//! observable outcome a caller can route to consent the same way T-211's
//! comparator already treats an unparseable recorded origin.
//!
//! [`MockEngine`] never actually hits this in practice for a caller who only
//! navigates to `http`/`https` URLs (its very first tab starts at a
//! synthetic-but-representable `https://mock-home.ferrite.test` placeholder,
//! specifically so a fresh engine still has *something* real to report —
//! unlike a real browser's `about:blank`). A test can still exercise the
//! opaque-origin path deliberately by navigating `MockEngine` to a `data:`
//! or other non-http(s) URL. `ServoEngine`, by contrast, genuinely starts
//! on `about:blank` and hits this path for real before the first navigation.
//!
//! # `js_execute` is privileged
//!
//! `js_execute` exists on this trait because *something* has to run the
//! model's arbitrary-script requests, but its existence here is not a safety
//! claim. `ferrite-ipi`'s comparator treats every `js.execute` primitive as
//! an unconditional deviation, at any scope, regardless of origin (ADR-003)
//! — that gating happens upstream of whatever calls this trait. A caller
//! that invokes [`BrowserEngine::js_execute`] directly, with no
//! comparator/consent step in front of it, has bypassed that job, not this
//! trait's: this method is not the safety net, it is the thing the safety
//! net watches.

#![deny(missing_docs)]

use ferrite_core::{Origin, Primitive};

mod mock;

pub mod digest;

pub use digest::{
    normalize_selector, parse_ref, ref_selector, sanitize_text, truncate_chars, DigestElement,
    PageDigest, RenderBudget, ScrollState, REF_ATTRIBUTE,
};
pub use mock::{MockEngine, MOCK_HOME};

#[cfg(any(test, feature = "test-util"))]
pub mod conformance;

/// Everything that can go wrong executing a [`BrowserEngine`] action.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// No tab exists with this id (never opened, or already closed).
    #[error("no such tab: {0}")]
    NoSuchTab(TabId),
    /// The requested selector resolved to no element.
    #[error("element not found: {0}")]
    ElementNotFound(String),
    /// A [`WaitCondition`] was not satisfied within its own timeout.
    #[error("wait timed out")]
    WaitTimedOut,
    /// The given URL could not be parsed or navigated to.
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    /// The active page's URL has no representable [`Origin`] (an opaque
    /// scheme — see the [module docs](self)). This is a deliberate, typed
    /// outcome, not a gap: fabricating an `Origin` for `about:blank` would
    /// misrepresent what the scope algebra can admit.
    #[error("current page has no representable origin: {0}")]
    OpaqueOrigin(String),
    /// This engine implementation does not (yet) support the requested
    /// action. Distinct from a transient failure: retrying will not help.
    #[error("unsupported by this engine: {0}")]
    Unsupported(&'static str),
    /// An underlying engine failure not covered by a more specific variant.
    #[error("engine error: {0}")]
    Internal(String),
}

/// Opaque handle to a browser tab.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct TabId(pub u64);

impl std::fmt::Display for TabId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tab-{}", self.0)
    }
}

/// Axis-aligned layout box, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bounds {
    /// Distance from the viewport's left edge.
    pub x: f64,
    /// Distance from the viewport's top edge.
    pub y: f64,
    /// Box width.
    pub width: f64,
    /// Box height.
    pub height: f64,
}

/// One node of a [`DomSnapshot`] — accessibility-tree-shaped, not a raw HTML
/// dump: the shape an agent actually reasons over (role, label, text,
/// layout), not markup it would have to re-parse.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct DomNode {
    /// ARIA-style role, e.g. `"button"`, `"textbox"`, `"link"`, `"generic"`.
    pub role: String,
    /// Accessible label/name, if any (`aria-label`, an associated `<label>`,
    /// `alt` text, ...).
    pub label: Option<String>,
    /// This node's own text content (not its descendants').
    pub text: Option<String>,
    /// A selector a later `query`/`click`/`type_text` call can be issued
    /// with to address this exact element, if the source page exposes one.
    pub selector: Option<String>,
    /// Layout bounds, if known.
    pub bounds: Option<Bounds>,
    /// Child nodes, in document order.
    pub children: Vec<DomNode>,
}

/// An accessibility-tree-style snapshot of a page — [`BrowserEngine::dom_snapshot`]'s result.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct DomSnapshot {
    /// The document's root node.
    pub root: DomNode,
}

/// One resolved element, as returned by [`BrowserEngine::query`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ElementHandle {
    /// A selector this handle can be re-issued as into `click`/`type_text`/...
    pub selector: String,
    /// ARIA-style role, if known.
    pub role: Option<String>,
    /// Visible text, if any.
    pub text: Option<String>,
}

/// One cookie, as returned by [`BrowserEngine::cookies_read`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
}

/// A captured viewport frame: `(width, height, RGBA bytes)` — the shape
/// `HeadlessServoSession::get_frame` already returns.
pub type Frame = (u32, u32, Vec<u8>);

/// A `wait_for` condition (directive §6/A9: "selector/idle/timeout").
#[derive(Debug, Clone, PartialEq)]
pub enum WaitCondition {
    /// Wait until `selector` resolves to at least one element.
    Selector(String),
    /// Wait until the engine reports no in-flight navigation/load activity.
    Idle,
    /// Wait for a fixed duration regardless of page state.
    Timeout(std::time::Duration),
}

/// One action taken against a [`BrowserEngine`], for introspection
/// ([`MockEngine::calls`]) and for repeated-action loop detection
/// (`ferrite_agent::browser_loop`).
///
/// This is *not* a second tool-id vocabulary: [`Call::primitive`] maps every
/// variant onto the existing [`ferrite_core::Primitive`] wire string rather
/// than inventing its own (see the [module docs](self)).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Call {
    /// [`BrowserEngine::navigate`].
    Navigate(String),
    /// [`BrowserEngine::go_back`].
    GoBack,
    /// [`BrowserEngine::go_forward`].
    GoForward,
    /// [`BrowserEngine::reload`].
    Reload,
    /// [`BrowserEngine::current_url`].
    CurrentUrl,
    /// [`BrowserEngine::dom_snapshot`].
    DomSnapshot,
    /// [`BrowserEngine::query`].
    Query(String),
    /// [`BrowserEngine::read_text`].
    ReadText(String),
    /// [`BrowserEngine::click`].
    Click(String),
    /// [`BrowserEngine::type_text`].
    TypeText(String, String),
    /// [`BrowserEngine::fill_form`].
    FillForm(Vec<(String, String)>),
    /// [`BrowserEngine::select_option`].
    SelectOption(String, String),
    /// [`BrowserEngine::scroll`].
    Scroll(i64, i64),
    /// [`BrowserEngine::wait_for`].
    WaitFor(WaitConditionKind),
    /// [`BrowserEngine::screenshot`].
    Screenshot,
    /// [`BrowserEngine::download`].
    Download(String),
    /// [`BrowserEngine::open_tab`].
    OpenTab(Option<String>),
    /// [`BrowserEngine::close_tab`].
    CloseTab(TabId),
    /// [`BrowserEngine::switch_tab`].
    SwitchTab(TabId),
    /// [`BrowserEngine::cookies_read`].
    CookiesRead(String),
    /// [`BrowserEngine::storage_read`].
    StorageRead(String),
    /// [`BrowserEngine::clipboard_read`].
    ClipboardRead,
    /// [`BrowserEngine::clipboard_write`].
    ClipboardWrite(String),
    /// [`BrowserEngine::js_execute`].
    JsExecute(String),
}

/// Serializable, `Eq`-friendly stand-in for [`WaitCondition`] (whose
/// `Duration` payload is otherwise faithfully mirrored as a millisecond
/// count, so a call-log assertion can compare it structurally).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WaitConditionKind {
    /// Mirrors [`WaitCondition::Selector`].
    Selector(String),
    /// Mirrors [`WaitCondition::Idle`].
    Idle,
    /// Mirrors [`WaitCondition::Timeout`], recording the millisecond count.
    TimeoutMillis(u128),
}

impl From<&WaitCondition> for WaitConditionKind {
    fn from(c: &WaitCondition) -> Self {
        match c {
            WaitCondition::Selector(s) => Self::Selector(s.clone()),
            WaitCondition::Idle => Self::Idle,
            WaitCondition::Timeout(d) => Self::TimeoutMillis(d.as_millis()),
        }
    }
}

impl Call {
    /// The [`ferrite_core::Primitive`] this call realizes.
    #[must_use]
    pub fn primitive(&self) -> Primitive {
        match self {
            Call::Navigate(_) | Call::GoBack | Call::GoForward | Call::Reload => {
                Primitive::Navigate
            }
            Call::CurrentUrl | Call::DomSnapshot => Primitive::DomRead,
            Call::Query(_) => Primitive::DomQuery,
            Call::ReadText(_) => Primitive::DomRead,
            Call::Click(_) => Primitive::Click,
            Call::TypeText(..) | Call::SelectOption(..) => Primitive::DomWrite,
            Call::FillForm(_) => Primitive::FormFill,
            Call::Scroll(..) => Primitive::Scroll,
            Call::WaitFor(_) => Primitive::Wait,
            Call::Screenshot => Primitive::Screenshot,
            Call::Download(_) => Primitive::Download,
            Call::OpenTab(_) => Primitive::TabOpen,
            Call::CloseTab(_) => Primitive::TabClose,
            // Switching the active tab changes which origin subsequent
            // actions act on — modelled as a navigation-class event, same
            // as go_back/go_forward/reload above.
            Call::SwitchTab(_) => Primitive::Navigate,
            Call::CookiesRead(_) => Primitive::CookieRead,
            Call::StorageRead(_) => Primitive::StorageRead,
            Call::ClipboardRead => Primitive::ClipboardRead,
            Call::ClipboardWrite(_) => Primitive::ClipboardWrite,
            Call::JsExecute(_) => Primitive::JsExecute,
        }
    }
}

/// The full agentic browser action surface (directive §6/A9, minimum list).
///
/// Every method reports `(result, origin_at_time_of_action)` — see the
/// [module docs](self). `&mut self` throughout: every action can change
/// engine state (navigation, tab focus, DOM, clipboard), so there is no
/// meaningfully read-only subset worth a separate `&self` split, and a
/// single mutable-borrow shape is what lets a caller hold `&mut dyn
/// BrowserEngine` uniformly.
///
/// **Deliberately no `Send` (or `Sync`) supertrait bound.** An earlier draft
/// of this trait required `Send`, which `MockEngine` trivially satisfies —
/// but the real `ServoEngine` (`ferrite-engine-servo`, `--features
/// engine-servo`) cannot: `HeadlessServoSession` holds `Rc<...>` state
/// throughout (Servo's own `WebView`/`Servo` handles, plus this crate's
/// shared delegate cells), because Servo's engine is a thread-local
/// singleton (`ferrite_servo::session`'s `get_or_init_servo`) that is not
/// meant to be moved across threads at all. This was found the hard way:
/// building `ferrite-engine-servo` against the real `servo` feature failed
/// with 24 "cannot be sent between threads safely" errors before this bound
/// was removed (`docs/handoffs/a09.md`). A caller that genuinely needs to
/// move a `BrowserEngine` across threads must own that requirement itself
/// (e.g. by not using `ServoEngine` off its creating thread) — this trait
/// does not manufacture a false promise that every implementation can.
pub trait BrowserEngine {
    /// Navigate the active tab to `url`.
    fn navigate(&mut self, url: &str) -> Result<((), Origin), EngineError>;
    /// Go back one entry in the active tab's history.
    fn go_back(&mut self) -> Result<((), Origin), EngineError>;
    /// Go forward one entry in the active tab's history.
    fn go_forward(&mut self) -> Result<((), Origin), EngineError>;
    /// Reload the active tab.
    fn reload(&mut self) -> Result<((), Origin), EngineError>;
    /// The active tab's current URL.
    fn current_url(&mut self) -> Result<(String, Origin), EngineError>;
    /// An accessibility-tree-style snapshot of the active tab's page.
    fn dom_snapshot(&mut self) -> Result<(DomSnapshot, Origin), EngineError>;
    /// Resolve `selector` to zero or more element handles.
    fn query(&mut self, selector: &str) -> Result<(Vec<ElementHandle>, Origin), EngineError>;
    /// The text content of the element `selector` resolves to.
    fn read_text(&mut self, selector: &str) -> Result<(String, Origin), EngineError>;
    /// Click the element `selector` resolves to.
    fn click(&mut self, selector: &str) -> Result<((), Origin), EngineError>;
    /// Type `text` into the element `selector` resolves to.
    fn type_text(&mut self, selector: &str, text: &str) -> Result<((), Origin), EngineError>;
    /// Fill each `(selector, value)` pair as a form field.
    fn fill_form(&mut self, fields: &[(String, String)]) -> Result<((), Origin), EngineError>;
    /// Select `value` in the `<select>`-shaped element `selector` resolves to.
    fn select_option(&mut self, selector: &str, value: &str) -> Result<((), Origin), EngineError>;
    /// Scroll the viewport by `(dx, dy)` CSS pixels.
    fn scroll(&mut self, dx: i64, dy: i64) -> Result<((), Origin), EngineError>;
    /// Block until `condition` is satisfied, or fail with
    /// [`EngineError::WaitTimedOut`].
    fn wait_for(&mut self, condition: WaitCondition) -> Result<((), Origin), EngineError>;
    /// Capture the active tab's rendered viewport. Encoding to a file
    /// format is a caller concern, not this trait's.
    fn screenshot(&mut self) -> Result<(Frame, Origin), EngineError>;
    /// Download the resource at `url`. Returns the local path/handle the
    /// engine saved it to.
    fn download(&mut self, url: &str) -> Result<(String, Origin), EngineError>;
    /// Open a new tab, optionally navigating it to `url`, and make it the
    /// active tab.
    fn open_tab(&mut self, url: Option<&str>) -> Result<(TabId, Origin), EngineError>;
    /// Close `tab`.
    fn close_tab(&mut self, tab: TabId) -> Result<((), Origin), EngineError>;
    /// Make `tab` the active tab.
    fn switch_tab(&mut self, tab: TabId) -> Result<((), Origin), EngineError>;
    /// Cookies visible to `scope` — **not** every cookie the engine holds
    /// across every origin (see the scoping conformance test).
    fn cookies_read(&mut self, scope: &Origin) -> Result<(Vec<Cookie>, Origin), EngineError>;
    /// Local/session storage key-value pairs visible to `scope` — **not**
    /// every origin's storage.
    fn storage_read(
        &mut self,
        scope: &Origin,
    ) -> Result<(Vec<(String, String)>, Origin), EngineError>;
    /// Read the system clipboard.
    fn clipboard_read(&mut self) -> Result<(String, Origin), EngineError>;
    /// Write `text` to the system clipboard.
    fn clipboard_write(&mut self, text: &str) -> Result<((), Origin), EngineError>;
    /// Run arbitrary JavaScript in the active tab.
    ///
    /// **Privileged — see the [module docs](self).** This method existing
    /// does not mean calling it is safe; the comparator upstream is what
    /// makes it safe, and this trait does not enforce that on its own.
    fn js_execute(&mut self, script: &str) -> Result<(String, Origin), EngineError>;
}
