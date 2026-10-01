//! Tests for the chrome added in the UI overhaul: the overflow menu, tab
//! switching by key, the keyboard shortcuts, queued scroll and pointer input,
//! the engine tick's idle rate, and Escape as the safe answer to a question.

use iced::keyboard::{key::Named, Key, Modifiers};

use ferrite_core::scope::OriginScope;
use ferrite_core::{Capability, ExpectedCapability, ExpectedCapabilitySet, Origin};
use ferrite_ipi::comparator::RuntimeGuard;

use super::*;
use crate::chrome::MenuCommand;
use crate::scroll::Wheel;

/// The platform's shortcut modifier, as `handle_key_press` reads it.
fn cmd() -> Modifiers {
    #[cfg(target_os = "macos")]
    return Modifiers::LOGO;
    #[cfg(not(target_os = "macos"))]
    return Modifiers::CTRL;
}

fn press(c: &str, mods: Modifiers) -> Option<FerriteBrowserMessage> {
    handle_key_press(Key::Character(c.into()), mods)
}

fn with_tabs(n: usize) -> FerriteBrowser {
    let mut state = FerriteBrowser::default();
    for _ in 1..n {
        push_tab_state(&mut state);
    }
    state.active_tab = 0;
    state
}

// ── Overflow menu ───────────────────────────────────────────────────────

#[test]
fn the_menu_toggles_and_closes() {
    let mut state = FerriteBrowser::default();
    assert!(!state.show_menu);
    let _ = update(&mut state, FerriteBrowserMessage::ToggleMenu);
    assert!(state.show_menu);
    let _ = update(&mut state, FerriteBrowserMessage::ToggleMenu);
    assert!(!state.show_menu);
    state.show_menu = true;
    let _ = update(&mut state, FerriteBrowserMessage::CloseMenu);
    assert!(!state.show_menu);
}

#[test]
fn picking_a_menu_row_closes_the_menu_and_does_the_thing() {
    let mut state = FerriteBrowser {
        show_menu: true,
        ..FerriteBrowser::default()
    };
    let _ = update(
        &mut state,
        FerriteBrowserMessage::Menu(MenuCommand::ToggleTheme),
    );
    assert!(!state.show_menu);
    assert_eq!(state.theme_mode, AppTheme::Light);
}

#[test]
fn the_menu_opens_each_library_tab_and_closes_the_other_drawers() {
    for tab in [
        LibraryTab::Bookmarks,
        LibraryTab::History,
        LibraryTab::Downloads,
    ] {
        let mut state = FerriteBrowser {
            show_menu: true,
            show_agent_sidebar: true,
            show_audit_panel: true,
            ..FerriteBrowser::default()
        };
        let _ = update(
            &mut state,
            FerriteBrowserMessage::Menu(MenuCommand::Library(tab)),
        );
        assert!(state.show_library_panel, "{tab:?}");
        assert_eq!(state.library_tab, tab);
        assert!(!state.show_agent_sidebar && !state.show_audit_panel && !state.show_menu);
    }
}

#[test]
fn opening_the_library_again_switches_tab_without_closing_it() {
    let mut state = FerriteBrowser::default();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::OpenLibrary(LibraryTab::History),
    );
    let _ = update(
        &mut state,
        FerriteBrowserMessage::OpenLibrary(LibraryTab::Downloads),
    );
    assert!(state.show_library_panel);
    assert_eq!(state.library_tab, LibraryTab::Downloads);
}

#[test]
fn escape_closes_the_menu_before_anything_else() {
    let mut state = FerriteBrowser {
        show_menu: true,
        show_find_bar: true,
        is_loading: true,
        ..FerriteBrowser::default()
    };
    let _ = update(&mut state, FerriteBrowserMessage::EscapePressed);
    assert!(!state.show_menu);
    assert!(state.show_find_bar, "the find bar is the next Escape's");
    assert!(state.is_loading);
}

// ── Tabs by key ─────────────────────────────────────────────────────────

#[test]
fn next_and_previous_tab_wrap_around() {
    let mut state = with_tabs(3);
    let _ = update(&mut state, FerriteBrowserMessage::NextTab);
    assert_eq!(state.active_tab, 1);
    let _ = update(&mut state, FerriteBrowserMessage::NextTab);
    let _ = update(&mut state, FerriteBrowserMessage::NextTab);
    assert_eq!(state.active_tab, 0, "wraps forward");
    let _ = update(&mut state, FerriteBrowserMessage::PrevTab);
    assert_eq!(state.active_tab, 2, "wraps backward");
}

#[test]
fn with_one_tab_next_and_previous_stay_put() {
    let mut state = with_tabs(1);
    let _ = update(&mut state, FerriteBrowserMessage::NextTab);
    let _ = update(&mut state, FerriteBrowserMessage::PrevTab);
    assert_eq!(state.active_tab, 0);
}

#[test]
fn tab_by_number_and_last_tab() {
    let mut state = with_tabs(4);
    let _ = update(&mut state, FerriteBrowserMessage::SelectTabNumber(2));
    assert_eq!(state.active_tab, 2);
    let _ = update(&mut state, FerriteBrowserMessage::SelectTabNumber(7));
    assert_eq!(
        state.active_tab, 2,
        "a tab that is not there changes nothing"
    );
    let _ = update(&mut state, FerriteBrowserMessage::SelectLastTab);
    assert_eq!(state.active_tab, 3);
}

#[test]
fn a_new_tab_starts_in_the_address_bar() {
    let mut state = FerriteBrowser::default();
    let _ = update(&mut state, FerriteBrowserMessage::AddTab);
    assert_eq!(state.active_tab, 1);
    assert!(state.address_bar_focused);
}

// ── Shortcuts ───────────────────────────────────────────────────────────

#[test]
fn browser_shortcuts_map_to_their_messages() {
    use FerriteBrowserMessage as M;
    assert!(matches!(press("t", cmd()), Some(M::AddTab)));
    assert!(matches!(press("w", cmd()), Some(M::CloseActiveTab)));
    assert!(matches!(press("l", cmd()), Some(M::FocusAddressBar)));
    assert!(matches!(press("r", cmd()), Some(M::Reload)));
    assert!(matches!(
        press("d", cmd()),
        Some(M::ToggleBookmarkCurrentPage)
    ));
    assert!(matches!(press("=", cmd()), Some(M::ZoomIn)));
    assert!(matches!(press("-", cmd()), Some(M::ZoomOut)));
    assert!(matches!(press("0", cmd()), Some(M::ZoomReset)));
    assert!(matches!(press(",", cmd()), Some(M::ToggleSettingsPanel)));
    assert!(matches!(press("[", cmd()), Some(M::GoBack)));
    assert!(matches!(press("]", cmd()), Some(M::GoForward)));
}

#[test]
fn the_agent_toggle_needs_shift_so_plain_a_is_left_alone() {
    use FerriteBrowserMessage as M;
    assert!(matches!(
        press("A", cmd() | Modifiers::SHIFT),
        Some(M::ToggleAgentSidebar)
    ));
    assert!(press("a", cmd()).is_none(), "Cmd/Ctrl+A is select-all");
}

#[test]
fn number_keys_pick_tabs_and_nine_is_the_last() {
    for n in 1..=8usize {
        let msg = press(&n.to_string(), cmd());
        assert!(
            matches!(msg, Some(FerriteBrowserMessage::SelectTabNumber(i)) if i == n - 1),
            "{n}: {msg:?}"
        );
    }
    assert!(matches!(
        press("9", cmd()),
        Some(FerriteBrowserMessage::SelectLastTab)
    ));
}

#[test]
fn bare_keys_are_never_shortcuts() {
    for c in ["t", "w", "d", "1", "[", ","] {
        assert!(press(c, Modifiers::empty()).is_none(), "{c}");
    }
}

#[test]
fn braces_and_ctrl_tab_step_between_tabs() {
    use FerriteBrowserMessage as M;
    assert!(matches!(
        press("}", cmd() | Modifiers::SHIFT),
        Some(M::NextTab)
    ));
    assert!(matches!(
        press("{", cmd() | Modifiers::SHIFT),
        Some(M::PrevTab)
    ));
    assert!(matches!(
        handle_key_press(Key::Named(Named::Tab), Modifiers::CTRL),
        Some(M::NextTab)
    ));
    assert!(matches!(
        handle_key_press(Key::Named(Named::Tab), Modifiers::CTRL | Modifiers::SHIFT),
        Some(M::PrevTab)
    ));
    assert!(
        handle_key_press(Key::Named(Named::Tab), Modifiers::empty()).is_none(),
        "plain Tab belongs to the page"
    );
}

#[test]
fn a_focused_field_does_not_swallow_the_new_shortcuts() {
    // `page_key_from_event` honours these even when a text field captured the
    // key (the address bar has focus right after typing an address).
    assert!(is_captured_chrome_shortcut(
        &Key::Character("d".into()),
        cmd()
    ));
    assert!(is_captured_chrome_shortcut(
        &Key::Named(Named::Tab),
        Modifiers::CTRL
    ));
    assert!(!is_captured_chrome_shortcut(
        &Key::Named(Named::Tab),
        Modifiers::empty()
    ));
}

// ── Scroll and pointer input ────────────────────────────────────────────

#[test]
fn wheel_input_is_queued_then_drained_one_frame_at_a_time() {
    let mut state = FerriteBrowser::default();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::ServoScroll(Wheel::Lines { x: 0.0, y: -1.0 }),
    );
    assert!(state.scroll_queue.is_pending());
    let mut ticks = 0;
    while state.scroll_queue.is_pending() {
        let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);
        ticks += 1;
        assert!(ticks < 100, "a notch must finish");
    }
    assert!(ticks > 3, "a notch is spread over frames, took {ticks}");
}

#[test]
fn pointer_moves_are_recorded_and_forwarded_once_per_tick() {
    let mut state = FerriteBrowser::default();
    for i in 0..50 {
        let _ = update(
            &mut state,
            FerriteBrowserMessage::ServoMouseMove {
                x: i as f32,
                y: 2.0,
            },
        );
    }
    assert_eq!(state.cursor_pos, (49.0, 2.0), "the latest position wins");
    assert!(state.pointer_moved);
    let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);
    assert!(!state.pointer_moved, "one tick forwards them all");
}

#[test]
fn switching_tabs_drops_scroll_meant_for_the_old_page() {
    let mut state = with_tabs(2);
    let _ = update(
        &mut state,
        FerriteBrowserMessage::ServoScroll(Wheel::Pixels { x: 0.0, y: 40.0 }),
    );
    let _ = update(&mut state, FerriteBrowserMessage::SelectTab(1));
    assert!(!state.scroll_queue.is_pending());
}

// ── The engine tick ─────────────────────────────────────────────────────

/// A browser whose content area and engine buffer agree, so only the things
/// under test make it "active".
fn settled() -> FerriteBrowser {
    let state = FerriteBrowser::default();
    let logical = state.content_area_size.get();
    let scale = state.scale_factor;
    FerriteBrowser {
        last_resized_content_px: (
            (logical.width * scale).round().max(1.0) as u32,
            (logical.height * scale).round().max(1.0) as u32,
        ),
        ..state
    }
}

#[test]
fn a_page_sitting_still_ticks_slowly() {
    assert_eq!(tick_interval(&settled()), IDLE_TICK);
}

#[test]
fn loading_input_and_new_pictures_keep_the_tick_fast() {
    let mut state = settled();
    state.is_loading = true;
    assert_eq!(tick_interval(&state), ACTIVE_TICK);

    let mut state = settled();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::ServoScroll(Wheel::Pixels { x: 0.0, y: 3.0 }),
    );
    assert_eq!(tick_interval(&state), ACTIVE_TICK);

    let mut state = settled();
    wake(&mut state);
    assert_eq!(tick_interval(&state), ACTIVE_TICK);
}

#[test]
fn the_tick_slows_down_after_the_page_goes_quiet() {
    let mut state = settled();
    wake(&mut state);
    for _ in 0..BUSY_TICKS {
        assert_eq!(tick_interval(&state), ACTIVE_TICK);
        let _ = update(&mut state, FerriteBrowserMessage::ServoFrame);
    }
    assert_eq!(tick_interval(&state), IDLE_TICK);
}

#[test]
fn a_running_agent_keeps_the_tick_fast() {
    let mut state = settled();
    state.agent_is_running = true;
    assert_eq!(tick_interval(&state), ACTIVE_TICK);
}

// ── Escape answers questions safely ─────────────────────────────────────

fn click_task() -> ExpectedFingerprint {
    let scope = || {
        OriginScope::exact([Origin::parse("https://news.example").expect("origin")])
            .expect("non-empty")
    };
    ExpectedFingerprint::from_capabilities(
        ExpectedCapabilitySet::new([ExpectedCapability::new(Capability::WebNavigate, scope())])
            .expect("distinct"),
    )
}

#[tokio::test]
async fn escape_on_the_agents_question_is_no() {
    let live = LiveAgentLoop::new(
        "go".into(),
        "go".into(),
        Default::default(),
        Default::default(),
    )
    .with_guard(Some(RuntimeGuard::new(click_task())));
    let mut state = FerriteBrowser {
        run_id: 1,
        live_loop: Some(live),
        tab_urls: vec!["https://news.example/".into()],
        ..FerriteBrowser::default()
    };
    let _ = update(
        &mut state,
        FerriteBrowserMessage::AgentStepReady {
            run_id: 1,
            action: Ok(AgentAction::Click {
                selector: "@4".into(),
            }),
        },
    );
    assert!(state.pending_runtime.is_some(), "the person is asked");
    let _ = update(&mut state, FerriteBrowserMessage::EscapePressed);
    assert!(state.pending_runtime.is_none(), "Escape answered it");
    // And "no" means the click did not run: it is recorded as blocked.
    assert!(
        state
            .agent_log
            .iter()
            .any(|e| matches!(e, AgentLogEntry::Step { blocked: true, .. })),
        "{:?}",
        state.agent_log
    );
}

#[test]
fn escape_on_the_consent_panel_cancels_it() {
    let mut state = FerriteBrowser::default();
    let mut diff = FingerprintDiff::default();
    diff.extra_primitives.insert(ToolId::new("js.execute"));
    state.pending_diff = Some(diff);
    state.pending_task = Some("t".into());
    let _ = update(&mut state, FerriteBrowserMessage::EscapePressed);
    assert!(state.pending_diff.is_none());
    assert!(state.pending_task.is_none());
}

#[test]
fn escape_does_not_approve_anything() {
    // The safe default is the whole point: Escape must never leave a decision
    // that lets something run.
    let mut state = FerriteBrowser::default();
    let mut diff = FingerprintDiff::default();
    diff.extra_primitives.insert(ToolId::new("js.execute"));
    state.pending_diff = Some(diff);
    let _ = update(&mut state, FerriteBrowserMessage::EscapePressed);
    assert!(state.pending_decision.approved.is_empty());
}

// ── Address bar focus ───────────────────────────────────────────────────

#[test]
fn the_first_press_focuses_the_address_bar_and_clearing_drops_it() {
    let mut state = FerriteBrowser::default();
    assert!(!state.address_bar_focused);
    let _ = update(&mut state, FerriteBrowserMessage::AddressBarPressed);
    assert!(state.address_bar_focused);
    let _ = update(&mut state, FerriteBrowserMessage::ClearAddressBarFocus);
    assert!(!state.address_bar_focused);
}

#[test]
fn navigating_lets_go_of_the_address_bar() {
    let mut state = FerriteBrowser {
        address_bar_focused: true,
        ..FerriteBrowser::default()
    };
    let _ = update(
        &mut state,
        FerriteBrowserMessage::NavigateRequested("example.com".into()),
    );
    assert!(!state.address_bar_focused);
}

// ── Window ──────────────────────────────────────────────────────────────

#[test]
fn the_window_title_follows_the_active_tab() {
    let mut state = FerriteBrowser::default();
    assert_eq!(window_title(&state), "Ferrite");
    state.tab_urls[0] = "https://rust-lang.org/".into();
    state.tab_titles[0] = "Rust".into();
    assert_eq!(window_title(&state), "Rust \u{2014} Ferrite");
}

#[test]
fn the_window_keeps_a_minimum_size_that_fits_the_toolbar() {
    let settings = window_settings();
    let min = settings.min_size.expect("a minimum size");
    assert!(min.width >= 600.0 && min.height >= 360.0);
}

#[cfg(target_os = "macos")]
#[test]
fn on_macos_the_title_bar_is_transparent_and_content_runs_under_it() {
    let p = window_settings().platform_specific;
    assert!(p.titlebar_transparent && p.title_hidden && p.fullsize_content_view);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn off_macos_the_strip_has_no_traffic_light_inset() {
    assert_eq!(chrome::TRAFFIC_LIGHT_INSET, 0.0);
}

#[test]
fn building_the_view_never_panics_across_chrome_states() {
    // The widget tree is built from state alone; a state that trips an index or
    // a layout assumption fails here rather than on a person's screen.
    let mut state = with_tabs(30);
    for flag in 0..8u32 {
        state.show_menu = flag & 1 != 0;
        state.is_loading = flag & 2 != 0;
        state.show_find_bar = flag & 4 != 0;
        let _ = view(&state);
        state.theme_mode = state.theme_mode.toggled();
    }
    state.tab_titles[3] = "x".repeat(500);
    state.tab_zoom[0] = 1.75;
    state.show_library_panel = true;
    let _ = view(&state);
}

#[test]
fn the_last_tab_cannot_be_closed() {
    let mut state = with_tabs(1);
    let _ = update(&mut state, FerriteBrowserMessage::CloseTab(0));
    assert_eq!(state.tabs.len(), 1);
}
