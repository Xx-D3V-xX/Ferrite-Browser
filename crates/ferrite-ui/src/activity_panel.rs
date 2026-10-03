//! The Audit panel's "Model calls" view: a timeline of everything the agent,
//! the LLM and Laya did, with timings, and — one click away — exactly what was
//! sent and what came back.
//!
//! The data is `ferrite_model::trace`'s in-memory ring, copied into
//! `FerriteBrowser::trace_events` once a second while the panel is open (a
//! copy per frame would clone every prompt at 60 Hz).

use ferrite_model::trace::{stats_of, StageStats, TraceBackend, TraceEvent};
use iced::widget::{column, horizontal_space, row, scrollable, text};
use iced::{Element, Font, Length};

use super::*;
use crate::activity::{format_ms, laya_effect_summary};

/// Newest events rendered (older ones stay in the file).
const MAX_ROWS: usize = 150;

/// Which view the Audit panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditTab {
    /// LLM / Laya / agent timeline with timings.
    #[default]
    Models,
    /// The hash-chained network/capability log.
    Security,
}

fn backend_color(palette: &Palette, backend: TraceBackend) -> Color {
    match backend {
        TraceBackend::Llm => palette.accent,
        TraceBackend::Laya => palette.safe,
        TraceBackend::Agent => palette.text_dim,
    }
}

fn clock(at_ms: u64) -> String {
    i64::try_from(at_ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or_else(String::new, |t| {
            t.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
}

/// The panel header: title, the two view tabs and the per-view actions.
pub(crate) fn header<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let tab_button = |label: &'static str, tab: AuditTab| {
        button(text(label).size(11))
            .padding([3, 10])
            .style(if state.audit_tab == tab {
                panel_btn_active
            } else {
                panel_btn_inactive
            })
            .on_press(FerriteBrowserMessage::SetAuditTab(tab))
    };
    let action: Element<FerriteBrowserMessage> = match state.audit_tab {
        AuditTab::Models => button(text("Clear").size(11))
            .padding([3, 10])
            .style(panel_btn_inactive)
            .on_press(FerriteBrowserMessage::ClearTrace)
            .into(),
        AuditTab::Security => button(text("Refresh").size(11))
            .padding([3, 10])
            .style(panel_btn_inactive)
            .on_press(FerriteBrowserMessage::RefreshAuditLog)
            .into(),
    };
    container(
        row![
            text("Activity")
                .size(13)
                .font(font_weight(iced::font::Weight::Semibold))
                .color(palette.text),
            tab_button("Model calls", AuditTab::Models),
            tab_button("Security log", AuditTab::Security),
            horizontal_space(),
            action,
            tip(
                button(icon(Icon::Close, 10.0, palette.text_dim))
                    .padding(5)
                    .style(close_btn_style)
                    .on_press(FerriteBrowserMessage::ToggleAuditPanel),
                "Close (F12)",
                palette,
            ),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .padding([6, PANEL_PADDING]),
    )
    .width(Length::Fill)
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(palette.raised)),
        ..container::Style::default()
    })
    .into()
}

fn stats_table<'a>(
    stats: &[StageStats],
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let cell = |label: String, width: Length, dim: bool| {
        text(label)
            .size(11)
            .color(if dim { palette.text_dim } else { palette.text })
            .width(width)
    };
    let head = row![
        cell("COMPONENT".into(), Length::Fixed(80.0), true),
        cell("STAGE".into(), Length::Fixed(150.0), true),
        cell("CALLS".into(), Length::Fixed(56.0), true),
        cell("AVERAGE".into(), Length::Fixed(80.0), true),
        cell("MEDIAN".into(), Length::Fixed(80.0), true),
        cell("SLOWEST".into(), Length::Fixed(80.0), true),
        cell("FAILED".into(), Length::Fixed(56.0), true),
    ]
    .spacing(8);
    let mut col = column![head].spacing(3);
    for s in stats {
        // Agent entries that are not timed operations (run start, consent)
        // have no latency to show.
        let untimed = s.backend == TraceBackend::Agent && s.max_ms == 0;
        let ms = |v: u64| {
            if untimed {
                "-".to_string()
            } else {
                format_ms(v)
            }
        };
        col = col.push(
            row![
                text(s.backend.label())
                    .size(11)
                    .color(backend_color(palette, s.backend))
                    .width(Length::Fixed(80.0)),
                cell(s.stage.clone(), Length::Fixed(150.0), false),
                cell(s.calls.to_string(), Length::Fixed(56.0), false),
                cell(ms(s.mean_ms), Length::Fixed(80.0), false),
                cell(ms(s.median_ms), Length::Fixed(80.0), false),
                cell(ms(s.max_ms), Length::Fixed(80.0), false),
                cell(s.failures.to_string(), Length::Fixed(56.0), s.failures == 0),
            ]
            .spacing(8),
        );
    }
    col.into()
}

fn block<'a>(
    title: &'static str,
    body: &str,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let copy = button(text("Copy").size(10))
        .padding([1, 7])
        .style(panel_btn_inactive)
        .on_press(FerriteBrowserMessage::CopyAnswer(body.to_string()));
    column![
        row![
            text(title).size(11).color(palette.text_dim),
            horizontal_space(),
            copy
        ]
        .align_y(iced::Alignment::Center),
        container(
            scrollable(
                text(if body.is_empty() {
                    "(empty)".to_string()
                } else {
                    body.to_string()
                })
                .size(11)
                .font(Font::MONOSPACE)
                .color(palette.text)
                .width(Length::Fill),
            )
            .height(Length::Fixed(110.0)),
        )
        .padding(8)
        .width(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette.base)),
            border: Border {
                radius: iced::border::Radius::new(6.0),
                width: 1.0,
                color: palette.divider,
            },
            ..container::Style::default()
        }),
    ]
    .spacing(4)
    .into()
}

fn event_row<'a>(
    event: &'a TraceEvent,
    expanded: bool,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let status = if event.ok {
        text("OK").size(11).color(palette.safe)
    } else {
        text("Failed").size(11).color(palette.danger)
    };
    let summary = row![
        text(clock(event.at_ms))
            .size(11)
            .color(palette.text_dim)
            .width(Length::Fixed(62.0)),
        text(event.backend.label())
            .size(11)
            .color(backend_color(palette, event.backend))
            .width(Length::Fixed(44.0)),
        text(event.stage.clone())
            .size(12)
            .color(palette.text)
            .width(Length::Fixed(140.0)),
        text(if event.model.is_empty() {
            String::new()
        } else {
            truncate(&event.model, 22)
        })
        .size(11)
        .color(palette.text_dim)
        .width(Length::Fixed(150.0)),
        text(
            if event.backend == TraceBackend::Agent && event.latency_ms == 0 {
                String::new()
            } else if event.cached {
                "cached".to_string()
            } else {
                format_ms(event.latency_ms)
            }
        )
        .size(12)
        .color(palette.text)
        .width(Length::Fixed(64.0)),
        container(status).width(Length::Fixed(46.0)),
        text(truncate(&event.note, 70))
            .size(11)
            .color(palette.text_dim)
            .width(Length::Fill),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let header = button(summary)
        .width(Length::Fill)
        .padding([4, PANEL_PADDING])
        .style(move |_: &Theme, s| {
            let hov = matches!(s, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: (hov || expanded).then_some(Background::Color(palette.raised)),
                text_color: palette.text,
                border: Border::default(),
                shadow: iced::Shadow::default(),
            }
        })
        .on_press(FerriteBrowserMessage::ToggleTraceEvent(event.seq));

    if !expanded {
        return header.into();
    }
    let (sent, received) = match event.backend {
        TraceBackend::Agent => ("Details", "Result"),
        _ => ("Sent", "Received"),
    };
    let details: Element<FerriteBrowserMessage> = if event.request.is_empty() {
        block(received, &event.response, palette)
    } else {
        row![
            container(block(sent, &event.request, palette)).width(Length::FillPortion(1)),
            container(block(received, &event.response, palette)).width(Length::FillPortion(1)),
        ]
        .spacing(12)
        .into()
    };
    column![
        header,
        container(details)
            .padding([6, PANEL_PADDING])
            .width(Length::Fill),
    ]
    .into()
}

/// The scrolling body of the "Model calls" view.
pub(crate) fn models_body<'a>(
    state: &'a FerriteBrowser,
    palette: &'static Palette,
) -> Element<'a, FerriteBrowserMessage> {
    let events = &state.trace_events;
    let summary = container(
        text(laya_effect_summary(events))
            .size(12)
            .color(palette.text),
    )
    .padding([8, PANEL_PADDING])
    .width(Length::Fill);

    let mut body = column![summary];
    if !events.is_empty() {
        body = body
            .push(container(stats_table(&stats_of(events), palette)).padding([0, PANEL_PADDING]));
        body = body.push(container(text("")).height(8));
        body = body.push(
            container(
                text("Newest first. Click a row to see what was sent and what came back.")
                    .size(11)
                    .color(palette.text_dim),
            )
            .padding([2, PANEL_PADDING]),
        );
        for event in events.iter().rev().take(MAX_ROWS) {
            body = body.push(event_row(
                event,
                state.trace_expanded == Some(event.seq),
                palette,
            ));
        }
    }
    scrollable(body).height(Length::Fill).into()
}
