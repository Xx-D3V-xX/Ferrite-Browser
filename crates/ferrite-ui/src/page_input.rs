//! Keyboard input for web pages.
//!
//! Mouse events were always forwarded to the active page, but keyboard events
//! never were: only the browser's own shortcuts listened to the keyboard, so a
//! page's text box, search field or form could be clicked into and never typed
//! in. [`page_key_from_iced`] converts an `iced` keyboard event into the plain
//! [`PageKeyEvent`] `ferrite-servo` forwards to the page; it is a pure
//! function so the conversion is testable without a window or a Servo build.

use ferrite_servo::session::{PageKey, PageKeyEvent, PageNamedKey};
use iced::keyboard::{self, key::Named, Key};

fn named(key: Named) -> Option<PageKey> {
    let named = match key {
        Named::Enter => PageNamedKey::Enter,
        Named::Backspace => PageNamedKey::Backspace,
        Named::Delete => PageNamedKey::Delete,
        Named::Tab => PageNamedKey::Tab,
        Named::Escape => PageNamedKey::Escape,
        Named::ArrowUp => PageNamedKey::ArrowUp,
        Named::ArrowDown => PageNamedKey::ArrowDown,
        Named::ArrowLeft => PageNamedKey::ArrowLeft,
        Named::ArrowRight => PageNamedKey::ArrowRight,
        Named::Home => PageNamedKey::Home,
        Named::End => PageNamedKey::End,
        Named::PageUp => PageNamedKey::PageUp,
        Named::PageDown => PageNamedKey::PageDown,
        Named::Insert => PageNamedKey::Insert,
        Named::Space => return Some(PageKey::Character(" ".to_string())),
        Named::F1 => PageNamedKey::F(1),
        Named::F2 => PageNamedKey::F(2),
        Named::F3 => PageNamedKey::F(3),
        Named::F4 => PageNamedKey::F(4),
        Named::F5 => PageNamedKey::F(5),
        Named::F6 => PageNamedKey::F(6),
        Named::F7 => PageNamedKey::F(7),
        Named::F8 => PageNamedKey::F(8),
        Named::F9 => PageNamedKey::F(9),
        Named::F10 => PageNamedKey::F(10),
        Named::F11 => PageNamedKey::F(11),
        Named::F12 => PageNamedKey::F(12),
        // Shift/Control/Alt/Meta/CapsLock and the media/IME keys: a page
        // learns about modifiers from the flags on every other event.
        _ => return None,
    };
    Some(PageKey::Named(named))
}

/// Converts an `iced` keyboard event to the event forwarded to the page, or
/// `None` for events a page has no use for (a bare modifier key, an
/// unidentified key, `ModifiersChanged`).
#[must_use]
pub fn page_key_from_iced(event: &keyboard::Event) -> Option<PageKeyEvent> {
    let (down, key, modified_key, modifiers, text) = match event {
        keyboard::Event::KeyPressed {
            key,
            modified_key,
            modifiers,
            text,
            ..
        } => (true, key, Some(modified_key), *modifiers, text.as_deref()),
        keyboard::Event::KeyReleased { key, modifiers, .. } => (false, key, None, *modifiers, None),
        keyboard::Event::ModifiersChanged(_) => return None,
    };
    let command = modifiers.control() || modifiers.logo();

    let page_key = match key {
        Key::Named(n) => named(*n)?,
        Key::Character(unmodified) => {
            // With Ctrl/Cmd held the shortcut is defined by the *unmodified*
            // key ("v" in Cmd+V); otherwise the typed text wins (it carries
            // Shift, dead-key and Option compositions: "A", "@", "é").
            let typed = if command {
                None
            } else {
                text.filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .or_else(|| match modified_key {
                        Some(Key::Character(c)) => Some(c.to_string()),
                        _ => None,
                    })
            };
            PageKey::Character(typed.unwrap_or_else(|| unmodified.to_string()))
        }
        Key::Unidentified => return None,
    };
    Some(PageKeyEvent {
        down,
        key: page_key,
        shift: modifiers.shift(),
        ctrl: modifiers.control(),
        alt: modifiers.alt(),
        meta: modifiers.logo(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{key::Physical, Location, Modifiers};

    fn pressed(key: Key, modified: Key, mods: Modifiers, text: Option<&str>) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key,
            modified_key: modified,
            physical_key: Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: mods,
            text: text.map(Into::into),
        }
    }

    fn character(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn a_typed_letter_is_forwarded_as_its_text() {
        let e = pressed(
            character("a"),
            character("a"),
            Modifiers::empty(),
            Some("a"),
        );
        let out = page_key_from_iced(&e).expect("a letter is forwarded");
        assert!(out.down);
        assert_eq!(out.key, PageKey::Character("a".into()));
        assert!(!out.shift && !out.ctrl && !out.alt && !out.meta);
    }

    #[test]
    fn shift_produces_the_shifted_text_not_the_base_key() {
        let e = pressed(character("a"), character("A"), Modifiers::SHIFT, Some("A"));
        let out = page_key_from_iced(&e).unwrap();
        assert_eq!(out.key, PageKey::Character("A".into()));
        assert!(out.shift);
    }

    #[test]
    fn text_from_the_os_wins_for_compositions_like_option_keys() {
        let e = pressed(character("e"), character("e"), Modifiers::ALT, Some("é"));
        assert_eq!(
            page_key_from_iced(&e).unwrap().key,
            PageKey::Character("é".into())
        );
    }

    #[test]
    fn the_command_key_uses_the_unmodified_key_so_shortcuts_work() {
        let e = pressed(character("v"), character("v"), Modifiers::LOGO, Some("v"));
        let out = page_key_from_iced(&e).unwrap();
        assert_eq!(out.key, PageKey::Character("v".into()));
        assert!(out.meta);
    }

    #[test]
    fn space_becomes_a_space_character() {
        let e = pressed(
            Key::Named(Named::Space),
            Key::Named(Named::Space),
            Modifiers::empty(),
            Some(" "),
        );
        assert_eq!(
            page_key_from_iced(&e).unwrap().key,
            PageKey::Character(" ".into())
        );
    }

    #[test]
    fn navigation_and_editing_keys_are_named() {
        for (n, want) in [
            (Named::Enter, PageNamedKey::Enter),
            (Named::Backspace, PageNamedKey::Backspace),
            (Named::Tab, PageNamedKey::Tab),
            (Named::ArrowLeft, PageNamedKey::ArrowLeft),
            (Named::F7, PageNamedKey::F(7)),
        ] {
            let e = pressed(Key::Named(n), Key::Named(n), Modifiers::empty(), None);
            assert_eq!(page_key_from_iced(&e).unwrap().key, PageKey::Named(want));
        }
    }

    #[test]
    fn a_bare_modifier_key_and_an_unidentified_key_are_dropped() {
        for k in [
            Key::Named(Named::Shift),
            Key::Named(Named::Control),
            Key::Unidentified,
        ] {
            let e = pressed(k.clone(), k, Modifiers::empty(), None);
            assert_eq!(page_key_from_iced(&e), None);
        }
        assert_eq!(
            page_key_from_iced(&keyboard::Event::ModifiersChanged(Modifiers::SHIFT)),
            None
        );
    }

    #[test]
    fn a_release_is_forwarded_as_key_up() {
        let e = keyboard::Event::KeyReleased {
            key: character("a"),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
        };
        let out = page_key_from_iced(&e).unwrap();
        assert!(!out.down);
        assert_eq!(out.key, PageKey::Character("a".into()));
    }
}
