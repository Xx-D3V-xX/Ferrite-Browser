// ferrite-ui — Iced UI shell for Ferrite Browser.
//
// ## Architecture
//
// Iced 0.13 functional builder API — no trait to implement. State is
// `FerriteBrowser`, messages are `FerriteBrowserMessage`, and the three
// free functions `update`, `view`, `subscription` are passed to the builder.
//
// ## Interaction model
//
// The Servo frame is rendered as an `iced_widget::image` inside a
// `mouse_area` that captures mouse move, mouse press, mouse release, and
// wheel events and forwards them to `HeadlessServoSession` as native Servo
// input events (`InputEvent::MouseMove`, `MouseButton`, `Wheel`).
//
// ## Coordinate spaces (C3a)
//
// `mouse_area`/`responsive` report positions and sizes in iced's logical
// points; `HeadlessServoSession`'s render buffer and `send_mouse_*`/
// `send_scroll`'s `DevicePoint` coordinates are physical pixels. Two
// fields bridge that gap: `scale_factor` (physical px per logical point,
// fetched once via `iced::window::get_scale_factor` in `launch()`) scales
// every pointer-event coordinate before it reaches the session, and
// `content_area_size` (the content container's true logical size, measured
// by wrapping it in `iced_widget::responsive` in `view()`) drives
// `ServoFrame`'s tick handler to keep the active tab's session buffer
// resized to match. Before this, the buffer stayed at its hardcoded
// startup size forever regardless of window size or panel visibility,
// which is what made the displayed frame blurry (stretched to fill a
// differently-sized area) and put the cursor Servo actually saw somewhere
// other than where it visually was.
//
// ## Live agent execution (B3, docs/TO-DO.md T-224/T-220/T-229)
//
// The agent loop drives `ferrite_agent::browser_loop::{AgentAction,
// execute_action}` against a `ferrite_engine_servo::BorrowedServoEngine`
// wrapping the active tab's own `HeadlessServoSession` — the same session
// this file's own Servo-frame code already drives correctly via a real
// winit event loop (Iced's), which is exactly the ingredient
// `ferrite_engine_servo::ServoEngine`'s own standalone conformance tests
// found missing (`docs/TO-DO.md` T-220).
//
// `ferrite_engine::BrowserEngine` deliberately has no `Send` bound (see
// that trait's own module docs) because the real engine wraps `Rc`-based
// Servo state — so it cannot be moved into a `tokio::spawn`ed background
// task, which rules out calling `browser_loop::run_agent_loop` directly
// against the live engine (unlike the dry run's `DryRunEngine`, which holds
// no such state and *is* driven by a direct `run_agent_loop` call — see
// `BrowserLoopDryRunDriver` below). Instead, only the per-step model round
// trip (`&dyn ModelProvider`, `Send + Sync`) is spawned in the background;
// each returned `AgentAction` is dispatched against the live engine
// synchronously, on the Iced update thread, via `execute_action` — see the
// `AgentStepReady` handler in `update()`. `docs/handoffs/b03.md` has the
// full reasoning for this deviation from calling `run_agent_loop` directly.
//
// ## Chats, context and the fast lane (agent-context-chats-laya)
//
// The agent panel is a chat (`agent_panel.rs`). `FerriteBrowser::chat` is the
// durable `ferrite_agent::chat::Chat`; `agent_log`/`agent_response`/
// `agent_is_running`/`live_loop` are the *current run's* live mirror. Every
// way a run ends goes through `conclude_run`, which finishes the turn, saves
// the chat atomically (a failure is a notice, never fatal) and updates the
// in-memory list. Chats are read from disk only in `launch()`.
//
// On submit (`submit_task`) the run is seeded: the chat so far, the open tabs
// and (if the message is about it) the page digest, read on this thread via
// the borrowed engine's `observe_page`, become the live loop's first message
// (`pending_seed`). **The injection defense never sees that seed** — it gets
// `trusted_task_text` (user words only), built by
// `agent_run::ipi_task_for_run`; see the comment in `submit_task`.
//
// The borrowed engine cannot manage tabs, so `open_tab`/`switch_tab`/
// `close_tab`/`list_tabs` are performed here (`run_tab_action`) through the
// same helpers as the tab strip, after the same consent check.
//
// With `FERRITE_LAYA_URL` set, the optional Laya fast lane may pick a step
// without an LLM call (`spawn_next_step` -> `agent_run::try_fast_lane` ->
// `FastStepReady`); the step then takes exactly the same handler, consent
// check and loop-safety limits as an LLM step. Off by default.
//
// ## Keyboard shortcuts (platform-aware)
//   macOS : Cmd+T/W/R/L/J/F/D, Cmd+=/-/0 (zoom), Cmd+[ / ] (back/forward),
//           Cmd+Shift+[ / ] and Ctrl+Tab (previous/next tab), Cmd+1-8/9 (tab
//           by number / last), Cmd+Shift+A (agent), Cmd+Shift+O (new agent
//           chat), Cmd+, (settings), Cmd+Opt+I (developer tools; Ctrl+Shift+I
//           elsewhere), F5, F12 (audit log), Alt+←/→, Esc
//   other : the same with Ctrl in place of Cmd
//
// Esc is context-sensitive (C3d): it closes the find bar first if one is
// open (`show_find_bar`), otherwise it falls through to its pre-existing
// meaning (stop loading, or unfocus the address bar) — resolved in
// `update()`'s `EscapePressed` handler, not in `handle_key_press` itself
// (`iced::keyboard::on_key_press` requires a plain `fn` pointer with no
// state access — see that function's own doc comment).
//
// ## Bookmarks/history/zoom/find-in-page/settings/downloads (C3d)
//
// Six features added on top of C3c's palette/theme system, sharing its
// conventions throughout (every new colour is `state.palette()`/
// `palette_for_theme(theme)`, no new hardcoded `Color` literals):
// - **Bookmarks** are pure UI + JSON persistence (`Bookmark`,
//   `load_bookmarks_from`/`save_bookmarks_to`, injected `&Path` for
//   testability, `default_bookmarks_path()` resolving the real one via
//   `dirs::home_dir()` — loaded once in `launch()`, never in `Default`, the
//   same test-safety discipline the model connection already
//   established for the model provider).
// - **History** has two independent halves, deliberately not one. Per-tab
//   `GoBack`/`GoForward`/`can_go_back`/`can_go_forward` now read Servo's own
//   real native session history (`HeadlessServoSession::history()`, backed
//   by `WebViewDelegate::notify_history_changed` — see `ferrite-servo`'s
//   `session.rs`), which replaced an approximated `Complete`-event counter
//   that could never tell forward history apart at all
//   (`can_go_forward()` used to just hardcode `false`) — a real correctness
//   fix, not new UI. The History **panel**'s browser-wide, recency-ordered
//   `FerriteBrowser::history` list is populated separately, from
//   `LoadStatusChanged`'s own already-existing "did this tab's URL really
//   change" signal (`record_history_visit`) rather than by reading
//   `session.history()` on every tick — simpler than reconciling N per-tab
//   Servo history lists into one recency-ordered view, and this crate's own
//   already-verified-working signal, rather than building the panel's
//   contents on top of `notify_history_changed`'s data shape, which (like
//   the rest of `ferrite-servo`'s `servo`-feature code) this session could
//   only verify by reading the pinned source, not by compiling it. Session-
//   only: not persisted across restarts (see `history`'s doc comment for
//   the honest reasoning).
// - **Zoom** is a per-tab `Vec<f32>` (`tab_zoom`, indexed exactly like
//   `tab_titles`/`tab_urls`/`tab_favicons`/`tab_favicons`), applied via a
//   CSS `transform: scale(...)` injected through `execute_js` (Servo has no
//   native zoom API at this pinned version — see the session's `set_zoom`
//   comment) and surfaced in the address bar only when it isn't 100% (a
//   chip; the overflow menu has the +/- control), plus a `default_zoom`
//   setting for new tabs in the Settings tab.
// - **Find-in-page** is a dismissible overlay-styled bar (not a permanent
//   toolbar element), driving a standards-based DOM search
//   (`document.createTreeWalker`/`Range`, no `window.find()`) via the same
//   `execute_js` path zoom uses — see `find_script`/`find_navigate_script`.
// - **Settings** is a read-only view of the resolved model config
//   (`model_tag_small`/`model_tag_main`/`model_cache_dir`) plus real working
//   toggles for the state this crate already has (theme, default zoom) — no
//   free-text model-tag entry, per `CLAUDE.md` §10.2's no-hardcoded/no-
//   silently-bypassed-config rule.
// - **Downloads** are a small, self-contained manager local to this crate
//   (`DownloadItem`/`DownloadState`, `run_download`): a `tokio::spawn`ed
//   `reqwest` GET streamed to `dirs::download_dir()`, reporting progress
//   through the same `agent_event_tx`/`agent_event_rx` channel
//   `spawn_next_step` already uses for its own background-task-to-UI
//   messages — deliberately independent of `ferrite_engine::BrowserEngine::
//   download()` (the agent's dry-run/consent-gated tool vocabulary, a
//   different concern entirely — see `docs/DECISIONS.md`'s IPI/consent
//   boundary ADRs). Trigger is a manual "Download current page" action in
//   the Library panel's Downloads tab; in-page `<a download>` click
//   interception is out of scope for this pass (stated here, not silently
//   dropped).
//
// Bookmarks/history/downloads share one drawer — the Library panel
// (`show_library_panel`/`library_tab`) — reached from the toolbar's overflow
// menu rather than from four more permanent toolbar buttons; Settings is a
// drawer of its own, also from that menu. See `chrome.rs`.
//
// ## New-tab hero: real quick-access-tile favicons
//
// `new_tab_page`'s six quick-access tiles (DuckDuckGo/Rust Docs/GitHub/
// Servo/Hacker News/Wikipedia) used to render a literal `"[D]"`/`"[R]"`/...
// bracketed-letter string as a placeholder "icon". They now show each
// site's real favicon, fetched directly from that site's own
// `https://<host>/favicon.ico` (never a third-party favicon-aggregator
// service — this project's whole identity is a privacy-conscious,
// capability-governed browser, and silently routing every new-tab-page load
// through a third party that then knows which sites this user's quick-
// access tiles name would contradict that directly).
//
// Mechanism, deliberately reusing three already-established patterns rather
// than inventing new ones:
// - **Fetch + cache + decode** (`fetch_tile_favicon`, spawned once per tile
//   by the new `FetchTileFavicons` message, itself sent once at real startup
//   — `launch()`'s startup `Task::batch`, mirroring `ServoReady`) is the
//   same "spawn the I/O in the background, report back over
//   `agent_event_tx`" shape `run_download` (C3d) already established: a
//   `reqwest::Client::get` of the site's own `/favicon.ico`, decoded via the
//   new `image` crate dependency (see `Cargo.toml`'s own comment on why this
//   adds no new dependency *version* to the graph — `libservo` already pulls
//   it in transitively) into raw RGBA8 (`decode_favicon_rgba`), reported back
//   as a `TileFaviconReady { index, width, height, rgba }` message that
//   constructs the actual `iced_widget::image::Handle` in `update()` — the
//   same "raw bytes over the channel, `ImageHandle::from_rgba` only inside
//   `update()`" shape the tab-bar favicon sync (C3b) already uses.
// - **On-disk cache** (`favicon_cache_dir`/`favicon_cache_path`,
//   `~/.cache/ferrite-ui/favicons/<host>.ico`) follows the same `.cache`-vs-
//   `.local/share` distinction `default_bookmarks_path`/
//   `ferrite_model::config::default_cache_dir()` already establish — a
//   favicon is re-fetchable, unlike a bookmark, so it belongs under
//   `.cache`, not the data directory. Checked before every fetch; written
//   only once decoding the fetched bytes has actually succeeded, so a
//   transient bad response (an HTML error page, a truncated body) is never
//   cached as if it were a real icon and can be retried on the next launch.
// - **Fallback** (monogram tile): a tile with no resolved
//   favicon yet — no network, first launch before the fetch completes, the
//   site's `/favicon.ico` doesn't resolve at all, or a decode failure —
//   shows a deliberately-designed monogram (the site's first letter in the
//   palette's dim text colour)
//   rather than a broken-image glyph or empty space, the same fallback real
//   browsers' own "top sites" tiles use before a favicon is cached. This is
//   the *default* rendering path, not an error state the UI has to detect
//   and react to — `tile_favicons[i]` simply stays `None` until/unless a
//   fetch succeeds, and `new_tab_page` always has a finished-looking tile to
//   show either way.
//
// **Never verified against a real network fetch in this sandbox** — see
// `docs/PROGRESS.md`'s entry for this change for exactly what was and
// wasn't proven (this sandbox's own outbound proxy blocks every one of the
// six real sites' domains outright, confirmed via `curl`, before any Rust
// code here was ever reached).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod activity;
mod activity_panel;
mod agent_panel;
mod agent_run;
mod chrome;
mod controls;
mod crash;
mod devtools;
mod devtools_panel;
mod icons;
mod identity;
mod layout;
mod lifecycle;
mod markdown;
mod page_input;
mod page_view;
mod pages;
mod perf;
mod permission;
mod runtime_guard;
mod scroll;
mod settings_panel;
mod signin;
mod tab_diag;
mod tokens;
mod widgets;
use activity_panel::AuditTab;
use icons::{icon, Icon};
use pages::new_tab_page;
pub(crate) use tokens::{
    accent_btn_style, close_btn_style, panel_btn_active, panel_btn_inactive, separator_style,
};
use tokens::{
    bottom_panel_style, page_style, tip, toolbar_btn_style, RADIUS_SM, SP_MD, SP_SM, SP_XL, SP_XS,
    TEXT_BODY, TEXT_CAPTION, TEXT_SMALL, TEXT_TITLE,
};

use ferrite_agent::browser_loop::{
    compact_observation, execute_action, run_agent_loop, trim_message_history,
    with_page_observation, AgentAction, LoopBudget, LoopStopReason, AGENT_LOOP_NUM_PREDICT,
    MAX_CONSECUTIVE_MALFORMED_STEPS, SYSTEM_PROMPT, SYSTEM_PROMPT_VERSION,
};
use ferrite_agent::chat::{
    default_chats_dir, Chat, ChatId, ChatStore, ChatSummary, Outcome, StepRecord,
};
use ferrite_agent::context::{trusted_task_text, ContextMode};
use ferrite_agent::decider::{FastAction, HistoryItem, LayaStepDecider};
use ferrite_audit_log::{AuditEntry, AuditEventKind, PersistentAuditLog};
use ferrite_engine::{BrowserEngine, PageDigest};
use ferrite_engine_servo::BorrowedServoEngine;
use ferrite_ipi::comparator::{compare, ConsentDecision, ExpectedFingerprint, FingerprintDiff};
use ferrite_ipi::dry_run::DryRunRecord;
use ferrite_ipi::tool_decision::{DefenseMode, LoopOutcome, ToolDecisionEngine, ToolId};
use ferrite_ipi::IpiTask;
use ferrite_model::{CompletionRequest, Message, ModelProvider, ModelTier, SamplingOptions};
use ferrite_servo::session::{HeadlessServoSession, LoadStatus};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, stack, text, text_input,
};
use iced::{
    keyboard, time, window, Background, Border, Color, Element, Font, Length, Padding, Size,
    Subscription, Task, Theme,
};
use iced_widget::image::{Handle as ImageHandle, Image as ServoImage};
use iced_widget::responsive;
use std::cell::Cell;

// ---------------------------------------------------------------------------
// C3d — pure data types: bookmarks, history, zoom, downloads.
//
// Kept together, ahead of the `FerriteBrowser` struct they're fields of,
// matching where `AgentLogEntry`/`StepFailure`/`LiveAgentLoop` already sit
// relative to it.
// ---------------------------------------------------------------------------

/// One saved bookmark. Serde-derived for JSON persistence
/// (`load_bookmarks_from`/`save_bookmarks_to`) — see this module's own doc
/// comment for the persistence story.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

/// One visited page, as folded into `FerriteBrowser::history` — see that
/// field's doc comment for why this is a browser-wide list rather than
/// Servo's own per-tab session history verbatim.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visited_at: chrono::DateTime<chrono::Utc>,
}

/// Which sub-view the Library panel currently shows. They are reached from the
/// toolbar's overflow menu (`chrome::menu_overlay`) rather than from a toolbar
/// button each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryTab {
    #[default]
    Bookmarks,
    History,
    Downloads,
}

/// One entry in `FerriteBrowser::downloads` — a page fetched by the
/// "Download current page" action (Library panel, Downloads tab), streamed
/// to disk by `run_download`.
#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub id: u64,
    pub url: String,
    pub file_name: String,
    pub path: PathBuf,
    pub state: DownloadState,
}

/// Progress state of one [`DownloadItem`], advanced by the
/// `DownloadProgress`/`DownloadCompleted`/`DownloadFailed` messages
/// `run_download` sends back over `agent_event_tx`.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadState {
    InProgress {
        downloaded_bytes: u64,
        /// `None` when the server's response carried no `Content-Length`
        /// (e.g. chunked transfer encoding) — the UI then shows bytes
        /// downloaded so far without a percentage, rather than guessing.
        total_bytes: Option<u64>,
    },
    Completed,
    Failed(String),
}

/// One entry in the new-tab hero's quick-access tile row — see this module's
/// own doc comment ("New-tab hero: real quick-access-tile favicons") for the
/// fetch/cache/fallback mechanism built around this list. `host` is the
/// exact host `favicon_url` and `NavigateRequested` both derive from `url`
/// (kept as a single source of truth rather than a separately-authored
/// string, so the two can never drift apart) — see `favicon_host`.
struct QuickAccessTile {
    label: &'static str,
    url: &'static str,
}

/// The new-tab hero's six quick-access tiles — unchanged set/order from the
/// pre-redesign `tiles: Vec<(&str, &str, &str)>` literal this replaces, only
/// the bracketed-letter placeholder (`"[D]"`/`"[R]"`/...) is gone, since a
/// tile's actual glyph is now either its fetched favicon or a monogram
/// derived from `label` at render time (`new_tab_page`), never authored
/// per-tile here.
const QUICK_ACCESS_TILES: [QuickAccessTile; 6] = [
    QuickAccessTile {
        label: "Google",
        url: "https://www.google.com",
    },
    QuickAccessTile {
        label: "Rust Docs",
        url: "https://doc.rust-lang.org",
    },
    QuickAccessTile {
        label: "GitHub",
        url: "https://github.com",
    },
    QuickAccessTile {
        label: "Servo",
        url: "https://servo.org",
    },
    QuickAccessTile {
        label: "Hacker News",
        url: "https://news.ycombinator.com",
    },
    QuickAccessTile {
        label: "Wikipedia",
        url: "https://en.m.wikipedia.org",
    },
];

// ---------------------------------------------------------------------------
// Live agent-action vocabulary helpers — map `AgentAction` (the loop's real
// action enum) onto `ferrite_core::Primitive`/`ToolId`, the same wire
// vocabulary the fingerprint/comparator/consent machinery already speaks,
// rather than a second, independent one.
// ---------------------------------------------------------------------------

fn primitive_of_action(action: &AgentAction) -> ferrite_core::Primitive {
    use ferrite_core::Primitive;
    match action {
        AgentAction::Navigate { .. }
        | AgentAction::GoBack
        | AgentAction::GoForward
        | AgentAction::Reload => Primitive::Navigate,
        AgentAction::ReadDom
        | AgentAction::ReadPage
        | AgentAction::ReadText { .. }
        | AgentAction::FindText { .. }
        | AgentAction::ExtractLinks { .. }
        | AgentAction::ListTabs => Primitive::DomRead,
        AgentAction::Query { .. } => Primitive::DomQuery,
        // Same conservative mapping as `ferrite_engine::Call::primitive`:
        // hover/set_checked/submit_form are never weaker than a click.
        AgentAction::Click { .. }
        | AgentAction::Hover { .. }
        | AgentAction::SetChecked { .. }
        | AgentAction::SubmitForm { .. } => Primitive::Click,
        AgentAction::TypeText { .. }
        | AgentAction::SelectOption { .. }
        | AgentAction::PressKey { .. } => Primitive::DomWrite,
        AgentAction::FillForm { .. } => Primitive::FormFill,
        AgentAction::Scroll { .. } | AgentAction::ScrollTo { .. } => Primitive::Scroll,
        AgentAction::WaitForSelector { .. }
        | AgentAction::WaitIdle
        | AgentAction::WaitMs { .. } => Primitive::Wait,
        AgentAction::OpenTab { .. } => Primitive::TabOpen,
        AgentAction::CloseTab { .. } => Primitive::TabClose,
        AgentAction::SwitchTab { .. } => Primitive::Navigate,
        AgentAction::Screenshot => Primitive::Screenshot,
        AgentAction::Download { .. } => Primitive::Download,
        AgentAction::ClipboardRead => Primitive::ClipboardRead,
        AgentAction::ClipboardWrite { .. } => Primitive::ClipboardWrite,
        AgentAction::JsExecute { .. } => Primitive::JsExecute,
        AgentAction::Finish { .. } | AgentAction::AskUser { .. } => {
            unreachable!(
                "Finish/AskUser are intercepted before an action is ever dispatched or logged"
            )
        }
    }
}

/// `ToolId` for a live `AgentAction` — the same string vocabulary the
/// comparator/consent panel already key on (`ferrite_core::Primitive::as_str()`).
fn action_tool_id(action: &AgentAction) -> ToolId {
    ToolId::new(primitive_of_action(action).as_str())
}

/// The URL an `AgentAction` itself carries, if any — the only actions that
/// can be checked against a rejected origin without a live session (the
/// same honest limitation the pre-B3 `FilteredToolExecutor::tool_url` had:
/// an action with no URL of its own acts on "whatever the active tab
/// currently is," which cannot be checked from the action alone).
fn action_url(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::Navigate { url } | AgentAction::Download { url } => Some(url.as_str()),
        // A tab opened at a rejected origin is a navigation to it.
        AgentAction::OpenTab { url: Some(url) } => Some(url.as_str()),
        _ => None,
    }
}

/// Normalizes a URL to `scheme://host`, lowercased — the same shape
/// `ferrite_ipi::dry_run::record::extract_origin` produces, reimplemented
/// locally since that function is crate-private to `ferrite-ipi`. Returns
/// `None` if `url` does not parse as an absolute URL with a host.
fn origin_of_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(
        match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        }
        .to_ascii_lowercase(),
    )
}

/// Whether `action` is blocked by the user's consent decision — checked
/// before every live-loop step executes. Real enforcement, not only a UI
/// filter: a rejected action never reaches `execute_action` (see
/// `a_rejected_tool_id_blocks_the_action_before_it_reaches_the_engine` and
/// `a_rejected_origin_blocks_a_navigate_before_it_reaches_the_engine`).
fn is_action_rejected(
    action: &AgentAction,
    rejected: &std::collections::HashSet<ToolId>,
    rejected_origins: &std::collections::HashSet<String>,
) -> bool {
    if rejected.contains(&action_tool_id(action)) {
        return true;
    }
    if let Some(origin) = action_url(action).and_then(origin_of_url) {
        if rejected_origins.contains(&origin) {
            return true;
        }
    }
    false
}

/// C2: a short, human-readable name for `action`'s kind — e.g. "Navigate",
/// "Click" — shown as the step's title in the agent sidebar's activity
/// feed. Paired with [`action_detail`] (the action's own parameter, if
/// any) and [`icon_for_action`] (its icon), replacing the old flat
/// `[primitive] detail` single-string log line (B3-era) with three
/// separately-styleable pieces.
fn action_label(action: &AgentAction) -> &'static str {
    match action {
        AgentAction::Navigate { .. } => "Navigate",
        AgentAction::GoBack => "Go back",
        AgentAction::GoForward => "Go forward",
        AgentAction::Reload => "Reload",
        AgentAction::ReadDom => "Read page",
        AgentAction::Query { .. } => "Query elements",
        AgentAction::ReadText { .. } => "Read text",
        AgentAction::Click { .. } => "Click",
        AgentAction::TypeText { .. } => "Type text",
        AgentAction::FillForm { .. } => "Fill form",
        AgentAction::SelectOption { .. } => "Select option",
        AgentAction::Scroll { .. } => "Scroll",
        AgentAction::WaitForSelector { .. } => "Wait for element",
        AgentAction::WaitIdle => "Wait",
        AgentAction::Screenshot => "Screenshot",
        AgentAction::Download { .. } => "Download",
        AgentAction::ClipboardRead => "Read clipboard",
        AgentAction::ClipboardWrite { .. } => "Write clipboard",
        AgentAction::JsExecute { .. } => "Run JavaScript",
        AgentAction::Finish { .. } => "Finish",
        AgentAction::ReadPage => "Read page",
        AgentAction::PressKey { .. } => "Press key",
        AgentAction::Hover { .. } => "Hover",
        AgentAction::SetChecked { .. } => "Set checkbox",
        AgentAction::ScrollTo { .. } => "Scroll to element",
        AgentAction::FindText { .. } => "Find text",
        AgentAction::ExtractLinks { .. } => "List links",
        AgentAction::SubmitForm { .. } => "Submit form",
        AgentAction::WaitMs { .. } => "Wait",
        AgentAction::OpenTab { .. } => "Open tab",
        AgentAction::SwitchTab { .. } => "Switch tab",
        AgentAction::CloseTab { .. } => "Close tab",
        AgentAction::ListTabs => "List tabs",
        AgentAction::AskUser { .. } => "Ask you",
    }
}

/// C2: `action`'s own parameter, rendered as the step's detail line (a URL,
/// a selector, the text typed, ...) — empty for an action with nothing
/// further to show (`GoBack`, `WaitIdle`, ...). See [`action_label`].
fn action_detail(action: &AgentAction) -> String {
    match action {
        AgentAction::Navigate { url } | AgentAction::Download { url } => url.clone(),
        AgentAction::Query { selector }
        | AgentAction::ReadText { selector }
        | AgentAction::Click { selector }
        | AgentAction::WaitForSelector { selector } => selector.clone(),
        AgentAction::TypeText { selector, text } => format!("{selector} \u{2192} \"{text}\""),
        AgentAction::SelectOption { selector, value } => format!("{selector} \u{2192} {value}"),
        AgentAction::FillForm { fields } => format!("{} field(s)", fields.len()),
        AgentAction::Scroll { dx, dy } => format!("({dx}, {dy})"),
        AgentAction::ClipboardWrite { text } => text.clone(),
        AgentAction::JsExecute { script } => script.clone(),
        AgentAction::Finish { answer } => answer.clone(),
        AgentAction::Hover { selector }
        | AgentAction::ScrollTo { selector }
        | AgentAction::SetChecked { selector, .. } => selector.clone(),
        AgentAction::PressKey { selector, key } => match selector {
            Some(selector) => format!("{key} \u{2192} {selector}"),
            None => key.clone(),
        },
        AgentAction::FindText { text } => text.clone(),
        AgentAction::ExtractLinks { selector } | AgentAction::SubmitForm { selector } => {
            selector.clone().unwrap_or_default()
        }
        AgentAction::WaitMs { ms } => format!("{ms} ms"),
        AgentAction::OpenTab { url } => url.clone().unwrap_or_default(),
        AgentAction::SwitchTab { tab } | AgentAction::CloseTab { tab } => format!("tab {tab}"),
        AgentAction::AskUser { question } => question.clone(),
        AgentAction::GoBack
        | AgentAction::GoForward
        | AgentAction::Reload
        | AgentAction::ReadDom
        | AgentAction::ReadPage
        | AgentAction::ListTabs
        | AgentAction::WaitIdle
        | AgentAction::Screenshot
        | AgentAction::ClipboardRead => String::new(),
    }
}

/// C2: the icon shown next to `action`'s step in the agent sidebar's
/// activity feed. Grouped by the same action-class boundaries
/// `ferrite_core::Primitive`'s own taxonomy uses (see `Icon`'s doc
/// comment) rather than one icon per raw `AgentAction` variant, so the
/// icon set stays small and each glyph stays visually distinct at the
/// sidebar's render size. `Finish` reuses `Icon::Approve` (its outcome —
/// the task completing) and `JsExecute` reuses `Icon::Console` (already
/// this crate's "code"/JS glyph, used by the JS-console toggle button) —
/// deliberate reuse, not a placeholder, since both already mean exactly
/// this elsewhere in the same chrome.
fn icon_for_action(action: &AgentAction) -> Icon {
    match action {
        AgentAction::Navigate { .. }
        | AgentAction::GoBack
        | AgentAction::GoForward
        | AgentAction::Reload => Icon::Navigate,
        AgentAction::ReadDom
        | AgentAction::ReadPage
        | AgentAction::ReadText { .. }
        | AgentAction::Query { .. }
        | AgentAction::FindText { .. }
        | AgentAction::ExtractLinks { .. }
        | AgentAction::ListTabs => Icon::Read,
        AgentAction::Click { .. }
        | AgentAction::Hover { .. }
        | AgentAction::SetChecked { .. }
        | AgentAction::SubmitForm { .. } => Icon::Click,
        AgentAction::TypeText { .. }
        | AgentAction::SelectOption { .. }
        | AgentAction::FillForm { .. }
        | AgentAction::PressKey { .. } => Icon::Write,
        AgentAction::Scroll { .. }
        | AgentAction::ScrollTo { .. }
        | AgentAction::WaitForSelector { .. }
        | AgentAction::WaitIdle
        | AgentAction::WaitMs { .. }
        | AgentAction::Screenshot => Icon::Activity,
        AgentAction::OpenTab { .. }
        | AgentAction::SwitchTab { .. }
        | AgentAction::CloseTab { .. } => Icon::Navigate,
        AgentAction::AskUser { .. } => Icon::Agent,
        AgentAction::Download { .. } => Icon::Download,
        AgentAction::ClipboardRead | AgentAction::ClipboardWrite { .. } => Icon::Clipboard,
        AgentAction::JsExecute { .. } => Icon::Console,
        AgentAction::Finish { .. } => Icon::Approve,
    }
}

// ---------------------------------------------------------------------------
// Dry-run driver: runs the real agent-decision loop
// (`ferrite_agent::browser_loop::run_agent_loop`) directly against
// `ferrite_ipi::dry_run::DryRunEngine`.
//
// This is the simplification B1's own handoff anticipated (docs/handoffs/
// b01.md, b02.md): `DryRunEngine` (unlike the live `BorrowedServoEngine`)
// holds no `Rc`/thread-local state — it is a plain, `Send`-safe recorder —
// so `run_agent_loop` can drive it directly inside the background tokio
// task the dry run already runs on, with zero bridging code. This replaces
// the old GeminiAgent/`EngineToolExecutor`-bridged `DryRunAgentDriver`.
// ---------------------------------------------------------------------------

struct BrowserLoopDryRunDriver<'a> {
    provider: &'a dyn ModelProvider,
    model_tag: String,
    prompt: String,
}

#[async_trait::async_trait]
impl<'a> ferrite_ipi::dry_run::DryRunDriver for BrowserLoopDryRunDriver<'a> {
    async fn drive(&self, engine: &mut ferrite_ipi::dry_run::DryRunEngine) -> Result<(), String> {
        let clock = ferrite_core::SystemClock;
        let result = run_agent_loop(
            self.provider,
            engine,
            &clock,
            &self.model_tag,
            ModelTier::Main,
            &self.prompt,
            LoopBudget::default(),
        )
        .await;
        // A model error means the dry run genuinely could not decide what
        // to do (e.g. no provider configured/reachable) — surfaced as a
        // real error so the caller does not mistake "the model never
        // answered" for "the plan was clean" (CLAUDE.md's "fail to empty,
        // never a bypass": an empty dry-run record must never be produced
        // by a silently-swallowed model failure). Every other stop reason
        // (finished, a budget exhausted, a repeated action, an unparseable
        // action) is real, honest partial-or-complete data for the
        // orchestrator's own record — not an error.
        match result.stop_reason {
            LoopStopReason::ModelError(e) => Err(e),
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Platform detection — used for keyboard shortcut labels
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
const MOD_LABEL: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const MOD_LABEL: &str = "Ctrl";

/// Chrome's chord for developer tools, as the menu and the panel's header
/// spell it (`handle_key_press` accepts it; Cmd/Ctrl+J opens the same panel).
#[cfg(target_os = "macos")]
const DEVTOOLS_SHORTCUT: &str = "Cmd+Opt+I";
#[cfg(not(target_os = "macos"))]
const DEVTOOLS_SHORTCUT: &str = "Ctrl+Shift+I";

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------
//
// Every prior UI pass on this file (C1's design system through the new-tab
// hero redesign) styled colour, spacing, radius, and animation deliberately
// — but never once touched the font, so every one of those passes actually
// rendered in whatever generic sans-serif iced falls back to when no font is
// configured, not a considered typeface. Embedding one real, well-regarded
// UI font and setting it as the app's default is a single, small change
// with more effect on how "finished" this app looks than most individual
// widget-styling passes — text is a majority of every screen in a browser
// chrome.
//
// Inter (SIL Open Font License 1.1 — full text at `assets/fonts/OFL.txt`,
// the standard way to embed and redistribute an OFL font): a widely used,
// highly legible UI typeface designed specifically for screens at small
// sizes, already the de facto choice for exactly this kind of clean,
// modern app chrome (it is not a random pick — it is what most of the
// professional SaaS/app UIs this project's own "not cheap, aesthetic"
// goal is implicitly benchmarked against actually use). Four static
// weights are embedded, not Inter's variable-font file: iced's text
// shaper (cosmic-text/fontdb) selects the closest already-registered
// *face* for a requested `Font::weight` — it does not interpolate a
// variable font's weight axis — so multiple static faces is the
// mechanically correct way to get more than one real weight out of one
// family here, not an arbitrary choice to embed four separate files.

const FONT_INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const FONT_INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const FONT_INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const FONT_INTER_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");

/// The family name all four embedded Inter weights register themselves
/// under (their own font-file name table, not something assigned here) —
/// `launch()`'s `default_font` and every [`font_weight`] call go through
/// this one constant rather than repeating the literal.
const FONT_FAMILY: &str = "Inter";

/// [`Font::with_name(FONT_FAMILY)`] at a given `Weight` — the one place a
/// call site that wants a heavier embedded face (a heading, the wordmark,
/// emphasised label text) goes, so which of the four embedded weights
/// exist is never spelled out ad hoc at each call site.
const fn font_weight(weight: iced::font::Weight) -> Font {
    Font {
        weight,
        ..Font::with_name(FONT_FAMILY)
    }
}

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

const ADDRESS_BAR_ID: &str = "ferrite_address_bar";
const JS_INPUT_ID: &str = "ferrite_js_input";
const FIND_INPUT_ID: &str = "ferrite_find_input";

const BORDER_RADIUS: f32 = tokens::RADIUS_MD;
const PANEL_PADDING: u16 = 12;

/// Ticks (`ServoFrame`, ~16ms each) to wait after calling
/// `HeadlessServoSession::resize()` before trusting a frame read from that
/// session again — see `FerriteBrowser::resize_settle_ticks`'s doc comment.
/// The actual crash/corruption this workaround was first written for
/// turned out to have a real, different root cause, fixed directly in
/// `HeadlessServoSession::resize()` itself (a redundant, unguarded
/// `rendering_context.resize()` call that both suppressed Servo's own
/// resize-triggered repaint and skipped `make_current()` before touching
/// the surface — see that method's doc comment for the full trace against
/// the pinned `libservo` source). This constant is now a small residual
/// safety margin, not the primary fix, and is kept low (one tick) so it
/// doesn't itself make resizing feel less smooth.
const RESIZE_SETTLE_TICKS: u8 = 1;

/// Icon sizes — the two sizes every `icon()` call site in this crate picks
/// from, so the icon set reads as one consistent scale rather than a grab
/// bag of ad hoc pixel sizes. `ICON_SIZE` is the default (toolbar/tab-bar/
/// panel-toggle chrome); `ICON_SIZE_SM` is for icons paired tightly with
/// text at a smaller point size (consent-item rows, the tab close button).
const ICON_SIZE: f32 = 15.0;
const ICON_SIZE_SM: f32 = 12.0;

/// The engine tick while the page is busy (one display frame at 60 Hz)...
const ACTIVE_TICK: std::time::Duration = std::time::Duration::from_millis(16);
/// ...and, while it is sitting still, the longest wait for a tick when the
/// engine has not asked for one (see `engine_wakes`).
const IDLE_TICK: std::time::Duration = std::time::Duration::from_millis(1000);
/// Ticks without a new picture before the page counts as idle (about half a
/// second at the active rate).
const BUSY_TICKS: u8 = 30;
/// While the page is active the tick follows the display's frames; this slower
/// timer runs it anyway if no frame has come for this long (a minimised or
/// covered window is not redrawn, and the engine still has to be pumped).
const WATCHDOG_AFTER: std::time::Duration = std::time::Duration::from_millis(80);
/// The longest step animation takes for one tick, so a stall does not make the
/// loading bar jump.
const MAX_TICK_STEP: std::time::Duration = std::time::Duration::from_millis(100);
/// How far the loading-bar phase moves per second (it was 0.02 per 16 ms tick).
const PROGRESS_PER_SECOND: f32 = 1.2;

/// Advance per `ConsentPanelTick` for `FerriteBrowser::consent_panel_anim` —
/// ticks fire every 16ms (the same cadence `ServoFrame` already uses, see
/// `subscription()`), so this reaches 1.0 in ~200ms: the fast, subtle end of
/// the 150-250ms range typical for this kind of UI entrance transition.
const CONSENT_ANIM_STEP: f32 = 16.0 / 200.0;

/// Advance per `MenuAnimTick`: the menu settles in ~140 ms.
const MENU_ANIM_STEP: f32 = 16.0 / 140.0;

/// Advance per `ThreadAnimTick` for each entering thread item's progress —
/// ticks fire every 16ms, so an entrance takes ~220ms.
const THREAD_ANIM_STEP: f32 = 16.0 / 220.0;

// ---------------------------------------------------------------------------
// Colour palette — C3c: real light/dark theme
// ---------------------------------------------------------------------------
//
// Through C3b this crate had exactly one theme: twelve module-level `Color`
// constants (`C_BASE`/`C_SURFACE`/.../`C_DANGER`) referenced by bare ident
// from `view()` and its helpers. `Palette` keeps the same twelve roles
// (lowercased, as struct fields) resolved per [`AppTheme`] instead of fixed
// at compile time — every call site's *meaning* is unchanged, only the
// lookup is now runtime.
//
// Two ways a function gets the right `&'static Palette` for the app's
// current theme, depending on what it already has in scope:
// - `view()`/`new_tab_page()`/`view_agent_sidebar()` already take
//   `state: &FerriteBrowser`, so each binds `let palette = state.palette();`
//   once near the top and every former `C_XXX` reference in that function
//   became `palette.xxx`.
// - The nine `*_style` functions below are passed to `.style(...)` as bare
//   `fn` pointers (e.g. `.style(nav_btn_style)`), so iced itself calls them
//   at render time with whatever `Theme` `launch()`'s `.theme(...)` closure
//   currently returns. `palette_for_theme(theme)` reads the `AppTheme` back
//   out of that `Theme`, so no extra state needs threading through the
//   `.style()` call sites at all — the parameter those functions already
//   had (previously named `_theme` and ignored) is now the one thing they
//   actually need.

/// One resolved colour per semantic role: background depth
/// (`base`/`surface`/`raised`/`divider`/`input`), text
/// (`text`/`text_dim`), the brand accent (`accent`/`accent_bright`), and the
/// three status colours (`safe`/`warn`/`danger`) — exactly the pre-C3c
/// `C_BASE`.../`C_DANGER` constants, as struct fields instead of bare idents.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The tab strip: one step away from `base` (the toolbar and the active
    /// tab), so the active tab reads as part of the toolbar.
    pub chrome: Color,
    pub base: Color,
    pub surface: Color,
    pub raised: Color,
    pub divider: Color,
    pub text: Color,
    pub text_dim: Color,
    pub accent: Color,
    pub accent_bright: Color,
    /// The fill of a solid primary button and its hover state. On the dark
    /// theme these are deeper than `accent`/`accent_bright` (which are tuned to
    /// read as text and icons on a dark surface), so a white label on them
    /// clears 4.5:1; on the light theme they are the accent itself.
    pub accent_fill: Color,
    pub accent_fill_hover: Color,
    pub input: Color,
    pub safe: Color,
    pub warn: Color,
    pub danger: Color,
}

/// The palette this crate shipped through C3b, unchanged value-for-value —
/// still the default theme (see [`AppTheme`]'s `Default` impl), so a user
/// who never touches the new toggle sees exactly what they always have.
const DARK_PALETTE: Palette = Palette {
    chrome: Color {
        r: 0.040,
        g: 0.040,
        b: 0.052,
        a: 1.0,
    },
    base: Color {
        r: 0.08,
        g: 0.08,
        b: 0.10,
        a: 1.0,
    },
    surface: Color {
        r: 0.11,
        g: 0.11,
        b: 0.14,
        a: 1.0,
    },
    raised: Color {
        r: 0.17,
        g: 0.17,
        b: 0.21,
        a: 1.0,
    },
    divider: Color {
        r: 0.20,
        g: 0.20,
        b: 0.25,
        a: 1.0,
    },
    text: Color {
        r: 0.93,
        g: 0.93,
        b: 0.96,
        a: 1.0,
    },
    // Secondary text: 5.0:1 on `raised` and 6.1:1 on `surface` (it was 3.6:1
    // and 4.4:1, under the 4.5:1 AA floor for the small captions it is used for).
    text_dim: Color {
        r: 0.60,
        g: 0.60,
        b: 0.68,
        a: 1.0,
    },
    accent: Color {
        r: 0.44,
        g: 0.38,
        b: 1.0,
        a: 1.0,
    },
    accent_bright: Color {
        r: 0.56,
        g: 0.50,
        b: 1.0,
        a: 1.0,
    },
    accent_fill: Color {
        r: 0.352,
        g: 0.304,
        b: 0.80,
        a: 1.0,
    },
    accent_fill_hover: Color {
        r: 0.448,
        g: 0.40,
        b: 0.80,
        a: 1.0,
    },
    input: Color {
        r: 0.14,
        g: 0.14,
        b: 0.18,
        a: 1.0,
    },
    safe: Color {
        r: 0.20,
        g: 0.84,
        b: 0.54,
        a: 1.0,
    },
    warn: Color {
        r: 0.95,
        g: 0.65,
        b: 0.20,
        a: 1.0,
    },
    danger: Color {
        r: 1.0,
        g: 0.35,
        b: 0.35,
        a: 1.0,
    },
};

/// A considered light palette, not a naive RGB inversion of
/// [`DARK_PALETTE`]: background depth still runs the same direction real
/// browser chrome uses (near-white page/toolbar, a touch grayer tab strip,
/// grayer still for hover/raised state — Chrome's and Firefox's own light
/// themes order it the same way), the accent keeps the same purple hue
/// family but is deepened (`rgb(112,97,255)`/`rgb(143,128,255)` for
/// `accent`/`accent_bright` in the dark palette &rarr; `rgb(89,66,235)`/
/// `rgb(107,82,250)` here) so it still reads as foreground-safe against a
/// light background instead of washing out, and `safe`/`warn`/`danger` are
/// each darkened from the dark theme's saturated, light-on-dark-friendly
/// versions for the same reason — the original mint/amber/coral are all
/// under ~3.2:1 contrast against a near-white background, well under WCAG
/// AA's 4.5:1 for normal text, and this crate uses all three as small-text
/// badge/label colours (e.g. the audit log's kind column), not just as
/// decoration.
///
/// Contrast figures below are hand-computed against the WCAG relative-
/// luminance formula (sRGB-to-linear per channel, then
/// `0.2126R+0.7152G+0.0722B`, then `(L_light+0.05)/(L_dark+0.05)`) — not run
/// through an automated checker, so treat them as "designed with a target
/// in mind" rather than a certified audit: `text` on `base` ≈ 17:1 (AAA),
/// `text_dim` on `base` ≈ 5.4:1, `accent` on `base` ≈ 6:1, `safe`/`warn`/
/// `danger` on `base` ≈ 5.2-5.4:1 — all at or above AA's 4.5:1 for normal
/// text.
const LIGHT_PALETTE: Palette = Palette {
    chrome: Color {
        r: 0.894,
        g: 0.898,
        b: 0.925,
        a: 1.0,
    },
    base: Color {
        r: 0.980,
        g: 0.980,
        b: 0.988,
        a: 1.0,
    },
    surface: Color {
        r: 0.949,
        g: 0.953,
        b: 0.969,
        a: 1.0,
    },
    raised: Color {
        r: 0.910,
        g: 0.910,
        b: 0.941,
        a: 1.0,
    },
    divider: Color {
        r: 0.863,
        g: 0.863,
        b: 0.902,
        a: 1.0,
    },
    text: Color {
        r: 0.090,
        g: 0.090,
        b: 0.122,
        a: 1.0,
    },
    text_dim: Color {
        r: 0.400,
        g: 0.400,
        b: 0.460,
        a: 1.0,
    },
    accent: Color {
        r: 0.349,
        g: 0.259,
        b: 0.922,
        a: 1.0,
    },
    accent_bright: Color {
        r: 0.420,
        g: 0.322,
        b: 0.980,
        a: 1.0,
    },
    accent_fill: Color {
        r: 0.349,
        g: 0.259,
        b: 0.922,
        a: 1.0,
    },
    accent_fill_hover: Color {
        r: 0.420,
        g: 0.322,
        b: 0.980,
        a: 1.0,
    },
    input: Color {
        r: 1.000,
        g: 1.000,
        b: 1.000,
        a: 1.0,
    },
    safe: Color {
        r: 0.039,
        g: 0.471,
        b: 0.294,
        a: 1.0,
    },
    warn: Color {
        r: 0.647,
        g: 0.333,
        b: 0.020,
        a: 1.0,
    },
    danger: Color {
        r: 0.784,
        g: 0.149,
        b: 0.149,
        a: 1.0,
    },
};

/// Which of the two shipped palettes the app is currently drawing from.
/// `Default` is `Dark` — this crate's only theme through C3b — so a user who
/// never touches `ToggleTheme` (see `FerriteBrowserMessage`) sees the exact
/// same app they always have; the toggle is opt-in, not a default-behavior
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
}

impl AppTheme {
    /// Flips to the other mode — the entire behavior of `ToggleTheme` (see
    /// `update()`).
    fn toggled(self) -> Self {
        match self {
            AppTheme::Dark => AppTheme::Light,
            AppTheme::Light => AppTheme::Dark,
        }
    }

    /// The resolved colour set for this mode.
    fn palette(self) -> &'static Palette {
        match self {
            AppTheme::Dark => &DARK_PALETTE,
            AppTheme::Light => &LIGHT_PALETTE,
        }
    }

    /// `launch()`'s `.theme(...)` closure reads this to pick iced's own
    /// built-in `Theme::Dark`/`Theme::Light` (which drives iced's default
    /// widget rendering — e.g. `text_input`'s selection/scrollbar chrome
    /// this crate doesn't style itself) — kept in lock-step with `Palette`
    /// selection rather than as independent state, so the two can never
    /// point at different themes.
    fn to_iced_theme(self) -> Theme {
        match self {
            AppTheme::Dark => Theme::Dark,
            AppTheme::Light => Theme::Light,
        }
    }
}

/// The same lookup as [`AppTheme::palette`], keyed by iced's own `Theme`
/// instead of `AppTheme` — for the nine `*_style` functions below, which
/// receive `&Theme` from iced itself at render time (whatever
/// `to_iced_theme()` last returned) rather than a `FerriteBrowser`
/// reference. Anything other than `Theme::Light` resolves to the dark
/// palette, so a hypothetical future `Theme::Custom(...)` (never constructed
/// by this crate today) degrades to the existing look instead of panicking.
fn palette_for_theme(theme: &Theme) -> &'static Palette {
    match theme {
        Theme::Light => &LIGHT_PALETTE,
        _ => &DARK_PALETTE,
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// C2: one entry in the agent sidebar's live activity feed
/// (`FerriteBrowser::agent_log`), replacing the flat `Vec<String>` label
/// list B3 left (`agent_tool_log`) with real per-step structure: an icon,
/// a title, the action's own parameter, and — unlike the old log — the
/// actual result `execute_action` returned, not just the fact that some
/// action ran.
#[derive(Debug, Clone)]
pub enum AgentLogEntry {
    /// A plain status line from a phase that has no individual action to
    /// itemize yet — currently only the dry-run phase's progress note
    /// (`AgentToolLogged`, e.g. "dry run complete — checking for
    /// unexpected activity").
    Note(String),
    /// One action the live loop actually took, or attempted.
    Step {
        icon: Icon,
        label: &'static str,
        detail: String,
        /// The real observation `execute_action` returned — or, if
        /// `blocked` is true, the fixed "blocked by user consent" string
        /// `is_action_rejected` produces instead of ever calling it.
        result: String,
        /// Whether the user's consent decision blocked this action
        /// before it reached the engine, rather than it having actually
        /// executed — styled differently in the sidebar (see
        /// `view_agent_sidebar`) so a blocked step doesn't read as if it
        /// succeeded.
        blocked: bool,
        /// Whether the optional Laya fast lane chose this step (no LLM call)
        /// rather than the model — shown as a small "fast" badge.
        fast: bool,
    },
}

/// Why one step's background model call did not produce a usable
/// `AgentAction`, carried over `AgentStepReady` in place of a plain
/// `String` so the handler in `update()` can tell the two failure modes
/// apart and treat them differently.
#[derive(Debug, Clone)]
pub enum StepFailure {
    /// The model provider call itself failed (network, auth, timeout,
    /// empty response, …). Never retried — a broken provider will not
    /// self-correct by being asked again in the same way.
    Model(String),
    /// The response body was not a single, complete, valid JSON
    /// `AgentAction` — most often a `finish.answer` truncated mid-string by
    /// the provider's own output cap, or stray prose/markdown fencing
    /// around the JSON. `AgentStepReady`'s handler retries this, up to
    /// `MAX_CONSECUTIVE_MALFORMED_STEPS` times, by feeding `message` back
    /// to the model as an observation rather than ending the run on what
    /// is often a one-off, self-correctable glitch.
    Malformed { raw: String, message: String },
}

/// Which view the agent sidebar shows: the current chat's message thread, or
/// the list of previous chats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarView {
    /// The current chat (the default).
    #[default]
    Thread,
    /// Previous chats, newest first.
    History,
}

/// Identifies one animated item of the message thread: a turn, and which part
/// of it (`SLOT_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemKey {
    pub turn: usize,
    pub slot: u32,
}

/// The user's message bubble of a turn.
pub const SLOT_USER: u32 = 0;
/// The first live step card of a turn (`SLOT_STEP_BASE + index into agent_log`).
pub const SLOT_STEP_BASE: u32 = 1;
/// The turn's outcome card (answer, question, stop, failure).
pub const SLOT_OUTCOME: u32 = u32::MAX;

/// State of an in-progress live agent-action loop (post-fingerprint, either
/// bypassed straight through or after a clean/consented dry run). Lives on
/// `FerriteBrowser` between the per-step background model calls
/// (`AgentStepReady`) that drive it — see this file's module docs for why
/// the loop is message-driven rather than a direct `run_agent_loop` call.
pub struct LiveAgentLoop {
    /// Conversation history sent to the model on every step — starts as
    /// `[Message::user(prompt)]`, then grows by one assistant (action) /
    /// user (observation) pair per completed step.
    messages: Vec<Message>,
    /// Every action actually executed so far, in order — used for
    /// step-budget accounting and repeated-action detection, mirroring
    /// `browser_loop::run_agent_loop`'s own bookkeeping exactly.
    actions_taken: Vec<AgentAction>,
    /// When this loop started — checked against `budget.max_wall_clock`
    /// before every step's model call.
    started_at: std::time::Instant,
    budget: LoopBudget,
    /// Tool ids the user rejected in the consent panel, if this loop is the
    /// post-consent real run — empty for a bypassed/clean-dry-run loop that
    /// never needed consent.
    rejected: std::collections::HashSet<ToolId>,
    /// Origins the user rejected in the consent panel, if this loop is the
    /// post-consent real run.
    rejected_origins: std::collections::HashSet<String>,
    /// Consecutive unparseable model responses seen in a row — mirrors
    /// `browser_loop::run_agent_loop`'s own counter of the same name.
    /// Reset to `0` the moment a step parses successfully; once it exceeds
    /// `MAX_CONSECUTIVE_MALFORMED_STEPS`, the run ends with the parse error
    /// instead of retrying again. See `AgentStepReady`'s handler.
    consecutive_malformed: u32,
    /// The user's goal for Laya (the fast lane): `trusted_task_text` — user
    /// words only — never the seed or anything page-derived.
    goal: String,
    /// Laya's `recent_actions`: the steps taken so far (fast-lane steps
    /// precisely, LLM steps approximately), newest last, bounded.
    history: Vec<HistoryItem>,
    /// The fast-lane action executed immediately before the step being
    /// decided, if the previous step was a fast-lane one — Laya's repeat
    /// suppression. `None` after an LLM step.
    previous_fast: Option<FastAction>,
    /// Signature of the page as of the last fast-lane digest, to tell Laya
    /// whether the previous action changed the page.
    last_page_sig: Option<u64>,
    /// The predicted fingerprint plus the user's approvals, enforced on every
    /// real action (ADR-014). `None` only when the defense is off for this run.
    guard: Option<ferrite_ipi::comparator::RuntimeGuard>,
    /// Actions the guard has blocked so far in this run.
    guard_blocks: u32,
    /// What the person decided about the action the guard just stopped;
    /// consumed by that one action, then back to `Ask`.
    guard_choice: runtime_guard::GuardChoice,
    /// The host the person already pressed *Continue* on at a sign-in
    /// handoff (see `signin`), so the agent is not stopped again at every page
    /// of the same sign-in. Forgotten when the agent reaches a page that needs
    /// no one.
    signin_cleared_host: Option<String>,
}

impl LiveAgentLoop {
    /// A fresh loop: `first_message` is the run's first user message (the
    /// seed), `goal` the trusted task text for the fast lane.
    fn new(
        first_message: String,
        goal: String,
        rejected: std::collections::HashSet<ToolId>,
        rejected_origins: std::collections::HashSet<String>,
    ) -> Self {
        Self {
            messages: vec![Message::user(first_message)],
            actions_taken: Vec::new(),
            started_at: std::time::Instant::now(),
            budget: LoopBudget::default(),
            rejected,
            rejected_origins,
            consecutive_malformed: 0,
            goal,
            history: Vec::new(),
            previous_fast: None,
            last_page_sig: None,
            guard: None,
            guard_blocks: 0,
            guard_choice: runtime_guard::GuardChoice::Ask,
            signin_cleared_host: None,
        }
    }

    /// Enforces `guard` on every action of this run.
    fn with_guard(mut self, guard: Option<ferrite_ipi::comparator::RuntimeGuard>) -> Self {
        self.guard = guard;
        self
    }
}

/// The agent tried something outside what the request implied, and the run is
/// paused on the question. Holds the action so *Allow* can run exactly it.
pub struct PendingRuntimeConsent {
    run_id: u64,
    action: AgentAction,
    fast: Option<(FastAction, HistoryItem)>,
    ask: runtime_guard::RuntimeAsk,
}

impl PendingRuntimeConsent {
    /// The one sentence the card shows.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.ask.summary
    }
}

pub struct FerriteBrowser {
    pub tabs: Vec<String>,
    pub active_tab: usize,
    pub address_bar_input: String,
    /// Whether the user has typed into the address bar since it last showed
    /// the page's own address (decides whether a click selects it all).
    pub address_bar_edited: bool,
    /// Committed (navigated-to) URL per tab.
    pub tab_urls: Vec<String>,
    pub show_audit_panel: bool,
    /// Which view of the Audit panel is showing.
    pub audit_tab: AuditTab,
    /// The model-activity trace as of the last refresh (see `activity_panel`).
    pub trace_events: Vec<ferrite_model::trace::TraceEvent>,
    /// The trace event whose request/response is expanded, by sequence number.
    pub trace_expanded: Option<u64>,
    pub show_js_console: bool,
    pub audit_entries: Vec<AuditEntry>,
    pub servo_sessions: HashMap<usize, HeadlessServoSession>,
    /// The picture each tab last showed, keyed by tab, shared with the engine
    /// (no copy) and tagged with its frame number, so the GPU texture is
    /// written once per *new* picture. The image handle is there only when the
    /// page is drawn with iced's image widget (`FERRITE_PAGE_DRAW=image`).
    frame_cache: HashMap<usize, (page_view::PageFrame, Option<ImageHandle>)>,
    /// `FERRITE_PERF=1`'s counters (see `perf.rs`); `None` when it is off.
    perf: Option<perf::PerfStats>,
    pub is_loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Phase for the animated loading bar (0..1).
    pub progress_offset: f32,
    pub address_bar_focused: bool,
    pub tab_error: Vec<Option<String>>,
    pub tab_titles: Vec<String>,
    /// The active page's favicon for each tab, same indexing as
    /// `tab_titles`/`tab_urls` — `None` until `HeadlessServoSession::
    /// get_favicon()` reports one (or forever, for a page that never sets
    /// one, e.g. `about:blank`). Synced every `ServoFrame` tick alongside
    /// the page title (see that handler).
    pub tab_favicons: Vec<Option<ImageHandle>>,
    pub new_tab_search_input: String,
    // ── DevTools, page controls and panel sizes ──────────────────────────────
    /// One entry per tab (same indexing as `tab_urls`): its console and network
    /// log, the control its page is waiting on, and a crash notice.
    pub(crate) tab_diag: Vec<tab_diag::TabDiag>,
    /// The DevTools panel's view and filters (the logs are per tab, above).
    pub(crate) devtools: devtools::DevToolsUi,
    /// Engine panics and page crashes this session.
    pub(crate) engine_log: devtools::EngineLog,
    /// How wide the side drawers and how tall the bottom panels are, and the
    /// drag in progress (`layout`).
    pub(crate) panels: layout::Panels,
    /// The window's size in logical points, kept current by resize events;
    /// panel sizes are clamped against it so the page always has room.
    pub window_size: Size,
    /// When the engine tick last ran, so animation advances by elapsed time
    /// rather than by tick count (a display frame is not always 16 ms).
    last_tick: Option<std::time::Instant>,
    /// A fingerprint of each tab's last favicon, so a handle is rebuilt (and
    /// the icon re-uploaded) only when the engine reports a different one.
    favicon_keys: Vec<Option<u64>>,
    /// Most recent cursor position over the Servo content area, in logical
    /// points (the same space `mouse_area::on_move` reports and `view()`
    /// lays widgets out in) — never physical/device pixels. Scaled by
    /// `scale_factor` at the point each is forwarded to
    /// `HeadlessServoSession::send_mouse_*`, which operates in the render
    /// buffer's physical-pixel space (see `scale_factor`'s doc comment).
    pub cursor_pos: (f32, f32),
    /// Physical pixels per logical point for the app's window, fetched once
    /// via `iced::window::get_scale_factor` shortly after launch
    /// (`ScaleFactorReady`) — defaults to `1.0` until that resolves. Needed
    /// because `HeadlessServoSession`'s render buffer and
    /// `send_mouse_*`/`send_scroll`'s coordinates are in physical pixels,
    /// while every iced-reported position (`mouse_area::on_move`,
    /// `content_area_size`) is in logical points; on a HiDPI/Retina display
    /// those differ, and conflating them is exactly what caused the
    /// blurry-frame and hover/click-offset bugs this field's introduction
    /// fixes (C3a).
    pub scale_factor: f32,
    /// Most recently measured logical size of the Servo content area —
    /// written from `view()`'s `responsive` wrapper around the content
    /// element (interior mutability: `view()` takes `&self`, so a `Cell`
    /// is how a read-only render pass can still record what it measured)
    /// and read back by `ServoFrame`'s tick handler to decide whether the
    /// active tab's session buffer needs `resize()`ing to match.
    ///
    /// Replaces the older `content_y_offset`/`ContentAreaResized`
    /// message, which tracked only a hardcoded vertical chrome-height
    /// offset and — verified by grep before this fix — was never actually
    /// wired to a real producer anywhere in `update()`'s message handling,
    /// so it always held its initial constant. `responsive` reports the
    /// container's true available size on every layout pass instead of
    /// requiring this file to hand-replicate the chrome layout's
    /// heights/widths (tab bar + toolbar + progress bar + optional
    /// audit/JS panel + optional agent sidebar) — correct by construction
    /// as that layout changes, not by keeping two copies of it in sync.
    pub content_area_size: Cell<Size>,
    /// The physical-pixel size the active tab's session was last actually
    /// told to `resize()` to — compared against on every `ServoFrame` tick
    /// to decide whether a new `resize()` call is needed at all. Tracked
    /// here rather than re-derived from `HeadlessServoSession::size()`
    /// because that getter reflects `resize()`'s own immediate, synchronous
    /// bookkeeping (it updates `self.width`/`self.height` the instant it's
    /// called), not whether libservo's own compositor has actually
    /// finished reallocating the backing surface — see
    /// `resize_settle_ticks`'s doc comment for why that distinction is the
    /// whole fix here. Initialized to `(1280, 700)`, matching every
    /// session's real starting buffer size, so the very first tick doesn't
    /// treat "unmeasured yet" as "must resize".
    pub last_resized_content_px: (u32, u32),
    /// Ticks remaining before it's safe to read a frame from the session
    /// most recently `resize()`d — see `ServoFrame`'s handler.
    ///
    /// **Corrected history:** this field was first written on the theory
    /// that the segfault/black-screen/stuck-rendering bug was a same-tick
    /// race — `session.resize()` immediately followed by
    /// `session.sync_and_read()` reading a frame before libservo finished
    /// reallocating the surface. That fix alone did **not** resolve the
    /// bug when tested on real hardware; the actual root cause was in
    /// `HeadlessServoSession::resize()` itself (a redundant, unguarded
    /// `rendering_context.resize()` call that made Servo's own
    /// `Painter::resize_rendering_context` change-detection see "no
    /// change" and skip the repaint-at-new-size step entirely, and that
    /// also skipped `make_current()` before touching the surface — see
    /// that method's doc comment for the full trace against the pinned
    /// `libservo` source). That is now fixed directly in `session.rs`.
    /// This field and `RESIZE_SETTLE_TICKS` are kept as a small residual
    /// safety margin (one tick) rather than removed outright, since a
    /// same-tick read immediately after `resize()` is still a real,
    /// separate race in principle even with the primary bug fixed — not
    /// because it was ever the actual cause of what the user observed.
    pub resize_settle_ticks: u8,
    // ── Agent bridge ──────────────────────────────────────────────────────────
    /// The real `ferrite_model::ModelProvider` constructed at startup
    /// (`settings_panel::connect_saved`), or `ferrite_model::MockProvider::new()`
    /// (fail-to-empty) when none is configured/reachable — T-224/T-229.
    pub model_provider: Arc<dyn ferrite_model::ModelProvider>,
    /// Configured tag for `ModelTier::Small` (the fingerprint's `may_use`
    /// prediction) — `"unconfigured"` when `model_provider` is the mock
    /// fallback, harmless since `MockProvider` ignores the tag entirely.
    pub model_tag_small: String,
    /// Configured tag for `ModelTier::Main` (the live agent loop's plan/act
    /// reasoning).
    pub model_tag_main: String,
    /// Handle to the currently in-flight background task (a dry run, or one
    /// live-loop step's model call), if any.
    pub agent_handle: Option<tokio::task::JoinHandle<()>>,
    /// Bumped on every fresh run (`AgentTaskSubmitted`, the post-consent
    /// run) and on every stop (`StopAgent`, `ConsentCancelled`). Each
    /// background step stamps the `run_id` it was spawned under onto its
    /// reply message; `update()` drops a `LiveRunReady`/`AgentStepReady`
    /// whose `run_id` no longer matches, so a step already in flight when
    /// the user hits Stop (or starts a new task) can never execute a live
    /// browser action after that point, even though `JoinHandle::abort()`
    /// cannot guarantee the in-flight future was actually cancelled before
    /// it sent its reply.
    pub run_id: u64,
    /// State of the in-progress live agent-action loop, if one is running —
    /// `None` whenever the agent is idle, in the middle of a dry run, or
    /// waiting on a pending consent decision.
    pub live_loop: Option<LiveAgentLoop>,
    // ── Agent sidebar UI ─────────────────────────────────────────────────────
    pub show_agent_sidebar: bool,
    pub agent_task_input: String,
    /// Live activity feed — one entry per dry-run status note or executed
    /// action, in order. See [`AgentLogEntry`].
    pub agent_log: Vec<AgentLogEntry>,
    pub agent_response: Option<String>,
    pub agent_is_running: bool,
    /// Sender used by spawned agent task to emit progress messages.
    pub agent_event_tx: Option<tokio::sync::mpsc::UnboundedSender<FerriteBrowserMessage>>,
    /// Receiver drained by the agent_event_sub subscription.
    pub agent_event_rx: Option<
        std::sync::Arc<
            tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<FerriteBrowserMessage>>,
        >,
    >,
    // ── Chats ────────────────────────────────────────────────────────────────
    // The chat is the durable record of a conversation; `agent_log`/
    // `agent_response`/`agent_is_running`/`live_loop` above are the *current
    // run's* live mirror. Every ending of a run goes through `conclude_run`,
    // which finishes the turn on `chat`, saves it and refreshes `chat_list`.
    /// The chat the sidebar is showing and the next message continues. A fresh
    /// empty chat is not written to disk until its first message.
    pub chat: Chat,
    /// Where chats are saved — resolved and opened only by `launch()`
    /// (`default_chats_dir()`), never by `Default`, the same test-safety
    /// discipline as `bookmarks_path`: `None` means chats live in memory only
    /// (every test, and a machine with no resolvable home directory).
    pub chat_store: Option<ChatStore>,
    /// One row per saved chat, newest first — read from disk once, in
    /// `launch()`, then kept current in memory as chats are saved and deleted.
    pub chat_list: Vec<ChatSummary>,
    /// The optional Laya step decider (the fast lane), resolved once by
    /// `launch()` from `FERRITE_LAYA_URL` and `None` — fast lane off, agent
    /// behaves exactly as before — everywhere else (`Default`, tests, unset or
    /// invalid configuration).
    pub laya: Option<Arc<LayaStepDecider>>,
    /// Thread or History.
    pub sidebar_view: SidebarView,
    /// The context chip's setting: Auto decides per message whether the
    /// current page is context, On always attaches it, Off never does. Per
    /// session only (not persisted).
    pub context_mode: ContextMode,
    /// The text of the *live* loop's first message for the run in flight — the
    /// seed (`build_seed`: chat history, open tabs, page digest, all labelled
    /// untrusted, then the user's request). Set by `submit_task`, taken by
    /// `start_live_loop` (from either `LiveRunReady` or the post-consent
    /// `ConsentSubmitted`, so both start from the same seed). It is
    /// deliberately never given to the IPI defense — see `submit_task`.
    pub pending_seed: Option<String>,
    /// Which chat row (if any) is showing its "Delete this chat?" confirm
    /// affordance — a delete is never one click.
    pub pending_chat_delete: Option<ChatId>,
    /// A small non-blocking line under the sidebar header ("Couldn't save this
    /// chat: ...", "Stop the current run first"), dismissible.
    pub panel_notice: Option<String>,
    /// Whether the message thread is scrolled to (or within a few pixels of)
    /// the bottom. New items pull the thread down only while this holds, so a
    /// user who scrolled up to read is never yanked back. Updated by the
    /// thread's `on_scroll`; set again by sending a message, opening a chat or
    /// the "Latest" button.
    pub thread_pinned: bool,
    /// Entrance progress (`0.0..1.0`) of thread items that are still fading and
    /// sliding in — advanced by `ThreadAnimTick` and empty (so the tick
    /// subscription is off) whenever nothing is animating.
    pub thread_anims: Vec<(ItemKey, f32)>,
    /// Expand/collapse state of thread details (a finished turn's step list, a
    /// long answer, a long step result), by the keys `agent_panel` builds.
    /// Cleared whenever the chat changes.
    pub expanded: std::collections::HashSet<String>,
    // ── IPI consent state ────────────────────────────────────────────────────
    // Every field below is scoped to exactly one pending consent decision and
    // is cleared on *both* ConsentSubmitted and ConsentCancelled — approvals
    // and rejections are per-task, never sticky across tasks. See
    // `consent_state_never_carries_into_a_second_task` for the test that
    // would fail if any of this leaked into a later task.
    /// Set when the dry run finds unexpected activity; cleared after consent
    /// or cancel.
    pub pending_diff: Option<FingerprintDiff>,
    /// The expected fingerprint `pending_diff` was compared against — kept
    /// alongside the diff so the consent panel can describe which origins
    /// *would* have been admitted, not just which ones weren't.
    pub pending_expected: Option<ExpectedFingerprint>,
    /// The dry-run record itself, kept only so the consent panel can show it
    /// on expand ("what did the agent actually do, in what order").
    pub pending_evidence: Option<DryRunRecord>,
    /// Tracks per-item approve/reject decisions while the consent panel is
    /// open, covering both `pending_diff.extra_primitives` and
    /// `pending_diff.out_of_scope_origins` (see `consent_items`).
    pub pending_decision: ConsentDecision,
    /// Original task prompt preserved between dry run and consent
    /// resolution — just the prompt (not a full `IpiTask`): the live loop
    /// always starts a fresh message history from this prompt regardless of
    /// what the dry run itself did, so nothing else needs to be carried.
    pub pending_task: Option<String>,
    /// Whether the dry-run evidence section is expanded.
    pub show_evidence: bool,
    // ── C1 design-system state ───────────────────────────────────────────────
    /// Index of the tab bar entry currently under the pointer, if any —
    /// drives the close-button-on-hover reveal pattern (`chrome::tab_view`).
    /// `None` when the pointer is not over any tab; reset on `CloseTab`
    /// since a close shifts every later tab's index, and a stale hovered
    /// index would otherwise show the close button on the wrong tab.
    pub hovered_tab: Option<usize>,
    /// Whether the toolbar's overflow menu is open.
    pub show_menu: bool,
    /// Slide-in progress of the overflow menu, `0.0` (just opened) to `1.0`;
    /// advanced by `MenuAnimTick` while it is below `1.0`. Decoration only.
    pub menu_anim: f32,
    /// When the tab strip's empty area was last pressed (double-click =
    /// maximize).
    last_titlebar_press: Option<std::time::Instant>,
    /// Wheel/trackpad input waiting to be handed to the engine on the next
    /// `ServoFrame` tick (see `scroll`).
    scroll_queue: scroll::ScrollQueue,
    /// Whether the pointer moved over the page since the last tick. Moves are
    /// recorded in `cursor_pos` as they arrive and forwarded to the engine at
    /// most once per tick.
    pointer_moved: bool,
    /// Ticks left before the page counts as idle: reset to `BUSY_TICKS` by a
    /// new picture or by input, counted down by `ServoFrame`. Decides how fast
    /// the tick runs (`tick_interval`).
    busy_ticks: u8,
    /// Fade/slide-in progress for the consent panel, `0.0` (just appeared)
    /// to `1.0` (fully settled) — advanced by `ConsentPanelTick` while
    /// `pending_diff` is `Some` and this is below `1.0` (see
    /// `subscription()`'s `consent_anim_tick`). Reset to `0.0` whenever
    /// `pending_diff` transitions from `None` to `Some` (`ConsentRequired`),
    /// so the panel replays its entrance every time a new one appears. Pure
    /// decoration only — see `view_agent_sidebar`'s consent body for why
    /// this never delays or hides any of the panel's actual content.
    pub consent_panel_anim: f32,
    // ── C3c: theme ────────────────────────────────────────────────────────
    /// Which palette `view()`/`new_tab_page()`/`view_agent_sidebar()`
    /// currently draw from, and which `iced::Theme` `launch()`'s
    /// `.theme(...)` closure returns — see [`AppTheme`]. Toggled by
    /// `ToggleTheme`, e.g. the toolbar's sun/moon button.
    pub theme_mode: AppTheme,
    // ── C3d: bookmarks ───────────────────────────────────────────────────
    /// Every saved bookmark, in the order they were added — no separate
    /// sort key; the Bookmarks tab shows them in this order and a removal
    /// (`RemoveBookmark`) is by index into this same `Vec`.
    pub bookmarks: Vec<Bookmark>,
    /// Where `bookmarks` is persisted, resolved once by `launch()`
    /// (`default_bookmarks_path()`) — `None` in every test and in any
    /// `FerriteBrowser::default()` construction (this field is never set
    /// there, kept test-safe/R7 the same way `model_provider`'s real
    /// construction is deferred to `launch()` alone), meaning a bookmark
    /// change in that context updates `bookmarks` in memory but is never
    /// written to disk — the correct, honest behavior for a test, not a
    /// silently-swallowed I/O error.
    pub bookmarks_path: Option<PathBuf>,
    // ── C3d: history ─────────────────────────────────────────────────────
    /// Every page visited this session, oldest first — one browser-wide
    /// list rather than Servo's own per-tab session history verbatim
    /// (`HeadlessServoSession::history()`), because most real browsers'
    /// history UI is itself one recency-ordered list across all tabs, not
    /// per-tab — simpler to build and to read than reconciling N per-tab
    /// lists into one view would be, at the cost of losing Servo's own
    /// per-tab back/forward *order* here (still available, unaffected, via
    /// `can_go_back`/`can_go_forward`/`GoBack`/`GoForward`, which read
    /// Servo's per-tab history directly — this list is for the History
    /// *panel* only, not navigation). Appended to by `LoadStatusChanged`
    /// (see that handler) whenever a tab's URL genuinely changes to
    /// something real (not `about:blank`), deduplicated against its own
    /// immediately-preceding entry (`record_history_visit`) so a reload
    /// doesn't spam the list with repeats of the same URL.
    ///
    /// **Not persisted across restarts — a deliberate scope cut, not an
    /// oversight.** A real browser does persist history, but doing that
    /// correctly (merge semantics across restarts, a retention/eviction
    /// policy, migrating the JSON shape over time) is materially more
    /// design work than bookmarks' simple "load once, save on every
    /// change" — out of proportion to what this pass can verify without a
    /// real display to check it against. Session-only history is still a
    /// real, working feature (find-in-page and zoom are also session-only
    /// state, for the same proportionality reason).
    pub history: Vec<HistoryEntry>,
    // ── C3d: zoom ─────────────────────────────────────────────────────────
    /// Per-tab zoom level, `1.0` = 100% — same indexing convention as
    /// `tab_titles`/`tab_urls`/`tab_favicons`, kept in sync on
    /// `AddTab`/`CloseTab`. Applied through the engine's native page
    /// zoom (`HeadlessServoSession::set_zoom`).
    pub tab_zoom: Vec<f32>,
    /// The zoom level a freshly-added tab starts at — a genuinely-settable
    /// preference (Settings drawer, `SetDefaultZoom`), distinct from `1.0`
    /// being merely `AddTab`'s old hardcoded default.
    pub default_zoom: f32,
    // ── C3d: find-in-page ────────────────────────────────────────────────
    /// Whether the find bar overlay is shown — toggled by `OpenFindBar`/
    /// `CloseFindBar` (Cmd/Ctrl+F to open, Escape to close while it's the
    /// thing on screen that Escape should act on — see `handle_key_press`).
    pub show_find_bar: bool,
    pub find_query: String,
    /// Total matches found by the most recent search (`find_script`) — `0`
    /// both before any search has run and when a search found nothing;
    /// `find_current_index` distinguishes those in the UI to avoid a
    /// content-free "0 of 0" line while the bar has just opened.
    pub find_match_count: usize,
    /// 1-based index of the currently-highlighted match, `0` when there is
    /// no current match (no search run yet, or the last search found
    /// nothing).
    pub find_current_index: usize,
    // ── C3d: downloads ───────────────────────────────────────────────────
    /// Every download started this session, in the order `DownloadCurrentPage`
    /// created them — `run_download`'s progress messages (`DownloadProgress`/
    /// `DownloadCompleted`/`DownloadFailed`) update the matching entry by
    /// `id` in place, so completed downloads stay in the list rather than
    /// disappearing.
    pub downloads: Vec<DownloadItem>,
    /// Monotonically increasing id source for `downloads` — never reused,
    /// even across a download's whole lifetime, so a stale progress message
    /// from an aborted/superseded download (there is no cancel action yet,
    /// but a duplicate id would still be a latent bug) can never be
    /// misattributed to a later one.
    pub next_download_id: u64,
    /// Real downloads directory, resolved once by `launch()`
    /// (`default_downloads_dir()`) — `None` in every test/`Default`
    /// construction, same test-safety discipline as `bookmarks_path`.
    /// `DownloadCurrentPage` is a no-op (returns `Task::none()`) when this
    /// is `None`, rather than guessing a path.
    pub downloads_dir: Option<PathBuf>,
    // ── C3d: Library panel (bookmarks/history/downloads/settings) ──────────
    /// Whether the Library panel (the toolbar's single consolidated entry
    /// point for all four C3d list/settings views) is shown — mutually
    /// exclusive with the audit/JS-console bottom panels, the same way
    /// those two are already mutually exclusive with each other.
    pub show_library_panel: bool,
    /// Which of the four sub-views the Library panel currently shows.
    pub library_tab: LibraryTab,
    /// `ferrite_model::ModelConfig::cache_dir`, resolved once by `launch()`
    /// alongside `model_tag_small`/`model_tag_main` — displayed read-only in
    /// the Settings drawer's footer. `None` when no real provider is configured
    /// (`model_tag_small`/`model_tag_main` stay `"unconfigured"` in that
    /// same case).
    pub model_cache_dir: Option<String>,
    /// Whether the Settings drawer (model provider, API key, appearance) is
    /// shown. A right-hand drawer like the Library and the agent: one at a
    /// time.
    pub show_settings_panel: bool,
    /// The Settings drawer's state: the saved and in-progress model choices,
    /// the key being typed, and the keyring/environment/network it talks to.
    pub settings: settings_panel::SettingsState,
    /// Set while the agent is paused on a page that needs the person (a
    /// sign-in or another secret): the run is stopped before any model call and
    /// the agent panel shows a card with *Continue*. See `signin`.
    pub signin_handoff: Option<signin::SignInWall>,
    /// An agent step that came in while a camera, microphone or screen request was
    /// waiting on the person. It is held (the loop stays as it was) and sent again once
    /// the request is answered or gone: the agent does not act on a page that is asking
    /// for something on the person's machine until the person has decided.
    pub deferred_agent_step: Option<FerriteBrowserMessage>,
    /// An action the guard stopped, put to the person. The run waits here;
    /// nothing outside the prediction runs until they say so. See
    /// `PendingRuntimeConsent`.
    pub pending_runtime: Option<PendingRuntimeConsent>,
    // ── New-tab hero: quick-access tile favicons ────────────────────────────
    /// The decoded favicon for each of `QUICK_ACCESS_TILES`, same indexing
    /// convention as `tab_favicons`/`tab_titles`/... elsewhere in this file
    /// — `None` until `TileFaviconReady` reports one (no network, first
    /// launch before the fetch completes, or the site's `/favicon.ico`
    /// doesn't resolve/decode). `new_tab_page` renders a designed monogram
    /// fallback for exactly that state — see this module's own doc comment
    /// ("New-tab hero: real quick-access-tile favicons").
    pub tile_favicons: Vec<Option<ImageHandle>>,
    /// Real favicon-cache directory, resolved once by `launch()`
    /// (`favicon_cache_dir()`) — `None` in every test/`Default` construction,
    /// the same test-safety discipline `bookmarks_path`/`downloads_dir`
    /// already establish. `FetchTileFavicons` is a no-op (spawns nothing)
    /// when this is `None`, rather than guessing a path or fetching without
    /// ever being able to cache the result.
    pub favicons_cache_dir: Option<PathBuf>,
}

impl FerriteBrowser {
    /// The resolved colour set for `self.theme_mode` — the one call every
    /// view function that needs colours makes, once, near its own top (see
    /// this module's "Colour palette" section header for the full
    /// threading story).
    pub fn palette(&self) -> &'static Palette {
        self.theme_mode.palette()
    }
}

impl Default for FerriteBrowser {
    fn default() -> Self {
        let (agent_event_tx, agent_event_rx) =
            tokio::sync::mpsc::unbounded_channel::<FerriteBrowserMessage>();
        // Test-safe by construction (R7, matching
        // `ferrite-eval::harness::try_real_provider()`'s own convention:
        // never called from a `Default`/test-reachable path — only from the
        // real app entry point, `launch()` below, which overwrites these
        // three fields with a real provider when one is configured/reachable
        // *before* the window ever opens). Every `FerriteBrowser::default()`
        // in this crate's own test suite therefore never touches the
        // environment or the OS keyring at all, let alone the network.
        let model_provider: Arc<dyn ferrite_model::ModelProvider> =
            Arc::new(ferrite_model::MockProvider::new());
        let model_tag_small = "unconfigured".to_string();
        let model_tag_main = "unconfigured".to_string();
        Self {
            tabs: vec!["New Tab".to_string()],
            active_tab: 0,
            address_bar_input: String::new(),
            address_bar_edited: false,
            tab_urls: vec!["about:blank".to_string()],
            show_audit_panel: false,
            audit_tab: AuditTab::default(),
            trace_events: Vec::new(),
            trace_expanded: None,
            show_js_console: false,
            audit_entries: vec![],
            servo_sessions: HashMap::new(),
            frame_cache: HashMap::new(),
            perf: perf::PerfStats::from_env(),
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
            progress_offset: 0.0,
            address_bar_focused: false,
            tab_error: vec![None],
            tab_titles: vec!["New Tab".to_string()],
            tab_favicons: vec![None],
            new_tab_search_input: String::new(),
            tab_diag: vec![tab_diag::TabDiag::default()],
            devtools: devtools::DevToolsUi::default(),
            engine_log: devtools::EngineLog::default(),
            panels: layout::Panels::default(),
            window_size: layout::ASSUMED_WINDOW,
            last_tick: None,
            favicon_keys: vec![None],
            cursor_pos: (0.0, 0.0),
            scale_factor: 1.0,
            content_area_size: Cell::new(Size::new(1280.0, 700.0)),
            last_resized_content_px: (1280, 700),
            resize_settle_ticks: 0,
            model_provider,
            model_tag_small,
            model_tag_main,
            agent_handle: None,
            run_id: 0,
            live_loop: None,
            show_agent_sidebar: false,
            agent_task_input: String::new(),
            agent_log: Vec::new(),
            agent_response: None,
            agent_is_running: false,
            agent_event_tx: Some(agent_event_tx),
            agent_event_rx: Some(std::sync::Arc::new(tokio::sync::Mutex::new(agent_event_rx))),
            // Test-safe by construction: no chat store (so nothing is read or
            // written), an empty list and a fresh in-memory chat.
            chat: Chat::new(),
            chat_store: None,
            chat_list: Vec::new(),
            sidebar_view: SidebarView::Thread,
            laya: None,
            context_mode: ContextMode::Auto,
            pending_seed: None,
            pending_chat_delete: None,
            panel_notice: None,
            expanded: std::collections::HashSet::new(),
            thread_pinned: true,
            thread_anims: Vec::new(),
            pending_diff: None,
            pending_expected: None,
            pending_evidence: None,
            pending_decision: ConsentDecision::default(),
            pending_task: None,
            show_evidence: false,
            hovered_tab: None,
            show_menu: false,
            menu_anim: 1.0,
            last_titlebar_press: None,
            scroll_queue: scroll::ScrollQueue::default(),
            pointer_moved: false,
            busy_ticks: 0,
            consent_panel_anim: 0.0,
            theme_mode: AppTheme::Dark,
            bookmarks: Vec::new(),
            bookmarks_path: None,
            history: Vec::new(),
            tab_zoom: vec![1.0],
            default_zoom: 1.0,
            show_find_bar: false,
            find_query: String::new(),
            find_match_count: 0,
            find_current_index: 0,
            downloads: Vec::new(),
            next_download_id: 0,
            downloads_dir: None,
            show_library_panel: false,
            library_tab: LibraryTab::default(),
            model_cache_dir: None,
            show_settings_panel: false,
            settings: settings_panel::SettingsState::default(),
            signin_handoff: None,
            deferred_agent_step: None,
            pending_runtime: None,
            tile_favicons: vec![None; QUICK_ACCESS_TILES.len()],
            favicons_cache_dir: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum FerriteBrowserMessage {
    AddTab,
    CloseTab(usize),
    SelectTab(usize),
    AddressBarChanged(String),
    NavigateRequested(String),
    ToggleAuditPanel,
    ToggleJsConsole,
    RefreshAuditLog,
    /// Switches the Audit panel between its two views.
    SetAuditTab(AuditTab),
    /// Copies the model-activity trace into the UI (sent every second while
    /// the Audit panel is open).
    RefreshTrace,
    /// Expands or collapses one trace event.
    ToggleTraceEvent(u64),
    /// Empties the in-memory trace.
    ClearTrace,
    /// The window's close button was pressed.
    WindowCloseRequested,
    ServoReady,
    ServoFrame,
    GoBack,
    GoForward,
    Reload,
    StopLoading,
    LoadStatusChanged {
        tab: usize,
        status: String,
        url: String,
    },
    FocusAddressBar,
    ClearAddressBarFocus,
    /// A click landed on the address bar.
    AddressBarPressed,
    CloseActiveTab,
    EscapePressed,
    NewTabSearchChanged(String),
    /// The DevTools prompt's text changed.
    JsInputChanged(String),
    /// Enter (or Run) at the DevTools prompt.
    JsExecuteRequested,
    /// The Console's Clear button (kept for the prompt's own shortcut).
    JsConsoleClear,
    /// Everything else the DevTools panel does (see `devtools`).
    DevTools(devtools::Msg),
    /// A reply to the control a page is waiting on (see `controls`).
    Control(controls::Msg),
    /// The answer to a camera, microphone or screen request, or "Stop sharing"
    /// (see `permission`).
    Permission(permission::Msg),
    /// The page-crash banner's buttons (see `crash`).
    Crash(crash::Msg),
    /// A press, move or release on a panel splitter (see `layout`).
    Panels(layout::Msg),
    /// The window changed size (logical points).
    WindowResized(Size),
    // Mouse/scroll events forwarded to Servo
    /// Mouse moved over the content area — position is relative to content area origin.
    ServoMouseMove {
        x: f32,
        y: f32,
    },
    /// Mouse button pressed (position taken from last ServoMouseMove).
    ServoMousePress,
    /// A key press/release that no widget in the browser chrome consumed,
    /// destined for the focused element of the active page (see
    /// `page_input`). Before this existed nothing forwarded the keyboard to
    /// pages, so text boxes could be clicked into but never typed in.
    PageKey(ferrite_servo::session::PageKeyEvent),
    /// Mouse button released (position taken from last ServoMouseMove).
    ServoMouseRelease,
    /// Secondary button pressed over the page (a context menu, usually).
    ServoRightPress,
    /// A slow timer that runs the engine tick if the frame-synchronised one
    /// has gone quiet (a minimised or covered window gets no redraws).
    ServoWatchdog,
    /// Scroll wheel / trackpad event; queued and delivered per tick.
    ServoScroll(scroll::Wheel),
    /// The window's real scale factor (physical px per logical point),
    /// fetched once shortly after launch — see `scale_factor`'s doc
    /// comment on `FerriteBrowser`.
    ScaleFactorReady(f32),
    // ── Live agent loop (T-224) ──────────────────────────────────────────────
    /// The dry run found nothing unexpected (or the defense mode bypassed
    /// it entirely) — begin the live loop from scratch with `prompt`. Not
    /// sent for the post-consent run, which `ConsentSubmitted` starts
    /// directly (it already has the prompt and the user's decision on
    /// hand, no round trip needed).
    LiveRunReady {
        run_id: u64,
        prompt: String,
        /// The prediction to enforce on the real run; `None` when the defense
        /// is off for this run.
        guard: Option<Box<ferrite_ipi::comparator::RuntimeGuard>>,
    },
    /// One step's background model call returned the next `AgentAction` to
    /// take (or `Err` if the model call itself failed or was unparseable
    /// — see `StepFailure`).
    AgentStepReady {
        run_id: u64,
        action: Result<AgentAction, StepFailure>,
    },
    /// The Laya fast lane chose this step (no LLM call). Handled by exactly
    /// the same code as `AgentStepReady` — the same consent check, execution,
    /// logging and loop-safety bookkeeping — plus the fast-lane bookkeeping
    /// (`fast` for repeat suppression, `history` for Laya's recent actions).
    FastStepReady {
        run_id: u64,
        action: AgentAction,
        fast: FastAction,
        history: HistoryItem,
    },
    // ── Agent sidebar ─────────────────────────────────────────────────────────
    ToggleAgentSidebar,
    AgentTaskInputChanged(String),
    AgentTaskSubmitted,
    AgentToolLogged(String),
    AgentCompleted(String),
    AgentFailed(String),
    StopAgent,
    /// The person pressed *Continue* on the sign-in handoff card: the agent
    /// picks up from the page as it is now.
    SigninContinue,
    /// Run the paused action this once.
    RuntimeAllowOnce,
    /// Run it, and allow the same kind of action for the rest of this task.
    RuntimeAllowTask,
    /// Do not run it; the agent is told it was blocked.
    RuntimeDeny,
    // ── Chats ─────────────────────────────────────────────────────────────────
    /// Starts a fresh, empty chat (the sidebar's "New chat" button, and
    /// Cmd/Ctrl+Shift+O). Refused, with a notice, while a run is active or a
    /// consent decision is pending: a chat is never abandoned mid-run.
    NewChat,
    /// Opens a saved chat from the history list (loaded from the store).
    /// Refused while a run is active, like `NewChat`.
    OpenChat(ChatId),
    /// First click of a delete: arms that row's confirm affordance.
    RequestDeleteChat(ChatId),
    /// Second, explicit click: deletes the armed chat.
    ConfirmDeleteChat,
    /// Disarms the delete confirm.
    CancelDeleteChat,
    /// Switches the sidebar between the thread and the chat list.
    SetSidebarView(SidebarView),
    /// Dismisses the sidebar's notice line.
    DismissNotice,
    /// The context chip: cycles Auto, On, Off.
    CycleContextMode,
    /// An empty-state suggestion chip: puts its text in the composer.
    SuggestionChosen(String),
    /// The message thread scrolled; `pinned` says whether it is at the bottom.
    ThreadScrolled {
        pinned: bool,
    },
    /// The "Latest" button: re-pin and jump to the newest item.
    ScrollThreadToBottom,
    /// One 16ms tick advancing the thread items' entrance animations (only
    /// subscribed to while something is animating).
    ThreadAnimTick,
    /// The answer card's Copy button: puts the full answer on the clipboard.
    CopyAnswer(String),
    /// A link in an agent answer, clicked: opens in a new tab.
    OpenLink(String),
    /// Toggles one expandable part of the thread (see `FerriteBrowser::expanded`).
    ToggleExpand(String),
    // ── IPI consent ───────────────────────────────────────────────────────────
    ConsentRequired {
        diff: FingerprintDiff,
        expected: ExpectedFingerprint,
        // Boxed solely to keep this enum's largest variant small (clippy
        // large_enum_variant) — every other variant is a handful of bytes,
        // and DryRunRecord's several Vec/HashSet fields make it the outlier.
        evidence: Box<DryRunRecord>,
    },
    /// `id` is either a real `ToolId` wire string (an `extra_primitives`
    /// item) or an `origin_item_id`-prefixed synthetic id (an
    /// `out_of_scope_origins` item) — see `consent_items`.
    ApproveTool(String),
    RejectTool(String),
    ToggleEvidence,
    ConsentSubmitted,
    ConsentCancelled,
    // ── C1 design-system messages ────────────────────────────────────────────
    /// Pointer entered tab `usize`'s hit area — drives the tab bar's
    /// close-button-on-hover reveal. Does not change tab selection/order/
    /// closing (`SelectTab`/`CloseTab`/`AddTab` are unchanged).
    TabHoverEnter(usize),
    /// Pointer left tab `usize`'s hit area.
    TabHoverExit(usize),
    // ── Chrome ───────────────────────────────────────────────────────────────
    /// Open or close the toolbar's overflow menu.
    ToggleMenu,
    CloseMenu,
    /// One animation tick of the overflow menu's slide-in.
    MenuAnimTick,
    /// A row of the overflow menu was picked: close the menu, then do it.
    Menu(chrome::MenuCommand),
    /// Open the library drawer on a given tab (from the overflow menu).
    OpenLibrary(LibraryTab),
    /// A press on the tab strip's empty area: drag the window, or maximize on
    /// a double-click.
    TitleBarPressed,
    /// Ctrl+Tab / Ctrl+Shift+Tab and the Cmd+Shift+[ ] pair.
    NextTab,
    PrevTab,
    /// Cmd/Ctrl+1..8 (zero-based) and Cmd/Ctrl+9 (the last tab).
    SelectTabNumber(usize),
    SelectLastTab,
    /// A press that only needs to be swallowed (on a popover's own card, so
    /// it does not reach the click-away layer beneath).
    Noop,
    /// One animation-subscription tick advancing
    /// `FerriteBrowser::consent_panel_anim` — see that field's docs and
    /// `subscription()`'s `consent_anim_tick`.
    ConsentPanelTick,
    // ── C3c: theme ────────────────────────────────────────────────────────
    /// Flips `FerriteBrowser::theme_mode` between `AppTheme::Dark` and
    /// `AppTheme::Light` — sent by the toolbar's sun/moon button.
    ToggleTheme,
    // ── C3d: bookmarks ────────────────────────────────────────────────────
    /// Star-toggle the active tab's current URL — bookmarks it if it isn't
    /// already, removes the existing bookmark if it is. No-op for
    /// `about:blank`/an empty URL.
    ToggleBookmarkCurrentPage,
    /// Removes `FerriteBrowser::bookmarks[index]` — a no-op (not a panic) if
    /// `index` is out of bounds, e.g. a stale button press racing a
    /// concurrent removal.
    RemoveBookmark(usize),
    // ── C3d: history ──────────────────────────────────────────────────────
    /// Empties `FerriteBrowser::history`. Session-only either way (see that
    /// field's doc comment) — this just lets the user clear it before the
    /// session ends too.
    ClearHistory,
    // ── C3d: zoom ─────────────────────────────────────────────────────────
    /// Steps the active tab's zoom to the next level up `ZOOM_LEVELS`.
    ZoomIn,
    /// Steps the active tab's zoom to the next level down `ZOOM_LEVELS`.
    ZoomOut,
    /// Resets the active tab's zoom to 100%.
    ZoomReset,
    /// Sets `FerriteBrowser::default_zoom` (Settings drawer) — affects tabs
    /// created after this point, not the currently active one.
    SetDefaultZoom(f32),
    // ── C3d: find-in-page ────────────────────────────────────────────────
    /// Opens the find bar (Cmd/Ctrl+F) and focuses its input.
    OpenFindBar,
    /// Closes the find bar and clears any highlights it left on the page.
    CloseFindBar,
    /// The find bar's input changed — re-runs the search immediately (live
    /// highlighting, the same convention every mainstream browser's own
    /// find bar uses) rather than waiting for Enter.
    FindQueryChanged(String),
    /// Advances to the next match, wrapping past the last one.
    FindNext,
    /// Moves to the previous match, wrapping past the first one.
    FindPrevious,
    // ── C3d: downloads ───────────────────────────────────────────────────
    /// "Download current page" (Library panel, Downloads tab) — starts a
    /// background `reqwest` GET of the active tab's current URL, streamed
    /// to `downloads_dir`.
    DownloadCurrentPage,
    /// One more chunk of `id`'s download arrived — `run_download` sends
    /// this on every chunk, not only periodically, so the UI's progress bar
    /// is exactly as current as the underlying stream.
    DownloadProgress {
        id: u64,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    DownloadCompleted {
        id: u64,
    },
    DownloadFailed {
        id: u64,
        error: String,
    },
    // ── C3d: Library panel ───────────────────────────────────────────────
    /// Toggles the Library panel (bookmarks/history/downloads/settings).
    ToggleLibraryPanel,
    /// Switches the Library panel's active sub-view.
    SelectLibraryTab(LibraryTab),
    // ── Settings drawer ──────────────────────────────────────────────────
    /// Toggles the Settings drawer (and the toolbar's gear).
    ToggleSettingsPanel,
    /// One action inside the Settings drawer (see `settings_panel`).
    Settings(settings_panel::SettingsMessage),
    // ── New-tab hero: quick-access tile favicons ────────────────────────────
    /// Sent once, at real startup (`launch()`'s startup `Task::batch`,
    /// mirroring `ServoReady`) — spawns one background fetch per
    /// `QUICK_ACCESS_TILES` entry. Never sent from a test/`Default`-
    /// constructed `FerriteBrowser` path.
    FetchTileFavicons,
    /// `fetch_tile_favicon` decoded a real favicon for tile `index` — raw
    /// RGBA8 pixels, converted into an `ImageHandle` only here in
    /// `update()` (the same "raw bytes over the channel" shape the tab-bar
    /// favicon sync already uses).
    TileFaviconReady {
        index: usize,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

pub fn update(
    state: &mut FerriteBrowser,
    message: FerriteBrowserMessage,
) -> Task<FerriteBrowserMessage> {
    match message {
        FerriteBrowserMessage::AddTab => {
            let (index, session_error) = add_tab(state);
            if let Some(e) = session_error {
                eprintln!("[ferrite-ui] Servo session tab {index}: {e}");
            }
            // A new tab starts at the address bar, ready to type, as in every
            // browser.
            return focus_address_bar(state);
        }
        FerriteBrowserMessage::OpenLink(url) => {
            // The answer's text came from a model that has read untrusted
            // pages, so a link only ever opens because a person clicked it,
            // only for http(s), and in a tab of its own.
            let lower = url.to_ascii_lowercase();
            if !(lower.starts_with("https://") || lower.starts_with("http://")) {
                return Task::none();
            }
            let (index, session_error) = add_tab(state);
            if let Some(e) = session_error {
                eprintln!("[ferrite-ui] Servo session tab {index}: {e}");
            }
            return update(state, FerriteBrowserMessage::NavigateRequested(url));
        }
        FerriteBrowserMessage::CloseTab(i) => {
            close_tab_at(state, i);
        }
        FerriteBrowserMessage::SelectTab(i) => {
            select_tab_at(state, i);
            return widgets::unfocus();
        }
        FerriteBrowserMessage::AddressBarChanged(s) => {
            state.address_bar_edited = true;
            state.address_bar_input = s;
        }
        FerriteBrowserMessage::NavigateRequested(raw) => {
            let url = resolve_url(&raw);
            // The page is about to be replaced: whatever it was asking is moot.
            controls::dismiss_tab(state, state.active_tab);
            state.address_bar_edited = false;
            state.address_bar_input = url.clone();
            state.tab_urls[state.active_tab] = url.clone();
            if state.active_tab < state.tab_error.len() {
                state.tab_error[state.active_tab] = None;
            }
            state.new_tab_search_input = String::new();
            state.is_loading = true;
            state.address_bar_focused = false;
            if crash::is_crashed(state, state.active_tab) {
                // The page's script thread is dead and may not take a
                // navigation; the address is loaded in a fresh session.
                crash::reload(state, state.active_tab);
            } else if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                session.navigate(&url);
            }
            // Let go of the address bar so keys reach the page again.
            return widgets::unfocus();
        }
        FerriteBrowserMessage::GoBack => {
            controls::dismiss_tab(state, state.active_tab);
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                session.go_back();
            }
            state.is_loading = true;
        }
        FerriteBrowserMessage::GoForward => {
            controls::dismiss_tab(state, state.active_tab);
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                session.go_forward();
            }
            state.is_loading = true;
        }
        FerriteBrowserMessage::Reload => {
            controls::dismiss_tab(state, state.active_tab);
            if crash::is_crashed(state, state.active_tab) {
                // A dead page answers no reload; it gets a fresh session.
                crash::reload(state, state.active_tab);
                return Task::none();
            }
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                session.reload();
            }
            state.is_loading = true;
        }
        FerriteBrowserMessage::StopLoading => {
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                session.stop();
            }
            state.is_loading = false;
        }
        FerriteBrowserMessage::LoadStatusChanged { tab, status, url } => {
            state.is_loading = status == "loading";
            if status == "failed" {
                if tab < state.tab_error.len() {
                    state.tab_error[tab] = Some(url.clone());
                }
            } else {
                if tab < state.tab_error.len() {
                    state.tab_error[tab] = None;
                }
                if !url.is_empty() && tab == state.active_tab {
                    let shown = address_bar_text(&url);
                    if shown != state.address_bar_input {
                        state.address_bar_input = shown;
                        state.address_bar_edited = false;
                    }
                }
                // C3d: record this visit before `tab_urls[tab]` is
                // overwritten below — `status == "complete"` only (not
                // every "loading" tick that also flows through this
                // branch), so a page is recorded once it actually finished
                // loading, not once per intermediate load event, and never
                // for `about:blank`/an empty URL.
                if status == "complete" && !url.is_empty() && url != "about:blank" {
                    let title = state
                        .tab_titles
                        .get(tab)
                        .cloned()
                        .filter(|t| !t.is_empty() && t != "New Tab")
                        .unwrap_or_else(|| url.clone());
                    record_history_visit(&mut state.history, url.clone(), title);
                    // A new tab's (or a fresh document's) page zoom may not be
                    // the level this tab is meant to have; set it through the
                    // engine when it differs. No script runs in the page.
                    let level = state.tab_zoom.get(tab).copied().unwrap_or(1.0);
                    if let Some(session) = state.servo_sessions.get(&tab) {
                        if (session.zoom() - level).abs() > 0.001 {
                            session.set_zoom(level);
                        }
                    }
                }
                if tab < state.tab_urls.len() && !url.is_empty() {
                    // If the page moved on by itself (a link, a redirect), a
                    // control it had open went with it.
                    controls::dismiss_if_moved(state, tab, &url);
                    state.tab_urls[tab] = url;
                }
            }
        }
        FerriteBrowserMessage::ToggleAuditPanel => {
            state.show_audit_panel = !state.show_audit_panel;
            if state.show_audit_panel {
                state.show_js_console = false;
                state.show_library_panel = false;
                state.show_settings_panel = false;
                refresh_audit_views(state);
            }
        }
        FerriteBrowserMessage::ToggleJsConsole => {
            state.show_js_console = !state.show_js_console;
            if state.show_js_console {
                state.show_audit_panel = false;
                state.show_library_panel = false;
                state.show_settings_panel = false;
                // The prompt takes the keyboard when the Console is showing.
                if state.devtools.tab == devtools::DevTab::Console {
                    state.address_bar_focused = false;
                    state.devtools.input_focused = true;
                    return text_input::focus(text_input::Id::new(JS_INPUT_ID));
                }
            } else {
                state.devtools.input_focused = false;
            }
        }
        FerriteBrowserMessage::SetAuditTab(tab) => {
            state.audit_tab = tab;
            refresh_audit_views(state);
        }
        FerriteBrowserMessage::RefreshTrace => {
            refresh_audit_views(state);
        }
        FerriteBrowserMessage::ToggleTraceEvent(seq) => {
            state.trace_expanded = (state.trace_expanded != Some(seq)).then_some(seq);
        }
        FerriteBrowserMessage::WindowCloseRequested => {
            // If the engine is wedged, shutdown can wait on it forever; the
            // process ends after a few seconds whatever happens.
            lifecycle::exit_soon();
            // Every session holds a handle to the engine; Servo writes the
            // profile (cookies, HSTS, credentials) when the last one goes.
            state.servo_sessions.clear();
            ferrite_servo::session::shutdown_engine();
            return iced::exit();
        }
        FerriteBrowserMessage::ClearTrace => {
            ferrite_model::trace::global().clear();
            state.trace_events.clear();
            state.trace_expanded = None;
        }
        FerriteBrowserMessage::RefreshAuditLog => {
            load_security_log(state);
        }
        FerriteBrowserMessage::NewTabSearchChanged(s) => {
            state.new_tab_search_input = s;
        }
        FerriteBrowserMessage::FocusAddressBar => {
            return focus_address_bar(state);
        }
        FerriteBrowserMessage::AddressBarPressed => {
            // The press that focuses the address bar selects the whole of it,
            // as in every other browser; once it has focus, clicks place the
            // caret as usual. (`widgets::PressProbe` reports the press: the
            // text input itself consumes it, so a `mouse_area` around it
            // never saw it.)
            if !state.address_bar_focused {
                state.address_bar_focused = true;
                return text_input::select_all(text_input::Id::new(ADDRESS_BAR_ID));
            }
        }
        FerriteBrowserMessage::ClearAddressBarFocus => {
            state.address_bar_focused = false;
        }
        FerriteBrowserMessage::CloseActiveTab => {
            let i = state.active_tab;
            return Task::done(FerriteBrowserMessage::CloseTab(i));
        }
        FerriteBrowserMessage::EscapePressed => {
            // C3d: the find bar, if open, is the most specific/topmost
            // thing Escape can mean — closing it takes priority over
            // Escape's pre-existing meanings below, the same "closest,
            // most specific overlay wins" convention every mainstream
            // browser's own find bar follows. `handle_key_press` cannot
            // make this decision itself (`keyboard::on_key_press` requires
            // a plain `fn` pointer with no `&FerriteBrowser` access — see
            // that function's own doc comment), so both cases route through
            // this one `EscapePressed` message and are told apart here,
            // where `state` is actually available.
            if let Some(prompt) = permission::active_prompt(state) {
                // A request for the camera, microphone or screen is on top of
                // everything; Escape is "block this time".
                let answer = permission::choice(prompt, false, false);
                return permission::update(state, permission::Msg::Answer(answer));
            } else if controls::active_control(state).is_some() {
                // A control the page is waiting on is the topmost thing there
                // is; Escape is "dismiss" (cancel, for a dialog).
                return controls::update(state, controls::Msg::Dismiss);
            } else if state.show_menu {
                state.show_menu = false;
            } else if state.show_find_bar {
                // Applied directly (not via `Task::done(CloseFindBar)`) so
                // a single `update()` call — the shape every test in this
                // module already calls `update()` with — observes the
                // effect immediately, exactly like the `is_loading`/
                // `address_bar_focused` branches below.
                state.show_find_bar = false;
                if let Some(session) = state.servo_sessions.get_mut(&state.active_tab) {
                    let _ = session.execute_js(FIND_CLEAR_SCRIPT);
                }
                state.find_query.clear();
                state.find_match_count = 0;
                state.find_current_index = 0;
            } else if state.pending_runtime.is_some() {
                // The agent is waiting on a question: Escape is "no", the
                // safe answer, never "yes".
                return update(state, FerriteBrowserMessage::RuntimeDeny);
            } else if state.pending_diff.is_some() {
                return update(state, FerriteBrowserMessage::ConsentCancelled);
            } else if state.is_loading {
                return Task::done(FerriteBrowserMessage::StopLoading);
            } else {
                state.address_bar_focused = false;
                return widgets::unfocus();
            }
        }
        FerriteBrowserMessage::JsInputChanged(s) => {
            state.devtools.input = s;
        }
        FerriteBrowserMessage::JsConsoleClear => {
            return devtools::update(state, devtools::Msg::Clear);
        }
        FerriteBrowserMessage::JsExecuteRequested => {
            devtools::run_prompt(state);
            return scrollable::snap_to(
                devtools::console_scroll_id(),
                scrollable::RelativeOffset::END,
            );
        }
        FerriteBrowserMessage::DevTools(msg) => return devtools::update(state, msg),
        FerriteBrowserMessage::Control(msg) => return controls::update(state, msg),
        FerriteBrowserMessage::Permission(msg) => return permission::update(state, msg),
        FerriteBrowserMessage::Crash(msg) => return crash::update(state, msg),
        FerriteBrowserMessage::Panels(msg) => layout::update(state, msg),
        FerriteBrowserMessage::WindowResized(size) => {
            state.window_size = size;
        }
        // ── Servo mouse/scroll events ──────────────────────────────────────
        FerriteBrowserMessage::ServoMouseMove { x, y } => {
            // Only record the position; the next `ServoFrame` tick forwards
            // it. A mouse reports far more moves than there are frames, and
            // each one used to be an engine call of its own.
            state.cursor_pos = (x, y);
            if page_input_blocked(state) {
                return Task::none();
            }
            state.pointer_moved = true;
            wake(state);
        }
        FerriteBrowserMessage::ServoRightPress => {
            if page_input_blocked(state) {
                return Task::none();
            }
            state.address_bar_focused = false;
            wake(state);
            let (x, y) = state.cursor_pos;
            let scale = state.scale_factor;
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                session.send_mouse_move(x * scale, y * scale);
                session.send_right_click(x * scale, y * scale);
            }
        }
        FerriteBrowserMessage::ServoMousePress => {
            if page_input_blocked(state) {
                return Task::none();
            }
            state.devtools.input_focused = false;
            // A click on the page takes keyboard focus from the address bar
            // (the bar's own widget unfocuses itself, but this flag is what
            // `PageKey` consults).
            state.address_bar_focused = false;
            wake(state);
            let (x, y) = state.cursor_pos;
            let scale = state.scale_factor;
            state.pointer_moved = false;
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                // Re-assert the pointer position first so the press hit-tests
                // where the cursor visually is even if no move event preceded it.
                session.send_mouse_move(x * scale, y * scale);
                session.send_mouse_down(x * scale, y * scale);
            }
        }
        FerriteBrowserMessage::PageKey(event) => {
            // A request for the camera, microphone or screen takes the keyboard (the
            // page must not be able to type at, or click through, a card meant for the
            // person); so does a control the page is waiting on.
            if permission::active_prompt(state).is_some() {
                return iced::Task::none();
            }
            if controls::active_control(state).is_some() {
                return controls::on_page_key(state, &event);
            }
            // Up and down at the DevTools prompt walk its history (the text
            // input leaves those keys unhandled, so they land here).
            if state.devtools.input_focused && state.show_js_console {
                if event.down {
                    match &event.key {
                        ferrite_servo::session::PageKey::Named(
                            ferrite_servo::session::PageNamedKey::ArrowUp,
                        ) => devtools::recall(state, true),
                        ferrite_servo::session::PageKey::Named(
                            ferrite_servo::session::PageNamedKey::ArrowDown,
                        ) => devtools::recall(state, false),
                        _ => {}
                    }
                }
                return Task::none();
            }
            if let Some(session) = page_key_target(state) {
                session.send_key(&event);
                wake(state);
            }
        }
        FerriteBrowserMessage::ServoMouseRelease => {
            if page_input_blocked(state) {
                return Task::none();
            }
            wake(state);
            let (x, y) = state.cursor_pos;
            let scale = state.scale_factor;
            if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                // One down (ServoMousePress) + one up is a complete click:
                // Servo raises `click` itself. A second synthesised
                // down/up pair here made every click arrive twice, so
                // checkboxes and radios toggled back and links fired twice.
                session.send_mouse_up(x * scale, y * scale);
            }
        }
        FerriteBrowserMessage::ServoScroll(wheel) => {
            if page_input_blocked(state) {
                return Task::none();
            }
            // Queued, not sent: pixel deltas are summed and notches eased out
            // over the next frames by the `ServoFrame` tick (see `scroll`).
            state.scroll_queue.push(wheel, state.scale_factor);
            wake(state);
        }
        FerriteBrowserMessage::ScaleFactorReady(factor) => {
            state.scale_factor = factor;
            // Tell the engine, or pages lay out as if the screen were `factor`
            // times wider than it is (see `ferrite_servo::session::set_display_scale`).
            ferrite_servo::session::set_display_scale(factor);
            for session in state.servo_sessions.values() {
                session.apply_display_scale();
            }
        }
        // ── Agent sidebar ─────────────────────────────────────────────────────
        FerriteBrowserMessage::ToggleAgentSidebar => {
            state.show_agent_sidebar = !state.show_agent_sidebar;
            if state.show_agent_sidebar {
                state.show_library_panel = false;
                state.show_settings_panel = false;
                // Opening the panel puts the cursor in the composer, ready to
                // type, with the thread at its newest message.
                if state.sidebar_view == SidebarView::Thread {
                    state.address_bar_focused = false;
                    return Task::batch([
                        text_input::focus(text_input::Id::new(AGENT_INPUT_ID)),
                        scroll_to_latest(state),
                    ]);
                }
            } else {
                // Nothing animates (or ticks) while the panel is hidden.
                state.thread_anims.clear();
            }
        }
        FerriteBrowserMessage::AgentTaskInputChanged(s) => {
            state.agent_task_input = s;
        }
        FerriteBrowserMessage::AgentTaskSubmitted => return submit_task(state),
        FerriteBrowserMessage::AgentToolLogged(s) => {
            state.agent_log.push(AgentLogEntry::Note(s));
        }
        FerriteBrowserMessage::AgentCompleted(s) => {
            return conclude_run(state, Outcome::Answered(s));
        }
        FerriteBrowserMessage::AgentFailed(s) => {
            return conclude_run(state, Outcome::Failed(s));
        }
        FerriteBrowserMessage::SigninContinue => {
            let (Some(wall), Some(mut live)) =
                (state.signin_handoff.take(), state.live_loop.take())
            else {
                return Task::none();
            };
            activity::record(
                "sign-in handoff",
                &wall.host,
                "continued by the user",
                true,
                "",
                0,
            );
            live.signin_cleared_host = Some(wall.host.clone());
            // Tell the model what happened, in the observation it is about to
            // read: it must not assume the sign-in succeeded or failed, only
            // that the person has finished with the page.
            let note = format!(
                "Observation: the user handled the sign-in or secret prompt on {} themselves \
                 and pressed Continue. Look at the page and carry on with the task.",
                wall.host
            );
            match live.messages.last_mut() {
                Some(last) if last.role == ferrite_model::Role::User => {
                    last.content.push_str("\n\n");
                    last.content.push_str(&note);
                }
                _ => live.messages.push(Message::user(note)),
            }
            let run_id = state.run_id;
            return spawn_next_step(state, run_id, live);
        }
        FerriteBrowserMessage::RuntimeAllowOnce
        | FerriteBrowserMessage::RuntimeAllowTask
        | FerriteBrowserMessage::RuntimeDeny => {
            let (Some(pending), Some(mut live)) =
                (state.pending_runtime.take(), state.live_loop.take())
            else {
                return Task::none();
            };
            let decision = match message {
                FerriteBrowserMessage::RuntimeAllowOnce => "allowed once",
                FerriteBrowserMessage::RuntimeAllowTask => "allowed for this task",
                _ => "denied",
            };
            activity::record(
                "runtime consent",
                pending.ask.summary.as_str(),
                decision,
                true,
                "the person was asked",
                0,
            );
            live.guard_choice = match message {
                FerriteBrowserMessage::RuntimeDeny => runtime_guard::GuardChoice::Deny,
                FerriteBrowserMessage::RuntimeAllowOnce => runtime_guard::GuardChoice::AllowOnce,
                _ => {
                    // Approve the tool or site for the rest of the task, the
                    // same approval the consent panel gives before a run.
                    live.guard = live.guard.take().map(|guard| {
                        guard.with_approvals(
                            pending.ask.approve_tool.clone(),
                            pending.ask.approve_origin.clone(),
                        )
                    });
                    runtime_guard::GuardChoice::Ask
                }
            };
            state.live_loop = Some(live);
            return handle_agent_step(state, pending.run_id, Ok(pending.action), pending.fast);
        }
        FerriteBrowserMessage::StopAgent => {
            state.run_id += 1;
            if let Some(handle) = state.agent_handle.take() {
                handle.abort();
            }
            return conclude_run(state, Outcome::Cancelled);
        }
        // ── Chats ─────────────────────────────────────────────────────────────
        FerriteBrowserMessage::NewChat => return new_chat(state),
        FerriteBrowserMessage::OpenChat(id) => return open_chat(state, &id),
        FerriteBrowserMessage::RequestDeleteChat(id) => {
            state.pending_chat_delete = Some(id);
        }
        FerriteBrowserMessage::ConfirmDeleteChat => {
            if let Some(id) = state.pending_chat_delete.take() {
                delete_chat(state, &id);
            }
        }
        FerriteBrowserMessage::CancelDeleteChat => {
            state.pending_chat_delete = None;
        }
        FerriteBrowserMessage::SetSidebarView(view) => {
            state.sidebar_view = view;
            state.pending_chat_delete = None;
            // The thread's scrollable is rebuilt when it comes back: put it at
            // the newest message rather than the top.
            return scroll_to_latest(state);
        }
        FerriteBrowserMessage::DismissNotice => {
            state.panel_notice = None;
        }
        FerriteBrowserMessage::CycleContextMode => {
            state.context_mode = agent_run::next_context_mode(state.context_mode);
        }
        FerriteBrowserMessage::SuggestionChosen(text) => {
            state.agent_task_input = text;
            return text_input::focus(text_input::Id::new(AGENT_INPUT_ID));
        }
        FerriteBrowserMessage::ThreadScrolled { pinned } => {
            state.thread_pinned = pinned;
        }
        FerriteBrowserMessage::ScrollThreadToBottom => {
            state.thread_pinned = true;
            return scroll_to_latest(state);
        }
        FerriteBrowserMessage::ThreadAnimTick => {
            agent_panel::advance_anims(&mut state.thread_anims, THREAD_ANIM_STEP);
        }
        FerriteBrowserMessage::CopyAnswer(text) => {
            return iced::clipboard::write(text);
        }
        FerriteBrowserMessage::ToggleExpand(key) => {
            if !state.expanded.remove(&key) {
                state.expanded.insert(key);
            }
        }
        // ── IPI consent handlers ──────────────────────────────────────────────
        FerriteBrowserMessage::ConsentRequired {
            diff,
            expected,
            evidence,
        } => {
            activity::record(
                "consent requested",
                "",
                &format!("{diff:?}"),
                true,
                "the agent's plan deviates from the predicted behaviour",
                0,
            );
            state.pending_diff = Some(diff);
            state.pending_expected = Some(expected);
            state.pending_evidence = Some(*evidence);
            state.pending_decision = ConsentDecision::default();
            state.show_evidence = false;
            state.agent_is_running = false;
            // pending_diff just transitioned None -> Some: (re)start the
            // panel's entrance animation from the beginning (C1).
            state.consent_panel_anim = 0.0;
        }
        FerriteBrowserMessage::ApproveTool(id) => {
            state.pending_decision.approve(ToolId::new(&id));
        }
        FerriteBrowserMessage::RejectTool(id) => {
            state.pending_decision.reject(ToolId::new(&id));
        }
        FerriteBrowserMessage::ToggleEvidence => {
            state.show_evidence = !state.show_evidence;
        }
        FerriteBrowserMessage::ConsentSubmitted => {
            // Defense in depth: the view only ever emits this message once
            // `consent_is_complete` holds (the Proceed button is disabled
            // otherwise), but this handler re-checks it directly rather than
            // trusting the view — ConsentSubmitted must never be reachable
            // with an undecided item, from any caller.
            let Some(diff) = state.pending_diff.as_ref() else {
                return Task::none();
            };
            if !consent_is_complete(diff, &state.pending_decision) {
                return Task::none();
            }

            activity::record(
                "consent decision",
                "",
                &format!("{} rejected item(s)", state.pending_decision.rejected.len()),
                true,
                "user chose Proceed",
                0,
            );
            let all_rejected = state.pending_decision.rejected.clone();
            // What the user approved is allowed through the guard; everything
            // else outside the prediction stays blocked.
            let (approved_origin_items, approved_tools): (Vec<ToolId>, Vec<ToolId>) = state
                .pending_decision
                .approved
                .iter()
                .cloned()
                .partition(|t| origin_item_origin(t).is_some());
            let approved_origins: Vec<String> = approved_origin_items
                .iter()
                .filter_map(|t| origin_item_origin(t).map(str::to_string))
                .collect();
            let guard = state.pending_expected.clone().map(|expected| {
                ferrite_ipi::comparator::RuntimeGuard::new(expected)
                    .with_approvals(approved_tools, approved_origins)
            });
            let rejected_origins: std::collections::HashSet<String> = all_rejected
                .iter()
                .filter_map(|t| origin_item_origin(t).map(str::to_string))
                .collect();
            let rejected: std::collections::HashSet<ToolId> = all_rejected
                .into_iter()
                .filter(|t| origin_item_origin(t).is_none())
                .collect();

            // Every field scoped to this consent decision is cleared here —
            // approvals/rejections are per-task, never sticky across tasks.
            state.pending_diff = None;
            state.pending_expected = None;
            state.pending_evidence = None;
            state.pending_decision = ConsentDecision::default();
            state.show_evidence = false;

            let prompt = match state.pending_task.take() {
                Some(p) => p,
                // Nothing to run (the run was already concluded): make sure
                // the chat does not keep a turn "in progress" forever.
                None => return conclude_run(state, Outcome::Cancelled),
            };
            state.run_id += 1;
            let run_id = state.run_id;
            return start_live_loop(state, run_id, prompt, rejected, rejected_origins, guard);
        }
        FerriteBrowserMessage::ConsentCancelled => {
            activity::record(
                "consent decision",
                "",
                "cancelled",
                true,
                "user cancelled the run",
                0,
            );
            // Same clearing as ConsentSubmitted — cancelling must leave no
            // trace of this task's pending decision behind either.
            state.pending_diff = None;
            state.pending_expected = None;
            state.pending_evidence = None;
            state.pending_decision = ConsentDecision::default();
            state.show_evidence = false;
            state.run_id += 1;
            // The user declined to let the run proceed: the turn ends as
            // cancelled (and `pending_task` is cleared by the funnel).
            return conclude_run(state, Outcome::Cancelled);
        }
        // ── Live agent loop (T-224) ──────────────────────────────────────────
        FerriteBrowserMessage::LiveRunReady {
            run_id,
            prompt,
            guard,
        } => {
            if run_id != state.run_id {
                return Task::none();
            }
            return start_live_loop(
                state,
                run_id,
                prompt,
                std::collections::HashSet::new(),
                std::collections::HashSet::new(),
                guard.map(|g| *g),
            );
        }
        FerriteBrowserMessage::AgentStepReady { run_id, action } => {
            return handle_agent_step(state, run_id, action, None);
        }
        FerriteBrowserMessage::FastStepReady {
            run_id,
            action,
            fast,
            history,
        } => {
            return handle_agent_step(state, run_id, Ok(action), Some((fast, history)));
        }
        // ── Agent bridge ──────────────────────────────────────────────────────
        FerriteBrowserMessage::ServoReady => {}
        FerriteBrowserMessage::ServoWatchdog => {
            // The frame-synchronised tick normally runs this; it only has to
            // when no display frame has come for a while.
            let quiet = state
                .last_tick
                .is_none_or(|at| at.elapsed() >= WATCHDOG_AFTER);
            if quiet {
                return update(state, FerriteBrowserMessage::ServoFrame);
            }
        }
        FerriteBrowserMessage::ServoFrame => {
            lifecycle::heartbeat();
            let now = std::time::Instant::now();
            // Animation advances by elapsed time, so a 120 Hz display does not
            // run the loading bar twice as fast as a 60 Hz one.
            let elapsed = state
                .last_tick
                .map_or(ACTIVE_TICK, |at| now.duration_since(at))
                .min(MAX_TICK_STEP);
            state.last_tick = Some(now);
            state.progress_offset =
                (state.progress_offset + elapsed.as_secs_f32() * PROGRESS_PER_SECOND) % 1.0;
            let mut tasks: Vec<Task<FerriteBrowserMessage>> = Vec::new();

            // Forward this tick's pointer position and scroll to the engine:
            // at most one move and one wheel event per frame, whatever the
            // input device's own rate. Positions are logical points; the
            // engine counts physical pixels (see `scale_factor`). While a
            // control the page is waiting on (or a panel drag) has the pointer,
            // the page gets none of it.
            {
                let (x, y) = state.cursor_pos;
                let scale = state.scale_factor;
                let moved = std::mem::take(&mut state.pointer_moved);
                let wheel = state.scroll_queue.next_frame();
                if !page_input_blocked(state) {
                    if let Some(session) = state.servo_sessions.get(&state.active_tab) {
                        if moved {
                            session.send_mouse_move(x * scale, y * scale);
                        }
                        if let Some((dx, dy)) = wheel {
                            session.send_scroll(x * scale, y * scale, f64::from(dx), f64::from(dy));
                        }
                    }
                }
            }

            // Keep the active tab's Servo render buffer matched to the real
            // content-area size (see `content_area_size`'s doc comment).
            // Runs every tick rather than off a dedicated resize event, so it
            // also picks up a size change caused by toggling or dragging a
            // panel, not only a window resize — those never fire a
            // window-level resize event at all. Skipped entirely while a
            // previous resize is still settling (`resize_settle_ticks > 0`) —
            // see that field's doc comment for the real crash this avoids;
            // `resize()` is never called again until the settle window from
            // the last one has fully elapsed.
            if state.resize_settle_ticks == 0 {
                let logical = state.content_area_size.get();
                let scale = state.scale_factor;
                let desired = (
                    (logical.width * scale).round().max(1.0) as u32,
                    (logical.height * scale).round().max(1.0) as u32,
                );
                if desired != state.last_resized_content_px {
                    if let Some(session) = state.servo_sessions.get_mut(&state.active_tab) {
                        session.resize(desired.0, desired.1);
                        state.last_resized_content_px = desired;
                        state.resize_settle_ticks = RESIZE_SETTLE_TICKS;
                    }
                }
            }

            // Tabs pages opened themselves (window.open, target=_blank, sign-in
            // popups) become real tabs, and the newest takes the front.
            for session in ferrite_servo::session::take_popup_sessions() {
                let index = push_tab_state(state);
                state.servo_sessions.insert(index, session);
                sync_active_webview(state);
            }

            // Pump engine once, then sync every tab's state and read pixels
            // — unless a resize just fired this tick (see above), in which
            // case every session's frame is skipped for `resize_settle_ticks`
            // more ticks rather than read against a buffer libservo may not
            // have finished reallocating yet. The previous frame stays
            // displayed in the meantime — a few tens of ms of an unchanged
            // image, not a black/corrupted one.
            let pump_started = std::time::Instant::now();
            if let Some(first) = state.servo_sessions.values().next() {
                first.pump_engine();
            }
            let read_started = std::time::Instant::now();
            if state.resize_settle_ticks > 0 {
                state.resize_settle_ticks -= 1;
            } else {
                let active_tab = state.active_tab;
                for (index, session) in state.servo_sessions.iter_mut() {
                    if *index == active_tab {
                        session.sync_and_read();
                    } else {
                        // A background tab's pixels are never shown.
                        session.sync_state();
                    }
                }
            }
            // Console messages, requests, crashes and page controls: emptied
            // from every session into the tabs' logs (and warnings and errors
            // to the log file, where the first clue to a misbehaving site is).
            let read_done = std::time::Instant::now();
            tasks.push(tab_diag::drain_all(state));
            let new_picture = refresh_frame_cache(state);
            if new_picture {
                state.busy_ticks = BUSY_TICKS;
            } else {
                state.busy_ticks = state.busy_ticks.saturating_sub(1);
            }
            if let Some(stats) = state.perf.as_mut() {
                let size = new_picture
                    .then(|| state.frame_cache.get(&state.active_tab))
                    .flatten()
                    .map(|(frame, _)| (frame.width, frame.height));
                let done = std::time::Instant::now();
                if let Some(line) = stats.record(
                    done,
                    read_started - pump_started,
                    read_done - read_started,
                    done - now,
                    size,
                ) {
                    eprintln!("{line}");
                }
            }
            let active = state.active_tab;
            if let Some(session) = state.servo_sessions.get(&active) {
                state.can_go_back = session.can_go_back();
                state.can_go_forward = session.can_go_forward();
                if let Some(title) = session.page_title() {
                    if active < state.tab_titles.len() && !title.is_empty() {
                        state.tab_titles[active] = title.to_string();
                    }
                }
                if let Some((w, h, bytes)) = session.get_favicon() {
                    // Only a different icon needs a new handle (and a new
                    // upload); the engine reports the same one every tick.
                    let key = favicon_key(w, h, &bytes);
                    if active < state.tab_favicons.len()
                        && state.favicon_keys.get(active).copied().flatten() != Some(key)
                    {
                        state.tab_favicons[active] = Some(ImageHandle::from_rgba(w, h, bytes));
                        if let Some(slot) = state.favicon_keys.get_mut(active) {
                            *slot = Some(key);
                        }
                    }
                }
                let is_now_loading = matches!(session.load_status(), LoadStatus::Loading);
                let new_url = session.current_url().to_string();
                let prev_url = state.tab_urls.get(active).cloned().unwrap_or_default();
                let status_changed = is_now_loading != state.is_loading;
                let url_changed =
                    !new_url.is_empty() && new_url != "about:blank" && new_url != prev_url;
                if status_changed || url_changed {
                    let status = if is_now_loading {
                        "loading"
                    } else {
                        "complete"
                    }
                    .to_string();
                    tasks.push(Task::done(FerriteBrowserMessage::LoadStatusChanged {
                        tab: active,
                        status,
                        url: new_url,
                    }));
                }
            }
            return Task::batch(tasks);
        }
        // ── C1 design-system messages ────────────────────────────────────────
        FerriteBrowserMessage::TabHoverEnter(i) => {
            state.hovered_tab = Some(i);
        }
        FerriteBrowserMessage::TabHoverExit(i) => {
            if state.hovered_tab == Some(i) {
                state.hovered_tab = None;
            }
        }
        // ── Chrome ───────────────────────────────────────────────────────────
        FerriteBrowserMessage::ToggleMenu => {
            state.show_menu = !state.show_menu;
            state.menu_anim = 0.0;
        }
        FerriteBrowserMessage::MenuAnimTick => {
            state.menu_anim = (state.menu_anim + MENU_ANIM_STEP).min(1.0);
        }
        FerriteBrowserMessage::CloseMenu => {
            state.show_menu = false;
        }
        FerriteBrowserMessage::Menu(command) => {
            state.show_menu = false;
            return update(state, command.message());
        }
        FerriteBrowserMessage::OpenLibrary(tab) => {
            if !state.show_library_panel {
                let _ = update(state, FerriteBrowserMessage::ToggleLibraryPanel);
            }
            state.library_tab = tab;
        }
        FerriteBrowserMessage::TitleBarPressed => {
            let now = std::time::Instant::now();
            let double = chrome::is_double_click(state.last_titlebar_press, now);
            state.last_titlebar_press = (!double).then_some(now);
            return window::get_latest().then(move |id| match id {
                Some(id) if double => window::toggle_maximize(id),
                Some(id) => window::drag(id),
                None => Task::none(),
            });
        }
        FerriteBrowserMessage::NextTab => {
            let count = state.tabs.len();
            select_tab_at(state, (state.active_tab + 1) % count.max(1));
            return widgets::unfocus();
        }
        FerriteBrowserMessage::PrevTab => {
            let count = state.tabs.len().max(1);
            select_tab_at(state, (state.active_tab + count - 1) % count);
            return widgets::unfocus();
        }
        FerriteBrowserMessage::SelectTabNumber(i) => {
            select_tab_at(state, i);
            return widgets::unfocus();
        }
        FerriteBrowserMessage::SelectLastTab => {
            select_tab_at(state, state.tabs.len().saturating_sub(1));
            return widgets::unfocus();
        }
        FerriteBrowserMessage::Noop => {}
        FerriteBrowserMessage::ConsentPanelTick => {
            state.consent_panel_anim = (state.consent_panel_anim + CONSENT_ANIM_STEP).min(1.0);
        }
        // ── C3c: theme ───────────────────────────────────────────────────────
        FerriteBrowserMessage::ToggleTheme => {
            state.theme_mode = state.theme_mode.toggled();
        }
        // ── C3d: bookmarks ──────────────────────────────────────────────────
        FerriteBrowserMessage::ToggleBookmarkCurrentPage => {
            let url = state
                .tab_urls
                .get(state.active_tab)
                .cloned()
                .unwrap_or_default();
            if url.is_empty() || url == "about:blank" {
                return Task::none();
            }
            if let Some(pos) = state.bookmarks.iter().position(|b| b.url == url) {
                state.bookmarks.remove(pos);
            } else {
                let title = state
                    .tab_titles
                    .get(state.active_tab)
                    .cloned()
                    .filter(|t| !t.is_empty() && t != "New Tab")
                    .unwrap_or_else(|| url.clone());
                state.bookmarks.push(Bookmark { title, url });
            }
            persist_bookmarks(state);
        }
        FerriteBrowserMessage::RemoveBookmark(index) => {
            if index < state.bookmarks.len() {
                state.bookmarks.remove(index);
                persist_bookmarks(state);
            }
        }
        // ── C3d: history ─────────────────────────────────────────────────────
        FerriteBrowserMessage::ClearHistory => {
            state.history.clear();
        }
        // ── C3d: zoom ───────────────────────────────────────────────────────
        FerriteBrowserMessage::ZoomIn => {
            let current = state.tab_zoom.get(state.active_tab).copied().unwrap_or(1.0);
            apply_zoom(state, next_zoom_level(current));
        }
        FerriteBrowserMessage::ZoomOut => {
            let current = state.tab_zoom.get(state.active_tab).copied().unwrap_or(1.0);
            apply_zoom(state, prev_zoom_level(current));
        }
        FerriteBrowserMessage::ZoomReset => {
            apply_zoom(state, 1.0);
        }
        FerriteBrowserMessage::SetDefaultZoom(level) => {
            state.default_zoom = level;
        }
        // ── C3d: find-in-page ───────────────────────────────────────────────
        FerriteBrowserMessage::OpenFindBar => {
            state.show_find_bar = true;
            state.find_query.clear();
            state.find_match_count = 0;
            state.find_current_index = 0;
            state.address_bar_focused = false;
            return text_input::focus(text_input::Id::new(FIND_INPUT_ID));
        }
        FerriteBrowserMessage::CloseFindBar => {
            state.show_find_bar = false;
            if let Some(session) = state.servo_sessions.get_mut(&state.active_tab) {
                let _ = session.execute_js(FIND_CLEAR_SCRIPT);
            }
            state.find_query.clear();
            state.find_match_count = 0;
            state.find_current_index = 0;
        }
        FerriteBrowserMessage::FindQueryChanged(s) => {
            state.find_query = s;
            run_find(state);
        }
        FerriteBrowserMessage::FindNext => {
            find_navigate(state, true);
        }
        FerriteBrowserMessage::FindPrevious => {
            find_navigate(state, false);
        }
        // ── C3d: downloads ──────────────────────────────────────────────────
        FerriteBrowserMessage::DownloadCurrentPage => {
            let url = state
                .tab_urls
                .get(state.active_tab)
                .cloned()
                .unwrap_or_default();
            if url.is_empty() || url == "about:blank" {
                return Task::none();
            }
            let Some(dir) = state.downloads_dir.clone() else {
                return Task::none();
            };
            let Some(tx) = state.agent_event_tx.clone() else {
                return Task::none();
            };
            let dest = resolve_download_path(&dir, &url, &state.downloads);
            let id = state.next_download_id;
            state.next_download_id += 1;
            let file_name = dest
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "download".to_string());
            state.downloads.push(DownloadItem {
                id,
                url: url.clone(),
                file_name,
                path: dest.clone(),
                state: DownloadState::InProgress {
                    downloaded_bytes: 0,
                    total_bytes: None,
                },
            });
            tokio::task::spawn(run_download(id, url, dest, tx));
        }
        FerriteBrowserMessage::DownloadProgress {
            id,
            downloaded_bytes,
            total_bytes,
        } => {
            if let Some(item) = state.downloads.iter_mut().find(|d| d.id == id) {
                item.state = DownloadState::InProgress {
                    downloaded_bytes,
                    total_bytes,
                };
            }
        }
        FerriteBrowserMessage::DownloadCompleted { id } => {
            if let Some(item) = state.downloads.iter_mut().find(|d| d.id == id) {
                item.state = DownloadState::Completed;
            }
        }
        FerriteBrowserMessage::DownloadFailed { id, error } => {
            if let Some(item) = state.downloads.iter_mut().find(|d| d.id == id) {
                item.state = DownloadState::Failed(error);
            }
        }
        // ── C3d: Library panel ──────────────────────────────────────────────
        FerriteBrowserMessage::ToggleLibraryPanel => {
            state.show_library_panel = !state.show_library_panel;
            if state.show_library_panel {
                state.show_audit_panel = false;
                state.show_js_console = false;
                // The right-hand drawers: one at a time.
                state.show_agent_sidebar = false;
                state.show_settings_panel = false;
            }
        }
        // ── Settings drawer ─────────────────────────────────────────────────
        FerriteBrowserMessage::ToggleSettingsPanel => {
            state.show_settings_panel = !state.show_settings_panel;
            if state.show_settings_panel {
                state.show_audit_panel = false;
                state.show_js_console = false;
                state.show_library_panel = false;
                state.show_agent_sidebar = false;
                settings_panel::on_open(state);
            }
        }
        FerriteBrowserMessage::Settings(message) => settings_panel::update(state, message),
        FerriteBrowserMessage::SelectLibraryTab(tab) => {
            state.library_tab = tab;
        }
        // ── New-tab hero: quick-access tile favicons ─────────────────────────
        FerriteBrowserMessage::FetchTileFavicons => {
            if let (Some(cache_dir), Some(tx)) = (
                state.favicons_cache_dir.clone(),
                state.agent_event_tx.clone(),
            ) {
                for (index, tile) in QUICK_ACCESS_TILES.iter().enumerate() {
                    let Some(host) = favicon_host(tile.url) else {
                        continue;
                    };
                    let cache_path = favicon_cache_path(&cache_dir, &host);
                    let favicon_url = format!("https://{host}/favicon.ico");
                    tokio::task::spawn(fetch_tile_favicon(
                        index,
                        favicon_url,
                        cache_path,
                        tx.clone(),
                    ));
                }
            }
        }
        FerriteBrowserMessage::TileFaviconReady {
            index,
            width,
            height,
            rgba,
        } => {
            if let Some(slot) = state.tile_favicons.get_mut(index) {
                *slot = Some(ImageHandle::from_rgba(width, height, rgba));
            }
        }
    }
    Task::none()
}

// ---------------------------------------------------------------------------
// Chats and the run lifecycle
// ---------------------------------------------------------------------------

/// `text_input::Id` of the agent composer, so New chat, a suggestion chip and
/// opening the sidebar can put the cursor there.
const AGENT_INPUT_ID: &str = "ferrite_agent_input";

/// Whether the user may switch away from (or delete) the current chat right
/// now. **Design choice:** switching is *blocked*, not "stop the run and
/// switch". A run is real work against real pages; a stray click on a history
/// row or a fumbled shortcut must not be able to cancel it, and blocking keeps
/// the invariant that the running chat is never reloaded from disk mid-run.
/// The view greys the controls and says why; the handlers re-check this so the
/// keyboard shortcut and any direct message obey it too.
fn chat_switch_blocked(state: &FerriteBrowser) -> bool {
    state.agent_is_running || state.pending_diff.is_some()
}

/// The notice shown when a chat switch is refused.
const SWITCH_BLOCKED_NOTICE: &str =
    "Stop the current run (or finish reviewing it) before switching chats.";

/// Clears the per-run live mirror and per-chat view state; used when the chat
/// on screen changes.
fn reset_chat_view(state: &mut FerriteBrowser) {
    state.agent_log.clear();
    state.agent_response = None;
    state.expanded.clear();
    state.pending_chat_delete = None;
    state.thread_anims.clear();
    state.thread_pinned = true;
}

/// Pulls the message thread to its newest item — but only while it is pinned
/// to the bottom (the user has not scrolled up to read), and only when the
/// thread is what is on screen. `scrollable::snap_to` on an id that is not in
/// the tree is a no-op, so this is safe to return from any handler.
fn scroll_to_latest(state: &FerriteBrowser) -> Task<FerriteBrowserMessage> {
    if should_snap_to_latest(state) {
        scrollable::snap_to(
            agent_panel::thread_scroll_id(),
            scrollable::RelativeOffset::END,
        )
    } else {
        Task::none()
    }
}

/// The gate behind [`scroll_to_latest`]: the thread is what is on screen and
/// the user has not scrolled up to read.
fn should_snap_to_latest(state: &FerriteBrowser) -> bool {
    state.thread_pinned
        && state.show_agent_sidebar
        && state.sidebar_view == SidebarView::Thread
        && state.pending_diff.is_none()
}

/// Starts the entrance animation of a thread item.
fn animate_item(state: &mut FerriteBrowser, slot: u32) {
    if let Some(turn) = state.chat.turns.len().checked_sub(1) {
        agent_panel::start_item_anim(&mut state.thread_anims, ItemKey { turn, slot });
    }
}

/// THE place a run ends. Every ending — finished, asked the user, model or
/// dry-run error, budget stops, the repeated-action stop, the Stop button,
/// consent cancelled — comes here with the turn's [`Outcome`]. It updates the
/// live mirror (`agent_response`, `agent_is_running`, `live_loop`,
/// `pending_task`), finishes the turn on the chat, then saves the chat and
/// refreshes the history list. A save failure is a notice, never fatal: the
/// in-memory chat is already right.
///
/// A no-op on the chat when no turn is running (finishing twice cannot
/// overwrite a real outcome — `Chat::finish_turn`'s own rule).
fn conclude_run(state: &mut FerriteBrowser, outcome: Outcome) -> Task<FerriteBrowserMessage> {
    activity::record("run finished", "", &format!("{outcome:?}"), true, "", 0);
    if let Some(text) = agent_run::outcome_mirror_text(&outcome) {
        state.agent_response = Some(text);
    }
    state.agent_is_running = false;
    state.live_loop = None;
    state.signin_handoff = None;
    state.pending_runtime = None;
    state.pending_task = None;
    state.pending_seed = None;
    // "Stop the current run before switching chats" is stale the moment the
    // run is over.
    if state.panel_notice.as_deref() == Some(SWITCH_BLOCKED_NOTICE) {
        state.panel_notice = None;
    }
    let had_running_turn = state.chat.has_running_turn();
    state.chat.finish_turn(outcome);
    if had_running_turn {
        persist_chat(state);
        animate_item(state, SLOT_OUTCOME);
    }
    scroll_to_latest(state)
}

/// Saves the current chat (atomically, via `ChatStore::save`) and keeps
/// `chat_list` in step, in memory. Does nothing without a store (`Default`,
/// tests, no home directory) or for a chat with no turns. A failure surfaces as
/// a non-blocking notice and a log line; it never stops anything.
fn persist_chat(state: &mut FerriteBrowser) {
    let Some(store) = state.chat_store.as_ref() else {
        return;
    };
    if state.chat.turns.is_empty() {
        return;
    }
    match store.save(&state.chat) {
        Ok(()) => agent_run::upsert_summary(&mut state.chat_list, state.chat.summary()),
        Err(e) => {
            eprintln!("[ferrite-ui] failed to save chat {}: {e}", state.chat.id);
            state.panel_notice = Some(format!("Couldn't save this chat: {e}"));
        }
    }
}

/// `NewChat`: a fresh empty chat (not written until its first message), the
/// thread view, the sidebar open and the cursor in the composer.
fn new_chat(state: &mut FerriteBrowser) -> Task<FerriteBrowserMessage> {
    if chat_switch_blocked(state) {
        state.panel_notice = Some(SWITCH_BLOCKED_NOTICE.to_string());
        return Task::none();
    }
    state.chat = Chat::new();
    reset_chat_view(state);
    state.panel_notice = None;
    state.sidebar_view = SidebarView::Thread;
    state.show_agent_sidebar = true;
    text_input::focus(text_input::Id::new(AGENT_INPUT_ID))
}

/// `OpenChat`: loads `id` from the store into the current chat. Blocked while
/// a run is active (see [`chat_switch_blocked`]); opening the chat already on
/// screen only switches to its thread. A chat that cannot be loaded is
/// reported and, if its file is gone, dropped from the list.
fn open_chat(state: &mut FerriteBrowser, id: &ChatId) -> Task<FerriteBrowserMessage> {
    // The chat already on screen is never reloaded — it may be mid-run, and
    // the in-memory copy is the truth. Tapping its row just shows the thread.
    if *id == state.chat.id {
        state.sidebar_view = SidebarView::Thread;
        return Task::none();
    }
    if chat_switch_blocked(state) {
        state.panel_notice = Some(SWITCH_BLOCKED_NOTICE.to_string());
        return Task::none();
    }
    let Some(store) = state.chat_store.as_ref() else {
        return Task::none();
    };
    match store.load(id) {
        Ok(chat) => {
            state.chat = chat;
            reset_chat_view(state);
            state.panel_notice = None;
            state.sidebar_view = SidebarView::Thread;
            Task::batch([
                text_input::focus(text_input::Id::new(AGENT_INPUT_ID)),
                scroll_to_latest(state),
            ])
        }
        Err(e) => {
            eprintln!("[ferrite-ui] failed to open chat {id}: {e}");
            if matches!(e, ferrite_agent::chat::ChatError::NotFound(_)) {
                state.chat_list.retain(|s| s.id != *id);
            }
            state.panel_notice = Some(format!("Couldn't open that chat: {e}"));
            Task::none()
        }
    }
}

/// `ConfirmDeleteChat`: deletes `id`'s file and its list row. The chat on
/// screen can be deleted too (when idle): it is replaced by a fresh empty one.
/// Deleting the running chat is refused like any other switch away from it.
fn delete_chat(state: &mut FerriteBrowser, id: &ChatId) {
    let is_current = *id == state.chat.id;
    if is_current && chat_switch_blocked(state) {
        state.panel_notice = Some(SWITCH_BLOCKED_NOTICE.to_string());
        return;
    }
    if let Some(store) = state.chat_store.as_ref() {
        if let Err(e) = store.delete(id) {
            eprintln!("[ferrite-ui] failed to delete chat {id}: {e}");
            state.panel_notice = Some(format!("Couldn't delete that chat: {e}"));
            return;
        }
    }
    state.chat_list.retain(|s| s.id != *id);
    if is_current {
        state.chat = Chat::new();
        reset_chat_view(state);
    }
}

/// `AgentTaskSubmitted`: starts a new turn on the current chat and spawns the
/// defense pipeline (fingerprint, dry run, compare) that ends in either
/// `LiveRunReady` or `ConsentRequired`. Ignored while a run is active or a
/// consent decision is pending, and for a blank message.
fn submit_task(state: &mut FerriteBrowser) -> Task<FerriteBrowserMessage> {
    if state.agent_is_running || state.pending_diff.is_some() {
        return Task::none();
    }
    let prompt = state.agent_task_input.trim().to_string();
    if prompt.is_empty() {
        return Task::none();
    }
    let event_tx = match state.agent_event_tx.clone() {
        Some(tx) => tx,
        None => return Task::none(),
    };

    // ── Context for this run ──────────────────────────────────────────────
    // The open tabs, and (when the message is about the current page) the
    // page itself, read on this thread through the borrowed engine. Fail-soft:
    // no digest just means the seed carries the tab list and a page header.
    // Whether the page is wanted is the deterministic `decide_page_use`
    // heuristic alone: Laya's `refine_page_use` second opinion is deliberately
    // left unwired (its own docs call it an unmeasured experiment, likely no
    // better than chance), so nothing here calls Laya for page relevance.
    let tabs = agent_run::tab_infos(
        &state.tab_titles,
        &state.tab_urls,
        state.active_tab,
        state.is_loading,
    );
    let active_url = state
        .tab_urls
        .get(state.active_tab)
        .cloned()
        .unwrap_or_default();
    let wants_page = ferrite_agent::context::decide_page_use(
        &prompt,
        &state.chat.turns,
        &active_url,
        state.context_mode,
    )
    .use_page;
    let digest = if wants_page {
        observe_active_page(state)
    } else {
        None
    };
    let context = agent_run::build_run_context(
        &state.chat,
        &tabs,
        digest.as_ref(),
        &prompt,
        state.context_mode,
    );

    state.chat.begin_turn(&prompt, Some(context.note));
    persist_chat(state);
    state.thread_pinned = true;
    animate_item(state, SLOT_USER);
    state.sidebar_view = SidebarView::Thread;
    state.panel_notice = None;
    state.agent_log.clear();
    state.agent_response = None;
    state.agent_task_input.clear();
    state.agent_is_running = true;
    state.run_id += 1;
    let run_id = state.run_id;
    activity::record(
        "run started",
        &prompt,
        &format!("page context mode: {:?}", state.context_mode),
        true,
        "",
        0,
    );

    let context_url = state.tab_urls.get(state.active_tab).cloned();
    // SECURITY — do not feed the seed to the defense. The injection defense's
    // premise is that the expected tool/origin fingerprint is predicted from
    // *trusted user intent alone*, and that whatever the agent then does is
    // compared against it. The seed holds earlier agent output, tab titles and
    // page text — all attacker-influenced — so it must never reach the
    // sanitizer, the fingerprint predictor or the dry run (which also runs on
    // synthetic data and must not be handed real page content or real chat
    // outputs). `ipi_task_for_run` builds the defense's input from
    // `trusted_task_text` (the user's own words, this message plus earlier user
    // messages) and nothing else. Only the LIVE loop's first message is the
    // seed, and it is held in `pending_seed` — this function's spawned task
    // below never captures it.
    let ipi_task: IpiTask = agent_run::ipi_task_for_run(&state.chat, &prompt, context_url.clone());
    state.pending_task = Some(prompt.clone());
    state.pending_seed = Some(context.seed.text);

    let provider = state.model_provider.clone();
    let model_tag_small = state.model_tag_small.clone();
    let model_tag_main = state.model_tag_main.clone();

    let handle = tokio::task::spawn(async move {
        // ── Defense-mode single decision point (Task 18) ──────────────
        // ToolDecisionEngine::new() reads FERRITE_DEFENSE once; On is the
        // unchanged default everywhere. Off skips straight to the real run
        // (no sanitizer, no dry-run, no consent). SanitizerOnly runs the
        // sanitizer but also skips straight to the real run. On runs the
        // sanitizer and continues into the existing fingerprint/dry-run/
        // compare/consent loop, unchanged.
        let engine = ToolDecisionEngine::new();
        let loop_outcome = engine.prepare_task(&ipi_task);
        let defense_mode = match &loop_outcome {
            LoopOutcome::Bypassed | LoopOutcome::RanSanitizerOnly { .. } => {
                // Defense off / sanitizer-only: no prediction, so no guard.
                let _ = event_tx.send(FerriteBrowserMessage::LiveRunReady {
                    run_id,
                    prompt: prompt.clone(),
                    guard: None,
                });
                return;
            }
            // LoopOnly and On both continue into the fingerprint/dry-run/
            // compare/consent loop below. They differ only in whether the
            // sanitizer ran first (On) or was bypassed (LoopOnly) — a
            // distinction the dry-run orchestrator now acts on directly
            // (T-215: set_defense_mode below), so both fall through here.
            LoopOutcome::RanLoopOnly => DefenseMode::LoopOnly,
            LoopOutcome::RanFullLoop { .. } => DefenseMode::On,
        };

        // ── IPI dry run ──────────────────────────────────────────────
        // T-224/T-229: `provider` is the real, live-configured
        // ModelProvider constructed at startup (or MockProvider,
        // fail-to-empty, if none is configured/reachable) — no
        // longer a hardcoded MockProvider::new() regardless of what
        // is actually available (T-229's exact fix).
        let fingerprint = engine
            .fingerprint_from_task(provider.as_ref(), &model_tag_small, &ipi_task)
            .await;
        let twin_path = std::env::temp_dir().join("ferrite-ipi-twin.enc");
        let mut orch = ferrite_ipi::dry_run::DryRunOrchestrator::new(twin_path);
        // T-215: derive detect_enabled AND strip_enabled together from
        // the mode this dry run is actually running under, instead of
        // leaving both at DryRunOrchestrator::new's defaults
        // (detect-only, strip off) regardless of mode.
        orch.set_defense_mode(defense_mode);
        // BrowserLoopDryRunDriver runs the real
        // `browser_loop::run_agent_loop` directly against the
        // synthetic `DryRunEngine` — see that type's own docs for
        // why this needs no GeminiAgent/EngineToolExecutor bridge
        // now that neither this loop nor the dry run's engine
        // depends on the old vocabulary.
        let driver = BrowserLoopDryRunDriver {
            provider: provider.as_ref(),
            model_tag: model_tag_main.clone(),
            // The defense's own trusted text — never the seed (see above).
            prompt: ipi_task.prompt.clone(),
        };
        let dry_record = match orch.run(&ipi_task, &driver).await {
            Ok(r) => r,
            Err(e) => {
                let _ = event_tx.send(FerriteBrowserMessage::AgentFailed(format!(
                    "dry run failed: {}",
                    e
                )));
                return;
            }
        };
        let _ = event_tx.send(FerriteBrowserMessage::AgentToolLogged(
            "[dry run complete — checking for unexpected activity]".to_string(),
        ));
        // No per-task per-capability origin-scope authoring exists yet
        // (T-001's live-path bridge, see ferrite_ipi::comparator's
        // module docs). The task's own declared context URL — already
        // threaded in above as `context_url` — narrows every
        // capability to an exact scope on it; task_open (which admits
        // any origin) is used only when no context URL is known at
        // all, never unconditionally.
        let context_origin = context_url
            .as_deref()
            .and_then(|url| ferrite_core::Origin::parse(url).ok());
        let expected = ExpectedFingerprint::from_fingerprint(&fingerprint, context_origin.as_ref());
        let diff = compare(&expected, &dry_record);
        // The real run is held to the same prediction (ADR-014): the dry run
        // saw only synthetic pages, so whatever an actual page provokes is
        // checked here, action by action.
        let guard = ferrite_ipi::comparator::RuntimeGuard::new(expected.clone());
        if !diff.is_clean() {
            let _ = event_tx.send(FerriteBrowserMessage::ConsentRequired {
                diff,
                expected,
                evidence: Box::new(dry_record),
            });
            return;
        }

        // ── Real run ─────────────────────────────────────────────────
        let _ = event_tx.send(FerriteBrowserMessage::LiveRunReady {
            run_id,
            prompt,
            guard: Some(Box::new(guard)),
        });
    });
    state.agent_handle = Some(handle);
    scroll_to_latest(state)
}

/// One step's model reply (or failure), or a fast-lane step: validates the
/// run, ends the run on a terminal action or a stop condition, otherwise
/// blocks or executes the action, logs and records it, and spawns the next
/// step. A fast-lane action (`fast`) is an ordinary [`AgentAction`] and takes
/// exactly this path — the repeated-action stop, the consent check
/// (`is_action_rejected`), execution, the step budget — so Laya can never
/// widen what a run is allowed to do or outrun its loop-safety limits.
fn handle_agent_step(
    state: &mut FerriteBrowser,
    run_id: u64,
    action: Result<AgentAction, StepFailure>,
    fast: Option<(FastAction, HistoryItem)>,
) -> Task<FerriteBrowserMessage> {
    if run_id != state.run_id {
        return Task::none();
    }
    // Rule: the agent does not act while the page's request for the camera, the
    // microphone or the screen waits on the person. Hold the step; `tab_diag` sends it
    // again when the request is answered or withdrawn.
    if permission::active_prompt(state).is_some() {
        state.deferred_agent_step = Some(match (fast, action) {
            (Some((fast, history)), Ok(action)) => FerriteBrowserMessage::FastStepReady {
                run_id,
                action,
                fast,
                history,
            },
            (_, action) => FerriteBrowserMessage::AgentStepReady { run_id, action },
        });
        return Task::none();
    }
    let Some(mut live) = state.live_loop.take() else {
        return Task::none();
    };

    let action = match action {
        Ok(a) => {
            live.consecutive_malformed = 0;
            a
        }
        Err(StepFailure::Model(reason)) => {
            return conclude_run(state, Outcome::Failed(reason));
        }
        Err(StepFailure::Malformed { raw, message }) => {
            live.consecutive_malformed += 1;
            if live.consecutive_malformed > MAX_CONSECUTIVE_MALFORMED_STEPS {
                return conclude_run(state, Outcome::Failed(format!("{message} (raw: {raw})")));
            }
            // Give the model a chance to self-correct — the same
            // retry-with-feedback shape `browser_loop::run_agent_loop` uses,
            // and for the same reason: a truncated or malformed response is
            // often a one-off glitch (e.g. a `finish.answer` cut short by the
            // provider's output cap) a model recovers from once told what was
            // wrong, so ending the whole task on the first one turned a
            // recoverable hiccup into a hard failure. Not counted against
            // `budget.max_steps` — no real browser action was taken — but
            // independently bounded by `MAX_CONSECUTIVE_MALFORMED_STEPS`.
            live.messages
                .push(Message::assistant(compact_observation(raw.trim())));
            live.messages.push(Message::user(format!(
                "Observation: your last response could not be parsed as a single, \
                 complete, valid JSON action ({message}). It may have been cut off \
                 or included extra text. Respond with EXACTLY ONE complete, valid \
                 JSON object and nothing else."
            )));
            trim_message_history(&mut live.messages);
            return spawn_next_step(state, run_id, live);
        }
    };

    if let AgentAction::Finish { answer } = action {
        return conclude_run(state, Outcome::Answered(answer));
    }
    // `ask_user` is terminal too and never reaches the engine; the question
    // ends the turn, and the user's next message answers it.
    if let AgentAction::AskUser { question } = action {
        return conclude_run(state, Outcome::AskedUser(question));
    }

    // Repeated-identical-action hard stop — mirrors
    // `browser_loop::run_agent_loop`'s own check exactly (fires *before*
    // executing the would-be Nth repeat).
    if live.budget.max_repeated_identical > 0 {
        let window = live.budget.max_repeated_identical - 1;
        if window <= live.actions_taken.len()
            && live.actions_taken[live.actions_taken.len() - window..]
                .iter()
                .all(|a| a == &action)
        {
            return conclude_run(
                state,
                Outcome::Stopped("the same action was about to repeat".to_string()),
            );
        }
    }

    // The predicted fingerprint is binding on the real run (ADR-014): an action
    // that is neither expected nor approved is not executed. The guard runs
    // first so a rejected-and-unexpected action is reported as the guard's
    // (the stronger statement) and so every blocked action is audited.
    let choice = std::mem::take(&mut live.guard_choice);
    let mut ask: Option<runtime_guard::RuntimeAsk> = None;
    let guard_block = live.guard.as_ref().and_then(|guard| {
        let tab_url = state
            .tab_urls
            .get(state.active_tab)
            .cloned()
            .unwrap_or_default();
        // Only a click by @ref needs the page: to see where a link leads.
        let digest = match &action {
            AgentAction::Click { selector } if selector.starts_with('@') => {
                observe_active_page(state)
            }
            _ => None,
        };
        let (verdict, effects) = runtime_guard::judge(guard, &action, &tab_url, digest.as_ref());
        let audited = effects.first().map_or_else(
            || (primitive_of_action(&action).as_str(), None),
            |(p, o)| (p.as_str(), o.as_deref()),
        );
        match runtime_guard::settle(&verdict, choice) {
            // Outside the prediction and nobody has said yes: the run waits
            // for the person (nothing is audited or run until they answer).
            runtime_guard::Settled::Ask => {
                ask = runtime_guard::ask_for(&verdict, action_label(&action));
                Some(verdict)
            }
            runtime_guard::Settled::Block => {
                ferrite_servo::session::audit_guard_decision(audited.0, audited.1, false);
                Some(verdict)
            }
            runtime_guard::Settled::Run { approved } => {
                if approved {
                    ferrite_servo::session::audit_guard_decision(audited.0, audited.1, true);
                }
                None
            }
        }
    });
    if let Some(ask) = ask {
        activity::record(
            "runtime consent",
            ask.summary.as_str(),
            "waiting for the person",
            true,
            "the agent reached outside what the request implied",
            0,
        );
        state.pending_runtime = Some(PendingRuntimeConsent {
            run_id,
            action,
            fast,
            ask,
        });
        state.live_loop = Some(live);
        return scroll_to_latest(state);
    }
    if guard_block.is_some() {
        live.guard_blocks += 1;
    }
    let blocked_by_consent = is_action_rejected(&action, &live.rejected, &live.rejected_origins);
    let blocked = blocked_by_consent || guard_block.is_some();
    let exec_started = std::time::Instant::now();
    let observation = if guard_block.is_some() {
        runtime_guard::BLOCKED_OBSERVATION.to_string()
    } else if blocked_by_consent {
        "blocked by user consent".to_string()
    } else if agent_run::is_tab_action(&action) {
        // The borrowed engine wraps one externally-owned tab and cannot do tab
        // management, so the UI performs these itself, through the same
        // helpers as the tab strip.
        run_tab_action(state, &action)
    } else if let Some(session) = state.servo_sessions.get_mut(&state.active_tab) {
        let mut engine = BorrowedServoEngine::new(session, 1280, 700);
        execute_action(&mut engine, &action)
    } else {
        "error: no active browser session".to_string()
    };

    let is_fast = fast.is_some();
    activity::record(
        "action",
        &serde_json::to_string(&action).unwrap_or_default(),
        &observation,
        !blocked && !observation.starts_with("error"),
        if guard_block.is_some() {
            "blocked by Ferrite's fingerprint guard"
        } else if blocked {
            "blocked by user consent"
        } else if is_fast {
            "chosen by Laya"
        } else {
            "chosen by the LLM"
        },
        u64::try_from(exec_started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    state.agent_log.push(AgentLogEntry::Step {
        icon: icon_for_action(&action),
        label: action_label(&action),
        detail: action_detail(&action),
        // The user sees why, in the guard's words; the agent sees only the
        // fixed `BLOCKED_OBSERVATION` (below, in `live.messages`).
        result: guard_block
            .as_ref()
            .map_or_else(|| observation.clone(), runtime_guard::block_detail),
        blocked,
        fast: is_fast,
    });
    let recorded_detail = agent_run::persisted_step_detail(&action);
    state.chat.record_step(StepRecord {
        label: action_label(&action).to_string(),
        detail: if is_fast {
            agent_run::mark_fast(&recorded_detail)
        } else {
            recorded_detail
        },
        result: agent_run::step_result_text(&observation),
        blocked,
    });
    // Laya bookkeeping. Even a blocked fast step counts as the previous one,
    // so Laya is not allowed to propose the same blocked click again.
    match fast {
        Some((fast_action, history)) => {
            live.history.push(history);
            live.previous_fast = Some(fast_action);
        }
        None => {
            live.history
                .push(agent_run::llm_history_item(&action, action_label(&action)));
            live.previous_fast = None;
        }
    }
    if live.history.len() > agent_run::MAX_HISTORY_ITEMS {
        let excess = live.history.len() - agent_run::MAX_HISTORY_ITEMS;
        live.history.drain(..excess);
    }
    live.messages.push(Message::assistant(
        serde_json::to_string(&action).unwrap_or_default(),
    ));
    let compacted_observation = compact_observation(&observation);
    live.messages.push(Message::user(format!(
        "Observation: {compacted_observation}"
    )));
    trim_message_history(&mut live.messages);
    live.actions_taken.push(action);
    if live.guard_blocks >= runtime_guard::MAX_GUARD_BLOCKS {
        return conclude_run(
            state,
            Outcome::Stopped(
                "the agent kept trying actions this task was not expected to need, so the run was stopped"
                    .to_string(),
            ),
        );
    }
    // The new step card slides in; keep the thread on the newest item.
    animate_item(
        state,
        SLOT_STEP_BASE + u32::try_from(state.agent_log.len() - 1).unwrap_or(0),
    );

    Task::batch([
        spawn_next_step(state, run_id, live),
        scroll_to_latest(state),
    ])
}

/// Loads the hash-chained network log every tab writes to. A missing file is
/// simply an empty log (nothing has been fetched yet).
fn load_security_log(state: &mut FerriteBrowser) {
    let path = ferrite_servo::session::audit_db_path();
    if !path.exists() {
        state.audit_entries.clear();
        return;
    }
    state.audit_entries = match PersistentAuditLog::load(&path.to_string_lossy()) {
        Ok(log) => log.log.entries,
        Err(e) => {
            eprintln!("[ferrite-ui] audit log load: {}", e);
            vec![]
        }
    };
}

/// Brings whichever Audit-panel view is showing up to date.
fn refresh_audit_views(state: &mut FerriteBrowser) {
    match state.audit_tab {
        AuditTab::Models => state.trace_events = ferrite_model::trace::global().snapshot(),
        AuditTab::Security => load_security_log(state),
    }
}

/// Saves `state.bookmarks` to `state.bookmarks_path` if one is set (real
/// startup, `launch()`) — a no-op, not a panic, when it isn't (every test/
/// `Default` construction, R7). A write failure is logged, not surfaced to
/// the UI — the in-memory `bookmarks` list (what the user actually sees) is
/// already correct either way; only the next restart would lose the change,
/// same class of degradation the audit log's own best-effort writes already
/// accept elsewhere in this crate.
fn persist_bookmarks(state: &FerriteBrowser) {
    if let Some(path) = &state.bookmarks_path {
        if let Err(e) = save_bookmarks_to(path, &state.bookmarks) {
            eprintln!("[ferrite-ui] failed to save bookmarks to {path:?}: {e}");
        }
    }
}

/// Sets the active tab's zoom level and applies it to the live page through
/// the engine's page zoom, if a session exists for that tab — the path
/// `ZoomIn`/`ZoomOut`/`ZoomReset` go through.
fn apply_zoom(state: &mut FerriteBrowser, level: f32) {
    let active = state.active_tab;
    if active < state.tab_zoom.len() {
        state.tab_zoom[active] = level;
    }
    if let Some(session) = state.servo_sessions.get(&active) {
        session.set_zoom(level);
    }
}

/// Runs `state.find_query` against the active tab's page (`find_script`) and
/// updates `find_match_count`/`find_current_index` from the result — the
/// live-search path (`FindQueryChanged`). A blank query clears any existing
/// highlights and zeroes both counters, same as `CloseFindBar`, but without
/// closing the bar itself.
fn run_find(state: &mut FerriteBrowser) {
    let query = state.find_query.clone();
    if query.is_empty() {
        if let Some(session) = state.servo_sessions.get_mut(&state.active_tab) {
            let _ = session.execute_js(FIND_CLEAR_SCRIPT);
        }
        state.find_match_count = 0;
        state.find_current_index = 0;
        return;
    }
    let Some(session) = state.servo_sessions.get_mut(&state.active_tab) else {
        return;
    };
    let raw = session.execute_js(&find_script(&query)).unwrap_or_default();
    apply_find_result(state, &raw);
}

/// `FindNext`/`FindPrevious` — moves the highlighted match without
/// re-running the whole-page search `run_find` does, matching every real
/// browser's find bar (next/previous is cheap; a fresh search is not).
fn find_navigate(state: &mut FerriteBrowser, forward: bool) {
    if state.find_match_count == 0 {
        return;
    }
    let Some(session) = state.servo_sessions.get_mut(&state.active_tab) else {
        return;
    };
    let raw = session
        .execute_js(&find_navigate_script(forward))
        .unwrap_or_default();
    apply_find_result(state, &raw);
}

/// Parses `raw` (whatever `HeadlessServoSession::execute_js` returned for a
/// `find_script`/`find_navigate_script` call — see `extract_json_object`'s
/// doc comment for why that parsing has to be defensive) and updates
/// `find_match_count`/`find_current_index` from it. Leaves both fields
/// unchanged (rather than zeroing them) if `raw` doesn't parse — a
/// malformed/unexpected response should not visibly reset an otherwise-valid
/// in-progress search.
fn apply_find_result(state: &mut FerriteBrowser, raw: &str) {
    let Some(value) = extract_json_object(raw) else {
        return;
    };
    if let Some(count) = value.get("count").and_then(|v| v.as_u64()) {
        state.find_match_count = count as usize;
    }
    if let Some(current) = value.get("current").and_then(|v| v.as_u64()) {
        state.find_current_index = current as usize;
    }
}

fn sync_nav_state(state: &mut FerriteBrowser) {
    let active = state.active_tab;
    if let Some(session) = state.servo_sessions.get(&active) {
        state.is_loading = matches!(session.load_status(), LoadStatus::Loading);
        state.can_go_back = session.can_go_back();
        state.can_go_forward = session.can_go_forward();
    } else {
        state.is_loading = false;
        state.can_go_back = false;
        state.can_go_forward = false;
    }
}

// ---------------------------------------------------------------------------
// Tabs — one set of helpers behind both the user's tab strip (`AddTab`/
// `SelectTab`/`CloseTab`) and the agent's `open_tab`/`switch_tab`/`close_tab`,
// so the two can never drift apart (the per-tab `Vec`s and `servo_sessions`
// are kept in step in exactly one place).
// ---------------------------------------------------------------------------

/// Puts the caret in the address bar with its text selected, and records that
/// it has focus (the focus ring and `PageKey` both read the flag). Order
/// matters: focusing moves the caret to the end, selecting all must follow.
fn focus_address_bar(state: &mut FerriteBrowser) -> Task<FerriteBrowserMessage> {
    state.address_bar_focused = true;
    text_input::focus(text_input::Id::new(ADDRESS_BAR_ID))
        .chain(text_input::select_all(text_input::Id::new(ADDRESS_BAR_ID)))
}

/// What the address bar shows for a tab at `url`: nothing for the blank page,
/// so the "Search or type an address" prompt shows and the first keystroke
/// starts a fresh address instead of extending `about:blank`.
fn address_bar_text(url: &str) -> String {
    if url == "about:blank" {
        String::new()
    } else {
        url.to_string()
    }
}

/// Pushes a fresh, blank tab onto every per-tab `Vec`, makes it the active one
/// and resets the navigation flags. Returns its index. Creates no session.
fn push_tab_state(state: &mut FerriteBrowser) -> usize {
    state.tabs.push("New Tab".to_string());
    state.tab_urls.push("about:blank".to_string());
    state.tab_error.push(None);
    state.tab_titles.push("New Tab".to_string());
    state.tab_favicons.push(None);
    state.favicon_keys.push(None);
    state.tab_zoom.push(state.default_zoom);
    state.tab_diag.push(tab_diag::TabDiag::default());
    let new_idx = state.tabs.len() - 1;
    state.active_tab = new_idx;
    state.address_bar_input = String::new();
    state.is_loading = false;
    state.can_go_back = false;
    state.can_go_forward = false;
    new_idx
}

/// Opens a new tab with its own Servo session — what the "+" button does.
/// Returns the tab's index and, if the session could not be created, why (the
/// tab still exists then, exactly as `AddTab` always behaved).
fn add_tab(state: &mut FerriteBrowser) -> (usize, Option<String>) {
    let index = push_tab_state(state);
    wake(state);
    match HeadlessServoSession::new(1280, 700) {
        Ok(session) => {
            state.servo_sessions.insert(index, session);
            sync_active_webview(state);
            (index, None)
        }
        Err(e) => (index, Some(e.to_string())),
    }
}

/// Makes Servo's own notion of "the tab being looked at" match
/// `state.active_tab`: the active WebView is shown and focused (keyboard input
/// goes to the focused WebView), every other one is blurred and hidden. Also
/// re-bases the resize tracking on the active session's real size — a new tab
/// starts at its creation size, not at the size the previous tab was last
/// resized to, and treating them as equal left every tab after the first
/// displayed at the wrong size with pointer input landing in the wrong place.
/// Keeps `frame_cache` current for the active tab (returns whether it changed):
/// a new entry only when the engine produced a new picture, and none kept for
/// tabs that are gone.
fn refresh_frame_cache(state: &mut FerriteBrowser) -> bool {
    state
        .frame_cache
        .retain(|index, _| state.servo_sessions.contains_key(index));
    let active = state.active_tab;
    let Some(frame) = state
        .servo_sessions
        .get(&active)
        .and_then(HeadlessServoSession::frame_shared)
    else {
        return false;
    };
    if state.frame_cache.get(&active).map(|(f, _)| f.seq) == Some(frame.seq) {
        return false;
    }
    // The image widget wants top-row-first RGBA in its own bytes: a copy per
    // picture, paid only on the fallback path.
    let handle = page_view::use_image_widget()
        .then(|| ImageHandle::from_rgba(frame.width, frame.height, frame.to_rgba_top_down()));
    state.frame_cache.insert(active, (frame, handle));
    true
}

fn sync_active_webview(state: &mut FerriteBrowser) {
    let active = state.active_tab;
    for (index, session) in &state.servo_sessions {
        session.set_active(*index == active);
    }
    if let Some(session) = state.servo_sessions.get(&active) {
        state.last_resized_content_px = session.size();
    }
}

/// Makes tab `i` the active one. `false` (and no change) if there is no such
/// tab.
fn select_tab_at(state: &mut FerriteBrowser, i: usize) -> bool {
    if i >= state.tabs.len() || i >= state.tab_urls.len() {
        return false;
    }
    if i != state.active_tab {
        // A control the page was waiting on is dismissed when the person
        // leaves it: it is not shown over a tab they are not looking at.
        controls::dismiss_tab(state, state.active_tab);
    }
    state.active_tab = i;
    state.address_bar_input = address_bar_text(&state.tab_urls[i]);
    state.address_bar_edited = false;
    // Scroll and pointer input belong to the page that was showing, and the
    // address bar no longer holds the keyboard (the callers unfocus it).
    state.address_bar_focused = false;
    state.scroll_queue.clear();
    state.pointer_moved = false;
    wake(state);
    sync_nav_state(state);
    sync_active_webview(state);
    true
}

/// Closes tab `i`, shifting every later tab's per-tab data and session down by
/// one and keeping the *same tab* active when an earlier one closes. The last
/// remaining tab is never closed. Returns whether a tab was removed.
fn close_tab_at(state: &mut FerriteBrowser, i: usize) -> bool {
    // Every later tab's index shifts by one — a stale hovered index would
    // otherwise show the close-on-hover button on the wrong tab until the
    // next real hover event.
    state.hovered_tab = None;
    let removed = state.tabs.len() > 1 && i < state.tabs.len();
    if removed {
        state.tabs.remove(i);
        state.tab_urls.remove(i);
        if i < state.tab_error.len() {
            state.tab_error.remove(i);
        }
        if i < state.tab_titles.len() {
            state.tab_titles.remove(i);
        }
        if i < state.tab_favicons.len() {
            state.tab_favicons.remove(i);
        }
        if i < state.favicon_keys.len() {
            state.favicon_keys.remove(i);
        }
        if i < state.tab_diag.len() {
            state.tab_diag.remove(i);
        }
        if i < state.tab_zoom.len() {
            state.tab_zoom.remove(i);
        }
        state.servo_sessions.remove(&i);
        let keys_to_shift: Vec<usize> = state
            .servo_sessions
            .keys()
            .copied()
            .filter(|&k| k > i)
            .collect();
        for k in keys_to_shift {
            if let Some(session) = state.servo_sessions.remove(&k) {
                state.servo_sessions.insert(k - 1, session);
            }
        }
        if i < state.active_tab {
            state.active_tab -= 1;
        }
    }
    state.active_tab = state.active_tab.min(state.tabs.len().saturating_sub(1));
    state.address_bar_input = address_bar_text(&state.tab_urls[state.active_tab]);
    state.address_bar_edited = false;
    state.scroll_queue.clear();
    state.pointer_moved = false;
    wake(state);
    sync_nav_state(state);
    sync_active_webview(state);
    removed
}

/// The active tab's page as the agent would see it, taken on the UI thread
/// through the borrowed engine's `observe_page` (the harness's own
/// observation: not an agent action, nothing to record). `None` — never an
/// error — when there is no session or the page cannot be read: callers fall
/// back to "header only".
fn observe_active_page(state: &mut FerriteBrowser) -> Option<PageDigest> {
    let session = state.servo_sessions.get_mut(&state.active_tab)?;
    let mut engine = BorrowedServoEngine::new(session, 1280, 700);
    engine.observe_page().ok().map(|(digest, _origin)| digest)
}

/// Appends a fresh compact page table for whichever tab is active *now* to
/// `observation` (unchanged when there is no session or nothing readable).
fn with_active_page_observation(state: &mut FerriteBrowser, observation: String) -> String {
    match state.servo_sessions.get_mut(&state.active_tab) {
        Some(session) => {
            let mut engine = BorrowedServoEngine::new(session, 1280, 700);
            with_page_observation(&mut engine, observation)
        }
        None => observation,
    }
}

/// Performs one of the agent's tab actions (`open_tab`, `switch_tab`,
/// `close_tab`, `list_tabs` — the borrowed engine cannot: it wraps a single
/// externally-owned tab) through the same helpers the tab strip uses, and
/// returns an honest observation for the model. Tab numbers are the 1-based
/// positions `list_tabs` shows. State-changing actions end with a fresh page
/// table for the tab that is active afterwards, so the next step's session is
/// looked up from `state.active_tab` as it is *now* (the step handler reads it
/// afresh every step).
///
/// The caller has already applied the consent check
/// (`is_action_rejected`), which sees these actions as `tab.open`/`tab.close`
/// (and, like the engine's own `Call::primitive`, `switch_tab` as
/// `navigate`).
fn run_tab_action(state: &mut FerriteBrowser, action: &AgentAction) -> String {
    let count = state.tabs.len();
    match action {
        AgentAction::ListTabs => {
            agent_run::format_tab_list(&state.tab_titles, &state.tab_urls, state.active_tab)
        }
        AgentAction::SwitchTab { tab } => match agent_run::resolve_tab_number(*tab, count) {
            Err(message) => message,
            Ok(index) => {
                select_tab_at(state, index);
                let observation = format!("switched to tab {tab}: {}", state.tab_urls[index]);
                with_active_page_observation(state, observation)
            }
        },
        AgentAction::CloseTab { tab } => {
            if count <= 1 {
                return "error: cannot close the last remaining tab".to_string();
            }
            match agent_run::resolve_tab_number(*tab, count) {
                Err(message) => message,
                Ok(index) => {
                    close_tab_at(state, index);
                    // Later tabs were renumbered: say how they are now.
                    let observation = format!(
                        "closed tab {tab}\n{}",
                        agent_run::format_tab_list(
                            &state.tab_titles,
                            &state.tab_urls,
                            state.active_tab
                        )
                    );
                    with_active_page_observation(state, observation)
                }
            }
        }
        AgentAction::OpenTab { url } => open_tab_for_agent(state, url.as_deref()),
        _ => "error: not a tab action".to_string(),
    }
}

/// `open_tab`: a new tab (a blank one, or navigated to `url` — an absolute
/// http(s) URL, see [`agent_run::validate_open_url`]) that becomes the active
/// one. If a session cannot be created the tab is rolled back and the model is
/// told, rather than leaving a dead tab behind.
fn open_tab_for_agent(state: &mut FerriteBrowser, url: Option<&str>) -> String {
    let target = match url.map(agent_run::validate_open_url).transpose() {
        Ok(target) => target,
        Err(message) => return format!("error: {message}"),
    };
    let previous = state.active_tab;
    let (index, session_error) = add_tab(state);
    if let Some(e) = session_error {
        close_tab_at(state, index);
        select_tab_at(state, previous.min(state.tabs.len().saturating_sub(1)));
        return format!("error: could not open a new tab: {e}");
    }
    let mut observation = format!(
        "opened tab {}: {}",
        index + 1,
        target.as_deref().unwrap_or("blank page")
    );
    if let Some(target) = target {
        // Blocks until the page has loaded, like the agent's own `navigate`.
        let navigated = state
            .servo_sessions
            .get_mut(&index)
            .map(|session| BorrowedServoEngine::new(session, 1280, 700).navigate(&target));
        match navigated {
            Some(Ok(_)) => {
                state.tab_urls[index] = target.clone();
                state.address_bar_input = target;
            }
            Some(Err(e)) => observation.push_str(&format!(" (navigation failed: {e})")),
            None => observation.push_str(" (navigation failed: no session)"),
        }
    }
    with_active_page_observation(state, observation)
}

// ---------------------------------------------------------------------------
// Consent panel — the security surface. Plain-English item summaries, the
// completeness check that gates ConsentSubmitted, and dry-run evidence
// rendering all live here as pure functions of a `FingerprintDiff` /
// `ExpectedFingerprint` / `DryRunRecord`, so they're testable without
// spinning up Iced at all (see the `tests` module at the bottom of this
// file).
// ---------------------------------------------------------------------------

/// Prefix distinguishing a synthetic `out_of_scope_origins` item id from a
/// real `ToolId` wire string (e.g. `"js.execute"`, `"dom.read"` never contain
/// `"::"`). `ConsentDecision` is keyed by `ToolId` alone (A7's type,
/// unmodified — this charter does not touch `ferrite-ipi`), so an
/// out-of-scope-origin item — which has no `ToolId` of its own, only an
/// origin string — borrows that same key space under this prefix rather than
/// requiring a second, parallel decision-tracking type.
const ORIGIN_ITEM_PREFIX: &str = "origin::";

/// The synthetic item id an out-of-scope-origin flagged item is tracked
/// under in `ConsentDecision::{approved,rejected}`.
fn origin_item_id(origin: &str) -> ToolId {
    ToolId::new(&format!("{ORIGIN_ITEM_PREFIX}{origin}"))
}

/// The reverse of [`origin_item_id`]: `Some(origin)` if `id` is a synthetic
/// origin item id, `None` if it's a real tool id.
fn origin_item_origin(id: &ToolId) -> Option<&str> {
    id.0.strip_prefix(ORIGIN_ITEM_PREFIX)
}

/// One flagged item ready for consent review: a stable identifier (used to
/// key the approve/reject decision, and to key the Iced widget row) and the
/// plain-English summary shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConsentItem {
    id: ToolId,
    summary: String,
}

/// Builds the plain-English per-item summaries for every flagged entry in
/// `diff` — both `extra_primitives` and `out_of_scope_origins` (the gap this
/// session closes: the old rendering only ever iterated `extra_primitives`).
///
/// Iteration order is deterministic (primitives sorted by tool id string,
/// then origins sorted by origin string) so this function's output — and
/// therefore the consent panel's rendering — is stable across runs, which is
/// what makes `consent_summary_snapshot_for_a_mixed_diff` a meaningful
/// regression test rather than a flaky one.
fn consent_items(
    diff: &FingerprintDiff,
    expected: Option<&ExpectedFingerprint>,
) -> Vec<ConsentItem> {
    let mut items = Vec::new();

    let mut extras: Vec<&ToolId> = diff.extra_primitives.iter().collect();
    extras.sort_by(|a, b| a.0.cmp(&b.0));
    for tool in extras {
        let summary = if tool.0 == "js.execute" {
            format!(
                "Used tool: {tool} — arbitrary JavaScript execution is always reviewed by \
                 design (it can synthesize any other action); nothing in your request could \
                 have authorized it."
            )
        } else {
            format!("Used tool: {tool} — nothing in your request authorized this action.")
        };
        items.push(ConsentItem {
            id: tool.clone(),
            summary,
        });
    }

    let mut origins: Vec<&String> = diff.out_of_scope_origins.iter().collect();
    origins.sort();
    for origin in origins {
        let allowed = expected
            .map(describe_authorized_origins)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "nothing in your request authorized any origin".to_string());
        let summary = format!(
            "Contacted {origin} — this origin is not authorized. Your request authorized: \
             {allowed}."
        );
        items.push(ConsentItem {
            id: origin_item_id(origin),
            summary,
        });
    }

    items
}

/// Plain-English description of every origin scope the expected fingerprint
/// carries, across all capabilities — the "which origin(s) would have been
/// fine" half of the directive's required summary.
///
/// This is coarser than per-primitive precision: `FingerprintDiff::out_of_scope_origins`
/// (A7's type, `ferrite_ipi::comparator::diff`) records only the offending
/// *origin* string, not which capability's realization the primitive at that
/// origin belonged to — so this function honestly reports every scope in the
/// expected set, not "the one scope that would have admitted this specific
/// primitive" (which the diff does not carry enough information to
/// determine). Documented as a known limitation in this session's
/// `docs/TO-DO.md` entry rather than papered over.
fn describe_authorized_origins(expected: &ExpectedFingerprint) -> String {
    let mut parts: Vec<String> = expected
        .lowered()
        .into_iter()
        .map(|(_, scope, _)| describe_scope(scope))
        .collect();
    parts.sort();
    parts.dedup();
    parts.join("; ")
}

fn describe_scope(scope: &ferrite_core::scope::OriginScope) -> String {
    use ferrite_core::scope::OriginScope;
    match scope {
        OriginScope::Exact(origins) => origins
            .iter()
            .map(ferrite_core::Origin::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        OriginScope::DomainSuffix(suffixes) => suffixes
            .iter()
            .map(|s| format!("*.{s}"))
            .collect::<Vec<_>>()
            .join(", "),
        OriginScope::TaskOpen { .. } => "any origin (open task)".to_string(),
    }
}

/// Whether every flagged item in `diff` — both buckets — has an explicit
/// approve or reject decision. This is the *only* gate `ConsentSubmitted`
/// may run behind; it replaces the pre-existing `ConsentDecision::is_complete`
/// call (which only ever checked `extra_primitives`) precisely because that
/// was the rendering/enforcement gap this session closes.
fn consent_is_complete(diff: &FingerprintDiff, decision: &ConsentDecision) -> bool {
    let decided = |id: &ToolId| decision.approved.contains(id) || decision.rejected.contains(id);
    diff.extra_primitives.iter().all(decided)
        && diff
            .out_of_scope_origins
            .iter()
            .all(|origin| decided(&origin_item_id(origin)))
}

/// Renders the dry-run evidence — what the agent actually did, in call
/// order — from a real `DryRunRecord`. Honestly scoped to what
/// `DryRunRecord` actually carries: the ordered (tool, origin) call log and
/// a sanitizer-finding count. It does **not** show page content, because
/// `DryRunRecord` does not record page content read — only which tool ran,
/// at which origin, in what order, and which sanitizer patterns fired.
fn dry_run_evidence_lines(record: &DryRunRecord) -> Vec<String> {
    let mut lines: Vec<String> = record
        .events_with_seq()
        .map(|(seq, event)| {
            format!(
                "#{seq}  {tool}  {origin}",
                tool = event.primitive.as_str(),
                origin = event.origin.as_deref().unwrap_or("(no origin recorded)")
            )
        })
        .collect();
    if !record.sanitizer_findings.is_empty() {
        lines.push(format!(
            "{} sanitizer finding(s) recorded during the dry run",
            record.sanitizer_findings.len()
        ));
    }
    if lines.is_empty() {
        lines.push("No tool calls were recorded during the dry run.".to_string());
    }
    lines
}

/// Ease-out-cubic easing curve, mapping linear progress `t` (`0.0..=1.0`,
/// clamped) to eased progress — accelerates out of the start rather than
/// moving at a constant rate, the standard curve for a short UI entrance
/// transition. Used by `view_agent_sidebar`'s consent panel to turn
/// `FerriteBrowser::consent_panel_anim`'s linear tick-driven progress into
/// the panel's actual background-alpha/slide-offset animation.
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

// ---------------------------------------------------------------------------
// C3c: loading-indicator animation
// ---------------------------------------------------------------------------
//
// Driven by the same `FerriteBrowser::progress_offset` `ServoFrame` already
// advances every ~16ms tick (see that handler) — consistent with
// `ease_out_cubic` above, no separate animation clock is introduced. The
// loading bar's sweep is `chrome::progress_band`. Kept as a pure `f32 -> f32`
// function rather than inlined into `view()`, the same reasoning
// `ease_out_cubic` documents: testable without spinning up Iced at all.

/// A smooth "something is happening" breathing alpha, `t` in `0.0..=1.0`
/// (`progress_offset`) with `cycles` full breaths per `t`'s wrap-around —
/// the one shared curve behind the loading placeholder's mark, a loading
/// tab's dot and the Agent button's working dot.
fn pulse_alpha(t: f32, cycles: f32, base: f32, amplitude: f32) -> f32 {
    base + amplitude * (t * std::f32::consts::TAU * cycles).sin().abs()
}

// Styling helpers live in `tokens.rs` (the spacing/type/radius scales, the
// elevation recipes and every shared button/container style).

// ---------------------------------------------------------------------------
// Audit helpers
// ---------------------------------------------------------------------------

/// `palette` supplies three of the five colours directly (`safe`/`warn`/
/// `danger`); "USED"/"EVAL" have no dedicated palette role (an informational
/// blue and a neutral gray, not one of the twelve semantic colours), so they
/// stay literal here — but still theme-aware via the separate `is_light`
/// flag (rather than inferring it by comparing `palette`'s address against
/// `&LIGHT_PALETTE`: both are `const`, not `static`, so the language gives
/// no guarantee two references to the same `const` item share an address —
/// an explicit `bool` the caller already has on hand is the safe way to ask
/// this), since the dark palette's light, saturated versions would fail the
/// light theme's contrast target the same way the pre-theme `C_SAFE`/
/// `C_WARN`/`C_DANGER` values did (see `LIGHT_PALETTE`'s doc comment).
fn kind_label(kind: &AuditEventKind, palette: &Palette, is_light: bool) -> (&'static str, Color) {
    match kind {
        AuditEventKind::CapabilityGranted => ("GRANTED", palette.safe),
        AuditEventKind::CapabilityDenied => ("DENIED", palette.danger),
        AuditEventKind::CapabilityExercised => (
            "USED",
            if is_light {
                Color::from_rgb(0.10, 0.40, 0.75)
            } else {
                Color::from_rgb(0.4, 0.7, 1.0)
            },
        ),
        AuditEventKind::ContentBlocked => ("BLOCKED", palette.warn),
        AuditEventKind::EvalExecutionRecorded => (
            "EVAL",
            if is_light {
                Color::from_rgb(0.42, 0.42, 0.46)
            } else {
                Color::from_rgb(0.6, 0.6, 0.6)
            },
        ),
    }
}

/// Shortens every long URL in a page's console message to its start and end,
/// so a message full of script addresses (Google's are hundreds of characters)
/// still shows the part that matters, the error itself, after `truncate`.
pub(crate) fn shorten_urls(message: &str) -> String {
    const KEEP_HEAD: usize = 56;
    const KEEP_TAIL: usize = 28;
    message
        .split(' ')
        .map(|word| {
            let is_url = word.starts_with("http://") || word.starts_with("https://");
            let len = word.chars().count();
            if is_url && len > KEEP_HEAD + KEEP_TAIL + 3 {
                let head: String = word.chars().take(KEEP_HEAD).collect();
                let tail: String = word.chars().skip(len - KEEP_TAIL).collect();
                format!("{head}…{tail}")
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod console_message_tests {
    use super::shorten_urls;

    #[test]
    fn a_long_script_url_keeps_its_ends_and_the_error_survives() {
        let url = format!(
            "https://www.google.com/xjs/_/js/k=xjs.hd.en_GB/am={}/rt=j:83:37",
            "A".repeat(400)
        );
        let message =
            format!("Error at {url} uncaught exception: SecurityError: The operation is insecure.");
        let short = shorten_urls(&message);
        assert!(short.chars().count() < 200, "{short}");
        assert!(short.starts_with("Error at https://www.google.com/xjs/"));
        assert!(short.contains("/rt=j:83:37 uncaught exception: SecurityError"));
    }

    #[test]
    fn short_urls_and_plain_words_are_left_alone() {
        let message = "Error at https://example.com/a.js:1:2 TypeError: x is undefined";
        assert_eq!(shorten_urls(message), message);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            &s[..s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len())]
        )
    }
}

// ---------------------------------------------------------------------------
// URL resolution
// ---------------------------------------------------------------------------

/// Smart URL resolver — only adds a scheme when one is absent, and picks
/// https vs search based on whether the input looks like a hostname.
fn resolve_url(input: &str) -> String {
    let trimmed = input.trim();

    // Already has a scheme → pass through unchanged.
    if trimmed.contains("://") {
        return trimmed.to_string();
    }

    // Special pages.
    if trimmed == "about:blank" || trimmed.starts_with("about:") {
        return trimmed.to_string();
    }

    // Looks like a hostname (no spaces, has a dot, no special chars that
    // would be illegal in a hostname).  Prepend https://.
    let no_spaces = !trimmed.contains(' ');
    let has_dot = trimmed.contains('.');
    let path_like = trimmed.starts_with('/');
    if no_spaces && (has_dot || path_like) {
        // A bare loopback or IP-literal address is a local server, which
        // almost never speaks TLS.
        let host = trimmed.split(['/', ':']).next().unwrap_or_default();
        let local =
            host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok();
        let scheme = if local { "http" } else { "https" };
        return format!("{scheme}://{trimmed}");
    }
    if no_spaces && trimmed.to_ascii_lowercase().starts_with("localhost") {
        return format!("http://{trimmed}");
    }

    // Everything else → a Google search.
    let encoded = urlencoding::encode(trimmed);
    format!("https://www.google.com/search?q={encoded}")
}

// ---------------------------------------------------------------------------
// C3d: bookmarks — JSON persistence
// ---------------------------------------------------------------------------
//
// `load_bookmarks_from`/`save_bookmarks_to` take an explicit `&Path` (unit-
// tested directly against a temp-directory path — real filesystem I/O, but
// never the network, so R7 is unaffected) rather than resolving the real
// home-directory path internally; `default_bookmarks_path()` is the
// separate, untested-directly wrapper that does that resolution, matching
// `ferrite-model::config::default_cache_dir()`'s own dependency-injection
// shape exactly (see that function for the template this follows).

/// The real bookmarks-file path — `~/.local/share/ferrite/bookmarks.json`,
/// XDG-style. Bookmarks are a real, non-disposable loss to the user (unlike
/// `ferrite-model`'s response cache), so this lives under a data directory,
/// not `~/.cache/...` — deliberately distinct from
/// `ferrite_model::config::default_cache_dir()`'s `~/.cache/ferrite-model`.
/// `None` if the home directory cannot be resolved at all; `launch()` treats
/// that the same way it treats no model provider being configured — the
/// feature degrades (bookmarks stay in-memory-only for the session) rather
/// than panicking.
fn default_bookmarks_path() -> Option<PathBuf> {
    Some(
        ferrite_agent::chat::ferrite_data_dir(
            std::env::var("FERRITE_HOME").ok().as_deref(),
            dirs::home_dir(),
        )?
        .join("bookmarks.json"),
    )
}

/// Loads the bookmark list from `path` — `Vec::new()` (not an error) if the
/// file doesn't exist yet (the common case: first ever run) or fails to
/// parse (a hand-edited or corrupted file should not crash the browser on
/// startup; the user simply starts with an empty list, same fail-open-to-
/// empty shape `CLAUDE.md`'s fingerprint invariant uses for a different
/// reason).
fn load_bookmarks_from(path: &Path) -> Vec<Bookmark> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persists `bookmarks` to `path` as pretty-printed JSON, creating the
/// containing directory if needed.
fn save_bookmarks_to(path: &Path, bookmarks: &[Bookmark]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(bookmarks).unwrap_or_else(|_| "[]".to_string());
    std::fs::write(path, json)
}

// ---------------------------------------------------------------------------
// C3d: history — recording
// ---------------------------------------------------------------------------

/// Appends one visit to `history`, deduplicated against the immediately
/// preceding entry only (not the whole list — visiting the same page again
/// after browsing elsewhere is a real, distinct visit worth recording again,
/// same as any real browser's history) so a reload or an in-page redirect
/// loop doesn't spam the list with consecutive repeats of the same URL.
fn record_history_visit(history: &mut Vec<HistoryEntry>, url: String, title: String) {
    if history.last().is_some_and(|last| last.url == url) {
        return;
    }
    history.push(HistoryEntry {
        url,
        title,
        visited_at: chrono::Utc::now(),
    });
}

// ---------------------------------------------------------------------------
// C3d: zoom — discrete levels + the CSS-transform injection script
// ---------------------------------------------------------------------------

/// Standard browser zoom steps, 50%–300% — the same step set Chrome/Firefox
/// expose in their own zoom menus, so `ZoomIn`/`ZoomOut` feel like a familiar
/// browser rather than an arbitrary continuous slider.
const ZOOM_LEVELS: &[f32] = &[
    0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// The next level up from `current`, or the top of the range if already
/// there (or above it, e.g. after a `SetDefaultZoom` value that doesn't
/// land exactly on a step).
fn next_zoom_level(current: f32) -> f32 {
    ZOOM_LEVELS
        .iter()
        .copied()
        .find(|&z| z > current + f32::EPSILON)
        .unwrap_or(*ZOOM_LEVELS.last().unwrap())
}

/// The next level down from `current`, or the bottom of the range if
/// already there (or below it).
fn prev_zoom_level(current: f32) -> f32 {
    ZOOM_LEVELS
        .iter()
        .rev()
        .copied()
        .find(|&z| z < current - f32::EPSILON)
        .unwrap_or(ZOOM_LEVELS[0])
}

// ---------------------------------------------------------------------------
// C3d: find-in-page — standards-based DOM search script
// ---------------------------------------------------------------------------
//
// Both scripts below are standards-based (`document.createTreeWalker`,
// `Range`, `NodeFilter`) rather than the legacy, nonstandard
// `window.find()`, which is not certain to exist in Servo's JS/DOM
// implementation — per this charter's own research brief. Each script
// returns `JSON.stringify(...)` as its final expression; `execute_js`
// returns whatever `HeadlessServoSession`'s underlying `evaluate_javascript`
// callback hands back, `Debug`-formatted from Servo's own JS-value type —
// this crate has not verified that type's exact shape against the pinned
// `libservo` source the way the favicon/history work was, so
// `extract_json_object` (below) parses defensively rather than assuming the
// return value is exactly the raw JSON string this script produces.

/// Clears every highlight `find_script`/`find_navigate_script` may have
/// left in the page — run on `CloseFindBar` and on an empty `FindQueryChanged`.
const FIND_CLEAR_SCRIPT: &str = "(function(){\
    document.querySelectorAll('mark[data-ferrite-find]').forEach(function(m){\
        var p = m.parentNode;\
        if (!p) return;\
        p.replaceChild(document.createTextNode(m.textContent), m);\
        p.normalize();\
    });\
    return JSON.stringify({count:0,current:0});\
})()";

/// Highlights every case-insensitive occurrence of `query` in the page's
/// visible text (skipping `<script>`/`<style>` text nodes) by wrapping each
/// match in a `<mark data-ferrite-find>`, marks the first one current, and
/// returns `{"count": N, "current": 1}` (or `{"count": 0, "current": 0}` for
/// no matches). Re-running this (e.g. on every `FindQueryChanged` keystroke)
/// first clears any previous highlights via the same logic
/// `FIND_CLEAR_SCRIPT` uses, so stale highlights from an earlier query never
/// linger alongside a new query's.
///
/// `query` is embedded via `serde_json::to_string`, which produces a
/// properly quoted-and-escaped JS string literal — not string-concatenated
/// raw, which would let a query containing a quote character break out of
/// the literal.
fn find_script(query: &str) -> String {
    let query_literal = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "(function(){{\
           document.querySelectorAll('mark[data-ferrite-find]').forEach(function(m){{\
               var p = m.parentNode; if (!p) return;\
               p.replaceChild(document.createTextNode(m.textContent), m);\
               p.normalize();\
           }});\
           var q = {query_literal};\
           if (!q) {{ return JSON.stringify({{count:0,current:0}}); }}\
           var lowerQ = q.toLowerCase();\
           var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, null);\
           var ranges = [];\
           var node;\
           while ((node = walker.nextNode())) {{\
               var parentTag = node.parentElement ? node.parentElement.tagName : '';\
               if (parentTag === 'SCRIPT' || parentTag === 'STYLE') continue;\
               var text = node.textContent;\
               var lower = text.toLowerCase();\
               var idx = 0;\
               while ((idx = lower.indexOf(lowerQ, idx)) !== -1) {{\
                   var r = document.createRange();\
                   r.setStart(node, idx);\
                   r.setEnd(node, idx + q.length);\
                   ranges.push(r);\
                   idx += q.length;\
               }}\
           }}\
           ranges.reverse().forEach(function(r){{\
               var mark = document.createElement('mark');\
               mark.setAttribute('data-ferrite-find', '1');\
               mark.style.backgroundColor = '#ffd54f';\
               mark.style.color = '#000000';\
               try {{ r.surroundContents(mark); }} catch (e) {{}}\
           }});\
           var marks = document.querySelectorAll('mark[data-ferrite-find]');\
           if (marks.length > 0) {{\
               marks[0].setAttribute('data-ferrite-find-current', '1');\
               marks[0].style.backgroundColor = '#ff9800';\
               marks[0].scrollIntoView({{block: 'center'}});\
           }}\
           return JSON.stringify({{count: marks.length, current: marks.length > 0 ? 1 : 0}});\
         }})()",
        query_literal = query_literal,
    )
}

/// Moves the "current" highlight among the marks `find_script` already left
/// in the page, wrapping past either end, scrolls the new current match into
/// view, and returns `{"count": N, "current": newIndex}` — does not
/// re-search the page (matches `find_script` already found stay found).
fn find_navigate_script(forward: bool) -> String {
    let step = if forward { 1 } else { -1 };
    format!(
        "(function(){{\
           var marks = document.querySelectorAll('mark[data-ferrite-find]');\
           if (marks.length === 0) {{ return JSON.stringify({{count:0,current:0}}); }}\
           var curIdx = 0;\
           for (var i = 0; i < marks.length; i++) {{\
               if (marks[i].hasAttribute('data-ferrite-find-current')) {{ curIdx = i; break; }}\
           }}\
           marks[curIdx].removeAttribute('data-ferrite-find-current');\
           marks[curIdx].style.backgroundColor = '#ffd54f';\
           var nextIdx = (curIdx + ({step}) + marks.length) % marks.length;\
           marks[nextIdx].setAttribute('data-ferrite-find-current', '1');\
           marks[nextIdx].style.backgroundColor = '#ff9800';\
           marks[nextIdx].scrollIntoView({{block: 'center'}});\
           return JSON.stringify({{count: marks.length, current: nextIdx + 1}});\
         }})()",
        step = step,
    )
}

/// Defensive extraction of a JSON object from whatever
/// `HeadlessServoSession::execute_js` returned for a `find_script`/
/// `find_navigate_script` call. Tries a direct parse first (the case if
/// Servo's `evaluate_javascript` callback hands back the raw JS string
/// value unwrapped); if that fails, falls back to locating the first `{`
/// and last `}` in the raw text and parsing that substring (tolerant of an
/// unknown `Debug`-formatted wrapper, e.g. an enum variant like
/// `String("{...}")`, around the JSON this crate's own scripts always
/// produce as their JS return value) — see this module's find-in-page
/// section header for why this crate cannot assume the exact shape without
/// having verified Servo's JS-value type directly. Returns `None` if
/// neither attempt parses, rather than guessing.
fn extract_json_object(raw: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        return Some(value);
    }
    // Try a Rust-`Debug`-quoted string wrapper next, e.g. `String("{\"count\":1}")`
    // — the content between the first and last `"`, with `\"`/`\\` unescaped,
    // parsed as JSON. This has to come before the brace-finding fallback
    // below: a `Debug`-escaped `\"` is not valid raw JSON syntax on its own,
    // so naively slicing between the first `{` and last `}` of the original
    // string (without unescaping first) would hand `serde_json` a string it
    // can never parse.
    if let (Some(start), Some(end)) = (raw.find('"'), raw.rfind('"')) {
        if end > start {
            let unescaped = raw[start + 1..end]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\");
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&unescaped) {
                return Some(value);
            }
        }
    }
    // Fall back to the first `{`/last `}` directly — handles a wrapper with
    // no quoting at all (e.g. a bare `Display` impl around the raw JSON).
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

// ---------------------------------------------------------------------------
// C3d: downloads — path resolution + the background streaming task
// ---------------------------------------------------------------------------
//
// Deliberately independent of `ferrite_engine::BrowserEngine::download()`
// (the agent's dry-run/consent-gated tool vocabulary) — see this module's
// own doc comment for the ADR/consent-boundary reasoning. This is a plain
// user-facing "download this link" feature, entirely local to `ferrite-ui`.

/// The real downloads directory — `dirs::download_dir()`, falling back to
/// the home directory if the platform has no dedicated one (matches this
/// crate's own already-established "fail to a documented fallback, not a
/// panic" shape for a resolvable-but-imperfect path, the same spirit as
/// `default_bookmarks_path`, though that one has no fallback of its own
/// since a bookmarks *file* needs a specific parent, not just any writable
/// directory).
fn default_downloads_dir() -> Option<PathBuf> {
    dirs::download_dir().or_else(dirs::home_dir)
}

/// The filename component of `url`'s path — `"download"` if `url` doesn't
/// parse, has no path segments, or its last segment is empty (e.g. a URL
/// ending in `/`).
fn download_file_name(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(str::to_string))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "download".to_string())
}

/// Where a new download of `url` into `dir` should be written — pure and
/// fully testable without any real filesystem I/O: collision-avoidance is
/// checked only against `existing`'s already-known destination paths (the
/// in-memory download list this session itself created), the same
/// dependency-injection shape `load_bookmarks_from`/`save_bookmarks_to` use
/// for a different reason (there, testability without touching `$HOME`;
/// here, testability without touching the real filesystem at all). A real
/// browser would also check for a same-named file already on disk from
/// outside this session, which this does not — an honest, documented scope
/// cut, not an oversight: the session's own download list is the only
/// collision source this crate can check without live filesystem I/O in an
/// automated test (R7 is about network, not local disk, but this function's
/// purity is worth keeping regardless).
fn resolve_download_path(dir: &Path, url: &str, existing: &[DownloadItem]) -> PathBuf {
    let base_name = download_file_name(url);
    let taken: std::collections::HashSet<&Path> =
        existing.iter().map(|d| d.path.as_path()).collect();
    let candidate = dir.join(&base_name);
    if !taken.contains(candidate.as_path()) {
        return candidate;
    }
    let (stem, ext) = match base_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (base_name.clone(), String::new()),
    };
    let mut n = 1u32;
    loop {
        let name = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let candidate = dir.join(name);
        if !taken.contains(candidate.as_path()) {
            return candidate;
        }
        n += 1;
    }
}

/// Human-readable byte count for the Downloads tab's progress line — B/KB/
/// MB/GB, one decimal place above the smallest unit, matching the
/// precision real browsers' own download managers show.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{bytes} {}", UNITS[unit_index])
    } else {
        format!("{value:.1} {}", UNITS[unit_index])
    }
}

/// The background task `DownloadCurrentPage` spawns (`tokio::task::spawn`,
/// the same "spawn the I/O, report back over `agent_event_tx`" shape
/// `spawn_next_step` already established for the agent loop's own
/// background model calls) — a real `reqwest::Client::get(url)`, streamed
/// chunk by chunk to `dest` via `tokio::fs::File`, reporting a
/// `DownloadProgress` message after every chunk. `iced::futures::StreamExt`
/// (the real `futures` crate iced re-exports — this file already imports it
/// elsewhere, see `subscription()`'s `agent_event_sub`) drives
/// `Response::bytes_stream()` without needing a second, independent async
/// stream dependency.
async fn run_download(
    id: u64,
    url: String,
    dest: PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<FerriteBrowserMessage>,
) {
    use iced::futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::new();
    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(FerriteBrowserMessage::DownloadFailed {
                id,
                error: e.to_string(),
            });
            return;
        }
    };
    if let Err(e) = response.error_for_status_ref() {
        let _ = tx.send(FerriteBrowserMessage::DownloadFailed {
            id,
            error: e.to_string(),
        });
        return;
    }
    let total_bytes = response.content_length();
    let mut file = match tokio::fs::File::create(&dest).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(FerriteBrowserMessage::DownloadFailed {
                id,
                error: format!("creating {}: {}", dest.display(), e),
            });
            return;
        }
    };

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(FerriteBrowserMessage::DownloadFailed {
                    id,
                    error: e.to_string(),
                });
                return;
            }
        };
        if let Err(e) = file.write_all(&chunk).await {
            let _ = tx.send(FerriteBrowserMessage::DownloadFailed {
                id,
                error: e.to_string(),
            });
            return;
        }
        downloaded += chunk.len() as u64;
        let _ = tx.send(FerriteBrowserMessage::DownloadProgress {
            id,
            downloaded_bytes: downloaded,
            total_bytes,
        });
    }
    let _ = tx.send(FerriteBrowserMessage::DownloadCompleted { id });
}

// ---------------------------------------------------------------------------
// New-tab hero: quick-access tile favicons — fetch + on-disk cache + decode
// ---------------------------------------------------------------------------
//
// See this file's own module docs ("New-tab hero: real quick-access-tile
// favicons") for the full design story. Every pure step below
// (`favicon_host`, `favicon_cache_dir`/`favicon_cache_path`,
// `decode_favicon_rgba`) is unit-tested directly with no filesystem or
// network I/O, the same dependency-injection discipline `resolve_download_
// path`/`load_bookmarks_from` already established in this file (R7: no live
// network call may ever be reachable from `#[test]`). Only `fetch_favicon_
// bytes`/`fetch_tile_favicon` touch the network or the real filesystem, and
// neither is ever called from a synchronous, directly-`#[test]`-reachable
// path — only from inside a `tokio::task::spawn`ed future the same way
// `run_download` above is, which this crate's own test-module header
// documents as never actually polled into making a live call by a
// `#[tokio::test]` that doesn't `.await` past the spawn point.

/// The host `url` would navigate to — `None` if `url` doesn't parse as an
/// absolute URL with a host. The single source of truth `fetch_tile_favicon`
/// derives both the cache-file name and the `https://<host>/favicon.ico`
/// fetch URL from, so the two can never name a different host than the tile
/// itself actually navigates to.
fn favicon_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// The real favicon-cache directory — `~/.cache/ferrite-ui/favicons`,
/// matching `ferrite_model::config::default_cache_dir()`'s own `.cache`
/// placement for the same reason: a favicon is re-fetchable, unlike a
/// bookmark (`default_bookmarks_path`, which deliberately lives under
/// `.local/share` instead). `None` if the home directory cannot be resolved
/// at all — `launch()` then simply never sends `FetchTileFavicons`'
/// fetches anywhere to cache, and every tile falls back to its monogram for
/// the session, the same "degrade, don't panic" shape every other real-path
/// resolver in this file already follows.
fn favicon_cache_dir() -> Option<PathBuf> {
    // Inside `$FERRITE_HOME` when the local setup set one (everything stays
    // in the project folder); otherwise the conventional per-user cache.
    match std::env::var("FERRITE_HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
    {
        Some(home) => Some(PathBuf::from(home).join("cache").join("favicons")),
        None => Some(
            dirs::home_dir()?
                .join(".cache")
                .join("ferrite-ui")
                .join("favicons"),
        ),
    }
}

/// Where `host`'s favicon is cached under `cache_dir` — pure and fully
/// testable without touching the real filesystem, the same DI shape
/// `resolve_download_path` uses `dir: &Path` for. `.ico` regardless of what
/// the fetched bytes actually decode as (`decode_favicon_rgba` sniffs the
/// real format from content, not this extension) — named for the one fixed
/// path this module ever fetches from, `/favicon.ico`.
fn favicon_cache_path(cache_dir: &Path, host: &str) -> PathBuf {
    cache_dir.join(format!("{host}.ico"))
}

/// Decodes arbitrary fetched/cached favicon bytes into `(width, height,
/// rgba8_pixels)` — `None` for anything that isn't a real, decodable image
/// (an HTML error page served with a 200 status at `/favicon.ico`, a
/// truncated download, a cache file from a format this build doesn't decode).
/// `image::load_from_memory` sniffs the real format from the data's own
/// magic bytes, so this handles both a genuine multi-frame `.ico` container
/// and a site that serves a plain PNG at that same path (both real,
/// observed-in-the-wild cases) through the one call. `DynamicImage::
/// to_rgba8()` is exactly the pixel format `iced_widget::image::Handle::
/// from_rgba` needs — no separate conversion step, unlike `ferrite-servo`'s
/// `favicon_to_rgba8`, which has to handle Servo's five internal
/// `PixelFormat` variants because it reads an already-decoded in-process
/// buffer rather than re-decoding compressed bytes itself.
fn decode_favicon_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((width, height, rgba.into_raw()))
}

/// Fetches `url` (a `/favicon.ico` URL) as raw bytes — `None` for any
/// network error or non-success HTTP status, never a panic. Kept as its own
/// tiny function, separate from `fetch_tile_favicon`'s cache/decode
/// bookkeeping, purely so the one real network call in this module has a
/// single, obvious call site.
async fn fetch_favicon_bytes(url: &str) -> Option<Vec<u8>> {
    let response = favicon_client().get(url).send().await.ok()?;
    let response = response.error_for_status().ok()?;
    response.bytes().await.ok().map(|b| b.to_vec())
}

/// The HTTP client for favicon fetches: a short timeout, and a User-Agent —
/// several sites (Wikipedia among them) refuse requests that carry none.
fn favicon_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; Ferrite favicon fetch)")
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// The icon URLs a page's HTML declares (`<link rel="icon" href=...>` and the
/// like), resolved against `base`, best candidates first: plain icons before
/// `apple-touch-icon`, and `.svg` ones last (the decoder cannot read SVG).
/// A deliberately small scanner, not an HTML parser: it only has to find
/// `<link` tags in the first bytes of a home page.
fn icon_links_in_html(html: &str, base: &url::Url) -> Vec<url::Url> {
    let lower = html.to_ascii_lowercase();
    let mut found: Vec<(u8, url::Url)> = Vec::new();
    let mut from = 0;
    while let Some(start) = lower[from..].find("<link") {
        let begin = from + start;
        let end = lower[begin..].find('>').map_or(lower.len(), |e| begin + e);
        let tag = &html[begin..end];
        from = end.max(begin + 5);
        let attr = |name: &str| -> Option<String> {
            let tag_lower = tag.to_ascii_lowercase();
            let key = format!("{name}=");
            let at = tag_lower.find(&key)?;
            let rest = &tag[at + key.len()..];
            let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'');
            match quote {
                Some(q) => rest[1..].split(q).next().map(str::to_string),
                None => rest
                    .split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .map(str::to_string),
            }
        };
        let (Some(rel), Some(href)) = (attr("rel"), attr("href")) else {
            continue;
        };
        let rel = rel.to_ascii_lowercase();
        if !rel
            .split_whitespace()
            .any(|r| r == "icon" || r == "apple-touch-icon")
        {
            continue;
        }
        let Ok(resolved) = base.join(href.trim()) else {
            continue;
        };
        let rank = if resolved.path().ends_with(".svg") {
            2
        } else if rel.contains("apple-touch-icon") {
            1
        } else {
            0
        };
        if !found.iter().any(|(_, u)| *u == resolved) {
            found.push((rank, resolved));
        }
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, u)| u).collect()
}

/// Fetches and decodes a site's icon: `/favicon.ico` first, then whatever its
/// home page declares. Returns the raw bytes that decoded (for the cache) and
/// the pixels.
async fn fetch_decodable_favicon(favicon_url: &str) -> Option<(Vec<u8>, (u32, u32, Vec<u8>))> {
    if let Some(bytes) = fetch_favicon_bytes(favicon_url).await {
        if let Some(pixels) = decode_favicon_rgba(&bytes) {
            return Some((bytes, pixels));
        }
    }
    let mut home = url::Url::parse(favicon_url).ok()?;
    home.set_path("/");
    home.set_query(None);
    let response = favicon_client().get(home.clone()).send().await.ok()?;
    let html = response.text().await.ok()?;
    let html: String = html.chars().take(131_072).collect();
    for link in icon_links_in_html(&html, &home).into_iter().take(4) {
        if let Some(bytes) = fetch_favicon_bytes(link.as_str()).await {
            if let Some(pixels) = decode_favicon_rgba(&bytes) {
                return Some((bytes, pixels));
            }
        }
    }
    None
}

/// The background task `FetchTileFavicons` spawns once per tile
/// (`tokio::task::spawn`, the same shape `run_download` already
/// establishes): a cache read first, a real `reqwest` GET of the site's own
/// `/favicon.ico` on a cache miss, then `decode_favicon_rgba` either way —
/// on success, reports `TileFaviconReady` with the raw RGBA8 pixels (the
/// `ImageHandle` itself is only ever constructed inside `update()`, the same
/// "raw bytes over the channel" shape the tab-bar favicon sync already
/// uses). Any failure along the way (no cache hit, network error, non-
/// success status, undecodable bytes) simply returns without sending
/// anything — `tile_favicons[index]` stays `None`, and `new_tab_page`
/// already renders a deliberately-designed monogram fallback for exactly
/// that state, so no separate "failed" message/variant is needed the way
/// `DownloadFailed` is for a user-initiated download the UI must visibly
/// report on.
///
/// A cache write only ever happens after a freshly-fetched response has
/// already decoded successfully — never before — so a transient bad
/// response (a truncated body, an HTML error page served with a 200 status)
/// is never cached as if it were a real icon; the next launch simply
/// retries the fetch instead of permanently failing from a poisoned cache
/// entry. The converse (a cache file that fails to decode) is not retried
/// within the same run — an honest, documented scope cut: this can only
/// happen from a cache file this exact code path never writes (a hand-
/// edited or foreign file at that path), not from anything this function
/// itself produces.
async fn fetch_tile_favicon(
    index: usize,
    favicon_url: String,
    cache_path: PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<FerriteBrowserMessage>,
) {
    let cached = match tokio::fs::read(&cache_path).await {
        Ok(cached) => decode_favicon_rgba(&cached).map(|pixels| (cached, pixels)),
        Err(_) => None,
    };
    let from_cache = cached.is_some();
    let Some((bytes, (width, height, rgba))) = (match cached {
        Some(hit) => Some(hit),
        None => fetch_decodable_favicon(&favicon_url).await,
    }) else {
        return;
    };

    if !from_cache {
        if let Some(parent) = cache_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&cache_path, &bytes).await;
    }

    let _ = tx.send(FerriteBrowserMessage::TileFaviconReady {
        index,
        width,
        height,
        rgba,
    });
}

// ---------------------------------------------------------------------------
// Live agent loop — message-driven step loop shared by the initial
// (bypassed/clean-dry-run) run and the post-consent real run.
//
// See this file's module docs for why this is a step-by-step message loop
// rather than one call to `browser_loop::run_agent_loop`: the live engine
// (`BorrowedServoEngine`, wrapping a real, `!Send` `HeadlessServoSession`)
// cannot cross the `tokio::spawn` boundary a single long-running background
// task would need. Only the model round trip (`&dyn ModelProvider`, `Send +
// Sync`) is ever spawned; every actual browser action executes synchronously
// on the Iced update thread, via the exact same `execute_action` dispatch
// `run_agent_loop` itself uses.
// ---------------------------------------------------------------------------

/// Initializes `state.live_loop` from `prompt` (a fresh message history) —
/// used both for a bypassed/clean-dry-run task (empty `rejected`/
/// `rejected_origins`) and for the post-consent real run (the user's actual
/// decisions) — then spawns the first step's background model call.
fn start_live_loop(
    state: &mut FerriteBrowser,
    run_id: u64,
    prompt: String,
    rejected: std::collections::HashSet<ToolId>,
    rejected_origins: std::collections::HashSet<String>,
    guard: Option<ferrite_ipi::comparator::RuntimeGuard>,
) -> Task<FerriteBrowserMessage> {
    state.agent_is_running = true;
    // The run's first message is the seed built at submit time (chat history,
    // tabs, page, then the request); the plain prompt only if there is none.
    // Both the direct start and the post-consent start come through here, so
    // both use the same seed.
    let goal = trusted_task_text(Some(&state.chat), &prompt);
    let first_message = state.pending_seed.take().unwrap_or(prompt);
    let live =
        LiveAgentLoop::new(first_message, goal, rejected, rejected_origins).with_guard(guard);
    spawn_next_step(state, run_id, live)
}

/// Checks the step/wall-clock budget (mirroring
/// `browser_loop::run_agent_loop`'s own pre-request checks exactly), then
/// spawns the background model call for the next step. `live` is stored
/// back onto `state.live_loop` for the resulting `AgentStepReady` to pick
/// up; a budget stop instead ends the run — through `conclude_run`, like every
/// other ending — with a benign, visible outcome, not an error, since a budget
/// cutoff is a designed safety limit, not a failure.
fn spawn_next_step(
    state: &mut FerriteBrowser,
    run_id: u64,
    mut live: LiveAgentLoop,
) -> Task<FerriteBrowserMessage> {
    if live.actions_taken.len() >= live.budget.max_steps {
        return conclude_run(state, Outcome::Stopped("step budget exhausted".to_string()));
    }
    if live.started_at.elapsed() >= live.budget.max_wall_clock {
        return conclude_run(
            state,
            Outcome::Stopped("wall-clock budget exhausted".to_string()),
        );
    }

    let provider = state.model_provider.clone();
    let model_tag_main = state.model_tag_main.clone();
    let messages = live.messages.clone();
    let event_tx = match state.agent_event_tx.clone() {
        Some(tx) => tx,
        None => {
            return conclude_run(
                state,
                Outcome::Failed("the agent's event channel is unavailable".to_string()),
            );
        }
    };

    // The page as it is now, read once on this thread (after the previous
    // action ran): the sign-in handoff below and the fast lane both use it.
    let page_digest = observe_active_page(state);

    // A page that needs the person (a password field, a known sign-in host)
    // stops the run here, before any model call, and asks. Nothing executes
    // while it waits; *Continue* resumes from the page as it then is.
    let active_url = state
        .tab_urls
        .get(state.active_tab)
        .cloned()
        .unwrap_or_default();
    if let signin::Gate::Pause(wall) = signin::gate(
        &mut live.signin_cleared_host,
        &active_url,
        page_digest.as_ref(),
    ) {
        activity::record(
            "sign-in handoff",
            &wall.host,
            "waiting for the user",
            true,
            "",
            0,
        );
        state.signin_handoff = Some(wall);
        state.live_loop = Some(live);
        return scroll_to_latest(state);
    }

    // The optional Laya fast lane: only when it is configured AND the active
    // page can be read right now. Anything less and this step is exactly the
    // normal LLM step.
    let laya_timing = state.laya.clone();
    let fast_inputs = state.laya.clone().and_then(|decider| {
        let digest = page_digest.clone()?;
        let signature = agent_run::page_signature(&digest);
        if let (Some(last), Some(previous)) = (live.history.last_mut(), live.last_page_sig) {
            if last.page_changed.is_none() {
                last.page_changed = Some(previous != signature);
            }
        }
        live.last_page_sig = Some(signature);
        Some(agent_run::FastLaneInputs {
            decider,
            digest,
            goal: live.goal.clone(),
            history: live.history.clone(),
            previous: live.previous_fast.clone(),
            provider: state.model_provider.clone(),
            small_model_tag: state.model_tag_small.clone(),
        })
    });

    let handle = tokio::task::spawn(async move {
        if let Some(inputs) = fast_inputs {
            if let Some((fast, history)) = agent_run::try_fast_lane(&inputs).await {
                let action = fast.to_agent_action();
                let _ = event_tx.send(FerriteBrowserMessage::FastStepReady {
                    run_id,
                    action,
                    fast,
                    history,
                });
                return;
            }
        }
        let request = CompletionRequest::new(model_tag_main, ModelTier::Main, messages)
            .with_label("agent step")
            .with_system_prompt(SYSTEM_PROMPT, SYSTEM_PROMPT_VERSION)
            .with_options(SamplingOptions::default().with_num_predict(AGENT_LOOP_NUM_PREDICT));
        let llm_started = std::time::Instant::now();
        let completion = provider.complete(request).await;
        if let Some(decider) = &laya_timing {
            // What the fast lane competes with; it decides whether asking
            // Laya first is worth it (`LaneGovernor`).
            decider.note_llm_step(
                u64::try_from(llm_started.elapsed().as_millis()).unwrap_or(u64::MAX),
            );
        }
        let action =
            match completion {
                Ok(response) => serde_json::from_str::<AgentAction>(response.content.trim())
                    .map_err(|e| StepFailure::Malformed {
                        raw: response.content,
                        message: e.to_string(),
                    }),
                Err(e) => Err(StepFailure::Model(e.to_string())),
            };
        let _ = event_tx.send(FerriteBrowserMessage::AgentStepReady { run_id, action });
    });

    state.agent_handle = Some(handle);
    state.live_loop = Some(live);
    Task::none()
}

// ---------------------------------------------------------------------------
// Library drawer (bookmarks / history / downloads)
// ---------------------------------------------------------------------------

/// A page row in the library: a ghost button (hover wash) holding the title
/// and the address, navigating on press.
fn library_page_row<'a>(
    palette: &'static Palette,
    title: String,
    subtitle: String,
    trailing: Option<String>,
    url: String,
) -> Element<'a, FerriteBrowserMessage> {
    let mut cells: Vec<Element<FerriteBrowserMessage>> = vec![column![
        text(title).size(TEXT_BODY).color(palette.text),
        text(subtitle).size(TEXT_CAPTION).color(palette.text_dim),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()];
    if let Some(trailing) = trailing {
        cells.push(
            text(trailing)
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .into(),
        );
    }
    button(row(cells).spacing(SP_SM).align_y(iced::Alignment::Center))
        .width(Length::Fill)
        .padding([SP_SM - 2.0, SP_MD])
        .style(tokens::menu_row_style)
        .on_press(FerriteBrowserMessage::NavigateRequested(url))
        .into()
}

/// What the library shows when a tab has nothing in it yet: an icon and one
/// sentence, centred in the space, not a bare line of grey text.
fn library_empty<'a>(
    palette: &'static Palette,
    glyph: Icon,
    message: &'static str,
) -> Element<'a, FerriteBrowserMessage> {
    container(
        column![
            icon(glyph, 22.0, tokens::tint(palette.text_dim, 0.6)),
            text(message).size(TEXT_SMALL).color(palette.text_dim),
        ]
        .spacing(SP_SM)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding([SP_XL, SP_MD])
    .center_x(Length::Fill)
    .into()
}

fn library_panel(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let tab_btn = |label: &'static str, tab: LibraryTab| {
        let is_active = state.library_tab == tab;
        button(text(label).size(TEXT_SMALL))
            .padding([SP_XS, SP_MD])
            .style(if is_active {
                panel_btn_active
            } else {
                panel_btn_inactive
            })
            .on_press(FerriteBrowserMessage::SelectLibraryTab(tab))
    };
    let header = container(
        column![
            row![
                text("Library")
                    .size(TEXT_TITLE)
                    .font(font_weight(iced::font::Weight::Semibold))
                    .color(palette.text)
                    .width(Length::Fill),
                tip(
                    button(icon(Icon::Close, 10.0, palette.text_dim))
                        .padding(SP_SM - 2.0)
                        .style(close_btn_style)
                        .on_press(FerriteBrowserMessage::ToggleLibraryPanel),
                    "Close",
                    palette,
                ),
            ]
            .align_y(iced::Alignment::Center),
            row![
                tab_btn("Bookmarks", LibraryTab::Bookmarks),
                tab_btn("History", LibraryTab::History),
                tab_btn("Downloads", LibraryTab::Downloads),
            ]
            .spacing(SP_SM - 2.0),
        ]
        .spacing(SP_SM + 2.0)
        .padding([SP_MD - 2.0, SP_MD]),
    )
    .width(Length::Fill)
    .style(tokens::raised_bar_style);

    fn toolbar_row(
        button: Element<'_, FerriteBrowserMessage>,
    ) -> Element<'_, FerriteBrowserMessage> {
        container(button).padding([SP_SM - 2.0, SP_MD]).into()
    }

    let rows: Vec<Element<FerriteBrowserMessage>> = match state.library_tab {
        LibraryTab::Bookmarks if state.bookmarks.is_empty() => vec![library_empty(
            palette,
            Icon::BookmarkOutline,
            "No bookmarks yet. Use the star in the address bar.",
        )],
        LibraryTab::Bookmarks => state
            .bookmarks
            .iter()
            .enumerate()
            .map(|(i, b)| {
                row![
                    library_page_row(
                        palette,
                        truncate(&b.title, 48),
                        truncate(&b.url, 56),
                        None,
                        b.url.clone()
                    ),
                    tip(
                        button(icon(Icon::Trash, ICON_SIZE_SM, palette.text_dim))
                            .padding(SP_SM - 2.0)
                            .style(close_btn_style)
                            .on_press(FerriteBrowserMessage::RemoveBookmark(i)),
                        "Remove bookmark",
                        palette,
                    ),
                ]
                .spacing(SP_XS)
                .align_y(iced::Alignment::Center)
                .padding([0.0, SP_SM])
                .into()
            })
            .collect(),
        LibraryTab::History => {
            let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::new();
            if state.history.is_empty() {
                rows.push(library_empty(
                    palette,
                    Icon::History,
                    "Nothing visited yet this session.",
                ));
            } else {
                rows.push(toolbar_row(
                    button(text("Clear history").size(TEXT_SMALL))
                        .padding([SP_XS, SP_MD])
                        .style(panel_btn_inactive)
                        .on_press(FerriteBrowserMessage::ClearHistory)
                        .into(),
                ));
                rows.extend(state.history.iter().rev().map(|entry| {
                    library_page_row(
                        palette,
                        truncate(&entry.title, 52),
                        truncate(&entry.url, 60),
                        Some(entry.visited_at.format("%H:%M").to_string()),
                        entry.url.clone(),
                    )
                }));
            }
            rows
        }
        LibraryTab::Downloads => {
            let mut rows = vec![toolbar_row(
                button(text("Download current page").size(TEXT_SMALL))
                    .padding([SP_XS + 1.0, SP_MD])
                    .style(accent_btn_style)
                    .on_press(FerriteBrowserMessage::DownloadCurrentPage)
                    .into(),
            )];
            if state.downloads.is_empty() {
                rows.push(library_empty(palette, Icon::Download, "No downloads yet."));
            } else {
                rows.extend(state.downloads.iter().rev().map(|d| {
                    let (status_text, status_color) = match &d.state {
                        DownloadState::InProgress {
                            downloaded_bytes,
                            total_bytes: Some(total),
                        } if *total > 0 => (
                            format!(
                                "{} \u{2014} {}%",
                                format_bytes(*downloaded_bytes),
                                (*downloaded_bytes as f64 / *total as f64 * 100.0).round() as u32
                            ),
                            palette.text_dim,
                        ),
                        DownloadState::InProgress {
                            downloaded_bytes, ..
                        } => (
                            format!("{} downloaded", format_bytes(*downloaded_bytes)),
                            palette.text_dim,
                        ),
                        DownloadState::Completed => ("Completed".to_string(), palette.safe),
                        DownloadState::Failed(e) => (format!("Failed: {e}"), palette.danger),
                    };
                    container(
                        column![
                            text(d.file_name.clone())
                                .size(TEXT_BODY)
                                .color(palette.text),
                            text(status_text).size(TEXT_CAPTION).color(status_color),
                        ]
                        .spacing(2),
                    )
                    .width(Length::Fill)
                    .padding([SP_SM - 2.0, SP_MD])
                    .into()
                }));
            }
            rows
        }
    };

    // A right-hand drawer like the agent's, so the page keeps its height and
    // the panel's content is not stranded at the far left of a full-width bar.
    container(column![
        header,
        scrollable(column(rows).spacing(0).padding([SP_XS, 0.0])).height(Length::Fill)
    ])
    .width(Length::Fixed(state.panels.side_width(state.window_size)))
    .height(Length::Fill)
    .style(tokens::side_panel_style)
    .into()
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let is_light_theme = state.theme_mode == AppTheme::Light;

    // The tab strip and toolbar live in `chrome.rs`; the rest of the layout
    // below is the page area, its drawers and the bottom panels.
    let tab_bar = chrome::tab_strip(state);
    let toolbar = chrome::toolbar(state);
    let hairline = container(text(""))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(separator_style);
    // The panels' user-chosen sizes, kept inside the window (`layout`).
    let bottom_height = state.panels.bottom_height(state.window_size);

    // ── Audit panel ────────────────────────────────────────────────────────
    let audit_panel: Option<Element<FerriteBrowserMessage>> = if state.show_audit_panel {
        let hdr = activity_panel::header(state, palette);

        let col_hdr = container(
            row![
                text("SEQ").size(11).color(palette.text_dim).width(36),
                text("TIME").size(11).color(palette.text_dim).width(76),
                text("KIND").size(11).color(palette.text_dim).width(72),
                text("PRINCIPAL").size(11).color(palette.text_dim).width(95),
                text("CAPABILITY")
                    .size(11)
                    .color(palette.text_dim)
                    .width(95),
                text("URL")
                    .size(11)
                    .color(palette.text_dim)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .padding([3, PANEL_PADDING]),
        )
        .width(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(Color {
                a: 0.5,
                ..palette.raised
            })),
            ..container::Style::default()
        });

        let rows: Vec<Element<FerriteBrowserMessage>> = if state.audit_entries.is_empty() {
            vec![container(
                text("No network requests logged yet. Load a page, then click Refresh.")
                    .size(12)
                    .color(palette.text_dim),
            )
            .width(Length::Fill)
            .padding([12, PANEL_PADDING])
            .into()]
        } else {
            state
                .audit_entries
                .iter()
                .map(|e| {
                    let (ks, kc) = kind_label(&e.kind, palette, is_light_theme);
                    let ts = e.timestamp.format("%H:%M:%S%.3f").to_string();
                    container(
                        row![
                            text(e.sequence.to_string())
                                .size(12)
                                .color(palette.text_dim)
                                .width(36),
                            text(ts).size(12).color(palette.text_dim).width(76),
                            text(ks).size(12).color(kc).width(72),
                            text(truncate(&e.principal_id.to_string(), 8))
                                .size(12)
                                .color(palette.text)
                                .width(95),
                            text(e.capability.as_deref().unwrap_or("-"))
                                .size(12)
                                .color(palette.text)
                                .width(95),
                            text(truncate(e.url.as_deref().unwrap_or("-"), 60))
                                .size(12)
                                .color(palette.text_dim)
                                .width(Length::Fill),
                        ]
                        .spacing(8)
                        .padding([3, PANEL_PADDING]),
                    )
                    .width(Length::Fill)
                    .into()
                })
                .collect()
        };

        let body: Element<FerriteBrowserMessage> = match state.audit_tab {
            AuditTab::Models => activity_panel::models_body(state, palette),
            AuditTab::Security => {
                column![col_hdr, scrollable(column(rows)).height(Length::Fill)].into()
            }
        };
        Some(
            container(column![hdr, body])
                .width(Length::Fill)
                .height(Length::Fixed(bottom_height))
                .style(bottom_panel_style)
                .into(),
        )
    } else {
        None
    };

    // ── DevTools panel (Console / Network / Engine) ────────────────────────
    let js_panel: Option<Element<FerriteBrowserMessage>> = state.show_js_console.then(|| {
        container(devtools_panel::view(state))
            .width(Length::Fill)
            .height(Length::Fixed(bottom_height))
            .style(bottom_panel_style)
            .into()
    });

    // ── Find bar ──────────────────────────────────────────────────────────
    // A floating card in the page's top-right corner (like every desktop
    // browser's), stacked over the page rather than laid out above it, so
    // opening it never resizes the engine's render buffer.
    let find_bar: Option<Element<FerriteBrowserMessage>> = if state.show_find_bar {
        let match_label = if state.find_query.is_empty() {
            String::new()
        } else if state.find_match_count == 0 {
            "No results".to_string()
        } else {
            format!("{} of {}", state.find_current_index, state.find_match_count)
        };
        let step_btn = |kind: Icon, tip_text: &'static str, msg: FerriteBrowserMessage| {
            tip(
                button(container(icon(kind, ICON_SIZE_SM, palette.text_dim)).center(Length::Fill))
                    .width(Length::Fixed(26.0))
                    .height(Length::Fixed(26.0))
                    .padding(0)
                    .style(toolbar_btn_style)
                    .on_press(msg),
                tip_text,
                palette,
            )
        };
        Some(
            container(
                row![
                    icon(Icon::Search, ICON_SIZE_SM, palette.text_dim),
                    text_input("Find in page", &state.find_query)
                        .id(text_input::Id::new(FIND_INPUT_ID))
                        .width(Length::Fixed(200.0))
                        .padding([4, 8])
                        .size(TEXT_BODY)
                        .style(|theme: &Theme, status| tokens::field_style(
                            theme, status, RADIUS_SM
                        ))
                        .on_input(FerriteBrowserMessage::FindQueryChanged)
                        .on_submit(FerriteBrowserMessage::FindNext),
                    text(match_label)
                        .size(TEXT_SMALL)
                        .color(palette.text_dim)
                        .width(Length::Fixed(64.0)),
                    step_btn(
                        Icon::ChevronUp,
                        "Previous match",
                        FerriteBrowserMessage::FindPrevious
                    ),
                    step_btn(
                        Icon::ChevronDown,
                        "Next match (Enter)",
                        FerriteBrowserMessage::FindNext
                    ),
                    step_btn(
                        Icon::Close,
                        "Close (Esc)",
                        FerriteBrowserMessage::CloseFindBar
                    ),
                ]
                .spacing(SP_XS)
                .align_y(iced::Alignment::Center),
            )
            .padding([SP_XS, SP_SM])
            .style(tokens::popover_style)
            .into(),
        )
    } else {
        None
    };

    // ── Library panel (C3d): bookmarks / history / downloads / settings ────
    let library_panel: Option<Element<FerriteBrowserMessage>> = if state.show_library_panel {
        Some(library_panel(state))
    } else {
        None
    };

    // ── Content area ───────────────────────────────────────────────────────
    let active = state.active_tab;

    // Wrapped in `responsive` so `state.content_area_size` always reflects
    // this container's true logical size — whatever else in the layout
    // (window size, the agent sidebar, the audit/JS panel) is currently
    // taking space — rather than a value this function would otherwise
    // have to compute by hand-replicating that layout's arithmetic. See
    // `content_area_size`'s doc comment and `ServoFrame`'s tick handler,
    // which reads this back to keep the Servo render buffer's physical
    // pixel size matched to it.
    let content: Element<FerriteBrowserMessage> = responsive(move |size: Size| {
        state.content_area_size.set(size);
        let page = if let Some(err_msg) = state.tab_error.get(active).and_then(|e| e.as_ref()) {
            let failed_url = state.tab_urls.get(active).map(String::as_str).unwrap_or("");
            pages::error_page(palette, failed_url, err_msg)
        } else if state
            .tab_urls
            .get(active)
            .map(|u| u == "about:blank")
            .unwrap_or(true)
        {
            // Home / new-tab page — always shown for about:blank, even if Servo
            // has produced a blank white frame for that URL.
            new_tab_page(state)
        } else if let Some((frame, handle)) = state.frame_cache.get(&active) {
            // Live Servo frame — interactive via mouse_area. The picture is
            // cached per frame (`refresh_frame_cache`), so a redraw that
            // changes nothing uploads nothing. Pointer moves and wheel input
            // only record intent here; `ServoFrame` forwards them once per
            // tick (see `update`).
            let picture: Element<FerriteBrowserMessage> = match handle {
                Some(handle) => ServoImage::new(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => iced::widget::shader(page_view::PageView::new(frame.clone()))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            };

            mouse_area(container(picture).width(Length::Fill).height(Length::Fill))
                .interaction(page_interaction(state))
                .on_move(|pos| FerriteBrowserMessage::ServoMouseMove { x: pos.x, y: pos.y })
                .on_press(FerriteBrowserMessage::ServoMousePress)
                .on_right_press(FerriteBrowserMessage::ServoRightPress)
                .on_release(FerriteBrowserMessage::ServoMouseRelease)
                .on_scroll(|delta| {
                    use iced::mouse::ScrollDelta;
                    FerriteBrowserMessage::ServoScroll(match delta {
                        ScrollDelta::Lines { x, y } => scroll::Wheel::Lines { x, y },
                        ScrollDelta::Pixels { x, y } => scroll::Wheel::Pixels { x, y },
                    })
                })
                .into()
        } else {
            // Loading placeholder (no frame yet for a non-blank URL)
            pages::loading_page(palette, state.progress_offset)
        };
        // What the page is waiting on a person for, and a crash notice, float
        // over it inside this same area, so a popup's anchor needs no offset.
        let mut layers = vec![page];
        layers.extend(controls::overlay(state, size));
        layers.extend(crash::banner(state));
        // The "this page is capturing" bar, and (on top of everything) a request the
        // page is waiting on a person for.
        layers.extend(permission::bar(state));
        layers.extend(permission::overlay(state));
        if layers.len() == 1 {
            layers.remove(0)
        } else {
            stack(layers).into()
        }
    })
    .into();

    // ── Compose layout ──────────────────────────────────────────────────────
    let mut layout: Vec<Element<FerriteBrowserMessage>> = vec![tab_bar, toolbar, hairline.into()];

    // The audit and JS panels are developer-style drawers and sit below the
    // page, like browser devtools; the library is a drawer on the right.
    let bottom_panel = if audit_panel.is_some() {
        audit_panel
    } else {
        js_panel
    };

    // The find bar floats over the page's top-right corner.
    let content: Element<FerriteBrowserMessage> = match find_bar {
        Some(bar) => stack([
            content,
            container(bar)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::Alignment::End)
                .padding(SP_SM)
                .into(),
        ])
        .into(),
        None => content,
    };

    // The page and, to its right, at most one drawer (library, settings or the
    // agent) with a splitter between them to resize it.
    let drawer: Option<Element<FerriteBrowserMessage>> = if let Some(library) = library_panel {
        Some(library)
    } else if state.show_settings_panel {
        Some(settings_panel::view(state))
    } else if state.show_agent_sidebar {
        Some(agent_panel::view_agent_sidebar(state))
    } else {
        None
    };
    let main_content: Element<FerriteBrowserMessage> = match drawer {
        Some(drawer) => iced::widget::row![
            content,
            layout::splitter(state, palette, layout::Handle::Side),
            drawer
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        None => content,
    };
    layout.push(main_content);
    if let Some(p) = bottom_panel {
        layout.push(layout::splitter(state, palette, layout::Handle::Bottom));
        layout.push(p);
    }

    let window: Element<FerriteBrowserMessage> = container(column(layout))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(page_style)
        .into();

    // The overflow menu floats over everything, with a click-away layer.
    match chrome::menu_overlay(state) {
        Some(menu) => stack([window, menu]).into(),
        None => window,
    }
}

// ---------------------------------------------------------------------------
// New-tab / home page
// ---------------------------------------------------------------------------

/// The glyph shown for `tile.label` before/without a real favicon — its
/// first character, uppercased (`"GitHub"` -> `"G"`, `"Hacker News"` ->
/// `"H"`). Pure and separate from `new_tab_page` so it's directly testable
/// without constructing any widget.
fn tile_monogram(label: &str) -> String {
    label
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

/// Whether `modifiers` hold what Cmd+I (macOS) / Ctrl+I (elsewhere) needs to
/// become the developer-tools chord: Option on macOS, Shift elsewhere.
fn devtools_chord(modifiers: keyboard::Modifiers) -> bool {
    #[cfg(target_os = "macos")]
    return modifiers.alt();
    #[cfg(not(target_os = "macos"))]
    return modifiers.shift();
}

/// A pure `Key + Modifiers -> Message` mapping — `iced::keyboard::on_key_press`
/// requires a plain `fn` pointer (verified against the pinned `iced_futures`
/// 0.13.2 source: it takes `fn(Key, Modifiers) -> Option<Message>`, not a
/// closure, so nothing here can capture `&FerriteBrowser` state directly).
/// Escape's two possible meanings (close the find bar vs. its pre-existing
/// stop-loading/unfocus-address-bar behavior) are therefore both resolved to
/// the *same* `EscapePressed` message here, and `update()`'s handler for it
/// is what actually reads `state.show_find_bar` to pick between them — see
/// that handler and this file's module docs' keyboard-shortcuts section.
fn handle_key_press(
    key: keyboard::Key,
    modifiers: keyboard::Modifiers,
) -> Option<FerriteBrowserMessage> {
    use keyboard::key::Named;

    // On macOS the modifier is the Super (Cmd) key; everywhere else it's Ctrl.
    #[cfg(target_os = "macos")]
    let mod_active = modifiers.command();
    #[cfg(not(target_os = "macos"))]
    let mod_active = modifiers.control();

    match key {
        keyboard::Key::Character(c) if mod_active => match c.as_str() {
            "t" => Some(FerriteBrowserMessage::AddTab),
            "w" => Some(FerriteBrowserMessage::CloseActiveTab),
            "r" => Some(FerriteBrowserMessage::Reload),
            "l" => Some(FerriteBrowserMessage::FocusAddressBar),
            "j" => Some(FerriteBrowserMessage::ToggleJsConsole),
            "i" | "I" if devtools_chord(modifiers) => Some(FerriteBrowserMessage::ToggleJsConsole),
            "f" => Some(FerriteBrowserMessage::OpenFindBar),
            // Cmd/Ctrl+Shift+O: new agent chat. With Shift held the OS
            // reports the shifted character, so both cases are accepted; a
            // bare Cmd/Ctrl+O is not bound.
            "o" | "O" if modifiers.shift() => Some(FerriteBrowserMessage::NewChat),
            // "=" covers the common US-keyboard case where Cmd/Ctrl+Plus is
            // actually sent as Cmd/Ctrl+'=' (Plus's un-shifted key); "+" is
            // handled too for a layout/OS that does send it directly.
            "=" | "+" => Some(FerriteBrowserMessage::ZoomIn),
            "-" => Some(FerriteBrowserMessage::ZoomOut),
            "0" => Some(FerriteBrowserMessage::ZoomReset),
            "d" => Some(FerriteBrowserMessage::ToggleBookmarkCurrentPage),
            "a" | "A" if modifiers.shift() => Some(FerriteBrowserMessage::ToggleAgentSidebar),
            "," => Some(FerriteBrowserMessage::ToggleSettingsPanel),
            // Cmd/Ctrl+[ and +] are back and forward (Chrome, Safari); with
            // Shift held the OS reports the brace, which steps between tabs.
            "[" => Some(FerriteBrowserMessage::GoBack),
            "]" => Some(FerriteBrowserMessage::GoForward),
            "{" => Some(FerriteBrowserMessage::PrevTab),
            "}" => Some(FerriteBrowserMessage::NextTab),
            // Cmd/Ctrl+1..8 jump to that tab, +9 to the last, as in Chrome.
            digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") => digit
                .parse::<usize>()
                .ok()
                .map(|n| FerriteBrowserMessage::SelectTabNumber(n - 1)),
            "9" => Some(FerriteBrowserMessage::SelectLastTab),
            _ => None,
        },
        keyboard::Key::Named(Named::Tab) if modifiers.control() => Some(if modifiers.shift() {
            FerriteBrowserMessage::PrevTab
        } else {
            FerriteBrowserMessage::NextTab
        }),
        keyboard::Key::Named(Named::F5) => Some(FerriteBrowserMessage::Reload),
        keyboard::Key::Named(Named::F12) => Some(FerriteBrowserMessage::ToggleAuditPanel),
        keyboard::Key::Named(Named::ArrowLeft) if modifiers.alt() => {
            Some(FerriteBrowserMessage::GoBack)
        }
        keyboard::Key::Named(Named::ArrowRight) if modifiers.alt() => {
            Some(FerriteBrowserMessage::GoForward)
        }
        keyboard::Key::Named(Named::Escape) => Some(FerriteBrowserMessage::EscapePressed),
        _ => None,
    }
}

/// The session a forwarded key should go to, or `None` when the keyboard
/// belongs to browser chrome or there is no real page to type into: the
/// address bar or find bar has focus, or the active tab is showing the
/// new-tab page (which has its own search box, a widget that consumes typed
/// keys itself).
fn page_key_target(state: &FerriteBrowser) -> Option<&HeadlessServoSession> {
    if state.address_bar_focused || state.show_find_bar || state.devtools.input_focused {
        return None;
    }
    if controls::active_control(state).is_some() || permission::active_prompt(state).is_some() {
        return None;
    }
    let showing_page = state
        .tab_urls
        .get(state.active_tab)
        .is_some_and(|u| !u.is_empty() && u != "about:blank");
    if !showing_page {
        return None;
    }
    state.servo_sessions.get(&state.active_tab)
}

/// Whether a key press that a focused widget captured is nevertheless one of
/// the browser's own shortcuts: Cmd/Ctrl plus a letter or symbol, or F5/F12.
/// Editing combinations (Cmd/Ctrl+A/C/V/X/Z) are not in
/// [`handle_key_press`]'s table, so they still reach the field.
fn is_captured_chrome_shortcut(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> bool {
    #[cfg(target_os = "macos")]
    let mod_active = modifiers.command();
    #[cfg(not(target_os = "macos"))]
    let mod_active = modifiers.control();
    match key {
        keyboard::Key::Character(_) => mod_active,
        keyboard::Key::Named(keyboard::key::Named::F5 | keyboard::key::Named::F12) => true,
        keyboard::Key::Named(keyboard::key::Named::Tab) => modifiers.control(),
        _ => false,
    }
}

/// `event::listen_with` filter for [`FerriteBrowserMessage::PageKey`]. Only
/// events no widget captured (a focused `text_input` captures what it types,
/// which is exactly how "typing in the address bar" never reaches the page)
/// and that are not one of the browser's own shortcuts.
fn page_key_from_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: window::Id,
) -> Option<FerriteBrowserMessage> {
    let iced::Event::Keyboard(key_event) = event else {
        return None;
    };
    if status != iced::event::Status::Ignored {
        // A focused text field (the address bar, the agent box) captures every
        // key, which used to leave Cmd/Ctrl+T, +L, +W, +R and friends dead
        // whenever one of them had focus — right after typing an address, say.
        // The browser's own shortcuts are honoured even then; `on_key_press`
        // (in `subscription`) already covers the uncaptured case, so this is
        // only for captured events and nothing is handled twice.
        return match &key_event {
            keyboard::Event::KeyPressed { key, modifiers, .. }
                if is_captured_chrome_shortcut(key, *modifiers) =>
            {
                handle_key_press(key.clone(), *modifiers)
            }
            _ => None,
        };
    }
    let (key, modifiers) = match &key_event {
        keyboard::Event::KeyPressed { key, modifiers, .. }
        | keyboard::Event::KeyReleased { key, modifiers, .. } => (key.clone(), *modifiers),
        keyboard::Event::ModifiersChanged(_) => return None,
    };
    if handle_key_press(key, modifiers).is_some() {
        return None;
    }
    page_input::page_key_from_iced(&key_event).map(FerriteBrowserMessage::PageKey)
}

/// The pointer the page wants, as a toolkit cursor. Iced has no hidden cursor,
/// so `cursor: none` shows the ordinary arrow.
fn cursor_interaction(cursor: ferrite_servo::diag::PageCursor) -> iced::mouse::Interaction {
    use ferrite_servo::diag::PageCursor as C;
    use iced::mouse::Interaction as I;
    match cursor {
        C::Default | C::Hidden => I::Idle,
        C::Pointer => I::Pointer,
        C::Text => I::Text,
        C::Crosshair => I::Crosshair,
        C::Grab => I::Grab,
        C::Grabbing => I::Grabbing,
        C::Move => I::Move,
        C::NotAllowed => I::NotAllowed,
        C::Wait => I::Working,
        C::Help => I::Help,
        C::ZoomIn => I::ZoomIn,
        C::ZoomOut => I::ZoomOut,
        C::ResizeHorizontal => I::ResizingHorizontally,
        C::ResizeVertical => I::ResizingVertically,
        C::ResizeDiagonalUp => I::ResizingDiagonallyUp,
        C::ResizeDiagonalDown => I::ResizingDiagonallyDown,
    }
}

/// The cursor over the page area: what the page asked for, except while a
/// control or a panel drag has the pointer (the plain arrow then).
fn page_interaction(state: &FerriteBrowser) -> iced::mouse::Interaction {
    if page_input_blocked(state) {
        return iced::mouse::Interaction::Idle;
    }
    state
        .servo_sessions
        .get(&state.active_tab)
        .map_or(iced::mouse::Interaction::Idle, |session| {
            cursor_interaction(session.cursor())
        })
}

/// How long the engine tick sleeps. Every tick rebuilds the view, so a page
/// that is sitting still should not pay 60 rebuilds a second: after
/// `BUSY_TICKS` ticks with no new picture, no load, no input, no queued scroll
/// and no resize, the tick slows to `IDLE_TICK`. Anything that wakes the page
/// (see `wake`) puts it straight back to `ACTIVE_TICK`.
fn tick_interval(state: &FerriteBrowser) -> std::time::Duration {
    if page_is_active(state) {
        ACTIVE_TICK
    } else {
        IDLE_TICK
    }
}

fn page_is_active(state: &FerriteBrowser) -> bool {
    let logical = state.content_area_size.get();
    let scale = state.scale_factor;
    let desired = (
        (logical.width * scale).round().max(1.0) as u32,
        (logical.height * scale).round().max(1.0) as u32,
    );
    state.busy_ticks > 0
        || state.is_loading
        || state.agent_is_running
        || state.scroll_queue.is_pending()
        || state.pointer_moved
        || state.resize_settle_ticks > 0
        || desired != state.last_resized_content_px
}

/// Marks the page busy now: input just arrived, so the next frames matter.
fn wake(state: &mut FerriteBrowser) {
    wake_flag(&mut state.busy_ticks);
}

/// [`wake`] for a caller that holds only the counter (it is borrowing other
/// parts of the state).
fn wake_flag(busy_ticks: &mut u8) {
    *busy_ticks = BUSY_TICKS;
}

/// Whether the page must not be sent pointer, wheel or key input right now: a
/// control it is waiting on is showing, or a panel is being dragged (the pointer
/// belongs to the splitter, and a release over the page must not click it).
fn page_input_blocked(state: &FerriteBrowser) -> bool {
    state.panels.drag.is_some()
        || controls::active_control(state).is_some()
        || permission::active_prompt(state).is_some()
}

/// A cheap fingerprint of a favicon, to tell a new icon from the same one
/// reported again.
fn favicon_key(width: u32, height: u32, rgba: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    rgba.hash(&mut hasher);
    hasher.finish()
}

/// One `ServoFrame` each time the engine's waker fires, or after `IDLE_TICK`
/// without one. The wait blocks, so it runs on a blocking thread; it ends when
/// the subscription does (the page got busy and ticks with the display again).
fn engine_wakes() -> impl iced::futures::Stream<Item = FerriteBrowserMessage> {
    use iced::futures::SinkExt;
    iced::stream::channel(1, |mut output| async move {
        loop {
            let _ = tokio::task::spawn_blocking(|| {
                ferrite_servo::session::wait_for_engine_wake(IDLE_TICK)
            })
            .await;
            if output
                .send(FerriteBrowserMessage::ServoFrame)
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

pub fn subscription(state: &FerriteBrowser) -> Subscription<FerriteBrowserMessage> {
    let keyboard_sub = keyboard::on_key_press(handle_key_press);
    let page_keys = iced::event::listen_with(page_key_from_event);

    // The engine tick. A page that is busy follows the display: one tick per
    // frame the window draws (`window::frames`), so pumping the engine, reading
    // its picture back and drawing it line up with the monitor's refresh and
    // never beat against a separate 16 ms timer. A page sitting still falls back
    // to a slow timer. Every message makes iced draw again, so the frame
    // subscription feeds itself while it is on and stops the moment the page
    // goes idle; the watchdog covers a window that is not being drawn at all.
    let servo_tick = if state.servo_sessions.is_empty() {
        Subscription::none()
    } else {
        let cadence = tick_interval(state);
        if cadence <= ACTIVE_TICK {
            Subscription::batch([
                window::frames().map(|_| FerriteBrowserMessage::ServoFrame),
                time::every(WATCHDOG_AFTER).map(|_| FerriteBrowserMessage::ServoWatchdog),
            ])
        } else {
            // Nothing is moving: tick only when the engine says it has work
            // (its event-loop waker), and at the latest every `IDLE_TICK`.
            // A timer tick redraws the whole window, so polling here drew a
            // still page twenty times a second.
            Subscription::run(engine_wakes)
        }
    };

    // While a splitter is being dragged, follow the pointer and the release.
    // Nothing is subscribed otherwise, so ordinary mouse moves cost nothing here.
    let panel_drag = if state.panels.drag.is_some() {
        iced::event::listen_with(layout::drag_events)
    } else {
        Subscription::none()
    };

    // The window's size, so panel sizes can be kept inside it.
    let window_size =
        window::resize_events().map(|(_, size)| FerriteBrowserMessage::WindowResized(size));

    // Consent-panel entrance animation (C1) — same 16ms tick shape as
    // `servo_tick` above, gated so it only ever runs while the panel is
    // actually animating in, not for the rest of the app's lifetime.
    let consent_anim_tick = if state.pending_diff.is_some() && state.consent_panel_anim < 1.0 {
        time::every(std::time::Duration::from_millis(16))
            .map(|_| FerriteBrowserMessage::ConsentPanelTick)
    } else {
        Subscription::none()
    };

    // The overflow menu's slide-in, only while it is opening.
    let menu_anim_tick = if state.show_menu && state.menu_anim < 1.0 {
        time::every(std::time::Duration::from_millis(16))
            .map(|_| FerriteBrowserMessage::MenuAnimTick)
    } else {
        Subscription::none()
    };

    // Chat-thread entrance animations — same 16ms tick shape, gated so it only
    // runs while an item is actually fading in.
    let thread_anim_tick = if agent_panel::thread_anim_active(state) {
        time::every(std::time::Duration::from_millis(16))
            .map(|_| FerriteBrowserMessage::ThreadAnimTick)
    } else {
        Subscription::none()
    };

    // Drain agent progress messages (AgentToolLogged, AgentCompleted, AgentFailed).
    struct AgentEventChannel;
    let agent_event_sub: Subscription<FerriteBrowserMessage> =
        if let Some(rx_arc) = &state.agent_event_rx {
            let rx_arc = rx_arc.clone();
            Subscription::run_with_id(
                std::any::TypeId::of::<AgentEventChannel>(),
                iced::stream::channel(64, move |mut sender| async move {
                    use iced::futures::SinkExt;
                    loop {
                        let msg = rx_arc.lock().await.recv().await;
                        match msg {
                            Some(msg) => {
                                let _ = sender.send(msg).await;
                            }
                            None => {
                                std::future::pending::<()>().await;
                            }
                        }
                    }
                }),
            )
        } else {
            Subscription::none()
        };

    // While the Audit panel is open its trace view refreshes once a second.
    let trace_tick = if state.show_audit_panel {
        time::every(std::time::Duration::from_secs(1)).map(|_| FerriteBrowserMessage::RefreshTrace)
    } else {
        Subscription::none()
    };

    let close_requests =
        window::close_requests().map(|_| FerriteBrowserMessage::WindowCloseRequested);

    Subscription::batch([
        keyboard_sub,
        page_keys,
        trace_tick,
        close_requests,
        servo_tick,
        panel_drag,
        window_size,
        agent_event_sub,
        consent_anim_tick,
        menu_anim_tick,
        thread_anim_tick,
    ])
}

// ---------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------

/// The app icon (assets/icon/ferrite.svg rendered by scripts/make_icons.py),
/// embedded so the binary needs no files beside it. Shown in the title bar and
/// task switcher on Windows and Linux; macOS takes its Dock icon from the
/// `.app` bundle (scripts/package.sh), not from the window.
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/ferrite-256.png");

fn window_icon() -> Option<window::Icon> {
    let (width, height, rgba) = decode_favicon_rgba(WINDOW_ICON_PNG)?;
    window::icon::from_rgba(rgba, width, height).ok()
}

/// The OS window title: the active page's title and the app name. Only
/// visible where the window keeps its native title bar (and in the task
/// switcher everywhere).
fn window_title(state: &FerriteBrowser) -> String {
    chrome::window_title(
        state
            .tab_titles
            .get(state.active_tab)
            .map_or("", String::as_str),
        state
            .tab_urls
            .get(state.active_tab)
            .map_or("about:blank", String::as_str),
    )
}

/// The window's own settings. On macOS the title bar is transparent, its text
/// hidden and the content extended underneath it, so the traffic lights sit
/// inside the tab strip (see `chrome`) and no native title row is left above
/// it. Linux and Windows keep their normal decorations.
fn window_settings() -> window::Settings {
    window::Settings {
        icon: window_icon(),
        // A window smaller than this cannot show the toolbar's controls.
        min_size: Some(Size::new(640.0, 420.0)),
        #[cfg(target_os = "macos")]
        platform_specific: window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
        // Lets a Linux desktop match the window to `ferrite.desktop`
        // (and so to its icon) in the launcher and task bar.
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "ferrite".to_string(),
            ..Default::default()
        },
        ..window::Settings::default()
    }
}

pub fn launch() -> iced::Result {
    lifecycle::start_stall_note();
    iced::application(window_title, update, view)
        .window(window_settings())
        .window_size(Size::new(1280.0, 800.0))
        .centered()
        // Closing the window is handled (`WindowCloseRequested`) so the engine
        // can shut down cleanly and write the browser profile to disk.
        .exit_on_close_request(false)
        .theme(|state: &FerriteBrowser| state.theme_mode.to_iced_theme())
        .subscription(subscription)
        // Embedded Inter (see the "Fonts" section above for why) — all
        // four weights are registered, and `default_font` is what makes
        // every `text`/`text_input`/`button` widget in this file that
        // never sets its own `.font(...)` use Inter Regular instead of
        // iced's built-in fallback sans-serif.
        .font(FONT_INTER_REGULAR)
        .font(FONT_INTER_MEDIUM)
        .font(FONT_INTER_SEMIBOLD)
        .font(FONT_INTER_BOLD)
        .default_font(Font::with_name(FONT_FAMILY))
        .run_with(|| {
            // C3a: `.window_size(...).centered()` above is only the frame
            // before the window manager has a chance to place it — the
            // startup task below immediately requests maximized, so the
            // app opens filling the display rather than the small
            // fixed-size centered window it used to. `get_scale_factor`
            // in the same chain is what makes the Servo render buffer and
            // every pointer-event coordinate correct on a HiDPI/Retina
            // display — see `scale_factor`'s doc comment on
            // `FerriteBrowser` for why both bugs (blurry frame, hover/
            // click landing in the wrong place) traced back to this
            // never being queried at all.
            let startup_window_task = window::get_latest().then(|id| match id {
                Some(id) => Task::batch([
                    window::maximize(id, true),
                    window::get_scale_factor(id).map(FerriteBrowserMessage::ScaleFactorReady),
                    window::get_size(id).map(FerriteBrowserMessage::WindowResized),
                ]),
                None => Task::none(),
            });

            // T-224/T-229: connect the real ModelProvider here, at actual app
            // startup — never inside `FerriteBrowser::default()` itself (kept
            // test-safe/R7, see that impl's own comment). The Settings
            // drawer's saved choice is read from the data directory, the key
            // from the environment or the OS keyring, and the exported
            // variables still win over the file, exactly as they always did.
            // A failure leaves the app on "No model connected"; it never
            // stops it starting.
            let mut state = FerriteBrowser {
                settings: settings_panel::SettingsState::for_launch(
                    ferrite_agent::chat::default_data_dir(),
                ),
                ..FerriteBrowser::default()
            };
            settings_panel::connect_saved(&mut state);
            // The panel sizes the person last chose, if any. A missing or
            // unreadable file is the default sizes, never an error.
            if let Some(path) = layout::default_layout_path() {
                state.panels.layout = layout::PanelLayout::load_from(&path);
                state.panels.path = Some(path);
            }
            // C3d: bookmarks/downloads real-path resolution and the one-time
            // bookmarks load — both only ever happen here, at real app
            // startup, never inside `FerriteBrowser::default()` (same
            // test-safety discipline as the model-provider construction
            // just above: dozens of tests construct `FerriteBrowser`
            // directly and must never touch `$HOME`/the filesystem).
            if let Some(path) = default_bookmarks_path() {
                state.bookmarks = load_bookmarks_from(&path);
                state.bookmarks_path = Some(path);
            } else {
                eprintln!(
                    "[ferrite-ui] cannot resolve the home directory for the bookmarks file — \
                     bookmarks will work for this session but won't be saved"
                );
            }
            // Chats: resolve the real chats directory and read the list of
            // saved chats once, here — never inside `Default` (same
            // test-safety discipline as bookmarks). A corrupt chat file costs
            // exactly that chat: `ChatStore::list` skips it and reports a
            // warning, which becomes a log line here and nothing more.
            match default_chats_dir() {
                Some(dir) => {
                    let store = ChatStore::new(dir);
                    let (list, warnings) = store.list();
                    for warning in warnings {
                        eprintln!("[ferrite-ui] skipping an unreadable chat: {warning}");
                    }
                    state.chat_list = list;
                    state.chat_store = Some(store);
                }
                None => eprintln!(
                    "[ferrite-ui] cannot resolve the home directory for saved chats — \
                     chats will work for this session but won't be saved"
                ),
            }
            // Optional Laya fast lane: resolved here only (never in `Default`).
            // Logs, once, whether it is on and — if the server is not on this
            // machine — that page text, URLs and element labels leave it.
            // Model-activity trace: also kept on disk under the data directory
            // (see `ferrite_model::trace` for what it holds and why it is local).
            if let Some(dir) = ferrite_agent::chat::default_data_dir() {
                ferrite_model::trace::global()
                    .set_file(dir.join("logs").join("model-activity.jsonl"));
            }
            let (laya, laya_log) = agent_run::resolve_laya(&ferrite_model::SystemEnv);
            for line in laya_log {
                eprintln!("{line}");
            }
            state.laya = laya;
            state.downloads_dir = default_downloads_dir();
            if state.downloads_dir.is_none() {
                eprintln!(
                    "[ferrite-ui] cannot resolve a downloads directory — \
                     the download-current-page action will be a no-op"
                );
            }
            // New-tab hero: real quick-access-tile favicons — resolved here
            // alongside bookmarks/downloads (same test-safety discipline: a
            // real home-directory lookup, never reachable from `Default`),
            // fetched via the startup `FetchTileFavicons` message below
            // rather than a direct `tokio::task::spawn` in this closure, so
            // the one place this crate ever spawns a background task is
            // `update()` — matching every other background task in this
            // file (`run_download`, the agent loop's model-call steps).
            state.favicons_cache_dir = favicon_cache_dir();
            if state.favicons_cache_dir.is_none() {
                eprintln!(
                    "[ferrite-ui] cannot resolve the home directory for the favicon cache — \
                     quick-access tiles will show their monogram fallback for this session"
                );
            }
            let favicon_task = Task::done(FerriteBrowserMessage::FetchTileFavicons);
            // The engine is built by the first session, once per process, so
            // the saved browser identity (the User-Agent sites see) is handed
            // over now, before it exists.
            ferrite_servo::session::set_user_agent(state.settings.identity.user_agent());
            match HeadlessServoSession::new(1280, 700) {
                Ok(session) => {
                    state.servo_sessions.insert(0, session);
                    sync_active_webview(&mut state);
                    (
                        state,
                        Task::batch([
                            Task::done(FerriteBrowserMessage::ServoReady),
                            startup_window_task,
                            favicon_task,
                        ]),
                    )
                }
                Err(e) => {
                    eprintln!("[ferrite-ui] Servo unavailable: {}", e);
                    (state, Task::batch([startup_window_task, favicon_task]))
                }
            }
        })
}

// ---------------------------------------------------------------------------
// Tests — the consent panel's state machine, summary rendering, and real
// enforcement, per `docs/REBUILD_DIRECTIVE.md` §6/A10's test requirements.
//
// No test here constructs a `HeadlessServoSession` (R7/no live resources) —
// `FerriteBrowser::default()` never does either, so `update()` is directly
// testable with a plain `FerriteBrowser` and no window, matching the
// directive's "no rendering needed" requirement. `FerriteBrowser::default()`
// also never reads the real keyring (`SettingsState::for_launch` only runs in
// `launch()`, the real app entry point — see that impl's own comment), so
// every test's `model_provider` is `MockProvider`, scripted with nothing —
// R7 holds even for the tests below that do trigger a `tokio::task::spawn`.
//
// `#[tokio::test]` is used wherever a message handler calls
// `tokio::task::spawn` (`ConsentSubmitted`/`LiveRunReady`/`AgentStepReady`)
// — spawning requires an active Tokio runtime context or it panics, even
// though the test never awaits the spawned future into existence. Since
// none of these tests `.await` anything after triggering the spawn, the
// current-thread test runtime never actually polls the spawned task before
// the test ends, so the step's `ModelProvider::complete` call inside it
// never executes at all — and even if it somehow did, `MockProvider` with
// nothing scripted only ever returns a typed `ModelError`, never reaches a
// socket. No live call is made, per R7.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod activity_tests;
#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod chrome_tests;
#[cfg(test)]
mod fast_lane_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod page_key_tests;

#[cfg(test)]
mod tests {
    use super::*;

    // ── Fixtures ─────────────────────────────────────────────────────────

    fn sample_expected() -> ExpectedFingerprint {
        ExpectedFingerprint::from_capabilities(
            ferrite_core::ExpectedCapabilitySet::new(vec![ferrite_core::ExpectedCapability::new(
                ferrite_core::Capability::WebRead,
                ferrite_core::scope::OriginScope::exact([ferrite_core::Origin::parse(
                    "https://example.com",
                )
                .unwrap()])
                .unwrap(),
            )])
            .unwrap(),
        )
    }

    fn diff_with_extra_primitive() -> FingerprintDiff {
        let mut diff = FingerprintDiff::default();
        diff.extra_primitives.insert(ToolId::new("js.execute"));
        diff
    }

    fn diff_with_out_of_scope_origin() -> FingerprintDiff {
        let mut diff = FingerprintDiff::default();
        diff.out_of_scope_origins
            .insert("https://attacker.example".to_string());
        diff
    }

    fn mixed_diff() -> FingerprintDiff {
        let mut diff = FingerprintDiff::default();
        diff.extra_primitives.insert(ToolId::new("js.execute"));
        diff.out_of_scope_origins
            .insert("https://attacker.example".to_string());
        diff
    }

    /// Builds an empty `DryRunRecord` tied to `task`'s ids, without needing a
    /// direct `uuid` dependency in this crate.
    fn evidence_for(task: &IpiTask) -> DryRunRecord {
        DryRunRecord::new(task.session_id, task.task_id)
    }

    // ── R7: no automated test call reaches the real keyring ──────────────

    /// `SettingsState::for_launch` is the one place the app reads the real OS
    /// keyring and the process environment (`Default` and `load_from` use an
    /// in-memory vault and an empty environment). Mirrors
    /// `ferrite-eval::harness::no_automated_test_calls_try_real_provider`
    /// (same technique): this crate's non-test source must call it exactly
    /// once, from `launch()`, and no source file may call it from a test, so
    /// a keychain prompt or a developer's exported variables can never reach
    /// `cargo test` (R7).
    #[test]
    fn only_launch_builds_the_settings_state_that_reads_the_real_keyring() {
        const NEEDLE: &str = "SettingsState::for_launch(";
        let calls = |src: &str| {
            src.lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !t.starts_with("//") && l.contains(NEEDLE)
                })
                .count()
        };
        let lib = include_str!("lib.rs");
        let lib_non_test = &lib[..lib.find("#[cfg(test)]\nmod tests").unwrap_or(lib.len())];
        assert_eq!(calls(lib_non_test), 1, "exactly one call site (launch)");
        // The panel module defines it and documents it; none of its own
        // tests (everything from `mod tests` down) may call it.
        let panel = include_str!("settings_panel.rs");
        let panel_tests = &panel[panel.find("#[cfg(test)]\nmod tests").expect("tests")..];
        assert_eq!(
            calls(panel_tests),
            0,
            "no settings test may read the real keyring"
        );
    }

    /// A fresh `LiveAgentLoop` with the default budget and no consent
    /// rejections — the shape `start_live_loop` builds for a
    /// bypassed/clean-dry-run task.
    fn fresh_live_loop() -> LiveAgentLoop {
        LiveAgentLoop::new(
            "go".to_string(),
            "go".to_string(),
            Default::default(),
            Default::default(),
        )
    }

    // ── consent_items: the plain-English summary snapshot ───────────────

    #[test]
    fn consent_summary_snapshot_for_a_mixed_diff() {
        let diff = mixed_diff();
        let expected = sample_expected();

        let items = consent_items(&diff, Some(&expected));
        assert_eq!(items.len(), 2, "{items:?}");

        assert_eq!(items[0].id, ToolId::new("js.execute"));
        assert_eq!(
            items[0].summary,
            "Used tool: js.execute — arbitrary JavaScript execution is always reviewed by \
             design (it can synthesize any other action); nothing in your request could have \
             authorized it."
        );

        assert_eq!(items[1].id, origin_item_id("https://attacker.example"));
        assert_eq!(
            items[1].summary,
            "Contacted https://attacker.example — this origin is not authorized. Your request \
             authorized: https://example.com."
        );
    }

    #[test]
    fn consent_summary_for_a_non_js_extra_primitive_says_nothing_authorized_it() {
        let mut diff = FingerprintDiff::default();
        diff.extra_primitives.insert(ToolId::new("dom.write"));
        let items = consent_items(&diff, None);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].summary,
            "Used tool: dom.write — nothing in your request authorized this action."
        );
    }

    #[test]
    fn consent_summary_for_an_out_of_scope_origin_with_no_expected_fingerprint_is_honest() {
        let diff = diff_with_out_of_scope_origin();
        let items = consent_items(&diff, None);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].summary,
            "Contacted https://attacker.example — this origin is not authorized. Your request \
             authorized: nothing in your request authorized any origin."
        );
    }

    #[test]
    fn consent_is_complete_requires_a_decision_on_both_bucket_kinds() {
        let diff = mixed_diff();
        let mut decision = ConsentDecision::default();
        assert!(!consent_is_complete(&diff, &decision));

        decision.reject(ToolId::new("js.execute"));
        assert!(
            !consent_is_complete(&diff, &decision),
            "the out-of-scope-origin item is still undecided"
        );

        decision.approve(origin_item_id("https://attacker.example"));
        assert!(consent_is_complete(&diff, &decision));
    }

    // ── update() state-machine tests ─────────────────────────────────────

    #[test]
    fn consent_required_populates_pending_state_fresh_and_clears_agent_running() {
        let mut state = FerriteBrowser {
            agent_is_running: true,
            ..FerriteBrowser::default()
        };
        let task = IpiTask::new("check my inbox", Some("https://example.com".to_string()));
        let diff = diff_with_extra_primitive();
        let expected = sample_expected();

        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff.clone(),
                expected: expected.clone(),
                evidence: Box::new(evidence_for(&task)),
            },
        );

        assert_eq!(state.pending_diff, Some(diff));
        assert_eq!(state.pending_expected, Some(expected));
        assert!(state.pending_evidence.is_some());
        assert!(state.pending_decision.approved.is_empty());
        assert!(state.pending_decision.rejected.is_empty());
        assert!(!state.agent_is_running);
    }

    #[tokio::test]
    async fn consent_submitted_is_a_no_op_while_any_item_is_undecided() {
        let mut state = FerriteBrowser::default();
        let task = IpiTask::new("t", None);
        state.pending_task = Some(task.prompt.clone());
        let diff = diff_with_extra_primitive();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff.clone(),
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task)),
            },
        );

        // Nothing decided yet — must be a no-op even though the message was
        // sent directly, bypassing the view's disabled Proceed button. This
        // is the defense-in-depth guard: ConsentSubmitted must never be
        // reachable with an undecided item, from any caller.
        let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);
        assert!(
            state.pending_diff.is_some(),
            "must not proceed while undecided"
        );
        assert!(state.pending_task.is_some());
        assert!(state.agent_handle.is_none());
    }

    #[tokio::test]
    async fn consent_flow_extra_primitive_only_reject_then_submit_clears_all_pending_state() {
        let mut state = FerriteBrowser::default();
        let task = IpiTask::new("t", None);
        state.pending_task = Some(task.prompt.clone());
        let diff = diff_with_extra_primitive();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff.clone(),
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task)),
            },
        );

        let _ = update(
            &mut state,
            FerriteBrowserMessage::RejectTool("js.execute".to_string()),
        );
        assert!(consent_is_complete(&diff, &state.pending_decision));

        let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);

        assert!(state.pending_diff.is_none());
        assert!(state.pending_expected.is_none());
        assert!(state.pending_evidence.is_none());
        assert!(state.pending_decision.approved.is_empty());
        assert!(state.pending_decision.rejected.is_empty());
        assert!(state.pending_task.is_none());
        assert!(!state.show_evidence);
        assert!(
            state.agent_handle.is_some(),
            "an approved-decision run must actually be spawned"
        );
    }

    #[tokio::test]
    async fn consent_flow_out_of_scope_origin_only_approve_then_submit() {
        let mut state = FerriteBrowser::default();
        let task = IpiTask::new("t", None);
        state.pending_task = Some(task.prompt.clone());
        let diff = diff_with_out_of_scope_origin();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff.clone(),
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task)),
            },
        );

        let item_id = origin_item_id("https://attacker.example").to_string();
        let _ = update(&mut state, FerriteBrowserMessage::ApproveTool(item_id));
        assert!(consent_is_complete(&diff, &state.pending_decision));

        let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);
        assert!(state.pending_diff.is_none());
        assert!(state.agent_handle.is_some());
    }

    #[tokio::test]
    async fn consent_flow_mixed_diff_requires_both_items_decided_before_submit_succeeds() {
        let mut state = FerriteBrowser::default();
        let task = IpiTask::new("t", None);
        state.pending_task = Some(task.prompt.clone());
        let diff = mixed_diff();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff.clone(),
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task)),
            },
        );

        let _ = update(
            &mut state,
            FerriteBrowserMessage::RejectTool("js.execute".to_string()),
        );
        let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);
        assert!(
            state.pending_diff.is_some(),
            "one undecided item (the out-of-scope origin) must still block submission"
        );

        let origin_id = origin_item_id("https://attacker.example").to_string();
        let _ = update(&mut state, FerriteBrowserMessage::RejectTool(origin_id));
        let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);
        assert!(state.pending_diff.is_none());
    }

    #[test]
    fn consent_cancelled_clears_all_pending_state_without_spawning_a_run() {
        let mut state = FerriteBrowser::default();
        let task = IpiTask::new("t", None);
        state.pending_task = Some(task.prompt.clone());
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff_with_extra_primitive(),
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task)),
            },
        );
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ApproveTool("js.execute".to_string()),
        );

        let _ = update(&mut state, FerriteBrowserMessage::ConsentCancelled);

        assert!(state.pending_diff.is_none());
        assert!(state.pending_expected.is_none());
        assert!(state.pending_evidence.is_none());
        assert!(state.pending_task.is_none());
        assert!(state.pending_decision.approved.is_empty());
        assert!(!state.show_evidence);
        assert!(!state.agent_is_running);
        assert!(
            state.agent_handle.is_none(),
            "cancel must never spawn a run"
        );
    }

    #[tokio::test]
    async fn second_tasks_consent_decision_never_carries_over_from_the_first() {
        let mut state = FerriteBrowser::default();

        // Task 1: flagged with js.execute, approved, submitted.
        let task1 = IpiTask::new("task one", None);
        state.pending_task = Some(task1.prompt.clone());
        let diff1 = diff_with_extra_primitive();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff1,
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task1)),
            },
        );
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ApproveTool("js.execute".to_string()),
        );
        let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);
        assert!(state.pending_decision.approved.is_empty());

        // Task 2: flagged with the SAME tool id, but never decided this time
        // — if approval leaked across tasks this would wrongly read as
        // already-decided.
        let task2 = IpiTask::new("task two", None);
        state.pending_task = Some(task2.prompt.clone());
        let diff2 = diff_with_extra_primitive();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ConsentRequired {
                diff: diff2.clone(),
                expected: sample_expected(),
                evidence: Box::new(evidence_for(&task2)),
            },
        );

        assert!(
            !consent_is_complete(&diff2, &state.pending_decision),
            "task 2's identical tool id must NOT be pre-approved from task 1's decision"
        );
        assert!(state.pending_decision.approved.is_empty());
        assert!(state.pending_decision.rejected.is_empty());
    }

    // ── is_action_rejected: the pure enforcement predicate ────────────────

    #[test]
    fn is_action_rejected_matches_on_tool_id() {
        let rejected: std::collections::HashSet<ToolId> =
            [ToolId::new("js.execute")].into_iter().collect();
        let action = AgentAction::JsExecute {
            script: "1+1".to_string(),
        };
        assert!(is_action_rejected(&action, &rejected, &Default::default()));
    }

    #[test]
    fn is_action_rejected_matches_on_navigate_origin() {
        let rejected_origins: std::collections::HashSet<String> =
            ["https://attacker.example".to_string()]
                .into_iter()
                .collect();
        let action = AgentAction::Navigate {
            url: "https://attacker.example/payload".to_string(),
        };
        assert!(is_action_rejected(
            &action,
            &Default::default(),
            &rejected_origins
        ));
    }

    #[test]
    fn is_action_rejected_is_false_for_an_undecided_action() {
        let action = AgentAction::ReadDom;
        assert!(!is_action_rejected(
            &action,
            &Default::default(),
            &Default::default()
        ));
    }

    // ── Real enforcement: a rejected live-loop step is actually blocked,
    // never reaching `execute_action`/the engine ──────────────────────────
    //
    // The automated suite has no real Servo session (R7 — see
    // `FerriteBrowser::default()`'s `servo_sessions: HashMap::new()`, never
    // populated outside a real `AddTab` against a real `servo` feature
    // build), so `AgentStepReady`'s handler always falls through to its own
    // "no active browser session" branch once past the rejection check.
    // These tests still prove real enforcement, not just the pure
    // predicate above: the handler's branch order means the observation
    // string can only say "blocked by user consent" if the rejection
    // branch fired *before* the no-session branch — "no active browser
    // session" is a structurally distinct string the handler would have
    // produced instead had the action not been rejected.

    #[tokio::test]
    async fn a_rejected_tool_id_blocks_the_action_before_it_reaches_the_engine() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let mut live = fresh_live_loop();
        live.rejected = [ToolId::new("js.execute")].into_iter().collect();
        state.live_loop = Some(live);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::JsExecute {
                    script: "1+1".to_string(),
                }),
            },
        );

        let live = state
            .live_loop
            .expect("the loop continues after a blocked, non-Finish action");
        let last = live.messages.last().expect("an observation was recorded");
        assert!(
            last.content.contains("blocked by user consent"),
            "rejected tool id must be blocked, not executed: {last:?}"
        );
    }

    #[tokio::test]
    async fn a_rejected_origin_blocks_a_navigate_before_it_reaches_the_engine() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let mut live = fresh_live_loop();
        live.rejected_origins = ["https://attacker.example".to_string()]
            .into_iter()
            .collect();
        state.live_loop = Some(live);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::Navigate {
                    url: "https://attacker.example/payload".to_string(),
                }),
            },
        );

        let live = state.live_loop.expect("the loop continues");
        let last = live.messages.last().expect("an observation was recorded");
        assert!(
            last.content.contains("blocked by user consent"),
            "rejected-origin navigate must be blocked, not executed: {last:?}"
        );
    }

    #[tokio::test]
    async fn a_non_rejected_action_reaches_the_no_session_fallback_not_the_consent_block() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::ReadDom),
            },
        );

        let live = state.live_loop.expect("the loop continues");
        let last = live.messages.last().expect("an observation was recorded");
        assert!(
            !last.content.contains("blocked by user consent"),
            "a non-rejected action must not be reported as consent-blocked: {last:?}"
        );
    }

    // ── C2: agent_log — real per-step icon/label/detail/result ────────────

    #[test]
    fn action_label_and_detail_are_distinct_per_action_kind() {
        assert_eq!(
            action_label(&AgentAction::Navigate {
                url: "https://a.example".to_string()
            }),
            "Navigate"
        );
        assert_eq!(
            action_detail(&AgentAction::Navigate {
                url: "https://a.example".to_string()
            }),
            "https://a.example"
        );
        assert_eq!(
            action_label(&AgentAction::Click {
                selector: "#go".to_string()
            }),
            "Click"
        );
        assert_eq!(
            action_detail(&AgentAction::Click {
                selector: "#go".to_string()
            }),
            "#go"
        );
        assert_eq!(
            action_detail(&AgentAction::TypeText {
                selector: "#q".to_string(),
                text: "hello".to_string(),
            }),
            "#q \u{2192} \"hello\""
        );
        assert_eq!(action_label(&AgentAction::WaitIdle), "Wait");
        assert_eq!(action_detail(&AgentAction::WaitIdle), "");
    }

    #[test]
    fn icon_for_action_groups_by_action_class_not_one_icon_per_variant() {
        // Navigate group shares one icon regardless of which navigation
        // variant fired.
        assert_eq!(
            icon_for_action(&AgentAction::Navigate { url: String::new() }),
            icon_for_action(&AgentAction::GoBack)
        );
        assert_eq!(
            icon_for_action(&AgentAction::GoBack),
            icon_for_action(&AgentAction::Reload)
        );
        // But a genuinely different class gets a different icon.
        assert_ne!(
            icon_for_action(&AgentAction::GoBack),
            icon_for_action(&AgentAction::Click {
                selector: String::new()
            })
        );
        // Finish/JsExecute deliberately reuse an existing chrome icon
        // rather than a dedicated one — pinned so that reuse stays
        // intentional, not silently regressed to something else later.
        assert_eq!(
            icon_for_action(&AgentAction::Finish {
                answer: String::new()
            }),
            Icon::Approve
        );
        assert_eq!(
            icon_for_action(&AgentAction::JsExecute {
                script: String::new()
            }),
            Icon::Console
        );
    }

    #[tokio::test]
    async fn agent_step_ready_records_a_real_step_with_its_actual_result() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::Click {
                    selector: "#submit".to_string(),
                }),
            },
        );

        assert_eq!(state.agent_log.len(), 1);
        match &state.agent_log[0] {
            AgentLogEntry::Step {
                icon: step_icon,
                label,
                detail,
                result,
                blocked,
                fast,
            } => {
                assert!(!fast, "an LLM-chosen step is not a fast-lane step");
                assert_eq!(*step_icon, Icon::Click);
                assert_eq!(*label, "Click");
                assert_eq!(detail, "#submit");
                assert!(!blocked);
                // No active Servo session in this test fixture (R7 — no
                // real session is ever constructed outside launch()), so
                // the real fallback string, not a fabricated one, must
                // land in the step's own result field.
                assert_eq!(result, "error: no active browser session");
            }
            other => panic!("expected a Step entry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_step_ready_records_a_blocked_step_as_blocked_not_a_silent_success() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let mut live = fresh_live_loop();
        live.rejected = [ToolId::new("js.execute")].into_iter().collect();
        state.live_loop = Some(live);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::JsExecute {
                    script: "1+1".to_string(),
                }),
            },
        );

        assert_eq!(state.agent_log.len(), 1);
        match &state.agent_log[0] {
            AgentLogEntry::Step {
                blocked, result, ..
            } => {
                assert!(
                    *blocked,
                    "a rejected action's step must record blocked = true"
                );
                assert_eq!(result, "blocked by user consent");
            }
            other => panic!("expected a Step entry, got {other:?}"),
        }
    }

    // ── ServoFrame: resize-settle window (real segfault/corruption fix) ───

    #[test]
    fn servo_frame_decrements_resize_settle_ticks_and_stops_at_zero() {
        let mut state = FerriteBrowser {
            resize_settle_ticks: 2,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);
        assert_eq!(state.resize_settle_ticks, 1);
        let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);
        assert_eq!(state.resize_settle_ticks, 0);
        let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);
        assert_eq!(
            state.resize_settle_ticks, 0,
            "must not underflow past zero on a later tick"
        );
    }

    #[test]
    fn servo_frame_does_not_arm_the_resize_settle_window_without_a_session_to_resize() {
        // R7: no HeadlessServoSession is ever constructed in this test
        // build (the stub's own `new()` always returns `Err`), so
        // `servo_sessions` is always empty here — this proves the settle
        // window only ever arms as a *result* of an actual `resize()`
        // call, not speculatively just because the measured content area
        // differs from `last_resized_content_px`.
        let mut state = FerriteBrowser {
            content_area_size: Cell::new(Size::new(999.0, 999.0)),
            ..FerriteBrowser::default()
        };
        assert_eq!(state.last_resized_content_px, (1280, 700));

        let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);

        assert_eq!(
            state.resize_settle_ticks, 0,
            "no session existed to resize, so nothing should have armed the settle window"
        );
        assert_eq!(
            state.last_resized_content_px,
            (1280, 700),
            "must not be updated unless a resize() call actually happened"
        );
    }

    // ── AgentStepReady/LiveRunReady: message-driven step-loop plumbing ────

    #[test]
    fn a_stale_run_id_is_ignored_by_live_run_ready() {
        let mut state = FerriteBrowser {
            run_id: 2,
            ..FerriteBrowser::default()
        };
        let _ = update(
            &mut state,
            FerriteBrowserMessage::LiveRunReady {
                run_id: 1,
                prompt: "go".to_string(),
                guard: None,
            },
        );
        assert!(
            state.live_loop.is_none(),
            "a stale run_id must never start a live loop"
        );
        assert!(!state.agent_is_running);
    }

    #[test]
    fn a_stale_run_id_is_ignored_by_agent_step_ready() {
        let mut state = FerriteBrowser {
            run_id: 2,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());
        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::Finish {
                    answer: "should never apply".to_string(),
                }),
            },
        );
        assert!(
            state.live_loop.is_some(),
            "a stale run_id's step must not be allowed to finish the current loop"
        );
        assert!(state.agent_response.is_none());
    }

    #[tokio::test]
    async fn live_run_ready_with_a_current_run_id_starts_the_loop() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let _ = update(
            &mut state,
            FerriteBrowserMessage::LiveRunReady {
                run_id: 1,
                prompt: "go".to_string(),
                guard: None,
            },
        );
        assert!(state.live_loop.is_some());
        assert!(state.agent_is_running);
    }

    #[test]
    fn agent_step_ready_with_finish_completes_the_task() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());
        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::Finish {
                    answer: "done".to_string(),
                }),
            },
        );
        assert_eq!(state.agent_response.as_deref(), Some("done"));
        assert!(!state.agent_is_running);
        assert!(state.live_loop.is_none());
    }

    #[test]
    fn agent_step_ready_with_a_model_error_fails_the_task() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());
        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Err(StepFailure::Model("no provider configured".to_string())),
            },
        );
        assert_eq!(
            state.agent_response.as_deref(),
            Some("[error] no provider configured")
        );
        assert!(!state.agent_is_running);
    }

    #[tokio::test]
    async fn agent_step_ready_with_a_malformed_action_retries_instead_of_failing_immediately() {
        // Real, reproduced failure mode: a `finish.answer` truncated
        // mid-string by the provider's output cap comes back as
        // unparseable JSON. A single such glitch must not end the whole
        // task the way it did before this fix — it must be fed back to
        // the model as a retry, with the run still alive afterward.
        let mut state = FerriteBrowser {
            run_id: 1,
            agent_is_running: true,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());
        let messages_before = state.live_loop.as_ref().unwrap().messages.len();

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Err(StepFailure::Malformed {
                    raw: r#"{"action":"finish","answer":"This page is a"#.to_string(),
                    message: "EOF while parsing a string at line 1 column 44".to_string(),
                }),
            },
        );

        assert!(
            state.agent_is_running,
            "a single malformed response must not end the run while retries remain"
        );
        assert!(
            state.agent_response.is_none(),
            "no raw parser error should ever reach the user while a retry is still possible"
        );
        let live = state
            .live_loop
            .expect("the loop must still be alive, retrying the step");
        assert_eq!(live.consecutive_malformed, 1);
        assert_eq!(
            live.messages.len(),
            messages_before + 2,
            "a retry-with-feedback pair (the raw response, then a request to retry) must \
             have been appended to the conversation"
        );
    }

    #[test]
    fn agent_step_ready_gives_up_after_max_consecutive_malformed_responses() {
        // A model stuck producing garbage must still fail fast rather than
        // retry forever — bounded by MAX_CONSECUTIVE_MALFORMED_STEPS, not
        // by the step budget (a retry never counts as a step).
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let mut live = fresh_live_loop();
        live.consecutive_malformed = MAX_CONSECUTIVE_MALFORMED_STEPS;
        state.live_loop = Some(live);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Err(StepFailure::Malformed {
                    raw: "still garbage".to_string(),
                    message: "EOF while parsing a string".to_string(),
                }),
            },
        );

        assert!(
            state
                .agent_response
                .as_deref()
                .unwrap_or_default()
                .contains("EOF while parsing a string"),
            "the real parse error must surface once retries are exhausted: {:?}",
            state.agent_response
        );
        assert!(!state.agent_is_running);
        assert!(state.live_loop.is_none());
    }

    #[test]
    fn repeated_identical_actions_stop_the_live_loop_before_a_third_execution() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let action = AgentAction::Click {
            selector: "#retry".to_string(),
        };
        let mut live = fresh_live_loop();
        live.actions_taken = vec![action.clone(), action.clone()];
        live.budget.max_repeated_identical = 3;
        state.live_loop = Some(live);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(action),
            },
        );

        assert!(
            state.live_loop.is_none(),
            "the loop must stop, not repeat a third time"
        );
        assert!(!state.agent_is_running);
        assert!(state
            .agent_response
            .as_deref()
            .unwrap_or_default()
            .contains("repeat"));
    }

    #[tokio::test]
    async fn step_budget_exhausted_stops_the_loop_instead_of_spawning_another_step() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        let mut live = fresh_live_loop();
        live.budget.max_steps = 1;
        state.live_loop = Some(live);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::AgentStepReady {
                run_id: 1,
                action: Ok(AgentAction::ReadDom),
            },
        );

        assert!(
            state.live_loop.is_none(),
            "the loop must stop once its one allowed step has executed"
        );
        assert!(!state.agent_is_running);
        assert!(state
            .agent_response
            .as_deref()
            .unwrap_or_default()
            .contains("step budget"));
    }

    #[test]
    fn stop_agent_bumps_run_id_and_clears_the_live_loop() {
        let mut state = FerriteBrowser {
            run_id: 1,
            ..FerriteBrowser::default()
        };
        state.live_loop = Some(fresh_live_loop());
        state.agent_is_running = true;

        let _ = update(&mut state, FerriteBrowserMessage::StopAgent);

        assert_eq!(state.run_id, 2);
        assert!(state.live_loop.is_none());
        assert!(!state.agent_is_running);
    }

    // ── Dry-run evidence rendering ────────────────────────────────────────

    #[test]
    fn dry_run_evidence_lines_render_ordered_call_log() {
        let task = IpiTask::new("t", None);
        let mut record = evidence_for(&task);
        record.record_tool(
            ferrite_core::Primitive::Navigate,
            Some("https://example.com".to_string()),
        );
        record.record_tool(
            ferrite_core::Primitive::DomRead,
            Some("https://example.com".to_string()),
        );
        record.record_tool(ferrite_core::Primitive::JsExecute, None);

        let lines = dry_run_evidence_lines(&record);
        assert_eq!(
            lines,
            vec![
                "#0  navigate  https://example.com",
                "#1  dom.read  https://example.com",
                "#2  js.execute  (no origin recorded)",
            ]
        );
    }

    #[test]
    fn dry_run_evidence_lines_says_so_honestly_when_nothing_was_recorded() {
        let task = IpiTask::new("t", None);
        let record = evidence_for(&task);
        assert_eq!(
            dry_run_evidence_lines(&record),
            vec!["No tool calls were recorded during the dry run."]
        );
    }

    // ── ease_out_cubic: the consent panel's entrance-transition curve ────

    #[test]
    fn ease_out_cubic_starts_at_zero_and_ends_at_one() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
    }

    #[test]
    fn ease_out_cubic_clamps_out_of_range_input() {
        assert_eq!(ease_out_cubic(-1.0), 0.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);
    }

    #[test]
    fn ease_out_cubic_is_monotonically_non_decreasing() {
        let mut prev = ease_out_cubic(0.0);
        let mut t = 0.0_f32;
        while t <= 1.0 {
            let cur = ease_out_cubic(t);
            assert!(cur >= prev, "not monotonic at t={t}: {cur} < {prev}");
            prev = cur;
            t += 0.05;
        }
    }

    #[test]
    fn ease_out_cubic_is_ahead_of_linear_partway_through_an_ease_out_curve() {
        // The defining property of ease-*out*: it front-loads progress, so
        // at the midpoint it's already past halfway (unlike a linear or
        // ease-in curve).
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    // ── C3c loading-indicator helpers ─────────────────────────────────────

    #[test]
    fn pulse_alpha_stays_within_base_and_base_plus_amplitude() {
        let mut t = 0.0_f32;
        while t <= 1.0 {
            let a = pulse_alpha(t, 1.7, 0.3, 0.5);
            assert!((0.3..=0.8 + f32::EPSILON).contains(&a), "t={t}: alpha={a}");
            t += 0.03;
        }
    }

    #[test]
    fn pulse_alpha_at_zero_is_exactly_base() {
        // sin(0) == 0, so the breath starts at its dimmest — matches the
        // "no light yet" reading at the very start of a loading state.
        assert_eq!(pulse_alpha(0.0, 1.0, 0.25, 0.20), 0.25);
    }

    // ── C3c theme toggle ───────────────────────────────────────────────────

    #[test]
    fn default_theme_is_dark_matching_pre_c3c_behavior() {
        let state = FerriteBrowser::default();
        assert_eq!(state.theme_mode, AppTheme::Dark);
    }

    #[test]
    fn toggle_theme_flips_dark_to_light_and_back() {
        let mut state = FerriteBrowser::default();
        let _ = update(&mut state, FerriteBrowserMessage::ToggleTheme);
        assert_eq!(state.theme_mode, AppTheme::Light);
        let _ = update(&mut state, FerriteBrowserMessage::ToggleTheme);
        assert_eq!(state.theme_mode, AppTheme::Dark);
    }

    #[test]
    fn each_theme_resolves_to_its_own_distinct_palette() {
        let mut state = FerriteBrowser::default();
        let dark_accent = state.palette().accent;
        let _ = update(&mut state, FerriteBrowserMessage::ToggleTheme);
        let light_accent = state.palette().accent;
        assert_ne!(
            (dark_accent.r, dark_accent.g, dark_accent.b),
            (light_accent.r, light_accent.g, light_accent.b),
            "toggling theme did not change the resolved accent colour"
        );
    }

    #[test]
    fn light_and_dark_palettes_each_keep_text_readable_on_base() {
        // Not a full contrast-ratio check (see `LIGHT_PALETTE`'s doc
        // comment for the hand-computed figures) — just the structural
        // property a palette must have to be usable at all: primary text
        // is not the same colour as the background it sits on, in either
        // theme.
        for palette in [&DARK_PALETTE, &LIGHT_PALETTE] {
            assert_ne!(
                (palette.text.r, palette.text.g, palette.text.b),
                (palette.base.r, palette.base.g, palette.base.b)
            );
        }
    }

    #[test]
    fn to_iced_theme_matches_app_theme() {
        assert_eq!(AppTheme::Dark.to_iced_theme(), Theme::Dark);
        assert_eq!(AppTheme::Light.to_iced_theme(), Theme::Light);
    }

    #[test]
    fn palette_for_theme_falls_back_to_dark_for_a_non_light_iced_theme() {
        // `palette_for_theme` is what the nine `.style(...)`-callback
        // functions use; it must resolve every `Theme` variant to
        // *something*; anything other than `Theme::Light` should be the
        // dark palette (see that function's own doc comment).
        let p = palette_for_theme(&Theme::Dark);
        assert_eq!(
            (p.base.r, p.base.g, p.base.b),
            (
                DARK_PALETTE.base.r,
                DARK_PALETTE.base.g,
                DARK_PALETTE.base.b
            )
        );
    }

    // ── Page-content decoupling: structural argument, not merely asserted ─

    /// This is not a runtime assertion so much as a compile-time witness:
    /// `view()`'s signature takes `&FerriteBrowser` and returns
    /// `Element<'_, FerriteBrowserMessage>` built exclusively from
    /// `iced_widget` constructors over plain Rust data
    /// (`String`/`FingerprintDiff`/`ExpectedFingerprint`/`DryRunRecord`).
    /// Page content never appears in any of those types — the only place
    /// Servo's page content enters this crate at all is
    /// `HeadlessServoSession::get_frame()`, which returns a decoded
    /// `(width, height, Vec<u8> RGBA bytes)` pixel buffer (see the `content`
    /// arm of `view()` for the one call site), rendered via
    /// `iced_widget::image::Image` — a bitmap, not markup Iced parses for
    /// style/position/layout. There is therefore no code path by which a
    /// page's HTML/CSS/text could set a style, position, or z-order on the
    /// consent panel: nothing in `FingerprintDiff`, `ExpectedFingerprint`,
    /// `DryRunRecord`, or `ConsentDecision` is ever populated from raw page
    /// markup, and even the one place page bytes DO reach this crate
    /// (`get_frame()`) they arrive as opaque pixels, never as a string Iced's
    /// widget tree would interpret. This test exists so that claim is
    /// pinned to a real function signature rather than left as a comment
    /// someone could invalidate without any test noticing.
    #[test]
    fn page_content_cannot_reach_the_consent_panels_inputs() {
        fn assert_view_signature(_f: fn(&FerriteBrowser) -> Element<'_, FerriteBrowserMessage>) {}
        assert_view_signature(view);

        // The only types a pending consent decision is built from — none of
        // them is, or contains, raw page markup.
        fn assert_consent_panel_inputs_are_plain_data(
            _diff: &FingerprintDiff,
            _expected: &ExpectedFingerprint,
            _evidence: &DryRunRecord,
            _decision: &ConsentDecision,
        ) {
        }
        let diff = FingerprintDiff::default();
        let expected = ExpectedFingerprint::empty();
        let task = IpiTask::new("t", None);
        let evidence = evidence_for(&task);
        let decision = ConsentDecision::default();
        assert_consent_panel_inputs_are_plain_data(&diff, &expected, &evidence, &decision);
    }

    // ── C3d: bookmarks ───────────────────────────────────────────────────

    /// A process-unique path under the real temp dir — real filesystem I/O
    /// (not the network, so R7 is unaffected, same as
    /// `RefreshAuditLog`/`HeadlessServoSession::new`'s own use of
    /// `std::env::temp_dir()` elsewhere in this workspace), but never a real
    /// `$HOME` path, matching `load_bookmarks_from`/`save_bookmarks_to`'s
    /// whole reason for taking an explicit `&Path`.
    fn temp_test_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ferrite_ui_test_{name}_{nanos}.json"))
    }

    #[test]
    fn load_bookmarks_from_a_missing_file_is_an_empty_list_not_an_error() {
        let path = temp_test_path("missing_bookmarks");
        assert_eq!(load_bookmarks_from(&path), Vec::new());
    }

    #[test]
    fn load_bookmarks_from_unparseable_json_is_an_empty_list_not_a_panic() {
        let path = temp_test_path("corrupt_bookmarks");
        std::fs::write(&path, b"not valid json").unwrap();
        assert_eq!(load_bookmarks_from(&path), Vec::new());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_then_load_bookmarks_round_trips() {
        let path = temp_test_path("round_trip_bookmarks");
        let bookmarks = vec![
            Bookmark {
                title: "Example".to_string(),
                url: "https://example.com".to_string(),
            },
            Bookmark {
                title: "Rust".to_string(),
                url: "https://rust-lang.org".to_string(),
            },
        ];
        save_bookmarks_to(&path, &bookmarks).unwrap();
        assert_eq!(load_bookmarks_from(&path), bookmarks);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_bookmarks_to_creates_missing_parent_directories() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ferrite_ui_test_bm_dir_{nanos}"));
        let path = dir.join("nested").join("bookmarks.json");
        assert!(!dir.exists());
        save_bookmarks_to(&path, &[]).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_bookmark_current_page_adds_then_removes_it() {
        let path = temp_test_path("toggle_bookmark");
        let mut state = FerriteBrowser {
            bookmarks_path: Some(path.clone()),
            tab_urls: vec!["https://example.com".to_string()],
            tab_titles: vec!["Example Site".to_string()],
            ..FerriteBrowser::default()
        };

        let _ = update(&mut state, FerriteBrowserMessage::ToggleBookmarkCurrentPage);
        assert_eq!(state.bookmarks.len(), 1);
        assert_eq!(state.bookmarks[0].url, "https://example.com");
        assert_eq!(state.bookmarks[0].title, "Example Site");
        // Persisted for real, since bookmarks_path is set in this test.
        assert_eq!(load_bookmarks_from(&path).len(), 1);

        let _ = update(&mut state, FerriteBrowserMessage::ToggleBookmarkCurrentPage);
        assert!(state.bookmarks.is_empty());
        assert!(load_bookmarks_from(&path).is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn toggle_bookmark_current_page_is_a_no_op_for_about_blank() {
        let mut state = FerriteBrowser::default();
        let _ = update(&mut state, FerriteBrowserMessage::ToggleBookmarkCurrentPage);
        assert!(state.bookmarks.is_empty());
    }

    #[test]
    fn remove_bookmark_out_of_bounds_is_a_no_op_not_a_panic() {
        let mut state = FerriteBrowser::default();
        let _ = update(&mut state, FerriteBrowserMessage::RemoveBookmark(0));
        assert!(state.bookmarks.is_empty());
    }

    #[test]
    fn remove_bookmark_by_index_removes_exactly_that_one() {
        let mut state = FerriteBrowser {
            bookmarks: vec![
                Bookmark {
                    title: "A".to_string(),
                    url: "https://a.example".to_string(),
                },
                Bookmark {
                    title: "B".to_string(),
                    url: "https://b.example".to_string(),
                },
            ],
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::RemoveBookmark(0));
        assert_eq!(state.bookmarks.len(), 1);
        assert_eq!(state.bookmarks[0].url, "https://b.example");
    }

    // ── C3d: history ─────────────────────────────────────────────────────

    #[test]
    fn record_history_visit_dedups_only_against_the_immediately_preceding_entry() {
        let mut history = Vec::new();
        record_history_visit(
            &mut history,
            "https://a.example".to_string(),
            "A".to_string(),
        );
        record_history_visit(
            &mut history,
            "https://a.example".to_string(),
            "A".to_string(),
        );
        assert_eq!(history.len(), 1, "consecutive repeat should be deduped");

        record_history_visit(
            &mut history,
            "https://b.example".to_string(),
            "B".to_string(),
        );
        record_history_visit(
            &mut history,
            "https://a.example".to_string(),
            "A".to_string(),
        );
        assert_eq!(
            history.len(),
            3,
            "revisiting a.example after browsing elsewhere is a real, distinct visit"
        );
    }

    #[test]
    fn load_status_changed_records_a_completed_real_navigation_in_history() {
        let mut state = FerriteBrowser::default();
        assert!(state.history.is_empty());
        let _ = update(
            &mut state,
            FerriteBrowserMessage::LoadStatusChanged {
                tab: 0,
                status: "complete".to_string(),
                url: "https://example.com".to_string(),
            },
        );
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.history[0].url, "https://example.com");
    }

    #[test]
    fn load_status_changed_does_not_record_loading_events_or_about_blank() {
        let mut state = FerriteBrowser::default();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::LoadStatusChanged {
                tab: 0,
                status: "loading".to_string(),
                url: "https://example.com".to_string(),
            },
        );
        assert!(
            state.history.is_empty(),
            "a loading event is not a completed visit"
        );

        let _ = update(
            &mut state,
            FerriteBrowserMessage::LoadStatusChanged {
                tab: 0,
                status: "complete".to_string(),
                url: "about:blank".to_string(),
            },
        );
        assert!(
            state.history.is_empty(),
            "about:blank is never a real visit"
        );
    }

    #[test]
    fn clear_history_empties_the_list() {
        let mut state = FerriteBrowser {
            history: vec![HistoryEntry {
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                visited_at: chrono::Utc::now(),
            }],
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::ClearHistory);
        assert!(state.history.is_empty());
    }

    // ── C3d: zoom ─────────────────────────────────────────────────────────

    #[test]
    fn next_zoom_level_steps_up_through_the_table() {
        assert_eq!(next_zoom_level(1.0), 1.1);
        assert_eq!(next_zoom_level(0.9), 1.0);
    }

    #[test]
    fn next_zoom_level_clamps_at_the_top_of_the_range() {
        assert_eq!(next_zoom_level(3.0), 3.0);
        assert_eq!(next_zoom_level(10.0), 3.0);
    }

    #[test]
    fn prev_zoom_level_steps_down_through_the_table() {
        assert_eq!(prev_zoom_level(1.0), 0.9);
        assert_eq!(prev_zoom_level(1.1), 1.0);
    }

    #[test]
    fn prev_zoom_level_clamps_at_the_bottom_of_the_range() {
        assert_eq!(prev_zoom_level(0.5), 0.5);
        assert_eq!(prev_zoom_level(0.1), 0.5);
    }

    #[test]
    fn zoom_in_and_out_change_the_active_tabs_zoom_only() {
        let mut state = FerriteBrowser {
            tabs: vec!["A".to_string(), "B".to_string()],
            tab_zoom: vec![1.0, 1.0],
            active_tab: 0,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::ZoomIn);
        assert_eq!(state.tab_zoom[0], 1.1);
        assert_eq!(
            state.tab_zoom[1], 1.0,
            "the inactive tab's zoom is untouched"
        );

        let _ = update(&mut state, FerriteBrowserMessage::ZoomOut);
        assert_eq!(state.tab_zoom[0], 1.0);

        state.tab_zoom[0] = 1.5;
        let _ = update(&mut state, FerriteBrowserMessage::ZoomReset);
        assert_eq!(state.tab_zoom[0], 1.0);
    }

    #[test]
    fn set_default_zoom_affects_only_tabs_added_after_it() {
        let mut state = FerriteBrowser::default();
        assert_eq!(state.tab_zoom, vec![1.0]);
        let _ = update(&mut state, FerriteBrowserMessage::SetDefaultZoom(1.25));
        assert_eq!(state.tab_zoom, vec![1.0], "the existing tab is unaffected");
        let _ = update(&mut state, FerriteBrowserMessage::AddTab);
        assert_eq!(state.tab_zoom.last().copied(), Some(1.25));
    }

    #[test]
    fn add_tab_and_close_tab_keep_tab_zoom_in_sync() {
        let mut state = FerriteBrowser::default();
        let _ = update(&mut state, FerriteBrowserMessage::AddTab);
        let _ = update(&mut state, FerriteBrowserMessage::AddTab);
        assert_eq!(state.tab_zoom.len(), state.tabs.len());
        let _ = update(&mut state, FerriteBrowserMessage::CloseTab(0));
        assert_eq!(state.tab_zoom.len(), state.tabs.len());
    }

    // ── C3d: find-in-page ────────────────────────────────────────────────

    #[test]
    fn find_script_json_escapes_the_query() {
        let script = find_script("a \"quoted\" term");
        assert!(script.contains(r#""a \"quoted\" term""#), "{script}");
        assert!(!script.contains("window.find("));
        assert!(script.contains("createTreeWalker"));
    }

    #[test]
    fn find_script_for_an_empty_query_short_circuits_to_zero() {
        let script = find_script("");
        assert!(script.contains("count:0,current:0"));
    }

    #[test]
    fn find_navigate_script_steps_forward_or_backward() {
        assert!(find_navigate_script(true).contains("(curIdx + (1)"));
        assert!(find_navigate_script(false).contains("(curIdx + (-1)"));
    }

    #[test]
    fn extract_json_object_parses_a_direct_json_string() {
        let value = extract_json_object(r#"{"count":3,"current":1}"#).unwrap();
        assert_eq!(value["count"], 3);
        assert_eq!(value["current"], 1);
    }

    #[test]
    fn extract_json_object_parses_json_wrapped_in_an_unknown_debug_format() {
        // Simulates `execute_js` handing back a `Debug`-formatted enum
        // wrapper around the JS string value, e.g. `String("{...}")` — see
        // this function's own doc comment for why this defensiveness
        // exists at all.
        let value = extract_json_object(r#"String("{\"count\":2,\"current\":2}")"#).unwrap();
        assert_eq!(value["count"], 2);
        assert_eq!(value["current"], 2);
    }

    #[test]
    fn extract_json_object_returns_none_for_text_with_no_braces() {
        assert!(extract_json_object("no json here").is_none());
    }

    #[test]
    fn apply_find_result_updates_match_count_and_current_index() {
        let mut state = FerriteBrowser::default();
        apply_find_result(&mut state, r#"{"count":5,"current":3}"#);
        assert_eq!(state.find_match_count, 5);
        assert_eq!(state.find_current_index, 3);
    }

    #[test]
    fn apply_find_result_leaves_counts_unchanged_on_unparseable_input() {
        let mut state = FerriteBrowser {
            find_match_count: 7,
            find_current_index: 2,
            ..FerriteBrowser::default()
        };
        apply_find_result(&mut state, "garbage");
        assert_eq!(state.find_match_count, 7);
        assert_eq!(state.find_current_index, 2);
    }

    #[test]
    fn open_find_bar_resets_query_and_counts_and_shows_the_bar() {
        let mut state = FerriteBrowser {
            find_query: "stale".to_string(),
            find_match_count: 4,
            find_current_index: 2,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::OpenFindBar);
        assert!(state.show_find_bar);
        assert!(state.find_query.is_empty());
        assert_eq!(state.find_match_count, 0);
        assert_eq!(state.find_current_index, 0);
    }

    #[test]
    fn close_find_bar_hides_it_and_clears_counts() {
        let mut state = FerriteBrowser {
            show_find_bar: true,
            find_query: "x".to_string(),
            find_match_count: 2,
            find_current_index: 1,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::CloseFindBar);
        assert!(!state.show_find_bar);
        assert!(state.find_query.is_empty());
        assert_eq!(state.find_match_count, 0);
    }

    #[test]
    fn find_query_changed_to_empty_clears_counts_even_with_no_session() {
        let mut state = FerriteBrowser {
            find_match_count: 3,
            find_current_index: 1,
            ..FerriteBrowser::default()
        };
        let _ = update(
            &mut state,
            FerriteBrowserMessage::FindQueryChanged(String::new()),
        );
        assert_eq!(state.find_match_count, 0);
        assert_eq!(state.find_current_index, 0);
    }

    // ── C3d: downloads ───────────────────────────────────────────────────

    #[test]
    fn download_file_name_uses_the_last_path_segment() {
        assert_eq!(
            download_file_name("https://example.com/files/report.pdf"),
            "report.pdf"
        );
    }

    #[test]
    fn download_file_name_falls_back_for_a_trailing_slash_or_unparseable_url() {
        assert_eq!(download_file_name("https://example.com/"), "download");
        assert_eq!(download_file_name("not a url"), "download");
    }

    #[test]
    fn resolve_download_path_uses_the_bare_name_when_theres_no_collision() {
        let dir = PathBuf::from("/tmp/downloads");
        let path = resolve_download_path(&dir, "https://example.com/report.pdf", &[]);
        assert_eq!(path, dir.join("report.pdf"));
    }

    #[test]
    fn resolve_download_path_avoids_colliding_with_an_existing_download() {
        let dir = PathBuf::from("/tmp/downloads");
        let existing = vec![DownloadItem {
            id: 1,
            url: "https://example.com/report.pdf".to_string(),
            file_name: "report.pdf".to_string(),
            path: dir.join("report.pdf"),
            state: DownloadState::Completed,
        }];
        let path = resolve_download_path(&dir, "https://example.com/report.pdf", &existing);
        assert_eq!(path, dir.join("report (1).pdf"));
    }

    #[test]
    fn resolve_download_path_keeps_stepping_past_multiple_collisions() {
        let dir = PathBuf::from("/tmp/downloads");
        let existing = vec![
            DownloadItem {
                id: 1,
                url: "u1".to_string(),
                file_name: "report.pdf".to_string(),
                path: dir.join("report.pdf"),
                state: DownloadState::Completed,
            },
            DownloadItem {
                id: 2,
                url: "u2".to_string(),
                file_name: "report (1).pdf".to_string(),
                path: dir.join("report (1).pdf"),
                state: DownloadState::Completed,
            },
        ];
        let path = resolve_download_path(&dir, "https://example.com/report.pdf", &existing);
        assert_eq!(path, dir.join("report (2).pdf"));
    }

    #[test]
    fn resolve_download_path_handles_an_extensionless_name() {
        let dir = PathBuf::from("/tmp/downloads");
        let existing = vec![DownloadItem {
            id: 1,
            url: "u".to_string(),
            file_name: "download".to_string(),
            path: dir.join("download"),
            state: DownloadState::Completed,
        }];
        let path = resolve_download_path(&dir, "https://example.com/", &existing);
        assert_eq!(path, dir.join("download (1)"));
    }

    #[test]
    fn format_bytes_scales_through_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[tokio::test]
    async fn download_current_page_is_a_no_op_without_a_configured_downloads_dir() {
        let mut state = FerriteBrowser {
            tab_urls: vec!["https://example.com/file.txt".to_string()],
            downloads_dir: None,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::DownloadCurrentPage);
        assert!(state.downloads.is_empty());
    }

    #[tokio::test]
    async fn download_current_page_is_a_no_op_for_about_blank() {
        let mut state = FerriteBrowser {
            downloads_dir: Some(PathBuf::from("/tmp")),
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::DownloadCurrentPage);
        assert!(state.downloads.is_empty());
    }

    #[tokio::test]
    async fn download_current_page_records_an_in_progress_item_and_spawns_the_fetch() {
        // R7: this handler spawns a real background `reqwest` task, but —
        // per this test module's own header comment — a `#[tokio::test]`
        // that never `.await`s past the point of spawning never actually
        // polls it into making a live call; only the synchronous, in-
        // `update()` bookkeeping below is exercised.
        let mut state = FerriteBrowser {
            tab_urls: vec!["https://example.com/report.pdf".to_string()],
            downloads_dir: Some(PathBuf::from("/tmp/ferrite_downloads_test")),
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::DownloadCurrentPage);
        assert_eq!(state.downloads.len(), 1);
        assert_eq!(state.downloads[0].id, 0);
        assert_eq!(state.downloads[0].file_name, "report.pdf");
        assert_eq!(state.next_download_id, 1);
        assert!(matches!(
            state.downloads[0].state,
            DownloadState::InProgress {
                downloaded_bytes: 0,
                total_bytes: None
            }
        ));
    }

    #[test]
    fn download_progress_updates_the_matching_item_by_id() {
        let mut state = FerriteBrowser {
            downloads: vec![DownloadItem {
                id: 42,
                url: "u".to_string(),
                file_name: "f".to_string(),
                path: PathBuf::from("/tmp/f"),
                state: DownloadState::InProgress {
                    downloaded_bytes: 0,
                    total_bytes: None,
                },
            }],
            ..FerriteBrowser::default()
        };
        let _ = update(
            &mut state,
            FerriteBrowserMessage::DownloadProgress {
                id: 42,
                downloaded_bytes: 100,
                total_bytes: Some(200),
            },
        );
        assert!(matches!(
            state.downloads[0].state,
            DownloadState::InProgress {
                downloaded_bytes: 100,
                total_bytes: Some(200)
            }
        ));
    }

    #[test]
    fn download_completed_and_failed_update_the_matching_item() {
        let mut state = FerriteBrowser {
            downloads: vec![DownloadItem {
                id: 1,
                url: "u".to_string(),
                file_name: "f".to_string(),
                path: PathBuf::from("/tmp/f"),
                state: DownloadState::InProgress {
                    downloaded_bytes: 0,
                    total_bytes: None,
                },
            }],
            ..FerriteBrowser::default()
        };
        let _ = update(
            &mut state,
            FerriteBrowserMessage::DownloadCompleted { id: 1 },
        );
        assert_eq!(state.downloads[0].state, DownloadState::Completed);

        let _ = update(
            &mut state,
            FerriteBrowserMessage::DownloadFailed {
                id: 1,
                error: "boom".to_string(),
            },
        );
        assert_eq!(
            state.downloads[0].state,
            DownloadState::Failed("boom".to_string())
        );
    }

    // ── New-tab hero: quick-access tile favicons ────────────────────────────

    #[test]
    fn favicon_host_extracts_the_lowercased_host_from_a_tile_url() {
        assert_eq!(
            favicon_host("https://GitHub.com"),
            Some("github.com".to_string())
        );
        assert_eq!(
            favicon_host("https://lite.duckduckgo.com/lite"),
            Some("lite.duckduckgo.com".to_string())
        );
    }

    #[test]
    fn favicon_host_is_none_for_an_unparseable_url() {
        assert_eq!(favicon_host("not a url"), None);
    }

    /// Every `QUICK_ACCESS_TILES` entry's `url` resolves to a real host —
    /// the exact `favicon_host` call `FetchTileFavicons`'s handler and
    /// `fetch_tile_favicon` both make, kept passing so a future edit to the
    /// tile list can't silently add an entry `favicon_host` can't parse.
    #[test]
    fn every_quick_access_tile_url_has_a_resolvable_host() {
        for tile in QUICK_ACCESS_TILES {
            assert!(
                favicon_host(tile.url).is_some(),
                "{}'s url {:?} has no resolvable host",
                tile.label,
                tile.url
            );
        }
    }

    #[test]
    fn favicon_cache_path_names_the_host_under_the_given_cache_dir() {
        let dir = PathBuf::from("/tmp/ferrite-ui-test-favicons");
        assert_eq!(
            favicon_cache_path(&dir, "github.com"),
            dir.join("github.com.ico")
        );
    }

    /// A tiny, real, freshly-encoded PNG (via the same `image` crate this
    /// module decodes with) — proves `decode_favicon_rgba`'s actual decode
    /// call works end to end against real image bytes, not just that the
    /// code type-checks. This is the one piece of the fetch/cache/decode
    /// pipeline this sandbox could exercise for real (see `docs/PROGRESS.md`
    /// for why the network half could not be — this sandbox's own outbound
    /// proxy blocks every one of the six real sites' domains outright).
    fn sample_png_bytes(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    #[test]
    fn decode_favicon_rgba_decodes_a_real_png_round_trip() {
        let bytes = sample_png_bytes(16, 16);
        let (width, height, rgba) = decode_favicon_rgba(&bytes).expect("should decode");
        assert_eq!((width, height), (16, 16));
        assert_eq!(rgba.len(), 16 * 16 * 4);
        // Every pixel is the exact colour `sample_png_bytes` encoded —
        // proves the bytes that come back are the real decoded pixels, not
        // some placeholder/zeroed buffer.
        assert_eq!(&rgba[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn decode_favicon_rgba_is_none_for_non_image_bytes() {
        assert_eq!(decode_favicon_rgba(b"not an image at all"), None);
    }

    #[test]
    fn window_icon_decodes_to_a_256px_square() {
        let (w, h, rgba) = decode_favicon_rgba(WINDOW_ICON_PNG).expect("embedded icon decodes");
        assert_eq!((w, h), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
        assert!(window_icon().is_some());
    }

    #[test]
    fn decode_favicon_rgba_is_none_for_empty_bytes() {
        assert_eq!(decode_favicon_rgba(&[]), None);
    }

    #[test]
    fn icon_links_are_found_resolved_and_ranked() {
        let base = url::Url::parse("https://example.org/").unwrap();
        let html = r#"<html><head>
            <link rel="stylesheet" href="/s.css">
            <LINK REL="apple-touch-icon" HREF="/touch.png">
            <link rel='shortcut icon' href='/static/fav.ico'>
            <link rel="icon" type="image/svg+xml" href="/logo.svg">
            <link rel=icon href=//cdn.example.net/a.png>
            </head>"#;
        let links: Vec<String> = icon_links_in_html(html, &base)
            .into_iter()
            .map(|u| u.to_string())
            .collect();
        assert_eq!(
            links,
            vec![
                "https://example.org/static/fav.ico",
                "https://cdn.example.net/a.png",
                "https://example.org/touch.png",
                "https://example.org/logo.svg",
            ]
        );
    }

    #[test]
    fn a_page_with_no_icon_links_yields_none() {
        let base = url::Url::parse("https://example.org/").unwrap();
        assert!(icon_links_in_html("<html><link rel=stylesheet href=x.css>", &base).is_empty());
    }

    #[test]
    fn tile_monogram_is_the_uppercased_first_character() {
        assert_eq!(tile_monogram("GitHub"), "G");
        assert_eq!(tile_monogram("Hacker News"), "H");
        assert_eq!(tile_monogram(""), "");
    }

    #[tokio::test]
    async fn fetch_tile_favicons_is_a_no_op_without_a_resolved_cache_dir() {
        // R7: `FetchTileFavicons` spawns real background `reqwest` tasks,
        // but — per this test module's own header comment — a
        // `#[tokio::test]` that never `.await`s past the spawn point never
        // actually polls one into making a live call. This case doesn't
        // even reach a spawn: no cache dir resolved means no fetch is ever
        // started.
        let mut state = FerriteBrowser {
            favicons_cache_dir: None,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::FetchTileFavicons);
        assert!(state.tile_favicons.iter().all(Option::is_none));
    }

    #[tokio::test]
    async fn fetch_tile_favicons_spawns_one_task_per_tile_with_a_cache_dir_set() {
        let mut state = FerriteBrowser {
            favicons_cache_dir: Some(PathBuf::from("/tmp/ferrite-ui-test-favicons-spawn")),
            ..FerriteBrowser::default()
        };
        // Only the synchronous bookkeeping is observable here (no message
        // is sent back before this test ends — see the no-op case above for
        // the full R7 reasoning); this asserts the handler at least ran to
        // completion without panicking and left every tile favicon
        // unresolved, which is the correct pre-fetch-completing state.
        let _ = update(&mut state, FerriteBrowserMessage::FetchTileFavicons);
        assert_eq!(state.tile_favicons.len(), QUICK_ACCESS_TILES.len());
        assert!(state.tile_favicons.iter().all(Option::is_none));
    }

    #[test]
    fn tile_favicon_ready_sets_the_matching_slot() {
        let mut state = FerriteBrowser::default();
        assert!(state.tile_favicons[0].is_none());
        let _ = update(
            &mut state,
            FerriteBrowserMessage::TileFaviconReady {
                index: 0,
                width: 4,
                height: 4,
                rgba: vec![0u8; 4 * 4 * 4],
            },
        );
        assert!(state.tile_favicons[0].is_some());
        // Every other slot is untouched.
        assert!(state.tile_favicons[1..].iter().all(Option::is_none));
    }

    #[test]
    fn tile_favicon_ready_with_an_out_of_range_index_is_ignored_not_a_panic() {
        let mut state = FerriteBrowser::default();
        let _ = update(
            &mut state,
            FerriteBrowserMessage::TileFaviconReady {
                index: 999,
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            },
        );
        assert!(state.tile_favicons.iter().all(Option::is_none));
    }

    #[test]
    fn default_ferrite_browser_starts_with_no_resolved_tile_favicons_or_cache_dir() {
        // R7/test-safety: exactly the same discipline `bookmarks_path`/
        // `downloads_dir` already establish — never resolved from a real
        // home directory inside `Default`, only from `launch()`.
        let state = FerriteBrowser::default();
        assert_eq!(state.tile_favicons.len(), QUICK_ACCESS_TILES.len());
        assert!(state.tile_favicons.iter().all(Option::is_none));
        assert!(state.favicons_cache_dir.is_none());
    }

    // ── C3d: Library panel ───────────────────────────────────────────────

    #[test]
    fn toggle_library_panel_closes_audit_and_js_panels() {
        let mut state = FerriteBrowser {
            show_audit_panel: true,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::ToggleLibraryPanel);
        assert!(state.show_library_panel);
        assert!(!state.show_audit_panel);
    }

    #[test]
    fn toggle_audit_panel_closes_the_library_panel() {
        let mut state = FerriteBrowser {
            show_library_panel: true,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::ToggleAuditPanel);
        assert!(state.show_audit_panel);
        assert!(!state.show_library_panel);
    }

    #[test]
    fn select_library_tab_switches_the_active_sub_view() {
        let mut state = FerriteBrowser::default();
        assert_eq!(state.library_tab, LibraryTab::Bookmarks);
        let _ = update(
            &mut state,
            FerriteBrowserMessage::SelectLibraryTab(LibraryTab::Downloads),
        );
        assert_eq!(state.library_tab, LibraryTab::Downloads);
    }

    // ── C3d: Escape resolves through EscapePressed, not handle_key_press ──

    #[test]
    fn escape_closes_the_find_bar_before_its_other_meanings() {
        let mut state = FerriteBrowser {
            show_find_bar: true,
            is_loading: true,
            ..FerriteBrowser::default()
        };
        let _ = update(&mut state, FerriteBrowserMessage::EscapePressed);
        assert!(!state.show_find_bar);
        // is_loading is untouched here — EscapePressed only *returns* a
        // Task::done(StopLoading) in the non-find-bar branch; this handler
        // exercised the find-bar branch instead, so no StopLoading dispatch
        // happened within this single `update()` call.
        assert!(state.is_loading);
    }
}
