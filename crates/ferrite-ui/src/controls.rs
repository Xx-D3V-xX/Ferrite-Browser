//! What a page asks the browser to show: a `<select>`'s list, `alert()` /
//! `confirm()` / `prompt()`, a colour or file picker, a context menu.
//!
//! The engine hands each of these over as plain data (`diag::PageControl`) and
//! waits for one answer (`diag::ControlAnswer`); until it gets one the page is
//! blocked, so every control here can always be answered or dismissed, and is
//! answered exactly once (`TabDiag::control` is `take`n before the engine is
//! told).
//!
//! **The page's text is untrusted.** A dialog's message, an option's label or a
//! menu row is drawn as plain text, never rich text, with control and bidi
//! characters removed and lengths capped, and dialogs are framed as coming from
//! the page (its host named, a line saying so) so one cannot pass for browser UI.
//!
//! Layout: the overlays are layers inside the page area's own `stack`, so an
//! anchor (device pixels in the page's frame) is placed by dividing by the
//! display scale; no window offset is involved. While a control is pending the
//! page receives no pointer or key input (see `lib.rs`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use ferrite_servo::diag::{
    parse_hex_color, ControlAnswer, DeviceRect, DialogKind, MenuItemView, PageControl,
    SelectOptionView,
};
use ferrite_servo::session::{PageKey, PageKeyEvent, PageNamedKey};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, stack, text, text_editor, text_input,
    Space,
};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Padding, Point, Rectangle, Size, Theme,
};

use crate::icons::{icon, Icon};
use crate::tokens::{
    accent_btn_style, hover_bg, outline_btn_style, popover_style, shadow_popover, tint, RADIUS_LG,
    RADIUS_MD, RADIUS_SM, SP_LG, SP_MD, SP_SM, SP_XS, TEXT_BODY, TEXT_CAPTION, TEXT_SMALL,
    TEXT_TITLE,
};
use crate::{font_weight, FerriteBrowser, FerriteBrowserMessage, Palette};

/// Height of one row of a popup list (options, group labels, menu items).
pub(crate) const ROW_H: f32 = 28.0;
/// The most rows a popup shows before it scrolls.
const MAX_VISIBLE_ROWS: usize = 10;
/// Option labels and menu labels are cut to this many characters.
const LABEL_CHARS: usize = 160;
/// A dialog's message is cut to this many characters.
const MESSAGE_CHARS: usize = 2_000;
/// A prompt's reply is cut to this many characters.
const REPLY_CHARS: usize = 2_000;
/// The most options a popup draws (the rest cannot be reached by clicking, and
/// the list says so).
const MAX_OPTIONS: usize = 600;
/// Rows drawn beyond the visible ones, so a quick scroll never shows a gap.
const OVERSCAN: usize = 6;
/// Space between an anchor and the popup that opens from it.
const GAP: f32 = 2.0;
/// Margin kept between a popup and the page area's edge.
const EDGE: f32 = 6.0;
const POPUP_PAD: f32 = 4.0;
/// Height of a multi-select popup's Apply / Cancel footer.
const FOOTER_H: f32 = 44.0;

pub(crate) const INPUT_ID: &str = "ferrite_control_input";

/// A small, fixed palette for the colour picker.
const SWATCHES: [&str; 16] = [
    "#000000", "#5f6368", "#9aa0a6", "#ffffff", "#d93025", "#e8710a", "#f9ab00", "#188038",
    "#12b5cb", "#1a73e8", "#5b3cc4", "#a142f4", "#e52592", "#8d5524", "#f6c6a7", "#b7e1cd",
];

// ── Untrusted text ───────────────────────────────────────────────────────

/// A page's text made safe to show: carriage returns folded into newlines,
/// control characters (other than newline and tab) dropped, bidirectional and
/// zero-width controls dropped (they can make text read as something else),
/// and the length capped.
pub(crate) fn sanitize_untrusted(raw: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(max_chars + 4));
    let mut count = 0usize;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                '\n'
            }
            '\t' => ' ',
            other => other,
        };
        let hidden = (c.is_control() && c != '\n')
            || matches!(
                c,
                '\u{200B}'..='\u{200F}'
                    | '\u{2028}'..='\u{2029}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
            );
        if hidden {
            continue;
        }
        if count == max_chars {
            out.push('\u{2026}');
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// The host of the page a control belongs to, for framing its dialogs.
pub(crate) fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this page".to_string())
}

// ── Geometry ─────────────────────────────────────────────────────────────

/// An engine rectangle (device pixels in the page's frame) as logical points in
/// the page area.
pub(crate) fn anchor_to_logical(anchor: DeviceRect, scale: f32) -> Rectangle {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    Rectangle {
        x: anchor.x / scale,
        y: anchor.y / scale,
        width: anchor.width / scale,
        height: anchor.height / scale,
    }
}

/// Where a popup of `popup` size goes for `anchor` in an `area`: under the
/// anchor, left edges aligned; above it when it would not fit below and does
/// above; and always inside the area.
pub(crate) fn place_popup(anchor: Rectangle, popup: Size, area: Size) -> Point {
    let below = anchor.y + anchor.height + GAP;
    let above = anchor.y - GAP - popup.height;
    let y = if below + popup.height <= area.height - EDGE {
        below
    } else if above >= EDGE {
        above
    } else {
        (area.height - popup.height - EDGE).max(0.0)
    };
    let x = anchor
        .x
        .min(area.width - popup.width - EDGE)
        .max(EDGE.min(area.width.max(0.0)));
    Point::new(x, y.max(0.0))
}

/// The height of a list that shows `rows` rows in a page area `area_height`
/// tall, leaving room for `footer`.
pub(crate) fn list_height(rows: usize, area_height: f32, footer: f32) -> f32 {
    let wanted = rows.min(MAX_VISIBLE_ROWS) as f32 * ROW_H;
    let room = (area_height - 2.0 * EDGE - 2.0 * POPUP_PAD - footer).max(ROW_H);
    wanted.min(room).max(ROW_H)
}

/// The scroll offset that brings the row at `top`..`top + ROW_H` into a
/// viewport `viewport_h` tall showing from `scroll_y`, or `None` if it is
/// already fully visible.
pub(crate) fn scroll_to_reveal(top: f32, scroll_y: f32, viewport_h: f32) -> Option<f32> {
    let bottom = top + ROW_H;
    if top < scroll_y {
        Some(top.max(0.0))
    } else if bottom > scroll_y + viewport_h {
        Some((bottom - viewport_h).max(0.0))
    } else {
        None
    }
}

/// The rows to build for a list scrolled to `scroll_y` in a `viewport_h`
/// viewport: `start..end`, with overscan, within `total`.
pub(crate) fn window_rows(scroll_y: f32, viewport_h: f32, total: usize) -> (usize, usize) {
    let first = (scroll_y.max(0.0) / ROW_H) as usize;
    let visible = (viewport_h / ROW_H).ceil() as usize + 1;
    let start = first.saturating_sub(OVERSCAN).min(total);
    let end = (first + visible + OVERSCAN).min(total);
    (start, end)
}

// ── Select ───────────────────────────────────────────────────────────────

/// One line of a select's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectRow {
    Group(String),
    /// The option at this position in `options`.
    Option(usize),
}

/// The popup's own state: which row the keyboard is on, what is ticked (for a
/// multi-select), and how far the list is scrolled.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SelectState {
    pub options: Vec<SelectOptionView>,
    pub multiple: bool,
    pub rows: Vec<SelectRow>,
    /// The row each option is on.
    row_of: Vec<usize>,
    pub highlighted: Option<usize>,
    pub checked: BTreeSet<usize>,
    pub scroll_y: f32,
    pub viewport_h: f32,
    /// Options past `MAX_OPTIONS`, not drawn.
    pub omitted: usize,
}

impl SelectState {
    pub(crate) fn new(
        mut options: Vec<SelectOptionView>,
        multiple: bool,
        area_height: f32,
    ) -> Self {
        let omitted = options.len().saturating_sub(MAX_OPTIONS);
        options.truncate(MAX_OPTIONS);
        let mut rows = Vec::with_capacity(options.len() + 8);
        let mut row_of = Vec::with_capacity(options.len());
        let mut group: Option<&str> = None;
        for (pos, option) in options.iter().enumerate() {
            if option.group.as_deref() != group {
                group = option.group.as_deref();
                if let Some(label) = group {
                    rows.push(SelectRow::Group(sanitize_untrusted(label, LABEL_CHARS)));
                }
            }
            row_of.push(rows.len());
            rows.push(SelectRow::Option(pos));
        }
        let checked = options
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.index)
            .collect();
        let highlighted = options
            .iter()
            .position(|o| o.selected && !o.disabled)
            .or_else(|| options.iter().position(|o| !o.disabled));
        let footer = if multiple { FOOTER_H } else { 0.0 };
        let mut state = Self {
            viewport_h: list_height(rows.len(), area_height, footer),
            options,
            multiple,
            rows,
            row_of,
            highlighted,
            checked,
            scroll_y: 0.0,
            omitted,
        };
        // Open with the chosen option in view.
        if let Some(pos) = state.highlighted {
            let top = state.row_of[pos] as f32 * ROW_H;
            if let Some(y) = scroll_to_reveal(top, 0.0, state.viewport_h) {
                state.scroll_y = y;
            }
        }
        state
    }

    /// The option `delta` enabled options away from `from` (negative is up),
    /// stopping at the ends rather than wrapping. `from` of `None` starts at
    /// the first or last enabled option.
    pub(crate) fn step(
        options: &[SelectOptionView],
        from: Option<usize>,
        delta: i32,
    ) -> Option<usize> {
        let enabled: Vec<usize> = options
            .iter()
            .enumerate()
            .filter(|(_, o)| !o.disabled)
            .map(|(i, _)| i)
            .collect();
        if enabled.is_empty() {
            return None;
        }
        let at = from.and_then(|f| enabled.iter().position(|e| *e == f));
        let next = match (at, delta.signum()) {
            (None, d) if d < 0 => enabled.len() - 1,
            (None, _) => 0,
            (Some(i), _) => {
                let moved = i as i64 + i64::from(delta);
                moved.clamp(0, enabled.len() as i64 - 1) as usize
            }
        };
        Some(enabled[next])
    }

    /// Moves the keyboard highlight; returns the scroll offset to apply if the
    /// row left the viewport.
    fn move_highlight(&mut self, delta: i32) -> Option<f32> {
        let next = Self::step(&self.options, self.highlighted, delta)?;
        self.highlighted = Some(next);
        self.reveal(next)
    }

    fn jump_to(&mut self, last: bool) -> Option<f32> {
        let target = if last {
            self.options.iter().rposition(|o| !o.disabled)
        } else {
            self.options.iter().position(|o| !o.disabled)
        }?;
        self.highlighted = Some(target);
        self.reveal(target)
    }

    fn reveal(&mut self, pos: usize) -> Option<f32> {
        let top = self.row_of.get(pos).copied()? as f32 * ROW_H;
        let y = scroll_to_reveal(top, self.scroll_y, self.viewport_h)?;
        self.scroll_y = y;
        Some(y)
    }

    /// The answer for picking option `pos` in a single-choice list.
    fn choose(&self, pos: usize) -> Option<ControlAnswer> {
        let option = self.options.get(pos)?;
        (!option.disabled).then(|| ControlAnswer::Select(vec![option.index]))
    }

    fn toggle(&mut self, pos: usize) {
        let Some(option) = self.options.get(pos) else {
            return;
        };
        if option.disabled {
            return;
        }
        if !self.checked.remove(&option.index) {
            self.checked.insert(option.index);
        }
    }

    fn apply(&self) -> ControlAnswer {
        ControlAnswer::Select(self.checked.iter().copied().collect())
    }
}

// ── Dialog ───────────────────────────────────────────────────────────────

/// The answer for a dialog the person accepted (`ok`) or cancelled.
pub(crate) fn dialog_answer(kind: DialogKind, ok: bool, reply: &str) -> ControlAnswer {
    match (kind, ok) {
        (DialogKind::Alert, _) => ControlAnswer::Accept(None),
        (DialogKind::Confirm, true) => ControlAnswer::Accept(None),
        (DialogKind::Prompt, true) => ControlAnswer::Accept(Some(reply.to_string())),
        (_, false) => ControlAnswer::Dismiss,
    }
}

// ── Colour ───────────────────────────────────────────────────────────────

/// `#rrggbb` (lowercase) for what was typed: `#f80`, `ff8000` and ` #FF8000 `
/// all work.
pub(crate) fn normalize_hex(typed: &str) -> Option<String> {
    let typed = typed.trim();
    let with_hash = if typed.starts_with('#') {
        typed.to_string()
    } else {
        format!("#{typed}")
    };
    let (r, g, b) = parse_hex_color(&with_hash)?;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

fn color_of(hex: &str) -> Option<Color> {
    let (r, g, b) = parse_hex_color(hex)?;
    Some(Color::from_rgb8(r, g, b))
}

// ── File ─────────────────────────────────────────────────────────────────

/// One path the person gave, and whether it can be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathCheck {
    pub path: PathBuf,
    pub problem: Option<&'static str>,
}

/// Splits what was typed into paths: one per line; a line that is not itself a
/// file but has commas is split at them; surrounding quotes are removed and a
/// leading `~/` is the home directory.
pub(crate) fn parse_paths(
    text: &str,
    is_file: impl Fn(&std::path::Path) -> bool,
    home: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let clean = |piece: &str| -> Option<PathBuf> {
        let piece = piece.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if piece.is_empty() {
            return None;
        }
        Some(match (piece.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(piece),
        })
    };
    let mut paths = Vec::new();
    for line in text.lines() {
        match clean(line) {
            Some(whole) if is_file(&whole) || !line.contains(',') => paths.push(whole),
            Some(_) => paths.extend(line.split(',').filter_map(clean)),
            None => {}
        }
    }
    paths
}

/// Checks each path: it must be a file, and only one is allowed unless the
/// page's input takes several.
pub(crate) fn check_paths(
    paths: Vec<PathBuf>,
    multiple: bool,
    is_file: impl Fn(&std::path::Path) -> bool,
) -> Vec<PathCheck> {
    paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            let problem = if !multiple && i > 0 {
                Some("only one file can be chosen here")
            } else if !is_file(&path) {
                Some("no such file")
            } else {
                None
            };
            PathCheck { path, problem }
        })
        .collect()
}

fn fs_is_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// The file picker's own state.
pub(crate) struct FileState {
    pub multiple: bool,
    pub accept: Vec<String>,
    /// The multi-line field (several files) or the single-line text (one).
    pub content: text_editor::Content,
    pub single: String,
    pub checks: Vec<PathCheck>,
}

impl FileState {
    fn new(multiple: bool, accept: Vec<String>) -> Self {
        Self {
            multiple,
            accept,
            content: text_editor::Content::new(),
            single: String::new(),
            checks: Vec::new(),
        }
    }

    fn text(&self) -> String {
        if self.multiple {
            self.content.text()
        } else {
            self.single.clone()
        }
    }

    fn recheck(&mut self) {
        let home = dirs::home_dir();
        let paths = parse_paths(&self.text(), fs_is_file, home.as_deref());
        self.checks = check_paths(paths, self.multiple, fs_is_file);
    }

    /// The files to answer with, when there is at least one and all are usable.
    fn answer(&self) -> Option<ControlAnswer> {
        (!self.checks.is_empty() && self.checks.iter().all(|c| c.problem.is_none()))
            .then(|| ControlAnswer::Files(self.checks.iter().map(|c| c.path.clone()).collect()))
    }
}

// ── Menu ─────────────────────────────────────────────────────────────────

/// The row `delta` rows from `from` that can be picked, stopping at the ends.
pub(crate) fn menu_step(items: &[MenuItemView], from: Option<usize>, delta: i32) -> Option<usize> {
    let pickable: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.enabled && i.index.is_some())
        .map(|(p, _)| p)
        .collect();
    if pickable.is_empty() {
        return None;
    }
    let at = from.and_then(|f| pickable.iter().position(|p| *p == f));
    let next = match (at, delta.signum()) {
        (None, d) if d < 0 => pickable.len() - 1,
        (None, _) => 0,
        (Some(i), _) => (i as i64 + i64::from(delta)).clamp(0, pickable.len() as i64 - 1) as usize,
    };
    Some(pickable[next])
}

// ── The pending control ──────────────────────────────────────────────────

pub(crate) enum ControlUi {
    Select(SelectState),
    Dialog { reply: String },
    File(FileState),
    Color { typed: String },
    Menu { highlighted: Option<usize> },
}

/// A control the engine is waiting on, with the UI state to answer it.
pub(crate) struct PendingControl {
    pub control: PageControl,
    pub ui: ControlUi,
}

/// A key the control cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Space,
}

#[derive(Debug, Clone)]
pub enum Msg {
    SelectHover(usize),
    SelectChoose(usize),
    SelectScrolled(f32),
    ReplyChanged(String),
    FileAction(text_editor::Action),
    FileTextChanged(String),
    ColorChanged(String),
    MenuHover(usize),
    MenuChoose(usize),
    /// OK / Apply / Open / Select, whatever the control's main button is.
    Accept,
    Dismiss,
    Key(ControlKey),
}

/// What handling a message came to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Nothing,
    /// Answer the page with this (and the control is finished).
    Answer(ControlAnswer),
    /// Scroll the list to this offset.
    ScrollTo(f32),
}

impl PendingControl {
    /// The UI state for `control`, in a page area `area` points square-ish
    /// (only its height matters, for sizing a list).
    pub(crate) fn new(control: PageControl, area: Size) -> Self {
        let ui = match &control {
            PageControl::Select {
                options, multiple, ..
            } => ControlUi::Select(SelectState::new(options.clone(), *multiple, area.height)),
            PageControl::Dialog { default, .. } => ControlUi::Dialog {
                reply: sanitize_untrusted(default, REPLY_CHARS),
            },
            PageControl::File { multiple, accept } => {
                ControlUi::File(FileState::new(*multiple, accept.clone()))
            }
            PageControl::Color { current, .. } => ControlUi::Color {
                typed: normalize_hex(current).unwrap_or_else(|| "#000000".to_string()),
            },
            PageControl::Menu { items, .. } => ControlUi::Menu {
                highlighted: menu_step(items, None, 1),
            },
        };
        Self { control, ui }
    }

    pub(crate) fn handle(&mut self, msg: Msg) -> Outcome {
        match (&mut self.ui, &self.control, msg) {
            (_, _, Msg::Dismiss) => Outcome::Answer(ControlAnswer::Dismiss),

            // ── select ──
            (ControlUi::Select(s), _, Msg::SelectHover(pos)) => {
                if s.options.get(pos).is_some_and(|o| !o.disabled) {
                    s.highlighted = Some(pos);
                }
                Outcome::Nothing
            }
            (ControlUi::Select(s), _, Msg::SelectChoose(pos)) => {
                if s.multiple {
                    s.highlighted = Some(pos);
                    s.toggle(pos);
                    Outcome::Nothing
                } else {
                    s.choose(pos).map_or(Outcome::Nothing, Outcome::Answer)
                }
            }
            (ControlUi::Select(s), _, Msg::SelectScrolled(y)) => {
                s.scroll_y = y;
                Outcome::Nothing
            }
            (ControlUi::Select(s), _, Msg::Key(key)) => {
                let moved = match key {
                    ControlKey::Up => s.move_highlight(-1),
                    ControlKey::Down => s.move_highlight(1),
                    ControlKey::PageUp => s.move_highlight(-(MAX_VISIBLE_ROWS as i32)),
                    ControlKey::PageDown => s.move_highlight(MAX_VISIBLE_ROWS as i32),
                    ControlKey::Home => s.jump_to(false),
                    ControlKey::End => s.jump_to(true),
                    ControlKey::Space if s.multiple => {
                        if let Some(pos) = s.highlighted {
                            s.toggle(pos);
                        }
                        None
                    }
                    ControlKey::Space | ControlKey::Enter if !s.multiple => {
                        return s
                            .highlighted
                            .and_then(|p| s.choose(p))
                            .map_or(Outcome::Nothing, Outcome::Answer);
                    }
                    ControlKey::Enter => return Outcome::Answer(s.apply()),
                    ControlKey::Space => None,
                };
                moved.map_or(Outcome::Nothing, Outcome::ScrollTo)
            }
            (ControlUi::Select(s), _, Msg::Accept) => {
                if s.multiple {
                    Outcome::Answer(s.apply())
                } else {
                    s.highlighted
                        .and_then(|p| s.choose(p))
                        .map_or(Outcome::Nothing, Outcome::Answer)
                }
            }

            // ── dialog ──
            (ControlUi::Dialog { reply }, _, Msg::ReplyChanged(text)) => {
                *reply = sanitize_untrusted(&text, REPLY_CHARS);
                Outcome::Nothing
            }
            (ControlUi::Dialog { reply }, PageControl::Dialog { kind, .. }, Msg::Accept)
            | (
                ControlUi::Dialog { reply },
                PageControl::Dialog { kind, .. },
                Msg::Key(ControlKey::Enter),
            ) => Outcome::Answer(dialog_answer(*kind, true, reply)),

            // ── file ──
            (ControlUi::File(f), _, Msg::FileAction(action)) => {
                let edits = action.is_edit();
                f.content.perform(action);
                if edits {
                    f.recheck();
                }
                Outcome::Nothing
            }
            (ControlUi::File(f), _, Msg::FileTextChanged(text)) => {
                f.single = text;
                f.recheck();
                Outcome::Nothing
            }
            (ControlUi::File(f), _, Msg::Accept) => {
                f.recheck();
                f.answer().map_or(Outcome::Nothing, Outcome::Answer)
            }

            // ── colour ──
            (ControlUi::Color { typed }, _, Msg::ColorChanged(text)) => {
                *typed = text.chars().take(9).collect();
                Outcome::Nothing
            }
            (ControlUi::Color { typed }, _, Msg::Accept | Msg::Key(ControlKey::Enter)) => {
                normalize_hex(typed).map_or(Outcome::Nothing, |hex| {
                    Outcome::Answer(ControlAnswer::Color(hex))
                })
            }

            // ── menu ──
            (
                ControlUi::Menu { highlighted },
                PageControl::Menu { items, .. },
                Msg::MenuHover(pos),
            ) => {
                if items
                    .get(pos)
                    .is_some_and(|i| i.enabled && i.index.is_some())
                {
                    *highlighted = Some(pos);
                }
                Outcome::Nothing
            }
            (ControlUi::Menu { .. }, PageControl::Menu { items, .. }, Msg::MenuChoose(pos)) => {
                items
                    .get(pos)
                    .filter(|i| i.enabled)
                    .and_then(|i| i.index)
                    .map_or(Outcome::Nothing, |i| {
                        Outcome::Answer(ControlAnswer::Menu(i))
                    })
            }
            (ControlUi::Menu { highlighted }, PageControl::Menu { items, .. }, Msg::Key(key)) => {
                match key {
                    ControlKey::Up => {
                        *highlighted = menu_step(items, *highlighted, -1).or(*highlighted);
                        Outcome::Nothing
                    }
                    ControlKey::Down => {
                        *highlighted = menu_step(items, *highlighted, 1).or(*highlighted);
                        Outcome::Nothing
                    }
                    ControlKey::Home => {
                        *highlighted = menu_step(items, None, 1).or(*highlighted);
                        Outcome::Nothing
                    }
                    ControlKey::End => {
                        *highlighted = menu_step(items, None, -1).or(*highlighted);
                        Outcome::Nothing
                    }
                    ControlKey::Enter | ControlKey::Space => highlighted
                        .and_then(|p| items.get(p))
                        .filter(|i| i.enabled)
                        .and_then(|i| i.index)
                        .map_or(Outcome::Nothing, |i| {
                            Outcome::Answer(ControlAnswer::Menu(i))
                        }),
                    ControlKey::PageUp | ControlKey::PageDown => Outcome::Nothing,
                }
            }
            (ControlUi::Menu { highlighted }, PageControl::Menu { items, .. }, Msg::Accept) => {
                highlighted
                    .and_then(|p| items.get(p))
                    .and_then(|i| i.index)
                    .map_or(Outcome::Nothing, |i| {
                        Outcome::Answer(ControlAnswer::Menu(i))
                    })
            }

            _ => Outcome::Nothing,
        }
    }
}

/// A key press, as the control's [`ControlKey`], if it is one it uses.
pub(crate) fn control_key(event: &PageKeyEvent) -> Option<ControlKey> {
    if !event.down {
        return None;
    }
    match &event.key {
        PageKey::Named(PageNamedKey::ArrowUp) => Some(ControlKey::Up),
        PageKey::Named(PageNamedKey::ArrowDown) => Some(ControlKey::Down),
        PageKey::Named(PageNamedKey::Home) => Some(ControlKey::Home),
        PageKey::Named(PageNamedKey::End) => Some(ControlKey::End),
        PageKey::Named(PageNamedKey::PageUp) => Some(ControlKey::PageUp),
        PageKey::Named(PageNamedKey::PageDown) => Some(ControlKey::PageDown),
        PageKey::Named(PageNamedKey::Enter) => Some(ControlKey::Enter),
        PageKey::Character(c) if c == " " => Some(ControlKey::Space),
        _ => None,
    }
}

// ── Applying messages to the app ─────────────────────────────────────────

/// Whether tab `tab` has a control waiting on the person.
pub(crate) fn tab_has_control(state: &FerriteBrowser, tab: usize) -> bool {
    state.tab_diag.get(tab).is_some_and(|d| d.control.is_some())
}

/// The active tab's pending control, if any.
pub(crate) fn active_control(state: &FerriteBrowser) -> Option<&PendingControl> {
    state
        .tab_diag
        .get(state.active_tab)
        .and_then(|d| d.control.as_ref())
}

/// Answers tab `tab`'s control exactly once: it is taken out of the tab before
/// the engine hears anything, so a second answer finds nothing to answer.
pub(crate) fn answer(state: &mut FerriteBrowser, tab: usize, answer: ControlAnswer) {
    let Some(pending) = state.tab_diag.get_mut(tab).and_then(|d| d.control.take()) else {
        return;
    };
    let _ = pending;
    if let Some(session) = state.servo_sessions.get_mut(&tab) {
        session.answer_control(answer);
    }
    crate::wake(state);
}

/// Dismisses tab `tab`'s control, if it has one (the person left the tab or the
/// page moved on).
pub(crate) fn dismiss_tab(state: &mut FerriteBrowser, tab: usize) {
    if tab_has_control(state, tab) {
        answer(state, tab, ControlAnswer::Dismiss);
    }
}

pub(crate) fn update(state: &mut FerriteBrowser, msg: Msg) -> iced::Task<FerriteBrowserMessage> {
    let tab = state.active_tab;
    let Some(pending) = state.tab_diag.get_mut(tab).and_then(|d| d.control.as_mut()) else {
        return iced::Task::none();
    };
    match pending.handle(msg) {
        Outcome::Nothing => iced::Task::none(),
        Outcome::Answer(a) => {
            answer(state, tab, a);
            iced::Task::none()
        }
        Outcome::ScrollTo(y) => iced::widget::scrollable::scroll_to(
            select_scroll_id(),
            scrollable::AbsoluteOffset { x: 0.0, y },
        ),
    }
}

/// A key press while a control is pending: the control's, or nothing, never
/// the page's.
pub(crate) fn on_page_key(
    state: &mut FerriteBrowser,
    event: &PageKeyEvent,
) -> iced::Task<FerriteBrowserMessage> {
    match control_key(event) {
        Some(key) => update(state, Msg::Key(key)),
        None => iced::Task::none(),
    }
}

pub(crate) fn select_scroll_id() -> scrollable::Id {
    scrollable::Id::new("ferrite_control_select")
}

/// The task that gives a freshly shown prompt or path field the keyboard.
pub(crate) fn focus_task(control: &PendingControl) -> iced::Task<FerriteBrowserMessage> {
    match &control.ui {
        ControlUi::Dialog { .. }
            if matches!(
                control.control,
                PageControl::Dialog {
                    kind: DialogKind::Prompt,
                    ..
                }
            ) =>
        {
            text_input::focus(text_input::Id::new(INPUT_ID))
        }
        ControlUi::File(f) if !f.multiple => text_input::focus(text_input::Id::new(INPUT_ID)),
        ControlUi::Color { .. } => text_input::focus(text_input::Id::new(INPUT_ID)),
        _ => iced::Task::none(),
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn control_msg(m: Msg) -> FerriteBrowserMessage {
    FerriteBrowserMessage::Control(m)
}

/// The layer for the active tab's pending control, to stack over the page.
pub(crate) fn overlay<'a>(
    state: &'a FerriteBrowser,
    area: Size,
) -> Option<Element<'a, FerriteBrowserMessage>> {
    let pending = active_control(state)?;
    let palette = state.palette();
    let host = host_of(
        state
            .tab_urls
            .get(state.active_tab)
            .map_or("", String::as_str),
    );
    let scale = state.scale_factor;
    Some(match (&pending.control, &pending.ui) {
        (PageControl::Select { anchor, .. }, ControlUi::Select(s)) => {
            select_view(palette, s, anchor_to_logical(*anchor, scale), area)
        }
        (PageControl::Dialog { kind, message, .. }, ControlUi::Dialog { reply }) => {
            dialog_view(palette, *kind, message, reply, &host)
        }
        (PageControl::File { .. }, ControlUi::File(f)) => file_view(palette, f, &host),
        (PageControl::Color { anchor, .. }, ControlUi::Color { typed }) => {
            color_view(palette, typed, anchor_to_logical(*anchor, scale), area)
        }
        (PageControl::Menu { items, anchor }, ControlUi::Menu { highlighted }) => menu_view(
            palette,
            items,
            *highlighted,
            anchor_to_logical(*anchor, scale),
            area,
        ),
        _ => return None,
    })
}

/// A layer that dismisses the control on any press outside the card.
fn click_away<'a>() -> Element<'a, FerriteBrowserMessage> {
    mouse_area(Space::new(Length::Fill, Length::Fill))
        .on_press(control_msg(Msg::Dismiss))
        .on_right_press(control_msg(Msg::Dismiss))
        .into()
}

/// A popup `card` of `size` placed at `at`, over a click-away layer.
fn popup_layer<'a>(
    card: Element<'a, FerriteBrowserMessage>,
    at: Point,
) -> Element<'a, FerriteBrowserMessage> {
    let placed = container(mouse_area(card).on_press(FerriteBrowserMessage::Noop))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: at.y,
            left: at.x,
            right: 0.0,
            bottom: 0.0,
        });
    stack([click_away(), placed.into()]).into()
}

/// A centred modal `card` over a dimmed, input-blocking layer.
fn modal_layer<'a>(card: Element<'a, FerriteBrowserMessage>) -> Element<'a, FerriteBrowserMessage> {
    let scrim = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(Color {
                    a: 0.46,
                    ..Color::BLACK
                })),
                ..container::Style::default()
            }),
    )
    .on_press(FerriteBrowserMessage::Noop);
    let centred = container(mouse_area(card).on_press(FerriteBrowserMessage::Noop))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(SP_LG);
    stack([scrim.into(), centred.into()]).into()
}

fn row_style(palette: &'static Palette, hot: bool) -> impl Fn(&Theme) -> container::Style {
    move |_: &Theme| container::Style {
        background: hot.then(|| Background::Color(hover_bg(palette))),
        border: Border {
            radius: RADIUS_SM.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

fn label_text<'a>(label: String, size: f32, color: Color) -> iced::widget::Text<'a, Theme> {
    text(label)
        .size(size)
        .color(color)
        .wrapping(text::Wrapping::None)
        .width(Length::Fill)
}

fn checkbox<'a>(
    palette: &'static Palette,
    checked: bool,
    disabled: bool,
) -> Element<'a, FerriteBrowserMessage> {
    container(if checked {
        icon(Icon::Check, 11.0, Color::WHITE)
    } else {
        Space::new(0.0, 0.0).into()
    })
    .width(Length::Fixed(16.0))
    .height(Length::Fixed(16.0))
    .center(Length::Fixed(16.0))
    .style(move |_: &Theme| container::Style {
        background: checked.then_some(Background::Color(palette.accent)),
        border: Border {
            radius: 4.0.into(),
            width: 1.5,
            color: if checked {
                palette.accent
            } else if disabled {
                tint(palette.text_dim, 0.4)
            } else {
                palette.text_dim
            },
        },
        ..container::Style::default()
    })
    .into()
}

fn select_view<'a>(
    palette: &'static Palette,
    s: &'a SelectState,
    anchor: Rectangle,
    area: Size,
) -> Element<'a, FerriteBrowserMessage> {
    let (start, end) = window_rows(s.scroll_y, s.viewport_h, s.rows.len());
    let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::with_capacity(end - start + 2);
    if start > 0 {
        rows.push(Space::with_height(Length::Fixed(start as f32 * ROW_H)).into());
    }
    for row_index in start..end {
        rows.push(match &s.rows[row_index] {
            SelectRow::Group(label) => container(
                text(label.clone())
                    .size(TEXT_CAPTION)
                    .font(font_weight(iced::font::Weight::Semibold))
                    .color(palette.text_dim)
                    .wrapping(text::Wrapping::None),
            )
            .height(Length::Fixed(ROW_H))
            .padding([0.0, SP_SM])
            .align_y(Alignment::Center)
            .into(),
            SelectRow::Option(pos) => option_row(palette, s, *pos),
        });
    }
    if end < s.rows.len() {
        rows.push(Space::with_height(Length::Fixed((s.rows.len() - end) as f32 * ROW_H)).into());
    }

    let list = scrollable(column(rows))
        .id(select_scroll_id())
        .on_scroll(|v| control_msg(Msg::SelectScrolled(v.absolute_offset().y)))
        .height(Length::Fixed(s.viewport_h));

    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![list.into()];
    if s.omitted > 0 {
        body.push(
            container(
                text(format!("{} more options not shown", s.omitted))
                    .size(TEXT_CAPTION)
                    .color(palette.text_dim),
            )
            .padding([SP_XS, SP_SM])
            .into(),
        );
    }
    let mut height = s.viewport_h + 2.0 * POPUP_PAD;
    if s.multiple {
        height += FOOTER_H;
        body.push(
            container(
                row![
                    Space::with_width(Length::Fill),
                    button(text("Cancel").size(TEXT_SMALL))
                        .padding([SP_XS + 1.0, SP_MD])
                        .style(outline_btn_style)
                        .on_press(control_msg(Msg::Dismiss)),
                    button(text("Apply").size(TEXT_SMALL))
                        .padding([SP_XS + 1.0, SP_MD])
                        .style(accent_btn_style)
                        .on_press(control_msg(Msg::Accept)),
                ]
                .spacing(SP_SM)
                .align_y(Alignment::Center),
            )
            .height(Length::Fixed(FOOTER_H))
            .padding([SP_SM, SP_SM])
            .into(),
        );
    }
    let width = anchor.width.clamp(200.0, 420.0);
    let card = container(column(body))
        .width(Length::Fixed(width))
        .padding(POPUP_PAD)
        .style(popover_style);
    popup_layer(
        card.into(),
        place_popup(anchor, Size::new(width, height), area),
    )
}

fn option_row<'a>(
    palette: &'static Palette,
    s: &'a SelectState,
    pos: usize,
) -> Element<'a, FerriteBrowserMessage> {
    let option = &s.options[pos];
    let hot = s.highlighted == Some(pos) && !option.disabled;
    let checked = if s.multiple {
        s.checked.contains(&option.index)
    } else {
        option.selected
    };
    let color = if option.disabled {
        tint(palette.text_dim, 0.7)
    } else {
        palette.text
    };
    let mark: Element<FerriteBrowserMessage> = if s.multiple {
        checkbox(palette, checked, option.disabled)
    } else if checked {
        icon(Icon::Check, 13.0, palette.accent_bright)
    } else {
        Space::with_width(Length::Fixed(13.0)).into()
    };
    let body = container(
        row![
            mark,
            label_text(
                sanitize_untrusted(&option.label, LABEL_CHARS),
                TEXT_BODY,
                color
            )
        ]
        .spacing(SP_SM)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(ROW_H))
    .padding([0.0, SP_SM])
    .align_y(Alignment::Center)
    .clip(true)
    .style(row_style(palette, hot));
    if option.disabled {
        body.into()
    } else {
        mouse_area(body)
            .on_press(control_msg(Msg::SelectChoose(pos)))
            .on_enter(control_msg(Msg::SelectHover(pos)))
            .into()
    }
}

fn menu_view<'a>(
    palette: &'static Palette,
    items: &'a [MenuItemView],
    highlighted: Option<usize>,
    anchor: Rectangle,
    area: Size,
) -> Element<'a, FerriteBrowserMessage> {
    let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::with_capacity(items.len());
    let mut height = 2.0 * POPUP_PAD;
    let mut widest = 120.0f32;
    for (pos, item) in items.iter().enumerate() {
        if item.index.is_none() {
            rows.push(
                container(Space::new(Length::Fill, Length::Fixed(1.0)))
                    .width(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(Background::Color(palette.divider)),
                        ..container::Style::default()
                    })
                    .into(),
            );
            height += 1.0;
            continue;
        }
        height += ROW_H;
        let label = sanitize_untrusted(&item.label, LABEL_CHARS);
        widest = widest.max(crate::chrome::text_width(&label, TEXT_BODY) + 2.0 * SP_MD);
        let hot = highlighted == Some(pos) && item.enabled;
        let color = if item.enabled {
            palette.text
        } else {
            tint(palette.text_dim, 0.7)
        };
        let body = container(label_text(label, TEXT_BODY, color))
            .width(Length::Fill)
            .height(Length::Fixed(ROW_H))
            .padding([0.0, SP_MD])
            .align_y(Alignment::Center)
            .clip(true)
            .style(row_style(palette, hot));
        rows.push(if item.enabled {
            mouse_area(body)
                .on_press(control_msg(Msg::MenuChoose(pos)))
                .on_enter(control_msg(Msg::MenuHover(pos)))
                .into()
        } else {
            body.into()
        });
    }
    let width = widest.clamp(160.0, 340.0);
    let card = container(column(rows).spacing(0))
        .width(Length::Fixed(width))
        .padding(POPUP_PAD)
        .style(popover_style);
    // A context menu opens at the pointer, not under an element.
    let at = place_popup(
        Rectangle {
            height: 0.0,
            ..anchor
        },
        Size::new(width, height),
        area,
    );
    popup_layer(card.into(), at)
}

fn color_view<'a>(
    palette: &'static Palette,
    typed: &'a str,
    anchor: Rectangle,
    area: Size,
) -> Element<'a, FerriteBrowserMessage> {
    let current = normalize_hex(typed);
    let valid = current.is_some();
    let swatch_color = current
        .as_deref()
        .and_then(color_of)
        .unwrap_or(Color::TRANSPARENT);
    let preview =
        container(Space::new(Length::Fixed(40.0), Length::Fixed(30.0))).style(move |_: &Theme| {
            container::Style {
                background: Some(Background::Color(swatch_color)),
                border: Border {
                    radius: RADIUS_SM.into(),
                    width: 1.0,
                    color: palette.divider,
                },
                ..container::Style::default()
            }
        });
    let field = text_input("#rrggbb", typed)
        .id(text_input::Id::new(INPUT_ID))
        .on_input(|t| control_msg(Msg::ColorChanged(t)))
        .on_submit(control_msg(Msg::Accept))
        .size(TEXT_BODY)
        .padding([SP_XS + 1.0, SP_SM])
        .style(move |theme: &Theme, status| {
            let mut style = crate::tokens::field_style(theme, status, RADIUS_SM);
            if !valid {
                style.border.color = palette.danger;
            }
            style
        });
    let swatch_button = |hex: &'static str| {
        let fill = color_of(hex).unwrap_or(Color::BLACK);
        let chosen = current.as_deref() == Some(hex);
        button(Space::new(Length::Fixed(20.0), Length::Fixed(20.0)))
            .padding(0)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(fill)),
                border: Border {
                    radius: 4.0.into(),
                    width: if chosen || status == button::Status::Hovered {
                        2.0
                    } else {
                        1.0
                    },
                    color: if chosen {
                        palette.accent_bright
                    } else if status == button::Status::Hovered {
                        palette.text
                    } else {
                        palette.divider
                    },
                },
                ..button::Style::default()
            })
            .on_press(control_msg(Msg::ColorChanged(hex.to_string())))
    };
    let grid = column![
        row(SWATCHES[..8].iter().map(|h| swatch_button(h).into())).spacing(SP_XS + 2.0),
        row(SWATCHES[8..].iter().map(|h| swatch_button(h).into())).spacing(SP_XS + 2.0),
    ]
    .spacing(SP_XS + 2.0);
    let card = container(
        column![
            text("Choose a colour")
                .size(TEXT_BODY)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
            row![preview, field]
                .spacing(SP_SM)
                .align_y(Alignment::Center),
            grid,
            row![
                Space::with_width(Length::Fill),
                button(text("Cancel").size(TEXT_SMALL))
                    .padding([SP_XS + 1.0, SP_MD])
                    .style(outline_btn_style)
                    .on_press(control_msg(Msg::Dismiss)),
                button(text("Select").size(TEXT_SMALL))
                    .padding([SP_XS + 1.0, SP_MD])
                    .style(accent_btn_style)
                    .on_press_maybe(valid.then(|| control_msg(Msg::Accept))),
            ]
            .spacing(SP_SM),
        ]
        .spacing(SP_MD),
    )
    .width(Length::Fixed(COLOR_W))
    .padding(SP_MD)
    .style(popover_style);
    popup_layer(
        card.into(),
        place_popup(anchor, Size::new(COLOR_W, COLOR_H), area),
    )
}

const COLOR_W: f32 = 248.0;
const COLOR_H: f32 = 196.0;

/// The frame every dialog the page can raise shares: neutral and raised, a
/// header that says it is the page speaking and which page, and a footer that
/// says so again, so none can pass for the browser's own UI.
fn page_card<'a>(
    palette: &'static Palette,
    title: &'static str,
    host: &str,
    body: Element<'a, FerriteBrowserMessage>,
    actions: Element<'a, FerriteBrowserMessage>,
) -> Element<'a, FerriteBrowserMessage> {
    let header = row![
        icon(Icon::Globe, 15.0, palette.text_dim),
        column![
            text(title)
                .size(TEXT_TITLE)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
            text(host.to_string())
                .size(TEXT_SMALL)
                .color(palette.text_dim)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(1),
    ]
    .spacing(SP_SM)
    .align_y(Alignment::Center);
    container(
        column![
            header,
            body,
            text("This message comes from the web page, not from Ferrite.")
                .size(TEXT_CAPTION)
                .color(palette.text_dim),
            actions,
        ]
        .spacing(SP_MD),
    )
    .width(Length::Fixed(DIALOG_W))
    .padding(SP_LG)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(palette.raised)),
        border: Border {
            radius: RADIUS_LG.into(),
            width: 1.0,
            color: palette.divider,
        },
        shadow: shadow_popover(),
        ..container::Style::default()
    })
    .into()
}

const DIALOG_W: f32 = 420.0;

fn quoted_box<'a>(
    palette: &'static Palette,
    content: Element<'a, FerriteBrowserMessage>,
) -> Element<'a, FerriteBrowserMessage> {
    container(content)
        .width(Length::Fill)
        .padding(SP_MD)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(palette.input)),
            border: Border {
                radius: RADIUS_MD.into(),
                width: 1.0,
                color: palette.divider,
            },
            ..container::Style::default()
        })
        .into()
}

fn buttons<'a>(
    cancel: Option<&'static str>,
    ok: &'static str,
    ok_enabled: bool,
) -> Element<'a, FerriteBrowserMessage> {
    let mut items: Vec<Element<FerriteBrowserMessage>> =
        vec![Space::with_width(Length::Fill).into()];
    if let Some(label) = cancel {
        items.push(
            button(text(label).size(TEXT_BODY))
                .padding([SP_SM - 1.0, SP_LG])
                .style(outline_btn_style)
                .on_press(control_msg(Msg::Dismiss))
                .into(),
        );
    }
    items.push(
        button(text(ok).size(TEXT_BODY))
            .padding([SP_SM - 1.0, SP_LG])
            .style(accent_btn_style)
            .on_press_maybe(ok_enabled.then(|| control_msg(Msg::Accept)))
            .into(),
    );
    row(items).spacing(SP_SM).align_y(Alignment::Center).into()
}

fn dialog_view<'a>(
    palette: &'static Palette,
    kind: DialogKind,
    message: &str,
    reply: &'a str,
    host: &str,
) -> Element<'a, FerriteBrowserMessage> {
    let shown = sanitize_untrusted(message, MESSAGE_CHARS);
    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![quoted_box(
        palette,
        container(scrollable(
            text(shown)
                .size(TEXT_BODY)
                .color(palette.text)
                .wrapping(text::Wrapping::WordOrGlyph)
                .width(Length::Fill),
        ))
        .max_height(180.0)
        .into(),
    )];
    if kind == DialogKind::Prompt {
        body.push(
            text_input("", reply)
                .id(text_input::Id::new(INPUT_ID))
                .on_input(|t| control_msg(Msg::ReplyChanged(t)))
                .on_submit(control_msg(Msg::Accept))
                .size(TEXT_BODY)
                .padding([SP_SM - 1.0, SP_SM])
                .style(|theme: &Theme, status| crate::tokens::field_style(theme, status, RADIUS_SM))
                .into(),
        );
    }
    let title = match kind {
        DialogKind::Alert => "This page says",
        DialogKind::Confirm => "This page asks",
        DialogKind::Prompt => "This page asks you",
    };
    let actions = buttons((kind != DialogKind::Alert).then_some("Cancel"), "OK", true);
    modal_layer(page_card(
        palette,
        title,
        host,
        column(body).spacing(SP_SM).into(),
        actions,
    ))
}

fn file_view<'a>(
    palette: &'static Palette,
    f: &'a FileState,
    host: &str,
) -> Element<'a, FerriteBrowserMessage> {
    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![text(if f.multiple {
        "Ferrite has no file browser yet. Type or paste the full path of each file, one per line (or separated by commas)."
    } else {
        "Ferrite has no file browser yet. Type or paste the full path of the file."
    })
    .size(TEXT_SMALL)
    .color(palette.text_dim)
    .wrapping(text::Wrapping::Word)
    .into()];
    let field_style = |theme: &Theme, status: text_input::Status| {
        crate::tokens::field_style(theme, status, RADIUS_SM)
    };
    if f.multiple {
        body.push(
            text_editor(&f.content)
                .placeholder("/Users/me/Pictures/photo.png")
                .on_action(|a| control_msg(Msg::FileAction(a)))
                .height(Length::Fixed(96.0))
                .size(TEXT_BODY)
                .padding(SP_SM)
                .style(move |theme: &Theme, status| {
                    let focused = matches!(status, text_editor::Status::Focused);
                    let base = field_style(
                        theme,
                        if focused {
                            text_input::Status::Focused
                        } else {
                            text_input::Status::Active
                        },
                    );
                    text_editor::Style {
                        background: base.background,
                        border: base.border,
                        icon: base.icon,
                        placeholder: base.placeholder,
                        value: base.value,
                        selection: base.selection,
                    }
                })
                .into(),
        );
    } else {
        body.push(
            text_input("/Users/me/Pictures/photo.png", &f.single)
                .id(text_input::Id::new(INPUT_ID))
                .on_input(|t| control_msg(Msg::FileTextChanged(t)))
                .on_submit(control_msg(Msg::Accept))
                .size(TEXT_BODY)
                .padding([SP_SM - 1.0, SP_SM])
                .style(field_style)
                .into(),
        );
    }
    if !f.accept.is_empty() {
        let shown: Vec<String> = f
            .accept
            .iter()
            .take(12)
            .map(|a| sanitize_untrusted(a, 40))
            .collect();
        body.push(
            text(format!("The page accepts: {}", shown.join(", ")))
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .wrapping(text::Wrapping::Word)
                .into(),
        );
    }
    for check in f.checks.iter().take(8) {
        let name = check.path.file_name().map_or_else(
            || check.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        body.push(
            row![
                icon(
                    if check.problem.is_none() {
                        Icon::Check
                    } else {
                        Icon::Reject
                    },
                    12.0,
                    if check.problem.is_none() {
                        palette.safe
                    } else {
                        palette.danger
                    },
                ),
                text(match check.problem {
                    None => name,
                    Some(problem) => format!("{name}: {problem}"),
                })
                .size(TEXT_SMALL)
                .color(palette.text)
                .wrapping(text::Wrapping::None)
                .width(Length::Fill),
            ]
            .spacing(SP_SM)
            .align_y(Alignment::Center)
            .into(),
        );
    }
    let ready = f.answer().is_some();
    modal_layer(page_card(
        palette,
        if f.multiple {
            "Choose files"
        } else {
            "Choose a file"
        },
        host,
        column(body).spacing(SP_SM).into(),
        buttons(Some("Cancel"), "Open", ready),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(index: usize, label: &str) -> SelectOptionView {
        SelectOptionView {
            index,
            label: label.to_string(),
            disabled: false,
            selected: false,
            group: None,
        }
    }

    fn disabled(index: usize, label: &str) -> SelectOptionView {
        SelectOptionView {
            disabled: true,
            ..option(index, label)
        }
    }

    fn selected(index: usize, label: &str) -> SelectOptionView {
        SelectOptionView {
            selected: true,
            ..option(index, label)
        }
    }

    fn select_control(options: Vec<SelectOptionView>, multiple: bool) -> PendingControl {
        PendingControl::new(
            PageControl::Select {
                options,
                multiple,
                anchor: DeviceRect::default(),
            },
            Size::new(800.0, 600.0),
        )
    }

    fn highlighted(p: &PendingControl) -> Option<usize> {
        match &p.ui {
            ControlUi::Select(s) => s.highlighted,
            _ => panic!("not a select"),
        }
    }

    // ── select keyboard ──

    #[test]
    fn a_select_opens_on_its_chosen_option() {
        let p = select_control(
            vec![option(0, "a"), selected(1, "b"), option(2, "c")],
            false,
        );
        assert_eq!(highlighted(&p), Some(1));
        let p = select_control(vec![disabled(0, "a"), option(1, "b")], false);
        assert_eq!(
            highlighted(&p),
            Some(1),
            "the first enabled one when nothing is chosen"
        );
    }

    #[test]
    fn arrows_skip_disabled_options_and_stop_at_the_ends() {
        let mut p = select_control(
            vec![
                option(0, "a"),
                disabled(1, "b"),
                option(2, "c"),
                option(3, "d"),
            ],
            false,
        );
        assert_eq!(highlighted(&p), Some(0));
        p.handle(Msg::Key(ControlKey::Down));
        assert_eq!(highlighted(&p), Some(2), "skips the disabled one");
        p.handle(Msg::Key(ControlKey::Down));
        p.handle(Msg::Key(ControlKey::Down));
        assert_eq!(highlighted(&p), Some(3), "stops at the end");
        p.handle(Msg::Key(ControlKey::Up));
        p.handle(Msg::Key(ControlKey::Up));
        p.handle(Msg::Key(ControlKey::Up));
        assert_eq!(highlighted(&p), Some(0), "stops at the start");
        p.handle(Msg::Key(ControlKey::End));
        assert_eq!(highlighted(&p), Some(3));
        p.handle(Msg::Key(ControlKey::Home));
        assert_eq!(highlighted(&p), Some(0));
    }

    #[test]
    fn stepping_works_with_nothing_highlighted_and_with_nothing_enabled() {
        let opts = vec![option(0, "a"), option(1, "b")];
        assert_eq!(SelectState::step(&opts, None, 1), Some(0));
        assert_eq!(SelectState::step(&opts, None, -1), Some(1));
        assert_eq!(SelectState::step(&[disabled(0, "x")], None, 1), None);
        assert_eq!(SelectState::step(&[], Some(0), 1), None);
    }

    #[test]
    fn enter_picks_the_highlighted_option_of_a_single_select() {
        let mut p = select_control(vec![option(4, "a"), option(7, "b")], false);
        p.handle(Msg::Key(ControlKey::Down));
        assert_eq!(
            p.handle(Msg::Key(ControlKey::Enter)),
            Outcome::Answer(ControlAnswer::Select(vec![7])),
            "the answer is the option's own index, not its row"
        );
    }

    #[test]
    fn clicking_a_disabled_option_does_nothing_and_an_enabled_one_answers() {
        let mut p = select_control(vec![disabled(0, "a"), option(1, "b")], false);
        assert_eq!(p.handle(Msg::SelectChoose(0)), Outcome::Nothing);
        assert_eq!(
            p.handle(Msg::SelectChoose(1)),
            Outcome::Answer(ControlAnswer::Select(vec![1]))
        );
        // Hovering a disabled row does not move the highlight onto it.
        p.handle(Msg::SelectHover(0));
        assert_eq!(highlighted(&p), Some(1));
    }

    #[test]
    fn a_multi_select_ticks_with_space_or_click_and_applies_what_is_ticked() {
        let mut p = select_control(vec![selected(0, "a"), option(1, "b"), option(2, "c")], true);
        assert_eq!(
            p.handle(Msg::SelectChoose(2)),
            Outcome::Nothing,
            "a click only ticks"
        );
        p.handle(Msg::Key(ControlKey::Up));
        assert_eq!(highlighted(&p), Some(1));
        p.handle(Msg::Key(ControlKey::Space));
        p.handle(Msg::SelectChoose(0)); // untick the one that was chosen
        assert_eq!(
            p.handle(Msg::Accept),
            Outcome::Answer(ControlAnswer::Select(vec![1, 2]))
        );
        // Enter applies too; applying nothing is a valid answer.
        let mut p = select_control(vec![option(0, "a")], true);
        assert_eq!(
            p.handle(Msg::Key(ControlKey::Enter)),
            Outcome::Answer(ControlAnswer::Select(vec![]))
        );
    }

    #[test]
    fn escape_and_click_away_dismiss_every_kind_of_control() {
        let controls = vec![
            select_control(vec![option(0, "a")], false),
            PendingControl::new(
                PageControl::Dialog {
                    kind: DialogKind::Confirm,
                    message: "m".into(),
                    default: String::new(),
                },
                Size::new(10.0, 10.0),
            ),
            PendingControl::new(
                PageControl::File {
                    multiple: false,
                    accept: vec![],
                },
                Size::new(10.0, 10.0),
            ),
            PendingControl::new(
                PageControl::Color {
                    current: "#112233".into(),
                    anchor: DeviceRect::default(),
                },
                Size::new(10.0, 10.0),
            ),
            PendingControl::new(
                PageControl::Menu {
                    items: vec![],
                    anchor: DeviceRect::default(),
                },
                Size::new(10.0, 10.0),
            ),
        ];
        for mut c in controls {
            assert_eq!(
                c.handle(Msg::Dismiss),
                Outcome::Answer(ControlAnswer::Dismiss)
            );
        }
    }

    #[test]
    fn the_keyboard_moves_the_list_so_the_highlight_stays_in_view() {
        let options: Vec<_> = (0..50).map(|i| option(i, &format!("o{i}"))).collect();
        let mut p = select_control(options, false);
        let ControlUi::Select(s) = &p.ui else {
            panic!()
        };
        let view = s.viewport_h;
        assert_eq!(view, MAX_VISIBLE_ROWS as f32 * ROW_H);
        let mut last_scroll = None;
        for _ in 0..12 {
            if let Outcome::ScrollTo(y) = p.handle(Msg::Key(ControlKey::Down)) {
                last_scroll = Some(y);
            }
        }
        // The 13th option (row 12) is now highlighted; the list scrolled just enough.
        assert_eq!(highlighted(&p), Some(12));
        assert_eq!(last_scroll, Some(13.0 * ROW_H - view));
        let ControlUi::Select(s) = &p.ui else {
            panic!()
        };
        assert_eq!(s.scroll_y, 13.0 * ROW_H - view);
    }

    #[test]
    fn scroll_to_reveal_only_moves_when_the_row_is_out_of_view() {
        let view = 280.0;
        assert_eq!(scroll_to_reveal(0.0, 0.0, view), None);
        assert_eq!(scroll_to_reveal(252.0, 0.0, view), None, "fits exactly");
        assert_eq!(scroll_to_reveal(280.0, 0.0, view), Some(28.0));
        assert_eq!(
            scroll_to_reveal(28.0, 112.0, view),
            Some(28.0),
            "above the view"
        );
        assert_eq!(scroll_to_reveal(-5.0, 10.0, view), Some(0.0));
    }

    #[test]
    fn only_the_rows_near_the_viewport_are_built() {
        assert_eq!(window_rows(0.0, 280.0, 600), (0, 17));
        let (start, end) = window_rows(28.0 * 100.0, 280.0, 600);
        assert_eq!(start, 100 - OVERSCAN);
        assert_eq!(end, 100 + 11 + OVERSCAN);
        assert_eq!(
            window_rows(1.0e9, 280.0, 600),
            (600, 600),
            "past the end is empty, not a panic"
        );
        assert_eq!(window_rows(0.0, 280.0, 3), (0, 3));
    }

    #[test]
    fn groups_get_a_header_row_and_options_keep_their_indices() {
        let mut a = option(10, "x");
        a.group = Some("Fruit".into());
        let mut b = option(11, "y");
        b.group = Some("Fruit".into());
        let mut c = option(12, "z");
        c.group = Some("Veg".into());
        let s = SelectState::new(vec![option(9, "none"), a, b, c], false, 600.0);
        assert_eq!(
            s.rows,
            vec![
                SelectRow::Option(0),
                SelectRow::Group("Fruit".into()),
                SelectRow::Option(1),
                SelectRow::Option(2),
                SelectRow::Group("Veg".into()),
                SelectRow::Option(3),
            ]
        );
        assert_eq!(s.row_of, vec![0, 2, 3, 5]);
    }

    #[test]
    fn a_huge_list_is_capped_and_says_how_much_was_left_out() {
        let options: Vec<_> = (0..MAX_OPTIONS + 40).map(|i| option(i, "o")).collect();
        let s = SelectState::new(options, false, 600.0);
        assert_eq!(s.options.len(), MAX_OPTIONS);
        assert_eq!(s.omitted, 40);
    }

    #[test]
    fn a_short_page_area_shrinks_the_list_to_fit() {
        assert_eq!(list_height(50, 800.0, 0.0), MAX_VISIBLE_ROWS as f32 * ROW_H);
        assert_eq!(list_height(3, 800.0, 0.0), 3.0 * ROW_H);
        let tight = list_height(50, 200.0, FOOTER_H);
        assert!((ROW_H..200.0).contains(&tight));
        assert_eq!(list_height(0, 800.0, 0.0), ROW_H);
    }

    // ── dialogs ──

    #[test]
    fn dialog_answers_map_to_what_the_engine_expects() {
        use DialogKind::*;
        assert_eq!(dialog_answer(Alert, true, ""), ControlAnswer::Accept(None));
        assert_eq!(
            dialog_answer(Alert, false, ""),
            ControlAnswer::Accept(None),
            "an alert cannot be cancelled"
        );
        assert_eq!(
            dialog_answer(Confirm, true, ""),
            ControlAnswer::Accept(None)
        );
        assert_eq!(dialog_answer(Confirm, false, ""), ControlAnswer::Dismiss);
        assert_eq!(
            dialog_answer(Prompt, true, "hi"),
            ControlAnswer::Accept(Some("hi".into()))
        );
        assert_eq!(dialog_answer(Prompt, false, "hi"), ControlAnswer::Dismiss);
    }

    #[test]
    fn enter_is_ok_in_a_dialog_and_a_prompt_starts_with_its_default() {
        let mut p = PendingControl::new(
            PageControl::Dialog {
                kind: DialogKind::Prompt,
                message: "Name?".into(),
                default: "Ann".into(),
            },
            Size::new(10.0, 10.0),
        );
        p.handle(Msg::ReplyChanged("Bo".into()));
        assert_eq!(
            p.handle(Msg::Key(ControlKey::Enter)),
            Outcome::Answer(ControlAnswer::Accept(Some("Bo".into())))
        );
        let p = PendingControl::new(
            PageControl::Dialog {
                kind: DialogKind::Prompt,
                message: String::new(),
                default: "Ann".into(),
            },
            Size::new(10.0, 10.0),
        );
        assert!(matches!(&p.ui, ControlUi::Dialog { reply } if reply == "Ann"));
        let mut confirm = PendingControl::new(
            PageControl::Dialog {
                kind: DialogKind::Confirm,
                message: "Sure?".into(),
                default: String::new(),
            },
            Size::new(10.0, 10.0),
        );
        assert_eq!(
            confirm.handle(Msg::Accept),
            Outcome::Answer(ControlAnswer::Accept(None))
        );
    }

    #[test]
    fn a_pages_text_is_cleaned_before_it_is_shown() {
        assert_eq!(sanitize_untrusted("a\r\nb\rc", 50), "a\nb\nc");
        assert_eq!(sanitize_untrusted("tab\there", 50), "tab here");
        assert_eq!(
            sanitize_untrusted("ok\u{0007}\u{001b}[31mred", 50),
            "ok[31mred"
        );
        // Bidi overrides and zero-width characters can make text read as something else.
        assert_eq!(sanitize_untrusted("pay\u{202E}fdp.exe", 50), "payfdp.exe");
        assert_eq!(sanitize_untrusted("a\u{200B}b\u{FEFF}c", 50), "abc");
        let long = "x".repeat(5_000);
        let cut = sanitize_untrusted(&long, MESSAGE_CHARS);
        assert_eq!(cut.chars().count(), MESSAGE_CHARS + 1);
        assert!(cut.ends_with('\u{2026}'));
        assert_eq!(sanitize_untrusted("exact", 5), "exact");
    }

    #[test]
    fn dialogs_name_the_page_they_came_from() {
        assert_eq!(
            host_of("https://shop.example.com/cart?x=1"),
            "shop.example.com"
        );
        assert_eq!(host_of("about:blank"), "this page");
        assert_eq!(host_of(""), "this page");
        assert_eq!(host_of("file:///tmp/a.html"), "this page");
    }

    // ── geometry ──

    #[test]
    fn an_anchor_in_device_pixels_becomes_logical_points() {
        let a = DeviceRect {
            x: 200.0,
            y: 100.0,
            width: 300.0,
            height: 40.0,
        };
        assert_eq!(
            anchor_to_logical(a, 2.0),
            Rectangle {
                x: 100.0,
                y: 50.0,
                width: 150.0,
                height: 20.0
            }
        );
        assert_eq!(anchor_to_logical(a, 1.0).x, 200.0);
        // A bad scale falls back to 1 rather than dividing by zero.
        assert_eq!(anchor_to_logical(a, 0.0).x, 200.0);
        assert_eq!(anchor_to_logical(a, f32::NAN).y, 100.0);
    }

    #[test]
    fn a_popup_opens_below_its_anchor_unless_there_is_no_room() {
        let area = Size::new(800.0, 600.0);
        let popup = Size::new(200.0, 280.0);
        let anchor = Rectangle {
            x: 100.0,
            y: 50.0,
            width: 150.0,
            height: 20.0,
        };
        assert_eq!(place_popup(anchor, popup, area), Point::new(100.0, 72.0));
        // Near the bottom it flips above.
        let low = Rectangle { y: 500.0, ..anchor };
        assert_eq!(
            place_popup(low, popup, area),
            Point::new(100.0, 500.0 - GAP - 280.0)
        );
        // Too tall for either side: pinned inside the area.
        let tall = Size::new(200.0, 590.0);
        let p = place_popup(anchor, tall, area);
        assert_eq!(p.y, 600.0 - 590.0 - EDGE);
        // Near the right edge it is pulled left; never past the left edge.
        let right = Rectangle { x: 790.0, ..anchor };
        assert_eq!(place_popup(right, popup, area).x, 800.0 - 200.0 - EDGE);
        let left = Rectangle { x: -30.0, ..anchor };
        assert_eq!(place_popup(left, popup, area).x, EDGE);
    }

    // ── colour ──

    #[test]
    fn typed_colours_are_normalised_or_refused() {
        assert_eq!(normalize_hex("#FF8000").as_deref(), Some("#ff8000"));
        assert_eq!(normalize_hex("ff8000").as_deref(), Some("#ff8000"));
        assert_eq!(normalize_hex(" #f80 ").as_deref(), Some("#ff8800"));
        assert_eq!(normalize_hex("#ff80"), None);
        assert_eq!(normalize_hex("red"), None);
        assert_eq!(normalize_hex(""), None);
        assert!(SWATCHES
            .iter()
            .all(|s| normalize_hex(s).as_deref() == Some(*s)));
    }

    #[test]
    fn the_colour_picker_answers_with_a_valid_colour_only() {
        let mut p = PendingControl::new(
            PageControl::Color {
                current: "#112233".into(),
                anchor: DeviceRect::default(),
            },
            Size::new(10.0, 10.0),
        );
        assert_eq!(
            p.handle(Msg::Accept),
            Outcome::Answer(ControlAnswer::Color("#112233".into()))
        );
        p.handle(Msg::ColorChanged("#12".into()));
        assert_eq!(
            p.handle(Msg::Accept),
            Outcome::Nothing,
            "an invalid colour is not an answer"
        );
        p.handle(Msg::ColorChanged("ABCDEF".into()));
        assert_eq!(
            p.handle(Msg::Key(ControlKey::Enter)),
            Outcome::Answer(ControlAnswer::Color("#abcdef".into()))
        );
    }

    // ── files ──

    #[test]
    fn paths_split_on_lines_and_on_commas_and_lose_their_quotes() {
        let all = |_: &std::path::Path| false;
        let home = Some(std::path::Path::new("/home/ann"));
        assert_eq!(
            parse_paths("/a/one.png\n  /a/two.png  \n\n", all, home),
            [PathBuf::from("/a/one.png"), PathBuf::from("/a/two.png")]
        );
        assert_eq!(
            parse_paths("/a/one.png, /a/two.png,/a/three.png", all, home),
            [
                PathBuf::from("/a/one.png"),
                PathBuf::from("/a/two.png"),
                PathBuf::from("/a/three.png")
            ]
        );
        assert_eq!(
            parse_paths("\"/a/my file.png\"\n'/b/x.png'", all, home),
            [PathBuf::from("/a/my file.png"), PathBuf::from("/b/x.png")]
        );
        assert_eq!(
            parse_paths("~/pics/a.png", all, home),
            [PathBuf::from("/home/ann/pics/a.png")]
        );
        // A real file with a comma in its name stays whole.
        let exists = |p: &std::path::Path| p == std::path::Path::new("/a/1,2.png");
        assert_eq!(
            parse_paths("/a/1,2.png", exists, home),
            [PathBuf::from("/a/1,2.png")]
        );
        assert!(parse_paths("   \n\n", all, home).is_empty());
    }

    #[test]
    fn a_single_file_input_takes_one_path_and_every_path_must_exist() {
        let exists = |p: &std::path::Path| p.starts_with("/ok");
        let checks = check_paths(
            vec![
                PathBuf::from("/ok/a"),
                PathBuf::from("/ok/b"),
                PathBuf::from("/missing"),
            ],
            false,
            exists,
        );
        assert_eq!(checks[0].problem, None);
        assert_eq!(checks[1].problem, Some("only one file can be chosen here"));
        assert_eq!(checks[2].problem, Some("only one file can be chosen here"));
        let checks = check_paths(
            vec![PathBuf::from("/ok/a"), PathBuf::from("/missing")],
            true,
            exists,
        );
        assert_eq!(checks[1].problem, Some("no such file"));
    }

    #[test]
    fn the_file_picker_answers_with_real_files_only() {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-controls-test-{}-{}",
            std::process::id(),
            ferrite_servo::diag::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let one = dir.join("one.txt");
        let two = dir.join("two.txt");
        std::fs::write(&one, "1").unwrap();
        std::fs::write(&two, "2").unwrap();

        let mut p = PendingControl::new(
            PageControl::File {
                multiple: true,
                accept: vec![".txt".into()],
            },
            Size::new(10.0, 10.0),
        );
        assert_eq!(p.handle(Msg::Accept), Outcome::Nothing, "nothing typed yet");
        let ControlUi::File(f) = &mut p.ui else {
            panic!()
        };
        f.content =
            text_editor::Content::with_text(&format!("{}\n{}", one.display(), two.display()));
        assert_eq!(
            p.handle(Msg::Accept),
            Outcome::Answer(ControlAnswer::Files(vec![one.clone(), two.clone()]))
        );

        let mut single = PendingControl::new(
            PageControl::File {
                multiple: false,
                accept: vec![],
            },
            Size::new(10.0, 10.0),
        );
        single.handle(Msg::FileTextChanged(format!(
            "{}, {}",
            one.display(),
            two.display()
        )));
        assert_eq!(
            single.handle(Msg::Accept),
            Outcome::Nothing,
            "two files into a single input"
        );
        single.handle(Msg::FileTextChanged(one.display().to_string()));
        assert_eq!(
            single.handle(Msg::Accept),
            Outcome::Answer(ControlAnswer::Files(vec![one]))
        );
        single.handle(Msg::FileTextChanged(
            dir.join("nope.txt").display().to_string(),
        ));
        assert_eq!(single.handle(Msg::Accept), Outcome::Nothing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── menu ──

    fn menu_item(index: Option<usize>, label: &str, enabled: bool) -> MenuItemView {
        MenuItemView {
            index,
            label: label.to_string(),
            enabled,
        }
    }

    #[test]
    fn a_context_menu_skips_separators_and_disabled_rows_and_answers_by_index() {
        let items = vec![
            menu_item(Some(0), "Back", false),
            menu_item(Some(1), "Reload", true),
            menu_item(None, "", false),
            menu_item(Some(2), "Inspect", true),
        ];
        let mut p = PendingControl::new(
            PageControl::Menu {
                items: items.clone(),
                anchor: DeviceRect::default(),
            },
            Size::new(10.0, 10.0),
        );
        let ControlUi::Menu { highlighted } = &p.ui else {
            panic!()
        };
        assert_eq!(*highlighted, Some(1), "starts on the first pickable row");
        p.handle(Msg::Key(ControlKey::Down));
        assert_eq!(
            p.handle(Msg::Key(ControlKey::Enter)),
            Outcome::Answer(ControlAnswer::Menu(2))
        );
        assert_eq!(p.handle(Msg::MenuChoose(0)), Outcome::Nothing, "disabled");
        assert_eq!(
            p.handle(Msg::MenuChoose(2)),
            Outcome::Nothing,
            "a separator"
        );
        assert_eq!(
            p.handle(Msg::MenuChoose(1)),
            Outcome::Answer(ControlAnswer::Menu(1))
        );
        assert_eq!(menu_step(&items, Some(3), 1), Some(3), "stops at the end");
        assert_eq!(menu_step(&items, None, -1), Some(3));
        assert_eq!(menu_step(&[], None, 1), None);
    }

    // ── keys ──

    fn key(down: bool, key: PageKey) -> PageKeyEvent {
        PageKeyEvent {
            down,
            key,
            shift: false,
            ctrl: false,
            alt: false,
            meta: false,
        }
    }

    #[test]
    fn only_presses_of_the_listed_keys_reach_a_control() {
        assert_eq!(
            control_key(&key(true, PageKey::Named(PageNamedKey::ArrowDown))),
            Some(ControlKey::Down)
        );
        assert_eq!(
            control_key(&key(true, PageKey::Named(PageNamedKey::Enter))),
            Some(ControlKey::Enter)
        );
        assert_eq!(
            control_key(&key(true, PageKey::Character(" ".into()))),
            Some(ControlKey::Space)
        );
        assert_eq!(
            control_key(&key(false, PageKey::Named(PageNamedKey::Enter))),
            None
        );
        assert_eq!(
            control_key(&key(true, PageKey::Character("a".into()))),
            None
        );
        assert_eq!(
            control_key(&key(true, PageKey::Named(PageNamedKey::Tab))),
            None
        );
    }

    // ── app integration ──

    fn state_with(control: PageControl) -> FerriteBrowser {
        let mut state = FerriteBrowser::default();
        state.tab_diag[0].control = Some(PendingControl::new(control, Size::new(800.0, 600.0)));
        state
    }

    fn a_select() -> PageControl {
        PageControl::Select {
            options: vec![option(0, "a"), option(1, "b")],
            multiple: false,
            anchor: DeviceRect::default(),
        }
    }

    #[test]
    fn answering_clears_the_control_so_it_cannot_be_answered_twice() {
        let mut state = state_with(a_select());
        assert!(tab_has_control(&state, 0));
        let _ = update(&mut state, Msg::SelectChoose(1));
        assert!(!tab_has_control(&state, 0), "answered once, gone");
        // A late second answer finds nothing.
        let _ = update(&mut state, Msg::SelectChoose(0));
        assert!(!tab_has_control(&state, 0));
    }

    #[test]
    fn dismissing_a_tab_that_has_no_control_is_a_no_op() {
        let mut state = FerriteBrowser::default();
        dismiss_tab(&mut state, 0);
        dismiss_tab(&mut state, 99);
        let mut state = state_with(a_select());
        dismiss_tab(&mut state, 0);
        assert!(!tab_has_control(&state, 0));
    }

    #[test]
    fn a_control_on_another_tab_is_not_answered_by_the_active_tabs_keys() {
        let mut state = FerriteBrowser::default();
        crate::push_tab_state(&mut state);
        state.active_tab = 0;
        state.tab_diag[1].control = Some(PendingControl::new(a_select(), Size::new(800.0, 600.0)));
        let _ = update(&mut state, Msg::Key(ControlKey::Enter));
        assert!(
            tab_has_control(&state, 1),
            "tab 1 is not the one being looked at"
        );
        assert!(active_control(&state).is_none());
    }
}
