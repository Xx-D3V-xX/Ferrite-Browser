//! `ServoEngine`/[`BorrowedServoEngine`]: the real, Servo-backed
//! [`BrowserEngine`] embeddings (`docs/REBUILD_DIRECTIVE.md` §6/A9, T-109;
//! `BorrowedServoEngine` added by B3, T-220/T-224).
//!
//! This crate wraps `ferrite_servo::session::HeadlessServoSession` — an
//! **existing**, feature-gated, real Servo integration — rather than
//! reimplementing raw `libservo`/winit plumbing. `HeadlessServoSession`
//! already compiles to a real Servo embedding behind `ferrite-servo`'s own
//! `servo` Cargo feature, and to an inert, always-succeeding stub when that
//! feature is off. This crate's own `engine-servo` feature does nothing
//! more than turn `ferrite-servo/servo` on:
//!
//! ```toml
//! [features]
//! engine-servo = ["ferrite-servo/servo"]
//! ```
//!
//! so `ServoEngine`/`BorrowedServoEngine` compile unconditionally (as part
//! of a normal `cargo build --workspace`, with zero Servo dependency cost —
//! directive §7.1), and only become a *real* browser when built with
//! `--features engine-servo` (`just build-servo`) — or, for
//! `BorrowedServoEngine`, whenever the process that constructs the
//! `HeadlessServoSession` it borrows was itself built with
//! `ferrite-servo/servo` (e.g. `cargo run -p ferrite-shell --features
//! ferrite-servo/servo`, which is how `ferrite-ui` gets a real one).
//!
//! # Two engines, one implementation (`mod ops`)
//!
//! [`ServoEngine`] **owns** one or more `HeadlessServoSession`s (one per
//! tab) and is the right choice for a caller that has no session of its own
//! yet and wants this crate to create and manage them — `ferrite-eval`'s
//! conformance suite, `ferrite-shell`'s `agent-smoke`.
//!
//! [`BorrowedServoEngine`] wraps a single **externally-owned**
//! `&mut HeadlessServoSession` instead of constructing one. This exists for
//! `ferrite-ui` (T-220/T-224, `docs/TO-DO.md`): the live app already owns a
//! `HeadlessServoSession` per tab, already drives it correctly via Iced's
//! real winit event loop (`FerriteBrowser::servo_sessions`, ticked every
//! 16ms by the `ServoFrame` subscription) — the exact ingredient T-220 found
//! missing from `ServoEngine`'s own standalone conformance-test environment,
//! which never had a real winit `EventLoop` in the process at all. Rather
//! than have `ferrite-ui` hand its live, correctly-driven session over to a
//! *second*, engine-owned session (duplicating Servo state, or requiring
//! `ferrite-ui`'s own navigation/rendering code to go through the engine
//! too), `BorrowedServoEngine` gives that exact session a `BrowserEngine`
//! face directly — one session, two views onto it (the tab UI's direct
//! calls, and the agent loop's `BrowserEngine` calls), never two sessions.
//!
//! Both engines share one implementation of every DOM/JS/navigation method
//! (the private `ops` module below, parameterized over `&mut
//! HeadlessServoSession` rather than `&mut self`) — no method's script or
//! logic is duplicated between them. `ServoEngine`'s own tab-management
//! methods (`open_tab`/`close_tab`/`switch_tab`) have no equivalent on
//! `BorrowedServoEngine`, which wraps exactly one, externally-managed tab —
//! see that impl's own methods for why they return
//! [`EngineError::Unsupported`] rather than silently doing something a
//! caller would not expect (opening a second, engine-owned session behind
//! the host application's back).
//!
//! # Mapping decisions — read before assuming a method "just works"
//!
//! `HeadlessServoSession` was built for `ferrite-shell`'s windowed UI, not
//! for an agent action surface, so several `BrowserEngine` methods are
//! implemented on top of primitives that were not designed for this
//! purpose. Documented plainly here, once, rather than scattered as
//! per-method surprises:
//!
//! - **`dom_snapshot` / `query` / `read_text` / `click` / `type_text` /
//!   `select_option`** are all implemented by injecting a small JavaScript
//!   snippet via `execute_js` that itself calls `JSON.stringify(...)`, and
//!   parsing the returned JSON. This depends on the page's JS engine being
//!   available and reflects only what page JS can see — it will not see
//!   content a page's own script could not see either (e.g. cross-origin
//!   iframe internals), and it fails if the page has disabled or not yet
//!   initialized its JS context. There is no separate, JS-independent
//!   accessibility-tree accessor exposed by `HeadlessServoSession` today.
//! - **`execute_js`'s `Ok` branch is a `Debug` rendering** of whatever
//!   value type Servo's `evaluate_javascript` callback produces, not the
//!   raw JS value (see `HeadlessServoSession::execute_js`'s own doc
//!   comment in `ferrite-servo`). For the `dom_snapshot`/`query`/
//!   `read_text` scripts above (which always return a JS string via
//!   `JSON.stringify`), this crate's [`unwrap_js_string_result`] strips the
//!   observed `String("...")` wrapping shape and unescapes the interior
//!   with `serde_json`'s string parser (JSON and Rust `Debug` string
//!   escaping coincide for the common ASCII/BMP case). This is a real,
//!   working mechanism, not a placeholder — but it depends on
//!   `execute_js`'s current, untyped return channel, and breaks if that
//!   channel's underlying type ever changes.
//! - **`js_execute` itself does *not* attempt this unwrapping.** A
//!   caller's arbitrary script can return any JS value type, each
//!   Debug-formatted differently, and guessing at all of them is out of
//!   scope here. `js_execute` returns `execute_js`'s raw string verbatim;
//!   a caller wanting a clean value should have their own script call
//!   `JSON.stringify()`, exactly as this crate's own internal scripts do.
//! - **`download` is unimplemented**: `HeadlessServoSession` has no
//!   download manager. Returns [`EngineError::Unsupported`].
//! - **`clipboard_read`/`clipboard_write` are unimplemented**:
//!   `HeadlessServoSession` exposes no synchronous clipboard API, and
//!   `navigator.clipboard` is Promise-based and permission-gated in a way
//!   `execute_js`'s single-shot synchronous callback cannot resolve.
//!   Returns [`EngineError::Unsupported`].
//! - **`cookies_read`/`storage_read` only succeed for the *currently
//!   loaded* page's own origin.** Both are implemented via `execute_js`
//!   reading `document.cookie`/`localStorage`, which only ever sees the
//!   active page's own origin — exactly like a same-origin script would.
//!   Rather than navigate away to satisfy a request for a different
//!   origin's data as a side effect of a "read" (a real, surprising
//!   behavior change this engine will not perform silently), a `scope`
//!   that does not match the active page's origin is
//!   [`EngineError::Unsupported`]. This is a real scoping restriction, not
//!   a missing feature dressed up as one: it makes reading another
//!   origin's cookies/storage impossible through this path, which is the
//!   security property `cookies_read`/`storage_read` scoping exists for.
//! - **`ServoEngine` tabs are real**: each open tab is its own
//!   `HeadlessServoSession`, sharing the same process-wide Servo engine
//!   (`ferrite-servo`'s own `get_or_init_servo` singleton) — this mirrors
//!   how `ferrite-shell` already runs multiple tabs. `BorrowedServoEngine`
//!   has no tab model of its own at all — see the module docs above.
//! - **A fresh `ServoEngine` tab starts at `about:blank`**, a real, opaque
//!   origin — see `ferrite_engine`'s module docs for why this is a typed
//!   [`EngineError::OpaqueOrigin`] here, unlike `MockEngine`'s synthetic
//!   placeholder home origin. A `BorrowedServoEngine` reports whatever
//!   origin its borrowed session is already showing, which may likewise be
//!   `about:blank` on a freshly opened tab.

#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ferrite_core::Origin;
use ferrite_engine::{
    BrowserEngine, Cookie, DomSnapshot, ElementHandle, EngineError, Frame, LinkInfo, PageDigest,
    TabId, TabInfo, TextMatches, WaitCondition,
};
use ferrite_servo::session::{HeadlessServoSession, LoadStatus};

mod script;

const NAV_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// How long an action that might start a navigation waits for it to begin.
const SETTLE_WINDOW: Duration = Duration::from_millis(150);

// ---------------------------------------------------------------------------
// ops: the one implementation of every BrowserEngine method, over a borrowed
// `&mut HeadlessServoSession`. `ServoEngine` and `BorrowedServoEngine` both
// call into this — see the module docs above for why.
// ---------------------------------------------------------------------------
mod ops {
    use super::script::{build_script, check};
    use super::{
        Cookie, ElementHandle, EngineError, HeadlessServoSession, Instant, LoadStatus, Origin,
        WaitCondition, NAV_TIMEOUT, POLL_INTERVAL, SETTLE_WINDOW,
    };
    use ferrite_engine::{
        normalize_selector, DomSnapshot, LinkInfo, PageDigest, TabId, TabInfo, TextMatches,
    };
    use serde_json::{json, Value};

    pub(crate) fn current_origin(session: &HeadlessServoSession) -> Result<Origin, EngineError> {
        let url = session.current_url().to_string();
        Origin::parse(&url).map_err(|e| EngineError::OpaqueOrigin(format!("{url}: {e}")))
    }

    /// Pumps `session` until it reports a settled load state or
    /// `NAV_TIMEOUT` elapses — mirrors
    /// `HeadlessServoSession::test_js_compat`'s own drive loop.
    pub(crate) fn drive_until_loaded(
        session: &mut HeadlessServoSession,
    ) -> Result<(), EngineError> {
        let deadline = Instant::now() + NAV_TIMEOUT;
        loop {
            session.spin();
            let done = matches!(
                session.load_status(),
                LoadStatus::Complete | LoadStatus::Failed(_)
            );
            if done || Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// After an action that may have *started* a navigation (a link click,
    /// Enter in a form, a `change` handler that submits): give the engine a
    /// brief window to begin loading and, if it does, wait for the load to
    /// settle — so the observation that follows describes the new page, not
    /// the one being torn down. A no-navigation action costs `SETTLE_WINDOW`.
    pub(crate) fn settle(session: &mut HeadlessServoSession) {
        let deadline = Instant::now() + SETTLE_WINDOW;
        loop {
            session.spin();
            if matches!(session.load_status(), LoadStatus::Loading) {
                let _ = drive_until_loaded(session);
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Runs `script` (which must itself call `JSON.stringify(...)`) and
    /// returns the unwrapped, unescaped JSON text it produced.
    pub(crate) fn run_json_js(
        session: &mut HeadlessServoSession,
        script: &str,
    ) -> Result<String, EngineError> {
        let raw = session.execute_js(script).map_err(EngineError::Internal)?;
        super::unwrap_js_string_result(&raw)
    }

    /// Runs one operation of the page script (`page_ops.js`) and parses its
    /// JSON answer. Failures the *script* reports (`{ok:false,...}`) are
    /// left in the value for [`check`] to type; only transport/parse
    /// failures are errors here.
    fn run_op(
        session: &mut HeadlessServoSession,
        op: &str,
        args: &Value,
    ) -> Result<Value, EngineError> {
        let json = run_json_js(session, &build_script(op, args))?;
        serde_json::from_str(&json).map_err(|e| {
            EngineError::Internal(format!("{op}: malformed JSON from page script: {e}"))
        })
    }

    /// `run_op` + [`check`] against the selector as the model wrote it.
    fn run_checked(
        session: &mut HeadlessServoSession,
        op: &str,
        selector: &str,
        args: &Value,
    ) -> Result<Value, EngineError> {
        let v = run_op(session, op, args)?;
        check(op, selector, &v)?;
        Ok(v)
    }

    pub(crate) fn navigate(
        session: &mut HeadlessServoSession,
        url: &str,
    ) -> Result<((), Origin), EngineError> {
        session.navigate(url);
        drive_until_loaded(session)?;
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn go_back(session: &mut HeadlessServoSession) -> Result<((), Origin), EngineError> {
        session.go_back();
        drive_until_loaded(session)?;
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn go_forward(
        session: &mut HeadlessServoSession,
    ) -> Result<((), Origin), EngineError> {
        session.go_forward();
        drive_until_loaded(session)?;
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn reload(session: &mut HeadlessServoSession) -> Result<((), Origin), EngineError> {
        session.reload();
        drive_until_loaded(session)?;
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn current_url(
        session: &HeadlessServoSession,
    ) -> Result<(String, Origin), EngineError> {
        let url = session.current_url().to_string();
        Ok((url, current_origin(session)?))
    }

    pub(crate) fn dom_snapshot(
        session: &mut HeadlessServoSession,
    ) -> Result<(DomSnapshot, Origin), EngineError> {
        let json = run_json_js(session, super::SNAPSHOT_SCRIPT)?;
        let snapshot: DomSnapshot = serde_json::from_str(&json).map_err(|e| {
            EngineError::Internal(format!(
                "dom_snapshot: malformed JSON from page script: {e}"
            ))
        })?;
        Ok((snapshot, current_origin(session)?))
    }

    /// The numbered, ref-addressable view of the page. Stamps
    /// `data-ferrite-ref` on the live DOM as a side effect (see
    /// `page_ops.js`), then re-bounds everything the script returned.
    pub(crate) fn page_digest(
        session: &mut HeadlessServoSession,
    ) -> Result<(PageDigest, Origin), EngineError> {
        let v = run_op(session, "digest", &json!({}))?;
        check("read_page", "", &v)?;
        let digest: PageDigest = serde_json::from_value(v).map_err(|e| {
            EngineError::Internal(format!("read_page: malformed digest from page script: {e}"))
        })?;
        Ok((digest.sanitized(), current_origin(session)?))
    }

    pub(crate) fn query(
        session: &mut HeadlessServoSession,
        selector: &str,
    ) -> Result<(Vec<ElementHandle>, Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector) });
        let v = run_checked(session, "query", selector, &args)?;
        let handles: Vec<ElementHandle> =
            serde_json::from_value(v.get("items").cloned().unwrap_or(Value::Null)).map_err(
                |e| EngineError::Internal(format!("query: malformed JSON from page script: {e}")),
            )?;
        Ok((handles, current_origin(session)?))
    }

    pub(crate) fn read_text(
        session: &mut HeadlessServoSession,
        selector: &str,
    ) -> Result<(String, Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector) });
        let v = run_checked(session, "read_text", selector, &args)?;
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok((text, current_origin(session)?))
    }

    pub(crate) fn click(
        session: &mut HeadlessServoSession,
        selector: &str,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector) });
        run_checked(session, "click", selector, &args)?;
        settle(session);
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn hover(
        session: &mut HeadlessServoSession,
        selector: &str,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector) });
        run_checked(session, "hover", selector, &args)?;
        session.spin();
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn scroll_to(
        session: &mut HeadlessServoSession,
        selector: &str,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector) });
        run_checked(session, "scroll_to", selector, &args)?;
        session.spin();
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn type_text(
        session: &mut HeadlessServoSession,
        selector: &str,
        text: &str,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector), "text": text });
        run_checked(session, "type_text", selector, &args)?;
        Ok(((), current_origin(session)?))
    }

    /// One script call for the whole form: each field is filled however its
    /// control takes a value (text, `<select>`, checkbox/radio), stopping at
    /// the first failure, which is reported against *that* field's selector.
    pub(crate) fn fill_form(
        session: &mut HeadlessServoSession,
        fields: &[(String, String)],
    ) -> Result<((), Origin), EngineError> {
        if fields.is_empty() {
            return Ok(((), current_origin(session)?));
        }
        let normalized: Vec<[String; 2]> = fields
            .iter()
            .map(|(sel, value)| [normalize_selector(sel), value.clone()])
            .collect();
        let v = run_op(session, "fill_form", &json!({ "fields": normalized }))?;
        let failed = v
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| fields.get(i))
            .map_or("", |(sel, _)| sel.as_str());
        check("fill_form", failed, &v)?;
        settle(session);
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn select_option(
        session: &mut HeadlessServoSession,
        selector: &str,
        value: &str,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector), "value": value });
        run_checked(session, "select_option", selector, &args)?;
        settle(session);
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn set_checked(
        session: &mut HeadlessServoSession,
        selector: &str,
        checked: bool,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": normalize_selector(selector), "checked": checked });
        run_checked(session, "set_checked", selector, &args)?;
        settle(session);
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn press_key(
        session: &mut HeadlessServoSession,
        selector: Option<&str>,
        key: &str,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": selector.map(normalize_selector), "key": key });
        run_checked(
            session,
            "press_key",
            selector.unwrap_or("the focused element"),
            &args,
        )?;
        settle(session);
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn submit_form(
        session: &mut HeadlessServoSession,
        selector: Option<&str>,
    ) -> Result<((), Origin), EngineError> {
        let args = json!({ "sel": selector.map(normalize_selector) });
        run_checked(
            session,
            "submit_form",
            selector.unwrap_or("the form"),
            &args,
        )?;
        settle(session);
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn find_text(
        session: &mut HeadlessServoSession,
        text: &str,
    ) -> Result<(TextMatches, Origin), EngineError> {
        let v = run_checked(session, "find_text", "", &json!({ "text": text }))?;
        let matches: TextMatches = serde_json::from_value(v).map_err(|e| {
            EngineError::Internal(format!("find_text: malformed JSON from page script: {e}"))
        })?;
        Ok((matches, current_origin(session)?))
    }

    pub(crate) fn extract_links(
        session: &mut HeadlessServoSession,
        selector: Option<&str>,
    ) -> Result<(Vec<LinkInfo>, Origin), EngineError> {
        let args = json!({ "sel": selector.map(normalize_selector) });
        let v = run_checked(
            session,
            "extract_links",
            selector.unwrap_or("the page"),
            &args,
        )?;
        let links: Vec<LinkInfo> = serde_json::from_value(
            v.get("links").cloned().unwrap_or(Value::Null),
        )
        .map_err(|e| {
            EngineError::Internal(format!(
                "extract_links: malformed JSON from page script: {e}"
            ))
        })?;
        Ok((links, current_origin(session)?))
    }

    /// The one tab of a session, as [`TabInfo`].
    pub(crate) fn tab_info(id: TabId, session: &HeadlessServoSession, active: bool) -> TabInfo {
        TabInfo {
            id,
            url: session.current_url().to_string(),
            title: session.page_title().unwrap_or_default().to_string(),
            active,
        }
    }

    pub(crate) fn scroll(
        session: &mut HeadlessServoSession,
        width: u32,
        height: u32,
        dx: i64,
        dy: i64,
    ) -> Result<((), Origin), EngineError> {
        let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
        session.send_scroll(cx, cy, dx as f64, dy as f64);
        session.spin();
        Ok(((), current_origin(session)?))
    }

    pub(crate) fn wait_for(
        session: &mut HeadlessServoSession,
        condition: WaitCondition,
    ) -> Result<((), Origin), EngineError> {
        match condition {
            WaitCondition::Idle => {
                drive_until_loaded(session)?;
                Ok(((), current_origin(session)?))
            }
            WaitCondition::Timeout(duration) => {
                let deadline = Instant::now() + duration;
                while Instant::now() < deadline {
                    session.spin();
                    std::thread::sleep(POLL_INTERVAL.min(duration));
                }
                Ok(((), current_origin(session)?))
            }
            WaitCondition::Selector(selector) => {
                let deadline = Instant::now() + NAV_TIMEOUT;
                loop {
                    if query(session, &selector)
                        .map(|(h, _)| !h.is_empty())
                        .unwrap_or(false)
                    {
                        return Ok(((), current_origin(session)?));
                    }
                    if Instant::now() >= deadline {
                        return Err(EngineError::WaitTimedOut);
                    }
                    session.spin();
                    std::thread::sleep(POLL_INTERVAL);
                }
            }
        }
    }

    pub(crate) fn screenshot(
        session: &mut HeadlessServoSession,
    ) -> Result<(super::Frame, Origin), EngineError> {
        session.spin();
        let frame = session
            .get_frame()
            .ok_or_else(|| EngineError::Internal("no frame has been rendered yet".to_string()))?;
        Ok((frame, current_origin(session)?))
    }

    pub(crate) fn cookies_read(
        session: &mut HeadlessServoSession,
        scope: &Origin,
    ) -> Result<(Vec<Cookie>, Origin), EngineError> {
        let origin = current_origin(session)?;
        if scope != &origin {
            return Err(EngineError::Unsupported(
                "cookies_read: scope must be the currently loaded page's own origin",
            ));
        }
        let json = run_json_js(
            session,
            "(function(){ return JSON.stringify(document.cookie); })()",
        )?;
        let raw: String = serde_json::from_str(&json).map_err(|e| {
            EngineError::Internal(format!(
                "cookies_read: malformed JSON from page script: {e}"
            ))
        })?;
        let cookies = raw
            .split(';')
            .filter_map(|pair| {
                let pair = pair.trim();
                if pair.is_empty() {
                    return None;
                }
                let (name, value) = pair.split_once('=')?;
                Some(Cookie {
                    name: name.trim().to_string(),
                    value: value.trim().to_string(),
                })
            })
            .collect();
        Ok((cookies, origin))
    }

    pub(crate) fn storage_read(
        session: &mut HeadlessServoSession,
        scope: &Origin,
    ) -> Result<(Vec<(String, String)>, Origin), EngineError> {
        let origin = current_origin(session)?;
        if scope != &origin {
            return Err(EngineError::Unsupported(
                "storage_read: scope must be the currently loaded page's own origin",
            ));
        }
        let json = run_json_js(
            session,
            "(function(){ var out=[]; for (var i=0;i<localStorage.length;i++){ \
             var k=localStorage.key(i); out.push([k, localStorage.getItem(k)]); } \
             return JSON.stringify(out); })()",
        )?;
        let kv: Vec<(String, String)> = serde_json::from_str(&json).map_err(|e| {
            EngineError::Internal(format!(
                "storage_read: malformed JSON from page script: {e}"
            ))
        })?;
        Ok((kv, origin))
    }

    pub(crate) fn js_execute(
        session: &mut HeadlessServoSession,
        script: &str,
    ) -> Result<(String, Origin), EngineError> {
        let raw = session.execute_js(script).map_err(EngineError::Internal)?;
        Ok((raw, current_origin(session)?))
    }
}

struct Tab {
    session: HeadlessServoSession,
}

/// The real, Servo-backed [`BrowserEngine`] that **owns** every tab's
/// session. See the [module docs](self) for when to use this vs.
/// [`BorrowedServoEngine`], and for the full method-by-method mapping onto
/// `HeadlessServoSession`, including the methods this engine honestly
/// cannot support today (`download`, `clipboard_*`, cross-origin
/// `cookies_read`/`storage_read`).
pub struct ServoEngine {
    tabs: BTreeMap<TabId, Tab>,
    active: TabId,
    next_tab: u64,
    width: u32,
    height: u32,
}

impl std::fmt::Debug for ServoEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServoEngine")
            .field("tab_count", &self.tabs.len())
            .field("active", &self.active)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl ServoEngine {
    /// Creates a new engine with one tab open at `about:blank`, sized
    /// `width × height`.
    ///
    /// # Errors
    ///
    /// [`EngineError::Internal`] if `HeadlessServoSession::new` fails (e.g.
    /// the `servo` feature is compiled in but Servo's native init panics —
    /// `HeadlessServoSession::new` already converts that panic into an
    /// `Err` rather than aborting the process).
    pub fn new(width: u32, height: u32) -> Result<Self, EngineError> {
        let session = HeadlessServoSession::new(width, height).map_err(EngineError::Internal)?;
        let mut tabs = BTreeMap::new();
        let home = TabId(0);
        tabs.insert(home, Tab { session });
        Ok(Self {
            tabs,
            active: home,
            next_tab: 1,
            width,
            height,
        })
    }

    fn active_tab(&self) -> Result<&Tab, EngineError> {
        self.tabs
            .get(&self.active)
            .ok_or(EngineError::NoSuchTab(self.active))
    }

    fn active_tab_mut(&mut self) -> Result<&mut Tab, EngineError> {
        self.tabs
            .get_mut(&self.active)
            .ok_or(EngineError::NoSuchTab(self.active))
    }
}

impl BrowserEngine for ServoEngine {
    fn navigate(&mut self, url: &str) -> Result<((), Origin), EngineError> {
        ops::navigate(&mut self.active_tab_mut()?.session, url)
    }

    fn go_back(&mut self) -> Result<((), Origin), EngineError> {
        ops::go_back(&mut self.active_tab_mut()?.session)
    }

    fn go_forward(&mut self) -> Result<((), Origin), EngineError> {
        ops::go_forward(&mut self.active_tab_mut()?.session)
    }

    fn reload(&mut self) -> Result<((), Origin), EngineError> {
        ops::reload(&mut self.active_tab_mut()?.session)
    }

    fn current_url(&mut self) -> Result<(String, Origin), EngineError> {
        ops::current_url(&self.active_tab()?.session)
    }

    fn dom_snapshot(&mut self) -> Result<(DomSnapshot, Origin), EngineError> {
        ops::dom_snapshot(&mut self.active_tab_mut()?.session)
    }

    fn query(&mut self, selector: &str) -> Result<(Vec<ElementHandle>, Origin), EngineError> {
        ops::query(&mut self.active_tab_mut()?.session, selector)
    }

    fn read_text(&mut self, selector: &str) -> Result<(String, Origin), EngineError> {
        ops::read_text(&mut self.active_tab_mut()?.session, selector)
    }

    fn click(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        ops::click(&mut self.active_tab_mut()?.session, selector)
    }

    fn type_text(&mut self, selector: &str, text: &str) -> Result<((), Origin), EngineError> {
        ops::type_text(&mut self.active_tab_mut()?.session, selector, text)
    }

    fn fill_form(&mut self, fields: &[(String, String)]) -> Result<((), Origin), EngineError> {
        ops::fill_form(&mut self.active_tab_mut()?.session, fields)
    }

    fn select_option(&mut self, selector: &str, value: &str) -> Result<((), Origin), EngineError> {
        ops::select_option(&mut self.active_tab_mut()?.session, selector, value)
    }

    fn scroll(&mut self, dx: i64, dy: i64) -> Result<((), Origin), EngineError> {
        let (width, height) = (self.width, self.height);
        ops::scroll(&mut self.active_tab_mut()?.session, width, height, dx, dy)
    }

    fn wait_for(&mut self, condition: WaitCondition) -> Result<((), Origin), EngineError> {
        ops::wait_for(&mut self.active_tab_mut()?.session, condition)
    }

    fn screenshot(&mut self) -> Result<(Frame, Origin), EngineError> {
        ops::screenshot(&mut self.active_tab_mut()?.session)
    }

    fn download(&mut self, _url: &str) -> Result<(String, Origin), EngineError> {
        Err(EngineError::Unsupported(
            "download: HeadlessServoSession has no download manager wired up",
        ))
    }

    fn open_tab(&mut self, url: Option<&str>) -> Result<(TabId, Origin), EngineError> {
        let session =
            HeadlessServoSession::new(self.width, self.height).map_err(EngineError::Internal)?;
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        self.tabs.insert(id, Tab { session });
        self.active = id;
        if let Some(url) = url {
            ops::navigate(&mut self.active_tab_mut()?.session, url)?;
        }
        Ok((id, ops::current_origin(&self.active_tab()?.session)?))
    }

    fn close_tab(&mut self, tab: TabId) -> Result<((), Origin), EngineError> {
        if self.tabs.remove(&tab).is_none() {
            return Err(EngineError::NoSuchTab(tab));
        }
        if self.active == tab {
            if let Some(&next) = self.tabs.keys().next() {
                self.active = next;
            } else {
                let session = HeadlessServoSession::new(self.width, self.height)
                    .map_err(EngineError::Internal)?;
                let id = TabId(self.next_tab);
                self.next_tab += 1;
                self.tabs.insert(id, Tab { session });
                self.active = id;
            }
        }
        Ok(((), ops::current_origin(&self.active_tab()?.session)?))
    }

    fn switch_tab(&mut self, tab: TabId) -> Result<((), Origin), EngineError> {
        if !self.tabs.contains_key(&tab) {
            return Err(EngineError::NoSuchTab(tab));
        }
        self.active = tab;
        Ok(((), ops::current_origin(&self.active_tab()?.session)?))
    }

    fn cookies_read(&mut self, scope: &Origin) -> Result<(Vec<Cookie>, Origin), EngineError> {
        ops::cookies_read(&mut self.active_tab_mut()?.session, scope)
    }

    fn storage_read(
        &mut self,
        scope: &Origin,
    ) -> Result<(Vec<(String, String)>, Origin), EngineError> {
        ops::storage_read(&mut self.active_tab_mut()?.session, scope)
    }

    fn clipboard_read(&mut self) -> Result<(String, Origin), EngineError> {
        Err(EngineError::Unsupported(
            "clipboard_read: HeadlessServoSession exposes no synchronous clipboard API",
        ))
    }

    fn clipboard_write(&mut self, _text: &str) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "clipboard_write: HeadlessServoSession exposes no synchronous clipboard API",
        ))
    }

    fn js_execute(&mut self, script: &str) -> Result<(String, Origin), EngineError> {
        ops::js_execute(&mut self.active_tab_mut()?.session, script)
    }

    fn page_digest(&mut self) -> Result<(PageDigest, Origin), EngineError> {
        ops::page_digest(&mut self.active_tab_mut()?.session)
    }

    fn press_key(
        &mut self,
        selector: Option<&str>,
        key: &str,
    ) -> Result<((), Origin), EngineError> {
        ops::press_key(&mut self.active_tab_mut()?.session, selector, key)
    }

    fn hover(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        ops::hover(&mut self.active_tab_mut()?.session, selector)
    }

    fn set_checked(&mut self, selector: &str, checked: bool) -> Result<((), Origin), EngineError> {
        ops::set_checked(&mut self.active_tab_mut()?.session, selector, checked)
    }

    fn scroll_to(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        ops::scroll_to(&mut self.active_tab_mut()?.session, selector)
    }

    fn find_text(&mut self, text: &str) -> Result<(TextMatches, Origin), EngineError> {
        ops::find_text(&mut self.active_tab_mut()?.session, text)
    }

    fn extract_links(
        &mut self,
        selector: Option<&str>,
    ) -> Result<(Vec<LinkInfo>, Origin), EngineError> {
        ops::extract_links(&mut self.active_tab_mut()?.session, selector)
    }

    fn submit_form(&mut self, selector: Option<&str>) -> Result<((), Origin), EngineError> {
        ops::submit_form(&mut self.active_tab_mut()?.session, selector)
    }

    fn list_tabs(&mut self) -> Result<(Vec<TabInfo>, Origin), EngineError> {
        let active = self.active;
        let tabs = self
            .tabs
            .iter()
            .map(|(id, tab)| ops::tab_info(*id, &tab.session, *id == active))
            .collect();
        Ok((tabs, ops::current_origin(&self.active_tab()?.session)?))
    }
}

/// A [`BrowserEngine`] face over a single, **externally-owned**
/// `HeadlessServoSession` — see the [module docs](self) for why this exists
/// alongside [`ServoEngine`] (T-220/T-224).
///
/// Wraps the session by mutable reference rather than taking ownership: the
/// host application (`ferrite-ui`) keeps driving the session's rendering
/// and navigation exactly as it already does (its own `ServoFrame`
/// subscription tick, its own tab bookkeeping) — this type only adds a
/// second way to *act* on the same session, for the duration of one
/// borrow, never a second session to keep in sync.
pub struct BorrowedServoEngine<'a> {
    session: &'a mut HeadlessServoSession,
    width: u32,
    height: u32,
}

impl<'a> BorrowedServoEngine<'a> {
    /// Wraps `session` (sized `width × height`, matching whatever the host
    /// last called `session.resize(width, height)` with) in a
    /// [`BrowserEngine`] face for the duration of this borrow.
    #[must_use]
    pub fn new(session: &'a mut HeadlessServoSession, width: u32, height: u32) -> Self {
        Self {
            session,
            width,
            height,
        }
    }
}

impl<'a> std::fmt::Debug for BorrowedServoEngine<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorrowedServoEngine")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl<'a> BrowserEngine for BorrowedServoEngine<'a> {
    fn navigate(&mut self, url: &str) -> Result<((), Origin), EngineError> {
        ops::navigate(self.session, url)
    }

    fn go_back(&mut self) -> Result<((), Origin), EngineError> {
        ops::go_back(self.session)
    }

    fn go_forward(&mut self) -> Result<((), Origin), EngineError> {
        ops::go_forward(self.session)
    }

    fn reload(&mut self) -> Result<((), Origin), EngineError> {
        ops::reload(self.session)
    }

    fn current_url(&mut self) -> Result<(String, Origin), EngineError> {
        ops::current_url(self.session)
    }

    fn dom_snapshot(&mut self) -> Result<(DomSnapshot, Origin), EngineError> {
        ops::dom_snapshot(self.session)
    }

    fn query(&mut self, selector: &str) -> Result<(Vec<ElementHandle>, Origin), EngineError> {
        ops::query(self.session, selector)
    }

    fn read_text(&mut self, selector: &str) -> Result<(String, Origin), EngineError> {
        ops::read_text(self.session, selector)
    }

    fn click(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        ops::click(self.session, selector)
    }

    fn type_text(&mut self, selector: &str, text: &str) -> Result<((), Origin), EngineError> {
        ops::type_text(self.session, selector, text)
    }

    fn fill_form(&mut self, fields: &[(String, String)]) -> Result<((), Origin), EngineError> {
        ops::fill_form(self.session, fields)
    }

    fn select_option(&mut self, selector: &str, value: &str) -> Result<((), Origin), EngineError> {
        ops::select_option(self.session, selector, value)
    }

    fn scroll(&mut self, dx: i64, dy: i64) -> Result<((), Origin), EngineError> {
        ops::scroll(self.session, self.width, self.height, dx, dy)
    }

    fn wait_for(&mut self, condition: WaitCondition) -> Result<((), Origin), EngineError> {
        ops::wait_for(self.session, condition)
    }

    fn screenshot(&mut self) -> Result<(Frame, Origin), EngineError> {
        ops::screenshot(self.session)
    }

    fn download(&mut self, _url: &str) -> Result<(String, Origin), EngineError> {
        Err(EngineError::Unsupported(
            "download: HeadlessServoSession has no download manager wired up",
        ))
    }

    fn open_tab(&mut self, _url: Option<&str>) -> Result<(TabId, Origin), EngineError> {
        Err(EngineError::Unsupported(
            "open_tab: BorrowedServoEngine wraps a single, externally-managed tab — open a new \
             tab through the host application instead",
        ))
    }

    fn close_tab(&mut self, _tab: TabId) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "close_tab: BorrowedServoEngine wraps a single, externally-managed tab — close it \
             through the host application instead",
        ))
    }

    fn switch_tab(&mut self, _tab: TabId) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "switch_tab: BorrowedServoEngine wraps a single, externally-managed tab — switch \
             tabs through the host application instead",
        ))
    }

    fn cookies_read(&mut self, scope: &Origin) -> Result<(Vec<Cookie>, Origin), EngineError> {
        ops::cookies_read(self.session, scope)
    }

    fn storage_read(
        &mut self,
        scope: &Origin,
    ) -> Result<(Vec<(String, String)>, Origin), EngineError> {
        ops::storage_read(self.session, scope)
    }

    fn clipboard_read(&mut self) -> Result<(String, Origin), EngineError> {
        Err(EngineError::Unsupported(
            "clipboard_read: HeadlessServoSession exposes no synchronous clipboard API",
        ))
    }

    fn clipboard_write(&mut self, _text: &str) -> Result<((), Origin), EngineError> {
        Err(EngineError::Unsupported(
            "clipboard_write: HeadlessServoSession exposes no synchronous clipboard API",
        ))
    }

    fn js_execute(&mut self, script: &str) -> Result<(String, Origin), EngineError> {
        ops::js_execute(self.session, script)
    }

    fn page_digest(&mut self) -> Result<(PageDigest, Origin), EngineError> {
        ops::page_digest(self.session)
    }

    fn press_key(
        &mut self,
        selector: Option<&str>,
        key: &str,
    ) -> Result<((), Origin), EngineError> {
        ops::press_key(self.session, selector, key)
    }

    fn hover(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        ops::hover(self.session, selector)
    }

    fn set_checked(&mut self, selector: &str, checked: bool) -> Result<((), Origin), EngineError> {
        ops::set_checked(self.session, selector, checked)
    }

    fn scroll_to(&mut self, selector: &str) -> Result<((), Origin), EngineError> {
        ops::scroll_to(self.session, selector)
    }

    fn find_text(&mut self, text: &str) -> Result<(TextMatches, Origin), EngineError> {
        ops::find_text(self.session, text)
    }

    fn extract_links(
        &mut self,
        selector: Option<&str>,
    ) -> Result<(Vec<LinkInfo>, Origin), EngineError> {
        ops::extract_links(self.session, selector)
    }

    fn submit_form(&mut self, selector: Option<&str>) -> Result<((), Origin), EngineError> {
        ops::submit_form(self.session, selector)
    }

    /// The single tab this engine wraps. The host application owns the real
    /// tab list (and intercepts the tab actions); this reports only the page
    /// the borrowed session is showing, as tab `0`.
    fn list_tabs(&mut self) -> Result<(Vec<TabInfo>, Origin), EngineError> {
        let origin = ops::current_origin(self.session)?;
        Ok((vec![ops::tab_info(TabId(0), self.session, true)], origin))
    }
}

/// Best-effort unwrap of [`HeadlessServoSession::execute_js`]'s
/// `Debug`-rendered result back into the raw string a `JSON.stringify(...)`
/// script actually returned — see the [module docs](self) for the full
/// reasoning and its honest limits.
fn unwrap_js_string_result(raw: &str) -> Result<String, EngineError> {
    let trimmed = raw.trim();
    let quoted = trimmed
        .strip_prefix("String(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(trimmed);
    serde_json::from_str::<String>(quoted).map_err(|e| {
        EngineError::Internal(format!("could not unwrap execute_js result {raw:?}: {e}"))
    })
}

const SNAPSHOT_SCRIPT: &str = r#"(function(){
  function nodeInfo(el, depth) {
    if (!el || depth > 8) return null;
    var role = el.getAttribute('role') ||
      ({A:'link',BUTTON:'button',INPUT:'textbox',SELECT:'combobox',IMG:'img',
        H1:'heading',H2:'heading',H3:'heading'}[el.tagName]) || 'generic';
    var label = el.getAttribute('aria-label') || el.getAttribute('alt') || null;
    var text = null;
    for (var i = 0; i < el.childNodes.length; i++) {
      var n = el.childNodes[i];
      if (n.nodeType === 3 && n.textContent && n.textContent.trim()) {
        text = (text || '') + n.textContent.trim();
      }
    }
    var rect = el.getBoundingClientRect ? el.getBoundingClientRect() : null;
    var children = [];
    for (var i = 0; i < el.children.length; i++) {
      var c = nodeInfo(el.children[i], depth + 1);
      if (c) children.push(c);
    }
    return {
      role: role, label: label, text: text,
      selector: el.id ? ('#' + el.id) : null,
      bounds: rect ? {x: rect.x, y: rect.y, width: rect.width, height: rect.height} : null,
      children: children
    };
  }
  return JSON.stringify({root: nodeInfo(document.documentElement, 0) || {role:'generic',children:[]}});
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_js_string_result_strips_the_observed_string_wrapper() {
        let raw = r#"String("{\"root\":{\"role\":\"generic\"}}")"#;
        let unwrapped = unwrap_js_string_result(raw).expect("must unwrap");
        assert_eq!(unwrapped, r#"{"root":{"role":"generic"}}"#);
    }

    #[test]
    fn unwrap_js_string_result_falls_back_to_parsing_the_whole_thing() {
        // If execute_js's channel ever returns an already-unwrapped JSON
        // string with no `String(...)` envelope, this must still work
        // rather than silently misparsing.
        let raw = r#""already unwrapped""#;
        let unwrapped = unwrap_js_string_result(raw).expect("must unwrap");
        assert_eq!(unwrapped, "already unwrapped");
    }

    #[test]
    fn unwrap_js_string_result_reports_a_typed_error_on_garbage() {
        let err = unwrap_js_string_result("not json at all").unwrap_err();
        assert!(matches!(err, EngineError::Internal(_)));
    }

    #[test]
    fn json_literal_escapes_quotes_and_backslashes() {
        assert_eq!(
            script::json_literal(&serde_json::Value::String("a\"b\\c".into())),
            "\"a\\\"b\\\\c\""
        );
    }

    // ── Engine construction without the `servo` feature ──
    //
    // With `engine-servo` off, `HeadlessServoSession::new` is the stub from
    // `ferrite-servo`, which always returns `Err`. This is exercised here
    // (not `#[ignore]`d) precisely because it needs no real Servo build —
    // it proves `ServoEngine::new`/`BorrowedServoEngine::new` surface that
    // stub failure/behavior in a typed, non-panicking way.
    #[cfg(not(feature = "engine-servo"))]
    #[test]
    fn without_the_engine_servo_feature_construction_fails_cleanly() {
        let err = ServoEngine::new(800, 600).unwrap_err();
        assert!(matches!(err, EngineError::Internal(_)));
    }

    #[cfg(not(feature = "engine-servo"))]
    #[test]
    fn without_the_engine_servo_feature_borrowed_engine_reports_unsupported_actions_not_panics() {
        // The stub `HeadlessServoSession` is a real, always-`Err`/no-op
        // type (see `ferrite_servo::session`'s `#[cfg(not(feature =
        // "servo"))]` stub impl) — constructing one needs no real Servo.
        let mut session = HeadlessServoSession;
        let mut engine = BorrowedServoEngine::new(&mut session, 800, 600);
        assert!(matches!(
            engine.open_tab(None),
            Err(EngineError::Unsupported(_))
        ));
        assert!(matches!(
            engine.clipboard_read(),
            Err(EngineError::Unsupported(_))
        ));
        // navigate() itself is a no-op against the stub and current_url()
        // always reports "about:blank" — current_origin() should therefore
        // report a real, parseable http(s)... no: "about:blank" is opaque,
        // so this must be a typed OpaqueOrigin error, not a panic.
        assert!(matches!(
            engine.navigate("https://example.com"),
            Err(EngineError::OpaqueOrigin(_))
        ));
    }
}
