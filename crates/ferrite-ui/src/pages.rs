// The full-area pages shown in place of a web page: the new-tab page, the
// loading placeholder and the load-failed page. All three are plain iced
// widgets built from Rust data; none of it is page-supplied.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};
use iced_widget::image::{Handle as ImageHandle, Image as ServoImage};

use crate::icons::{icon, Icon};
use crate::tokens::{
    accent_btn_style, field_style, keycap_style, nav_btn_style, page_style, panel_btn_inactive,
    tint, RADIUS_LG, RADIUS_MD, SP_LG, SP_MD, SP_SM, SP_XL, SP_XS, TEXT_BODY, TEXT_CAPTION,
    TEXT_HEADING, TEXT_SMALL, TEXT_TITLE,
};
use crate::{
    font_weight, pulse_alpha, resolve_url, tile_monogram, truncate, FerriteBrowser,
    FerriteBrowserMessage as Msg, Palette, MOD_LABEL, QUICK_ACCESS_TILES,
};

/// Width shared by the new-tab page's search field and quick-access grid so
/// the two edges line up (six 92 px tiles with gaps).
const CONTENT_WIDTH: f32 = 604.0;
const TILE_WIDTH: f32 = 92.0;

/// How many recently visited pages the new-tab page offers.
const RECENT_LIMIT: usize = 4;

/// The page shown when a navigation failed: what failed, why, and the two
/// things to do about it.
pub(crate) fn error_page<'a>(
    palette: &'static Palette,
    failed_url: &'a str,
    reason: &'a str,
) -> Element<'a, Msg> {
    let mark = container(icon(Icon::Warning, 22.0, palette.danger))
        .center(Length::Fixed(48.0))
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(tint(palette.danger, 0.12))),
            border: Border {
                radius: 24.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    container(
        column![
            mark,
            text("This page could not be loaded")
                .size(TEXT_HEADING)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
            column![
                text(truncate(failed_url, 90))
                    .size(TEXT_BODY)
                    .color(palette.text_dim),
                text(truncate(reason, 160))
                    .size(TEXT_SMALL)
                    .color(palette.text_dim),
            ]
            .spacing(SP_XS)
            .align_x(Alignment::Center),
            row![
                button(text("Try again").size(TEXT_BODY))
                    .padding([SP_SM, SP_XL])
                    .style(accent_btn_style)
                    .on_press(Msg::Reload),
                button(text("Open a new tab page").size(TEXT_BODY))
                    .padding([SP_SM, SP_LG])
                    .style(panel_btn_inactive)
                    .on_press(Msg::NavigateRequested("about:blank".to_string())),
            ]
            .spacing(SP_MD),
        ]
        .spacing(SP_LG)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .style(page_style)
    .into()
}

/// The placeholder before a page's first frame: the mark, breathing.
pub(crate) fn loading_page(palette: &'static Palette, phase: f32) -> Element<'static, Msg> {
    let pulse = pulse_alpha(phase, 1.0, 0.25, 0.20);
    container(
        column![
            mark(palette, 44.0, tint(palette.accent, pulse)),
            text("Loading\u{2026}")
                .size(TEXT_BODY)
                .color(palette.text_dim),
        ]
        .spacing(SP_MD)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .style(page_style)
    .into()
}

/// The Ferrite mark: a rounded square with an F.
fn mark(palette: &'static Palette, size: f32, fill: Color) -> Element<'static, Msg> {
    container(
        text("F")
            .size(size * 0.5)
            .font(font_weight(iced::font::Weight::Bold))
            .color(palette.base),
    )
    .center(Length::Fixed(size))
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(fill)),
        border: Border {
            radius: (size * 0.28).into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// A rounded, tinted square housing either the tile's real favicon (once
/// `TileFaviconReady` has landed) or its monogram fallback — one shared
/// container style for both, so a favicon arriving mid-session never makes
/// the tile jump to a differently-sized/positioned glyph, only swaps what's
/// drawn inside the same backdrop. `accent` colours the monogram and the
/// backdrop's tint; a real favicon already carries the site's own colours.
fn tile_glyph<'a>(
    favicon: Option<&ImageHandle>,
    monogram: &str,
    accent: Color,
) -> Element<'a, Msg> {
    // A real favicon sits on a light tile so a dark icon (GitHub's, Rust's) is
    // still visible on the dark theme; a monogram sits on a quiet tint.
    let has_icon = favicon.is_some();
    let backdrop_style = move |_: &Theme| container::Style {
        background: Some(Background::Color(if has_icon {
            tint(Color::WHITE, 0.96)
        } else {
            tint(accent, 0.14)
        })),
        border: Border {
            radius: 11.0.into(),
            width: 1.0,
            color: if has_icon {
                tint(Color::BLACK, 0.20)
            } else {
                tint(accent, 0.30)
            },
        },
        ..container::Style::default()
    };
    let inner: Element<'a, Msg> = match favicon {
        Some(handle) => ServoImage::new(handle.clone())
            .width(Length::Fixed(24.0))
            .height(Length::Fixed(24.0))
            .into(),
        None => text(monogram.to_string())
            .size(TEXT_TITLE)
            .color(accent)
            .into(),
    };
    // `center(len)` sets BOTH width and height to `len`; pairing it with
    // `width`/`height` first silently made the tile fill its whole parent.
    container(inner)
        .center(Length::Fixed(38.0))
        .style(backdrop_style)
        .into()
}

/// A small keyboard hint: the key in a keycap, then what it does.
fn hint<'a>(palette: &'static Palette, keys: String, what: &'static str) -> Element<'a, Msg> {
    row![
        container(
            text(keys)
                .size(TEXT_CAPTION)
                .font(font_weight(iced::font::Weight::Medium))
                .color(palette.text_dim),
        )
        .padding([2.0, SP_SM - 2.0])
        .style(keycap_style),
        text(what).size(TEXT_CAPTION).color(palette.text_dim),
    ]
    .spacing(SP_SM - 2.0)
    .align_y(Alignment::Center)
    .into()
}

/// The newest distinct pages visited this session, newest first.
fn recent_pages(state: &FerriteBrowser) -> Vec<(&str, &str)> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for entry in state.history.iter().rev() {
        if seen.contains(&entry.url.as_str()) {
            continue;
        }
        seen.push(&entry.url);
        out.push((entry.title.as_str(), entry.url.as_str()));
        if out.len() == RECENT_LIMIT {
            break;
        }
    }
    out
}

/// The new-tab page: a flat, quiet start screen — wordmark, one search field,
/// an "ask the agent" shortcut, quick-access sites, this session's recent
/// pages when there are any, and a footer of keyboard hints. No animation and
/// no gradients: the search field is the only thing that draws the eye, and it
/// is usable from the first frame.
pub(crate) fn new_tab_page(state: &FerriteBrowser) -> Element<'_, Msg> {
    let palette = state.palette();

    let wordmark = row![
        mark(palette, 40.0, palette.accent),
        text("Ferrite")
            .size(26)
            .font(font_weight(iced::font::Weight::Semibold))
            .color(palette.text),
    ]
    .spacing(SP_MD)
    .align_y(Alignment::Center);

    let search = text_input(
        "Search the web or enter an address",
        &state.new_tab_search_input,
    )
    .width(CONTENT_WIDTH)
    .padding([SP_MD + 2.0, SP_XL - 4.0])
    .size(TEXT_TITLE)
    .style(|theme: &Theme, status| field_style(theme, status, 26.0))
    .on_input(Msg::NewTabSearchChanged)
    .on_submit(Msg::NavigateRequested(resolve_url(
        &state.new_tab_search_input,
    )));

    // A quiet second entry point for the agent; hidden once its panel is open.
    let agent_hint: Element<'_, Msg> = if state.show_agent_sidebar {
        container(text("")).height(30).into()
    } else {
        button(
            row![
                icon(Icon::Agent, 14.0, palette.text_dim),
                text("Ask the agent instead")
                    .size(TEXT_BODY)
                    .color(palette.text_dim),
            ]
            .spacing(SP_SM)
            .align_y(Alignment::Center),
        )
        .padding([SP_XS + 2.0, SP_MD])
        .style(nav_btn_style)
        .on_press(Msg::ToggleAgentSidebar)
        .into()
    };

    // ── Quick access ──
    let tiles: Vec<Element<Msg>> = QUICK_ACCESS_TILES
        .iter()
        .enumerate()
        .map(|(index, tile)| {
            let favicon = state.tile_favicons.get(index).and_then(|f| f.as_ref());
            let glyph = tile_glyph(favicon, &tile_monogram(tile.label), palette.text_dim);
            button(
                column![glyph, text(tile.label).size(TEXT_SMALL).color(palette.text)]
                    .spacing(SP_SM)
                    .align_x(Alignment::Center),
            )
            .width(TILE_WIDTH)
            .padding([SP_MD, SP_XS + 2.0])
            .style(|theme: &Theme, status| {
                let mut s = nav_btn_style(theme, status);
                s.border.radius = RADIUS_LG.into();
                s
            })
            .on_press(Msg::NavigateRequested(tile.url.to_string()))
            .into()
        })
        .collect();
    let quick_access = column![
        text("Quick access")
            .size(TEXT_SMALL)
            .font(font_weight(iced::font::Weight::Medium))
            .color(palette.text_dim),
        row(tiles).spacing(SP_SM + 1.0).wrap(),
    ]
    .spacing(SP_SM)
    .width(CONTENT_WIDTH);

    // ── Recent pages: only when there are some ──
    let recent = recent_pages(state);
    let recent_section: Option<Element<Msg>> = (!recent.is_empty()).then(|| {
        let rows: Vec<Element<Msg>> = recent
            .into_iter()
            .map(|(title, url)| {
                let label = if title.is_empty() { url } else { title };
                button(
                    row![
                        icon(Icon::History, 13.0, palette.text_dim),
                        text(truncate(label, 54))
                            .size(TEXT_BODY)
                            .color(palette.text),
                        text(truncate(url, 40))
                            .size(TEXT_CAPTION)
                            .color(palette.text_dim),
                    ]
                    .spacing(SP_MD)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([SP_SM - 2.0, SP_MD])
                .style(|theme: &Theme, status| {
                    let mut s = nav_btn_style(theme, status);
                    s.border.radius = RADIUS_MD.into();
                    s
                })
                .on_press(Msg::NavigateRequested(url.to_string()))
                .into()
            })
            .collect();
        column![
            text("Recently visited")
                .size(TEXT_SMALL)
                .font(font_weight(iced::font::Weight::Medium))
                .color(palette.text_dim),
            column(rows).spacing(2),
        ]
        .spacing(SP_SM)
        .width(CONTENT_WIDTH)
        .into()
    });

    // ── Footer: keyboard hints ──
    let footer = row![
        hint(palette, format!("{MOD_LABEL}+T"), "New tab"),
        hint(palette, format!("{MOD_LABEL}+L"), "Address bar"),
        hint(palette, format!("{MOD_LABEL}+Shift+A"), "Agent"),
        hint(palette, "F12".to_string(), "Audit log"),
    ]
    .spacing(SP_LG)
    .align_y(Alignment::Center);

    let mut body: Vec<Element<Msg>> = vec![wordmark.into(), search.into(), agent_hint];
    body.push(quick_access.into());
    if let Some(recent) = recent_section {
        body.push(recent);
    }
    body.push(footer.into());

    container(
        column(body)
            .spacing(SP_LG + SP_XS)
            .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .style(page_style)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HistoryEntry;

    fn visit(url: &str, title: &str) -> HistoryEntry {
        HistoryEntry {
            url: url.to_string(),
            title: title.to_string(),
            visited_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn recent_pages_are_newest_first_and_distinct() {
        let state = FerriteBrowser {
            history: vec![
                visit("https://a.example", "A"),
                visit("https://b.example", "B"),
                visit("https://a.example", "A again"),
            ],
            ..FerriteBrowser::default()
        };
        let recent = recent_pages(&state);
        assert_eq!(
            recent,
            vec![("A again", "https://a.example"), ("B", "https://b.example")]
        );
    }

    #[test]
    fn recent_pages_are_capped() {
        let state = FerriteBrowser {
            history: (0..20)
                .map(|i| visit(&format!("https://{i}.example"), "x"))
                .collect(),
            ..FerriteBrowser::default()
        };
        assert_eq!(recent_pages(&state).len(), RECENT_LIMIT);
        assert_eq!(recent_pages(&state)[0].1, "https://19.example");
    }

    #[test]
    fn no_history_means_no_recent_section() {
        assert!(recent_pages(&FerriteBrowser::default()).is_empty());
    }

    #[test]
    fn the_quick_access_grid_fits_its_container() {
        // Six tiles with their gaps must fit the width the search field uses,
        // so the grid does not wrap to a ragged second row on its own.
        let n = QUICK_ACCESS_TILES.len() as f32;
        assert!(n * TILE_WIDTH + (n - 1.0) * (SP_SM + 1.0) <= CONTENT_WIDTH);
    }
}
