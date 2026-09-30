//! Tests for forwarding keyboard input to web pages (`page_key_from_event`,
//! `page_key_target`, the `PageKey` handler).

use iced::keyboard::{self, key::Named, key::Physical, Key, Location, Modifiers};

use super::*;

fn key_event(key: Key, mods: Modifiers, text: Option<&str>) -> iced::Event {
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
        location: Location::Standard,
        modifiers: mods,
        text: text.map(Into::into),
    })
}

fn wid() -> window::Id {
    window::Id::unique()
}

#[test]
fn an_uncaptured_typed_letter_is_forwarded_to_the_page() {
    let e = key_event(Key::Character("a".into()), Modifiers::empty(), Some("a"));
    assert!(matches!(
        page_key_from_event(e, iced::event::Status::Ignored, wid()),
        Some(FerriteBrowserMessage::PageKey(_))
    ));
}

#[test]
fn a_key_a_widget_captured_never_reaches_the_page() {
    // A focused text_input (address bar, agent box, find bar, new-tab search)
    // captures what it types — the page must not also receive it.
    let e = key_event(Key::Character("a".into()), Modifiers::empty(), Some("a"));
    assert!(page_key_from_event(e, iced::event::Status::Captured, wid()).is_none());
}

#[test]
fn the_browsers_own_shortcuts_are_not_forwarded() {
    #[cfg(target_os = "macos")]
    let cmd = Modifiers::LOGO;
    #[cfg(not(target_os = "macos"))]
    let cmd = Modifiers::CTRL;
    for e in [
        key_event(Key::Character("t".into()), cmd, Some("t")),
        key_event(Key::Character("l".into()), cmd, Some("l")),
        key_event(Key::Named(Named::F5), Modifiers::empty(), None),
        key_event(Key::Named(Named::Escape), Modifiers::empty(), None),
    ] {
        assert!(
            page_key_from_event(e, iced::event::Status::Ignored, wid()).is_none(),
            "a chrome shortcut must stay with the browser"
        );
    }
}

#[test]
fn enter_and_arrows_are_forwarded() {
    for n in [Named::Enter, Named::ArrowDown, Named::Backspace] {
        let e = key_event(Key::Named(n), Modifiers::empty(), None);
        assert!(page_key_from_event(e, iced::event::Status::Ignored, wid()).is_some());
    }
}

#[test]
fn no_target_while_the_address_bar_or_find_bar_has_the_keyboard() {
    let mut state = FerriteBrowser {
        tab_urls: vec!["https://a.example/".to_string()],
        ..FerriteBrowser::default()
    };
    // No session exists in the automated build (R7), so the only thing left
    // to prove is that each gate returns None rather than panicking.
    assert!(page_key_target(&state).is_none());
    state.address_bar_focused = true;
    assert!(page_key_target(&state).is_none());
    state.address_bar_focused = false;
    state.show_find_bar = true;
    assert!(page_key_target(&state).is_none());
}

#[test]
fn no_target_on_the_new_tab_page() {
    for url in ["about:blank", ""] {
        let state = FerriteBrowser {
            tab_urls: vec![url.to_string()],
            ..FerriteBrowser::default()
        };
        assert!(page_key_target(&state).is_none(), "{url:?}");
    }
}

#[test]
fn a_page_key_with_no_session_is_a_harmless_no_op() {
    let mut state = FerriteBrowser::default();
    let ev = ferrite_servo::session::PageKeyEvent {
        down: true,
        key: ferrite_servo::session::PageKey::Character("a".into()),
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };
    let _ = update(&mut state, FerriteBrowserMessage::PageKey(ev));
}

#[test]
fn clicking_the_page_takes_focus_from_the_address_bar() {
    let mut state = FerriteBrowser {
        address_bar_focused: true,
        ..FerriteBrowser::default()
    };
    let _ = update(&mut state, FerriteBrowserMessage::ServoMousePress);
    assert!(!state.address_bar_focused);
}

#[test]
fn chrome_shortcuts_work_even_when_a_text_field_captured_the_key() {
    // Cmd on macOS, Ctrl elsewhere.
    #[cfg(target_os = "macos")]
    let mods = Modifiers::LOGO;
    #[cfg(not(target_os = "macos"))]
    let mods = Modifiers::CTRL;
    let new_tab = key_event(Key::Character("t".into()), mods, None);
    assert!(matches!(
        page_key_from_event(new_tab, iced::event::Status::Captured, wid()),
        Some(FerriteBrowserMessage::AddTab)
    ));
    let focus_bar = key_event(Key::Character("l".into()), mods, None);
    assert!(matches!(
        page_key_from_event(focus_bar, iced::event::Status::Captured, wid()),
        Some(FerriteBrowserMessage::FocusAddressBar)
    ));
}

#[test]
fn a_captured_editing_combination_is_left_to_the_field() {
    #[cfg(target_os = "macos")]
    let mods = Modifiers::LOGO;
    #[cfg(not(target_os = "macos"))]
    let mods = Modifiers::CTRL;
    for letter in ["a", "c", "v", "x", "z"] {
        let e = key_event(Key::Character(letter.into()), mods, None);
        assert!(
            page_key_from_event(e, iced::event::Status::Captured, wid()).is_none(),
            "{letter}"
        );
    }
    // A plain captured letter is never a shortcut either.
    let plain = key_event(Key::Character("t".into()), Modifiers::empty(), Some("t"));
    assert!(page_key_from_event(plain, iced::event::Status::Captured, wid()).is_none());
}
