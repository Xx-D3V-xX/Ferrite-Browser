//! The agent sidebar as a chat: header (title, step counter, History and New
//! chat), a message thread (user bubbles right, agent activity and answers
//! left), a composer with a context chip, an empty state with suggestion
//! chips, and the chat-history list. The consent panel (the security surface)
//! also lives here, unchanged in content, now inside the same panel frame.
//!
//! # Conventions kept from the rest of `ferrite-ui`
//!
//! * Every colour is `state.palette()` / `palette_for_theme(theme)`; nothing
//!   here hardcodes a colour except `Color::WHITE` on accent/danger fills, as
//!   the existing buttons do.
//! * Icons go through `icons::icon` at `ICON_SIZE`/`ICON_SIZE_SM`, in fixed
//!   square boxes so nothing shifts when a glyph or hover state changes.
//! * Every new control has hover and pressed styles (`button::Status`), and a
//!   disabled look where it can be disabled.
//! * Animation is tick-driven (`ThreadAnimTick`, gated in `subscription()` so
//!   it only runs while something is animating, exactly like the consent
//!   panel's `consent_anim_tick`) and eased with `ease_out_cubic`; the view is
//!   a pure function of state.
//!
//! # Things that are pure and tested here
//!
//! [`relative_time`], [`suggestions`], [`steps_summary`],
//! [`is_pinned_to_bottom`], [`page_note_text`], [`composer_placeholder`],
//! [`expand_key`] and the entrance-animation helpers. The widget-building code
//! is straight-line composition of those; it cannot be rendered in the build
//! sandbox (no display), so it is kept simple and reviewed by reading.

use chrono::{DateTime, TimeZone};
use ferrite_agent::chat::{PageContextNote, StepRecord, Turn};
use ferrite_agent::context::{decide_page_use, describe_context};
use iced::widget::{column, horizontal_space, row};

use super::*;
use crate::tokens::{
    danger_btn_style, field_style, on_fill, outline_btn_style, raised_bar_style, rule_card,
    safe_btn_style, tint, tip, toolbar_btn_style, RADIUS_SM, SP_LG, SP_MD, SP_SM, SP_XS, TEXT_BODY,
    TEXT_CAPTION, TEXT_SMALL, TEXT_TITLE,
};

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

/// Widest a user bubble grows (the sidebar is `SIDE_PANEL_WIDTH`, the same
/// width as every other right-hand drawer, so switching drawers never moves
/// the page edge).
const BUBBLE_MAX_WIDTH: f32 = 310.0;
/// Fixed side of the square icon buttons in the header and rows.
const ICON_BTN: f32 = 28.0;
/// Fixed side of the composer's send/stop button.
const SEND_BTN: f32 = 34.0;
/// Vertical gap after every thread item; also the total room an item's
/// entrance slide moves within, so the item's height never changes mid-slide.
const ITEM_GAP: f32 = 10.0;
/// How far (px) an item slides while it fades in. Must be `<= ITEM_GAP`.
const ENTRANCE_SLIDE: f32 = 6.0;
/// Characters of a user message shown before "Show more".
const USER_TEXT_LIMIT: usize = 600;
/// Characters of an answer shown before "Show more".
/// Characters of a step's result shown before "Show more".
/// How much of a page's text shows before "Show more".
const PAGE_PREVIEW_CHARS: usize = 140;
const RESULT_TEXT_LIMIT: usize = 140;
/// The most characters ever laid out for one expanded text (Copy still copies
/// all of it); keeps a 20,000-character answer from making layout crawl.
const EXPANDED_MAX_CHARS: usize = 6_000;
/// A chat title is cut to this many characters in the header.
const HEADER_TITLE_CHARS: usize = 28;

/// `scrollable::Id` of the message thread, for `snap_to`.
pub(crate) fn thread_scroll_id() -> scrollable::Id {
    scrollable::Id::new("ferrite_agent_thread")
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested in `mod tests` below)
// ---------------------------------------------------------------------------

/// "just now", "12m ago", "3h ago", "yesterday", "4d ago", then a date
/// ("Sep 3", or "Sep 3, 2025" from another year). `now` and `then` are read in
/// the same time zone (the view passes local times), so "yesterday" means the
/// previous *calendar* day for the person reading it. A `then` in the future
/// (clock skew) reads as "just now".
pub(crate) fn relative_time<Tz: TimeZone>(now: DateTime<Tz>, then: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let secs = (now.clone() - then.clone()).num_seconds();
    if secs < 45 {
        return "just now".to_string();
    }
    if secs < 60 * 60 {
        return format!("{}m ago", (secs / 60).max(1));
    }
    let days = (now.date_naive() - then.date_naive()).num_days();
    if days <= 0 {
        return format!("{}h ago", secs / 3600);
    }
    if days == 1 {
        return "yesterday".to_string();
    }
    if days < 7 {
        return format!("{days}d ago");
    }
    if now.date_naive().format("%Y").to_string() == then.date_naive().format("%Y").to_string() {
        then.format("%b %-d").to_string()
    } else {
        then.format("%b %-d, %Y").to_string()
    }
}

/// One empty-state suggestion: what the chip says and what it puts in the
/// composer when clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Suggestion {
    pub label: &'static str,
    pub fill: &'static str,
}

/// Three suggestion chips that fit what is on screen: page-oriented ones when
/// a real web page is open, otherwise ones that start from nothing. A chip
/// ending in an ellipsis leaves the composer waiting for the rest.
pub(crate) fn suggestions(has_page: bool) -> [Suggestion; 3] {
    if has_page {
        [
            Suggestion {
                label: "Summarize this page",
                fill: "Summarize this page",
            },
            Suggestion {
                label: "Find the important links",
                fill: "Find the important links on this page",
            },
            Suggestion {
                label: "Fill in this form",
                fill: "Fill in this form",
            },
        ]
    } else {
        [
            Suggestion {
                label: "Search the web for\u{2026}",
                fill: "Search the web for ",
            },
            Suggestion {
                label: "Open a site\u{2026}",
                fill: "Open ",
            },
            Suggestion {
                label: "Compare a few options\u{2026}",
                fill: "Compare ",
            },
        ]
    }
}

/// Whether `url` is a real web page (what makes page-oriented suggestions and
/// "use the page" meaningful).
pub(crate) fn is_real_page(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// The collapsed activity header's text: `"1 step"`, `"3 steps"`, and
/// `"3 steps \u{b7} 1 blocked"` when the defense or consent blocked some.
pub(crate) fn steps_summary(total: usize, blocked: usize) -> String {
    let base = format!("{total} step{}", if total == 1 { "" } else { "s" });
    if blocked > 0 {
        format!("{base} \u{b7} {blocked} blocked")
    } else {
        base
    }
}

/// Whether a scrollable showing `viewport_height` of `content_height`, scrolled
/// `offset_y` down, is at (or within a few pixels of) the bottom — or has
/// nothing to scroll at all. `on_scroll` feeds this; new content only pulls the
/// thread to the bottom while it holds, so a user who scrolled up to read is
/// never yanked back down.
pub(crate) fn is_pinned_to_bottom(
    offset_y: f32,
    viewport_height: f32,
    content_height: f32,
) -> bool {
    const SLACK: f32 = 24.0;
    if !(offset_y.is_finite() && viewport_height.is_finite() && content_height.is_finite()) {
        return true;
    }
    content_height <= viewport_height + 1.0 || offset_y + viewport_height >= content_height - SLACK
}

/// The small line under a user bubble saying what context the agent got.
pub(crate) fn page_note_text(note: &PageContextNote) -> String {
    let title = if note.title.trim().is_empty() {
        note.url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
    } else {
        note.title.as_str()
    };
    if note.used_full_page {
        format!("Used current page \u{b7} {}", truncate(title, 36))
    } else if note.reason.trim().is_empty() {
        "Page not used".to_string()
    } else {
        format!("Page not used \u{b7} {}", truncate(note.reason.trim(), 44))
    }
}

/// The composer's placeholder: "Reply to the agent\u{2026}" when the last turn
/// ended with a question for the user, otherwise a plain prompt.
pub(crate) fn composer_placeholder(
    last_outcome: Option<&Outcome>,
    running: bool,
    reviewing: bool,
) -> &'static str {
    if reviewing {
        "Review the request above\u{2026}"
    } else if running {
        "Working\u{2026} you can type ahead"
    } else if matches!(last_outcome, Some(Outcome::AskedUser(_))) {
        "Reply to the agent\u{2026}"
    } else {
        "Message the agent\u{2026}"
    }
}

/// Which expandable part of the thread a key names.
pub(crate) enum ExpandPart<'a> {
    /// A finished turn's step list.
    Steps,
    /// A long user message.
    User,
    /// One step's long result (by step index).
    Result(&'a usize),
}

/// The `FerriteBrowser::expanded` key for `part` of the turn `turn_id`.
pub(crate) fn expand_key(turn_id: &str, part: ExpandPart<'_>) -> String {
    match part {
        ExpandPart::Steps => format!("steps:{turn_id}"),
        ExpandPart::User => format!("user:{turn_id}"),
        ExpandPart::Result(i) => format!("result:{turn_id}:{i}"),
    }
}

/// Entrance progress (`0.0..=1.0`) of thread item `key`: `1.0` (fully in) for
/// anything not currently animating.
pub(crate) fn item_progress(anims: &[(ItemKey, f32)], key: ItemKey) -> f32 {
    anims
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(1.0, |(_, t)| t.clamp(0.0, 1.0))
}

/// Starts (or restarts) `key`'s entrance animation.
pub(crate) fn start_item_anim(anims: &mut Vec<(ItemKey, f32)>, key: ItemKey) {
    if let Some(entry) = anims.iter_mut().find(|(k, _)| *k == key) {
        entry.1 = 0.0;
    } else {
        anims.push((key, 0.0));
    }
}

/// Advances every running entrance by `step` and drops the finished ones.
pub(crate) fn advance_anims(anims: &mut Vec<(ItemKey, f32)>, step: f32) {
    for (_, t) in anims.iter_mut() {
        *t = (*t + step).min(1.0);
    }
    anims.retain(|(_, t)| *t < 1.0);
}

/// Whether the thread's entrance-animation tick should be subscribed: only
/// while the panel is showing and something is still animating.
pub(crate) fn thread_anim_active(state: &FerriteBrowser) -> bool {
    state.show_agent_sidebar && !state.thread_anims.is_empty()
}

/// `color` at `alpha` times its own alpha — the fade of an entrance.
fn fade(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha.clamp(0.0, 1.0),
        ..color
    }
}

/// The eased alpha and slide for an entrance progress `t`.
fn eased(t: f32) -> f32 {
    ease_out_cubic(t)
}

/// A step is an error when the observation says so (and was not blocked).
fn is_error_result(result: &str) -> bool {
    result
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("error")
}

// ---------------------------------------------------------------------------
// Small building blocks
// ---------------------------------------------------------------------------

fn sep<'a>() -> Element<'a, FerriteBrowserMessage> {
    container(text(""))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(separator_style)
        .into()
}

/// Wraps a thread item so it slides in `ENTRANCE_SLIDE` px while its own
/// alpha (applied by the builder) fades: top padding shrinks as bottom padding
/// grows, so the item's total height is constant and nothing below it jumps.
fn entrance<'a>(
    content: impl Into<Element<'a, FerriteBrowserMessage>>,
    t: f32,
) -> Element<'a, FerriteBrowserMessage> {
    let offset = (1.0 - eased(t)) * ENTRANCE_SLIDE;
    container(content)
        .width(Length::Fill)
        .padding(Padding {
            top: offset,
            right: 0.0,
            bottom: ITEM_GAP - offset,
            left: 0.0,
        })
        .into()
}

/// A fixed-size icon-only button (hover/pressed via `nav_btn_style`, or the
/// active look while its view is showing), with a tooltip.
fn icon_button<'a>(
    kind: Icon,
    color: Color,
    label: &'a str,
    on_press: Option<FerriteBrowserMessage>,
    active: bool,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let btn = button(container(icon(kind, ICON_SIZE, color)).center(Length::Fill))
        .width(Length::Fixed(ICON_BTN))
        .height(Length::Fixed(ICON_BTN))
        .padding(0)
        .style(if active {
            panel_btn_active
        } else {
            toolbar_btn_style
        })
        .on_press_maybe(on_press);
    tip(btn, label, palette)
}

/// A quiet text-style button ("Show more", "Copy"): transparent until hovered.
pub(crate) fn link_button_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => Color {
                a: 0.12,
                ..palette.accent
            },
            _ => Color::TRANSPARENT,
        })),
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => palette.accent_bright,
            _ => palette.accent,
        },
        border: Border {
            radius: iced::border::Radius::new(4.0),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Text that is bounded and, when longer than `limit`, offers "Show more" /
/// "Show less" (`key` in `FerriteBrowser::expanded`). Wraps at glyph level as a
/// fallback so an unbroken URL can never overflow the sidebar.
fn expandable_text<'a>(
    full: &str,
    limit: usize,
    expanded: bool,
    key: String,
    size: f32,
    color: Color,
) -> Element<'a, FerriteBrowserMessage> {
    let long = full.chars().count() > limit;
    let shown = if !long {
        full.to_string()
    } else if expanded {
        truncate(full, EXPANDED_MAX_CHARS)
    } else {
        truncate(full, limit)
    };
    // Shrink (not Fill) width: inside a shrink-wrapped bubble a Fill child
    // would stretch the bubble to its maximum even for "hi"; inside a full
    // width card the text still wraps at the card's edge.
    let body = text(shown)
        .size(size)
        .color(color)
        .wrapping(text::Wrapping::WordOrGlyph);
    if !long {
        return body.into();
    }
    column![
        body,
        button(text(if expanded { "Show less" } else { "Show more" }).size(11))
            .padding([2, 6])
            .style(link_button_style)
            .on_press(FerriteBrowserMessage::ToggleExpand(key)),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// The whole agent sidebar. Thin composition: header, optional notice, the
/// body (consent panel, thread or history), and the composer.
pub(crate) fn view_agent_sidebar(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let reviewing = state.pending_diff.is_some();

    let mut items: Vec<Element<FerriteBrowserMessage>> = vec![view_header(state), sep()];
    // With no model connected the agent cannot act; say so, with the button
    // that fixes it, before anything else in the panel.
    if let Some(banner) = super::settings_panel::connect_banner(state) {
        items.push(container(banner).padding(PANEL_PADDING).into());
        items.push(sep());
    }
    if let Some(notice) = &state.panel_notice {
        items.push(view_notice(notice, palette));
        items.push(sep());
    }
    if reviewing {
        items.push(consent_body(state));
    } else if state.sidebar_view == SidebarView::History {
        items.push(history_body(state));
    } else {
        items.push(thread_body(state));
    }
    // The agent is paused on a page that needs the person: say so, above the
    // composer where the eye already is, with the one button that resumes it.
    if let Some(wall) = &state.signin_handoff {
        items.push(sep());
        items.push(view_signin_card(wall, palette));
    }
    // The guard stopped an action outside what the request implied. The run
    // waits; nothing outside the prediction runs until the person answers.
    if let Some(pending) = &state.pending_runtime {
        items.push(sep());
        items.push(view_runtime_card(pending, palette));
    }
    if state.sidebar_view == SidebarView::Thread || reviewing {
        items.push(sep());
        items.push(view_composer(state));
    }

    container(column(items).width(Length::Fill).height(Length::Fill))
        .width(Length::Fixed(state.panels.side_width(state.window_size)))
        .height(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(palette.surface)),
            border: Border {
                color: palette.divider,
                width: 1.0,
                radius: iced::border::Radius::new(0.0),
            },
            ..container::Style::default()
        })
        .into()
}

fn view_header(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let history = state.sidebar_view == SidebarView::History && state.pending_diff.is_none();
    let blocked = chat_switch_blocked(state);

    let mut items: Vec<Element<FerriteBrowserMessage>> = Vec::new();
    if history {
        items.push(icon_button(
            Icon::Back,
            palette.text_dim,
            "Back to the chat",
            Some(FerriteBrowserMessage::SetSidebarView(SidebarView::Thread)),
            false,
            palette,
        ));
    } else {
        items.push(icon(Icon::Agent, ICON_SIZE, palette.accent));
    }
    let title = if history {
        "Chats".to_string()
    } else {
        truncate(&state.chat.title, HEADER_TITLE_CHARS)
    };
    items.push(
        text(title)
            .size(15)
            .font(font_weight(iced::font::Weight::Semibold))
            .color(palette.text)
            .width(Length::Fill)
            .into(),
    );
    if let Some(live) = &state.live_loop {
        items.push(
            text(format!(
                "Step {}/{}",
                live.actions_taken.len() + 1,
                live.budget.max_steps
            ))
            .size(11)
            .color(palette.text_dim)
            .into(),
        );
    }
    if !history {
        items.push(icon_button(
            Icon::History,
            palette.text_dim,
            "Chat history",
            Some(FerriteBrowserMessage::SetSidebarView(SidebarView::History)),
            false,
            palette,
        ));
    }
    // Still pressable while a run is active: the handler explains why it is
    // refused (a notice), which beats a dead button. The dimmed icon says so
    // before the click.
    items.push(icon_button(
        Icon::Add,
        if blocked {
            fade(palette.text_dim, 0.5)
        } else {
            palette.text_dim
        },
        if blocked {
            "Stop the current run to start a new chat"
        } else {
            new_chat_tip()
        },
        Some(FerriteBrowserMessage::NewChat),
        false,
        palette,
    ));

    container(
        row(items)
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .padding([8, 12]),
    )
    .width(Length::Fill)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(palette.raised)),
        ..container::Style::default()
    })
    .into()
}

/// "New chat (Ctrl+Shift+O)" with the platform's modifier name.
fn new_chat_tip() -> &'static str {
    if cfg!(target_os = "macos") {
        "New chat (Cmd+Shift+O)"
    } else {
        "New chat (Ctrl+Shift+O)"
    }
}

/// A button in a decision card's action row: label, optional key hint.
fn card_action<'a>(
    label: &'a str,
    hint: Option<&'a str>,
    style: fn(&Theme, button::Status) -> button::Style,
    portion: u16,
    on_press: FerriteBrowserMessage,
) -> Element<'a, FerriteBrowserMessage> {
    let mut cells: Vec<Element<FerriteBrowserMessage>> = vec![text(label).size(TEXT_SMALL).into()];
    if let Some(hint) = hint {
        // The key hint takes the button's own label colour, softened, so it
        // stays readable on whichever fill the button has in either theme.
        cells.push(
            text(hint)
                .size(10)
                .style(move |theme: &Theme| text::Style {
                    color: Some(Color {
                        a: 0.8,
                        ..style(theme, button::Status::Active).text_color
                    }),
                })
                .into(),
        );
    }
    button(
        container(
            row(cells)
                .spacing(SP_XS + 2.0)
                .align_y(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .center_x(Length::Fill),
    )
    .width(Length::FillPortion(portion))
    .padding([SP_SM - 1.0, SP_SM])
    .style(style)
    .on_press(on_press)
    .into()
}

/// The frame every decision the person has to make shares: a neutral raised
/// card inset from the panel's edges, a coloured rule down its left edge, an
/// icon and a title, the question, and a row of actions. `tone` is warn for
/// "the agent needs you" and danger for "something looks wrong"; it colours the
/// rule and the icon only, never the surface (`tokens::rule_card`).
fn decision_card<'a>(
    palette: &'static Palette,
    tone: Color,
    title: impl Into<String>,
    body: Element<'a, FerriteBrowserMessage>,
    actions: Element<'a, FerriteBrowserMessage>,
) -> Element<'a, FerriteBrowserMessage> {
    container(rule_card(
        palette,
        tone,
        column![
            row![
                icon(Icon::Warning, ICON_SIZE, tone),
                text(title.into())
                    .size(TEXT_BODY)
                    .font(font_weight(iced::font::Weight::Semibold))
                    .color(palette.text)
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(SP_SM)
            .align_y(iced::Alignment::Center),
            body,
            actions,
        ]
        .spacing(SP_SM + 2.0)
        .into(),
    ))
    .padding([SP_SM, SP_MD])
    .width(Length::Fill)
    .into()
}

/// The sign-in handoff card: what the page wants, why Ferrite is not doing it,
/// and *Continue* / *Stop task*.
fn view_signin_card<'a>(
    wall: &'a super::signin::SignInWall,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    decision_card(
        palette,
        palette.warn,
        wall.title(),
        text(wall.body())
            .size(TEXT_SMALL)
            .color(palette.text_dim)
            .wrapping(text::Wrapping::Word)
            .into(),
        row![
            card_action(
                "I\u{2019}ve done it \u{2014} continue",
                None,
                accent_btn_style,
                3,
                FerriteBrowserMessage::SigninContinue,
            ),
            card_action(
                "Stop task",
                None,
                outline_btn_style,
                2,
                FerriteBrowserMessage::StopAgent,
            ),
        ]
        .spacing(SP_SM)
        .into(),
    )
}

/// The runtime-consent card: what the agent wants to do that the request did
/// not imply, and *Don't allow* / *Allow once* / *Allow for this task*. Says
/// only the action and the site, never text a page wrote. The safe answer is
/// the prominent one and has the Esc key.
fn view_runtime_card<'a>(
    pending: &'a super::PendingRuntimeConsent,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    decision_card(
        palette,
        palette.warn,
        "The agent needs your approval",
        column![
            text(pending.summary())
                .size(TEXT_SMALL)
                .color(palette.text)
                .wrapping(text::Wrapping::WordOrGlyph),
            text(
                "This is outside what your request implied. The agent is paused until you decide."
            )
            .size(TEXT_CAPTION)
            .color(palette.text_dim)
            .wrapping(text::Wrapping::Word),
        ]
        .spacing(SP_XS)
        .into(),
        row![
            card_action(
                "Don\u{2019}t allow",
                Some("Esc"),
                danger_btn_style,
                4,
                FerriteBrowserMessage::RuntimeDeny,
            ),
            card_action(
                "Allow once",
                None,
                outline_btn_style,
                3,
                FerriteBrowserMessage::RuntimeAllowOnce,
            ),
            card_action(
                "Allow for task",
                None,
                outline_btn_style,
                4,
                FerriteBrowserMessage::RuntimeAllowTask,
            ),
        ]
        .spacing(SP_SM)
        .into(),
    )
}

fn view_notice<'a>(
    notice: &'a str,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    container(
        row![
            icon(Icon::Warning, ICON_SIZE_SM, palette.warn),
            text(notice)
                .size(TEXT_CAPTION)
                .color(palette.text)
                .width(Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
            button(icon(Icon::Close, ICON_SIZE_SM, palette.text_dim))
                .padding(3)
                .style(close_btn_style)
                .on_press(FerriteBrowserMessage::DismissNotice),
        ]
        .spacing(SP_SM)
        .align_y(iced::Alignment::Center),
    )
    .padding([SP_XS + 2.0, SP_MD])
    .width(Length::Fill)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(palette.raised)),
        ..container::Style::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------

fn thread_body(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    if state.chat.turns.is_empty() && !state.agent_is_running {
        return empty_state(state);
    }
    let palette = state.palette();
    let last = state.chat.turns.len().saturating_sub(1);
    let mut items: Vec<Element<FerriteBrowserMessage>> = Vec::new();
    for (ti, turn) in state.chat.turns.iter().enumerate() {
        let live =
            ti == last && matches!(turn.outcome, Outcome::InProgress) && state.agent_is_running;
        items.push(user_message(state, ti, turn));
        if let Some(section) = agent_section(state, ti, turn, live) {
            items.push(section);
        }
    }

    scrollable(column(items).width(Length::Fill).padding(Padding {
        top: 10.0,
        right: 12.0,
        bottom: 4.0,
        left: 12.0,
    }))
    .id(thread_scroll_id())
    .on_scroll(|viewport| FerriteBrowserMessage::ThreadScrolled {
        pinned: is_pinned_to_bottom(
            viewport.absolute_offset().y,
            viewport.bounds().height,
            viewport.content_bounds().height,
        ),
    })
    .height(Length::Fill)
    .style(move |theme: &Theme, status| {
        let mut style = scrollable::default(theme, status);
        style.container = container::Style {
            background: Some(Background::Color(palette.surface)),
            ..container::Style::default()
        };
        style
    })
    .into()
}

/// A user's message: a right-aligned accent bubble and, under it, a small note
/// on what page context the agent was given.
fn user_message<'a>(
    state: &'a FerriteBrowser,
    ti: usize,
    turn: &'a Turn,
) -> Element<'a, FerriteBrowserMessage> {
    let palette = state.palette();
    let t = item_progress(
        &state.thread_anims,
        ItemKey {
            turn: ti,
            slot: SLOT_USER,
        },
    );
    let a = eased(t);
    let turn_id = turn.id.to_string();
    let key = expand_key(&turn_id, ExpandPart::User);
    let expanded = state.expanded.contains(&key);

    let bubble = container(expandable_text(
        &turn.user,
        USER_TEXT_LIMIT,
        expanded,
        key,
        13.0,
        fade(Color::WHITE, a),
    ))
    .padding([8, 12])
    .max_width(BUBBLE_MAX_WIDTH)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(fade(palette.accent, a))),
        border: Border {
            radius: iced::border::Radius::new(14.0).bottom_right(4.0),
            ..Border::default()
        },
        ..container::Style::default()
    });

    let mut col: Vec<Element<FerriteBrowserMessage>> = vec![container(bubble)
        .width(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .into()];
    if let Some(note) = &turn.page_context {
        col.push(
            container(
                text(page_note_text(note))
                    .size(10)
                    .color(fade(palette.text_dim, a)),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .into(),
        );
    }
    entrance(column(col).spacing(3).width(Length::Fill), t)
}

/// Everything the agent did and said for a turn: the activity section (live
/// step cards while running, a collapsed "N steps" once finished) and the
/// outcome (answer, question, stop, failure).
fn agent_section<'a>(
    state: &'a FerriteBrowser,
    ti: usize,
    turn: &'a Turn,
    live: bool,
) -> Option<Element<'a, FerriteBrowserMessage>> {
    let mut parts: Vec<Element<FerriteBrowserMessage>> = Vec::new();
    if live {
        parts.push(live_activity(state, ti));
    } else if !turn.steps.is_empty() {
        parts.push(finished_activity(state, ti, turn));
    }
    if let Some(outcome) = outcome_card(state, ti, turn) {
        parts.push(outcome);
    }
    if parts.is_empty() {
        return None;
    }
    Some(
        container(column(parts).spacing(ITEM_GAP).width(Length::Fill))
            .width(Length::Fill)
            .padding(Padding {
                bottom: ITEM_GAP,
                ..Padding::ZERO
            })
            .into(),
    )
}

/// The activity container's frame.
fn activity_frame<'a>(
    content: impl Into<Element<'a, FerriteBrowserMessage>>,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    container(content)
        .width(Length::Fill)
        .padding([8, 10])
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(Color {
                a: 0.55,
                ..palette.input
            })),
            border: Border {
                radius: iced::border::Radius::new(10.0),
                width: 1.0,
                color: palette.divider,
            },
            ..container::Style::default()
        })
        .into()
}

/// One step, live or recorded: an icon, the title (with an optional "fast"
/// badge), the action's own parameter and its result (bounded, expandable).
struct StepView<'a> {
    icon: Icon,
    icon_color: Color,
    label: &'a str,
    detail: &'a str,
    result: &'a str,
    blocked: bool,
    fast: bool,
    result_key: String,
    result_expanded: bool,
    alpha: f32,
}

fn step_row<'a>(v: StepView<'a>, palette: &'static Palette) -> Element<'a, FerriteBrowserMessage> {
    let a = v.alpha;
    let mut title: Vec<Element<FerriteBrowserMessage>> = vec![
        icon(v.icon, ICON_SIZE_SM, fade(v.icon_color, a)),
        text(v.label).size(12).color(fade(palette.text, a)).into(),
    ];
    if v.fast {
        title.push(
            container(text("fast").size(9).color(fade(palette.accent, a)))
                .padding([1, 5])
                .style(move |_: &Theme| container::Style {
                    border: Border {
                        radius: iced::border::Radius::new(8.0),
                        width: 1.0,
                        color: fade(palette.accent, a),
                    },
                    ..container::Style::default()
                })
                .into(),
        );
    }
    let mut lines: Vec<Element<FerriteBrowserMessage>> = vec![row(title)
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into()];
    if !v.detail.is_empty() {
        lines.push(
            text(truncate(v.detail, 90))
                .size(11)
                .color(fade(palette.text_dim, a))
                .width(Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph)
                .into(),
        );
    }
    let result_color = if v.blocked {
        palette.danger
    } else if is_error_result(v.result) {
        palette.warn
    } else {
        palette.text_dim
    };
    // A page read is a headline (what page, how far down) with the raw text
    // collapsed under it, not a wall of grey text.
    match crate::agent_run::parse_page_read(v.result).filter(|_| !v.blocked) {
        Some(read) => {
            let mut headline = if read.title.is_empty() {
                read.host.clone()
            } else {
                format!("{} \u{b7} {}", read.title, read.host)
            };
            if let Some(scroll) = &read.scroll {
                headline.push_str(&format!(" \u{b7} {scroll}"));
            }
            lines.push(
                text(truncate(&headline, 110))
                    .size(11)
                    .color(fade(palette.text_dim, a))
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph)
                    .into(),
            );
            if !read.text.is_empty() {
                lines.push(expandable_text(
                    &read.text,
                    PAGE_PREVIEW_CHARS,
                    v.result_expanded,
                    v.result_key,
                    11.0,
                    fade(tint(palette.text_dim, 0.8), a),
                ));
            }
        }
        None => lines.push(expandable_text(
            v.result,
            RESULT_TEXT_LIMIT,
            v.result_expanded,
            v.result_key,
            11.0,
            fade(result_color, a),
        )),
    }
    container(column(lines).spacing(2).width(Length::Fill))
        .padding([4, 0])
        .width(Length::Fill)
        .into()
}

/// The running turn: one card per `agent_log` entry (the existing step-card
/// look), each entering with its own animation, then the "Working" indicator.
fn live_activity(state: &FerriteBrowser, ti: usize) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let turn_id = state
        .chat
        .turns
        .get(ti)
        .map(|t| t.id.to_string())
        .unwrap_or_default();
    let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::new();
    let mut step_index = 0usize;
    for (i, entry) in state.agent_log.iter().enumerate() {
        let t = item_progress(
            &state.thread_anims,
            ItemKey {
                turn: ti,
                slot: SLOT_STEP_BASE + i as u32,
            },
        );
        let a = eased(t);
        match entry {
            AgentLogEntry::Note(s) => rows.push(
                text(s.as_str())
                    .size(11)
                    .color(fade(palette.text_dim, a))
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph)
                    .into(),
            ),
            AgentLogEntry::Step {
                icon: step_icon,
                label,
                detail,
                result,
                blocked,
                fast,
            } => {
                let key = expand_key(&turn_id, ExpandPart::Result(&step_index));
                let result_expanded = state.expanded.contains(&key);
                step_index += 1;
                rows.push(step_row(
                    StepView {
                        icon: *step_icon,
                        icon_color: if *blocked {
                            palette.danger
                        } else {
                            palette.accent
                        },
                        label,
                        detail,
                        result,
                        blocked: *blocked,
                        fast: *fast,
                        result_key: key,
                        result_expanded,
                        alpha: a,
                    },
                    palette,
                ));
            }
        }
    }
    rows.push(working_indicator(state));
    activity_frame(column(rows).spacing(2).width(Length::Fill), palette)
}

/// "Working" with the existing animated dots.
fn working_indicator(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let dots = match ((state.progress_offset * 3.0) as usize) % 4 {
        0 => "",
        1 => ".",
        2 => "..",
        _ => "...",
    };
    row![
        icon(
            Icon::Activity,
            ICON_SIZE_SM,
            fade(
                palette.accent,
                pulse_alpha(state.progress_offset, 1.5, 0.55, 0.45)
            )
        ),
        // A fixed-width slot for the dots so the line never reflows as they
        // change.
        text(format!("Working{dots}"))
            .size(12)
            .color(palette.text_dim)
            .width(Length::Fixed(80.0)),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A finished turn's steps: a "N steps" toggle, collapsed by default.
fn finished_activity<'a>(
    state: &'a FerriteBrowser,
    _ti: usize,
    turn: &'a Turn,
) -> Element<'a, FerriteBrowserMessage> {
    let palette = state.palette();
    let turn_id = turn.id.to_string();
    let key = expand_key(&turn_id, ExpandPart::Steps);
    // A turn that is still open (awaiting consent) has no "finished" summary
    // to collapse to: show its steps.
    let open = matches!(turn.outcome, Outcome::InProgress);
    let expanded = open || state.expanded.contains(&key);
    let blocked = turn.steps.iter().filter(|s| s.blocked).count();

    let header = button(
        row![
            icon(
                if expanded {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                ICON_SIZE_SM,
                palette.text_dim
            ),
            text(steps_summary(turn.steps.len(), blocked))
                .size(12)
                .color(if blocked > 0 {
                    palette.danger
                } else {
                    palette.text_dim
                }),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .padding([4, 8])
    .style(|theme: &Theme, status| {
        let palette = palette_for_theme(theme);
        button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => palette.raised,
                button::Status::Pressed => Color {
                    r: palette.raised.r * 0.85,
                    g: palette.raised.g * 0.85,
                    b: palette.raised.b * 0.85,
                    a: 1.0,
                },
                _ => Color::TRANSPARENT,
            })),
            text_color: palette.text_dim,
            border: Border {
                radius: iced::border::Radius::new(6.0),
                ..Border::default()
            },
            ..button::Style::default()
        }
    })
    .on_press_maybe((!open).then(|| FerriteBrowserMessage::ToggleExpand(key)));

    if !expanded {
        return header.into();
    }
    let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::new();
    for (i, step) in turn.steps.iter().enumerate() {
        rows.push(recorded_step_row(state, &turn_id, i, step));
    }
    column![header, activity_frame(column(rows).spacing(2), palette)]
        .spacing(4)
        .width(Length::Fill)
        .into()
}

fn recorded_step_row<'a>(
    state: &'a FerriteBrowser,
    turn_id: &str,
    i: usize,
    step: &'a StepRecord,
) -> Element<'a, FerriteBrowserMessage> {
    let palette = state.palette();
    let (detail, fast) = agent_run::split_fast_mark(&step.detail);
    let key = expand_key(turn_id, ExpandPart::Result(&i));
    let error = is_error_result(&step.result);
    step_row(
        StepView {
            icon: if step.blocked || error {
                Icon::Warning
            } else {
                Icon::Approve
            },
            icon_color: if step.blocked {
                palette.danger
            } else if error {
                palette.warn
            } else {
                palette.safe
            },
            label: &step.label,
            detail,
            result: &step.result,
            blocked: step.blocked,
            fast,
            result_expanded: state.expanded.contains(&key),
            result_key: key,
            alpha: 1.0,
        },
        palette,
    )
}

/// The turn's outcome as a card: an answer (readable, wrapped, Copy), a
/// question for the user (visually distinct), a stop, a failure, or a quiet
/// "Stopped." for a cancel. Nothing while the turn is still running.
fn outcome_card<'a>(
    state: &'a FerriteBrowser,
    ti: usize,
    turn: &'a Turn,
) -> Option<Element<'a, FerriteBrowserMessage>> {
    let palette = state.palette();
    let t = item_progress(
        &state.thread_anims,
        ItemKey {
            turn: ti,
            slot: SLOT_OUTCOME,
        },
    );
    let a = eased(t);

    let card: Element<FerriteBrowserMessage> = match &turn.outcome {
        Outcome::InProgress => return None,
        Outcome::Answered(answer) => {
            let body = answer_body(answer, palette, a);
            let copy = button(
                row![
                    icon(Icon::Copy, ICON_SIZE_SM, palette.accent),
                    text("Copy").size(11)
                ]
                .spacing(5)
                .align_y(iced::Alignment::Center),
            )
            .padding([3, 8])
            .style(link_button_style)
            .on_press(FerriteBrowserMessage::CopyAnswer(answer.clone()));
            answer_frame(
                column![body, copy].spacing(6).width(Length::Fill),
                palette,
                a,
                false,
            )
        }
        Outcome::AskedUser(question) => {
            let body = answer_body(question, palette, a);
            answer_frame(
                column![
                    row![
                        icon(Icon::Agent, ICON_SIZE_SM, fade(palette.accent, a)),
                        text("The agent has a question")
                            .size(11)
                            .color(fade(palette.accent, a)),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                    body,
                    text("Reply below to continue.")
                        .size(11)
                        .color(fade(palette.text_dim, a)),
                ]
                .spacing(6)
                .width(Length::Fill),
                palette,
                a,
                true,
            )
        }
        Outcome::Stopped(why) => note_card(
            Icon::Warning,
            palette.warn,
            format!("Stopped early: {why}"),
            palette,
            a,
        ),
        Outcome::Failed(why) => note_card(
            Icon::Warning,
            palette.danger,
            format!("Something went wrong: {why}"),
            palette,
            a,
        ),
        Outcome::Cancelled => text("Stopped.")
            .size(11)
            .color(fade(palette.text_dim, a))
            .into(),
    };
    Some(entrance(card, t))
}

/// The most of an answer that is laid out; a longer one ends with a note.
const ANSWER_RENDER_LIMIT: usize = 24_000;

/// What the model wrote, laid out as headings, lists, code, tables and links
/// (see `markdown`).
fn answer_body<'a>(
    answer: &str,
    palette: &'static Palette,
    alpha: f32,
) -> Element<'a, FerriteBrowserMessage> {
    if answer.chars().count() > ANSWER_RENDER_LIMIT {
        let shown: String = answer.chars().take(ANSWER_RENDER_LIMIT).collect();
        return column![
            crate::markdown::view(&shown, palette, 13.0, alpha),
            text("The rest of this answer is not shown. Use Copy to read all of it.")
                .size(11)
                .color(fade(palette.text_dim, alpha)),
        ]
        .spacing(6)
        .into();
    }
    crate::markdown::view(answer, palette, 13.0, alpha)
}

/// The answer/question card frame: a raised surface; the question variant gets
/// an accent border so it reads as a prompt to the user, not a result.
fn answer_frame<'a>(
    content: impl Into<Element<'a, FerriteBrowserMessage>>,
    palette: &'static Palette,
    a: f32,
    question: bool,
) -> Element<'a, FerriteBrowserMessage> {
    container(content)
        .width(Length::Fill)
        .padding([10, 12])
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(fade(palette.raised, a))),
            border: Border {
                radius: iced::border::Radius::new(12.0).bottom_left(4.0),
                width: if question { 1.5 } else { 1.0 },
                color: fade(
                    if question {
                        palette.accent
                    } else {
                        palette.divider
                    },
                    a,
                ),
            },
            ..container::Style::default()
        })
        .into()
}

fn note_card<'a>(
    kind: Icon,
    color: Color,
    message: String,
    palette: &'static Palette,
    a: f32,
) -> Element<'a, FerriteBrowserMessage> {
    container(
        row![
            icon(kind, ICON_SIZE_SM, fade(color, a)),
            text(truncate(&message, 400))
                .size(12)
                .color(fade(palette.text, a))
                .width(Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Start),
    )
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(Color {
            a: 0.10 * a,
            ..color
        })),
        border: Border {
            radius: iced::border::Radius::new(10.0),
            width: 1.0,
            color: Color {
                a: 0.35 * a,
                ..color
            },
        },
        ..container::Style::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

fn empty_state(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let has_page = state
        .tab_urls
        .get(state.active_tab)
        .is_some_and(|u| is_real_page(u));

    let chips: Vec<Element<FerriteBrowserMessage>> = suggestions(has_page)
        .into_iter()
        .map(|s| {
            button(text(s.label).size(12))
                .padding([6, 12])
                .style(|theme: &Theme, status| {
                    let palette = palette_for_theme(theme);
                    button::Style {
                        background: Some(Background::Color(match status {
                            button::Status::Hovered => palette.raised,
                            button::Status::Pressed => Color {
                                r: palette.raised.r * 0.85,
                                g: palette.raised.g * 0.85,
                                b: palette.raised.b * 0.85,
                                a: 1.0,
                            },
                            _ => palette.surface,
                        })),
                        text_color: match status {
                            button::Status::Hovered | button::Status::Pressed => palette.text,
                            _ => palette.text_dim,
                        },
                        border: Border {
                            radius: iced::border::Radius::new(16.0),
                            width: 1.0,
                            color: match status {
                                button::Status::Hovered | button::Status::Pressed => palette.accent,
                                _ => palette.divider,
                            },
                        },
                        ..button::Style::default()
                    }
                })
                .on_press(FerriteBrowserMessage::SuggestionChosen(s.fill.to_string()))
                .into()
        })
        .collect();

    container(
        column![
            container(icon(Icon::Agent, 22.0, palette.accent))
                .width(Length::Fixed(48.0))
                .height(Length::Fixed(48.0))
                .center(Length::Fixed(48.0))
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(Color {
                        a: 0.12,
                        ..palette.accent
                    })),
                    border: Border {
                        radius: iced::border::Radius::new(24.0),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            text("What can I help with?")
                .size(16)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
            text(if has_page {
                "I can read this page, click, fill in forms and work across your tabs."
            } else {
                "I can search, open sites, read pages, click and fill in forms across your tabs."
            })
            .size(12)
            .color(palette.text_dim)
            .align_x(iced::alignment::Horizontal::Center)
            .width(Length::Fill),
            column(chips).spacing(8).align_x(iced::Alignment::Center),
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center)
        .max_width(280),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .padding(16)
    .into()
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

fn history_body(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let blocked = chat_switch_blocked(state);

    if state.chat_list.is_empty() {
        return container(
            column![
                icon(Icon::History, 22.0, palette.text_dim),
                text("No chats yet").size(14).color(palette.text),
                text(if state.chat_store.is_some() {
                    "Your conversations with the agent are saved here."
                } else {
                    "Chats work for this session but cannot be saved on this machine."
                })
                .size(12)
                .color(palette.text_dim)
                .align_x(iced::alignment::Horizontal::Center)
                .width(Length::Fill),
            ]
            .spacing(8)
            .align_x(iced::Alignment::Center)
            .max_width(240),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .padding(16)
        .into();
    }

    let now = chrono::Local::now();
    let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::new();
    if blocked {
        rows.push(
            text("Stop the current run to switch chats.")
                .size(11)
                .color(palette.text_dim)
                .into(),
        );
    }
    for summary in &state.chat_list {
        let is_current = summary.id == state.chat.id;
        let confirming = state.pending_chat_delete.as_ref() == Some(&summary.id);
        let dim = blocked && !is_current;
        let when = relative_time(now, summary.updated_at.with_timezone(&chrono::Local));
        let meta = format!(
            "{when} \u{b7} {} message{}",
            summary.turn_count,
            if summary.turn_count == 1 { "" } else { "s" }
        );
        let title_color = if dim { palette.text_dim } else { palette.text };

        let main = button(
            column![
                text(truncate(&summary.title, 44))
                    .size(13)
                    .font(font_weight(iced::font::Weight::Medium))
                    .color(title_color),
                text(truncate(&summary.last_preview, 80))
                    .size(11)
                    .color(palette.text_dim)
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
                text(meta).size(10).color(fade(palette.text_dim, 0.8)),
            ]
            .spacing(2)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding([8, 10])
        .style(move |theme: &Theme, status| history_row_style(theme, status, is_current))
        .on_press(FerriteBrowserMessage::OpenChat(summary.id.clone()));

        let trash = icon_button(
            Icon::Trash,
            if is_current && blocked {
                fade(palette.text_dim, 0.4)
            } else {
                palette.text_dim
            },
            "Delete chat",
            Some(FerriteBrowserMessage::RequestDeleteChat(summary.id.clone())),
            false,
            palette,
        );

        let top = row![main, trash]
            .spacing(4)
            .align_y(iced::Alignment::Center);
        if confirming {
            rows.push(
                column![
                    top,
                    container(
                        row![
                            text("Delete this chat?")
                                .size(12)
                                .color(palette.danger)
                                .width(Length::Fill),
                            button(text("Cancel").size(11))
                                .padding([3, 9])
                                .style(panel_btn_inactive)
                                .on_press(FerriteBrowserMessage::CancelDeleteChat),
                            button(text("Delete").size(11))
                                .padding([3, 9])
                                .style(move |_: &Theme, status| button::Style {
                                    background: Some(Background::Color(match status {
                                        button::Status::Hovered | button::Status::Pressed => {
                                            Color {
                                                a: 0.85,
                                                ..palette.danger
                                            }
                                        }
                                        _ => palette.danger,
                                    })),
                                    text_color: Color::WHITE,
                                    border: Border {
                                        radius: iced::border::Radius::new(BORDER_RADIUS),
                                        ..Border::default()
                                    },
                                    ..button::Style::default()
                                })
                                .on_press(FerriteBrowserMessage::ConfirmDeleteChat),
                        ]
                        .spacing(6)
                        .align_y(iced::Alignment::Center)
                    )
                    .padding([4, 10]),
                ]
                .spacing(2)
                .into(),
            );
        } else {
            rows.push(top.into());
        }
    }

    scrollable(column(rows).spacing(4).width(Length::Fill).padding([8, 8]))
        .height(Length::Fill)
        .into()
}

fn history_row_style(theme: &Theme, status: button::Status, is_current: bool) -> button::Style {
    let palette = palette_for_theme(theme);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if is_current {
            palette.raised
        } else if hovered {
            Color {
                a: 0.6,
                ..palette.raised
            }
        } else {
            Color::TRANSPARENT
        })),
        text_color: palette.text,
        border: Border {
            radius: iced::border::Radius::new(8.0),
            width: if is_current { 1.0 } else { 0.0 },
            color: palette.accent,
        },
        ..button::Style::default()
    }
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

fn view_composer(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let reviewing = state.pending_diff.is_some();
    let running = state.agent_is_running;
    let has_text = !state.agent_task_input.trim().is_empty();
    // Typing stays possible while the agent works (draft the next message;
    // the input keeps focus), but sending waits for the run to end.
    let can_type = !reviewing;
    let can_send = !running && !reviewing && has_text;

    // The context chip: what *this* message would attach, given what is typed
    // now. Click cycles Auto, On, Off.
    let active_url = state
        .tab_urls
        .get(state.active_tab)
        .map_or("", String::as_str);
    let would_use_page = decide_page_use(
        &state.agent_task_input,
        &state.chat.turns,
        active_url,
        state.context_mode,
    )
    .use_page;
    let chip = button(
        row![
            text(agent_run::context_mode_label(state.context_mode))
                .size(11)
                .font(font_weight(iced::font::Weight::Semibold)),
            text(describe_context(would_use_page, state.tabs.len())).size(11),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .padding([3, 9])
    .style(move |theme: &Theme, status| {
        let palette = palette_for_theme(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let on = would_use_page;
        button::Style {
            background: Some(Background::Color(if hovered {
                palette.raised
            } else {
                palette.surface
            })),
            text_color: if hovered {
                palette.text
            } else if on {
                palette.accent
            } else {
                palette.text_dim
            },
            border: Border {
                radius: iced::border::Radius::new(12.0),
                width: 1.0,
                color: if hovered || on {
                    palette.accent
                } else {
                    palette.divider
                },
            },
            ..button::Style::default()
        }
    })
    .on_press(FerriteBrowserMessage::CycleContextMode);

    let mut chip_row: Vec<Element<FerriteBrowserMessage>> = vec![tip(
        chip,
        "Whether the agent gets the current page. Click: Auto, On, Off.",
        palette,
    )];
    chip_row.push(horizontal_space().into());
    if !state.thread_pinned
        && state.sidebar_view == SidebarView::Thread
        && !state.chat.turns.is_empty()
    {
        chip_row.push(
            button(
                row![
                    icon(Icon::ChevronDown, ICON_SIZE_SM, palette.accent),
                    text("Latest").size(11)
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            )
            .padding([3, 8])
            .style(link_button_style)
            .on_press(FerriteBrowserMessage::ScrollThreadToBottom)
            .into(),
        );
    }

    let placeholder = composer_placeholder(
        state.chat.turns.last().map(|t| &t.outcome),
        running,
        reviewing,
    );
    let input = text_input(placeholder, &state.agent_task_input)
        .id(text_input::Id::new(AGENT_INPUT_ID))
        .width(Length::Fill)
        .padding([8, 10])
        .size(13)
        .style(|theme: &Theme, status| field_style(theme, status, 10.0))
        .on_input_maybe(can_type.then_some(
            FerriteBrowserMessage::AgentTaskInputChanged as fn(String) -> FerriteBrowserMessage,
        ))
        .on_submit_maybe(can_send.then_some(FerriteBrowserMessage::AgentTaskSubmitted));

    // Send, which becomes Stop while a run is active (as the header's Stop did).
    let send: Element<FerriteBrowserMessage> = if running {
        button(container(icon(Icon::Stop, ICON_SIZE, on_fill(palette.danger))).center(Length::Fill))
            .width(Length::Fixed(SEND_BTN))
            .height(Length::Fixed(SEND_BTN))
            .padding(0)
            .style(danger_btn_style)
            .on_press(FerriteBrowserMessage::StopAgent)
            .into()
    } else {
        button(
            container(icon(
                Icon::Send,
                ICON_SIZE,
                if can_send {
                    on_fill(palette.accent_fill)
                } else {
                    palette.text_dim
                },
            ))
            .center(Length::Fill),
        )
        .width(Length::Fixed(SEND_BTN))
        .height(Length::Fixed(SEND_BTN))
        .padding(0)
        .style(if can_send {
            accent_btn_style
        } else {
            panel_btn_inactive
        })
        .on_press_maybe(can_send.then_some(FerriteBrowserMessage::AgentTaskSubmitted))
        .into()
    };
    let send = tip(
        send,
        if running {
            "Stop the agent"
        } else {
            "Send (Enter)"
        },
        palette,
    );

    container(
        column![
            row(chip_row).align_y(iced::Alignment::Center),
            row![input, send]
                .spacing(8)
                .align_y(iced::Alignment::Center),
        ]
        .spacing(6)
        .width(Length::Fill),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Consent panel — the security surface (moved here unchanged in content)
// ---------------------------------------------------------------------------

/// One reviewable item: what the dry run did, and a Reject / Approve pair. The
/// decided side is filled (danger / safe); the undecided side is an outline, so
/// "nothing chosen yet" never looks like "approved".
fn consent_item_card<'a>(
    state: &FerriteBrowser,
    item: &ConsentItem,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let approved = state.pending_decision.approved.contains(&item.id);
    let rejected = state.pending_decision.rejected.contains(&item.id);

    // An out-of-scope-origin item gets a small external-link glyph ahead of
    // its summary — the one place this crate renders `Icon::Origin`,
    // distinguishing "contacted an unauthorized origin" rows from "used an
    // unexpected tool" rows at a glance, on top of the text difference already
    // in `item.summary` itself.
    let lead = if origin_item_origin(&item.id).is_some() {
        Icon::Origin
    } else {
        Icon::Warning
    };
    let summary = row![
        icon(lead, ICON_SIZE_SM, palette.text_dim),
        text(item.summary.clone())
            .size(TEXT_SMALL)
            .color(palette.text)
            .width(Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(SP_SM)
    .align_y(iced::Alignment::Start);

    // Reject is listed first and styled as the safe default: `consent_is_complete`
    // never lets Proceed fire while any item, including this one, is undecided,
    // so there is no path to a silent approve-by-default. (iced 0.13 buttons
    // are not keyboard-focusable, so the reading order is the available
    // equivalent of a focus default.)
    let choice = |label: &'static str,
                  glyph: Icon,
                  chosen: bool,
                  chosen_style: fn(&Theme, button::Status) -> button::Style,
                  chosen_fill: Color,
                  msg: FerriteBrowserMessage| {
        button(
            container(
                row![
                    icon(
                        glyph,
                        11.0,
                        if chosen {
                            on_fill(chosen_fill)
                        } else {
                            palette.text
                        }
                    ),
                    text(label).size(TEXT_SMALL)
                ]
                .spacing(SP_XS + 2.0)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .center_x(Length::Fill),
        )
        .width(Length::FillPortion(1))
        .padding([SP_XS + 1.0, SP_SM])
        .style(if chosen {
            chosen_style
        } else {
            outline_btn_style
        })
        .on_press(msg)
    };

    // The surface is neutral; the rule down the left edge is the one colour
    // that says where the item stands: amber while undecided, green once
    // approved, red once rejected.
    let tone = if approved {
        palette.safe
    } else if rejected {
        palette.danger
    } else {
        palette.warn
    };
    rule_card(
        palette,
        tone,
        column![
            summary,
            row![
                choice(
                    "Reject",
                    Icon::Reject,
                    rejected,
                    danger_btn_style,
                    palette.danger,
                    FerriteBrowserMessage::RejectTool(item.id.to_string()),
                ),
                choice(
                    "Approve",
                    Icon::Approve,
                    approved,
                    safe_btn_style,
                    palette.safe,
                    FerriteBrowserMessage::ApproveTool(item.id.to_string()),
                ),
            ]
            .spacing(SP_SM),
        ]
        .spacing(SP_SM)
        .into(),
    )
}

/// The consent panel: shown instead of the thread while a dry run's deviation
/// awaits the user's decision. The items scroll; the decision (Proceed /
/// Cancel) is pinned underneath so it is always on screen, with a plain line
/// saying what is still undecided.
///
/// This is the security surface (`docs/REBUILD_DIRECTIVE.md` §6/A10): it is
/// built entirely from `iced_widget` native widgets, laid out here from Rust
/// values (`diff`/`expected`/`evidence` — plain Rust structs, never
/// page-supplied markup). Servo's page content only ever reaches this process
/// as a decoded pixel buffer, rendered elsewhere as an `iced_widget::image`,
/// so there is no code path by which a page's HTML/CSS/text is parsed into a
/// style, position, or z-order for *this* panel. See
/// `page_content_cannot_reach_the_consent_panels_inputs` for the structural
/// argument.
fn consent_body(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let Some(diff) = &state.pending_diff else {
        return column![].into();
    };
    let items = consent_items(diff, state.pending_expected.as_ref());
    let decided = items
        .iter()
        .filter(|i| {
            state.pending_decision.approved.contains(&i.id)
                || state.pending_decision.rejected.contains(&i.id)
        })
        .count();
    let complete = consent_is_complete(diff, &state.pending_decision);

    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![
        row![
            icon(Icon::Warning, ICON_SIZE, palette.danger),
            text("Review before running")
                .size(TEXT_TITLE)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text)
                .width(Length::Fill),
            text(format!("{decided} of {} decided", items.len()))
                .size(TEXT_CAPTION)
                .color(palette.text_dim),
        ]
        .spacing(SP_SM)
        .align_y(iced::Alignment::Center)
        .into(),
        text(diff.summary())
            .size(TEXT_SMALL)
            .color(palette.text_dim)
            .wrapping(text::Wrapping::Word)
            .into(),
    ];
    for item in &items {
        body.push(consent_item_card(state, item, palette));
    }

    // ── Dry-run evidence, collapsed by default ──
    body.push(
        button(
            row![
                icon(
                    if state.show_evidence {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    ICON_SIZE_SM,
                    palette.text_dim
                ),
                text(if state.show_evidence {
                    "Hide dry-run evidence"
                } else {
                    "Show dry-run evidence"
                })
                .size(TEXT_SMALL),
            ]
            .spacing(SP_XS + 2.0)
            .align_y(iced::Alignment::Center),
        )
        .padding([SP_XS, SP_SM])
        .style(link_button_style)
        .on_press(FerriteBrowserMessage::ToggleEvidence)
        .into(),
    );
    if state.show_evidence {
        if let Some(evidence) = &state.pending_evidence {
            let lines: Vec<Element<FerriteBrowserMessage>> = dry_run_evidence_lines(evidence)
                .into_iter()
                .map(|line| {
                    text(line)
                        .size(TEXT_CAPTION)
                        .color(palette.text_dim)
                        .wrapping(text::Wrapping::WordOrGlyph)
                        .into()
                })
                .collect();
            body.push(
                container(column(lines).spacing(2))
                    .padding([SP_SM - 2.0, SP_SM])
                    .width(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(Background::Color(palette.input)),
                        border: Border {
                            radius: RADIUS_SM.into(),
                            width: 1.0,
                            color: palette.divider,
                        },
                        ..container::Style::default()
                    })
                    .into(),
            );
        }
    }

    // ── Entrance transition (C1) ──
    // A brief slide-in-and-settle plus a background-tint fade, driven by
    // `consent_panel_anim` (advanced 16ms at a time by `ConsentPanelTick`, see
    // `subscription()`) through `ease_out_cubic`. Purely decorative: every
    // item's own text above is already at full opacity/its final position from
    // the very first frame — only this wrapper's background tint and top inset
    // animate, so nothing about what the user is being asked to approve is
    // ever delayed, dimmed, or obscured while this plays out (~200ms total).
    let anim_t = ease_out_cubic(state.consent_panel_anim);
    let slide_offset = (1.0 - anim_t) * 16.0;
    let items_view = scrollable(
        container(column(body).spacing(SP_SM + 2.0).padding(Padding {
            top: SP_MD + slide_offset,
            right: SP_MD,
            bottom: SP_MD,
            left: SP_MD,
        }))
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            // Neutral: the panel's own surface. (This used to wash the whole
            // review in amber, which on the dark theme read as brown.)
            background: Some(Background::Color(palette.surface)),
            ..container::Style::default()
        }),
    )
    .height(Length::Fill);

    // ── The decision, pinned ──
    let proceed = button(
        container(text("Proceed with approved").size(TEXT_BODY))
            .width(Length::Fill)
            .center_x(Length::Fill),
    )
    .width(Length::Fill)
    .padding([SP_SM + 1.0, SP_MD])
    .style(accent_btn_style)
    .on_press_maybe(complete.then_some(FerriteBrowserMessage::ConsentSubmitted));
    let cancel = button(
        row![
            text("Cancel").size(TEXT_BODY),
            text("Esc").size(TEXT_CAPTION).color(palette.text_dim)
        ]
        .spacing(SP_SM)
        .align_y(iced::Alignment::Center),
    )
    .padding([SP_SM + 1.0, SP_LG])
    .style(outline_btn_style)
    .on_press(FerriteBrowserMessage::ConsentCancelled);
    let footer = container(
        column![
            if complete {
                text("Only what you approved will run.")
            } else {
                text("Approve or reject every item to continue.")
            }
            .size(TEXT_CAPTION)
            .color(palette.text_dim),
            row![proceed, cancel]
                .spacing(SP_SM)
                .align_y(iced::Alignment::Center),
        ]
        .spacing(SP_SM),
    )
    .padding(SP_MD)
    .width(Length::Fill)
    .style(raised_bar_style);

    column![items_view, sep(), footer]
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};

    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn relative_time_reads_naturally_from_now_to_a_date() {
        let now = at(0);
        assert_eq!(relative_time(now, at(-10)), "just now");
        assert_eq!(relative_time(now, at(5)), "just now", "clock skew");
        assert_eq!(relative_time(now, at(-50)), "1m ago");
        assert_eq!(relative_time(now, at(-12 * 60)), "12m ago");
        assert_eq!(relative_time(now, at(-59 * 60)), "59m ago");
        assert_eq!(relative_time(now, at(-3 * 3600)), "3h ago");
    }

    #[test]
    fn relative_time_uses_calendar_days_for_yesterday_and_dates() {
        // 2023-11-15 12:00 UTC.
        let noon = DateTime::from_timestamp(1_700_049_600, 0).unwrap();
        let yesterday_evening = noon - chrono::Duration::hours(15); // 21:00 the day before
        assert_eq!(relative_time(noon, yesterday_evening), "yesterday");
        assert_eq!(
            relative_time(noon, noon - chrono::Duration::days(3)),
            "3d ago"
        );
        assert_eq!(
            relative_time(noon, noon - chrono::Duration::days(20)),
            "Oct 26"
        );
        assert_eq!(
            relative_time(noon, noon - chrono::Duration::days(500)),
            "Jul 3, 2022"
        );
        // Late last night, a few hours ago but on the previous date.
        let just_after_midnight = DateTime::from_timestamp(1_700_010_000, 0).unwrap(); // 00:20 Nov 15
        let late_last_night = just_after_midnight - chrono::Duration::hours(3);
        assert_eq!(
            relative_time(just_after_midnight, late_last_night),
            "yesterday"
        );
    }

    #[test]
    fn relative_time_respects_the_readers_time_zone() {
        let east = FixedOffset::east_opt(10 * 3600).unwrap();
        // 2023-11-14 22:00 UTC is already Nov 15 08:00 in +10:00.
        let now = DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap()
            .with_timezone(&east);
        let then = (now.with_timezone(&Utc) - chrono::Duration::hours(10)).with_timezone(&east);
        assert_eq!(relative_time(now, then), "yesterday");
    }

    #[test]
    fn suggestions_adapt_to_whether_a_page_is_open() {
        let page = suggestions(true);
        assert_eq!(page[0].label, "Summarize this page");
        assert!(page.iter().any(|s| s.label == "Find the important links"));
        assert!(page.iter().any(|s| s.label == "Fill in this form"));
        let blank = suggestions(false);
        assert!(blank[0].label.starts_with("Search the web for"));
        assert!(blank.iter().any(|s| s.label.starts_with("Open a site")));
        // Open-ended chips leave the composer waiting for the rest.
        for s in blank {
            assert!(s.fill.ends_with(' '), "{:?}", s.fill);
        }
        assert_ne!(page, blank);
    }

    #[test]
    fn only_http_pages_count_as_a_real_page() {
        assert!(is_real_page("https://a.example"));
        assert!(is_real_page("http://localhost:3000"));
        for no in ["about:blank", "", "file:///x", "data:text/html,x"] {
            assert!(!is_real_page(no), "{no}");
        }
    }

    #[test]
    fn steps_summary_pluralizes_and_counts_blocked() {
        assert_eq!(steps_summary(1, 0), "1 step");
        assert_eq!(steps_summary(3, 0), "3 steps");
        assert_eq!(steps_summary(3, 1), "3 steps \u{b7} 1 blocked");
    }

    #[test]
    fn pinned_to_bottom_tracks_the_scroll_offset_with_slack() {
        // 1000px of content in a 400px viewport.
        assert!(
            is_pinned_to_bottom(600.0, 400.0, 1000.0),
            "exactly at the bottom"
        );
        assert!(is_pinned_to_bottom(590.0, 400.0, 1000.0), "within slack");
        assert!(
            !is_pinned_to_bottom(300.0, 400.0, 1000.0),
            "scrolled up to read"
        );
        assert!(!is_pinned_to_bottom(0.0, 400.0, 1000.0));
        // Nothing to scroll: always "pinned", so the first messages follow.
        assert!(is_pinned_to_bottom(0.0, 400.0, 250.0));
        // Garbage (a NaN relative offset) never strands the thread.
        assert!(is_pinned_to_bottom(f32::NAN, 400.0, 1000.0));
    }

    #[test]
    fn page_note_says_what_context_the_agent_had() {
        let used = PageContextNote {
            url: "https://shop.example/cart".into(),
            title: "Your cart".into(),
            used_full_page: true,
            reason: "the message refers to the current page".into(),
        };
        assert_eq!(page_note_text(&used), "Used current page \u{b7} Your cart");
        let untitled = PageContextNote {
            title: String::new(),
            ..used.clone()
        };
        assert_eq!(
            page_note_text(&untitled),
            "Used current page \u{b7} shop.example/cart"
        );
        let unused = PageContextNote {
            used_full_page: false,
            reason: "the message names a different site".into(),
            ..used.clone()
        };
        assert_eq!(
            page_note_text(&unused),
            "Page not used \u{b7} the message names a different site"
        );
        let no_reason = PageContextNote {
            used_full_page: false,
            reason: String::new(),
            ..used
        };
        assert_eq!(page_note_text(&no_reason), "Page not used");
    }

    #[test]
    fn the_composer_asks_for_a_reply_after_a_question() {
        let asked = Outcome::AskedUser("which one?".into());
        assert_eq!(
            composer_placeholder(Some(&asked), false, false),
            "Reply to the agent\u{2026}"
        );
        assert_eq!(
            composer_placeholder(Some(&Outcome::Answered("x".into())), false, false),
            "Message the agent\u{2026}"
        );
        assert_eq!(
            composer_placeholder(None, false, false),
            "Message the agent\u{2026}"
        );
        assert_eq!(
            composer_placeholder(Some(&asked), true, false),
            "Working\u{2026} you can type ahead"
        );
        assert!(composer_placeholder(None, false, true).starts_with("Review"));
    }

    #[test]
    fn expand_keys_are_distinct_per_turn_and_part() {
        let a = expand_key("t1", ExpandPart::Steps);
        assert_ne!(a, expand_key("t2", ExpandPart::Steps));
        assert_ne!(
            expand_key("t1", ExpandPart::Result(&0)),
            expand_key("t1", ExpandPart::Result(&1))
        );
    }

    #[test]
    fn entrance_animation_starts_advances_and_finishes() {
        let key = ItemKey {
            turn: 2,
            slot: SLOT_USER,
        };
        let other = ItemKey {
            turn: 2,
            slot: SLOT_OUTCOME,
        };
        let mut anims = Vec::new();
        assert_eq!(
            item_progress(&anims, key),
            1.0,
            "not animating means fully in"
        );

        start_item_anim(&mut anims, key);
        start_item_anim(&mut anims, other);
        start_item_anim(&mut anims, key); // restarting does not duplicate
        assert_eq!(anims.len(), 2);
        assert_eq!(item_progress(&anims, key), 0.0);

        advance_anims(&mut anims, 0.4);
        assert!((item_progress(&anims, key) - 0.4).abs() < 1e-6);
        advance_anims(&mut anims, 0.4);
        advance_anims(&mut anims, 0.4);
        assert!(anims.is_empty(), "finished animations are dropped");
        assert_eq!(item_progress(&anims, key), 1.0);
    }

    #[test]
    fn fade_scales_alpha_and_clamps() {
        let c = Color {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 0.8,
        };
        assert!((fade(c, 0.5).a - 0.4).abs() < 1e-6);
        assert_eq!(fade(c, 2.0).a, 0.8);
        assert_eq!(fade(c, -1.0).a, 0.0);
        assert_eq!(fade(c, 0.5).r, 1.0);
    }

    #[test]
    fn entrance_slide_fits_inside_the_item_gap() {
        // The slide must never need more room than the gap provides, or the
        // item's height would change mid-animation.
        const _: () = assert!(ENTRANCE_SLIDE <= ITEM_GAP);
    }

    #[test]
    fn error_results_are_recognised() {
        assert!(is_error_result("error: no tab 9"));
        assert!(is_error_result("  Error: x"));
        assert!(!is_error_result("clicked @3"));
    }
}
