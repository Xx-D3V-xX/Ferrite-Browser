// ferrite-ui design tokens: the spacing, type and radius scales, the elevation
// recipes, and the shared button/container styles every view builds from.
//
// One scale, used everywhere, is what makes the chrome read as one product:
//
// - **Spacing** is a 4 px grid: `SP_XS` 4, `SP_SM` 8, `SP_MD` 12, `SP_LG` 16,
//   `SP_XL` 24. Padding and gaps are always one of these (the odd 6 px inside
//   a control that is itself on the grid is the one exception).
// - **Type** has five steps: caption 11, small 12, body 13, title 15,
//   heading 20. Weight (`font_weight`) carries emphasis, not extra sizes.
// - **Radius**: `RADIUS_SM` 6 (chips, inputs), `RADIUS_MD` 8 (buttons, tabs),
//   `RADIUS_LG` 12 (cards, popovers); toolbar controls are pills.
// - **Elevation** is a shadow, never a heavier border: `shadow_card` for
//   things that sit on a surface, `shadow_popover` for things that float over
//   the page (the menu, the find bar).
// - **State** is an overlay, not a second palette: hover is the text colour at
//   8 % alpha, pressed 14 %, so every control gets the same cue in both
//   themes. `iced` 0.13 buttons expose no keyboard-focus status, so focus is
//   shown only on text inputs (`field_style`'s accent ring).
//
// Every colour comes from the active [`Palette`]; nothing here hardcodes a
// theme.

use iced::widget::{button, container, stack, text_input, tooltip, Space};
use iced::{Background, Border, Color, Element, Length, Padding, Shadow, Theme, Vector};

use crate::{palette_for_theme, FerriteBrowserMessage, Palette};

// ── Spacing (4 px grid) ──────────────────────────────────────────────────
pub(crate) const SP_XS: f32 = 4.0;
pub(crate) const SP_SM: f32 = 8.0;
pub(crate) const SP_MD: f32 = 12.0;
pub(crate) const SP_LG: f32 = 16.0;
pub(crate) const SP_XL: f32 = 24.0;

// ── Type scale ───────────────────────────────────────────────────────────
pub(crate) const TEXT_CAPTION: f32 = 11.0;
pub(crate) const TEXT_SMALL: f32 = 12.0;
pub(crate) const TEXT_BODY: f32 = 13.0;
pub(crate) const TEXT_TITLE: f32 = 15.0;
pub(crate) const TEXT_HEADING: f32 = 20.0;

// ── Radius scale ─────────────────────────────────────────────────────────
pub(crate) const RADIUS_SM: f32 = 6.0;
pub(crate) const RADIUS_MD: f32 = 8.0;
pub(crate) const RADIUS_LG: f32 = 12.0;

/// `color` with its alpha replaced by `alpha` (not multiplied).
pub(crate) fn tint(color: Color, alpha: f32) -> Color {
    Color {
        a: alpha.clamp(0.0, 1.0),
        ..color
    }
}

/// `a` blended towards `b` by `t` (0 = `a`, 1 = `b`), fully opaque.
pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}

/// WCAG relative luminance of an opaque colour.
fn luminance(c: Color) -> f32 {
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio between two opaque colours, `1.0..=21.0`.
pub(crate) fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The label colour for text on a solid `fill`: white or near-black, whichever
/// reads better. The dark theme's bright mint and coral fills take the dark
/// label (white on them is 1.9:1 and 3.1:1); the light theme's deep ones, white.
pub(crate) fn on_fill(fill: Color) -> Color {
    let white = Color::WHITE;
    let ink = Color::from_rgb(0.05, 0.05, 0.07);
    if contrast_ratio(white, fill) >= contrast_ratio(ink, fill) {
        white
    } else {
        ink
    }
}

/// The background of a control the pointer is over.
pub(crate) fn hover_bg(palette: &Palette) -> Color {
    tint(palette.text, 0.08)
}

/// The background of a control being pressed.
pub(crate) fn pressed_bg(palette: &Palette) -> Color {
    tint(palette.text, 0.14)
}

fn radius(r: f32) -> iced::border::Radius {
    iced::border::Radius::new(r)
}

// ── Elevation ────────────────────────────────────────────────────────────

/// A card resting on a surface.
pub(crate) fn shadow_card() -> Shadow {
    Shadow {
        color: Color {
            a: 0.22,
            ..Color::BLACK
        },
        offset: Vector::new(0.0, 2.0),
        blur_radius: 8.0,
    }
}

/// A popover floating over the page.
pub(crate) fn shadow_popover() -> Shadow {
    Shadow {
        color: Color {
            a: 0.38,
            ..Color::BLACK
        },
        offset: Vector::new(0.0, 8.0),
        blur_radius: 24.0,
    }
}

// ── Containers ───────────────────────────────────────────────────────────

pub(crate) fn separator_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.divider)),
        ..container::Style::default()
    }
}

/// The strip behind the tabs: one step darker than the toolbar so the active
/// tab, which shares the toolbar's colour, reads as part of it.
pub(crate) fn tab_bar_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.chrome)),
        ..container::Style::default()
    }
}

pub(crate) fn toolbar_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.base)),
        ..container::Style::default()
    }
}

/// A raised bar inside a panel (a section header, a column header).
pub(crate) fn raised_bar_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.raised)),
        ..container::Style::default()
    }
}

/// The page-coloured fill behind full-area views (new tab, error page).
pub(crate) fn page_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.base)),
        ..container::Style::default()
    }
}

/// A bottom drawer (audit log, DevTools). No shadow: iced's software renderer
/// (tiny-skia) paints a shadow without honouring the area being redrawn, so a
/// shadow around something this large darkened the whole panel a little more
/// on every partial redraw (a click, a keystroke). The splitter above it and
/// its own border separate it from the page.
pub(crate) fn bottom_panel_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.surface)),
        border: Border {
            color: palette.divider,
            width: 1.0,
            radius: iced::border::Radius {
                top_left: RADIUS_MD,
                top_right: RADIUS_MD,
                bottom_left: 0.0,
                bottom_right: 0.0,
            },
        },
        ..container::Style::default()
    }
}

/// A side drawer (library, agent, settings): surface fill, hairline on the
/// page side only.
pub(crate) fn side_panel_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.surface)),
        border: Border {
            color: palette.divider,
            width: 1.0,
            radius: radius(0.0),
        },
        ..container::Style::default()
    }
}

/// A card on a surface: raised fill, hairline border, soft shadow.
pub(crate) fn card_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.raised)),
        border: Border {
            color: palette.divider,
            width: 1.0,
            radius: radius(RADIUS_LG),
        },
        shadow: shadow_card(),
        ..container::Style::default()
    }
}

/// Width of the coloured rule on a [`rule_card`].
const RULE_WIDTH: f32 = 4.0;

/// A card that wants a decision: a neutral raised surface with a hairline
/// border, and the only colour a thin `tone` rule down its left edge (warn for
/// "the agent needs you", danger for "something looks wrong"). The tone says
/// what kind of decision it is; it never tints the surface, so the text on it
/// keeps its full contrast and the card does not read as a coloured banner.
pub(crate) fn rule_card<'a>(
    palette: &'static Palette,
    tone: Color,
    content: Element<'a, FerriteBrowserMessage>,
) -> Element<'a, FerriteBrowserMessage> {
    // The card is the base layer and the rule a second layer that takes the
    // card's own height (a row cannot do this: a `Fill`-height child laid out
    // before its taller sibling is given a height of zero).
    let card = container(content)
        .padding(Padding {
            left: SP_MD + RULE_WIDTH,
            ..Padding::new(SP_MD)
        })
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(palette.raised)),
            border: Border {
                color: palette.divider,
                width: 1.0,
                radius: radius(RADIUS_MD),
            },
            shadow: shadow_card(),
            text_color: Some(palette.text),
        });
    let rule =
        container(Space::new(Length::Fixed(RULE_WIDTH), Length::Fill)).style(move |_: &Theme| {
            container::Style {
                background: Some(Background::Color(tone)),
                border: Border {
                    radius: iced::border::Radius {
                        top_left: RADIUS_MD,
                        bottom_left: RADIUS_MD,
                        top_right: 0.0,
                        bottom_right: 0.0,
                    },
                    ..Border::default()
                },
                ..container::Style::default()
            }
        });
    stack![card, rule]
        .width(Length::Fill)
        .height(Length::Shrink)
        .into()
}

/// A floating popover (the overflow menu).
pub(crate) fn popover_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(palette.raised)),
        border: Border {
            color: palette.divider,
            width: 1.0,
            radius: radius(RADIUS_LG),
        },
        shadow: shadow_popover(),
        ..container::Style::default()
    }
}

/// A small keycap-style label for a shortcut.
pub(crate) fn keycap_style(theme: &Theme) -> container::Style {
    let palette = palette_for_theme(theme);
    container::Style {
        background: Some(Background::Color(tint(palette.text, 0.06))),
        border: Border {
            color: palette.divider,
            width: 1.0,
            radius: radius(RADIUS_SM - 2.0),
        },
        ..container::Style::default()
    }
}

// ── Buttons ──────────────────────────────────────────────────────────────

/// Ghost button: transparent until hovered. For icon buttons in the toolbar,
/// the tab strip and panel headers.
pub(crate) fn nav_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => hover_bg(palette),
            button::Status::Pressed => pressed_bg(palette),
            _ => Color::TRANSPARENT,
        })),
        text_color: match status {
            button::Status::Disabled => tint(palette.text_dim, 0.30),
            _ => palette.text,
        },
        border: Border {
            radius: radius(RADIUS_MD),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// The toolbar's round icon button (back, forward, reload, menu): the same
/// ghost behaviour as [`nav_btn_style`] with a full pill radius.
pub(crate) fn toolbar_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        border: Border {
            radius: radius(100.0),
            ..Border::default()
        },
        ..nav_btn_style(theme, status)
    }
}

/// A toolbar toggle that is currently on: tinted accent, so "this panel is
/// open" is visible without a heavy filled button.
pub(crate) fn toolbar_btn_on_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(tint(
            palette.accent,
            match status {
                button::Status::Hovered => 0.28,
                button::Status::Pressed => 0.34,
                _ => 0.20,
            },
        ))),
        text_color: palette.accent_bright,
        border: Border {
            radius: radius(100.0),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Tab close button: ghost that turns danger-tinted under the pointer.
pub(crate) fn close_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => tint(palette.danger, 0.16),
            button::Status::Pressed => tint(palette.danger, 0.26),
            _ => Color::TRANSPARENT,
        })),
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => palette.danger,
            _ => palette.text_dim,
        },
        border: Border {
            radius: radius(100.0),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Active panel toggle / selected segment: solid accent.
pub(crate) fn panel_btn_active(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => palette.accent_fill_hover,
            _ => palette.accent_fill,
        })),
        text_color: on_fill(palette.accent_fill),
        border: Border {
            radius: radius(RADIUS_MD),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Secondary button: outlined, quiet until hovered. Also the "unselected"
/// segment of a segmented control.
pub(crate) fn panel_btn_inactive(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => hover_bg(palette),
            button::Status::Pressed => pressed_bg(palette),
            _ => Color::TRANSPARENT,
        })),
        text_color: match status {
            button::Status::Disabled => tint(palette.text_dim, 0.45),
            button::Status::Hovered | button::Status::Pressed => palette.text,
            _ => palette.text_dim,
        },
        border: Border {
            radius: radius(RADIUS_MD),
            width: 1.0,
            color: palette.divider,
        },
        ..button::Style::default()
    }
}

/// Outlined button with full-strength text, for the choices in a decision
/// card: quiet, but readable as an action rather than as disabled.
pub(crate) fn outline_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        text_color: match status {
            button::Status::Disabled => tint(palette.text_dim, 0.45),
            _ => palette.text,
        },
        ..panel_btn_inactive(theme, status)
    }
}

/// Primary button: solid accent. Dimmed (not hidden) when disabled.
pub(crate) fn accent_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    let fill = match status {
        button::Status::Hovered => palette.accent_fill_hover,
        button::Status::Pressed => darken(palette.accent_fill, 0.88),
        button::Status::Disabled => tint(palette.accent_fill, 0.35),
        button::Status::Active => palette.accent_fill,
    };
    button::Style {
        background: Some(Background::Color(fill)),
        text_color: match status {
            button::Status::Disabled => tint(on_fill(palette.accent_fill), 0.7),
            _ => on_fill(palette.accent_fill),
        },
        border: Border {
            radius: radius(RADIUS_MD),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// `color` with each channel scaled by `factor`.
fn darken(color: Color, factor: f32) -> Color {
    Color {
        r: color.r * factor,
        g: color.g * factor,
        b: color.b * factor,
        a: 1.0,
    }
}

/// Destructive button: solid danger.
pub(crate) fn danger_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => tint(palette.danger, 0.88),
            button::Status::Pressed => darken(palette.danger, 0.85),
            button::Status::Disabled => tint(palette.danger, 0.35),
            button::Status::Active => palette.danger,
        })),
        text_color: on_fill(palette.danger),
        border: Border {
            radius: radius(RADIUS_MD),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// A choice that is made with colour but not with a loud fill: a wash of
/// `tone` over whatever is behind, a hairline in `tone`, and the ordinary text
/// colour, so the label reads as well as on any other button.
fn soft_btn(tone: Color, text: Color, status: button::Status) -> button::Style {
    let wash = match status {
        button::Status::Active => 0.16,
        button::Status::Hovered => 0.24,
        button::Status::Pressed => 0.32,
        button::Status::Disabled => 0.06,
    };
    button::Style {
        background: Some(Background::Color(tint(tone, wash))),
        text_color: text,
        border: Border {
            radius: radius(RADIUS_MD),
            width: 1.0,
            color: tint(tone, 0.6),
        },
        ..button::Style::default()
    }
}

/// "No" in a decision card: a soft danger wash, not a solid red fill. It is the
/// safe answer, so it is the one with colour, but a person deciding should not
/// be shouted at.
pub(crate) fn deny_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    soft_btn(palette.danger, palette.text, status)
}

/// "Yes" in a decision card, in the same soft style as [`deny_btn_style`].
pub(crate) fn allow_btn_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    soft_btn(palette.safe, palette.text, status)
}

/// A row in a menu or list: transparent, a hover wash, square-ish corners.
pub(crate) fn menu_row_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = palette_for_theme(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => hover_bg(palette),
            button::Status::Pressed => pressed_bg(palette),
            _ => Color::TRANSPARENT,
        })),
        text_color: match status {
            button::Status::Disabled => tint(palette.text_dim, 0.4),
            _ => palette.text,
        },
        border: Border {
            radius: radius(RADIUS_SM),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

// ── Inputs ───────────────────────────────────────────────────────────────

/// A boxed text field with the accent focus ring every input shares.
pub(crate) fn field_style(
    theme: &Theme,
    status: text_input::Status,
    corner: f32,
) -> text_input::Style {
    let palette = palette_for_theme(theme);
    let focused = matches!(status, text_input::Status::Focused);
    let disabled = matches!(status, text_input::Status::Disabled);
    text_input::Style {
        background: Background::Color(if disabled {
            tint(palette.input, 0.6)
        } else {
            palette.input
        }),
        border: Border {
            radius: radius(corner),
            width: if focused { 1.5 } else { 1.0 },
            color: if focused {
                palette.accent
            } else {
                palette.divider
            },
        },
        icon: palette.text_dim,
        placeholder: palette.text_dim,
        value: if disabled {
            palette.text_dim
        } else {
            palette.text
        },
        selection: tint(palette.accent, 0.30),
    }
}

/// A text input with no box of its own, for a field drawn inside a pill (the
/// address bar draws the pill and the focus ring around it).
pub(crate) fn bare_field_style(theme: &Theme, _status: text_input::Status) -> text_input::Style {
    let palette = palette_for_theme(theme);
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border {
            radius: radius(0.0),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        icon: palette.text_dim,
        placeholder: palette.text_dim,
        value: palette.text,
        selection: tint(palette.accent, 0.30),
    }
}

// ── Tooltips ─────────────────────────────────────────────────────────────

/// `content` with a tooltip below it, in the raised-surface style. The one
/// tooltip every icon-only control uses.
pub(crate) fn tip<'a>(
    content: impl Into<Element<'a, FerriteBrowserMessage>>,
    label: impl Into<String>,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    tooltip(
        content,
        container(
            iced::widget::text(label.into())
                .size(TEXT_CAPTION)
                .color(palette.text),
        )
        .padding([SP_XS, SP_SM])
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(palette.raised)),
            border: Border {
                radius: radius(RADIUS_SM),
                width: 1.0,
                color: palette.divider,
            },
            shadow: shadow_card(),
            ..container::Style::default()
        }),
        tooltip::Position::Bottom,
    )
    .gap(SP_XS)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DARK_PALETTE, LIGHT_PALETTE};

    #[test]
    fn the_spacing_scale_is_a_four_pixel_grid() {
        for s in [SP_XS, SP_SM, SP_MD, SP_LG, SP_XL] {
            assert_eq!(s % 4.0, 0.0, "{s} is off the 4 px grid");
        }
        const _: () = assert!(SP_XS < SP_SM && SP_SM < SP_MD && SP_MD < SP_LG && SP_LG < SP_XL);
    }

    #[test]
    fn the_type_scale_only_goes_up() {
        const _: () = assert!(
            TEXT_CAPTION < TEXT_SMALL
                && TEXT_SMALL < TEXT_BODY
                && TEXT_BODY < TEXT_TITLE
                && TEXT_TITLE < TEXT_HEADING
        );
    }

    #[test]
    fn hover_is_a_lighter_wash_than_pressed_in_both_themes() {
        for palette in [&DARK_PALETTE, &LIGHT_PALETTE] {
            assert!(hover_bg(palette).a < pressed_bg(palette).a);
        }
    }

    #[test]
    fn deny_and_allow_are_washes_whose_labels_stay_readable_in_both_themes() {
        for (theme, palette) in [(Theme::Dark, &DARK_PALETTE), (Theme::Light, &LIGHT_PALETTE)] {
            for style in [deny_btn_style, allow_btn_style] {
                for status in [
                    button::Status::Active,
                    button::Status::Hovered,
                    button::Status::Pressed,
                ] {
                    let s = style(&theme, status);
                    let Some(Background::Color(wash)) = s.background else {
                        panic!("a decision button has a background");
                    };
                    assert!(wash.a <= 0.35, "{status:?}: a wash, not a solid fill");
                    let under = mix(palette.raised, Color { a: 1.0, ..wash }, wash.a);
                    assert!(
                        contrast_ratio(s.text_color, under) >= 4.5,
                        "{status:?} label on {under:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn tint_replaces_alpha_and_clamps() {
        let c = tint(Color::WHITE, 2.0);
        assert_eq!(c.a, 1.0);
        assert_eq!(tint(Color::WHITE, -1.0).a, 0.0);
        assert_eq!(tint(Color::BLACK, 0.5).a, 0.5);
    }

    #[test]
    fn the_tab_strip_is_a_step_away_from_the_toolbar_in_both_themes() {
        // The active tab shares the toolbar's colour; if the strip matched it,
        // the tab would not read as a tab.
        for palette in [&DARK_PALETTE, &LIGHT_PALETTE] {
            assert_ne!(
                (palette.chrome.r, palette.chrome.g, palette.chrome.b),
                (palette.base.r, palette.base.g, palette.base.b)
            );
        }
    }

    #[test]
    fn a_disabled_primary_button_is_dimmed_not_invisible() {
        let s = accent_btn_style(&Theme::Dark, button::Status::Disabled);
        let Some(Background::Color(c)) = s.background else {
            panic!("a disabled primary button keeps a background");
        };
        assert!(c.a > 0.2 && c.a < 0.6, "alpha {}", c.a);
    }
}
