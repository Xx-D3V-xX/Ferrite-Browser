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
    normalize_selector, not_found_message, parse_ref, ref_selector, sanitize_text, truncate_chars,
    DigestElement, LinkInfo, PageDigest, RenderBudget, ScrollState, TextMatches, REF_ATTRIBUTE,
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

/// One open tab, as returned by [`BrowserEngine::list_tabs`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TabInfo {
    /// The handle `switch_tab`/`close_tab` take.
    pub id: TabId,
    /// The tab's current URL.
    pub url: String,
    /// The tab's document title (empty if unknown).
    pub title: String,
    /// Whether this is the tab actions currently act on.
    pub active: bool,
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
    /// [`BrowserEngine::page_digest`] (the agent-initiated read; the
    /// harness's own [`BrowserEngine::observe_page`] is deliberately never
    /// a `Call` — see that method).
    PageDigest,
    /// [`BrowserEngine::press_key`]: `(selector, key)`.
    PressKey(Option<String>, String),
    /// [`BrowserEngine::hover`].
    Hover(String),
    /// [`BrowserEngine::set_checked`].
    SetChecked(String, bool),
    /// [`BrowserEngine::scroll_to`].
    ScrollTo(String),
    /// [`BrowserEngine::find_text`].
    FindText(String),
    /// [`BrowserEngine::extract_links`].
    ExtractLinks(Option<String>),
    /// [`BrowserEngine::submit_form`].
    SubmitForm(Option<String>),
    /// [`BrowserEngine::list_tabs`].
    ListTabs,
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
            // The new read-only surface: reading the page's content, links,
            // matches or the tab list is a page/browser *read*, whatever
            // shape the result takes.
            Call::CurrentUrl
            | Call::DomSnapshot
            | Call::PageDigest
            | Call::FindText(_)
            | Call::ExtractLinks(_)
            | Call::ListTabs => Primitive::DomRead,
            Call::Query(_) => Primitive::DomQuery,
            Call::ReadText(_) => Primitive::DomRead,
            // Conservative mapping: hover and set_checked drive the same
            // pointer path as a click (they can trigger handlers, toggle
            // state), and a form submission is the effect of clicking its
            // submit button — none may be admitted by a weaker primitive.
            Call::Click(_) | Call::Hover(_) | Call::SetChecked(..) | Call::SubmitForm(_) => {
                Primitive::Click
            }
            // A key press is input into whatever has focus.
            Call::TypeText(..) | Call::SelectOption(..) | Call::PressKey(..) => Primitive::DomWrite,
            Call::FillForm(_) => Primitive::FormFill,
            Call::Scroll(..) | Call::ScrollTo(_) => Primitive::Scroll,
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

    // ── Page understanding and richer interaction ──────────────────────
    //
    // Every method below has a default implementation, so an engine that
    // predates them (or a test double) keeps compiling. The defaults are
    // honest: derived from `dom_snapshot`/`current_url` where a sensible
    // derivation exists, `EngineError::Unsupported` where acting on a page
    // needs a real DOM.

    /// The agent's numbered, ref-addressable view of the active page — the
    /// agent-initiated `read_page` action. Recorded as a page read
    /// ([`Call::PageDigest`], `dom.read`).
    ///
    /// The default builds a digest from [`Self::dom_snapshot`]
    /// ([`PageDigest::from_snapshot`]); real engines override it with a page
    /// script that also stamps live refs ([`REF_ATTRIBUTE`]).
    fn page_digest(&mut self) -> Result<(PageDigest, Origin), EngineError> {
        let (snapshot, origin) = self.dom_snapshot()?;
        Ok((
            PageDigest::from_snapshot(&snapshot, origin.as_str()),
            origin,
        ))
    }

    /// The **harness's own** per-step observation of the page — what the
    /// agent loop appends after a state-changing action so the model need not
    /// spend a step on `read_page`.
    ///
    /// This is deliberately *not* an agent-initiated action. An engine that
    /// records what the agent did (the dry run's `DryRunEngine`) must
    /// implement it **without** recording a call: were it logged, every
    /// task's dry-run record would contain `dom.read`, the comparator would
    /// flag a deviation on nearly every task, and the user would be
    /// consent-prompted constantly. The default simply delegates to
    /// [`Self::page_digest`], which is right for engines that keep no
    /// record.
    fn observe_page(&mut self) -> Result<(PageDigest, Origin), EngineError> {
        self.page_digest()
    }

    /// Presses `key` (`Enter`, `Escape`, `Tab`, `ArrowDown`, a single
    /// character, optionally with `Ctrl+`/`Shift+`/`Alt+`/`Meta+` prefixes)
    /// on the element `selector` resolves to, or on whatever has focus when
    /// `selector` is `None`.
    fn press_key(
        &mut self,
        _selector: Option<&str>,
        _key: &str,
    ) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "press_key: this engine has no keyboard input path",
        ))
    }

    /// Moves the pointer over the element `selector` resolves to.
    fn hover(&mut self, _selector: &str) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "hover: this engine has no pointer input path",
        ))
    }

    /// Makes the checkbox/radio/switch `selector` resolves to checked or
    /// unchecked. Idempotent: it only clicks when the current state differs.
    fn set_checked(
        &mut self,
        _selector: &str,
        _checked: bool,
    ) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "set_checked: this engine cannot read or toggle element state",
        ))
    }

    /// Scrolls the element `selector` resolves to into the middle of the
    /// viewport.
    fn scroll_to(&mut self, _selector: &str) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "scroll_to: this engine cannot scroll to an element",
        ))
    }

    /// Find-in-page: how often `text` occurs on the page, with a few
    /// surrounding snippets. The default searches [`Self::page_digest`]'s
    /// (bounded) text.
    fn find_text(&mut self, text: &str) -> Result<(TextMatches, Origin), EngineError> {
        let (digest, origin) = self.page_digest()?;
        Ok((digest.find_text(text), origin))
    }

    /// The links on the page, or inside the element `selector` resolves to.
    /// The default lists the digest's links and cannot scope to a selector.
    fn extract_links(
        &mut self,
        selector: Option<&str>,
    ) -> Result<(Vec<LinkInfo>, Origin), EngineError> {
        if selector.is_some() {
            return Err(EngineError::Unsupported(
                "extract_links: this engine cannot scope link extraction to a selector",
            ));
        }
        let (digest, origin) = self.page_digest()?;
        Ok((digest.links(), origin))
    }

    /// Submits the form `selector` resolves to (or contains the resolved
    /// element), or the page's only/first form when `selector` is `None` —
    /// via `requestSubmit()`, so validation and submit handlers run.
    fn submit_form(&mut self, _selector: Option<&str>) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "submit_form: this engine cannot submit forms",
        ))
    }

    /// The open tabs. The default reports the one tab an engine with no tab
    /// model has: the active page, as [`TabId`] `0`.
    fn list_tabs(&mut self) -> Result<(Vec<TabInfo>, Origin), EngineError> {
        let (url, origin) = self.current_url()?;
        Ok((
            vec![TabInfo {
                id: TabId(0),
                url,
                title: String::new(),
                active: true,
            }],
            origin,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_calls_map_to_conservative_primitives() {
        // Reads.
        for call in [
            Call::PageDigest,
            Call::FindText("x".into()),
            Call::ExtractLinks(None),
            Call::ExtractLinks(Some("nav".into())),
            Call::ListTabs,
        ] {
            assert_eq!(call.primitive(), Primitive::DomRead, "{call:?}");
        }
        // Key input is input.
        assert_eq!(
            Call::PressKey(None, "Enter".into()).primitive(),
            Primitive::DomWrite
        );
        // Pointer-driven or submitting actions are never weaker than a click.
        for call in [
            Call::Hover("@1".into()),
            Call::SetChecked("@2".into(), true),
            Call::SubmitForm(None),
            Call::SubmitForm(Some("form".into())),
        ] {
            assert_eq!(call.primitive(), Primitive::Click, "{call:?}");
        }
        assert_eq!(Call::ScrollTo("@3".into()).primitive(), Primitive::Scroll);
    }

    #[test]
    fn js_execute_is_still_its_own_unscopable_primitive() {
        assert_eq!(
            Call::JsExecute("1".into()).primitive(),
            Primitive::JsExecute
        );
    }
}
