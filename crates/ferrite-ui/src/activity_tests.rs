//! Tests for the Audit panel's model-activity view state, and the address bar
//! showing nothing for the blank page.

use ferrite_model::trace::{global, TraceBackend, TraceEvent};

use super::*;

#[test]
fn the_blank_page_shows_an_empty_address_bar() {
    assert_eq!(address_bar_text("about:blank"), "");
    assert_eq!(
        address_bar_text("https://example.org/"),
        "https://example.org/"
    );
}

#[test]
fn a_blank_page_load_event_leaves_the_address_bar_empty() {
    let mut state = FerriteBrowser::default();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::LoadStatusChanged {
            tab: 0,
            status: "complete".to_string(),
            url: "about:blank".to_string(),
        },
    );
    assert_eq!(state.address_bar_input, "");
}

#[test]
fn a_load_event_from_a_background_tab_does_not_touch_the_address_bar() {
    let mut state = FerriteBrowser::default();
    let _ = update(&mut state, FerriteBrowserMessage::AddTab);
    let _ = update(&mut state, FerriteBrowserMessage::SelectTab(0));
    state.address_bar_input = "https://front.example/".to_string();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::LoadStatusChanged {
            tab: 1,
            status: "complete".to_string(),
            url: "https://back.example/".to_string(),
        },
    );
    assert_eq!(state.address_bar_input, "https://front.example/");
}

#[test]
fn a_click_selects_the_address_until_the_user_starts_typing() {
    let mut state = FerriteBrowser::default();
    assert!(!state.address_bar_edited);
    let _ = update(&mut state, FerriteBrowserMessage::AddressBarPressed);
    assert!(state.address_bar_focused);
    assert!(!state.address_bar_edited, "a click alone is not an edit");

    let _ = update(
        &mut state,
        FerriteBrowserMessage::AddressBarChanged("rust".to_string()),
    );
    assert!(state.address_bar_edited);

    // Submitting shows the resolved address again: back to "unedited".
    let _ = update(
        &mut state,
        FerriteBrowserMessage::NavigateRequested("https://example.org".to_string()),
    );
    assert!(!state.address_bar_edited);
}

#[test]
fn toggling_a_trace_event_expands_then_collapses_it() {
    let mut state = FerriteBrowser::default();
    let _ = update(&mut state, FerriteBrowserMessage::ToggleTraceEvent(4));
    assert_eq!(state.trace_expanded, Some(4));
    let _ = update(&mut state, FerriteBrowserMessage::ToggleTraceEvent(9));
    assert_eq!(state.trace_expanded, Some(9), "only one is open at a time");
    let _ = update(&mut state, FerriteBrowserMessage::ToggleTraceEvent(9));
    assert_eq!(state.trace_expanded, None);
}

#[test]
fn opening_the_audit_panel_loads_the_trace_and_switching_tabs_keeps_it_fresh() {
    let mut event = TraceEvent::new(TraceBackend::Agent, "ui-test marker", "");
    event.request = "marker".to_string();
    global().record(event);

    let mut state = FerriteBrowser::default();
    let _ = update(&mut state, FerriteBrowserMessage::ToggleAuditPanel);
    assert!(state.show_audit_panel);
    assert!(state
        .trace_events
        .iter()
        .any(|e| e.stage == "ui-test marker"));

    let _ = update(
        &mut state,
        FerriteBrowserMessage::SetAuditTab(AuditTab::Security),
    );
    assert_eq!(state.audit_tab, AuditTab::Security);
}

#[test]
fn clearing_the_trace_empties_the_view_and_collapses_any_open_event() {
    let mut state = FerriteBrowser {
        trace_events: vec![TraceEvent::new(TraceBackend::Llm, "x", "m")],
        trace_expanded: Some(0),
        ..FerriteBrowser::default()
    };
    let _ = update(&mut state, FerriteBrowserMessage::ClearTrace);
    assert!(state.trace_events.is_empty());
    assert_eq!(state.trace_expanded, None);
}

#[test]
fn the_activity_panel_renders_for_every_tab_and_with_an_expanded_row() {
    let mut event = TraceEvent::new(TraceBackend::Llm, "agent step", "gemma");
    event.request = "prompt".to_string();
    event.response = "answer".to_string();
    event.latency_ms = 1200;
    event.seq = 7;
    let mut laya = TraceEvent::new(TraceBackend::Laya, "browser step", "v10s");
    laya.seq = 8;
    let mut state = FerriteBrowser {
        show_audit_panel: true,
        trace_events: vec![event, laya],
        trace_expanded: Some(7),
        ..FerriteBrowser::default()
    };
    let _ = view(&state);
    state.audit_tab = AuditTab::Security;
    let _ = view(&state);
}

#[test]
fn a_bare_local_address_gets_http_and_a_bare_site_gets_https() {
    assert_eq!(
        resolve_url("127.0.0.1:8099/form.html"),
        "http://127.0.0.1:8099/form.html"
    );
    assert_eq!(resolve_url("localhost:3000"), "http://localhost:3000");
    assert_eq!(resolve_url("localhost"), "http://localhost");
    assert_eq!(resolve_url("example.org/a"), "https://example.org/a");
    assert_eq!(resolve_url("https://example.org"), "https://example.org");
    assert!(resolve_url("rust async").contains("duckduckgo"));
}
