//! What a page asks for on the person's machine (the camera, the microphone, the
//! screen), and the bar that says a page is using them.
//!
//! The rules live in `ferrite_servo::permissions`; this is the part that is drawn.
//! Three things matter here:
//!
//! * **The card is browser chrome, and a person answers it.** It is drawn by the
//!   shell over the page, names the site, says what it wants, and is answered with
//!   the mouse or the keyboard on the shell's own buttons. Nothing the page draws, and
//!   nothing the AI agent can do (it acts on page elements by reference), reaches it.
//! * **The agent's presence is said on the card.** When the agent was working in the
//!   tab as the request came, the card is marked and "always" is not offered, because
//!   a standing permission is not used while the agent works (see the rules in
//!   `ferrite_servo::permissions`).
//! * **A page that is capturing is always visible.** While a camera, microphone or
//!   screen track is live a bar across the top of the page says which and offers
//!   "Stop sharing". The page cannot hide it.

use ferrite_servo::permissions::{CapabilityKind, PermissionChoice, PermissionPrompt};
use iced::widget::{button, column, container, mouse_area, row, stack, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::icons::{icon, Icon};
use crate::tokens::{
    accent_btn_style, outline_btn_style, shadow_popover, tint, RADIUS_LG, RADIUS_MD, SP_LG, SP_MD,
    SP_SM, SP_XS, TEXT_BODY, TEXT_CAPTION, TEXT_SMALL, TEXT_TITLE,
};
use crate::{font_weight, FerriteBrowser, FerriteBrowserMessage};

/// What a press on the card or the bar means.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Answer(PermissionChoice),
    /// "Stop sharing" on the bar.
    StopSharing,
}

const CARD_W: f32 = 430.0;

/// The prompt the active tab is showing, if any.
pub(crate) fn active_prompt(state: &FerriteBrowser) -> Option<&PermissionPrompt> {
    state
        .tab_diag
        .get(state.active_tab)
        .and_then(|d| d.permission.as_ref())
}

/// What the active tab has live: `(camera, microphone, screen)`.
pub(crate) fn active_capture(state: &FerriteBrowser) -> (bool, bool, bool) {
    state
        .tab_diag
        .get(state.active_tab)
        .map_or((false, false, false), |d| d.capture)
}

fn msg(m: Msg) -> FerriteBrowserMessage {
    FerriteBrowserMessage::Permission(m)
}

/// "your camera", "your camera and microphone", "your screen".
fn what(kinds: &[CapabilityKind]) -> String {
    let names: Vec<&str> = kinds
        .iter()
        .map(|k| match k {
            CapabilityKind::Camera => "camera",
            CapabilityKind::Microphone => "microphone",
            CapabilityKind::Screen => "screen",
        })
        .collect();
    match names.as_slice() {
        [] => "your computer".to_string(),
        [one] => format!("your {one}"),
        [a, b] => format!("your {a} and {b}"),
        [init @ .., last] => format!("your {} and {last}", init.join(", ")),
    }
}

/// Whether this prompt may offer to remember an "allow": not while the agent works,
/// and not for the screen.
fn may_offer_always(prompt: &PermissionPrompt) -> bool {
    !prompt.agent_active
        && prompt
            .kinds
            .iter()
            .all(|k| k.may_be_remembered_as_allowed())
}

/// What a press on a card button answers, as a pure function so it can be tested:
/// `allow` or block, and whether to remember it.
pub(crate) fn choice(prompt: &PermissionPrompt, allow: bool, always: bool) -> PermissionChoice {
    if allow {
        PermissionChoice::Allow {
            remember: always && may_offer_always(prompt),
        }
    } else {
        PermissionChoice::Block { remember: always }
    }
}

/// The card, centred over a dimmed layer that blocks the page.
pub(crate) fn overlay(state: &FerriteBrowser) -> Option<Element<'_, FerriteBrowserMessage>> {
    let prompt = active_prompt(state)?;
    let palette = state.palette();
    let screen = prompt.kinds.contains(&CapabilityKind::Screen);

    let header = row![
        icon(
            if prompt.agent_active {
                Icon::Warning
            } else {
                Icon::Origin
            },
            16.0,
            if prompt.agent_active {
                palette.warn
            } else {
                palette.text_dim
            },
        ),
        column![text(format!(
            "Allow {} to use {}?",
            prompt.origin,
            what(&prompt.kinds)
        ))
        .size(TEXT_TITLE)
        .font(font_weight(iced::font::Weight::Semibold))
        .color(palette.text)
        .wrapping(text::Wrapping::WordOrGlyph),]
        .width(Length::Fill),
    ]
    .spacing(SP_SM)
    .align_y(Alignment::Start);

    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![header.into()];
    if prompt.agent_active {
        body.push(
            container(
                text(
                    "The AI agent is working in this tab. Allow this only if you started it \
                     and expect it. A permission you gave this site before is not used while \
                     the agent works.",
                )
                .size(TEXT_SMALL)
                .color(palette.text),
            )
            .padding(SP_MD)
            .width(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(tint(palette.warn, 0.14))),
                border: Border {
                    radius: RADIUS_MD.into(),
                    width: 1.0,
                    color: tint(palette.warn, 0.5),
                },
                ..container::Style::default()
            })
            .into(),
        );
    }
    body.push(
        text(if screen {
            "It will see everything on your screen, including other windows. You are asked \
             every time you share the screen."
        } else {
            "Only share with a site you trust. You can stop at any time from the bar at \
             the top of the page."
        })
        .size(TEXT_CAPTION)
        .color(palette.text_dim)
        .into(),
    );

    let block = |always: bool, label: &'static str, primary: bool| {
        let b = button(text(label).size(TEXT_BODY))
            .padding([SP_SM - 1.0, SP_LG])
            .on_press(msg(Msg::Answer(choice(prompt, false, always))));
        if primary {
            b.style(accent_btn_style)
        } else {
            b.style(outline_btn_style)
        }
    };
    let allow = |always: bool, label: &'static str, primary: bool| {
        let b = button(text(label).size(TEXT_BODY))
            .padding([SP_SM - 1.0, SP_LG])
            .on_press(msg(Msg::Answer(choice(prompt, true, always))));
        if primary {
            b.style(accent_btn_style)
        } else {
            b.style(outline_btn_style)
        }
    };
    // The safe answer is first and the one Enter would take is not "always".
    let mut actions: Vec<Element<FerriteBrowserMessage>> = vec![
        Space::with_width(Length::Fill).into(),
        block(false, "Block", false).into(),
    ];
    if may_offer_always(prompt) {
        actions.push(allow(true, "Always allow", false).into());
    }
    actions.push(allow(false, "Allow this time", true).into());
    body.push(
        row(actions)
            .spacing(SP_SM)
            .align_y(Alignment::Center)
            .into(),
    );

    let card = container(column(body).spacing(SP_MD))
        .width(Length::Fixed(CARD_W))
        .padding(SP_LG)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(palette.raised)),
            border: Border {
                radius: RADIUS_LG.into(),
                width: 1.0,
                color: if prompt.agent_active {
                    palette.warn
                } else {
                    palette.divider
                },
            },
            shadow: shadow_popover(),
            ..container::Style::default()
        });

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
    Some(stack([scrim.into(), centred.into()]).into())
}

/// The bar across the top of the page while it is capturing.
pub(crate) fn bar(state: &FerriteBrowser) -> Option<Element<'_, FerriteBrowserMessage>> {
    let (camera, microphone, screen) = active_capture(state);
    if !(camera || microphone || screen) {
        return None;
    }
    let palette = state.palette();
    let mut kinds = vec![];
    if camera {
        kinds.push(CapabilityKind::Camera);
    }
    if microphone {
        kinds.push(CapabilityKind::Microphone);
    }
    let sentence = if screen && kinds.is_empty() {
        "This page is sharing your screen".to_string()
    } else if screen {
        format!(
            "This page is using {} and sharing your screen",
            what(&kinds)
        )
    } else {
        format!("This page is using {}", what(&kinds))
    };
    let bar = container(
        row![
            // A filled dot, the colour every browser uses for "recording".
            container(Space::new(Length::Fixed(9.0), Length::Fixed(9.0))).style(
                move |_: &Theme| container::Style {
                    background: Some(Background::Color(palette.danger)),
                    border: Border {
                        radius: 5.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            ),
            text(sentence)
                .size(TEXT_SMALL)
                .color(palette.text)
                .font(font_weight(iced::font::Weight::Semibold)),
            Space::with_width(Length::Fill),
            button(text("Stop sharing").size(TEXT_SMALL))
                .padding([SP_XS, SP_MD])
                .style(outline_btn_style)
                .on_press(msg(Msg::StopSharing)),
        ]
        .spacing(SP_SM)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([SP_XS + 2.0, SP_MD])
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(palette.raised)),
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: tint(palette.danger, 0.6),
        },
        ..container::Style::default()
    });
    Some(column![bar, Space::new(Length::Fill, Length::Fill)].into())
}

/// Applies a press on the card or the bar.
pub(crate) fn update(state: &mut FerriteBrowser, m: Msg) -> iced::Task<FerriteBrowserMessage> {
    let tab = state.active_tab;
    match m {
        Msg::Answer(choice) => {
            // Taken out of the tab before the engine hears anything, so a second press
            // finds nothing to answer.
            if let Some(diag) = state.tab_diag.get_mut(tab) {
                if diag.permission.take().is_some() {
                    if let Some(session) = state.servo_sessions.get_mut(&tab) {
                        session.answer_permission(choice);
                    }
                }
            }
        }
        Msg::StopSharing => {
            if let Some(session) = state.servo_sessions.get(&tab) {
                session.stop_capture();
            }
            if let Some(diag) = state.tab_diag.get_mut(tab) {
                diag.capture = (false, false, false);
            }
        }
    }
    crate::wake_flag(&mut state.busy_ticks);
    iced::Task::none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(kinds: &[CapabilityKind], agent: bool) -> PermissionPrompt {
        PermissionPrompt {
            origin: "https://meet.example".to_string(),
            kinds: kinds.to_vec(),
            agent_active: agent,
        }
    }

    #[test]
    fn the_sentence_names_what_is_asked() {
        use CapabilityKind::*;
        assert_eq!(what(&[Camera]), "your camera");
        assert_eq!(what(&[Camera, Microphone]), "your camera and microphone");
        assert_eq!(what(&[Screen]), "your screen");
    }

    #[test]
    fn always_is_offered_only_for_the_camera_and_microphone_with_no_agent_working() {
        use CapabilityKind::*;
        assert!(may_offer_always(&prompt(&[Camera, Microphone], false)));
        assert!(
            !may_offer_always(&prompt(&[Camera], true)),
            "the agent is working"
        );
        assert!(
            !may_offer_always(&prompt(&[Screen], false)),
            "the screen is asked every time"
        );
    }

    #[test]
    fn a_press_cannot_ask_for_more_than_the_card_offered() {
        use CapabilityKind::*;
        // "Always allow" for the screen, or while the agent works, becomes a one-time allow.
        assert_eq!(
            choice(&prompt(&[Screen], false), true, true),
            PermissionChoice::Allow { remember: false }
        );
        assert_eq!(
            choice(&prompt(&[Camera], true), true, true),
            PermissionChoice::Allow { remember: false }
        );
        assert_eq!(
            choice(&prompt(&[Camera], false), true, true),
            PermissionChoice::Allow { remember: true }
        );
        // A block may always be kept.
        assert_eq!(
            choice(&prompt(&[Screen], true), false, true),
            PermissionChoice::Block { remember: true }
        );
    }

    #[test]
    fn a_request_shows_a_card_and_blocks_the_page_until_it_is_answered() {
        let mut state = FerriteBrowser::default();
        assert!(overlay(&state).is_none());
        assert!(bar(&state).is_none());
        state.tab_diag[0].permission = Some(prompt(&[CapabilityKind::Camera], false));
        assert!(overlay(&state).is_some());
        assert!(
            crate::page_input_blocked(&state),
            "the page gets no input under the card"
        );
        // Answering takes the request out, once.
        let _ = update(
            &mut state,
            Msg::Answer(PermissionChoice::Block { remember: false }),
        );
        assert!(state.tab_diag[0].permission.is_none());
        assert!(overlay(&state).is_none());
        assert!(!crate::page_input_blocked(&state));
    }

    #[test]
    fn a_capturing_page_always_shows_the_bar_and_stopping_clears_it() {
        let mut state = FerriteBrowser::default();
        state.tab_diag[0].capture = (false, true, false);
        assert!(bar(&state).is_some());
        let _ = update(&mut state, Msg::StopSharing);
        assert_eq!(state.tab_diag[0].capture, (false, false, false));
        assert!(bar(&state).is_none());
    }

    /// The defense rule: the agent does not act on a page whose request for the camera,
    /// microphone or screen is waiting on the person; the step is held and goes on only
    /// when there is no request.
    #[test]
    fn an_agent_step_waits_for_the_person_to_answer() {
        use ferrite_agent::browser_loop::AgentAction;
        let mut state = FerriteBrowser {
            run_id: 7,
            ..FerriteBrowser::default()
        };
        state.tab_diag[0].permission = Some(prompt(&[CapabilityKind::Camera], true));
        let _ = crate::handle_agent_step(&mut state, 7, Ok(AgentAction::GoBack), None);
        assert!(
            matches!(
                state.deferred_agent_step,
                Some(FerriteBrowserMessage::AgentStepReady { run_id: 7, .. })
            ),
            "the step is held"
        );
        // Still asking: nothing is released.
        let _ = crate::tab_diag::drain_all(&mut state);
        assert!(state.deferred_agent_step.is_some());
        // Answered: it goes on.
        let _ = update(
            &mut state,
            Msg::Answer(PermissionChoice::Block { remember: false }),
        );
        let _ = crate::tab_diag::drain_all(&mut state);
        assert!(state.deferred_agent_step.is_none());
    }
}
