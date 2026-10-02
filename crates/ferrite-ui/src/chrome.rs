// The browser chrome above the page: the tab strip, the toolbar with its
// address bar, the loading bar and the overflow menu.
//
// Layout, top to bottom, with no wasted rows:
//
//   tab strip  34 px  [traffic-light inset][tab][tab]...[+][drag area]
//   toolbar    40 px  [<][>][reload][ address bar          ][Agent][shield][menu]
//   hairline    1 px
//   page
//
// On macOS the window uses a transparent, full-size-content title bar (see
// `launch`), so the native title row is gone and the traffic lights sit inside
// the tab strip's left inset; the strip doubles as the window's drag area. On
// Linux and Windows the window keeps its normal decorations and the inset is 0.
//
// The active tab, the toolbar and the page share one colour (`Palette::base`)
// and the strip behind them is a step darker (`Palette::chrome`), which is
// what makes the active tab read as part of the toolbar.
//
// The pure sizing and text-fitting rules (`tab_width`, `fit_title`,
// `tab_label`, `progress_band`) are separate functions so they are testable
// without a window.

use std::time::{Duration, Instant};

use iced::widget::{
    button, column, container, horizontal_space, mouse_area, row, scrollable, stack, text,
    text_input,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Size, Theme};
use iced_widget::image::Image as ServoImage;
use iced_widget::responsive;

use crate::icons::{icon, Icon};
use crate::tokens::{
    bare_field_style, close_btn_style, hover_bg, menu_row_style, popover_style, tab_bar_style,
    tint, tip, toolbar_btn_on_style, toolbar_btn_style, SP_MD, SP_SM, SP_XS, TEXT_BODY,
    TEXT_CAPTION, TEXT_SMALL,
};
use crate::widgets::PressProbe;
use crate::{
    font_weight, pulse_alpha, AppTheme, FerriteBrowser, FerriteBrowserMessage as Msg, LibraryTab,
    Palette, ADDRESS_BAR_ID, MOD_LABEL,
};

// ── Geometry ─────────────────────────────────────────────────────────────

/// Height of the tab strip.
pub(crate) const TAB_STRIP_HEIGHT: f32 = 34.0;
/// Height of the toolbar row.
pub(crate) const TOOLBAR_HEIGHT: f32 = 40.0;
/// Height of the loading bar drawn over the toolbar's bottom edge.
const PROGRESS_HEIGHT: f32 = 2.0;
/// The widest and narrowest a tab gets: tabs share the strip equally between
/// these, so they shrink as more open and the strip scrolls only past that.
pub(crate) const TAB_MAX_WIDTH: f32 = 220.0;
pub(crate) const TAB_MIN_WIDTH: f32 = 40.0;
/// The active tab never gets narrower than this (room for icon, a few
/// characters of title and close), however many tabs are open.
pub(crate) const ACTIVE_TAB_MIN_WIDTH: f32 = 100.0;
/// A tab shows its close button on hover only from this width up.
const TAB_HOVER_CLOSE_MIN_WIDTH: f32 = 64.0;
/// Below this width a tab shows only its icon (and, if active or hovered, close).
const TAB_TITLE_MIN_WIDTH: f32 = 88.0;
/// Height of a tab, flush with the strip's bottom edge.
const TAB_HEIGHT: f32 = TAB_STRIP_HEIGHT - 4.0;
/// The favicon slot and close button are this wide.
const TAB_ICON_SLOT: f32 = 16.0;
/// Inner horizontal padding of a tab, and the gap between its parts.
const TAB_PAD: f32 = 10.0;
const TAB_GAP: f32 = 6.0;
/// Gap between tabs.
const TAB_SPACING: f32 = 2.0;
/// The new-tab button.
const NEW_TAB_BTN: f32 = 26.0;
/// Room kept free at the strip's right end so there is always something to
/// grab to drag the window.
const DRAG_GUTTER: f32 = 40.0;
/// Room for the macOS traffic lights; 0 where the window keeps its native bar.
#[cfg(target_os = "macos")]
pub(crate) const TRAFFIC_LIGHT_INSET: f32 = 76.0;
#[cfg(not(target_os = "macos"))]
pub(crate) const TRAFFIC_LIGHT_INSET: f32 = 0.0;

/// Toolbar icon buttons are this square.
const BTN: f32 = 28.0;
const BTN_ICON: f32 = 15.0;
/// The address bar's height.
const OMNIBOX_HEIGHT: f32 = 30.0;
/// Width of the overflow menu.
const MENU_WIDTH: f32 = 264.0;

/// Two presses on the title area this close together are a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

// ── Pure rules ───────────────────────────────────────────────────────────

/// The width tabs get when `count` of them share `available` pixels:
/// `(other tabs, active tab)`. Equal shares, never wider than
/// [`TAB_MAX_WIDTH`], never narrower than [`TAB_MIN_WIDTH`] (past which the
/// strip scrolls instead). Once the shares get narrower than
/// [`ACTIVE_TAB_MIN_WIDTH`], the active tab keeps that much and the others
/// split the rest, so the tab you are on always stays readable.
pub(crate) fn tab_widths(count: usize, available: f32) -> (f32, f32) {
    if count == 0 {
        return (TAB_MAX_WIDTH, TAB_MAX_WIDTH);
    }
    let gaps = TAB_SPACING * (count as f32 - 1.0);
    let even = ((available - gaps) / count as f32).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
    if count == 1 || even >= ACTIVE_TAB_MIN_WIDTH {
        return (even, even);
    }
    let others = ((available - gaps - ACTIVE_TAB_MIN_WIDTH) / (count as f32 - 1.0))
        .clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
    (others, ACTIVE_TAB_MIN_WIDTH)
}

/// Whether `count` tabs at their narrowest still do not fit `available`.
pub(crate) fn tabs_overflow(count: usize, available: f32) -> bool {
    if count == 0 {
        return false;
    }
    let gaps = TAB_SPACING * (count as f32 - 1.0);
    let narrowest = TAB_MIN_WIDTH * (count as f32 - 1.0) + ACTIVE_TAB_MIN_WIDTH.min(TAB_MAX_WIDTH);
    narrowest + gaps > available
}

/// What a tab of `width` shows besides its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TabParts {
    pub(crate) title: bool,
    pub(crate) close: bool,
}

pub(crate) fn tab_parts(width: f32, active: bool, hovered: bool, can_close: bool) -> TabParts {
    TabParts {
        title: width >= TAB_TITLE_MIN_WIDTH,
        // Always reachable on the active tab; on others only under the
        // pointer (and only if there is room), so a row of tabs is not a row
        // of crosses.
        close: can_close && (active || (hovered && width >= TAB_HOVER_CLOSE_MIN_WIDTH)),
    }
}

/// The pixels a tab's title may use at `width`.
pub(crate) fn tab_title_budget(width: f32, parts: TabParts) -> f32 {
    let mut used = TAB_PAD * 2.0 + TAB_ICON_SLOT;
    if parts.title {
        used += TAB_GAP;
    }
    if parts.close {
        used += TAB_GAP + TAB_ICON_SLOT;
    }
    (width - used).max(0.0)
}

/// A close estimate of the rendered width of `s` at `size` px in Inter: a
/// per-character class table, not a measurement. It only has to decide where
/// to cut a title, so being a few percent off costs a character, not layout.
pub(crate) fn text_width(s: &str, size: f32) -> f32 {
    s.chars().map(|c| char_em(c) * size).sum()
}

fn char_em(c: char) -> f32 {
    match c {
        ' ' => 0.28,
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' => 0.27,
        'f' | 't' | 'r' | 'I' | '(' | ')' | '[' | ']' | '/' | '-' => 0.36,
        'm' | 'w' | 'M' | 'W' | '@' | '%' => 0.88,
        '0'..='9' => 0.60,
        'A'..='Z' => 0.67,
        'a'..='z' => 0.56,
        // CJK and other full-width scripts.
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.60,
    }
}

/// `title` cut to fit `max_px` at `size`, ending in an ellipsis when cut.
/// Always one line: a tab title never wraps.
pub(crate) fn fit_title(title: &str, max_px: f32, size: f32) -> String {
    let title = title.trim();
    if text_width(title, size) <= max_px {
        return title.to_string();
    }
    let ellipsis = char_em('\u{2026}').max(0.9) * size;
    let mut out = String::new();
    let mut used = ellipsis;
    for c in title.chars() {
        let w = char_em(c) * size;
        if used + w > max_px {
            break;
        }
        used += w;
        out.push(c);
    }
    let out = out.trim_end();
    if out.is_empty() {
        // Not even one character fits: show nothing rather than a bare dot.
        String::new()
    } else {
        format!("{out}\u{2026}")
    }
}

/// The title a tab shows: the page's own, else its host, else "New Tab".
pub(crate) fn tab_label(title: &str, url: &str, loading: bool) -> String {
    let title = title.trim();
    let blank = url.is_empty() || url == "about:blank";
    if blank {
        return "New Tab".to_string();
    }
    if !title.is_empty() && title != "New Tab" {
        return title.to_string();
    }
    match url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    {
        Some(host) => host,
        None if loading => "Loading\u{2026}".to_string(),
        None => url.to_string(),
    }
}

/// The window title: the page's title and the app name, as every browser shows.
pub(crate) fn window_title(title: &str, url: &str) -> String {
    let blank = url.is_empty() || url == "about:blank";
    let shown = tab_label(title, url, false);
    if blank {
        "Ferrite".to_string()
    } else {
        format!("{shown} \u{2014} Ferrite")
    }
}

/// The three `FillPortion`s (left gap, band, right gap, out of 1000) of the
/// indeterminate loading bar at animation phase `t` in `0.0..=1.0`: a band
/// 30 % wide that enters from the left edge, crosses and leaves at the right,
/// eased so it lingers neither at the edges nor in the middle.
pub(crate) fn progress_band(t: f32) -> (u16, u16, u16) {
    const BAND: f32 = 300.0;
    const TOTAL: f32 = 1000.0;
    let t = t.clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t);
    let start = -BAND + eased * (TOTAL + BAND);
    let left = start.max(0.0);
    let right = (start + BAND).min(TOTAL);
    let band = (right - left).max(0.0);
    (
        left.round() as u16,
        band.round() as u16,
        (TOTAL - right.max(left)).round() as u16,
    )
}

/// Whether a press at `now` after one at `previous` is a double-click.
pub(crate) fn is_double_click(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_some_and(|p| now.saturating_duration_since(p) <= DOUBLE_CLICK)
}

// ── Overflow menu commands ───────────────────────────────────────────────

/// What a row of the overflow menu does. Picking one closes the menu and then
/// sends [`MenuCommand::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    NewTab,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Find,
    BookmarkPage,
    Library(LibraryTab),
    JsConsole,
    ToggleTheme,
    Settings,
}

impl MenuCommand {
    pub(crate) fn message(self) -> Msg {
        match self {
            MenuCommand::NewTab => Msg::AddTab,
            MenuCommand::ZoomIn => Msg::ZoomIn,
            MenuCommand::ZoomOut => Msg::ZoomOut,
            MenuCommand::ZoomReset => Msg::ZoomReset,
            MenuCommand::Find => Msg::OpenFindBar,
            MenuCommand::BookmarkPage => Msg::ToggleBookmarkCurrentPage,
            MenuCommand::Library(tab) => Msg::OpenLibrary(tab),
            MenuCommand::JsConsole => Msg::ToggleJsConsole,
            MenuCommand::ToggleTheme => Msg::ToggleTheme,
            MenuCommand::Settings => Msg::ToggleSettingsPanel,
        }
    }
}

// ── Tab strip ────────────────────────────────────────────────────────────

/// The tab strip: tabs sharing the width, the new-tab button, and the rest of
/// the row as the window's drag area.
pub(crate) fn tab_strip(state: &FerriteBrowser) -> Element<'_, Msg> {
    container(responsive(move |size: Size| tab_row(state, size.width)))
        .width(Length::Fill)
        .height(Length::Fixed(TAB_STRIP_HEIGHT))
        .style(tab_bar_style)
        .into()
}

fn tab_row(state: &FerriteBrowser, width: f32) -> Element<'_, Msg> {
    let palette = state.palette();
    let count = state.tabs.len();
    let side_pad = SP_SM;
    let available =
        width - TRAFFIC_LIGHT_INSET - side_pad * 2.0 - NEW_TAB_BTN - TAB_SPACING - DRAG_GUTTER;
    let (other_w, active_w) = tab_widths(count, available);
    let can_close = count > 1;

    let tabs: Vec<Element<Msg>> = (0..count)
        .map(|i| {
            let w = if i == state.active_tab {
                active_w
            } else {
                other_w
            };
            tab_view(state, i, w, can_close)
        })
        .collect();
    let tabs = row(tabs)
        .spacing(TAB_SPACING)
        .height(Length::Fixed(TAB_HEIGHT));
    let tabs: Element<Msg> = if tabs_overflow(count, available) {
        scrollable(tabs)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new()
                    .width(0)
                    .scroller_width(0)
                    .margin(0),
            ))
            .width(Length::Fill)
            .into()
    } else {
        tabs.into()
    };

    let new_tab = container(tip(
        button(container(icon(Icon::Add, BTN_ICON - 2.0, palette.text_dim)).center(Length::Fill))
            .width(Length::Fixed(NEW_TAB_BTN))
            .height(Length::Fixed(NEW_TAB_BTN))
            .padding(0)
            .style(toolbar_btn_style)
            .on_press(Msg::AddTab),
        format!("New tab ({MOD_LABEL}+T)"),
        palette,
    ))
    .height(Length::Fixed(TAB_HEIGHT))
    .align_y(Alignment::Center)
    .padding(Padding {
        left: TAB_SPACING + SP_XS,
        ..Padding::ZERO
    });

    // The strip's empty parts drag the window; a double-click maximizes it.
    mouse_area(
        row![
            container(horizontal_space())
                .width(Length::Fixed(TRAFFIC_LIGHT_INSET + side_pad))
                .height(Length::Fill),
            tabs,
            new_tab,
            container(horizontal_space())
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .align_y(Alignment::End)
        .height(Length::Fill)
        .padding(Padding {
            right: side_pad,
            ..Padding::ZERO
        }),
    )
    .on_press(Msg::TitleBarPressed)
    .into()
}

/// The favicon slot: the page's icon, a pulsing dot while it loads and has
/// none, and a globe otherwise.
fn tab_icon(state: &FerriteBrowser, i: usize, active: bool) -> Element<'_, Msg> {
    let palette = state.palette();
    let light = state.theme_mode == AppTheme::Light;
    let loading = state.is_loading && i == state.active_tab;
    let slot =
        |inner: Element<'static, Msg>| container(inner).center(Length::Fixed(TAB_ICON_SLOT)).into();
    if let Some(handle) = state.tab_favicons.get(i).and_then(|f| f.as_ref()) {
        // On the dark theme a light backing keeps dark icons visible.
        return container(
            ServoImage::new(handle.clone())
                .width(Length::Fixed(TAB_ICON_SLOT - 2.0))
                .height(Length::Fixed(TAB_ICON_SLOT - 2.0)),
        )
        .center(Length::Fixed(TAB_ICON_SLOT))
        .style(move |_: &Theme| container::Style {
            background: (!light).then_some(Background::Color(tint(Color::WHITE, 0.92))),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into();
    }
    if loading {
        let a = pulse_alpha(state.progress_offset, 1.2, 0.35, 0.65);
        return slot(
            text("\u{2022}")
                .size(TEXT_SMALL)
                .color(tint(palette.accent, a))
                .into(),
        );
    }
    let color = if active {
        palette.text
    } else {
        palette.text_dim
    };
    slot(icon(Icon::Globe, TAB_ICON_SLOT - 3.0, tint(color, 0.8)))
}

fn tab_view(state: &FerriteBrowser, i: usize, width: f32, can_close: bool) -> Element<'_, Msg> {
    let palette = state.palette();
    let active = i == state.active_tab;
    let hovered = state.hovered_tab == Some(i);
    let parts = tab_parts(width, active, hovered, can_close);

    let url = state.tab_urls.get(i).map_or("", String::as_str);
    let title = state.tab_titles.get(i).map_or("", String::as_str);
    let loading = state.is_loading && active;
    let label = fit_title(
        &tab_label(title, url, loading),
        tab_title_budget(width, parts),
        TEXT_SMALL,
    );

    let mut items: Vec<Element<Msg>> = vec![tab_icon(state, i, active)];
    if parts.title {
        items.push(
            text(label)
                .size(TEXT_SMALL)
                .wrapping(text::Wrapping::None)
                .width(Length::Fill)
                .color(if active {
                    palette.text
                } else {
                    palette.text_dim
                })
                .into(),
        );
    } else {
        items.push(horizontal_space().into());
    }
    if parts.close {
        items.push(
            button(container(icon(Icon::Close, 9.0, palette.text_dim)).center(Length::Fill))
                .width(Length::Fixed(TAB_ICON_SLOT))
                .height(Length::Fixed(TAB_ICON_SLOT))
                .padding(0)
                .style(close_btn_style)
                .on_press(Msg::CloseTab(i))
                .into(),
        );
    }

    let body = container(
        row(items)
            .spacing(TAB_GAP)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding([0.0, TAB_PAD])
    .align_y(Alignment::Center)
    .clip(true)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(if active {
            palette.base
        } else if hovered {
            hover_bg(palette)
        } else {
            Color::TRANSPARENT
        })),
        border: Border {
            radius: if active {
                iced::border::Radius {
                    top_left: 8.0,
                    top_right: 8.0,
                    bottom_left: 0.0,
                    bottom_right: 0.0,
                }
            } else {
                8.0.into()
            },
            ..Border::default()
        },
        ..container::Style::default()
    });

    // Inactive tabs float 4 px off the strip's bottom edge (their hover wash
    // is a rounded chip); the active one runs flush into the toolbar below.
    let outer = container(body)
        .width(Length::Fixed(width))
        .height(Length::Fixed(TAB_HEIGHT))
        .padding(Padding {
            top: if active { 0.0 } else { 2.0 },
            bottom: if active { 0.0 } else { 4.0 },
            ..Padding::ZERO
        });

    mouse_area(outer)
        .on_press(Msg::SelectTab(i))
        .on_middle_press(Msg::CloseTab(i))
        .on_enter(Msg::TabHoverEnter(i))
        .on_exit(Msg::TabHoverExit(i))
        .into()
}

// ── Toolbar ──────────────────────────────────────────────────────────────

/// A square icon-only button with a tooltip.
fn icon_btn<'a>(
    kind: Icon,
    color: Color,
    label: impl Into<String>,
    on_press: Option<Msg>,
    palette: &'static Palette,
) -> Element<'a, Msg> {
    tip(
        button(container(icon(kind, BTN_ICON, color)).center(Length::Fill))
            .width(Length::Fixed(BTN))
            .height(Length::Fixed(BTN))
            .padding(0)
            .style(toolbar_btn_style)
            .on_press_maybe(on_press),
        label,
        palette,
    )
}

/// The toolbar: navigation, address bar, and the right-hand cluster, with the
/// loading bar laid over its bottom edge so a load never moves the page.
pub(crate) fn toolbar(state: &FerriteBrowser) -> Element<'_, Msg> {
    let palette = state.palette();

    // ── Navigation ──
    let dim = |enabled: bool| {
        if enabled {
            palette.text
        } else {
            tint(palette.text_dim, 0.3)
        }
    };
    let back = icon_btn(
        Icon::Back,
        dim(state.can_go_back),
        "Back (Alt+\u{2190})",
        state.can_go_back.then_some(Msg::GoBack),
        palette,
    );
    let forward = icon_btn(
        Icon::Forward,
        dim(state.can_go_forward),
        "Forward (Alt+\u{2192})",
        state.can_go_forward.then_some(Msg::GoForward),
        palette,
    );
    let reload = if state.is_loading {
        icon_btn(
            Icon::Close,
            palette.text,
            "Stop loading (Esc)",
            Some(Msg::StopLoading),
            palette,
        )
    } else {
        icon_btn(
            Icon::Reload,
            palette.text,
            format!("Reload ({MOD_LABEL}+R)"),
            Some(Msg::Reload),
            palette,
        )
    };

    // ── Right cluster ──
    let agent = agent_button(state);
    let audit = tip(
        button(
            container(icon(
                Icon::Shield,
                BTN_ICON,
                if state.show_audit_panel {
                    palette.accent_bright
                } else {
                    palette.text_dim
                },
            ))
            .center(Length::Fill),
        )
        .width(Length::Fixed(BTN))
        .height(Length::Fixed(BTN))
        .padding(0)
        .style(if state.show_audit_panel {
            toolbar_btn_on_style
        } else {
            toolbar_btn_style
        })
        .on_press(Msg::ToggleAuditPanel),
        "Audit log (F12)",
        palette,
    );
    let menu_on = state.show_menu;
    let menu_btn = button(
        container(icon(
            Icon::Dots,
            BTN_ICON,
            if menu_on {
                palette.accent_bright
            } else {
                palette.text_dim
            },
        ))
        .center(Length::Fill),
    )
    .width(Length::Fixed(BTN))
    .height(Length::Fixed(BTN))
    .padding(0)
    .style(if menu_on {
        toolbar_btn_on_style
    } else {
        toolbar_btn_style
    })
    .on_press(Msg::ToggleMenu);
    // No tooltip while the menu is open: it would float over the menu itself.
    let menu: Element<Msg> = if menu_on {
        menu_btn.into()
    } else {
        tip(menu_btn, "Menu", palette)
    };

    let bar = row![
        back,
        forward,
        reload,
        container(omnibox(state))
            .width(Length::Fill)
            .padding([0.0, SP_XS]),
        agent,
        audit,
        menu,
    ]
    .spacing(2)
    .align_y(Alignment::Center)
    .padding([0.0, SP_SM]);

    let body = container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(TOOLBAR_HEIGHT))
        .align_y(Alignment::Center)
        .style(crate::tokens::toolbar_style);

    if !state.is_loading {
        return body.into();
    }
    stack([body.into(), loading_bar(state)]).into()
}

/// The indeterminate loading bar, a band sweeping the toolbar's bottom edge.
fn loading_bar(state: &FerriteBrowser) -> Element<'_, Msg> {
    let palette = state.palette();
    let (left, band, right) = progress_band(state.progress_offset);
    let part = |portion: u16, color: Color| {
        container(text(""))
            .width(Length::FillPortion(portion.max(1)))
            .height(Length::Fixed(PROGRESS_HEIGHT))
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(color)),
                ..container::Style::default()
            })
    };
    let bar = row![
        part(left, tint(palette.accent, 0.18)),
        part(band, palette.accent_bright),
        part(right, tint(palette.accent, 0.18)),
    ]
    .width(Length::Fill)
    .height(Length::Fixed(PROGRESS_HEIGHT));
    container(bar)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::End)
        .into()
}

/// The Agent toggle: the one labelled control in the toolbar. Tinted when the
/// sidebar is closed, solid when open, with a dot when something in it needs
/// the person (an approval, a sign-in) or the agent is working.
fn agent_button(state: &FerriteBrowser) -> Element<'_, Msg> {
    let palette = state.palette();
    let open = state.show_agent_sidebar;
    let needs_you = state.pending_diff.is_some()
        || state.pending_runtime.is_some()
        || state.signin_handoff.is_some();
    let working = state.agent_is_running;

    let fg = if open {
        Color::WHITE
    } else {
        palette.accent_bright
    };
    let mut items: Vec<Element<Msg>> = vec![
        icon(Icon::Agent, 14.0, fg),
        text("Agent")
            .size(TEXT_SMALL)
            .font(font_weight(iced::font::Weight::Medium))
            .into(),
    ];
    if needs_you || working {
        let color = if needs_you {
            palette.warn
        } else {
            tint(
                palette.safe,
                pulse_alpha(state.progress_offset, 1.5, 0.45, 0.55),
            )
        };
        items.push(
            container(text(""))
                .width(Length::Fixed(7.0))
                .height(Length::Fixed(7.0))
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(color)),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                })
                .into(),
        );
    }
    let label = if needs_you {
        format!("Agent needs your decision ({MOD_LABEL}+Shift+A)")
    } else {
        format!("Agent ({MOD_LABEL}+Shift+A)")
    };
    tip(
        button(
            container(row(items).spacing(6).align_y(Alignment::Center))
                .height(Length::Fill)
                .align_y(Alignment::Center),
        )
        .height(Length::Fixed(BTN))
        .padding([0.0, SP_MD])
        .style(move |theme: &Theme, status| {
            if open {
                crate::tokens::panel_btn_active(theme, status)
            } else {
                toolbar_btn_on_style(theme, status)
            }
        })
        .on_press(Msg::ToggleAgentSidebar),
        label,
        palette,
    )
}

/// The address bar: a pill holding the security indicator, the field, the
/// zoom chip and the bookmark star. The pill, not the field, draws the focus
/// ring, so the icons sit inside it.
fn omnibox(state: &FerriteBrowser) -> Element<'_, Msg> {
    let palette = state.palette();
    let url = state
        .tab_urls
        .get(state.active_tab)
        .map_or("about:blank", String::as_str);
    let blank = url == "about:blank";
    let focused = state.address_bar_focused;

    // ── Security indicator ──
    let secure: Element<Msg> = if blank {
        icon(Icon::Search, 13.0, palette.text_dim)
    } else if url.starts_with("https://") {
        icon(Icon::Lock, 13.0, palette.text_dim)
    } else if url.starts_with("http://") {
        row![
            icon(Icon::Warning, 12.0, palette.warn),
            text("Not secure")
                .size(TEXT_CAPTION)
                .font(font_weight(iced::font::Weight::Medium))
                .color(palette.warn),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    } else {
        icon(Icon::Globe, 13.0, palette.text_dim)
    };

    // ── Field ──
    let field = text_input(
        if blank {
            "Search or type an address"
        } else {
            ""
        },
        &state.address_bar_input,
    )
    .id(text_input::Id::new(ADDRESS_BAR_ID))
    .width(Length::Fill)
    .padding([5.0, 2.0])
    .size(TEXT_BODY)
    .style(bare_field_style)
    .on_input(Msg::AddressBarChanged)
    .on_submit(Msg::NavigateRequested(state.address_bar_input.clone()));
    // Tell the app about the press that focuses the field (select everything)
    // and, while it holds focus, about presses elsewhere (drop the ring).
    let field = PressProbe::new(
        field,
        Msg::AddressBarPressed,
        focused.then_some(Msg::ClearAddressBarFocus),
    );

    let mut items: Vec<Element<Msg>> = vec![
        container(secure)
            .align_x(Alignment::Center)
            .padding([0.0, SP_XS])
            .into(),
        field.into(),
    ];

    // ── Zoom chip, only while not at 100 % ──
    let zoom = state.tab_zoom.get(state.active_tab).copied().unwrap_or(1.0);
    if (zoom - 1.0).abs() > f32::EPSILON {
        items.push(tip(
            button(
                text(format!("{}%", (zoom * 100.0).round() as i32))
                    .size(TEXT_CAPTION)
                    .font(font_weight(iced::font::Weight::Medium)),
            )
            .padding([2.0, SP_SM])
            .style(|theme: &Theme, status| {
                let p = crate::palette_for_theme(theme);
                button::Style {
                    background: Some(Background::Color(match status {
                        button::Status::Hovered | button::Status::Pressed => tint(p.accent, 0.28),
                        _ => tint(p.accent, 0.16),
                    })),
                    text_color: p.accent_bright,
                    border: Border {
                        radius: 100.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                }
            })
            .on_press(Msg::ZoomReset),
            format!(
                "Zoom {}% \u{2014} click to reset ({MOD_LABEL}+0)",
                (zoom * 100.0).round() as i32
            ),
            palette,
        ));
    }

    // ── Bookmark star ──
    if !blank {
        let marked = state.bookmarks.iter().any(|b| b.url == url);
        items.push(tip(
            button(
                container(icon(
                    if marked {
                        Icon::BookmarkFilled
                    } else {
                        Icon::BookmarkOutline
                    },
                    13.0,
                    if marked {
                        palette.accent
                    } else {
                        palette.text_dim
                    },
                ))
                .center(Length::Fill),
            )
            .width(Length::Fixed(24.0))
            .height(Length::Fixed(24.0))
            .padding(0)
            .style(toolbar_btn_style)
            .on_press(Msg::ToggleBookmarkCurrentPage),
            if marked {
                format!("Remove bookmark ({MOD_LABEL}+D)")
            } else {
                format!("Bookmark this page ({MOD_LABEL}+D)")
            },
            palette,
        ));
    }

    container(
        row(items)
            .spacing(SP_XS)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fixed(OMNIBOX_HEIGHT))
    .padding([0.0, SP_SM])
    .align_y(Alignment::Center)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(if focused {
            palette.input
        } else {
            palette.surface
        })),
        border: Border {
            radius: 100.0.into(),
            width: if focused { 2.0 } else { 1.0 },
            color: if focused {
                palette.accent
            } else {
                tint(palette.divider, 0.7)
            },
        },
        ..container::Style::default()
    })
    .into()
}

// ── Overflow menu ────────────────────────────────────────────────────────

/// One menu row: icon, label, optional shortcut hint on the right.
fn menu_item<'a>(
    kind: Icon,
    label: impl Into<String>,
    hint: Option<String>,
    command: MenuCommand,
    palette: &'static Palette,
) -> Element<'a, Msg> {
    let mut cells: Vec<Element<Msg>> = vec![
        icon(kind, 14.0, palette.text_dim),
        text(label.into())
            .size(TEXT_BODY)
            .width(Length::Fill)
            .into(),
    ];
    if let Some(hint) = hint {
        cells.push(text(hint).size(TEXT_CAPTION).color(palette.text_dim).into());
    }
    button(row(cells).spacing(SP_MD).align_y(Alignment::Center))
        .width(Length::Fill)
        .padding([6.0, SP_MD])
        .style(menu_row_style)
        .on_press(Msg::Menu(command))
        .into()
}

fn menu_divider<'a>(palette: &'static Palette) -> Element<'a, Msg> {
    container(
        container(text(""))
            .width(Length::Fill)
            .height(Length::Fixed(1.0))
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(palette.divider)),
                ..container::Style::default()
            }),
    )
    .padding([SP_XS, SP_SM])
    .into()
}

/// The zoom row of the menu: label, minus, current level (click resets), plus.
fn zoom_row(state: &FerriteBrowser) -> Element<'_, Msg> {
    let palette = state.palette();
    let zoom = state.tab_zoom.get(state.active_tab).copied().unwrap_or(1.0);
    let step = |glyph: Icon, command: MenuCommand| {
        button(container(icon(glyph, 13.0, palette.text)).center(Length::Fill))
            .width(Length::Fixed(BTN))
            .height(Length::Fixed(BTN))
            .padding(0)
            .style(crate::tokens::panel_btn_inactive)
            .on_press(Msg::Menu(command))
    };
    row![
        // Lines up with the icon column of the rows around it.
        container(text("")).width(Length::Fixed(14.0)),
        text("Zoom").size(TEXT_BODY).width(Length::Fill),
        step(Icon::Minus, MenuCommand::ZoomOut),
        button(
            container(
                text(format!("{}%", (zoom * 100.0).round() as i32))
                    .size(TEXT_SMALL)
                    .align_x(Alignment::Center),
            )
            .center(Length::Fill),
        )
        .width(Length::Fixed(48.0))
        .height(Length::Fixed(BTN))
        .padding(0)
        .style(menu_row_style)
        .on_press(Msg::Menu(MenuCommand::ZoomReset)),
        step(Icon::Add, MenuCommand::ZoomIn),
    ]
    .spacing(SP_SM)
    .align_y(Alignment::Center)
    .padding([2.0, SP_MD])
    .into()
}

/// The overflow menu and the invisible layer that closes it on an outside
/// click, or `None` when it is closed. Meant to be stacked over the whole
/// window.
pub(crate) fn menu_overlay(state: &FerriteBrowser) -> Option<Element<'_, Msg>> {
    if !state.show_menu {
        return None;
    }
    let palette = state.palette();
    let light = state.theme_mode == AppTheme::Light;
    let current_url = state
        .tab_urls
        .get(state.active_tab)
        .map_or("about:blank", String::as_str);
    let marked = state.bookmarks.iter().any(|b| b.url == current_url);
    let page = current_url != "about:blank";

    let mut rows: Vec<Element<Msg>> = vec![menu_item(
        Icon::Add,
        "New tab",
        Some(format!("{MOD_LABEL}+T")),
        MenuCommand::NewTab,
        palette,
    )];
    rows.push(menu_divider(palette));
    rows.push(zoom_row(state));
    rows.push(menu_item(
        Icon::Search,
        "Find in page",
        Some(format!("{MOD_LABEL}+F")),
        MenuCommand::Find,
        palette,
    ));
    if page {
        rows.push(menu_item(
            if marked {
                Icon::BookmarkFilled
            } else {
                Icon::BookmarkOutline
            },
            if marked {
                "Remove bookmark"
            } else {
                "Bookmark this page"
            },
            Some(format!("{MOD_LABEL}+D")),
            MenuCommand::BookmarkPage,
            palette,
        ));
    }
    rows.push(menu_divider(palette));
    rows.push(menu_item(
        Icon::BookmarkOutline,
        "Bookmarks",
        None,
        MenuCommand::Library(LibraryTab::Bookmarks),
        palette,
    ));
    rows.push(menu_item(
        Icon::History,
        "History",
        None,
        MenuCommand::Library(LibraryTab::History),
        palette,
    ));
    rows.push(menu_item(
        Icon::Download,
        "Downloads",
        None,
        MenuCommand::Library(LibraryTab::Downloads),
        palette,
    ));
    rows.push(menu_divider(palette));
    rows.push(menu_item(
        Icon::Console,
        "JavaScript console",
        Some(format!("{MOD_LABEL}+J")),
        MenuCommand::JsConsole,
        palette,
    ));
    rows.push(menu_item(
        if light { Icon::Moon } else { Icon::Sun },
        if light {
            "Switch to dark theme"
        } else {
            "Switch to light theme"
        },
        None,
        MenuCommand::ToggleTheme,
        palette,
    ));
    rows.push(menu_item(
        Icon::Settings,
        "Settings",
        None,
        MenuCommand::Settings,
        palette,
    ));

    // A press on the card itself must not reach the click-away layer below.
    let card = mouse_area(
        container(column(rows).spacing(2).width(Length::Fill))
            .width(Length::Fixed(MENU_WIDTH))
            .padding(SP_XS)
            .style(popover_style),
    )
    .on_press(Msg::Noop);

    let anchored = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .padding(Padding {
            // Slides down into place as it opens (decoration only; the rows
            // are fully drawn from the first frame).
            top: TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT
                - SP_XS
                - (1.0 - crate::ease_out_cubic(state.menu_anim)) * 8.0,
            right: SP_SM,
            ..Padding::ZERO
        });
    let click_away = mouse_area(container(text("")).width(Length::Fill).height(Length::Fill))
        .on_press(Msg::CloseMenu);
    Some(stack([click_away.into(), anchored.into()]).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_tab_is_capped_and_many_tabs_share_the_strip() {
        assert_eq!(tab_widths(1, 1200.0), (TAB_MAX_WIDTH, TAB_MAX_WIDTH));
        let (w, a) = tab_widths(10, 1400.0);
        assert_eq!(w, a, "while the shares are wide enough everyone is equal");
        assert!(w < TAB_MAX_WIDTH && w > TAB_MIN_WIDTH, "{w}");
        // The shares and the gaps fill the strip exactly.
        assert!((w * 10.0 + TAB_SPACING * 9.0 - 1400.0).abs() < 0.01);
    }

    #[test]
    fn tabs_shrink_as_more_open_down_to_a_floor() {
        let mut last = f32::MAX;
        for n in 1..=60 {
            let (w, _) = tab_widths(n, 900.0);
            assert!(w <= last, "tab {n} got wider");
            assert!(w >= TAB_MIN_WIDTH);
            last = w;
        }
        assert_eq!(tab_widths(60, 900.0).0, TAB_MIN_WIDTH);
    }

    #[test]
    fn the_active_tab_stays_readable_when_the_others_squeeze() {
        let (others, active) = tab_widths(20, 900.0);
        assert!(others < ACTIVE_TAB_MIN_WIDTH);
        assert_eq!(active, ACTIVE_TAB_MIN_WIDTH);
        // And the pair still adds up to the strip.
        let total = others * 19.0 + active + TAB_SPACING * 19.0;
        assert!(total <= 900.0 + 0.01, "{total}");
    }

    #[test]
    fn the_strip_scrolls_only_once_tabs_hit_the_floor() {
        assert!(!tabs_overflow(5, 900.0));
        assert!(tabs_overflow(60, 900.0));
        // The exact edge: n-1 tabs at the floor, the active one at its minimum.
        let n = 8;
        let edge = (n as f32 - 1.0) * TAB_MIN_WIDTH
            + ACTIVE_TAB_MIN_WIDTH
            + TAB_SPACING * (n as f32 - 1.0);
        assert!(!tabs_overflow(n, edge));
        assert!(tabs_overflow(n, edge - 1.0));
    }

    #[test]
    fn zero_tabs_and_a_tiny_strip_do_not_panic() {
        assert_eq!(tab_widths(0, 100.0).0, TAB_MAX_WIDTH);
        assert_eq!(tab_widths(3, -50.0).0, TAB_MIN_WIDTH);
        assert!(!tabs_overflow(0, 0.0));
    }

    #[test]
    fn narrow_tabs_drop_the_title_and_hide_close_until_needed() {
        let wide = tab_parts(200.0, false, false, true);
        assert!(wide.title && !wide.close);
        assert!(
            tab_parts(200.0, false, true, true).close,
            "hover shows close"
        );
        assert!(
            tab_parts(200.0, true, false, true).close,
            "active shows close"
        );
        let narrow = tab_parts(TAB_MIN_WIDTH, false, false, true);
        assert!(!narrow.title && !narrow.close);
        // No room for a close button under the pointer on an icon-only tab.
        assert!(!tab_parts(TAB_MIN_WIDTH, false, true, true).close);
        // The last tab cannot be closed.
        assert!(!tab_parts(200.0, true, true, false).close);
    }

    #[test]
    fn the_title_budget_leaves_room_for_icon_and_close() {
        let with_close = TabParts {
            title: true,
            close: true,
        };
        let without = TabParts {
            title: true,
            close: false,
        };
        assert!(tab_title_budget(200.0, with_close) < tab_title_budget(200.0, without));
        assert_eq!(tab_title_budget(10.0, with_close), 0.0);
    }

    #[test]
    fn a_title_that_fits_is_left_alone() {
        assert_eq!(fit_title("Example", 200.0, 12.0), "Example");
        assert_eq!(fit_title("  padded  ", 200.0, 12.0), "padded");
    }

    #[test]
    fn a_long_title_is_cut_to_one_line_with_an_ellipsis_inside_the_budget() {
        let long = "rayanjainn/Ferrite-Browser: an agentic browser with an architectural defense";
        let budget = 120.0;
        let cut = fit_title(long, budget, 12.0);
        assert!(cut.ends_with('\u{2026}'), "{cut}");
        assert!(!cut.contains('\n'));
        assert!(
            text_width(&cut, 12.0) <= budget + 1.0,
            "{cut} is wider than {budget}"
        );
        assert!(cut.starts_with("rayanjainn/"), "{cut}");
    }

    #[test]
    fn a_budget_too_small_for_any_character_shows_nothing() {
        assert_eq!(fit_title("Anything", 3.0, 12.0), "");
    }

    #[test]
    fn cutting_never_splits_a_multibyte_character() {
        let cut = fit_title("日本語のとても長いタイトルです", 60.0, 12.0);
        assert!(cut.ends_with('\u{2026}'));
        assert!(cut.chars().count() >= 2);
    }

    #[test]
    fn wide_letters_cost_more_than_narrow_ones() {
        assert!(text_width("WWWW", 12.0) > text_width("iiii", 12.0) * 2.0);
        assert!(text_width("日本", 12.0) > text_width("ab", 12.0));
    }

    #[test]
    fn a_tabs_label_is_its_title_then_its_host_then_new_tab() {
        assert_eq!(tab_label("Rust", "https://rust-lang.org/", false), "Rust");
        assert_eq!(tab_label("", "https://docs.rs/x", false), "docs.rs");
        assert_eq!(tab_label("New Tab", "https://docs.rs/x", false), "docs.rs");
        assert_eq!(tab_label("Whatever", "about:blank", false), "New Tab");
        assert_eq!(tab_label("", "", true), "New Tab");
        assert_eq!(tab_label("", "weird", true), "Loading\u{2026}");
        assert_eq!(tab_label("", "weird", false), "weird");
    }

    #[test]
    fn the_window_title_names_the_page_and_the_app() {
        assert_eq!(window_title("", "about:blank"), "Ferrite");
        assert_eq!(
            window_title("Rust", "https://rust-lang.org"),
            "Rust \u{2014} Ferrite"
        );
        assert_eq!(
            window_title("", "https://docs.rs/x"),
            "docs.rs \u{2014} Ferrite"
        );
    }

    #[test]
    fn the_loading_band_enters_crosses_and_leaves() {
        let (l, b, r) = progress_band(0.0);
        assert_eq!((l, b), (0, 0), "starts fully off the left edge");
        assert_eq!(r, 1000);
        let (l, b, r) = progress_band(0.5);
        assert!(
            b > 0 && l > 0 && r > 0,
            "mid-sweep is inside the bar: {l} {b} {r}"
        );
        assert_eq!(l + b + r, 1000);
        let (l, b, r) = progress_band(1.0);
        assert_eq!((b, r), (0, 0), "ends fully off the right edge");
        assert_eq!(l, 1000);
    }

    #[test]
    fn the_loading_band_always_adds_up_to_the_whole_bar() {
        for step in 0..=100 {
            let (l, b, r) = progress_band(step as f32 / 100.0);
            assert_eq!(l + b + r, 1000, "t={step}%");
            assert!(b <= 300);
        }
        // Out-of-range phases clamp.
        assert_eq!(progress_band(-3.0), progress_band(0.0));
        assert_eq!(progress_band(9.0), progress_band(1.0));
    }

    #[test]
    fn two_quick_presses_are_a_double_click_and_slow_ones_are_not() {
        let t0 = Instant::now();
        assert!(!is_double_click(None, t0));
        assert!(is_double_click(Some(t0), t0 + Duration::from_millis(250)));
        assert!(!is_double_click(Some(t0), t0 + Duration::from_millis(900)));
    }

    #[test]
    fn every_menu_command_maps_to_its_message() {
        assert!(matches!(MenuCommand::NewTab.message(), Msg::AddTab));
        assert!(matches!(MenuCommand::ZoomIn.message(), Msg::ZoomIn));
        assert!(matches!(MenuCommand::ZoomOut.message(), Msg::ZoomOut));
        assert!(matches!(MenuCommand::ZoomReset.message(), Msg::ZoomReset));
        assert!(matches!(MenuCommand::Find.message(), Msg::OpenFindBar));
        assert!(matches!(
            MenuCommand::BookmarkPage.message(),
            Msg::ToggleBookmarkCurrentPage
        ));
        assert!(matches!(
            MenuCommand::Library(LibraryTab::History).message(),
            Msg::OpenLibrary(LibraryTab::History)
        ));
        assert!(matches!(
            MenuCommand::JsConsole.message(),
            Msg::ToggleJsConsole
        ));
        assert!(matches!(
            MenuCommand::ToggleTheme.message(),
            Msg::ToggleTheme
        ));
        assert!(matches!(
            MenuCommand::Settings.message(),
            Msg::ToggleSettingsPanel
        ));
    }

    #[test]
    fn the_strip_and_toolbar_together_are_compact() {
        // The owner's complaint was wasted rows above the page.
        const _: () = assert!(TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT <= 76.0);
    }
}
