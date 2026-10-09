//! The DevTools panel: Console, Network and Engine tabs under a bottom drawer.
//!
//! State and rules live in `devtools`; this draws them. Rows are built only for
//! what is shown (the newest `RENDER_STEP`, with "show older" for the rest), and
//! which rows match the filter is cached incrementally (`devtools::FilterCache`),
//! so an open panel costs one window of rows per tick, not the whole log.

use ferrite_servo::diag::ConsoleLevel;
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, stack, text, text_input, Space,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Theme};

use crate::agent_panel::link_button_style;
use crate::devtools::{
    self, clock_ms, ConsoleRow, DevTab, EngineEvent, EngineKind, LevelFilter, Msg, NetFilter,
    NetKind, NetRow, RowKind, Section, RENDER_STEP,
};
use crate::icons::{icon, Icon};
use crate::tokens::{
    accent_btn_style, close_btn_style, field_style, hover_bg, mix, on_fill, panel_btn_inactive,
    raised_bar_style, tint, tip, SP_MD, SP_SM, SP_XS, TEXT_BODY, TEXT_CAPTION, TEXT_SMALL,
};
use crate::widgets::PressProbe;
use crate::{
    font_weight, FerriteBrowser, FerriteBrowserMessage, Palette, DEVTOOLS_SHORTCUT, JS_INPUT_ID,
};

/// Padding at a row's left and right edge.
const EDGE: f32 = 12.0;
/// A message is folded to this many lines / characters until expanded.
const FOLD_LINES: usize = 4;
const FOLD_CHARS: usize = 320;
/// A location is shown at most this many characters long (the full text is
/// what Copy takes).
const SOURCE_CHARS: usize = 46;
/// Width of the Network tab's size column.
const NET_SIZE_W: f32 = 76.0;
/// Width of the Network tab's duration column.
const NET_DURATION_W: f32 = 56.0;

/// Shown URL length in a collapsed Network row.
const URL_CHARS: usize = 140;
/// The window width below which the header's buttons lose their labels.
const COMPACT_BELOW: f32 = 900.0;
/// Width of the time column in the Console and the first Network column.
const TIME_W: f32 = 84.0;
const NET_TIME_W: f32 = 64.0;

fn dev(msg: Msg) -> FerriteBrowserMessage {
    FerriteBrowserMessage::DevTools(msg)
}

fn mono() -> Font {
    Font::MONOSPACE
}

// ── Text rules ───────────────────────────────────────────────────────────

/// Whether a message is long enough to fold.
pub(crate) fn is_long(message: &str) -> bool {
    message.lines().count() > FOLD_LINES || message.chars().count() > FOLD_CHARS
}

/// A message folded to its first lines and characters, with an ellipsis.
pub(crate) fn fold(message: &str) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    for (i, line) in message.lines().enumerate() {
        if i >= FOLD_LINES || chars >= FOLD_CHARS {
            out.push('\u{2026}');
            return out;
        }
        if i > 0 {
            out.push('\n');
        }
        let room = FOLD_CHARS - chars;
        if line.chars().count() > room {
            out.extend(line.chars().take(room));
            out.push('\u{2026}');
            return out;
        }
        out.push_str(line);
        chars += line.chars().count();
    }
    out
}

/// A `file:line:col` cut from the front when it is long, so the file name and
/// position (the useful end) stay.
pub(crate) fn shorten_source(source: &str) -> String {
    let count = source.chars().count();
    if count <= SOURCE_CHARS {
        return source.to_string();
    }
    let tail: String = source.chars().skip(count - (SOURCE_CHARS - 1)).collect();
    format!("\u{2026}{tail}")
}

// ── Styles ───────────────────────────────────────────────────────────────

fn chip_style(
    palette: &'static Palette,
    active: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_: &Theme, status: button::Status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if active {
                tint(palette.accent, if hover { 0.28 } else { 0.20 })
            } else if hover {
                hover_bg(palette)
            } else {
                Color::TRANSPARENT
            })),
            text_color: if active {
                palette.accent_bright
            } else if hover {
                palette.text
            } else {
                palette.text_dim
            },
            border: Border {
                radius: 100.0.into(),
                width: 1.0,
                color: if active {
                    tint(palette.accent, 0.55)
                } else {
                    palette.divider
                },
            },
            ..button::Style::default()
        }
    }
}

fn pill<'a>(label: String, fill: Color) -> Element<'a, FerriteBrowserMessage> {
    let ink = on_fill(fill);
    container(text(label).size(TEXT_CAPTION).color(ink))
        .padding([0.0, 6.0])
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(fill)),
            border: Border {
                radius: 100.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// The glyph in a message's gutter. Plain logs carry none, as in DevTools: the
/// icon is for what needs a second look.
fn level_icon(palette: &'static Palette, level: ConsoleLevel) -> Option<(Icon, Color)> {
    match level {
        ConsoleLevel::Error => Some((Icon::Reject, palette.danger)),
        ConsoleLevel::Warn => Some((Icon::Warning, palette.warn)),
        ConsoleLevel::Info => Some((Icon::Info, palette.accent_bright)),
        ConsoleLevel::Log => None,
        ConsoleLevel::Debug => Some((Icon::Bug, tint(palette.text_dim, 0.8))),
    }
}

fn kind_color(palette: &'static Palette, kind: NetKind) -> Color {
    match kind {
        NetKind::Document => palette.accent_bright,
        NetKind::Script => palette.warn,
        NetKind::Style => palette.safe,
        NetKind::Image => mix(palette.accent_bright, palette.safe, 0.5),
        NetKind::Font => mix(palette.text_dim, palette.accent_bright, 0.4),
        NetKind::Media => mix(palette.danger, palette.warn, 0.5),
        NetKind::Other => palette.text_dim,
    }
}

// ── The panel ────────────────────────────────────────────────────────────

pub(crate) fn view(state: &FerriteBrowser) -> Element<'_, FerriteBrowserMessage> {
    let palette = state.palette();
    let ui = &state.devtools;
    let compact = state.window_size.width < COMPACT_BELOW;
    let mut parts: Vec<Element<FerriteBrowserMessage>> =
        vec![header(state, palette, compact), toolbar(state, palette)];
    parts.push(match ui.tab {
        DevTab::Console => console_list(state, palette),
        DevTab::Network => network_list(state, palette),
        DevTab::Engine => engine_list(state, palette),
    });
    if ui.tab == DevTab::Console {
        parts.push(prompt(state, palette));
    }
    column(parts).into()
}

fn tab_button<'a>(
    palette: &'static Palette,
    label: &'static str,
    tab: DevTab,
    active: bool,
    badges: Vec<Element<'a, FerriteBrowserMessage>>,
) -> Element<'a, FerriteBrowserMessage> {
    let mut cells: Vec<Element<FerriteBrowserMessage>> = vec![text(label)
        .size(TEXT_BODY)
        .font(font_weight(if active {
            iced::font::Weight::Semibold
        } else {
            iced::font::Weight::Normal
        }))
        .into()];
    cells.extend(badges);
    // The underline is a second layer over the button, so the tab is exactly
    // as wide as its label (a column with a full-width underline would make
    // every tab share the header equally).
    stack![
        button(row(cells).spacing(SP_XS + 2.0).align_y(Alignment::Center))
            .padding([SP_XS + 2.0, SP_MD])
            .style(move |_: &Theme, status: button::Status| button::Style {
                background: Some(Background::Color(
                    if matches!(status, button::Status::Hovered) && !active {
                        hover_bg(palette)
                    } else {
                        Color::TRANSPARENT
                    },
                )),
                text_color: if active {
                    palette.text
                } else {
                    palette.text_dim
                },
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
            .on_press(dev(Msg::SetTab(tab))),
        container(
            container(Space::new(Length::Fill, Length::Fixed(2.0)))
                .width(Length::Fill)
                .style(move |_: &Theme| container::Style {
                    background: active.then_some(Background::Color(palette.accent)),
                    ..container::Style::default()
                })
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::End),
    ]
    .into()
}

fn header<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
    compact: bool,
) -> Element<'a, FerriteBrowserMessage> {
    let ui = &state.devtools;
    let diag = state.tab_diag.get(state.active_tab);
    let (errors, warnings, requests) = diag.map_or((0, 0, 0), |d| {
        (
            d.log.error_count(),
            d.log.warning_count(),
            d.log.kind_count(NetFilter::All),
        )
    });
    let panics = state.engine_log.len();

    let mut console_badges = Vec::new();
    if errors > 0 {
        console_badges.push(pill(errors.to_string(), palette.danger));
    }
    if warnings > 0 {
        console_badges.push(pill(warnings.to_string(), palette.warn));
    }
    let network_badges = if requests > 0 {
        vec![text(requests.to_string())
            .size(TEXT_CAPTION)
            .color(palette.text_dim)
            .into()]
    } else {
        vec![]
    };
    let engine_badges = if panics > 0 {
        vec![pill(panics.to_string(), palette.danger)]
    } else {
        vec![]
    };

    let action = |glyph: Icon, label: &'static str, tip_text: &'static str, msg: Msg| {
        let content: Element<FerriteBrowserMessage> = if compact {
            icon(glyph, 14.0, palette.text_dim)
        } else {
            row![
                icon(glyph, 13.0, palette.text_dim),
                text(label).size(TEXT_SMALL)
            ]
            .spacing(SP_XS + 2.0)
            .align_y(Alignment::Center)
            .into()
        };
        tip(
            button(content)
                .padding([SP_XS, if compact { SP_SM } else { SP_MD }])
                .style(panel_btn_inactive)
                .on_press(dev(msg)),
            tip_text,
            palette,
        )
    };

    let mut items: Vec<Element<FerriteBrowserMessage>> = vec![
        tab_button(
            palette,
            "Console",
            DevTab::Console,
            ui.tab == DevTab::Console,
            console_badges,
        ),
        tab_button(
            palette,
            "Network",
            DevTab::Network,
            ui.tab == DevTab::Network,
            network_badges,
        ),
        tab_button(
            palette,
            "Engine",
            DevTab::Engine,
            ui.tab == DevTab::Engine,
            engine_badges,
        ),
        text(DEVTOOLS_SHORTCUT)
            .size(TEXT_CAPTION)
            .color(palette.text_dim)
            .into(),
        Space::with_width(Length::Fill).into(),
    ];
    if let Some(notice) = &ui.notice {
        items.push(
            text(crate::truncate(notice, 70))
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .wrapping(text::Wrapping::None)
                .into(),
        );
    }
    items.push(action(
        Icon::Copy,
        "Copy all",
        "Copy everything in this tab",
        Msg::CopyAll,
    ));
    items.push(action(
        Icon::Download,
        "Save log\u{2026}",
        "Save this tab as a text file in the log folder",
        Msg::SaveLog,
    ));
    items.push(action(
        Icon::Folder,
        "Open log folder",
        "Open the folder the app's log files are in",
        Msg::OpenLogFolder,
    ));
    items.push(tip(
        button(icon(Icon::Close, 10.0, palette.text_dim))
            .padding(5)
            .style(close_btn_style)
            .on_press(FerriteBrowserMessage::ToggleJsConsole),
        "Close",
        palette,
    ));
    container(
        row(items)
            .spacing(SP_SM)
            .align_y(Alignment::Center)
            .padding([0.0, EDGE]),
    )
    .width(Length::Fill)
    .style(raised_bar_style)
    .into()
}

fn search_field<'a>(
    placeholder: &'static str,
    value: &'a str,
    on_input: fn(String) -> FerriteBrowserMessage,
) -> Element<'a, FerriteBrowserMessage> {
    text_input(placeholder, value)
        .on_input(on_input)
        .size(TEXT_SMALL)
        .padding([3.0, SP_SM])
        .width(Length::Fixed(220.0))
        .style(|theme: &Theme, status| field_style(theme, status, 6.0))
        .into()
}

fn toolbar<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let ui = &state.devtools;
    let diag = state.tab_diag.get(state.active_tab);

    let clear = tip(
        button(icon(Icon::Ban, 13.0, palette.text_dim))
            .padding(SP_XS + 1.0)
            .style(close_btn_style)
            .on_press(dev(Msg::Clear)),
        match ui.tab {
            DevTab::Console => "Clear console",
            DevTab::Network => "Clear requests",
            DevTab::Engine => "Clear panics and crashes",
        },
        palette,
    );
    let mut items: Vec<Element<FerriteBrowserMessage>> = vec![clear];

    match ui.tab {
        DevTab::Console => {
            items.push(search_field("Filter", &ui.search, |t| dev(Msg::Search(t))));
            let chips: Vec<Element<FerriteBrowserMessage>> = LevelFilter::ALL
                .iter()
                .map(|f| {
                    let count = diag.map_or(0, |d| d.log.level_count(*f));
                    chip(
                        palette,
                        f.label(),
                        count,
                        ui.level == *f,
                        dev(Msg::SetLevel(*f)),
                    )
                })
                .collect();
            items.push(row(chips).spacing(SP_XS + 2.0).wrap().into());
            items.push(Space::with_width(Length::Fill).into());
            items.push(preserve_toggle(palette, ui.preserve));
        }
        DevTab::Network => {
            items.push(search_field("Filter URLs", &ui.net_search, |t| {
                dev(Msg::NetSearch(t))
            }));
            let mut filters = vec![NetFilter::All];
            filters.extend(NetKind::ALL.iter().map(|k| NetFilter::Kind(*k)));
            let chips: Vec<Element<FerriteBrowserMessage>> = filters
                .into_iter()
                .map(|f| {
                    let count = diag.map_or(0, |d| d.log.kind_count(f));
                    let label = match f {
                        NetFilter::All => "All",
                        NetFilter::Kind(k) => k.label(),
                    };
                    chip(
                        palette,
                        label,
                        count,
                        ui.net_filter == f,
                        dev(Msg::SetNetFilter(f)),
                    )
                })
                .collect();
            items.push(row(chips).spacing(SP_XS + 2.0).wrap().into());
            items.push(Space::with_width(Length::Fill).into());
            items.push(preserve_toggle(palette, ui.preserve));
        }
        DevTab::Engine => {
            items.push(
                text("Panics on engine threads and pages whose script thread died, this session.")
                    .size(TEXT_SMALL)
                    .color(palette.text_dim)
                    .into(),
            );
        }
    }
    container(
        row(items)
            .spacing(SP_SM)
            .align_y(Alignment::Center)
            .padding([SP_XS + 1.0, EDGE]),
    )
    .width(Length::Fill)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(palette.surface)),
        border: Border {
            width: 0.0,
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

fn chip<'a>(
    palette: &'static Palette,
    label: &'static str,
    count: usize,
    active: bool,
    msg: FerriteBrowserMessage,
) -> Element<'a, FerriteBrowserMessage> {
    button(
        row![
            text(label).size(TEXT_SMALL),
            text(count.to_string()).size(TEXT_CAPTION)
        ]
        .spacing(SP_XS + 1.0)
        .align_y(Alignment::Center),
    )
    .padding([2.0, SP_SM + 1.0])
    .style(chip_style(palette, active))
    .on_press(msg)
    .into()
}

fn preserve_toggle<'a>(palette: &'static Palette, on: bool) -> Element<'a, FerriteBrowserMessage> {
    let boxed = container(if on {
        icon(Icon::Check, 10.0, on_fill(palette.accent_fill))
    } else {
        Space::new(0.0, 0.0).into()
    })
    .width(Length::Fixed(14.0))
    .height(Length::Fixed(14.0))
    .center(Length::Fixed(14.0))
    .style(move |_: &Theme| container::Style {
        background: on.then_some(Background::Color(palette.accent_fill)),
        border: Border {
            radius: 3.0.into(),
            width: 1.5,
            color: if on {
                palette.accent_fill
            } else {
                palette.text_dim
            },
        },
        ..container::Style::default()
    });
    tip(
        button(
            row![boxed, text("Preserve log").size(TEXT_SMALL)]
                .spacing(SP_SM - 1.0)
                .align_y(Alignment::Center),
        )
        .padding([2.0, SP_SM])
        .style(move |_: &Theme, status: button::Status| button::Style {
            background: Some(Background::Color(
                if matches!(status, button::Status::Hovered) {
                    hover_bg(palette)
                } else {
                    Color::TRANSPARENT
                },
            )),
            text_color: palette.text,
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        })
        .on_press(dev(Msg::TogglePreserve)),
        "Keep messages and requests when the page navigates",
        palette,
    )
}

// ── Console ──────────────────────────────────────────────────────────────

fn empty_state<'a>(
    palette: &'static Palette,
    glyph: Icon,
    message: &'static str,
) -> Element<'a, FerriteBrowserMessage> {
    container(
        column![
            icon(glyph, 22.0, tint(palette.text_dim, 0.6)),
            text(message).size(TEXT_SMALL).color(palette.text_dim),
        ]
        .spacing(SP_SM)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

fn older_button<'a>(hidden: usize) -> Element<'a, FerriteBrowserMessage> {
    container(
        button(
            text(format!(
                "Show {} older ({} not drawn)",
                RENDER_STEP.min(hidden),
                hidden
            ))
            .size(TEXT_SMALL),
        )
        .padding([2.0, SP_MD])
        .style(link_button_style)
        .on_press(dev(Msg::ShowOlder)),
    )
    .padding([SP_XS, EDGE])
    .into()
}

fn console_list<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let ui = &state.devtools;
    let Some(diag) = state.tab_diag.get(state.active_tab) else {
        return empty_state(palette, Icon::Console, "No page");
    };
    let view = diag
        .log
        .console_view(ui.level, &ui.search, ui.console_limit);
    if view.rows.is_empty() {
        return empty_state(
            palette,
            Icon::Console,
            if diag.log.console_len() == 0 {
                "Console is clear. Console messages from the page appear here."
            } else {
                "No messages match the filter."
            },
        );
    }
    let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::with_capacity(view.rows.len() + 1);
    if view.hidden > 0 {
        rows.push(older_button(view.hidden));
    }
    for r in view.rows {
        let expanded = ui.expanded.contains(&(Section::Console, r.id));
        rows.push(console_row(palette, r, expanded));
    }
    scrollable(column(rows).width(Length::Fill))
        .id(devtools::console_scroll_id())
        .on_scroll(|v| {
            dev(Msg::Scrolled {
                pinned: devtools::is_pinned(v.relative_offset().y),
            })
        })
        .height(Length::Fill)
        .into()
}

fn console_row<'a>(
    palette: &'static Palette,
    r: &ConsoleRow,
    expanded: bool,
) -> Element<'a, FerriteBrowserMessage> {
    if r.kind == RowKind::Navigation {
        return container(
            text(format!("Navigated to {}", crate::truncate(&r.message, 160)))
                .size(TEXT_CAPTION)
                .color(palette.text_dim),
        )
        .width(Length::Fill)
        .padding([SP_XS, EDGE])
        .style(move |_: &Theme| container::Style {
            border: Border {
                width: 1.0,
                color: palette.divider,
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into();
    }

    let glyph = match r.kind {
        RowKind::Input => Some((Icon::ChevronRight, palette.accent_bright)),
        RowKind::Result => Some((Icon::Back, palette.text_dim)),
        _ => level_icon(palette, r.level),
    };
    let body_color = match (r.kind, r.level) {
        (RowKind::Input, _) => palette.accent_bright,
        (_, ConsoleLevel::Error) => palette.danger,
        (_, ConsoleLevel::Warn) => palette.warn,
        (_, ConsoleLevel::Debug) => palette.text_dim,
        _ => palette.text,
    };
    let long = r.kind == RowKind::Message && is_long(&r.message);
    let shown = if long && !expanded {
        fold(&r.message)
    } else {
        r.message.clone()
    };
    let mut message: Vec<Element<FerriteBrowserMessage>> = vec![text(shown)
        .size(TEXT_SMALL)
        .font(mono())
        .color(body_color)
        .wrapping(text::Wrapping::WordOrGlyph)
        .width(Length::Fill)
        .into()];
    if long {
        message.push(
            button(
                row![
                    icon(
                        if expanded {
                            Icon::ChevronUp
                        } else {
                            Icon::ChevronDown
                        },
                        10.0,
                        palette.accent
                    ),
                    text(if expanded { "Show less" } else { "Show more" }).size(TEXT_CAPTION),
                ]
                .spacing(SP_XS)
                .align_y(Alignment::Center),
            )
            .padding([0.0, SP_XS])
            .style(link_button_style)
            .on_press(dev(Msg::Toggle(Section::Console, r.id)))
            .into(),
        );
    }

    let mut cells: Vec<Element<FerriteBrowserMessage>> = vec![
        text(clock_ms(r.first_ms))
            .size(TEXT_CAPTION)
            .font(mono())
            .color(palette.text_dim)
            .width(Length::Fixed(TIME_W))
            .into(),
        container(match glyph {
            Some((glyph, tone)) => icon(glyph, 13.0, tone),
            None => Space::with_width(Length::Fixed(13.0)).into(),
        })
        .width(Length::Fixed(16.0))
        .center_x(Length::Fixed(16.0))
        .padding([2.0, 0.0])
        .into(),
    ];
    if r.count > 1 {
        cells.push(pill(
            r.count.to_string(),
            mix(palette.text_dim, palette.surface, 0.55),
        ));
    }
    cells.push(column(message).spacing(1).width(Length::Fill).into());
    if let Some(source) = &r.source {
        cells.push(tip(
            button(
                text(shorten_source(source))
                    .size(TEXT_CAPTION)
                    .font(mono())
                    .wrapping(text::Wrapping::None),
            )
            .padding([0.0, SP_XS])
            .style(move |_: &Theme, status: button::Status| button::Style {
                background: None,
                text_color: if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    palette.accent_bright
                } else {
                    palette.text_dim
                },
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
            .on_press(dev(Msg::CopyText(source.clone()))),
            "Copy location",
            palette,
        ));
    }

    let wash = match r.level {
        ConsoleLevel::Error if r.kind == RowKind::Message => Some(tint(palette.danger, 0.08)),
        _ => None,
    };
    container(
        row(cells)
            .spacing(SP_SM)
            .align_y(Alignment::Start)
            .padding([2.0, EDGE]),
    )
    .width(Length::Fill)
    .style(move |_: &Theme| container::Style {
        background: wash.map(Background::Color),
        border: Border {
            width: 0.0,
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

fn prompt<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let field = text_input("Run JavaScript on this page", &state.devtools.input)
        .id(text_input::Id::new(JS_INPUT_ID))
        .width(Length::Fill)
        .padding([5.0, SP_SM])
        .size(TEXT_BODY)
        .font(mono())
        .style(|theme: &Theme, status| field_style(theme, status, 6.0))
        .on_input(FerriteBrowserMessage::JsInputChanged)
        .on_submit(FerriteBrowserMessage::JsExecuteRequested);
    // The prompt needs to know when it has the keyboard: ↑ and ↓ recall earlier
    // input only then, and reach the page otherwise.
    let field = PressProbe::new(
        field,
        dev(Msg::InputFocus(true)),
        Some(dev(Msg::InputFocus(false))),
    );
    container(
        row![
            icon(Icon::ChevronRight, 14.0, palette.accent_bright),
            field,
            button(text("Run").size(TEXT_SMALL))
                .padding([SP_XS + 1.0, SP_MD])
                .style(accent_btn_style)
                .on_press(FerriteBrowserMessage::JsExecuteRequested),
        ]
        .spacing(SP_SM)
        .align_y(Alignment::Center)
        .padding([SP_XS + 1.0, EDGE]),
    )
    .width(Length::Fill)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(palette.base)),
        border: Border {
            width: 0.0,
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

// ── Network ──────────────────────────────────────────────────────────────

fn network_list<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let ui = &state.devtools;
    let banner = container(
        row![
            icon(Icon::Info, 13.0, palette.accent_bright),
            text("Each request is listed as it starts. Size and time come from the page's own Resource Timing once it finishes; the status code is not reported by the engine yet.")
                .size(TEXT_SMALL)
                .color(palette.text_dim)
                .wrapping(text::Wrapping::Word),
        ]
        .spacing(SP_SM)
        .align_y(Alignment::Center)
        .padding([SP_XS, EDGE]),
    )
    .width(Length::Fill)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(tint(palette.accent, 0.10))),
        ..container::Style::default()
    });

    let Some(diag) = state.tab_diag.get(state.active_tab) else {
        return empty_state(palette, Icon::Network, "No page");
    };
    let view = diag
        .log
        .net_view(ui.net_filter, &ui.net_search, ui.net_limit);

    let head = container(
        row![
            text("TIME")
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .width(Length::Fixed(NET_TIME_W)),
            text("METHOD")
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .width(Length::Fixed(56.0)),
            text("TYPE")
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .width(Length::Fixed(50.0)),
            text("SIZE")
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .width(Length::Fixed(NET_SIZE_W)),
            text("TIME")
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .width(Length::Fixed(NET_DURATION_W)),
            text("URL")
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .width(Length::Fill),
        ]
        .spacing(SP_SM)
        .padding([2.0, EDGE]),
    )
    .width(Length::Fill)
    .style(raised_bar_style);

    let body: Element<FerriteBrowserMessage> = if view.rows.is_empty() {
        empty_state(
            palette,
            Icon::Network,
            if diag.log.net_len() == 0 {
                "No requests yet. Load a page and they appear here."
            } else {
                "No requests match the filter."
            },
        )
    } else {
        let mut rows: Vec<Element<FerriteBrowserMessage>> = Vec::with_capacity(view.rows.len() + 1);
        if view.hidden > 0 {
            rows.push(older_button(view.hidden));
        }
        for r in view.rows {
            let expanded = ui.expanded.contains(&(Section::Network, r.id));
            rows.push(net_row(palette, r, diag.log.nav_ms, expanded));
        }
        scrollable(column(rows).width(Length::Fill))
            .id(devtools::net_scroll_id())
            .height(Length::Fill)
            .into()
    };
    column![banner, head, body].into()
}

fn net_row<'a>(
    palette: &'static Palette,
    r: &NetRow,
    nav_ms: Option<u64>,
    expanded: bool,
) -> Element<'a, FerriteBrowserMessage> {
    let tone = kind_color(palette, r.kind);
    let url_text: Element<FerriteBrowserMessage> = if expanded {
        text(r.url.clone())
            .size(TEXT_SMALL)
            .font(mono())
            .color(palette.text)
            .wrapping(text::Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into()
    } else {
        text(crate::truncate(&r.url, URL_CHARS))
            .size(TEXT_SMALL)
            .font(mono())
            .color(if r.main_frame {
                palette.text
            } else {
                palette.text_dim
            })
            .wrapping(text::Wrapping::None)
            .width(Length::Fill)
            .into()
    };
    let mut url_cell: Vec<Element<FerriteBrowserMessage>> = vec![url_text];
    if expanded {
        url_cell.push(
            button(
                row![
                    icon(Icon::Copy, 11.0, palette.accent),
                    text("Copy URL").size(TEXT_CAPTION)
                ]
                .spacing(SP_XS + 1.0)
                .align_y(Alignment::Center),
            )
            .padding([0.0, SP_XS])
            .style(link_button_style)
            .on_press(dev(Msg::CopyText(r.url.clone())))
            .into(),
        );
    }
    let line = row![
        text(devtools::net_time(r.at_ms, nav_ms))
            .size(TEXT_CAPTION)
            .font(mono())
            .color(palette.text_dim)
            .width(Length::Fixed(NET_TIME_W)),
        text(r.method.clone())
            .size(TEXT_CAPTION)
            .font(mono())
            .color(palette.text)
            .width(Length::Fixed(56.0)),
        container(text(r.kind.label()).size(TEXT_CAPTION).color(tone))
            .padding([0.0, 5.0])
            .width(Length::Fixed(50.0))
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(tint(tone, 0.14))),
                border: Border {
                    radius: 4.0.into(),
                    width: 1.0,
                    color: tint(tone, 0.35),
                },
                ..container::Style::default()
            }),
        text(devtools::net_size(r))
            .size(TEXT_CAPTION)
            .font(mono())
            .color(palette.text_dim)
            .width(Length::Fixed(NET_SIZE_W)),
        text(devtools::net_duration(r))
            .size(TEXT_CAPTION)
            .font(mono())
            .color(palette.text_dim)
            .width(Length::Fixed(NET_DURATION_W)),
        column(url_cell).spacing(1).width(Length::Fill),
    ]
    .spacing(SP_SM)
    .align_y(Alignment::Start)
    .padding([2.0, EDGE]);
    mouse_area(
        container(line)
            .width(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: expanded.then(|| Background::Color(hover_bg(palette))),
                ..container::Style::default()
            }),
    )
    .on_press(dev(Msg::Toggle(Section::Network, r.id)))
    .into()
}

// ── Engine ───────────────────────────────────────────────────────────────

fn engine_list<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let events = state.engine_log.events();
    if events.is_empty() {
        return empty_state(
            palette,
            Icon::Shield,
            "No engine panics or page crashes this session.",
        );
    }
    let rows: Vec<Element<FerriteBrowserMessage>> = events
        .iter()
        .rev()
        .flat_map(|e| {
            let expanded = state.devtools.expanded.contains(&(Section::Engine, e.id));
            [
                engine_row(palette, e, expanded),
                container(Space::new(Length::Fill, Length::Fixed(1.0)))
                    .width(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(Background::Color(palette.divider)),
                        ..container::Style::default()
                    })
                    .into(),
            ]
        })
        .collect();
    scrollable(column(rows).width(Length::Fill))
        .height(Length::Fill)
        .into()
}

fn engine_row<'a>(
    palette: &'static Palette,
    e: &EngineEvent,
    expanded: bool,
) -> Element<'a, FerriteBrowserMessage> {
    let (label, tone) = match e.kind {
        EngineKind::Panic => ("PANIC", palette.danger),
        EngineKind::Crash => ("CRASH", palette.danger),
    };
    let mut head: Vec<Element<FerriteBrowserMessage>> = vec![
        text(clock_ms(e.at_ms))
            .size(TEXT_CAPTION)
            .font(mono())
            .color(palette.text_dim)
            .into(),
        pill(label.to_string(), tone),
        text(e.thread.clone())
            .size(TEXT_SMALL)
            .font(font_weight(iced::font::Weight::Semibold))
            .color(palette.text)
            .into(),
    ];
    if !e.tab.is_empty() {
        head.push(
            text(e.tab.clone())
                .size(TEXT_CAPTION)
                .color(palette.text_dim)
                .into(),
        );
    }
    head.push(Space::with_width(Length::Fill).into());
    head.push(
        button(
            row![
                icon(Icon::Copy, 11.0, palette.accent),
                text("Copy").size(TEXT_CAPTION)
            ]
            .spacing(SP_XS + 1.0)
            .align_y(Alignment::Center),
        )
        .padding([0.0, SP_XS])
        .style(link_button_style)
        .on_press(dev(Msg::CopyText(devtools::engine_text(e))))
        .into(),
    );
    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![
        row(head).spacing(SP_SM).align_y(Alignment::Center).into(),
        text(e.message.clone())
            .size(TEXT_SMALL)
            .font(mono())
            .color(palette.text)
            .wrapping(text::Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into(),
    ];
    if !e.location.is_empty() {
        body.push(
            text(format!("at {}", e.location))
                .size(TEXT_CAPTION)
                .font(mono())
                .color(palette.text_dim)
                .wrapping(text::Wrapping::WordOrGlyph)
                .into(),
        );
    }
    if let Some(backtrace) = &e.backtrace {
        body.push(
            button(
                row![
                    icon(
                        if expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        10.0,
                        palette.accent
                    ),
                    text(if expanded {
                        "Hide backtrace"
                    } else {
                        "Show backtrace"
                    })
                    .size(TEXT_CAPTION),
                ]
                .spacing(SP_XS)
                .align_y(Alignment::Center),
            )
            .padding([0.0, SP_XS])
            .style(link_button_style)
            .on_press(dev(Msg::Toggle(Section::Engine, e.id)))
            .into(),
        );
        if expanded {
            body.push(
                container(
                    text(crate::truncate(backtrace, 8_000))
                        .size(TEXT_CAPTION)
                        .font(mono())
                        .color(palette.text_dim)
                        .wrapping(text::Wrapping::WordOrGlyph)
                        .width(Length::Fill),
                )
                .padding(SP_SM)
                .width(Length::Fill)
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(palette.input)),
                    border: Border {
                        radius: 6.0.into(),
                        width: 1.0,
                        color: palette.divider,
                    },
                    ..container::Style::default()
                })
                .into(),
            );
        }
    }
    container(column(body).spacing(SP_XS))
        .width(Length::Fill)
        .padding([SP_SM, EDGE])
        .style(move |_: &Theme| container::Style {
            border: Border {
                width: 0.0,
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_messages_are_not_folded_and_long_ones_are() {
        assert!(!is_long("one line"));
        assert!(!is_long("a\nb\nc\nd"));
        assert!(is_long("a\nb\nc\nd\ne"));
        assert!(is_long(&"x".repeat(FOLD_CHARS + 1)));
        assert!(!is_long(&"x".repeat(FOLD_CHARS)));
    }

    #[test]
    fn folding_keeps_the_first_lines_and_marks_the_cut() {
        assert_eq!(fold("a\nb\nc\nd\ne\nf"), "a\nb\nc\nd\u{2026}");
        let long = "y".repeat(1_000);
        let cut = fold(&long);
        assert_eq!(cut.chars().count(), FOLD_CHARS + 1);
        assert!(cut.ends_with('\u{2026}'));
        // Lines are kept as lines.
        assert_eq!(fold("first\nsecond"), "first\nsecond");
    }

    #[test]
    fn a_long_location_keeps_its_file_name_and_position() {
        let src = "https://cdn.example.com/assets/very/long/path/to/bundle.min.js:1234:56";
        let short = shorten_source(src);
        assert!(short.chars().count() <= SOURCE_CHARS);
        assert!(short.starts_with('\u{2026}'));
        assert!(short.ends_with("bundle.min.js:1234:56"));
        assert_eq!(shorten_source("a.js:1:2"), "a.js:1:2");
    }
}
