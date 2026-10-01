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
        SERVO_ENGINE.with(|cell| drop(cell.borrow_mut().take()));
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
                let mut prefs = servo::Preferences {
                    dom_indexeddb_enabled: true,
                    dom_cookiestore_enabled: true,
                    dom_intersection_observer_enabled: true,
                    // Web APIs that sign-in and anti-abuse scripts probe for
                    // (and that ordinary sites use) and that Servo ships off
                    // by default. Each is a real implementation being turned
                    // on, not a stub: see `examples/web_api_probe.rs`.
                    dom_permissions_enabled: true,
                    dom_notification_enabled: true,
                    dom_async_clipboard_enabled: true,
                    dom_webgl2_enabled: true,
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
                *guard = Some(
                    ServoBuilder::default()
                        .opts(opts)
                        .preferences(prefs)
                        .build(),
                );
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
    }

    impl WebViewDelegate for HeadlessDelegate {
        fn notify_new_frame_ready(&self, webview: servo::WebView) {
            webview.paint();
        }

        fn notify_load_status_changed(&self, webview: servo::WebView, status: servo::LoadStatus) {
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
            // Only capture error-level messages to keep the list focused on
            // actionable JS failures.
            if matches!(level, servo::ConsoleLogLevel::Error) {
                self.console_errors.borrow_mut().push(message);
            }
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
            match HeadlessServoSession::assemble(
                1280,
                700,
                |_servo, rendering_context, delegate| {
                    request
                        .builder(rendering_context)
                        .delegate(delegate)
                        .build()
                },
            ) {
                Ok(session) => POPUP_SESSIONS.with(|q| q.borrow_mut().push(session)),
                Err(e) => eprintln!("[ferrite-session] cannot open a page-requested tab: {e}"),
            }
        }

        fn load_web_resource(&self, _webview: servo::WebView, load: servo::WebResourceLoad) {
            let url = load.request().url.to_string();
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
        rendering_context: Rc<SoftwareRenderingContext>,
        width: u32,
        height: u32,
        /// Cached last frame as raw RGBA bytes (width × height × 4).
        last_frame: Option<Vec<u8>>,
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
        /// Most recently synced favicon (updated in `sync_and_read()`).
        last_favicon: Option<(u32, u32, Vec<u8>)>,
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
            make: impl FnOnce(
                &Servo,
                Rc<SoftwareRenderingContext>,
                Rc<HeadlessDelegate>,
            ) -> servo::WebView,
        ) -> Result<Self, String> {
            // ── rustls crypto provider ─────────────────────────────────────
            let _ = aws_lc_rs::default_provider().install_default();

            // ── Audit log ──────────────────────────────────────────────────
            let audit_log = shared_audit_log()?;

            // ── Shared delegate ↔ session state ────────────────────────────
            let shared_load_status = Rc::new(std::cell::RefCell::new(LoadStatus::Loading));
            let shared_url = Rc::new(std::cell::RefCell::new("about:blank".to_string()));
            let shared_history: Rc<std::cell::RefCell<(Vec<String>, usize)>> =
                Rc::new(std::cell::RefCell::new((Vec::new(), 0)));
            let shared_page_title: Rc<std::cell::RefCell<Option<String>>> =
                Rc::new(std::cell::RefCell::new(None));
            let shared_console_errors: Rc<std::cell::RefCell<Vec<String>>> =
                Rc::new(std::cell::RefCell::new(Vec::new()));
            let shared_favicon: SharedFavicon = Rc::new(std::cell::RefCell::new(None));

            // ── Rendering context ──────────────────────────────────────────
            let rendering_context = Rc::new(
                SoftwareRenderingContext::new(PhysicalSize { width, height })
                    .map_err(|e| format!("SoftwareRenderingContext: {:?}", e))?,
            );
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
            });
            let webview = make(&servo, rendering_context.clone(), delegate);

            webview.resize(PhysicalSize { width, height });
            servo.spin_event_loop();

            Ok(Self {
                servo,
                webview,
                rendering_context,
                width,
                height,
                last_frame: None,
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
                last_favicon: None,
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
            self.current_url = self.shared_url.borrow().clone();
            self.last_page_title = self.shared_page_title.borrow().clone();
            self.last_favicon = self.shared_favicon.borrow().clone();
            // Without this the delegate's history was written and never read, so
            // `can_go_back()`/`can_go_forward()`/`history()` always reported an
            // empty history in a real Servo build (found by the first real
            // `--features servo` compile, as a dead-code warning on this field).
            self.last_history = self.shared_history.borrow().clone();
        }

        /// The render-surface size this session was last asked to have,
        /// `(width, height)` in physical pixels.
        pub fn size(&self) -> (u32, u32) {
            (self.width, self.height)
        }

        fn read_frame(&mut self) {
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
                self.last_frame = Some(rgba.into_raw());
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
                .map(|b| (self.width, self.height, b.clone()))
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
            if active {
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
