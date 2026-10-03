//! DevTools state and rules: what each tab's console and network logs hold,
//! how they are filtered, searched, collapsed and exported, and what the
//! panel's buttons do.
//!
//! Everything here is plain data and pure functions so it is testable without a
//! window or an engine; `devtools_panel` draws it.
//!
//! **Cost.** Entries are drained from each session once per tick and appended
//! here, so ingesting is O(new entries). Filtering is incremental too: a
//! [`FilterCache`] remembers which rows matched the current filter and only
//! scans rows added since, so an open Console with a search typed in costs
//! nothing per tick while the page is quiet. Memory is bounded three ways:
//! at most `diag::MAX_ENTRIES` rows per log, at most [`MAX_MESSAGE_CHARS`] per
//! message, and at most [`MAX_LOG_BYTES`] of message text per tab.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use ferrite_servo::diag::{
    ConsoleEntry, ConsoleLevel, CrashNote, NetEvent, PanicNote, MAX_ENTRIES,
};
use iced::widget::scrollable;
use iced::Task;

use crate::{FerriteBrowser, FerriteBrowserMessage};

/// The longest a single console message is kept; the rest is dropped with a note.
pub(crate) const MAX_MESSAGE_CHARS: usize = 4_000;
/// The most message text one tab's console keeps (oldest rows go first).
pub(crate) const MAX_LOG_BYTES: usize = 8 * 1024 * 1024;
/// The most engine panics and crashes kept.
const MAX_ENGINE_EVENTS: usize = 200;
/// Rows drawn at first, and added by each "show older".
pub(crate) const RENDER_STEP: usize = 200;
/// Longest a mirrored stderr line is, in characters.
const MIRROR_MAX_CHARS: usize = 400;
/// The most console lines mirrored to stderr per tick, so one chatty page
/// cannot flood the log file.
pub(crate) const MIRROR_PER_TICK: usize = 20;
/// The most typed expressions remembered for ↑/↓.
const HISTORY_MAX: usize = 100;

// ── What the panel is showing ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevTab {
    #[default]
    Console,
    Network,
    Engine,
}

/// Which severities the Console shows. `Info` covers `console.log` and
/// `console.info`, as DevTools' "Info" does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LevelFilter {
    #[default]
    All,
    Errors,
    Warnings,
    Info,
    Debug,
}

impl LevelFilter {
    pub(crate) const ALL: [LevelFilter; 5] = [
        LevelFilter::All,
        LevelFilter::Errors,
        LevelFilter::Warnings,
        LevelFilter::Info,
        LevelFilter::Debug,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Errors => "Errors",
            Self::Warnings => "Warnings",
            Self::Info => "Info",
            Self::Debug => "Debug",
        }
    }

    pub(crate) fn admits(self, level: ConsoleLevel) -> bool {
        match self {
            Self::All => true,
            Self::Errors => level == ConsoleLevel::Error,
            Self::Warnings => level == ConsoleLevel::Warn,
            Self::Info => matches!(level, ConsoleLevel::Log | ConsoleLevel::Info),
            Self::Debug => level == ConsoleLevel::Debug,
        }
    }
}

/// What a request is for, grouped the way DevTools' type filter groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetKind {
    Document,
    Script,
    Style,
    Image,
    Font,
    Media,
    Other,
}

impl NetKind {
    pub(crate) const ALL: [NetKind; 7] = [
        NetKind::Document,
        NetKind::Script,
        NetKind::Style,
        NetKind::Image,
        NetKind::Font,
        NetKind::Media,
        NetKind::Other,
    ];

    /// Groups the engine's request destination (`format!("{:?}")` of Servo's
    /// `Destination`). Anything unlisted, including `fetch()` and XHR (which
    /// have no destination), is `Other`.
    pub(crate) fn of(kind: &str) -> Self {
        match kind {
            "Document" | "Frame" | "IFrame" | "Embed" | "Object" => Self::Document,
            "Script" | "Worker" | "ServiceWorker" | "SharedWorker" | "AudioWorklet"
            | "PaintWorklet" => Self::Script,
            "Style" | "Xslt" => Self::Style,
            "Image" => Self::Image,
            "Font" => Self::Font,
            "Audio" | "Video" | "Track" => Self::Media,
            _ => Self::Other,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Document => "Doc",
            Self::Script => "JS",
            Self::Style => "CSS",
            Self::Image => "Img",
            Self::Font => "Font",
            Self::Media => "Media",
            Self::Other => "Other",
        }
    }

    fn slot(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// The Network tab's type filter: everything, or one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetFilter {
    #[default]
    All,
    Kind(NetKind),
}

impl NetFilter {
    pub(crate) fn admits(self, kind: NetKind) -> bool {
        match self {
            Self::All => true,
            Self::Kind(k) => k == kind,
        }
    }
}

/// Which list a "toggle expanded" or "copy" refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Console,
    Network,
    Engine,
}

// ── Rows ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// A `console.*` call or uncaught exception.
    Message,
    /// Code the person typed at the prompt.
    Input,
    /// What that code evaluated to.
    Result,
    /// The page navigated (shown when the log is preserved across loads).
    Navigation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsoleRow {
    /// Unique and increasing; row `p` of the log has id `front_id + p`.
    pub id: u64,
    pub kind: RowKind,
    pub level: ConsoleLevel,
    pub message: String,
    pub source: Option<String>,
    pub first_ms: u64,
    pub last_ms: u64,
    /// How many identical messages in a row this stands for.
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetRow {
    pub id: u64,
    pub at_ms: u64,
    pub method: String,
    pub url: String,
    pub kind: NetKind,
    pub main_frame: bool,
}

fn level_slot(level: ConsoleLevel) -> usize {
    match level {
        ConsoleLevel::Debug => 0,
        ConsoleLevel::Log => 1,
        ConsoleLevel::Info => 2,
        ConsoleLevel::Warn => 3,
        ConsoleLevel::Error => 4,
    }
}

/// Cuts `message` to [`MAX_MESSAGE_CHARS`], saying how much was dropped.
pub(crate) fn cap_message(message: String) -> String {
    let total = message.chars().count();
    if total <= MAX_MESSAGE_CHARS {
        return message;
    }
    let mut cut: String = message.chars().take(MAX_MESSAGE_CHARS).collect();
    cut.push_str(&format!(
        "\n… ({} more characters not kept)",
        total - MAX_MESSAGE_CHARS
    ));
    cut
}

/// Whether `haystack` contains `needle_lower` ignoring case, without
/// allocating for the common ASCII case.
pub(crate) fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if needle_lower.is_ascii() {
        let h = haystack.as_bytes();
        let n = needle_lower.as_bytes();
        if n.len() > h.len() {
            return false;
        }
        return h
            .windows(n.len())
            .any(|w| w.iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b));
    }
    haystack.to_lowercase().contains(needle_lower)
}

// ── Incremental filtering ────────────────────────────────────────────────

/// Which rows of an append-only log match the current filter, kept up to date
/// by scanning only what was added since.
#[derive(Debug)]
struct FilterCache<K> {
    key: Option<K>,
    epoch: u64,
    ids: VecDeque<u64>,
    /// The id of the next row to look at.
    scanned_to: u64,
}

impl<K> Default for FilterCache<K> {
    fn default() -> Self {
        Self {
            key: None,
            epoch: 0,
            ids: VecDeque::new(),
            scanned_to: 0,
        }
    }
}

impl<K: PartialEq + Clone> FilterCache<K> {
    /// Brings the cache current for `key` over `rows` (whose first row has id
    /// `front_id`). `epoch` changes whenever rows other than appends changed.
    fn refresh<R>(
        &mut self,
        key: &K,
        epoch: u64,
        rows: &VecDeque<R>,
        front_id: u64,
        matches: impl Fn(&R) -> bool,
    ) {
        if self.key.as_ref() != Some(key) || self.epoch != epoch {
            self.key = Some(key.clone());
            self.epoch = epoch;
            self.ids.clear();
            self.scanned_to = front_id;
        }
        while self.ids.front().is_some_and(|id| *id < front_id) {
            self.ids.pop_front();
        }
        let start = usize::try_from(self.scanned_to.saturating_sub(front_id)).unwrap_or(0);
        for (offset, row) in rows.iter().enumerate().skip(start) {
            if matches(row) {
                self.ids.push_back(front_id + offset as u64);
            }
        }
        self.scanned_to = front_id + rows.len() as u64;
    }
}

/// The rows to draw: the newest `limit` that match, oldest first.
pub(crate) struct View<'a, R> {
    pub rows: Vec<&'a R>,
    /// Matching rows older than the ones drawn.
    pub hidden: usize,
}

fn pick<'a, R>(
    ids: &VecDeque<u64>,
    rows: &'a VecDeque<R>,
    front_id: u64,
    limit: usize,
) -> View<'a, R> {
    let skip = ids.len().saturating_sub(limit);
    let picked = ids
        .iter()
        .skip(skip)
        .filter_map(|id| rows.get(usize::try_from(id - front_id).ok()?))
        .collect();
    View {
        rows: picked,
        hidden: skip,
    }
}

// ── One tab's logs ───────────────────────────────────────────────────────

/// A tab's console and network logs.
#[derive(Debug, Default)]
pub(crate) struct TabLog {
    console: VecDeque<ConsoleRow>,
    console_next_id: u64,
    /// Messages per level (a collapsed row counts as its `count`).
    level_counts: [usize; 5],
    console_bytes: usize,
    console_epoch: u64,
    console_cache: RefCell<FilterCache<(LevelFilter, String)>>,

    net: VecDeque<NetRow>,
    net_next_id: u64,
    kind_counts: [usize; 7],
    net_epoch: u64,
    net_cache: RefCell<FilterCache<(NetFilter, String)>>,
    /// When the page last navigated (its main document was requested);
    /// request times are shown relative to it.
    pub nav_ms: Option<u64>,
}

/// What an [`TabLog::ingest`] added.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Ingested {
    pub console: usize,
    pub net: usize,
    pub errors: usize,
}

impl TabLog {
    fn front_id(&self) -> u64 {
        self.console.front().map_or(self.console_next_id, |r| r.id)
    }

    fn net_front_id(&self) -> u64 {
        self.net.front().map_or(self.net_next_id, |r| r.id)
    }

    /// Appends one drained batch. A main-frame document request is a
    /// navigation: unless `preserve`, both logs start afresh (messages the old
    /// page logged before the request are dropped with them).
    pub(crate) fn ingest(
        &mut self,
        console: Vec<ConsoleEntry>,
        net: Vec<NetEvent>,
        preserve: bool,
    ) -> Ingested {
        let mut added = Ingested::default();
        let navigated = net
            .iter()
            .filter(|n| n.is_main_frame && NetKind::of(&n.kind) == NetKind::Document)
            .max_by_key(|n| n.at_ms);
        let mut keep_from = 0;
        if let Some(nav) = navigated {
            keep_from = nav.at_ms;
            self.nav_ms = Some(nav.at_ms);
            if preserve {
                self.push_row(ConsoleRow {
                    id: 0,
                    kind: RowKind::Navigation,
                    level: ConsoleLevel::Info,
                    message: nav.url.clone(),
                    source: None,
                    first_ms: nav.at_ms,
                    last_ms: nav.at_ms,
                    count: 1,
                });
            } else {
                self.clear_console();
                self.clear_net();
            }
        }
        for entry in console {
            if !preserve && entry.at_ms < keep_from {
                continue;
            }
            if entry.level == ConsoleLevel::Error {
                added.errors += 1;
            }
            self.push_message(entry);
            added.console += 1;
        }
        for event in net {
            if !preserve && event.at_ms < keep_from {
                continue;
            }
            self.push_net(event);
            added.net += 1;
        }
        added
    }

    fn push_row(&mut self, mut row: ConsoleRow) {
        row.id = self.console_next_id;
        self.console_next_id += 1;
        if row.kind == RowKind::Message || row.kind == RowKind::Result {
            self.level_counts[level_slot(row.level)] += 1;
        }
        self.console_bytes += row.message.len();
        self.console.push_back(row);
        self.trim_console();
    }

    fn trim_console(&mut self) {
        while self.console.len() > MAX_ENTRIES || self.console_bytes > MAX_LOG_BYTES {
            let Some(old) = self.console.pop_front() else {
                break;
            };
            self.console_bytes = self.console_bytes.saturating_sub(old.message.len());
            if old.kind == RowKind::Message || old.kind == RowKind::Result {
                let slot = &mut self.level_counts[level_slot(old.level)];
                *slot = slot.saturating_sub(old.count as usize);
            }
        }
    }

    /// Appends a console message, folding it into the previous row when it is
    /// identical (same level, text and source), as DevTools does.
    pub(crate) fn push_message(&mut self, entry: ConsoleEntry) {
        let message = cap_message(entry.message);
        if let Some(last) = self.console.back_mut() {
            if last.kind == RowKind::Message
                && last.level == entry.level
                && last.message == message
                && last.source == entry.source
            {
                last.count = last.count.saturating_add(1);
                last.last_ms = entry.at_ms;
                self.level_counts[level_slot(entry.level)] += 1;
                return;
            }
        }
        self.push_row(ConsoleRow {
            id: 0,
            kind: RowKind::Message,
            level: entry.level,
            message,
            source: entry.source,
            first_ms: entry.at_ms,
            last_ms: entry.at_ms,
            count: 1,
        });
    }

    /// Records code the person typed at the prompt.
    pub(crate) fn push_input(&mut self, code: &str) {
        let now = ferrite_servo::diag::now_ms();
        self.push_row(ConsoleRow {
            id: 0,
            kind: RowKind::Input,
            level: ConsoleLevel::Log,
            message: cap_message(code.to_string()),
            source: None,
            first_ms: now,
            last_ms: now,
            count: 1,
        });
    }

    /// Records the value an expression evaluated to.
    pub(crate) fn push_result(&mut self, value: &str) {
        let now = ferrite_servo::diag::now_ms();
        self.push_row(ConsoleRow {
            id: 0,
            kind: RowKind::Result,
            level: ConsoleLevel::Log,
            message: cap_message(value.to_string()),
            source: None,
            first_ms: now,
            last_ms: now,
            count: 1,
        });
    }

    fn push_net(&mut self, event: NetEvent) {
        let kind = NetKind::of(&event.kind);
        self.kind_counts[kind.slot()] += 1;
        let id = self.net_next_id;
        self.net_next_id += 1;
        self.net.push_back(NetRow {
            id,
            at_ms: event.at_ms,
            method: event.method,
            url: event.url,
            kind,
            main_frame: event.is_main_frame,
        });
        while self.net.len() > MAX_ENTRIES {
            if let Some(old) = self.net.pop_front() {
                let slot = &mut self.kind_counts[old.kind.slot()];
                *slot = slot.saturating_sub(1);
            }
        }
    }

    pub(crate) fn clear_console(&mut self) {
        self.console.clear();
        self.level_counts = [0; 5];
        self.console_bytes = 0;
        self.console_epoch += 1;
    }

    pub(crate) fn clear_net(&mut self) {
        self.net.clear();
        self.kind_counts = [0; 7];
        self.net_epoch += 1;
    }

    /// How many messages the filter's chip counts.
    pub(crate) fn level_count(&self, filter: LevelFilter) -> usize {
        match filter {
            LevelFilter::All => self.level_counts.iter().sum(),
            LevelFilter::Errors => self.level_counts[level_slot(ConsoleLevel::Error)],
            LevelFilter::Warnings => self.level_counts[level_slot(ConsoleLevel::Warn)],
            LevelFilter::Info => {
                self.level_counts[level_slot(ConsoleLevel::Log)]
                    + self.level_counts[level_slot(ConsoleLevel::Info)]
            }
            LevelFilter::Debug => self.level_counts[level_slot(ConsoleLevel::Debug)],
        }
    }

    pub(crate) fn error_count(&self) -> usize {
        self.level_count(LevelFilter::Errors)
    }

    pub(crate) fn warning_count(&self) -> usize {
        self.level_count(LevelFilter::Warnings)
    }

    /// How many requests the type chip counts.
    pub(crate) fn kind_count(&self, filter: NetFilter) -> usize {
        match filter {
            NetFilter::All => self.kind_counts.iter().sum(),
            NetFilter::Kind(k) => self.kind_counts[k.slot()],
        }
    }

    pub(crate) fn console_len(&self) -> usize {
        self.console.len()
    }

    pub(crate) fn net_len(&self) -> usize {
        self.net.len()
    }

    /// The Console's rows for `filter` and `search`, newest `limit` of them.
    /// Input and result rows follow the level filter like any other (an input
    /// row counts as `Log`), and a navigation marker always shows.
    pub(crate) fn console_view(
        &self,
        filter: LevelFilter,
        search: &str,
        limit: usize,
    ) -> View<'_, ConsoleRow> {
        let needle = search.trim().to_lowercase();
        let front = self.front_id();
        let mut cache = self.console_cache.borrow_mut();
        cache.refresh(
            &(filter, needle.clone()),
            self.console_epoch,
            &self.console,
            front,
            |row| {
                if row.kind == RowKind::Navigation {
                    return true;
                }
                filter.admits(row.level)
                    && (needle.is_empty()
                        || contains_ci(&row.message, &needle)
                        || row
                            .source
                            .as_deref()
                            .is_some_and(|s| contains_ci(s, &needle)))
            },
        );
        pick(&cache.ids, &self.console, front, limit)
    }

    /// The Network's rows for `filter` and `search`, newest `limit` of them.
    pub(crate) fn net_view(
        &self,
        filter: NetFilter,
        search: &str,
        limit: usize,
    ) -> View<'_, NetRow> {
        let needle = search.trim().to_lowercase();
        let front = self.net_front_id();
        let mut cache = self.net_cache.borrow_mut();
        cache.refresh(
            &(filter, needle.clone()),
            self.net_epoch,
            &self.net,
            front,
            |row| {
                filter.admits(row.kind)
                    && (needle.is_empty()
                        || contains_ci(&row.url, &needle)
                        || contains_ci(&row.method, &needle))
            },
        );
        pick(&cache.ids, &self.net, front, limit)
    }

    pub(crate) fn console_rows(&self) -> impl Iterator<Item = &ConsoleRow> {
        self.console.iter()
    }

    pub(crate) fn net_rows(&self) -> impl Iterator<Item = &NetRow> {
        self.net.iter()
    }
}

// ── Engine panics and crashes ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineKind {
    /// A thread of the app or engine panicked.
    Panic,
    /// A page's script thread died.
    Crash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineEvent {
    pub id: u64,
    pub kind: EngineKind,
    pub at_ms: u64,
    /// `"tab 2"` for a crash; empty for a panic, which belongs to no tab.
    pub tab: String,
    pub thread: String,
    pub message: String,
    pub location: String,
    pub backtrace: Option<String>,
}

/// Panics and crashes seen this session, newest last.
#[derive(Debug, Default)]
pub(crate) struct EngineLog {
    events: VecDeque<EngineEvent>,
    next_id: u64,
}

impl EngineLog {
    fn push(&mut self, mut event: EngineEvent) {
        event.id = self.next_id;
        self.next_id += 1;
        self.events.push_back(event);
        while self.events.len() > MAX_ENGINE_EVENTS {
            self.events.pop_front();
        }
    }

    pub(crate) fn push_panic(&mut self, note: PanicNote) {
        self.push(EngineEvent {
            id: 0,
            kind: EngineKind::Panic,
            at_ms: note.at_ms,
            tab: String::new(),
            thread: note.thread,
            message: note.message,
            location: note.location,
            backtrace: None,
        });
    }

    pub(crate) fn push_crash(&mut self, tab: usize, note: &CrashNote) {
        self.push(EngineEvent {
            id: 0,
            kind: EngineKind::Crash,
            at_ms: note.at_ms,
            tab: format!("tab {}", tab + 1),
            thread: "page script thread".to_string(),
            message: note.reason.clone(),
            location: String::new(),
            backtrace: note.backtrace.clone(),
        });
    }

    pub(crate) fn events(&self) -> &VecDeque<EngineEvent> {
        &self.events
    }

    pub(crate) fn len(&self) -> usize {
        self.events.len()
    }

    pub(crate) fn clear(&mut self) {
        self.events.clear();
    }
}

// ── The prompt's history ─────────────────────────────────────────────────

/// Expressions typed at the prompt, recalled with ↑ and ↓.
#[derive(Debug, Default)]
pub(crate) struct InputHistory {
    entries: Vec<String>,
    /// Which entry is showing, or `None` while the person's own text is.
    pos: Option<usize>,
    /// What was being typed when they first pressed ↑.
    draft: String,
}

impl InputHistory {
    pub(crate) fn push(&mut self, entry: &str) {
        self.pos = None;
        self.draft.clear();
        if entry.is_empty() || self.entries.last().is_some_and(|last| last == entry) {
            return;
        }
        self.entries.push(entry.to_string());
        if self.entries.len() > HISTORY_MAX {
            self.entries.remove(0);
        }
    }

    /// ↑: the next older entry, or `None` when there is nothing older.
    pub(crate) fn older(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.pos {
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => 0,
            Some(p) => p - 1,
        };
        self.pos = Some(next);
        Some(self.entries[next].clone())
    }

    /// ↓: the next newer entry, ending back at what was being typed.
    pub(crate) fn newer(&mut self) -> Option<String> {
        let p = self.pos?;
        if p + 1 < self.entries.len() {
            self.pos = Some(p + 1);
            Some(self.entries[p + 1].clone())
        } else {
            self.pos = None;
            Some(std::mem::take(&mut self.draft))
        }
    }
}

// ── The panel's own state ────────────────────────────────────────────────

/// Which view and filters the panel has, shared by every tab (the logs
/// themselves are per tab).
#[derive(Debug)]
pub(crate) struct DevToolsUi {
    pub tab: DevTab,
    pub level: LevelFilter,
    pub search: String,
    pub preserve: bool,
    pub net_filter: NetFilter,
    pub net_search: String,
    pub expanded: HashSet<(Section, u64)>,
    pub console_limit: usize,
    pub net_limit: usize,
    /// The prompt's text.
    pub input: String,
    pub history: InputHistory,
    /// Whether the prompt has the keyboard (so ↑/↓ recall history rather than
    /// reaching the page).
    pub input_focused: bool,
    /// Whether the Console is scrolled to the newest message (new ones then
    /// keep it there).
    pub pinned: bool,
    /// A one-line result of the last Copy / Save / Open ("Saved to …").
    pub notice: Option<String>,
}

impl Default for DevToolsUi {
    fn default() -> Self {
        Self {
            tab: DevTab::Console,
            level: LevelFilter::All,
            search: String::new(),
            preserve: false,
            net_filter: NetFilter::All,
            net_search: String::new(),
            expanded: HashSet::new(),
            console_limit: RENDER_STEP,
            net_limit: RENDER_STEP,
            input: String::new(),
            history: InputHistory::default(),
            input_focused: false,
            pinned: true,
            notice: None,
        }
    }
}

pub(crate) fn console_scroll_id() -> scrollable::Id {
    scrollable::Id::new("ferrite_devtools_console")
}

pub(crate) fn net_scroll_id() -> scrollable::Id {
    scrollable::Id::new("ferrite_devtools_network")
}

#[derive(Debug, Clone)]
pub enum Msg {
    SetTab(DevTab),
    SetLevel(LevelFilter),
    Search(String),
    SetNetFilter(NetFilter),
    NetSearch(String),
    TogglePreserve,
    /// Empties the log on the current tab of the panel.
    Clear,
    Toggle(Section, u64),
    ShowOlder,
    CopyAll,
    SaveLog,
    OpenLogFolder,
    /// Puts some text (a location, a URL, a backtrace) on the clipboard.
    CopyText(String),
    /// The Console scrolled; `pinned` says whether it is at the newest message.
    Scrolled {
        pinned: bool,
    },
    /// The prompt gained or lost the keyboard.
    InputFocus(bool),
}

pub(crate) fn update(state: &mut FerriteBrowser, msg: Msg) -> Task<FerriteBrowserMessage> {
    match msg {
        Msg::SetTab(tab) => {
            state.devtools.tab = tab;
            state.devtools.notice = None;
            // The prompt only exists on the Console tab.
            state.devtools.input_focused = false;
        }
        Msg::SetLevel(level) => {
            state.devtools.level = level;
            state.devtools.console_limit = RENDER_STEP;
        }
        Msg::Search(text) => {
            state.devtools.search = text;
            state.devtools.console_limit = RENDER_STEP;
        }
        Msg::SetNetFilter(filter) => {
            state.devtools.net_filter = filter;
            state.devtools.net_limit = RENDER_STEP;
        }
        Msg::NetSearch(text) => {
            state.devtools.net_search = text;
            state.devtools.net_limit = RENDER_STEP;
        }
        Msg::TogglePreserve => state.devtools.preserve = !state.devtools.preserve,
        Msg::Clear => {
            let active = state.active_tab;
            match state.devtools.tab {
                DevTab::Console => {
                    if let Some(diag) = state.tab_diag.get_mut(active) {
                        diag.log.clear_console();
                    }
                    state
                        .devtools
                        .expanded
                        .retain(|(s, _)| *s != Section::Console);
                    state.devtools.console_limit = RENDER_STEP;
                }
                DevTab::Network => {
                    if let Some(diag) = state.tab_diag.get_mut(active) {
                        diag.log.clear_net();
                    }
                    state
                        .devtools
                        .expanded
                        .retain(|(s, _)| *s != Section::Network);
                    state.devtools.net_limit = RENDER_STEP;
                }
                DevTab::Engine => {
                    state.engine_log.clear();
                    state
                        .devtools
                        .expanded
                        .retain(|(s, _)| *s != Section::Engine);
                }
            }
            state.devtools.notice = None;
        }
        Msg::Toggle(section, id) => {
            let key = (section, id);
            if !state.devtools.expanded.remove(&key) {
                state.devtools.expanded.insert(key);
            }
        }
        Msg::ShowOlder => match state.devtools.tab {
            DevTab::Network => state.devtools.net_limit += RENDER_STEP,
            _ => state.devtools.console_limit += RENDER_STEP,
        },
        Msg::CopyAll => {
            let text = export_text(state, state.devtools.tab);
            let lines = text.lines().count();
            state.devtools.notice = Some(format!("Copied {lines} lines"));
            return iced::clipboard::write(text);
        }
        Msg::CopyText(text) => {
            state.devtools.notice = Some("Copied".to_string());
            return iced::clipboard::write(text);
        }
        Msg::SaveLog => {
            let text = export_text(state, state.devtools.tab);
            state.devtools.notice = Some(match log_dir() {
                Some(dir) => {
                    let name = save_file_name(state.active_tab, state.devtools.tab);
                    match save_log(&dir, &name, &text) {
                        Ok(path) => format!("Saved to {}", path.display()),
                        Err(e) => format!("Could not save the log: {e}"),
                    }
                }
                None => "Could not save the log: no log folder on this machine".to_string(),
            });
        }
        Msg::OpenLogFolder => {
            state.devtools.notice = Some(match log_dir() {
                Some(dir) => match open_folder(&dir) {
                    Ok(()) => format!("Opened {}", dir.display()),
                    Err(e) => format!("Could not open {}: {e}", dir.display()),
                },
                None => "No log folder on this machine".to_string(),
            });
        }
        Msg::Scrolled { pinned } => state.devtools.pinned = pinned,
        Msg::InputFocus(focused) => state.devtools.input_focused = focused,
    }
    Task::none()
}

/// Whether the Console is pinned to its newest message, from its scroll
/// position (`relative` is 0 at the top, 1 at the bottom).
pub(crate) fn is_pinned(relative: f32) -> bool {
    relative >= 0.97
}

// ── Typing at the prompt ─────────────────────────────────────────────────

/// What `execute_js` returned, shown as DevTools shows a value: the engine
/// reports `Debug` text (`String("hi")`, `Number(2.0)`), which is unwrapped for
/// the plain cases and left alone otherwise.
pub(crate) fn pretty_js_value(raw: &str) -> String {
    let raw = raw.trim();
    match raw {
        "Undefined" => return "undefined".to_string(),
        "Null" => return "null".to_string(),
        _ => {}
    }
    if let Some(inner) = raw
        .strip_prefix("Boolean(")
        .and_then(|r| r.strip_suffix(')'))
    {
        return inner.to_string();
    }
    if let Some(inner) = raw
        .strip_prefix("Number(")
        .and_then(|r| r.strip_suffix(')'))
    {
        return match inner.parse::<f64>() {
            Ok(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", n as i64),
            _ => inner.to_string(),
        };
    }
    if let Some(inner) = raw
        .strip_prefix("String(")
        .and_then(|r| r.strip_suffix(')'))
    {
        if let Some(quoted) = inner.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
            return format!("\"{}\"", unescape_debug(quoted));
        }
    }
    raw.to_string()
}

fn unescape_debug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Runs the prompt's text on the active page and echoes both in the Console.
pub(crate) fn run_prompt(state: &mut FerriteBrowser) {
    let script = state.devtools.input.trim().to_string();
    if script.is_empty() {
        return;
    }
    state.devtools.input.clear();
    state.devtools.history.push(&script);
    let active = state.active_tab;
    let outcome = match state.servo_sessions.get_mut(&active) {
        Some(session) => session.execute_js(&script),
        None => Err("no active page to run this on".to_string()),
    };
    if let Some(diag) = state.tab_diag.get_mut(active) {
        diag.log.push_input(&script);
        match outcome {
            Ok(value) => diag.log.push_result(&pretty_js_value(&value)),
            Err(e) => diag.log.push_message(ConsoleEntry {
                at_ms: ferrite_servo::diag::now_ms(),
                level: ConsoleLevel::Error,
                message: e,
                source: None,
            }),
        }
    }
    state.devtools.pinned = true;
}

/// ↑ or ↓ at the prompt.
pub(crate) fn recall(state: &mut FerriteBrowser, older: bool) {
    let recalled = if older {
        let current = state.devtools.input.clone();
        state.devtools.history.older(&current)
    } else {
        state.devtools.history.newer()
    };
    if let Some(text) = recalled {
        state.devtools.input = text;
    }
}

// ── Mirroring to stderr ──────────────────────────────────────────────────

/// The log-file line for a console message worth keeping in the app's log:
/// warnings and errors only, with long URLs shortened and the length capped.
/// `None` for lower levels.
pub(crate) fn stderr_line(tab: usize, level: ConsoleLevel, message: &str) -> Option<String> {
    let tag = match level {
        ConsoleLevel::Warn => "warn",
        ConsoleLevel::Error => "error",
        _ => return None,
    };
    let flat = crate::shorten_urls(message).replace('\n', " \u{21b5} ");
    Some(format!(
        "[console:{tag}] tab {}: {}",
        tab + 1,
        crate::truncate(&flat, MIRROR_MAX_CHARS)
    ))
}

// ── Export ───────────────────────────────────────────────────────────────

/// `HH:MM:SS.mmm` in local time.
pub(crate) fn clock_ms(at_ms: u64) -> String {
    i64::try_from(at_ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or_else(String::new, |t| {
            t.with_timezone(&chrono::Local)
                .format("%H:%M:%S%.3f")
                .to_string()
        })
}

fn level_word(level: ConsoleLevel) -> &'static str {
    match level {
        ConsoleLevel::Debug => "DEBUG",
        ConsoleLevel::Log => "LOG",
        ConsoleLevel::Info => "INFO",
        ConsoleLevel::Warn => "WARN",
        ConsoleLevel::Error => "ERROR",
    }
}

pub(crate) fn console_line(row: &ConsoleRow) -> String {
    let mut line = match row.kind {
        RowKind::Navigation => format!(
            "[{}] -- navigated to {}",
            clock_ms(row.first_ms),
            row.message
        ),
        RowKind::Input => format!("[{}] > {}", clock_ms(row.first_ms), row.message),
        RowKind::Result => format!("[{}] < {}", clock_ms(row.first_ms), row.message),
        RowKind::Message => format!(
            "[{}] {:<5} {}",
            clock_ms(row.first_ms),
            level_word(row.level),
            row.message
        ),
    };
    if let Some(source) = &row.source {
        line.push_str(&format!("  ({source})"));
    }
    if row.count > 1 {
        line.push_str(&format!("  (x{})", row.count));
    }
    line
}

/// How long after the page's navigation a request started, `+0.123s`, or the
/// clock time when there is no navigation to measure from.
pub(crate) fn net_time(at_ms: u64, nav_ms: Option<u64>) -> String {
    match nav_ms {
        Some(nav) if at_ms >= nav => {
            let ms = at_ms - nav;
            if ms < 10_000 {
                format!("+{ms} ms")
            } else {
                format!("+{:.1} s", ms as f64 / 1000.0)
            }
        }
        _ => clock_ms(at_ms),
    }
}

pub(crate) fn net_line(row: &NetRow, nav_ms: Option<u64>) -> String {
    format!(
        "[{}] {:<6} {:<5} {}",
        net_time(row.at_ms, nav_ms),
        row.method,
        row.kind.label(),
        row.url
    )
}

pub(crate) fn engine_text(event: &EngineEvent) -> String {
    let what = match event.kind {
        EngineKind::Panic => "PANIC",
        EngineKind::Crash => "CRASH",
    };
    let mut text = format!(
        "[{}] {what} {}: {}",
        clock_ms(event.at_ms),
        event.thread,
        event.message
    );
    if !event.tab.is_empty() {
        text.push_str(&format!(" ({})", event.tab));
    }
    if !event.location.is_empty() {
        text.push_str(&format!("\n    at {}", event.location));
    }
    if let Some(backtrace) = &event.backtrace {
        text.push('\n');
        text.push_str(backtrace);
    }
    text
}

/// Everything in the section the person is looking at, as plain text.
pub(crate) fn export_text(state: &FerriteBrowser, section: DevTab) -> String {
    let tab_label = state.active_tab + 1;
    let url = state
        .tab_urls
        .get(state.active_tab)
        .map_or("", String::as_str);
    let mut out = match section {
        DevTab::Console => format!("# Ferrite console, tab {tab_label}: {url}\n"),
        DevTab::Network => format!("# Ferrite requests, tab {tab_label}: {url}\n"),
        DevTab::Engine => "# Ferrite engine panics and crashes\n".to_string(),
    };
    match (section, state.tab_diag.get(state.active_tab)) {
        (DevTab::Console, Some(diag)) => {
            for row in diag.log.console_rows() {
                out.push_str(&console_line(row));
                out.push('\n');
            }
        }
        (DevTab::Network, Some(diag)) => {
            out.push_str("# status, size and timing are not reported by the engine\n");
            for row in diag.log.net_rows() {
                out.push_str(&net_line(row, diag.log.nav_ms));
                out.push('\n');
            }
        }
        (DevTab::Engine, _) => {
            for event in state.engine_log.events() {
                out.push_str(&engine_text(event));
                out.push('\n');
            }
        }
        _ => {}
    }
    out
}

// ── The log folder ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostOs {
    Mac,
    Windows,
    Other,
}

impl HostOs {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// Where the app's log file lives: the same rule as `ferrite-shell`'s
/// `logging::log_dir` (this crate cannot depend on the shell). macOS:
/// `~/Library/Logs/Ferrite`; Windows: `%LOCALAPPDATA%\Ferrite\logs`; elsewhere
/// `$XDG_STATE_HOME/ferrite` or `~/.local/state/ferrite`.
pub(crate) fn log_dir_for(
    os: HostOs,
    home: Option<PathBuf>,
    xdg_state: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> Option<PathBuf> {
    match os {
        HostOs::Mac => home.map(|h| h.join("Library").join("Logs").join("Ferrite")),
        HostOs::Windows => local_app_data.map(|d| d.join("Ferrite").join("logs")),
        HostOs::Other => xdg_state
            .or_else(|| home.map(|h| h.join(".local").join("state")))
            .map(|d| d.join("ferrite")),
    }
}

pub(crate) fn log_dir() -> Option<PathBuf> {
    log_dir_for(
        HostOs::current(),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

/// `ferrite-console-tab2-20261003-141500.log`.
pub(crate) fn save_file_name(tab: usize, section: DevTab) -> String {
    let what = match section {
        DevTab::Console => "console",
        DevTab::Network => "network",
        DevTab::Engine => "engine",
    };
    format!(
        "ferrite-{what}-tab{}-{}.log",
        tab + 1,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )
}

/// Writes `text` to `dir/name`, creating the folder.
pub(crate) fn save_log(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    std::fs::write(&path, text)?;
    Ok(path)
}

/// The program that opens a folder in the platform's file manager.
pub(crate) fn open_command(os: HostOs) -> &'static str {
    match os {
        HostOs::Mac => "open",
        HostOs::Windows => "explorer",
        HostOs::Other => "xdg-open",
    }
}

fn open_folder(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::process::Command::new(open_command(HostOs::current()))
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at_ms: u64, level: ConsoleLevel, message: &str) -> ConsoleEntry {
        ConsoleEntry {
            at_ms,
            level,
            message: message.to_string(),
            source: None,
        }
    }

    fn net(at_ms: u64, kind: &str, url: &str, main: bool) -> NetEvent {
        NetEvent {
            at_ms,
            method: "GET".to_string(),
            url: url.to_string(),
            kind: kind.to_string(),
            is_main_frame: main,
        }
    }

    fn log_with(entries: Vec<ConsoleEntry>) -> TabLog {
        let mut log = TabLog::default();
        log.ingest(entries, vec![], true);
        log
    }

    fn texts(view: &View<'_, ConsoleRow>) -> Vec<String> {
        view.rows.iter().map(|r| r.message.clone()).collect()
    }

    #[test]
    fn identical_messages_in_a_row_collapse_with_a_count() {
        let log = log_with(vec![
            entry(1, ConsoleLevel::Warn, "slow"),
            entry(2, ConsoleLevel::Warn, "slow"),
            entry(3, ConsoleLevel::Warn, "slow"),
            entry(4, ConsoleLevel::Log, "slow"),
            entry(5, ConsoleLevel::Warn, "slow"),
        ]);
        let view = log.console_view(LevelFilter::All, "", 50);
        assert_eq!(view.rows.len(), 3, "three runs, not five rows");
        assert_eq!(view.rows[0].count, 3);
        assert_eq!(view.rows[0].first_ms, 1);
        assert_eq!(view.rows[0].last_ms, 3);
        assert_eq!(view.rows[1].count, 1);
        // The chip counts messages, not rows.
        assert_eq!(log.level_count(LevelFilter::Warnings), 4);
        assert_eq!(log.level_count(LevelFilter::All), 5);
    }

    #[test]
    fn the_same_text_from_another_source_is_not_collapsed() {
        let mut log = TabLog::default();
        let mut a = entry(1, ConsoleLevel::Error, "boom");
        a.source = Some("a.js:1:1".into());
        let mut b = a.clone();
        b.source = Some("b.js:2:2".into());
        log.push_message(a);
        log.push_message(b);
        assert_eq!(log.console_len(), 2);
    }

    #[test]
    fn level_filters_match_what_devtools_calls_them() {
        let log = log_with(vec![
            entry(1, ConsoleLevel::Error, "e"),
            entry(2, ConsoleLevel::Warn, "w"),
            entry(3, ConsoleLevel::Log, "l"),
            entry(4, ConsoleLevel::Info, "i"),
            entry(5, ConsoleLevel::Debug, "d"),
        ]);
        let shown = |f| texts(&log.console_view(f, "", 50));
        assert_eq!(shown(LevelFilter::All), ["e", "w", "l", "i", "d"]);
        assert_eq!(shown(LevelFilter::Errors), ["e"]);
        assert_eq!(shown(LevelFilter::Warnings), ["w"]);
        assert_eq!(shown(LevelFilter::Info), ["l", "i"]);
        assert_eq!(shown(LevelFilter::Debug), ["d"]);
        assert_eq!(log.level_count(LevelFilter::Info), 2);
    }

    #[test]
    fn search_is_case_insensitive_and_covers_the_source() {
        let mut log = TabLog::default();
        log.push_message(entry(1, ConsoleLevel::Log, "Loaded Widget"));
        let mut e = entry(2, ConsoleLevel::Error, "nope");
        e.source = Some("https://cdn.example/App.js:3:9".into());
        log.push_message(e);
        assert_eq!(
            texts(&log.console_view(LevelFilter::All, "widget", 9)),
            ["Loaded Widget"]
        );
        assert_eq!(
            texts(&log.console_view(LevelFilter::All, "APP.JS", 9)),
            ["nope"]
        );
        assert!(log
            .console_view(LevelFilter::All, "absent", 9)
            .rows
            .is_empty());
        // Search and level combine.
        assert!(log
            .console_view(LevelFilter::Warnings, "widget", 9)
            .rows
            .is_empty());
    }

    #[test]
    fn contains_ci_handles_ascii_and_non_ascii() {
        assert!(contains_ci("Hello World", "lo wo"));
        assert!(!contains_ci("Hello", "world"));
        assert!(contains_ci("anything", ""));
        assert!(contains_ci("Ünïcode ÉCOLE", "école"));
        assert!(!contains_ci("ab", "abc"));
    }

    #[test]
    fn only_the_newest_rows_are_drawn_and_the_rest_counted() {
        let log = log_with(
            (0..10)
                .map(|i| entry(i, ConsoleLevel::Log, &format!("m{i}")))
                .collect(),
        );
        let view = log.console_view(LevelFilter::All, "", 4);
        assert_eq!(texts(&view), ["m6", "m7", "m8", "m9"]);
        assert_eq!(view.hidden, 6);
    }

    #[test]
    fn the_filter_cache_stays_correct_as_rows_arrive_and_age_out() {
        let mut log = TabLog::default();
        log.push_message(entry(1, ConsoleLevel::Error, "a"));
        assert_eq!(log.console_view(LevelFilter::Errors, "", 9).rows.len(), 1);
        log.push_message(entry(2, ConsoleLevel::Log, "b"));
        log.push_message(entry(3, ConsoleLevel::Error, "c"));
        assert_eq!(
            texts(&log.console_view(LevelFilter::Errors, "", 9)),
            ["a", "c"]
        );
        // Changing the filter and changing it back rescans from scratch.
        assert_eq!(texts(&log.console_view(LevelFilter::Info, "", 9)), ["b"]);
        assert_eq!(
            texts(&log.console_view(LevelFilter::Errors, "", 9)),
            ["a", "c"]
        );
        // Clearing resets what is cached.
        log.clear_console();
        assert!(log.console_view(LevelFilter::Errors, "", 9).rows.is_empty());
        log.push_message(entry(4, ConsoleLevel::Error, "d"));
        assert_eq!(texts(&log.console_view(LevelFilter::Errors, "", 9)), ["d"]);
    }

    #[test]
    fn the_log_is_bounded_and_the_cache_follows_the_front() {
        let mut log = TabLog::default();
        for i in 0..(MAX_ENTRIES + 25) {
            log.push_message(entry(i as u64, ConsoleLevel::Log, &format!("m{i}")));
            if i % 1000 == 0 {
                let _ = log.console_view(LevelFilter::All, "", 5);
            }
        }
        assert_eq!(log.console_len(), MAX_ENTRIES);
        let view = log.console_view(LevelFilter::All, "", 3);
        assert_eq!(
            texts(&view),
            [
                format!("m{}", MAX_ENTRIES + 22),
                format!("m{}", MAX_ENTRIES + 23),
                format!("m{}", MAX_ENTRIES + 24)
            ]
        );
        assert_eq!(view.hidden, MAX_ENTRIES - 3);
        assert_eq!(log.level_count(LevelFilter::All), MAX_ENTRIES);
    }

    #[test]
    fn a_huge_message_is_capped_and_the_log_has_a_byte_budget() {
        let mut log = TabLog::default();
        log.push_message(entry(
            1,
            ConsoleLevel::Log,
            &"x".repeat(MAX_MESSAGE_CHARS * 3),
        ));
        let view = log.console_view(LevelFilter::All, "", 1);
        let kept = &view.rows[0].message;
        assert!(kept.chars().count() < MAX_MESSAGE_CHARS + 60);
        assert!(kept.ends_with("not kept)"));

        // Distinct big messages age out by bytes before they hit the row cap.
        for i in 0..4_000 {
            log.push_message(entry(
                i,
                ConsoleLevel::Log,
                &format!("{i}{}", "y".repeat(MAX_MESSAGE_CHARS)),
            ));
        }
        assert!(log.console_bytes <= MAX_LOG_BYTES);
        assert!(log.console_len() < 4_000);
    }

    #[test]
    fn a_navigation_clears_the_logs_unless_they_are_preserved() {
        let mut log = TabLog::default();
        log.ingest(
            vec![entry(10, ConsoleLevel::Error, "old page")],
            vec![net(10, "Image", "https://a.example/x.png", false)],
            false,
        );
        assert_eq!(log.console_len(), 1);
        // The next batch carries a main-frame request at t=100. A message the
        // old page logged at t=90 (same batch) goes with it; one at t=120 stays.
        let added = log.ingest(
            vec![
                entry(90, ConsoleLevel::Error, "old, late"),
                entry(120, ConsoleLevel::Warn, "new page"),
            ],
            vec![
                net(100, "Document", "https://b.example/", true),
                net(110, "Script", "https://b.example/app.js", false),
            ],
            false,
        );
        assert_eq!(
            added,
            Ingested {
                console: 1,
                net: 2,
                errors: 0
            }
        );
        assert_eq!(log.nav_ms, Some(100));
        assert_eq!(
            log.console_rows()
                .map(|r| r.message.as_str())
                .collect::<Vec<_>>(),
            ["new page"]
        );
        assert_eq!(log.net_len(), 2);
        assert_eq!(log.level_count(LevelFilter::Errors), 0);
    }

    #[test]
    fn preserving_the_log_keeps_everything_and_marks_the_navigation() {
        let mut log = TabLog::default();
        log.ingest(vec![entry(10, ConsoleLevel::Error, "before")], vec![], true);
        log.ingest(
            vec![entry(120, ConsoleLevel::Log, "after")],
            vec![net(100, "Document", "https://b.example/", true)],
            true,
        );
        let rows: Vec<_> = log.console_rows().collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].kind, RowKind::Navigation);
        assert_eq!(rows[1].message, "https://b.example/");
        // A marker is not a message.
        assert_eq!(log.level_count(LevelFilter::All), 2);
        // And it shows through any filter.
        assert_eq!(log.console_view(LevelFilter::Errors, "", 9).rows.len(), 2);
    }

    #[test]
    fn a_subframe_document_is_not_a_navigation() {
        let mut log = TabLog::default();
        log.ingest(vec![entry(1, ConsoleLevel::Log, "keep")], vec![], false);
        log.ingest(
            vec![],
            vec![net(5, "IFrame", "https://ads.example/", false)],
            false,
        );
        assert_eq!(log.console_len(), 1);
        assert_eq!(log.nav_ms, None);
    }

    #[test]
    fn requests_are_grouped_filtered_and_counted() {
        let mut log = TabLog::default();
        log.ingest(
            vec![],
            vec![
                net(1, "Document", "https://a.example/", true),
                net(2, "Script", "https://a.example/app.js", false),
                net(3, "Style", "https://a.example/site.css", false),
                net(4, "Image", "https://a.example/Logo.png", false),
                net(5, "None", "https://a.example/api/items", false),
                net(6, "Font", "https://a.example/f.woff2", false),
            ],
            true,
        );
        assert_eq!(log.kind_count(NetFilter::All), 6);
        assert_eq!(log.kind_count(NetFilter::Kind(NetKind::Other)), 1);
        let urls = |f, q: &str| -> Vec<String> {
            log.net_view(f, q, 50)
                .rows
                .iter()
                .map(|r| r.url.clone())
                .collect()
        };
        assert_eq!(
            urls(NetFilter::Kind(NetKind::Script), ""),
            ["https://a.example/app.js"]
        );
        assert_eq!(urls(NetFilter::All, "logo"), ["https://a.example/Logo.png"]);
        assert_eq!(
            urls(NetFilter::Kind(NetKind::Style), "logo"),
            Vec::<String>::new()
        );
        assert_eq!(NetKind::of("Frame"), NetKind::Document);
        assert_eq!(NetKind::of("Video"), NetKind::Media);
        assert_eq!(NetKind::of("anything else"), NetKind::Other);
    }

    #[test]
    fn request_times_read_relative_to_the_navigation() {
        assert_eq!(net_time(1_150, Some(1_000)), "+150 ms");
        assert_eq!(net_time(31_000, Some(1_000)), "+30.0 s");
        // Before any navigation, or from before it: the wall clock.
        assert_eq!(net_time(0, None), clock_ms(0));
        assert_eq!(net_time(500, Some(1_000)), clock_ms(500));
    }

    #[test]
    fn typed_code_and_its_result_are_rows_and_errors_count() {
        let mut log = TabLog::default();
        log.push_input("1 + 1");
        log.push_result("2");
        log.push_message(entry(
            1,
            ConsoleLevel::Error,
            "ReferenceError: x is not defined",
        ));
        let kinds: Vec<_> = log.console_rows().map(|r| r.kind).collect();
        assert_eq!(kinds, [RowKind::Input, RowKind::Result, RowKind::Message]);
        assert_eq!(log.error_count(), 1);
    }

    #[test]
    fn engine_events_are_bounded_and_carry_their_details() {
        let mut log = EngineLog::default();
        log.push_panic(PanicNote {
            at_ms: 5,
            thread: "Script#3".into(),
            message: "boom".into(),
            location: "a.rs:1:2".into(),
        });
        log.push_crash(
            1,
            &CrashNote {
                at_ms: 6,
                reason: "script thread died".into(),
                backtrace: Some("0: foo\n1: bar".into()),
            },
        );
        let events: Vec<_> = log.events().iter().collect();
        assert_eq!(events[0].kind, EngineKind::Panic);
        assert_eq!(events[1].tab, "tab 2");
        assert!(engine_text(events[1]).contains("1: bar"));
        assert!(engine_text(events[0]).contains("at a.rs:1:2"));
        for i in 0..(MAX_ENGINE_EVENTS + 5) {
            log.push_panic(PanicNote {
                at_ms: i as u64,
                thread: "t".into(),
                message: "m".into(),
                location: String::new(),
            });
        }
        assert_eq!(log.len(), MAX_ENGINE_EVENTS);
    }

    #[test]
    fn history_walks_back_and_forward_and_restores_the_draft() {
        let mut h = InputHistory::default();
        assert_eq!(h.older("x"), None);
        h.push("a");
        h.push("b");
        h.push("b"); // consecutive duplicates are not repeated
        assert_eq!(h.older("draft").as_deref(), Some("b"));
        assert_eq!(h.older("ignored").as_deref(), Some("a"));
        assert_eq!(
            h.older("ignored").as_deref(),
            Some("a"),
            "stops at the oldest"
        );
        assert_eq!(h.newer().as_deref(), Some("b"));
        assert_eq!(
            h.newer().as_deref(),
            Some("draft"),
            "back to what was typed"
        );
        assert_eq!(h.newer(), None, "nothing newer than the draft");
        h.push("c");
        assert_eq!(h.older("").as_deref(), Some("c"));
    }

    #[test]
    fn history_is_bounded() {
        let mut h = InputHistory::default();
        for i in 0..(HISTORY_MAX + 10) {
            h.push(&i.to_string());
        }
        assert_eq!(h.entries.len(), HISTORY_MAX);
        assert_eq!(h.entries.last().map(String::as_str), Some("109"));
    }

    #[test]
    fn engine_values_read_like_devtools_values() {
        assert_eq!(pretty_js_value("Undefined"), "undefined");
        assert_eq!(pretty_js_value("Null"), "null");
        assert_eq!(pretty_js_value("Boolean(true)"), "true");
        assert_eq!(pretty_js_value("Number(2.0)"), "2");
        assert_eq!(pretty_js_value("Number(2.5)"), "2.5");
        assert_eq!(pretty_js_value("String(\"hi\\nthere\")"), "\"hi\nthere\"");
        assert_eq!(
            pretty_js_value("Array([Number(1.0)])"),
            "Array([Number(1.0)])"
        );
    }

    #[test]
    fn only_warnings_and_errors_are_mirrored_to_the_log_file() {
        assert_eq!(stderr_line(0, ConsoleLevel::Log, "hi"), None);
        assert_eq!(stderr_line(0, ConsoleLevel::Debug, "hi"), None);
        assert_eq!(
            stderr_line(2, ConsoleLevel::Warn, "careful"),
            Some("[console:warn] tab 3: careful".to_string())
        );
        let long = format!(
            "Error at https://cdn.example/{}.js:1:2 {}",
            "a".repeat(300),
            "x".repeat(900)
        );
        let line = stderr_line(0, ConsoleLevel::Error, &long).expect("error is mirrored");
        assert!(line.starts_with("[console:error] tab 1: "));
        assert!(
            line.chars().count() < 460,
            "capped: {}",
            line.chars().count()
        );
        assert!(!line.contains(&"a".repeat(300)), "the URL is shortened");
        let multi = stderr_line(0, ConsoleLevel::Error, "one\ntwo").unwrap();
        assert!(!multi.contains('\n'), "one line per message in the log");
    }

    #[test]
    fn the_log_folder_follows_the_shells_rule_on_each_platform() {
        let home = Some(PathBuf::from("/home/ann"));
        assert_eq!(
            log_dir_for(HostOs::Mac, home.clone(), None, None),
            Some(PathBuf::from("/home/ann/Library/Logs/Ferrite"))
        );
        assert_eq!(
            log_dir_for(HostOs::Other, home.clone(), None, None),
            Some(PathBuf::from("/home/ann/.local/state/ferrite"))
        );
        assert_eq!(
            log_dir_for(
                HostOs::Other,
                home.clone(),
                Some(PathBuf::from("/state")),
                None
            ),
            Some(PathBuf::from("/state/ferrite"))
        );
        assert_eq!(
            log_dir_for(HostOs::Windows, home, None, Some(PathBuf::from("C:/Local"))),
            Some(PathBuf::from("C:/Local/Ferrite/logs"))
        );
        assert_eq!(log_dir_for(HostOs::Mac, None, None, None), None);
        assert_eq!(log_dir_for(HostOs::Windows, None, None, None), None);
    }

    #[test]
    fn the_folder_opens_with_the_platforms_own_command() {
        assert_eq!(open_command(HostOs::Mac), "open");
        assert_eq!(open_command(HostOs::Windows), "explorer");
        assert_eq!(open_command(HostOs::Other), "xdg-open");
    }

    #[test]
    fn saving_a_log_writes_the_file_and_creates_the_folder() {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-devtools-test-{}-{}",
            std::process::id(),
            ferrite_servo::diag::now_ms()
        ));
        let path = save_log(&dir.join("nested"), "a.log", "hello\n").expect("saved");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
        let _ = std::fs::remove_dir_all(&dir);
        let name = save_file_name(1, DevTab::Console);
        assert!(name.starts_with("ferrite-console-tab2-") && name.ends_with(".log"));
    }

    #[test]
    fn exported_lines_carry_time_level_text_source_and_count() {
        let mut log = TabLog::default();
        let mut e = entry(0, ConsoleLevel::Error, "boom");
        e.source = Some("a.js:1:2".into());
        log.push_message(e.clone());
        log.push_message(e);
        let row = log.console_rows().next().unwrap();
        let line = console_line(row);
        assert!(line.contains("ERROR boom"), "{line}");
        assert!(line.contains("(a.js:1:2)"), "{line}");
        assert!(line.contains("(x2)"), "{line}");
    }

    #[test]
    fn the_console_counts_as_pinned_only_at_the_bottom() {
        assert!(is_pinned(1.0));
        assert!(is_pinned(0.98));
        assert!(!is_pinned(0.5));
    }
}
