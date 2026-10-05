// session.rs — Headless Servo session for embedding in Iced.
//
// `HeadlessServoSession` wraps a `SoftwareRenderingContext` + a single Servo
// `WebView`.  It does NOT own a winit event loop — the caller (Iced) drives it
// by calling `spin()` on every tick and reads rendered frames via `get_frame()`.
//
// This module is only compiled when the `servo` Cargo feature is enabled:
//   cargo build -p ferrite-servo --features servo
//
// ## Usage in Iced
//
//   let mut session = HeadlessServoSession::new(1280, 800)?;
//   session.navigate("https://example.com");
//
//   // On each Iced subscription tick:
//   session.spin();
//   if let Some((w, h, bytes)) = session.get_frame() {
//       let handle = image::Handle::from_rgba(w, h, bytes);
//       // display with iced::widget::image(handle)
//   }

/// Result of a JS compatibility probe run by [`HeadlessServoSession::test_js_compat`].
#[derive(Debug, Clone)]
pub struct JSCompatResult {
    /// The URL that was probed.
    pub url: String,
    /// `true` if any JavaScript ran — inferred from the page title being set by JS.
    pub js_executed: bool,
    /// JavaScript console errors collected during the load (requires the
    /// `servo` feature; always empty in stub builds).
    pub console_errors: Vec<String>,
    /// Final page title as set by JS (or `None` if the page never set one).
    pub page_title: Option<String>,
}

/// Load state of the active WebView.
///
/// Produced by [`HeadlessServoSession::load_status()`] and synced from the
/// `WebViewDelegate` callbacks on every [`HeadlessServoSession::spin()`] call.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadStatus {
    /// A navigation is in progress.
    Loading,
    /// The page has finished loading successfully.
    Complete,
    /// The page failed to load. Contains an error description.
    Failed(String),
}

/// A named (non-character) key a page can receive. A deliberately small set:
/// everything a text field, form or page shortcut actually needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNamedKey {
    /// Enter / Return.
    Enter,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// Arrow up.
    ArrowUp,
    /// Arrow down.
    ArrowDown,
    /// Arrow left.
    ArrowLeft,
    /// Arrow right.
    ArrowRight,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Insert.
    Insert,
    /// Function key `F1`..=`F12`.
    F(u8),
}

/// The key of a [`PageKeyEvent`]: the text it types, or a named key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageKey {
    /// The character(s) the key produces with modifiers applied (`"a"`,
    /// `"A"`, `"@"`, `" "`).
    Character(String),
    /// A non-character key.
    Named(PageNamedKey),
}

/// A clipboard editing action Servo handles itself (it owns the clipboard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageEdit {
    /// Copy the page's selection.
    Copy,
    /// Cut the focused field's selection.
    Cut,
    /// Paste into the focused field.
    Paste,
}

/// One key press or release destined for the focused element of the page.
///
/// Plain data, independent of both `iced` and `libservo`, so the UI can build
/// it and test the conversion without a Servo build; only the real session
/// turns it into a Servo `InputEvent`. Before this existed, nothing
/// forwarded keyboard input to pages at all — mouse events were forwarded, so
/// a text box could be clicked into but never typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageKeyEvent {
    /// `true` for key-down, `false` for key-up.
    pub down: bool,
    /// The key.
    pub key: PageKey,
    /// Shift is held.
    pub shift: bool,
    /// Control is held.
    pub ctrl: bool,
    /// Alt/Option is held.
    pub alt: bool,
    /// Command (macOS) / Windows key is held.
    pub meta: bool,
}

impl PageKeyEvent {
    /// Whether the platform's copy/paste modifier (Cmd on macOS, Ctrl
    /// elsewhere) is held without Alt.
    fn command_held(&self) -> bool {
        #[cfg(target_os = "macos")]
        let held = self.meta;
        #[cfg(not(target_os = "macos"))]
        let held = self.ctrl;
        held && !self.alt
    }

    /// The clipboard action this key combination means (`Cmd/Ctrl + C/X/V`),
    /// regardless of press/release. Servo performs these itself via
    /// `EditingActionEvent`; the raw key events for them are not forwarded.
    #[must_use]
    pub fn edit_combo(&self) -> Option<PageEdit> {
        if !self.command_held() {
            return None;
        }
        match &self.key {
            PageKey::Character(c) if c.eq_ignore_ascii_case("c") => Some(PageEdit::Copy),
            PageKey::Character(c) if c.eq_ignore_ascii_case("x") => Some(PageEdit::Cut),
            PageKey::Character(c) if c.eq_ignore_ascii_case("v") => Some(PageEdit::Paste),
            _ => None,
        }
    }
}

/// Where Ferrite keeps its data: `$FERRITE_HOME`, else `~/.local/share/ferrite`
/// (`None` when neither can be resolved).
fn data_dir() -> Option<std::path::PathBuf> {
    match std::env::var_os("FERRITE_HOME").filter(|h| !h.is_empty()) {
        Some(home) => Some(std::path::PathBuf::from(home)),
        None => std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(|home| {
                std::path::PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("ferrite")
            }),
    }
}

/// The browser profile directory: cookies, HSTS and cached HTTP credentials
/// (written by Servo when it shuts down cleanly) and web storage live here, so
/// a login survives a restart. `None` runs with an in-memory profile.
#[must_use]
pub fn profile_dir() -> Option<std::path::PathBuf> {
    data_dir().map(|dir| dir.join("profile"))
}

/// The hash-chained network audit log every tab writes to (`$FERRITE_HOME/
/// audit/network.db`; the temp directory when no data directory resolves).
#[must_use]
pub fn audit_db_path() -> std::path::PathBuf {
    data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("audit")
        .join("network.db")
}

#[cfg(feature = "servo")]
pub use inner::{shutdown_engine, take_popup_sessions, HeadlessServoSession};

// ── Browser identity (the User-Agent sites see) ─────────────────────────────

/// The User-Agent the app asked for, read once when the engine is built. The
/// `FERRITE_USER_AGENT` environment variable still wins over it.
static USER_AGENT_OVERRIDE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Sets the User-Agent the engine will present, or `None` for Servo's own.
/// Only has an effect before the first page is opened: the engine is built
/// once per process, so a change takes effect on the next launch.
pub fn set_user_agent(user_agent: Option<String>) {
    let mut slot = USER_AGENT_OVERRIDE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = user_agent.filter(|ua| !ua.trim().is_empty());
}

#[cfg(feature = "servo")]
fn user_agent_override() -> Option<String> {
    USER_AGENT_OVERRIDE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Servo's own User-Agent for this platform (names `Servo/<version>`), or
/// `None` in a build without the engine.
#[must_use]
pub fn platform_default_user_agent() -> Option<String> {
    #[cfg(feature = "servo")]
    {
        Some(servo::Preferences::default().user_agent)
    }
    #[cfg(not(feature = "servo"))]
    {
        None
    }
}

/// A Firefox-compatible form of `servo_default`: the same platform and version,
/// with Servo's own `Servo/<version>` product token replaced by the `Gecko`
/// token every Firefox sends.
///
/// This is the long-standing compatibility convention (every mainstream browser
/// names an older engine in its User-Agent), and it is deliberately the *least*
/// change: nothing is made up. Servo already claims `Firefox/<n>`; many sites
/// and sign-in pages treat an unrecognized engine token as an unsupported
/// browser, and this removes that one signal. It does not make Ferrite Firefox,
/// and it does not change what the engine can do.
///
/// `None` when `servo_default` does not have the expected shape, so a changed
/// Servo default is never silently mangled.
#[must_use]
pub fn compatible_user_agent(servo_default: &str) -> Option<String> {
    let start = servo_default.find(" Servo/")?;
    let rest = &servo_default[start + 1..];
    let end = rest
        .find(' ')
        .map_or(servo_default.len(), |i| start + 1 + i);
    let compatible = format!(
        "{} Gecko/20100101{}",
        &servo_default[..start],
        &servo_default[end..]
    );
    compatible.contains(" Firefox/").then_some(compatible)
}

/// Commits a runtime-guard decision to the hash-chained audit log, so the
/// containment decision is verifiable after the fact: a `CapabilityDenied`
/// entry (`capability` = `guard.<primitive>`, `url` = the origin) for an action
/// the guard blocked, or a `CapabilityGranted` entry for a deviation the user
/// approved. Allowed, expected actions are not recorded (the log would be mostly
/// noise). Never fails the caller: a log that cannot be written is reported on
/// stderr, because refusing to block an action over a logging error would turn
/// a logging fault into a bypass. A no-op in a build without the Servo engine,
/// which has no log.
pub fn audit_guard_decision(primitive: &str, origin: Option<&str>, allowed: bool) {
    #[cfg(feature = "servo")]
    inner::audit_guard_decision(primitive, origin, allowed);
    #[cfg(not(feature = "servo"))]
    let _ = (primitive, origin, allowed);
}

/// Makes inline SVG icons keep their colours; see the script's own header.
#[cfg(feature = "servo")]
const SVG_COMPAT_JS: &str = include_str!("svg_compat.js");

/// The display's scale factor (physical pixels per CSS pixel at 100% zoom),
/// as `f32` bits. Pages are laid out in CSS pixels: a Retina display must tell
/// the engine its scale, or every page is laid out as if the screen were twice
/// as wide as it looks (tiny text, desktop layouts at 2560 px, and Google
/// results pinned to the left edge).
static DISPLAY_SCALE_BITS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0x3f80_0000); // 1.0

/// Records the display's scale; sessions apply it when created and when
/// [`HeadlessServoSession::apply_display_scale`] is called.
pub fn set_display_scale(scale: f32) {
    if scale.is_finite() && scale >= 0.5 {
        DISPLAY_SCALE_BITS.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// The display scale last recorded (1.0 until the UI knows it).
#[must_use]
pub fn display_scale() -> f32 {
    f32::from_bits(DISPLAY_SCALE_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Which WebGL a page gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebGlMode {
    /// None: `getContext('webgl')` returns null and a page falls back.
    Off,
    /// WebGL 1 only (the default).
    V1,
    /// WebGL 1 and 2.
    V2,
}

/// Which WebGL pages get, and the reason, from `FERRITE_WEBGL=off|webgl1|on|auto`
/// (default auto, which is WebGL 1 only). WebGL 2 is off by default because the
/// engine's WebGL 2 `drawBuffers`/`readBuffer` on the default framebuffer leave a
/// GL error pending, which on macOS made the next buffer swap fail and killed the
/// WebGL thread (servo/servo#48550, fixed upstream in #48620; the vendored
/// `servo-webgl` has the swap half of that fix). `off` is the way out if a page's
/// WebGL ever freezes it. `on` turns WebGL 2 back on.
#[must_use]
pub fn webgl_decision(setting: Option<&str>) -> (WebGlMode, &'static str) {
    match setting.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("on" | "1" | "true" | "yes" | "webgl2") => (WebGlMode::V2, "FERRITE_WEBGL=on"),
        Some("webgl1") => (WebGlMode::V1, "FERRITE_WEBGL=webgl1"),
        Some("off" | "0" | "false" | "no") => (WebGlMode::Off, "FERRITE_WEBGL=off"),
        _ => (
            WebGlMode::V1,
            "WebGL 2 is off by default; FERRITE_WEBGL=on turns it on",
        ),
    }
}

#[cfg(test)]
mod webgl_decision_tests {
    use super::{webgl_decision, WebGlMode};

    #[test]
    fn auto_is_webgl_1_only() {
        assert_eq!(webgl_decision(None).0, WebGlMode::V1);
        assert_eq!(webgl_decision(Some("")).0, WebGlMode::V1);
        assert_eq!(webgl_decision(Some("auto")).0, WebGlMode::V1);
        assert_eq!(webgl_decision(Some("webgl1")).0, WebGlMode::V1);
    }

    #[test]
    fn the_setting_turns_it_on_or_off() {
        assert_eq!(webgl_decision(Some(" ON ")).0, WebGlMode::V2);
        assert_eq!(webgl_decision(Some("webgl2")).0, WebGlMode::V2);
        assert_eq!(webgl_decision(Some("off")).0, WebGlMode::Off);
        assert_eq!(webgl_decision(Some("0")).0, WebGlMode::Off);
    }

    #[test]
    fn the_reason_names_the_setting_when_one_was_given() {
        assert_eq!(webgl_decision(Some("off")).1, "FERRITE_WEBGL=off");
        assert!(webgl_decision(None).1.contains("FERRITE_WEBGL=on"));
    }
}

/// How often a page's script thread is asked whether it is still there.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// How long an unanswered question can wait before the page counts as stalled.
const WATCH_STALL_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// What a [`ScriptWatch`] step wants done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEvent {
    /// Nothing.
    Quiet,
    /// Ask the page's script thread a trivial question now.
    SendProbe,
    /// The question has gone unanswered for this long: the page's script is busy
    /// or stuck. Scrolling and clicking go through that thread, so they stop too.
    Stalled(std::time::Duration),
    /// The page answered after having been reported stalled; it took this long.
    Recovered(std::time::Duration),
}

/// Watches whether a page's script thread still answers. A page that never
/// finishes loading and cannot be scrolled or clicked, but still paints, has a
/// script thread that is busy or stuck; nothing else in the app can tell that
/// from a page that is merely slow, so the log would say nothing.
#[derive(Debug, Default)]
pub struct ScriptWatch {
    sent_at: Option<std::time::Instant>,
    next_at: Option<std::time::Instant>,
    stalled: bool,
}

impl ScriptWatch {
    /// One look at the clock. `answered` is whether the outstanding question,
    /// if there is one, has been answered.
    pub fn step(&mut self, now: std::time::Instant, answered: bool) -> WatchEvent {
        if let Some(sent) = self.sent_at {
            let waited = now.saturating_duration_since(sent);
            if answered {
                self.sent_at = None;
                self.next_at = Some(now + WATCH_INTERVAL);
                return if std::mem::take(&mut self.stalled) {
                    WatchEvent::Recovered(waited)
                } else {
                    WatchEvent::Quiet
                };
            }
            if !self.stalled && waited >= WATCH_STALL_AFTER {
                self.stalled = true;
                return WatchEvent::Stalled(waited);
            }
            return WatchEvent::Quiet;
        }
        if self.next_at.is_none_or(|at| now >= at) {
            self.sent_at = Some(now);
            return WatchEvent::SendProbe;
        }
        WatchEvent::Quiet
    }

    /// Forget the outstanding question: the page navigated, so its answer will
    /// never come and the silence means nothing.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod script_watch_tests {
    use super::{ScriptWatch, WatchEvent};
    use std::time::{Duration, Instant};

    fn secs(base: Instant, s: u64) -> Instant {
        base + Duration::from_secs(s)
    }

    #[test]
    fn it_asks_at_once_then_every_two_seconds() {
        let t0 = Instant::now();
        let mut w = ScriptWatch::default();
        assert_eq!(w.step(t0, false), WatchEvent::SendProbe);
        assert_eq!(w.step(secs(t0, 1), true), WatchEvent::Quiet);
        assert_eq!(w.step(secs(t0, 2), false), WatchEvent::Quiet);
        assert_eq!(w.step(secs(t0, 3), false), WatchEvent::SendProbe);
    }

    #[test]
    fn a_question_unanswered_for_five_seconds_is_a_stall_reported_once() {
        let t0 = Instant::now();
        let mut w = ScriptWatch::default();
        assert_eq!(w.step(t0, false), WatchEvent::SendProbe);
        assert_eq!(w.step(secs(t0, 4), false), WatchEvent::Quiet);
        assert_eq!(
            w.step(secs(t0, 5), false),
            WatchEvent::Stalled(Duration::from_secs(5))
        );
        assert_eq!(w.step(secs(t0, 9), false), WatchEvent::Quiet);
    }

    #[test]
    fn answering_after_a_stall_reports_the_recovery_once() {
        let t0 = Instant::now();
        let mut w = ScriptWatch::default();
        let _ = w.step(t0, false);
        let _ = w.step(secs(t0, 6), false);
        assert_eq!(
            w.step(secs(t0, 8), true),
            WatchEvent::Recovered(Duration::from_secs(8))
        );
        assert_eq!(w.step(secs(t0, 9), false), WatchEvent::Quiet);
    }

    #[test]
    fn a_reset_forgets_the_question() {
        let t0 = Instant::now();
        let mut w = ScriptWatch::default();
        let _ = w.step(t0, false);
        w.reset();
        assert_eq!(w.step(secs(t0, 1), false), WatchEvent::SendProbe);
    }
}

/// Whether pages get `IntersectionObserver`, from
/// `FERRITE_INTERSECTION_OBSERVER=on|off` (default on). Servo ships it off, and
/// Ferrite turns it on because lazy-loading and framework routers call it. It is
/// also what runs the engine's containing-block walk on every frame, where a bug
/// (since fixed in `vendor/servo-layout`) froze the Google results page; `off`
/// is the quick way to find out whether a stuck page is that kind of problem.
#[must_use]
pub fn intersection_observer_enabled(setting: Option<&str>) -> bool {
    !matches!(
        setting.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "0" | "false" | "no")
    )
}

#[cfg(test)]
mod intersection_observer_setting_tests {
    use super::intersection_observer_enabled;

    #[test]
    fn it_is_on_unless_turned_off() {
        assert!(intersection_observer_enabled(None));
        assert!(intersection_observer_enabled(Some("")));
        assert!(intersection_observer_enabled(Some("on")));
        assert!(!intersection_observer_enabled(Some("off")));
        assert!(!intersection_observer_enabled(Some(" OFF ")));
        assert!(!intersection_observer_enabled(Some("0")));
    }
}

/// A process-wide counter for frame numbers, so two tabs never share one.
#[cfg(feature = "servo")]
pub(crate) fn next_frame_seq() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Defines web interfaces Servo lacks that real sites test for; see the
/// script's own header.
#[cfg(feature = "servo")]
const WEB_COMPAT_JS: &str = include_str!("web_compat.js");

/// CSS features the style engine ships off and that work once switched on:
/// `:has()`, `:nth-child(n of S)` and `@scope`, each checked in the real engine by
/// `examples/web_api_probe.rs`. Not here, with the reason in `docs/TO-DO.md`
/// T-312: container queries (the style engine parses `@container` only when built
/// for Gecko, so the rule is dropped whatever the preference says).
#[cfg(feature = "servo")]
fn apply_style_prefs() {
    stylo_static_prefs::set_pref!("layout.css.has-selector.enabled", true);
    stylo_static_prefs::set_pref!("layout.css.nth-child-of.enabled", true);
    stylo_static_prefs::set_pref!("layout.css.at-scope.enabled", true);
}

/// The Cache API (`caches`), built on IndexedDB; see the script's own header.
#[cfg(feature = "servo")]
const STORAGE_COMPAT_JS: &str = include_str!("storage_compat.js");

#[cfg(feature = "servo")]
mod inner {
    use std::cell::RefCell;
    use std::rc::Rc;

    use rustls::crypto::aws_lc_rs;
    use servo::{
        Code, DevicePoint, EditingActionEvent, InputEvent, Key, KeyState, KeyboardEvent, Location,
        Modifiers, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NamedKey,
        RenderingContext, Servo, ServoBuilder, ServoDelegate, SoftwareRenderingContext,
        WebViewBuilder, WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
    };
    use winit::dpi::PhysicalSize;

    /// The `Code` (physical key) that best matches a typed character. Pages
    /// mostly read `key`; `code` matters for shortcuts and games, so an
    /// unmapped character honestly reports `Unidentified` rather than a wrong
    /// physical key.
    fn code_for_char(c: char) -> Code {
        match c.to_ascii_lowercase() {
            'a' => Code::KeyA,
            'b' => Code::KeyB,
            'c' => Code::KeyC,
            'd' => Code::KeyD,
            'e' => Code::KeyE,
            'f' => Code::KeyF,
            'g' => Code::KeyG,
            'h' => Code::KeyH,
            'i' => Code::KeyI,
            'j' => Code::KeyJ,
            'k' => Code::KeyK,
            'l' => Code::KeyL,
            'm' => Code::KeyM,
            'n' => Code::KeyN,
            'o' => Code::KeyO,
            'p' => Code::KeyP,
            'q' => Code::KeyQ,
            'r' => Code::KeyR,
            's' => Code::KeyS,
            't' => Code::KeyT,
            'u' => Code::KeyU,
            'v' => Code::KeyV,
            'w' => Code::KeyW,
            'x' => Code::KeyX,
            'y' => Code::KeyY,
            'z' => Code::KeyZ,
            '0' => Code::Digit0,
            '1' => Code::Digit1,
            '2' => Code::Digit2,
            '3' => Code::Digit3,
            '4' => Code::Digit4,
            '5' => Code::Digit5,
            '6' => Code::Digit6,
            '7' => Code::Digit7,
            '8' => Code::Digit8,
            '9' => Code::Digit9,
            ' ' => Code::Space,
            '-' => Code::Minus,
            '=' => Code::Equal,
            '[' => Code::BracketLeft,
            ']' => Code::BracketRight,
            '\\' => Code::Backslash,
            ';' => Code::Semicolon,
            '\'' => Code::Quote,
            '`' => Code::Backquote,
            ',' => Code::Comma,
            '.' => Code::Period,
            '/' => Code::Slash,
            _ => Code::Unidentified,
        }
    }

    fn named_key(key: PageNamedKey) -> (NamedKey, Code) {
        match key {
            PageNamedKey::Enter => (NamedKey::Enter, Code::Enter),
            PageNamedKey::Backspace => (NamedKey::Backspace, Code::Backspace),
            PageNamedKey::Delete => (NamedKey::Delete, Code::Delete),
            PageNamedKey::Tab => (NamedKey::Tab, Code::Tab),
            PageNamedKey::Escape => (NamedKey::Escape, Code::Escape),
            PageNamedKey::ArrowUp => (NamedKey::ArrowUp, Code::ArrowUp),
            PageNamedKey::ArrowDown => (NamedKey::ArrowDown, Code::ArrowDown),
            PageNamedKey::ArrowLeft => (NamedKey::ArrowLeft, Code::ArrowLeft),
            PageNamedKey::ArrowRight => (NamedKey::ArrowRight, Code::ArrowRight),
            PageNamedKey::Home => (NamedKey::Home, Code::Home),
            PageNamedKey::End => (NamedKey::End, Code::End),
            PageNamedKey::PageUp => (NamedKey::PageUp, Code::PageUp),
            PageNamedKey::PageDown => (NamedKey::PageDown, Code::PageDown),
            PageNamedKey::Insert => (NamedKey::Insert, Code::Insert),
            PageNamedKey::F(n) => match n {
                1 => (NamedKey::F1, Code::F1),
                2 => (NamedKey::F2, Code::F2),
                3 => (NamedKey::F3, Code::F3),
                4 => (NamedKey::F4, Code::F4),
                5 => (NamedKey::F5, Code::F5),
                6 => (NamedKey::F6, Code::F6),
                7 => (NamedKey::F7, Code::F7),
                8 => (NamedKey::F8, Code::F8),
                9 => (NamedKey::F9, Code::F9),
                10 => (NamedKey::F10, Code::F10),
                11 => (NamedKey::F11, Code::F11),
                12 => (NamedKey::F12, Code::F12),
                _ => (NamedKey::Unidentified, Code::Unidentified),
            },
        }
    }

    // Servo's `opts` module uses a global singleton that panics if initialised
    // more than once per process.  We therefore create the `Servo` engine once
    // and share it (via `Clone`, which is a cheap `Rc` bump) across all tabs.
    thread_local! {
        static SERVO_ENGINE: RefCell<Option<Servo>> = const { RefCell::new(None) };
        /// The one set of injected page content (see `svg_compat.js`), shared
        /// by every tab.
        static USER_CONTENT: RefCell<Option<Rc<servo::UserContentManager>>> =
            const { RefCell::new(None) };
    }

    thread_local! {
        /// Tabs that pages opened (`window.open`, `target="_blank"`), waiting
        /// for the UI to adopt them.
        static POPUP_SESSIONS: RefCell<Vec<HeadlessServoSession>> = const { RefCell::new(Vec::new()) };
    }

    /// Takes the tabs pages have opened since the last call, oldest first.
    pub fn take_popup_sessions() -> Vec<HeadlessServoSession> {
        POPUP_SESSIONS.with(|q| std::mem::take(&mut *q.borrow_mut()))
    }

    thread_local! {
        static AUDIT_LOG: RefCell<Option<Rc<RefCell<PersistentAuditLog>>>> =
            const { RefCell::new(None) };
    }

    /// The one audit log every tab appends to. Tabs used to open (and delete)
    /// their own file each, so each tab's chain overwrote the last and nothing
    /// could read them back; one chain per process, at [`super::audit_db_path`],
    /// starts fresh on each launch.
    pub(super) fn audit_guard_decision(primitive: &str, origin: Option<&str>, allowed: bool) {
        let kind = if allowed {
            AuditEventKind::CapabilityGranted
        } else {
            AuditEventKind::CapabilityDenied
        };
        let result = shared_audit_log().and_then(|log| {
            log.borrow_mut()
                .append(
                    kind,
                    uuid::Uuid::new_v4(),
                    Some(format!("guard.{primitive}")),
                    origin.map(str::to_string),
                )
                .map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            eprintln!("[ferrite-session] audit write error (guard decision): {e}");
        }
    }

    fn shared_audit_log() -> Result<Rc<RefCell<PersistentAuditLog>>, String> {
        AUDIT_LOG.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(log) = slot.as_ref() {
                return Ok(log.clone());
            }
            let path = super::audit_db_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::remove_file(&path);
            let log =
                PersistentAuditLog::new(&path.to_string_lossy()).map_err(|e| e.to_string())?;
            let log = Rc::new(RefCell::new(log));
            *slot = Some(log.clone());
            Ok(log)
        })
    }

    /// Shuts the process-wide engine down cleanly, which is what makes Servo
    /// write the profile (cookies, HSTS, credentials) to disk. Every
    /// [`HeadlessServoSession`] must have been dropped first: each holds a
    /// handle to the engine, and shutdown happens when the last one goes.
    pub fn shutdown_engine() {
        // The content manager talks to the engine when it is dropped, so it
        // goes first.
        USER_CONTENT.with(|cell| drop(cell.borrow_mut().take()));
        SERVO_ENGINE.with(|cell| drop(cell.borrow_mut().take()));
    }

    /// Makes the rendering context for one tab: always the CPU (software)
    /// renderer. A GPU renderer was tried and removed: on an Apple M1 with it,
    /// Google never finished loading and could not be scrolled or clicked, while
    /// the CPU renderer worked (`docs/DECISIONS.md` ADR-021, `docs/TO-DO.md`
    /// T-281 and T-305). Says once, in the log, what it uses.
    fn make_rendering_context(size: PhysicalSize<u32>) -> Result<Rc<dyn RenderingContext>, String> {
        static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[ferrite-render] CPU rendering (software); there is no GPU renderer");
            if std::env::var_os("FERRITE_RENDERER").is_some() {
                eprintln!(
                    "[ferrite-render] FERRITE_RENDERER is ignored: there is only the CPU renderer"
                );
            }
        }
        SoftwareRenderingContext::new(size)
            .map(|cpu| Rc::new(cpu) as Rc<dyn RenderingContext>)
            .map_err(|e| format!("SoftwareRenderingContext: {e:?}"))
    }

    /// The page content every tab is given: the SVG compatibility script.
    fn user_content_manager(servo: &Servo) -> Rc<servo::UserContentManager> {
        USER_CONTENT.with(|cell| {
            cell.borrow_mut()
                .get_or_insert_with(|| {
                    let manager = servo::UserContentManager::new(servo);
                    manager.add_script(Rc::new(servo::UserScript::from(super::SVG_COMPAT_JS)));
                    manager.add_script(Rc::new(servo::UserScript::from(super::WEB_COMPAT_JS)));
                    manager.add_script(Rc::new(servo::UserScript::from(super::STORAGE_COMPAT_JS)));
                    Rc::new(manager)
                })
                .clone()
        })
    }

    /// Return (or lazily create) the process-wide `Servo` engine.
    ///
    /// The first call builds the engine; every subsequent call clones the `Rc`
    /// wrapper, so `opts::initialize_options` is only ever called once.
    fn get_or_init_servo() -> Servo {
        SERVO_ENGINE.with(|cell| {
            let mut guard = cell.borrow_mut();
            if guard.is_none() {
                let mut opts = servo::Opts::default();
                if let Some(dir) = super::profile_dir() {
                    match std::fs::create_dir_all(&dir) {
                        Ok(()) => opts.config_dir = Some(dir),
                        Err(e) => eprintln!(
                            "[ferrite-session] cannot create the profile directory {dir:?}: {e}; \
                             logins will not survive a restart"
                        ),
                    }
                }
                // Sites that keep a login in IndexedDB or the async cookie API
                // (Google's sign-in among them) need both. IntersectionObserver
                // is what lazy-loading and framework routers (Next.js among
                // them) reach for; Servo ships it off by default. (Web Crypto
                // is a compile-time feature of the `servo` crate — see the
                // workspace Cargo.toml — its `dom_crypto_subtle_enabled`
                // preference is already on.)
                let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
                let (webgl_mode, webgl_why) =
                    super::webgl_decision(std::env::var("FERRITE_WEBGL").ok().as_deref());
                eprintln!(
                    "[ferrite-webgl] {} ({webgl_why})",
                    match webgl_mode {
                        super::WebGlMode::Off => "off",
                        super::WebGlMode::V1 => "WebGL 1 only",
                        super::WebGlMode::V2 => "WebGL 1 and 2",
                    }
                );
                let observer_on = super::intersection_observer_enabled(
                    std::env::var("FERRITE_INTERSECTION_OBSERVER")
                        .ok()
                        .as_deref(),
                );
                eprintln!(
                    "[ferrite-observer] IntersectionObserver {}",
                    if observer_on { "on" } else { "off" }
                );
                let mut prefs = servo::Preferences {
                    dom_indexeddb_enabled: true,
                    dom_cookiestore_enabled: true,
                    dom_intersection_observer_enabled: observer_on,
                    // Web APIs that sign-in and anti-abuse scripts probe for
                    // (and that ordinary sites use) and that Servo ships off
                    // by default. Each is a real implementation being turned
                    // on, not a stub: see `examples/web_api_probe.rs`.
                    dom_permissions_enabled: true,
                    dom_notification_enabled: true,
                    dom_async_clipboard_enabled: true,
                    dom_webgl2_enabled: webgl_mode == super::WebGlMode::V2,
                    // No runtime off-switch exists for WebGL 1 (it is a compile-time
                    // feature); forcing context creation to fail makes
                    // `getContext('webgl')` return null, which pages handle.
                    webgl_testing_context_creation_error: webgl_mode == super::WebGlMode::Off,
                    // Seen failing on GitHub (`e.adoptedStyleSheets is undefined`,
                    // dozens of times while its components start) and Google
                    // (`document.fonts.load is not a function`): both ship off.
                    // Each was checked to *work* before being kept: a half-built
                    // feature is worse than a missing one, because pages detect
                    // it and skip their fallback. Tried and left off: container
                    // queries (the property parses but `@container` rules are
                    // dropped), writing modes (the layout engine panics on a page
                    // mixing horizontal and vertical text) and multi-column
                    // layout (no effect). See `docs/TO-DO.md` T-264.
                    dom_adoptedstylesheet_enabled: true,
                    dom_fontface_enabled: true,
                    layout_css_attr_enabled: true,
                    // Swept one at a time against a battery page (every
                    // default-off boolean preference, each alone, then the
                    // survivors together). Kept because each exposes a working
                    // API that sites feature-detect and that fails soft:
                    // credentials/wake lock reject rather than hang, the rest
                    // resolve. Left off, with the reason, in `docs/TO-DO.md`
                    // T-265: WebRTC (`getUserMedia` resolves with no consent
                    // prompt), geolocation (the request never settles),
                    // service workers (a non-script response still "registers"),
                    // Web Animations (`animate()` returns no `finished`).
                    dom_credential_management_enabled: true,
                    dom_wakelock_enabled: true,
                    dom_storage_manager_api_enabled: true,
                    dom_offscreen_canvas_enabled: true,
                    dom_sanitizer_enabled: true,
                    dom_visual_viewport_enabled: true,
                    dom_exec_command_enabled: true,
                    // Style and layout fan out over this many threads; the
                    // engine's default is 3 whatever the machine (its own
                    // source calls that a TODO). WebRender's raster pool and
                    // the worker pools are capped the same way, by the cores
                    // there are.
                    layout_threads: cores.clamp(3, 8) as i64,
                    thread_pool_webrender_workers_max: cores.clamp(4, 8) as u64,
                    thread_pool_workers_max: cores.clamp(4, 8) as u64,
                    ..servo::Preferences::default()
                };
                // Some sites (Google's sign-in among them) decide whether a
                // browser is acceptable from its user-agent string. Servo's own
                // default names Servo; set FERRITE_USER_AGENT to present another.
                if let Some(ua) = std::env::var("FERRITE_USER_AGENT")
                    .ok()
                    .filter(|ua| !ua.trim().is_empty())
                    .or_else(super::user_agent_override)
                {
                    prefs.user_agent = ua;
                }
                let servo = ServoBuilder::default()
                    .opts(opts)
                    .preferences(prefs)
                    .build();
                super::apply_style_prefs();
                *guard = Some(servo);
            }
            guard.as_ref().unwrap().clone()
        })
    }

    use ferrite_audit_log::{AuditEventKind, PersistentAuditLog};

    use super::{LoadStatus, PageEdit, PageKey, PageKeyEvent, PageNamedKey};

    /// Converts a Servo-decoded favicon [`servo::Image`] to raw, straight
    /// (non-premultiplied) RGBA8 bytes — the format
    /// `iced_widget::image::Handle::from_rgba` expects, matching the
    /// conversion `get_frame()` already relies on `read_to_image` to do for
    /// the main page surface.
    ///
    /// Favicons can arrive in any of Servo's decoded [`servo::PixelFormat`]
    /// variants depending on the source image (a `.ico` with a paletted or
    /// grayscale frame, a plain PNG, ...), not just RGBA8 — this is the one
    /// place in `ferrite-servo` that has to handle the full set rather than
    /// assuming a single decoder output format.
    #[allow(clippy::chunks_exact_to_as_chunks)] // chunks_exact reads clearer here and works on older toolchains
    fn favicon_to_rgba8(
        width: u32,
        height: u32,
        format: servo::PixelFormat,
        data: &[u8],
    ) -> Vec<u8> {
        let pixel_count = (width as usize) * (height as usize);
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        match format {
            servo::PixelFormat::RGBA8 => rgba.extend_from_slice(data),
            servo::PixelFormat::BGRA8 => {
                for px in data.chunks_exact(4) {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                }
            }
            servo::PixelFormat::RGB8 => {
                for px in data.chunks_exact(3) {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            servo::PixelFormat::KA8 => {
                for px in data.chunks_exact(2) {
                    let luminance = px[0];
                    rgba.extend_from_slice(&[luminance, luminance, luminance, px[1]]);
                }
            }
            servo::PixelFormat::K8 => {
                for &luminance in data {
                    rgba.extend_from_slice(&[luminance, luminance, luminance, 255]);
                }
            }
        }
        rgba
    }

    // -------------------------------------------------------------------------
    // Servo delegate (global browser-level callbacks — all no-ops)
    // -------------------------------------------------------------------------

    /// Latest favicon (width, height, RGBA8) shared between Servo's delegate and the session.
    type SharedFavicon = Rc<std::cell::RefCell<Option<(u32, u32, Vec<u8>)>>>;

    /// What the engine is waiting on a person for, with a plain-data view of
    /// it for the UI.
    type SharedControl =
        Rc<std::cell::RefCell<Option<(crate::diag::PageControl, servo::EmbedderControl)>>>;

    /// `#rrggbb` of an engine colour.
    fn hex_of(c: servo::RgbColor) -> String {
        format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
    }

    fn rect_of(r: servo::DeviceIntRect) -> crate::diag::DeviceRect {
        crate::diag::DeviceRect {
            x: r.min.x as f32,
            y: r.min.y as f32,
            width: r.width() as f32,
            height: r.height() as f32,
        }
    }

    /// The UI-facing description of an engine control, or `None` for one that
    /// needs no UI (an input-method request).
    fn describe_control(control: &servo::EmbedderControl) -> Option<crate::diag::PageControl> {
        use crate::diag::{DialogKind, MenuItemView, PageControl, SelectOptionView};
        Some(match control {
            servo::EmbedderControl::SelectElement(select) => {
                let chosen = select.selected_options();
                let mut options = Vec::new();
                for entry in select.options() {
                    match entry {
                        servo::SelectElementOptionOrOptgroup::Option(o) => {
                            options.push(SelectOptionView {
                                index: o.id,
                                label: o.label.clone(),
                                disabled: o.is_disabled,
                                selected: chosen.contains(&o.id),
                                group: None,
                            });
                        }
                        servo::SelectElementOptionOrOptgroup::Optgroup {
                            label,
                            options: group,
                        } => {
                            for o in group {
                                options.push(SelectOptionView {
                                    index: o.id,
                                    label: o.label.clone(),
                                    disabled: o.is_disabled,
                                    selected: chosen.contains(&o.id),
                                    group: Some(label.clone()),
                                });
                            }
                        }
                    }
                }
                PageControl::Select {
                    options,
                    multiple: select.allow_select_multiple(),
                    anchor: rect_of(select.position()),
                }
            }
            servo::EmbedderControl::SimpleDialog(dialog) => match dialog {
                servo::SimpleDialog::Alert(a) => PageControl::Dialog {
                    kind: DialogKind::Alert,
                    message: a.message().to_string(),
                    default: String::new(),
                },
                servo::SimpleDialog::Confirm(c) => PageControl::Dialog {
                    kind: DialogKind::Confirm,
                    message: c.message().to_string(),
                    default: String::new(),
                },
                servo::SimpleDialog::Prompt(p) => PageControl::Dialog {
                    kind: DialogKind::Prompt,
                    message: p.message().to_string(),
                    default: p.current_value().to_string(),
                },
            },
            servo::EmbedderControl::FilePicker(f) => PageControl::File {
                multiple: f.allow_select_multiple(),
                accept: f.filter_patterns().iter().map(|p| p.0.clone()).collect(),
            },
            servo::EmbedderControl::ColorPicker(c) => PageControl::Color {
                current: c
                    .current_color()
                    .map_or_else(|| "#000000".to_string(), hex_of),
                anchor: rect_of(c.position()),
            },
            servo::EmbedderControl::ContextMenu(m) => {
                let mut next = 0usize;
                let items = m
                    .items()
                    .iter()
                    .map(|item| match item {
                        servo::ContextMenuItem::Item { label, enabled, .. } => {
                            let index = next;
                            next += 1;
                            MenuItemView {
                                index: Some(index),
                                label: label.clone(),
                                enabled: *enabled,
                            }
                        }
                        servo::ContextMenuItem::Separator => MenuItemView {
                            index: None,
                            label: String::new(),
                            enabled: false,
                        },
                    })
                    .collect();
                PageControl::Menu {
                    items,
                    anchor: rect_of(m.position()),
                }
            }
            servo::EmbedderControl::InputMethod(_) => return None,
        })
    }

    fn cursor_of(cursor: servo::Cursor) -> crate::diag::PageCursor {
        use crate::diag::PageCursor as C;
        use servo::Cursor as S;
        match cursor {
            S::None => C::Hidden,
            S::Pointer | S::Alias => C::Pointer,
            S::Text | S::VerticalText => C::Text,
            S::Crosshair | S::Cell => C::Crosshair,
            S::Grab => C::Grab,
            S::Grabbing => C::Grabbing,
            S::Move | S::AllScroll => C::Move,
            S::NotAllowed | S::NoDrop => C::NotAllowed,
            S::Wait | S::Progress => C::Wait,
            S::Help => C::Help,
            S::ZoomIn => C::ZoomIn,
            S::ZoomOut => C::ZoomOut,
            S::EResize | S::WResize | S::EwResize | S::ColResize => C::ResizeHorizontal,
            S::NResize | S::SResize | S::NsResize | S::RowResize => C::ResizeVertical,
            S::NeResize | S::SwResize | S::NeswResize => C::ResizeDiagonalUp,
            S::NwResize | S::SeResize | S::NwseResize => C::ResizeDiagonalDown,
            S::Default | S::ContextMenu | S::Copy => C::Default,
        }
    }

    struct HeadlessServoDelegate;
    impl ServoDelegate for HeadlessServoDelegate {}

    // -------------------------------------------------------------------------
    // WebView delegate
    // -------------------------------------------------------------------------

    struct HeadlessDelegate {
        audit_log: Rc<std::cell::RefCell<PersistentAuditLog>>,
        /// Shared load status — written by delegate callbacks, read by session in `spin()`.
        load_status: Rc<std::cell::RefCell<LoadStatus>>,
        /// Shared current URL — written by delegate callbacks, read by session in `spin()`.
        current_url: Rc<std::cell::RefCell<String>>,
        /// Servo's own native session-history list for this tab
        /// (`WebViewDelegate::notify_history_changed`) — `(entries, current
        /// index)`, written whenever the WebView's history changes (a real
        /// navigation, `go_back`/`go_forward`), read back by
        /// `HeadlessServoSession::sync_and_read()`. Replaces the old
        /// `nav_count`-based approximation this field's predecessor used
        /// for `can_go_back()` (a bare "how many `Complete` events have we
        /// seen" counter that could never tell forward-history apart at
        /// all — `can_go_forward()` simply returned `false`
        /// unconditionally). See `HeadlessServoSession::can_go_back`/
        /// `can_go_forward`/`history` for what reads this.
        history: Rc<std::cell::RefCell<(Vec<String>, usize)>>,
        /// Shared page title — written by `notify_page_title_changed`, read in `spin()`.
        page_title: Rc<std::cell::RefCell<Option<String>>>,
        /// Accumulated JS console errors — appended by `notify_console_message`, drained by
        /// `HeadlessServoSession::take_console_errors()`.
        console_errors: Rc<std::cell::RefCell<Vec<String>>>,
        /// Shared favicon cell — written by `notify_favicon_changed`, read in
        /// `sync_and_read()`. `(width, height, rgba_bytes)`, already
        /// converted from whatever `servo::PixelFormat` the page's icon
        /// decoded to.
        favicon: SharedFavicon,
        /// Every console message, any level (the DevTools-style console).
        console_log: Rc<std::cell::RefCell<std::collections::VecDeque<crate::diag::ConsoleEntry>>>,
        /// Every request the engine announced for this tab.
        net_log: Rc<std::cell::RefCell<std::collections::VecDeque<crate::diag::NetEvent>>>,
        /// The one thing the page is waiting on a person for.
        control: SharedControl,
        /// The pointer the page asked for.
        cursor: Rc<std::cell::Cell<crate::diag::PageCursor>>,
        /// Set when the page's script thread or process died.
        crash: Rc<std::cell::RefCell<Option<crate::diag::CrashNote>>>,
        /// Set when the engine says it has a new frame; cleared when the
        /// session has read that frame back. Reading pixels is by far the most
        /// expensive thing a tick does, so an unchanged page costs nothing.
        frame_ready: Rc<std::cell::Cell<bool>>,
        /// The throttle state the session last asked for (see `set_active`).
        /// The engine applies a throttle to the page that is current when it
        /// arrives and does not carry it to the next page loaded in the same tab,
        /// so a background tab that navigates must be told again.
        throttle: Rc<std::cell::Cell<Option<bool>>>,
    }

    impl WebViewDelegate for HeadlessDelegate {
        /// Ferrite has no permission prompt yet, so every request Servo
        /// forwards (notifications, persistent storage, ...) is refused outright
        /// rather than left unanswered. The one exception is the screen wake lock:
        /// it exposes nothing about the user and costs nothing, so a page that
        /// asks (Speedometer, video players) gets it instead of an error in its
        /// console. The engine's wake-lock backend does nothing on the operating
        /// system, so granting it does not actually keep the screen awake.
        /// Servo does not forward geolocation or getUserMedia here at all, which
        /// is why those stay switched off (docs/TO-DO.md T-265).
        fn request_permission(&self, _webview: servo::WebView, request: servo::PermissionRequest) {
            if matches!(
                request.feature(),
                servo::PermissionFeature::ScreenWakeLock(_)
            ) {
                request.allow();
                return;
            }
            eprintln!(
                "[ferrite-session] denied permission request: {:?}",
                request.feature()
            );
            request.deny();
        }

        /// A `<select>`, an `alert()`/`confirm()`/`prompt()`, a file or colour
        /// picker or a context menu: kept for the UI to show. A newer one
        /// replaces (and so dismisses) an older one.
        fn show_embedder_control(&self, _webview: servo::WebView, control: servo::EmbedderControl) {
            match describe_control(&control) {
                Some(view) => *self.control.borrow_mut() = Some((view, control)),
                None => drop(control),
            }
        }

        fn hide_embedder_control(&self, _webview: servo::WebView, id: servo::EmbedderControlId) {
            let mut slot = self.control.borrow_mut();
            if slot.as_ref().is_some_and(|(_, c)| c.id() == id) {
                *slot = None;
            }
        }

        fn notify_cursor_changed(&self, _webview: servo::WebView, cursor: servo::Cursor) {
            self.cursor.set(cursor_of(cursor));
        }

        /// A page's script thread panicked. The page stops answering; say so
        /// instead of leaving it looking merely slow.
        fn notify_crashed(
            &self,
            _webview: servo::WebView,
            reason: String,
            backtrace: Option<String>,
        ) {
            eprintln!("[ferrite-engine] a page crashed: {reason}");
            if let Some(bt) = &backtrace {
                eprintln!("{bt}");
            }
            *self.crash.borrow_mut() = Some(crate::diag::CrashNote {
                at_ms: crate::diag::now_ms(),
                reason,
                backtrace,
            });
        }

        fn notify_new_frame_ready(&self, webview: servo::WebView) {
            webview.paint();
            self.frame_ready.set(true);
        }

        fn notify_load_status_changed(&self, webview: servo::WebView, status: servo::LoadStatus) {
            // One line per step a page's load takes, so a page that never
            // finishes shows how far it got (started, head parsed, complete).
            eprintln!(
                "[ferrite-load] {status:?} {}",
                webview
                    .url()
                    .map_or_else(|| "<unknown>".to_string(), |u| u.to_string())
            );
            // A new page in a background tab starts un-throttled: tell the engine
            // again now that the page (and its window) exists.
            if status == servo::LoadStatus::HeadParsed && self.throttle.get() == Some(true) {
                webview.set_throttled(true);
            }
            match status {
                servo::LoadStatus::Complete => {
                    let url = webview
                        .url()
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    *self.current_url.borrow_mut() = url;
                    *self.load_status.borrow_mut() = LoadStatus::Complete;
                }
                _ => {
                    // Treat all non-Complete statuses (Loading, Failed, etc.) as Loading.
                    *self.load_status.borrow_mut() = LoadStatus::Loading;
                }
            }
        }

        fn notify_page_title_changed(&self, _webview: servo::WebView, title: Option<String>) {
            *self.page_title.borrow_mut() = title;
        }

        fn notify_favicon_changed(&self, webview: servo::WebView) {
            // The new image isn't passed as a parameter — WebViewDelegate's
            // own docs point at `WebView::favicon()` for it.
            *self.favicon.borrow_mut() = webview.favicon().map(|image| {
                let rgba = favicon_to_rgba8(image.width, image.height, image.format, image.data());
                (image.width, image.height, rgba)
            });
        }

        /// Servo's own native session-history hook — fires with the whole
        /// history list and the current index on every change (a real
        /// navigation, `go_back`/`go_forward`), giving this crate real
        /// forward/back history per tab for free rather than needing to
        /// hand-track it from URL-change events. `entries` is stored as
        /// `String` (via `Url::to_string()`) rather than the `url::Url`
        /// type itself, matching how `current_url`/`shared_url` already
        /// store URLs elsewhere in this same struct.
        fn notify_history_changed(
            &self,
            _webview: servo::WebView,
            entries: Vec<url::Url>,
            current: usize,
        ) {
            let urls: Vec<String> = entries.iter().map(url::Url::to_string).collect();
            *self.history.borrow_mut() = (urls, current);
        }

        fn show_console_message(
            &self,
            _webview: servo::WebView,
            level: servo::ConsoleLogLevel,
            message: String,
        ) {
            use crate::diag::{push_bounded, ConsoleEntry, ConsoleLevel};
            let mapped = match level {
                servo::ConsoleLogLevel::Debug | servo::ConsoleLogLevel::Trace => {
                    ConsoleLevel::Debug
                }
                servo::ConsoleLogLevel::Log | servo::ConsoleLogLevel::Dir => ConsoleLevel::Log,
                servo::ConsoleLogLevel::Info => ConsoleLevel::Info,
                servo::ConsoleLogLevel::Warn => ConsoleLevel::Warn,
                servo::ConsoleLogLevel::Error => ConsoleLevel::Error,
            };
            // The error list the app has always printed stays errors only.
            if mapped == ConsoleLevel::Error {
                self.console_errors.borrow_mut().push(message.clone());
            }
            push_bounded(
                &mut self.console_log.borrow_mut(),
                ConsoleEntry {
                    at_ms: crate::diag::now_ms(),
                    level: mapped,
                    source: crate::diag::source_of(&message),
                    message,
                },
            );
        }

        /// A page asked for a new WebView (`window.open`, `target="_blank"`,
        /// a sign-in popup). Build it as a tab of its own and queue it for the
        /// UI to adopt (`take_popup_sessions`); ignoring the request would
        /// silently break every such link and most "Sign in with ..." flows.
        fn request_create_new(
            &self,
            _parent: servo::WebView,
            request: servo::CreateNewWebViewRequest,
        ) {
            match HeadlessServoSession::assemble(1280, 700, |servo, rendering_context, delegate| {
                request
                    .builder(rendering_context)
                    .delegate(delegate)
                    .user_content_manager(user_content_manager(servo))
                    .build()
            }) {
                Ok(session) => POPUP_SESSIONS.with(|q| q.borrow_mut().push(session)),
                Err(e) => eprintln!("[ferrite-session] cannot open a page-requested tab: {e}"),
            }
        }

        fn load_web_resource(&self, _webview: servo::WebView, load: servo::WebResourceLoad) {
            let url = load.request().url.to_string();
            crate::diag::push_bounded(
                &mut self.net_log.borrow_mut(),
                crate::diag::NetEvent {
                    at_ms: crate::diag::now_ms(),
                    method: load.request().method.to_string(),
                    url: url.chars().take(2000).collect(),
                    kind: format!("{:?}", load.request().destination),
                    is_main_frame: load.request().is_for_main_frame,
                },
            );
            if let Err(e) = self.audit_log.borrow_mut().append(
                AuditEventKind::CapabilityGranted,
                uuid::Uuid::new_v4(),
                Some("network.fetch".to_string()),
                Some(url),
            ) {
                eprintln!("[ferrite-session] audit write error: {}", e);
            }
            // Drop load without intercepting — Servo fetches normally.
        }
    }

    // -------------------------------------------------------------------------
    // HeadlessServoSession
    // -------------------------------------------------------------------------

    /// A self-contained, headless Servo browsing session.
    ///
    /// Rendering uses `SoftwareRenderingContext` (CPU rasteriser, no GPU or
    /// window handle required).  Callers drive it by calling `spin()` each tick
    /// and read rendered frames via `get_frame()`.
    ///
    /// Load status and current URL are synced from `WebViewDelegate` callbacks
    /// on each `spin()` call and exposed via `load_status()` and `current_url()`.
    pub struct HeadlessServoSession {
        servo: servo::Servo,
        webview: servo::WebView,
        rendering_context: Rc<dyn RenderingContext>,
        width: u32,
        height: u32,
        /// Cached last frame as raw RGBA bytes (width × height × 4).
        last_frame: Option<std::sync::Arc<Vec<u8>>>,
        /// Unique (process-wide) number of `last_frame`; changes exactly when
        /// the pixels do, so a caller can tell "same picture" without
        /// comparing or copying them.
        frame_seq: u64,
        /// Shared with the delegate: the engine has painted something new.
        frame_ready: Rc<std::cell::Cell<bool>>,
        /// Most recently synced load status (updated in `spin()`).
        last_load_status: LoadStatus,
        /// Most recently synced current URL (updated in `spin()`).
        current_url: String,
        /// Shared load status cell — written by `HeadlessDelegate`, read in `spin()`.
        shared_load_status: Rc<std::cell::RefCell<LoadStatus>>,
        /// Shared URL cell — written by `HeadlessDelegate`, read in `spin()`.
        shared_url: Rc<std::cell::RefCell<String>>,
        /// Shared session-history cell (`(entries, current_index)`) —
        /// written by `HeadlessDelegate::notify_history_changed`, read in
        /// `sync_and_read()`. See that field's doc comment on
        /// `HeadlessDelegate` for why this replaced `nav_count`.
        shared_history: Rc<std::cell::RefCell<(Vec<String>, usize)>>,
        /// Most recently synced session history (updated in `sync_and_read()`).
        last_history: (Vec<String>, usize),
        /// Shared page title cell — written by `HeadlessDelegate`, read in `spin()`.
        shared_page_title: Rc<std::cell::RefCell<Option<String>>>,
        /// Most recently synced page title (updated in `spin()`).
        last_page_title: Option<String>,
        /// JS console errors shared with `HeadlessDelegate` — accumulated until drained.
        shared_console_errors: Rc<std::cell::RefCell<Vec<String>>>,
        /// Shared favicon cell — written by `HeadlessDelegate`, read in `sync_and_read()`.
        shared_favicon: SharedFavicon,
        shared_control: SharedControl,
        shared_cursor: Rc<std::cell::Cell<crate::diag::PageCursor>>,
        shared_crash: Rc<std::cell::RefCell<Option<crate::diag::CrashNote>>>,
        /// Console messages of every level, drained by `take_console_entries`.
        shared_console_log:
            Rc<std::cell::RefCell<std::collections::VecDeque<crate::diag::ConsoleEntry>>>,
        /// Requests made, drained by `take_net_events`.
        shared_net_log: Rc<std::cell::RefCell<std::collections::VecDeque<crate::diag::NetEvent>>>,
        /// Most recently synced favicon (updated in `sync_and_read()`).
        last_favicon: Option<(u32, u32, Vec<u8>)>,
        /// Whether the page's script thread still answers (see [`super::ScriptWatch`]).
        script_watch: super::ScriptWatch,
        /// Set by the callback of the outstanding probe.
        probe_answered: Rc<std::cell::Cell<bool>>,
        /// The throttle state last sent to the engine (`None` before the first).
        throttle_sent: Rc<std::cell::Cell<Option<bool>>>,
    }

    impl HeadlessServoSession {
        /// Create a new headless session with a `width × height` render surface.
        ///
        /// Opens `$TMPDIR/ferrite_servo_session.db` for the audit log and mints
        /// a wildcard `NetworkFetch` token (3600 s TTL).
        ///
        /// On Windows, Servo's EGL/surfman backend requires ANGLE (`libEGL.dll`,
        /// `libGLESv2.dll`) in the executable's directory or on PATH.  If those
        /// DLLs are absent the EGL bindings panic at function-pointer load time.
        /// This constructor wraps the entire initialisation in `catch_unwind` so
        /// the panic is converted to an `Err` rather than crashing the process —
        /// the caller can then fall back to a no-Servo UI gracefully.
        pub fn new(width: u32, height: u32) -> Result<Self, String> {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::new_inner(width, height)
            }))
            .unwrap_or_else(|payload| {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    format!("Servo init panic: {}", s)
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    format!("Servo init panic: {}", s)
                } else {
                    "Servo init panic: EGL not available (ANGLE DLLs missing on Windows?)"
                        .to_string()
                };
                Err(msg)
            })
        }

        fn new_inner(width: u32, height: u32) -> Result<Self, String> {
            let session = Self::assemble(width, height, |servo, rendering_context, delegate| {
                WebViewBuilder::new(servo, rendering_context)
                    .delegate(delegate)
                    .user_content_manager(user_content_manager(servo))
                    .url(url::Url::parse("about:blank").unwrap())
                    .build()
            })?;
            // Let the tab's initial about:blank finish loading before handing
            // the session out: a `navigate` issued while it is still in flight
            // is lost to it (the tab stays blank), which is what a tab opened
            // and immediately pointed somewhere — by the agent's `open_tab`,
            // say — would hit.
            let settle_deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(1500);
            while *session.shared_load_status.borrow() != LoadStatus::Complete
                && std::time::Instant::now() < settle_deadline
            {
                session.servo.spin_event_loop();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(session)
        }

        /// Builds a session around a WebView made by `make` — the shared part
        /// of opening a fresh tab and of accepting a page-opened one
        /// (`window.open`, `target="_blank"`).
        fn assemble(
            width: u32,
            height: u32,
            make: impl FnOnce(&Servo, Rc<dyn RenderingContext>, Rc<HeadlessDelegate>) -> servo::WebView,
        ) -> Result<Self, String> {
            // ── rustls crypto provider ─────────────────────────────────────
            let _ = aws_lc_rs::default_provider().install_default();

            // ── Audit log ──────────────────────────────────────────────────
            let audit_log = shared_audit_log()?;

            // ── Shared delegate ↔ session state ────────────────────────────
            let throttle_sent: Rc<std::cell::Cell<Option<bool>>> = Rc::default();
            let shared_load_status = Rc::new(std::cell::RefCell::new(LoadStatus::Loading));
            let shared_url = Rc::new(std::cell::RefCell::new("about:blank".to_string()));
            let shared_history: Rc<std::cell::RefCell<(Vec<String>, usize)>> =
                Rc::new(std::cell::RefCell::new((Vec::new(), 0)));
            let shared_page_title: Rc<std::cell::RefCell<Option<String>>> =
                Rc::new(std::cell::RefCell::new(None));
            let shared_console_errors: Rc<std::cell::RefCell<Vec<String>>> =
                Rc::new(std::cell::RefCell::new(Vec::new()));
            let shared_favicon: SharedFavicon = Rc::new(std::cell::RefCell::new(None));
            let frame_ready = Rc::new(std::cell::Cell::new(true));
            let shared_control: SharedControl = Rc::default();
            let shared_cursor: Rc<std::cell::Cell<crate::diag::PageCursor>> = Rc::default();
            let shared_crash: Rc<std::cell::RefCell<Option<crate::diag::CrashNote>>> =
                Rc::default();
            let shared_console_log: Rc<
                std::cell::RefCell<std::collections::VecDeque<crate::diag::ConsoleEntry>>,
            > = Rc::default();
            let shared_net_log: Rc<
                std::cell::RefCell<std::collections::VecDeque<crate::diag::NetEvent>>,
            > = Rc::default();

            // ── Rendering context ──────────────────────────────────────────
            let rendering_context = make_rendering_context(PhysicalSize { width, height })?;
            rendering_context
                .make_current()
                .map_err(|e| format!("make_current: {:?}", e))?;

            // ── Servo engine ───────────────────────────────────────────────
            // Reuse the process-wide Servo instance (opts can only be
            // initialised once; subsequent tabs clone the Rc handle).
            let servo = get_or_init_servo();
            servo.set_delegate(Rc::new(HeadlessServoDelegate));

            // ── WebView ───────────────────────────────────────────────────
            let delegate = Rc::new(HeadlessDelegate {
                audit_log,
                load_status: shared_load_status.clone(),
                current_url: shared_url.clone(),
                history: shared_history.clone(),
                page_title: shared_page_title.clone(),
                console_errors: shared_console_errors.clone(),
                favicon: shared_favicon.clone(),
                control: shared_control.clone(),
                cursor: shared_cursor.clone(),
                crash: shared_crash.clone(),
                console_log: shared_console_log.clone(),
                net_log: shared_net_log.clone(),
                frame_ready: frame_ready.clone(),
                throttle: throttle_sent.clone(),
            });
            let webview = make(&servo, rendering_context.clone(), delegate);

            webview.set_hidpi_scale_factor(euclid::Scale::new(super::display_scale()));
            webview.resize(PhysicalSize { width, height });
            servo.spin_event_loop();

            Ok(Self {
                servo,
                webview,
                rendering_context,
                width,
                height,
                last_frame: None,
                frame_seq: 0,
                frame_ready,
                last_load_status: LoadStatus::Loading,
                current_url: "about:blank".to_string(),
                shared_load_status,
                shared_url,
                shared_history,
                last_history: (Vec::new(), 0),
                shared_page_title,
                last_page_title: None,
                shared_console_errors,
                shared_favicon,
                shared_control,
                shared_cursor,
                shared_crash,
                shared_console_log,
                shared_net_log,
                last_favicon: None,
                script_watch: super::ScriptWatch::default(),
                probe_answered: Rc::default(),
                throttle_sent,
            })
        }

        /// Navigate the active WebView to `url`.  No-op if `url` is not a
        /// valid absolute URL (logs a warning instead of panicking).
        pub fn navigate(&self, url: &str) {
            match url::Url::parse(url) {
                Ok(parsed) => self.webview.load(parsed),
                Err(e) => eprintln!("[ferrite-session] invalid URL '{}': {}", url, e),
            }
        }

        /// Go back one step in the navigation history.
        ///
        /// Requires servo v0.0.5 `WebView::go_back()`.  If the method does not
        /// exist in your build, replace this call with an appropriate alternative.
        pub fn go_back(&self) {
            self.webview.go_back(1);
        }

        /// Go forward one step in the navigation history.
        ///
        /// Requires servo v0.0.5 `WebView::go_forward()`.
        pub fn go_forward(&self) {
            self.webview.go_forward(1);
        }

        /// Reload the current page.
        ///
        /// Falls back to re-navigating to `current_url()` if `WebView::reload()`
        /// is not available in this servo build.
        pub fn reload(&self) {
            self.webview.reload();
        }

        /// Stop the current page load.
        ///
        /// `WebView::stop()` is not available in this servo build; this is a no-op for now.
        pub fn stop(&self) {
            // TODO: servo WebView does not expose a stop() method yet.
            log::warn!("stop() called but servo WebView::stop() is not available");
        }

        /// Returns the current load status of the active WebView.
        ///
        /// Reflects the most recent state synced in the last `spin()` call.
        pub fn load_status(&self) -> &LoadStatus {
            &self.last_load_status
        }

        /// Returns the current URL of the active WebView.
        ///
        /// Reflects the most recent URL synced when load completed in `spin()`.
        pub fn current_url(&self) -> &str {
            &self.current_url
        }

        /// Returns `true` if there is at least one page to go back to —
        /// backed by Servo's own native session history
        /// (`WebViewDelegate::notify_history_changed`), not an
        /// approximated `Complete`-event counter.
        pub fn can_go_back(&self) -> bool {
            self.last_history.1 > 0
        }

        /// Returns `true` if there are forward pages in the navigation
        /// history — real, not hardcoded `false`: Servo's own history list
        /// (`self.last_history`) carries the full entry list and the
        /// current index, so "is there an entry after the current one" is
        /// answerable directly.
        pub fn can_go_forward(&self) -> bool {
            self.last_history.1 + 1 < self.last_history.0.len()
        }

        /// The full session-history list and current index for this tab —
        /// `(entries, current_index)`, straight from Servo's own
        /// `WebViewDelegate::notify_history_changed`. Exposed for a
        /// History panel that wants this tab's real forward/back order,
        /// distinct from `ferrite-ui`'s own browser-wide, recency-ordered
        /// visit list (see that crate's `FerriteBrowser::history` doc
        /// comment for why the two are different, deliberately).
        pub fn history(&self) -> (&[String], usize) {
            (&self.last_history.0, self.last_history.1)
        }

        /// Pump the shared Servo engine for one turn.
        ///
        /// When multiple tabs are open this must be called **exactly once per
        /// tick** (on any one session) before calling `sync_and_read()` on
        /// every session.  Calling it more than once per tick risks double-
        /// processing paint messages and can cause a segfault inside Servo.
        pub fn pump_engine(&self) {
            self.servo.spin_event_loop();
        }

        /// Sync per-tab state from delegate callbacks and read back the latest
        /// composited frame into `last_frame`.
        ///
        /// Call this on **every** session after one `pump_engine()` call.
        pub fn sync_and_read(&mut self) {
            self.sync_state();
            self.read_frame();
        }

        /// Sync per-tab state from delegate callbacks without reading pixels —
        /// what a background tab needs each tick (title, URL, load status),
        /// without the cost of copying a full frame nobody is looking at.
        pub fn sync_state(&mut self) {
            // Sync load status, URL, and page title from delegate callbacks.
            self.last_load_status = self.shared_load_status.borrow().clone();
            let new_url = self.shared_url.borrow().clone();
            if new_url != self.current_url {
                // A new page: the old probe's answer will never come.
                self.script_watch.reset();
                self.probe_answered = Rc::default();
            }
            self.current_url = new_url;
            self.watch_script_thread();
            self.last_page_title = self.shared_page_title.borrow().clone();
            self.last_favicon = self.shared_favicon.borrow().clone();
            // Without this the delegate's history was written and never read, so
            // `can_go_back()`/`can_go_forward()`/`history()` always reported an
            // empty history in a real Servo build (found by the first real
            // `--features servo` compile, as a dead-code warning on this field).
            self.last_history = self.shared_history.borrow().clone();
        }

        /// Asks the page's script thread a trivial question every two seconds
        /// and says, in the tab's console and the log, when it stops answering
        /// and when it answers again. It never waits for the answer.
        fn watch_script_thread(&mut self) {
            use super::WatchEvent;
            use crate::diag::{push_bounded, ConsoleEntry, ConsoleLevel};
            let event = self
                .script_watch
                .step(std::time::Instant::now(), self.probe_answered.get());
            let note = |level: ConsoleLevel, message: String| {
                push_bounded(
                    &mut self.shared_console_log.borrow_mut(),
                    ConsoleEntry {
                        at_ms: crate::diag::now_ms(),
                        level,
                        source: None,
                        message,
                    },
                );
            };
            match event {
                WatchEvent::Quiet => {}
                WatchEvent::SendProbe => {
                    let answered: Rc<std::cell::Cell<bool>> = Rc::default();
                    self.probe_answered = answered.clone();
                    self.webview
                        .evaluate_javascript("0", move |_| answered.set(true));
                }
                WatchEvent::Stalled(waited) => note(
                    ConsoleLevel::Warn,
                    format!(
                        "Ferrite: this page's script has not answered for {} s. It is busy or \
                         stuck, and scrolling and clicking need it. ({})",
                        waited.as_secs(),
                        self.current_url
                    ),
                ),
                WatchEvent::Recovered(waited) => note(
                    ConsoleLevel::Info,
                    format!(
                        "Ferrite: this page's script answered again after {} s.",
                        waited.as_secs()
                    ),
                ),
            }
        }

        /// The render-surface size this session was last asked to have,
        /// `(width, height)` in physical pixels.
        pub fn size(&self) -> (u32, u32) {
            (self.width, self.height)
        }

        fn read_frame(&mut self) {
            // Nothing new was painted since the last read: keep the picture we
            // have (and its number) rather than copying megabytes of identical
            // pixels out of the GL context sixty times a second.
            if !self.frame_ready.get() && self.last_frame.is_some() {
                return;
            }
            // Read back the current frame after paint.
            //
            // `pump_engine()` may have called `make_current()` on another
            // tab's rendering context (GL context is a per-thread global).
            // Re-establish this tab's context as current before the readback so
            // `glReadPixels` reads the correct surface.
            let _ = self.rendering_context.make_current();
            let rect = servo::DeviceIntRect::from_origin_and_size(
                servo::DeviceIntPoint::origin(),
                servo::DeviceIntSize::new(self.width as i32, self.height as i32),
            );
            if let Some(rgba) = self.rendering_context.read_to_image(rect) {
                self.frame_ready.set(false);
                self.last_frame = Some(std::sync::Arc::new(rgba.into_raw()));
                self.frame_seq = super::next_frame_seq();
            }
        }

        /// Convenience wrapper for the single-tab case: pump + sync + read in one call.
        pub fn spin(&mut self) {
            self.pump_engine();
            self.sync_and_read();
        }

        /// Returns `(width, height, rgba_bytes)` of the most recently rendered
        /// frame, or `None` if no frame has been produced yet.
        pub fn get_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
            self.last_frame
                .as_ref()
                .map(|b| (self.width, self.height, b.as_ref().clone()))
        }

        /// The current frame without copying it: `(sequence, width, height,
        /// pixels)`. The sequence number changes exactly when the picture
        /// does, so a caller that already has that number needs nothing
        /// (this is what keeps scrolling smooth: a handle is built once per
        /// new picture, not once per redraw).
        pub fn frame_shared(&self) -> Option<(u64, u32, u32, std::sync::Arc<Vec<u8>>)> {
            self.last_frame
                .as_ref()
                .map(|b| (self.frame_seq, self.width, self.height, b.clone()))
        }

        /// Returns `(width, height, rgba_bytes)` of the page's current favicon, or
        /// `None` if the page has not set one (yet, or at all).
        pub fn get_favicon(&self) -> Option<(u32, u32, Vec<u8>)> {
            self.last_favicon.clone()
        }

        /// Returns the most recently received page title, or `None` if the page has
        /// not set a title yet.
        pub fn page_title(&self) -> Option<&str> {
            self.last_page_title.as_deref()
        }

        /// Execute `script` in the active WebView and return the result as a `String`.
        ///
        /// Uses `WebView::evaluate_javascript` (servo v0.0.5).  The result is
        /// serialised to JSON by Servo and returned as-is.  Returns `Err` if the
        /// WebView call itself fails (e.g. engine not initialised).
        pub fn execute_js(&mut self, script: &str) -> Result<String, String> {
            // Servo v0.0.5 exposes evaluate_javascript on WebView.
            // The callback receives Option<String> (Some = success, None = exception).
            let result_cell: Rc<std::cell::RefCell<Option<Result<String, String>>>> =
                Rc::new(std::cell::RefCell::new(None));
            let cell_clone = result_cell.clone();

            self.webview.evaluate_javascript(script, move |res| {
                *cell_clone.borrow_mut() = Some(match res {
                    Ok(v) => Ok(format!("{:?}", v)),
                    Err(e) => Err(format!("{:?}", e)),
                });
            });

            // Drive the event loop until the callback fires (max 2 s).
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                self.pump_engine();
                if result_cell.borrow().is_some() {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    return Err("JS evaluation timed out".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }

            let result = result_cell
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Err("JS evaluation result missing".to_string()));

            match &result {
                Ok(v) => println!(
                    "[ferrite-js] execute: {} chars, result: {}",
                    script.len(),
                    &v[..50.min(v.len())]
                ),
                Err(e) => println!("[ferrite-js] execute: {} chars, error: {}", script.len(), e),
            }

            result
        }

        /// Drains and returns all JS console errors collected since the last call.
        pub fn take_console_errors(&mut self) -> Vec<String> {
            std::mem::take(&mut *self.shared_console_errors.borrow_mut())
        }

        /// Every console message (any level) since the last call, oldest first.
        pub fn take_console_entries(&mut self) -> Vec<crate::diag::ConsoleEntry> {
            self.shared_console_log.borrow_mut().drain(..).collect()
        }

        /// Tells the engine the display's current scale (see [`set_display_scale`]).
        pub fn apply_display_scale(&self) {
            self.webview
                .set_hidpi_scale_factor(euclid::Scale::new(super::display_scale()));
        }

        /// What the page is waiting on a person for, if anything.
        pub fn page_control(&self) -> Option<crate::diag::PageControl> {
            self.shared_control
                .borrow()
                .as_ref()
                .map(|(v, _)| v.clone())
        }

        /// Answers (or dismisses) the page's pending control.
        pub fn answer_control(&mut self, answer: crate::diag::ControlAnswer) {
            use crate::diag::ControlAnswer as A;
            let Some((_, control)) = self.shared_control.borrow_mut().take() else {
                return;
            };
            match (control, answer) {
                (servo::EmbedderControl::SelectElement(mut select), A::Select(chosen)) => {
                    select.select(chosen);
                    select.submit();
                }
                (servo::EmbedderControl::SimpleDialog(servo::SimpleDialog::Alert(a)), _) => {
                    a.confirm();
                }
                (
                    servo::EmbedderControl::SimpleDialog(servo::SimpleDialog::Confirm(c)),
                    A::Accept(_),
                ) => {
                    c.confirm();
                }
                (servo::EmbedderControl::SimpleDialog(servo::SimpleDialog::Confirm(c)), _) => {
                    c.dismiss();
                }
                (
                    servo::EmbedderControl::SimpleDialog(servo::SimpleDialog::Prompt(mut p)),
                    A::Accept(text),
                ) => {
                    if let Some(text) = text {
                        p.set_current_value(&text);
                    }
                    p.confirm();
                }
                (servo::EmbedderControl::SimpleDialog(servo::SimpleDialog::Prompt(p)), _) => {
                    p.dismiss();
                }
                (servo::EmbedderControl::FilePicker(mut f), A::Files(paths)) => {
                    f.select(&paths);
                    f.submit();
                }
                (servo::EmbedderControl::FilePicker(f), _) => f.dismiss(),
                (servo::EmbedderControl::ColorPicker(mut c), A::Color(text)) => {
                    if let Some((red, green, blue)) = crate::diag::parse_hex_color(&text) {
                        c.select(Some(servo::RgbColor { red, green, blue }));
                    }
                    c.submit();
                }
                (servo::EmbedderControl::ContextMenu(menu), A::Menu(index)) => {
                    let action = menu
                        .items()
                        .iter()
                        .filter_map(|item| match item {
                            servo::ContextMenuItem::Item {
                                action,
                                enabled: true,
                                ..
                            } => Some(*action),
                            servo::ContextMenuItem::Item { .. } => None,
                            servo::ContextMenuItem::Separator => None,
                        })
                        .nth(index);
                    match action {
                        Some(action) => menu.select(action),
                        None => menu.dismiss(),
                    }
                }
                (servo::EmbedderControl::ContextMenu(menu), _) => menu.dismiss(),
                // Anything else (a mismatched answer, a colour picker closed,
                // a select closed): dropping it tells the engine "no change".
                (other, _) => drop(other),
            }
        }

        /// The pointer the page currently asks for.
        pub fn cursor(&self) -> crate::diag::PageCursor {
            self.shared_cursor.get()
        }

        /// Takes the crash note, if the page died since the last call.
        pub fn take_crash(&mut self) -> Option<crate::diag::CrashNote> {
            self.shared_crash.borrow_mut().take()
        }

        /// Every request announced since the last call, oldest first.
        pub fn take_net_events(&mut self) -> Vec<crate::diag::NetEvent> {
            self.shared_net_log.borrow_mut().drain(..).collect()
        }

        /// Navigate to `url`, drive the event loop for up to `timeout_secs`, and
        /// return a [`JSCompatResult`] summarising what happened.
        ///
        /// JS execution is inferred from whether the page title changed from
        /// `None` — most non-trivial pages set their `<title>` via JS.
        pub fn test_js_compat(&mut self, url: &str) -> super::JSCompatResult {
            // Clear any state left over from a previous probe.
            *self.shared_page_title.borrow_mut() = None;
            self.last_page_title = None;
            let _ = self.take_console_errors();

            self.navigate(url);

            let timeout = std::time::Duration::from_secs(5);
            let started = std::time::Instant::now();

            loop {
                self.spin();

                if matches!(
                    self.last_load_status,
                    LoadStatus::Complete | LoadStatus::Failed(_)
                ) {
                    break;
                }
                if started.elapsed() >= timeout {
                    break;
                }

                // Yield the thread briefly so we don't busy-spin at 100% CPU.
                std::thread::sleep(std::time::Duration::from_millis(16));
            }

            let title = self.last_page_title.clone();
            let js_executed = title.is_some();
            let console_errors = self.take_console_errors();

            super::JSCompatResult {
                url: url.to_string(),
                js_executed,
                console_errors,
                page_title: title,
            }
        }

        /// Resize the render surface and notify the WebView.
        ///
        /// **Real bug this shape fixes — verified against the pinned
        /// `libservo` source (`components/paint/painter.rs`'s
        /// `resize_rendering_context`, `components/servo/webview.rs`'s
        /// `WebView::resize`), not guessed:** this used to also call
        /// `self.rendering_context.resize(size)` directly, in addition to
        /// `webview.resize(size)`. `webview.resize()` shares the exact same
        /// `RenderingContext` this session holds (`WebViewBuilder::new`
        /// below is given `rendering_context.clone()`), and internally
        /// calls `Painter::resize_rendering_context`, which starts with
        /// `if self.rendering_context.size() == new_size { return; }`
        /// before it does anything else — including the transaction that
        /// tells the compositor the viewport changed and schedules a
        /// repaint at the new size. Calling `rendering_context.resize()`
        /// ourselves *first* made that check see "no change" every time
        /// (we'd already set the size Servo was about to compare against),
        /// so the repaint-at-new-size step silently never ran — the page
        /// stayed rendered at its old size forever, with the rest of the
        /// now-larger buffer left uninitialized (observed directly: page
        /// content confined to a small region, the rest of the window
        /// black). Separately, that direct call also never established
        /// this context as current first (unlike `sync_and_read()`'s own
        /// `make_current()` call below, and unlike what
        /// `resize_rendering_context` itself does internally before
        /// touching the surface) — a real, additional risk of corrupting
        /// GL/surfman state if some other tab's context was left current,
        /// consistent with the "texture unloadable" GL warning and
        /// segfault also observed. Servo's own reference headless embedder
        /// (`servoshell`'s `headless_window.rs::request_resize`) calls only
        /// `webview.resize()`, with a comment explaining why:
        /// "[we] must notify `Paint` here" — `webview.resize()` alone is
        /// the complete, correct call; this now matches that exactly.
        pub fn resize(&mut self, width: u32, height: u32) {
            self.width = width;
            self.height = height;
            self.frame_ready.set(true);
            self.webview.resize(PhysicalSize { width, height });
        }

        /// Sets the page zoom (1.0 = 100%) natively, through the engine: layout
        /// and pointer hit-testing follow it, and no script runs in the page.
        /// (An earlier version injected a CSS transform with JavaScript, which
        /// ran inside heavy pages on every load and could interfere with them.)
        pub fn set_zoom(&self, level: f32) {
            self.webview.set_page_zoom(level);
        }

        /// The page zoom currently in effect.
        pub fn zoom(&self) -> f32 {
            self.webview.page_zoom()
        }

        /// Marks this tab as the one the user is looking at (`true`) or as a
        /// background tab (`false`). Servo routes keyboard input to the focused
        /// WebView and hit-tests pointer input only against shown WebViews, so a
        /// tab that was never focused/shown does not react to input.
        pub fn set_active(&self, active: bool) {
            // A background tab is throttled (timers slowed, animations stopped)
            // and its document is hidden; the active one is not. Sent only when it
            // changes, because this is called whenever tabs are re-synced.
            if self.throttle_sent.get() != Some(!active) {
                self.throttle_sent.set(Some(!active));
                self.webview.set_throttled(!active);
            }
            if active {
                self.frame_ready.set(true);
                self.webview.show();
                self.webview.focus();
            } else {
                self.webview.blur();
                self.webview.hide();
            }
        }

        /// Send a mouse-move event to the WebView at pixel coordinates `(x, y)`.
        pub fn send_mouse_move(&self, x: f32, y: f32) {
            let point = WebViewPoint::Device(DevicePoint::new(x, y));
            self.webview
                .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point)));
        }

        /// Send a right mouse-button click at pixel coordinates `(x, y)`.
        pub fn send_right_click(&self, x: f32, y: f32) {
            let point = WebViewPoint::Device(DevicePoint::new(x, y));
            self.webview
                .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Down,
                    MouseButton::Secondary,
                    point,
                )));
            self.webview
                .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Up,
                    MouseButton::Secondary,
                    point,
                )));
        }

        /// Send a scroll (wheel) event at pixel coordinates `(x, y)`.
        ///
        /// `delta_x`/`delta_y` follow the OS wheel convention the UI passes through
        /// unchanged: positive `delta_y` scrolls the page up (content moves down).
        /// The wheel event alone scrolls the page; an extra legacy `Scroll` call
        /// used to be sent as well, which made every scroll travel twice as far
        /// (measured with `examples/input_probe.rs`).
        pub fn send_scroll(&self, x: f32, y: f32, delta_x: f64, delta_y: f64) {
            let point = WebViewPoint::Device(DevicePoint::new(x, y));
            self.webview
                .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                    WheelDelta {
                        x: delta_x,
                        y: delta_y,
                        z: 0.0,
                        mode: WheelMode::DeltaPixel,
                    },
                    point,
                )));
        }

        /// Forwards a key press/release to the page's focused element.
        ///
        /// Mirrors what `servoshell` does for a real window
        /// (`keyboard_event_from_winit`): a `KeyboardEvent` with the typed
        /// character as `Key::Character`, a best-effort physical `Code`, and
        /// the modifier state. `Cmd/Ctrl + C/X/V` become Servo
        /// `EditingActionEvent`s instead (Servo owns the clipboard), and the
        /// raw key events for those combinations are not sent.
        pub fn send_key(&self, event: &PageKeyEvent) {
            if let Some(edit) = event.edit_combo() {
                if event.down {
                    let action = match edit {
                        PageEdit::Copy => EditingActionEvent::Copy,
                        PageEdit::Cut => EditingActionEvent::Cut,
                        PageEdit::Paste => EditingActionEvent::Paste,
                    };
                    self.webview
                        .notify_input_event(InputEvent::EditingAction(action));
                }
                return;
            }
            let (key, code) = match &event.key {
                PageKey::Character(text) => (
                    Key::Character(text.clone()),
                    text.chars()
                        .next()
                        .filter(|_| text.chars().count() == 1)
                        .map_or(Code::Unidentified, code_for_char),
                ),
                PageKey::Named(named) => {
                    let (named, code) = named_key(*named);
                    (Key::Named(named), code)
                }
            };
            let mut modifiers = Modifiers::empty();
            if event.shift {
                modifiers |= Modifiers::SHIFT;
            }
            if event.ctrl {
                modifiers |= Modifiers::CONTROL;
            }
            if event.alt {
                modifiers |= Modifiers::ALT;
            }
            if event.meta {
                modifiers |= Modifiers::META;
            }
            let state = if event.down {
                KeyState::Down
            } else {
                KeyState::Up
            };
            self.webview.notify_input_event(InputEvent::Keyboard(
                KeyboardEvent::new_without_event(
                    state,
                    key,
                    code,
                    Location::Standard,
                    modifiers,
                    false,
                    false,
                ),
            ));
        }

        /// Send a mouse-down event (without the subsequent up) — for drag start.
        pub fn send_mouse_down(&self, x: f32, y: f32) {
            let point = WebViewPoint::Device(DevicePoint::new(x, y));
            self.webview
                .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Down,
                    MouseButton::Primary,
                    point,
                )));
        }

        /// Send a mouse-up event — for drag end.
        pub fn send_mouse_up(&self, x: f32, y: f32) {
            let point = WebViewPoint::Device(DevicePoint::new(x, y));
            self.webview
                .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    MouseButtonAction::Up,
                    MouseButton::Primary,
                    point,
                )));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // `favicon_to_rgba8` is a pure function of its inputs — testable
        // without a real `Servo`/`WebView` (which R7 rules out building in
        // an automated test anyway). Only compiled and run under
        // `--features servo` (this whole module is `#[cfg(feature =
        // "servo")]`-gated), so `just test`/`cargo test --workspace`
        // (default, Servo-free features) never exercises it — but it is the
        // one real correctness check on this conversion logic for anyone
        // who does build with the feature, since neither CI job (`ci`:
        // Servo-free; `build-servo-release`: `cargo build`, not `test` or
        // `clippy`) currently type-checks `mod inner` at all otherwise.

        #[test]
        fn rgba8_passes_through_unchanged() {
            let data = [10u8, 20, 30, 40, 50, 60, 70, 80];
            assert_eq!(
                favicon_to_rgba8(2, 1, servo::PixelFormat::RGBA8, &data),
                data
            );
        }

        #[test]
        fn bgra8_swaps_red_and_blue_leaving_green_and_alpha_in_place() {
            let bgra = [1u8, 2, 3, 4];
            assert_eq!(
                favicon_to_rgba8(1, 1, servo::PixelFormat::BGRA8, &bgra),
                vec![3, 2, 1, 4]
            );
        }

        #[test]
        fn rgb8_expands_to_rgba_with_opaque_alpha() {
            let rgb = [10u8, 20, 30, 40, 50, 60];
            assert_eq!(
                favicon_to_rgba8(2, 1, servo::PixelFormat::RGB8, &rgb),
                vec![10, 20, 30, 255, 40, 50, 60, 255]
            );
        }

        #[test]
        fn ka8_replicates_luminance_into_rgb_and_keeps_the_real_alpha() {
            let ka = [200u8, 128];
            assert_eq!(
                favicon_to_rgba8(1, 1, servo::PixelFormat::KA8, &ka),
                vec![200, 200, 200, 128]
            );
        }

        #[test]
        fn k8_replicates_luminance_into_rgb_with_opaque_alpha() {
            let k = [42u8, 99];
            assert_eq!(
                favicon_to_rgba8(2, 1, servo::PixelFormat::K8, &k),
                vec![42, 42, 42, 255, 99, 99, 99, 255]
            );
        }
    }
}

// Stub for non-servo builds so the type name is always resolvable.
/// Without the `servo` feature there is no engine to shut down.
#[cfg(not(feature = "servo"))]
pub fn shutdown_engine() {}

/// Without the `servo` feature no page can open a tab.
#[cfg(not(feature = "servo"))]
pub fn take_popup_sessions() -> Vec<HeadlessServoSession> {
    Vec::new()
}

#[cfg(not(feature = "servo"))]
pub struct HeadlessServoSession;

#[cfg(not(feature = "servo"))]
impl HeadlessServoSession {
    pub fn new(_width: u32, _height: u32) -> Result<Self, String> {
        Err("ferrite-servo compiled without the `servo` feature".to_string())
    }

    pub fn navigate(&self, _url: &str) {}

    pub fn go_back(&self) {}

    pub fn go_forward(&self) {}

    pub fn reload(&self) {}

    pub fn stop(&self) {}

    pub fn pump_engine(&self) {}

    pub fn sync_and_read(&mut self) {}

    pub fn sync_state(&mut self) {}

    pub fn size(&self) -> (u32, u32) {
        (0, 0)
    }

    pub fn spin(&mut self) {}

    pub fn get_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        None
    }

    pub fn frame_shared(&self) -> Option<(u64, u32, u32, std::sync::Arc<Vec<u8>>)> {
        None
    }

    pub fn resize(&mut self, _width: u32, _height: u32) {}

    pub fn load_status(&self) -> &LoadStatus {
        const STATUS: LoadStatus = LoadStatus::Loading;
        &STATUS
    }

    pub fn current_url(&self) -> &str {
        "about:blank"
    }

    pub fn can_go_back(&self) -> bool {
        false
    }

    pub fn can_go_forward(&self) -> bool {
        false
    }

    pub fn history(&self) -> (&[String], usize) {
        (&[], 0)
    }

    pub fn page_title(&self) -> Option<&str> {
        None
    }

    pub fn get_favicon(&self) -> Option<(u32, u32, Vec<u8>)> {
        None
    }

    pub fn execute_js(&mut self, _script: &str) -> Result<String, String> {
        Err("ferrite-servo compiled without the `servo` feature".to_string())
    }

    pub fn take_console_errors(&mut self) -> Vec<String> {
        vec![]
    }

    pub fn take_console_entries(&mut self) -> Vec<crate::diag::ConsoleEntry> {
        Vec::new()
    }

    pub fn take_net_events(&mut self) -> Vec<crate::diag::NetEvent> {
        Vec::new()
    }

    pub fn apply_display_scale(&self) {}

    pub fn page_control(&self) -> Option<crate::diag::PageControl> {
        None
    }

    pub fn answer_control(&mut self, _answer: crate::diag::ControlAnswer) {}

    pub fn cursor(&self) -> crate::diag::PageCursor {
        crate::diag::PageCursor::Default
    }

    pub fn take_crash(&mut self) -> Option<crate::diag::CrashNote> {
        None
    }

    pub fn test_js_compat(&mut self, url: &str) -> JSCompatResult {
        JSCompatResult {
            url: url.to_string(),
            js_executed: false,
            console_errors: vec![],
            page_title: None,
        }
    }

    pub fn send_key(&self, _event: &PageKeyEvent) {}
    pub fn set_active(&self, _active: bool) {}
    pub fn set_zoom(&self, _level: f32) {}
    pub fn zoom(&self) -> f32 {
        1.0
    }
    pub fn send_mouse_move(&self, _x: f32, _y: f32) {}
    pub fn send_right_click(&self, _x: f32, _y: f32) {}
    pub fn send_scroll(&self, _x: f32, _y: f32, _dx: f64, _dy: f64) {}
    pub fn send_mouse_down(&self, _x: f32, _y: f32) {}
    pub fn send_mouse_up(&self, _x: f32, _y: f32) {}
}

#[cfg(test)]
mod page_key_tests {
    use super::*;

    fn ev(down: bool, key: PageKey, ctrl: bool, meta: bool, alt: bool) -> PageKeyEvent {
        PageKeyEvent {
            down,
            key,
            shift: false,
            ctrl,
            alt,
            meta,
        }
    }

    fn ch(s: &str) -> PageKey {
        PageKey::Character(s.to_string())
    }

    #[test]
    fn a_plain_character_is_never_a_clipboard_combo() {
        assert_eq!(ev(true, ch("v"), false, false, false).edit_combo(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cmd_c_x_v_are_the_edit_combos_on_macos_and_ctrl_is_not() {
        assert_eq!(
            ev(true, ch("c"), false, true, false).edit_combo(),
            Some(PageEdit::Copy)
        );
        assert_eq!(
            ev(false, ch("X"), false, true, false).edit_combo(),
            Some(PageEdit::Cut)
        );
        assert_eq!(
            ev(true, ch("v"), false, true, false).edit_combo(),
            Some(PageEdit::Paste)
        );
        assert_eq!(ev(true, ch("v"), true, false, false).edit_combo(), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ctrl_c_x_v_are_the_edit_combos_off_macos_and_cmd_is_not() {
        assert_eq!(
            ev(true, ch("c"), true, false, false).edit_combo(),
            Some(PageEdit::Copy)
        );
        assert_eq!(
            ev(false, ch("X"), true, false, false).edit_combo(),
            Some(PageEdit::Cut)
        );
        assert_eq!(
            ev(true, ch("v"), true, false, false).edit_combo(),
            Some(PageEdit::Paste)
        );
        assert_eq!(ev(true, ch("v"), false, true, false).edit_combo(), None);
    }

    #[test]
    fn alt_with_the_command_key_is_not_a_clipboard_combo() {
        let ctrl_or_meta = cfg!(target_os = "macos");
        assert_eq!(
            ev(true, ch("v"), !ctrl_or_meta, ctrl_or_meta, true).edit_combo(),
            None
        );
    }

    #[test]
    fn named_keys_are_never_clipboard_combos() {
        let ctrl_or_meta = cfg!(target_os = "macos");
        assert_eq!(
            ev(
                true,
                PageKey::Named(PageNamedKey::Enter),
                !ctrl_or_meta,
                ctrl_or_meta,
                false
            )
            .edit_combo(),
            None
        );
    }
}

#[cfg(test)]
mod user_agent_tests {
    use super::*;

    #[test]
    fn the_compatible_form_swaps_only_the_engine_token() {
        assert_eq!(
            compatible_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:153.0) Servo/0.6.0 Firefox/153.0").as_deref(),
            Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:153.0) Gecko/20100101 Firefox/153.0")
        );
        assert_eq!(
            compatible_user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:153.0) Servo/0.6.0 Firefox/153.0"
            )
            .as_deref(),
            Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:153.0) Gecko/20100101 Firefox/153.0"
            )
        );
    }

    #[test]
    fn an_unexpected_shape_is_left_alone_not_mangled() {
        assert_eq!(compatible_user_agent("Mozilla/5.0 Something/1.0"), None);
        assert_eq!(
            compatible_user_agent("Mozilla/5.0 (X11) Servo/0.6.0"),
            None,
            "no Firefox token"
        );
        assert_eq!(compatible_user_agent(""), None);
    }

    #[test]
    fn a_blank_override_is_no_override() {
        set_user_agent(Some("   ".to_string()));
        assert_eq!(USER_AGENT_OVERRIDE.lock().unwrap().clone(), None);
    }
}

#[cfg(all(test, feature = "servo"))]
mod svg_compat_tests {
    use super::{STORAGE_COMPAT_JS, SVG_COMPAT_JS, WEB_COMPAT_JS};

    #[test]
    fn the_compat_scripts_parse_under_node() {
        for (name, source) in [
            ("svg_compat.js", SVG_COMPAT_JS),
            ("web_compat.js", WEB_COMPAT_JS),
            ("storage_compat.js", STORAGE_COMPAT_JS),
        ] {
            parses_under_node(name, source);
        }
    }

    fn parses_under_node(name: &str, source: &str) {
        let node_ok = std::process::Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !node_ok {
            eprintln!("SKIPPED: `node` is not installed; {name} not machine-checked");
            return;
        }
        let dir = std::env::temp_dir().join(format!("ferrite-svg-compat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, source).unwrap();
        let out = std::process::Command::new("node")
            .arg("--check")
            .arg(&path)
            .output()
            .expect("node runs");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn the_script_only_writes_inline_style_on_svg_content() {
        // The contract in the script's header, pinned: it must not touch the
        // page outside <svg> subtrees, and must not run twice on one element.
        assert!(SVG_COMPAT_JS.contains("__ferriteSvgFixed"));
        assert!(SVG_COMPAT_JS.contains("querySelectorAll(SHAPES)"));
        assert!(!SVG_COMPAT_JS.contains("document.body.appendChild"));
        assert!(!SVG_COMPAT_JS.contains("fetch("));
        assert!(!SVG_COMPAT_JS.contains("XMLHttpRequest"));
    }
}
