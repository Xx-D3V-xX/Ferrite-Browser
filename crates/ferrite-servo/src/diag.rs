//! What a page and the engine report about themselves, kept for a DevTools
//! style view: every console message with its level, the requests a page
//! made, and the panics an engine thread hit.
//!
//! The types are plain data with no engine dependency, so the UI uses them
//! with or without the `servo` feature.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The most entries a tab keeps of each kind; the oldest are dropped first.
pub const MAX_ENTRIES: usize = 5_000;

/// A console message's severity, as the page logged it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsoleLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

impl ConsoleLevel {
    /// A short lowercase name, as DevTools filters spell it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Log => "log",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One `console.*` call or uncaught exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    /// Milliseconds since the Unix epoch when it arrived.
    pub at_ms: u64,
    pub level: ConsoleLevel,
    pub message: String,
    /// `file:line:col` when the message names one (uncaught exceptions do:
    /// `Error at <url>:<line>:<col> ...`).
    pub source: Option<String>,
}

/// One request a page made, as the engine announced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetEvent {
    pub at_ms: u64,
    pub method: String,
    pub url: String,
    /// What the request is for: `Document`, `Script`, `Image`, `Style`, ...
    pub kind: String,
    /// `true` for the page itself rather than something it loads.
    pub is_main_frame: bool,
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Pulls `file:line:col` out of an uncaught-exception message
/// (`Error at https://a.example/x.js:12:34 uncaught exception: ...`).
#[must_use]
pub fn source_of(message: &str) -> Option<String> {
    let rest = message.strip_prefix("Error at ")?;
    let location = rest.split_whitespace().next()?;
    let mut parts = location.rsplitn(3, ':');
    let col = parts.next()?;
    let line = parts.next()?;
    let file = parts.next()?;
    (col.chars().all(|c| c.is_ascii_digit()) && line.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{file}:{line}:{col}"))
}

/// Appends to a bounded queue.
pub fn push_bounded<T>(queue: &mut VecDeque<T>, item: T) {
    if queue.len() >= MAX_ENTRIES {
        queue.pop_front();
    }
    queue.push_back(item);
}

/// A panic on one of the engine's (or the app's) threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanicNote {
    pub at_ms: u64,
    pub thread: String,
    pub message: String,
    pub location: String,
}

static PANICS: Mutex<Vec<PanicNote>> = Mutex::new(Vec::new());

/// Records a panic for the UI to show (called from the process's panic hook).
pub fn record_panic(thread: &str, message: &str, location: &str) {
    if let Ok(mut panics) = PANICS.lock() {
        if panics.len() < 200 {
            panics.push(PanicNote {
                at_ms: now_ms(),
                thread: thread.to_string(),
                message: message.to_string(),
                location: location.to_string(),
            });
        }
    }
}

/// Panics recorded since the last call.
#[must_use]
pub fn take_panics() -> Vec<PanicNote> {
    PANICS
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Things a page asks the browser to show
// ---------------------------------------------------------------------------

/// The pointer a page asks for (`cursor:` in CSS), reduced to what a UI
/// toolkit can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageCursor {
    #[default]
    Default,
    Hidden,
    Pointer,
    Text,
    Crosshair,
    Grab,
    Grabbing,
    Move,
    NotAllowed,
    Wait,
    Help,
    ZoomIn,
    ZoomOut,
    ResizeHorizontal,
    ResizeVertical,
    ResizeDiagonalUp,
    ResizeDiagonalDown,
}

/// A rectangle in device pixels, in the page's own coordinates (the same space
/// as the frame).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DeviceRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// One choice in a `<select>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOptionView {
    /// What to answer with to pick it.
    pub index: usize,
    pub label: String,
    pub disabled: bool,
    pub selected: bool,
    /// The `<optgroup>` label it sits under, if any.
    pub group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

/// One row of a context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItemView {
    /// What to answer with to pick it; `None` for a separator.
    pub index: Option<usize>,
    pub label: String,
    pub enabled: bool,
}

/// Something the page needs a person to deal with before it can go on. The
/// page's text in it (a dialog's message, an option's label) is untrusted:
/// show it as plain text, framed so it cannot pass for browser UI.
#[derive(Debug, Clone, PartialEq)]
pub enum PageControl {
    /// A `<select>` dropdown or list.
    Select {
        options: Vec<SelectOptionView>,
        multiple: bool,
        anchor: DeviceRect,
    },
    /// `alert()`, `confirm()` or `prompt()`.
    Dialog {
        kind: DialogKind,
        message: String,
        /// A prompt's starting text.
        default: String,
    },
    /// `<input type=file>`.
    File { multiple: bool, accept: Vec<String> },
    /// `<input type=color>`, with the current colour as `#rrggbb`.
    Color { current: String, anchor: DeviceRect },
    /// A right-click menu.
    Menu {
        items: Vec<MenuItemView>,
        anchor: DeviceRect,
    },
    /// HTTP authentication (a `401` or, for a proxy, `407` with a
    /// `WWW-Authenticate` challenge): a username and password for `host`.
    Auth { host: String, for_proxy: bool },
}

/// A person's answer to a [`PageControl`].
#[derive(Debug, Clone, PartialEq)]
pub enum ControlAnswer {
    /// The chosen option indices of a `<select>`.
    Select(Vec<usize>),
    /// OK / Yes; a prompt's text.
    Accept(Option<String>),
    /// Cancel, or closed without choosing.
    Dismiss,
    /// The files picked.
    Files(Vec<std::path::PathBuf>),
    /// A colour as `#rrggbb`.
    Color(String),
    /// A context-menu row by its index.
    Menu(usize),
    /// A username and password for an [`PageControl::Auth`] prompt.
    Credentials {
        username: String,
        password: Password,
    },
}

/// A password on its way to the engine. Its `Debug` form never shows it, so
/// it cannot reach a log through a `{:?}` of the answer it is in.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(pub String);

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(••••)")
    }
}

/// A page's process or script thread died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashNote {
    pub at_ms: u64,
    pub reason: String,
    pub backtrace: Option<String>,
}

/// Parses `#rgb` or `#rrggbb` into its channels.
#[must_use]
pub fn parse_hex_color(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.trim().strip_prefix('#')?;
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        6 => Some((
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
        )),
        3 => {
            let d = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
            Some((d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_of_an_uncaught_exception_is_pulled_out() {
        assert_eq!(
            source_of("Error at https://a.example/x.js:12:34 uncaught exception: Boom"),
            Some("https://a.example/x.js:12:34".to_string())
        );
        assert_eq!(source_of("just a log line"), None);
        assert_eq!(
            source_of("Error at :0:0 Script error."),
            Some(":0:0".to_string())
        );
        assert_eq!(source_of("Error at nolocation here"), None);
    }

    #[test]
    fn a_full_queue_drops_the_oldest() {
        let mut q = VecDeque::new();
        for i in 0..(MAX_ENTRIES + 3) {
            push_bounded(&mut q, i);
        }
        assert_eq!(q.len(), MAX_ENTRIES);
        assert_eq!(q.front(), Some(&3));
    }

    #[test]
    fn hex_colours_parse_in_both_lengths_and_refuse_junk() {
        assert_eq!(parse_hex_color("#ff8000"), Some((255, 128, 0)));
        assert_eq!(parse_hex_color(" #f80 "), Some((255, 136, 0)));
        assert_eq!(parse_hex_color("ff8000"), None);
        assert_eq!(parse_hex_color("#ff80"), None);
        assert_eq!(parse_hex_color("#gg0000"), None);
    }

    #[test]
    fn levels_order_by_severity() {
        assert!(ConsoleLevel::Error > ConsoleLevel::Warn);
        assert!(ConsoleLevel::Warn > ConsoleLevel::Log);
    }

    #[test]
    fn panics_are_taken_once() {
        record_panic("Script#3", "boom", "a.rs:1:2");
        let first = take_panics();
        assert!(first.iter().any(|p| p.message == "boom"));
        assert!(!take_panics().iter().any(|p| p.message == "boom"));
    }
}
