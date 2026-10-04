//! The notice shown when a page's script thread dies.
//!
//! The engine reports it (`take_crash`); the page itself stays on screen, frozen
//! at its last picture, so the notice is a non-blocking banner over it, not a
//! dialog: *Reload* gives the tab a fresh session and loads the same address
//! again, *Details* shows the reason and backtrace (with Copy), *Dismiss* hides
//! the banner. The crash is also in DevTools' Engine tab and the log file.

use ferrite_servo::diag::CrashNote;
use ferrite_servo::session::HeadlessServoSession;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Element, Length, Task, Theme};

use crate::icons::{icon, Icon};
use crate::tokens::{
    accent_btn_style, close_btn_style, outline_btn_style, rule_card, shadow_popover, SP_MD, SP_SM,
    SP_XS, TEXT_BODY, TEXT_CAPTION, TEXT_SMALL,
};
use crate::{font_weight, FerriteBrowser, FerriteBrowserMessage, Palette};

/// The longest backtrace shown or copied, in characters.
const BACKTRACE_CHARS: usize = 8_000;

/// A crash the person has been told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrashState {
    pub note: CrashNote,
    pub details: bool,
}

impl CrashState {
    pub(crate) fn new(note: CrashNote) -> Self {
        Self {
            note,
            details: false,
        }
    }

    /// Everything about the crash as text, for Copy.
    pub(crate) fn report(&self, url: &str) -> String {
        let mut out = format!(
            "Ferrite page crash\npage: {url}\nreason: {}\n",
            self.note.reason
        );
        if let Some(backtrace) = &self.note.backtrace {
            out.push('\n');
            out.push_str(&crate::truncate(backtrace, BACKTRACE_CHARS));
            out.push('\n');
        }
        out
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Msg {
    Reload,
    ToggleDetails,
    Dismiss,
    Copy,
}

pub(crate) fn update(state: &mut FerriteBrowser, msg: Msg) -> Task<FerriteBrowserMessage> {
    let tab = state.active_tab;
    match msg {
        Msg::Reload => reload(state, tab),
        Msg::ToggleDetails => {
            if let Some(crash) = state.tab_diag.get_mut(tab).and_then(|d| d.crash.as_mut()) {
                crash.details = !crash.details;
            }
        }
        Msg::Dismiss => {
            if let Some(diag) = state.tab_diag.get_mut(tab) {
                diag.crash = None;
            }
        }
        Msg::Copy => {
            let url = state.tab_urls.get(tab).map_or("", String::as_str);
            if let Some(crash) = state.tab_diag.get(tab).and_then(|d| d.crash.as_ref()) {
                return iced::clipboard::write(crash.report(url));
            }
        }
    }
    Task::none()
}

/// Gives tab `tab` a fresh session and loads its address again. The dead
/// session is dropped; if a new one cannot be made the tab shows the error page
/// rather than keeping a session that will never answer.
pub(crate) fn reload(state: &mut FerriteBrowser, tab: usize) {
    let Some(url) = state.tab_urls.get(tab).cloned() else {
        return;
    };
    if let Some(diag) = state.tab_diag.get_mut(tab) {
        diag.crash = None;
        diag.control = None;
    }
    // The new session is made before the old one goes, so the engine (which
    // shuts down with its last session) is never left with none.
    match HeadlessServoSession::new(1280, 700) {
        Ok(session) => {
            if url != "about:blank" && !url.is_empty() {
                session.navigate(&url);
            }
            state.servo_sessions.insert(tab, session);
            state.frame_cache.remove(&tab);
            if let Some(slot) = state.tab_error.get_mut(tab) {
                *slot = None;
            }
            if tab == state.active_tab {
                state.is_loading = true;
            }
            crate::sync_active_webview(state);
            eprintln!(
                "[ferrite-ui] tab {}: restarted the page engine session for {url}",
                tab + 1
            );
        }
        Err(e) => {
            state.servo_sessions.remove(&tab);
            state.frame_cache.remove(&tab);
            if let Some(slot) = state.tab_error.get_mut(tab) {
                *slot = Some(format!("Could not restart this page: {e}"));
            }
            eprintln!(
                "[ferrite-ui] tab {}: could not restart the page: {e}",
                tab + 1
            );
        }
    }
    crate::wake(state);
}

/// The banner for the active tab's crash, to stack over the page.
pub(crate) fn banner<'a>(state: &'a FerriteBrowser) -> Option<Element<'a, FerriteBrowserMessage>> {
    let crash = state
        .tab_diag
        .get(state.active_tab)
        .and_then(|d| d.crash.as_ref())?;
    let palette = state.palette();
    Some(
        container(banner_card(palette, crash))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(SP_MD)
            .align_x(Alignment::Center)
            .into(),
    )
}

fn banner_card<'a>(
    palette: &'static Palette,
    crash: &'a CrashState,
) -> Element<'a, FerriteBrowserMessage> {
    let send = |m: Msg| FerriteBrowserMessage::Crash(m);
    let mut body: Vec<Element<FerriteBrowserMessage>> = vec![row![
        icon(Icon::Warning, 16.0, palette.danger),
        column![
            text("This page stopped responding")
                .size(TEXT_BODY)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
            text("The engine hit an internal error while running it. Reloading starts it again.")
                .size(TEXT_SMALL)
                .color(palette.text_dim)
                .wrapping(text::Wrapping::Word),
        ]
        .spacing(2)
        .width(Length::Fill),
        button(text("Reload").size(TEXT_SMALL))
            .padding([SP_XS + 1.0, SP_MD])
            .style(accent_btn_style)
            .on_press(send(Msg::Reload)),
        button(
            text(if crash.details {
                "Hide details"
            } else {
                "Details"
            })
            .size(TEXT_SMALL)
        )
        .padding([SP_XS + 1.0, SP_MD])
        .style(outline_btn_style)
        .on_press(send(Msg::ToggleDetails)),
        button(icon(Icon::Close, 10.0, palette.text_dim))
            .padding(SP_SM - 2.0)
            .style(close_btn_style)
            .on_press(send(Msg::Dismiss)),
    ]
    .spacing(SP_SM)
    .align_y(Alignment::Center)
    .into()];
    if crash.details {
        let mut detail = crash.note.reason.clone();
        if let Some(backtrace) = &crash.note.backtrace {
            detail.push_str("\n\n");
            detail.push_str(&crate::truncate(backtrace, BACKTRACE_CHARS));
        }
        body.push(
            container(scrollable(
                text(detail)
                    .size(TEXT_CAPTION)
                    .font(iced::Font::MONOSPACE)
                    .color(palette.text)
                    .wrapping(text::Wrapping::WordOrGlyph)
                    .width(Length::Fill),
            ))
            .max_height(180.0)
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
        body.push(
            row![
                Space::with_width(Length::Fill),
                button(
                    row![
                        icon(Icon::Copy, 12.0, palette.text),
                        text("Copy").size(TEXT_SMALL)
                    ]
                    .spacing(SP_XS + 2.0)
                    .align_y(Alignment::Center),
                )
                .padding([SP_XS, SP_MD])
                .style(outline_btn_style)
                .on_press(send(Msg::Copy)),
            ]
            .into(),
        );
    }
    container(rule_card(
        palette,
        palette.danger,
        column(body).spacing(SP_SM).into(),
    ))
    .max_width(760.0)
    .style(|_: &Theme| container::Style {
        shadow: shadow_popover(),
        ..container::Style::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> CrashNote {
        CrashNote {
            at_ms: 1,
            reason: "script thread panicked".to_string(),
            backtrace: Some("0: servo::foo\n1: servo::bar".to_string()),
        }
    }

    fn state_with_crash() -> FerriteBrowser {
        let mut state = FerriteBrowser::default();
        state.tab_urls[0] = "https://a.example/".to_string();
        state.tab_diag[0].crash = Some(CrashState::new(note()));
        state
    }

    #[test]
    fn dismissing_hides_the_banner_and_details_toggle() {
        let mut state = state_with_crash();
        assert!(banner(&state).is_some());
        let _ = update(&mut state, Msg::ToggleDetails);
        assert!(state.tab_diag[0].crash.as_ref().unwrap().details);
        let _ = update(&mut state, Msg::ToggleDetails);
        assert!(!state.tab_diag[0].crash.as_ref().unwrap().details);
        let _ = update(&mut state, Msg::Dismiss);
        assert!(banner(&state).is_none());
    }

    #[test]
    fn the_report_names_the_page_the_reason_and_the_backtrace() {
        let report = CrashState::new(note()).report("https://a.example/");
        assert!(report.contains("page: https://a.example/"));
        assert!(report.contains("reason: script thread panicked"));
        assert!(report.contains("1: servo::bar"));
        let mut huge = note();
        huge.backtrace = Some("x".repeat(50_000));
        assert!(CrashState::new(huge).report("u").chars().count() < BACKTRACE_CHARS + 200);
    }

    #[test]
    fn a_reload_that_cannot_make_a_session_leaves_an_error_page_not_a_dead_tab() {
        // This build has no engine, so a new session cannot be made: exactly
        // the failure path.
        let mut state = state_with_crash();
        let _ = update(&mut state, Msg::Reload);
        assert!(state.tab_diag[0].crash.is_none(), "the banner is gone");
        assert!(state.servo_sessions.is_empty(), "no dead session is kept");
        assert!(state.tab_error[0]
            .as_deref()
            .is_some_and(|e| e.starts_with("Could not restart this page")));
    }

    #[test]
    fn only_the_active_tabs_banner_shows() {
        let mut state = state_with_crash();
        crate::push_tab_state(&mut state);
        assert_eq!(state.active_tab, 1);
        assert!(
            banner(&state).is_none(),
            "tab 0 crashed, tab 1 is being looked at"
        );
        state.active_tab = 0;
        assert!(banner(&state).is_some());
    }
}
