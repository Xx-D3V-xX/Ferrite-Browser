//! What an [`AgentAction`] does, in the comparator's vocabulary: the
//! `(primitive, origin)` effects the runtime guard classifies before the action
//! runs (ADR-014), and the fixed text the agent is told when one is refused.
//!
//! This is the engine-agnostic half of the live guard. The decision itself is
//! `ferrite_ipi::comparator::RuntimeGuard`; this module only knows the agent's
//! action vocabulary, so any caller that runs [`crate::browser_loop`] (the
//! evaluation runner, a headless harness) can enforce a prediction exactly the way
//! the app does.
//!
//! The app's own copy of this logic lives in `ferrite-ui`'s `runtime_guard`
//! module (it predates this one and is owned by a different change); the two must
//! agree, and `docs/TO-DO.md` tracks moving the app onto this module.

use ferrite_core::Primitive;
use ferrite_engine::PageDigest;

use crate::browser_loop::AgentAction;

/// What the agent sees when the guard refuses an action. Deliberately fixed text
/// with nothing from the page or from the action in it: an observation goes back
/// to the model, and anything attacker-chosen echoed there is a prompt-injection
/// channel.
pub const BLOCKED_OBSERVATION: &str =
    "blocked: this action is outside what the user's task was expected to need, so Ferrite did not \
     run it. Continue the task without it, or finish and tell the user what you could not do.";

/// The primitive an action realizes, or `None` for the two terminal actions
/// ([`AgentAction::Finish`], [`AgentAction::AskUser`]), which never reach an
/// engine.
///
/// The mapping is the same conservative one `ferrite_engine::Call::primitive`
/// uses: hover, set-checked and submit are never weaker than a click.
#[must_use]
pub fn primitive_of_action(action: &AgentAction) -> Option<Primitive> {
    Some(match action {
        AgentAction::Navigate { .. }
        | AgentAction::GoBack
        | AgentAction::GoForward
        | AgentAction::Reload
        | AgentAction::SwitchTab { .. } => Primitive::Navigate,
        AgentAction::ReadDom
        | AgentAction::ReadPage
        | AgentAction::ReadText { .. }
        | AgentAction::FindText { .. }
        | AgentAction::ExtractLinks { .. }
        | AgentAction::ListTabs => Primitive::DomRead,
        AgentAction::Query { .. } => Primitive::DomQuery,
        AgentAction::Click { .. }
        | AgentAction::Hover { .. }
        | AgentAction::SetChecked { .. }
        | AgentAction::SubmitForm { .. } => Primitive::Click,
        // As `ferrite_engine::Call::primitive`: Enter or Space is a click.
        AgentAction::PressKey { key, .. } if ferrite_engine::key_activates(key) => Primitive::Click,
        AgentAction::TypeText { .. }
        | AgentAction::SelectOption { .. }
        | AgentAction::PressKey { .. } => Primitive::DomWrite,
        AgentAction::FillForm { .. } => Primitive::FormFill,
        AgentAction::Scroll { .. } | AgentAction::ScrollTo { .. } => Primitive::Scroll,
        AgentAction::WaitForSelector { .. }
        | AgentAction::WaitIdle
        | AgentAction::WaitMs { .. } => Primitive::Wait,
        AgentAction::OpenTab { .. } => Primitive::TabOpen,
        AgentAction::CloseTab { .. } => Primitive::TabClose,
        AgentAction::Screenshot => Primitive::Screenshot,
        AgentAction::Download { .. } => Primitive::Download,
        AgentAction::ClipboardRead => Primitive::ClipboardRead,
        AgentAction::ClipboardWrite { .. } => Primitive::ClipboardWrite,
        AgentAction::JsExecute { .. } => Primitive::JsExecute,
        AgentAction::Finish { .. } | AgentAction::AskUser { .. } => return None,
    })
}

/// The URL an action carries, when it has one: `navigate`, `download`, and
/// `open_tab` with a URL.
#[must_use]
pub fn action_url(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::Navigate { url } | AgentAction::Download { url } => Some(url.as_str()),
        // A tab opened at a rejected origin is a navigation to it.
        AgentAction::OpenTab { url: Some(url) } => Some(url.as_str()),
        _ => None,
    }
}

/// The origin of an http(s) URL in the comparator's normal form
/// (`scheme://host[:port]`, lower-cased, default port dropped), or `None`.
#[must_use]
pub fn origin_of_url(url: &str) -> Option<String> {
    ferrite_core::Origin::parse(url)
        .ok()
        .map(|o| o.as_str().to_string())
}

/// The origin of a URL as the guard sees it: [`origin_of_url`], or just
/// `scheme:` for a URL with no host (`data:`, `javascript:`, `about:`), which no
/// scope can admit. A URL with no scheme at all (`www.example.com`) has no
/// origin: it is a deviation, never silently assumed to be `https`.
#[must_use]
pub fn effect_origin(url: &str) -> Option<String> {
    origin_of_url(url).or_else(|| {
        let scheme: String = url
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            .take(20)
            .collect();
        let has_colon = url.trim().chars().nth(scheme.chars().count()) == Some(':');
        (has_colon && !scheme.is_empty()).then(|| format!("{}:", scheme.to_ascii_lowercase()))
    })
}

/// The `(primitive, origin)` effects of `action`, given the URL of the active tab
/// and (for a click by `@ref`) the page digest. Empty for the terminal actions.
///
/// * An action carrying a URL (`navigate`, `open_tab`, `download`) acts at that
///   URL's origin.
/// * Every other action acts at the active tab's origin.
/// * A click on a link whose destination is another origin is **also** a
///   navigation to that origin: following a link is the most ordinary way to
///   leave the site a task was scoped to, and it needs no `navigate` action. Only
///   `@ref` selectors can be resolved to a link; a CSS selector is checked at the
///   current origin only (a stated limit).
#[must_use]
pub fn action_effects(
    action: &AgentAction,
    active_tab_url: &str,
    digest: Option<&PageDigest>,
) -> Vec<(Primitive, Option<String>)> {
    let Some(primitive) = primitive_of_action(action) else {
        return Vec::new();
    };
    if let Some(url) = action_url(action) {
        return vec![(primitive, effect_origin(url))];
    }
    let here = origin_of_url(active_tab_url);
    let mut effects = vec![(primitive, here.clone())];
    if let (AgentAction::Click { selector }, Some(digest)) = (action, digest) {
        let link_origin = selector
            .strip_prefix('@')
            .and_then(|n| n.parse::<u32>().ok())
            .and_then(|n| digest.element(n))
            .filter(|e| e.role == "link")
            .and_then(|e| e.href.as_deref())
            .and_then(effect_origin);
        if let Some(there) = link_origin {
            if Some(&there) != here.as_ref() {
                effects.push((Primitive::Navigate, Some(there)));
            }
        }
    }
    effects
}

#[cfg(test)]
mod tests {
    use ferrite_engine::DigestElement;

    use super::*;

    const HERE: &str = "https://shop.example/cart?x=1";

    fn page_with_link(href: &str) -> PageDigest {
        PageDigest {
            url: "https://shop.example/".into(),
            elements: vec![DigestElement {
                ref_id: 7,
                role: "link".into(),
                label: "Next".into(),
                href: Some(href.into()),
                ..DigestElement::default()
            }],
            ..PageDigest::default()
        }
    }

    #[test]
    fn a_url_action_acts_at_the_urls_origin_not_the_tabs() {
        for action in [
            AgentAction::Navigate {
                url: "https://Other.example:8443/a?b#c".into(),
            },
            AgentAction::Download {
                url: "https://Other.example:8443/f.zip".into(),
            },
            AgentAction::OpenTab {
                url: Some("https://Other.example:8443/".into()),
            },
        ] {
            let effects = action_effects(&action, HERE, None);
            assert_eq!(effects.len(), 1);
            assert_eq!(
                effects[0].1.as_deref(),
                Some("https://other.example:8443"),
                "{action:?}"
            );
        }
    }

    #[test]
    fn urls_with_no_host_become_a_bare_scheme_that_no_scope_admits() {
        for (url, want) in [
            ("javascript:alert(1)", "javascript:"),
            ("JavaScript:alert(1)", "javascript:"),
            ("data:text/html,<script>1</script>", "data:"),
            ("about:blank", "about:"),
        ] {
            assert_eq!(effect_origin(url).as_deref(), Some(want), "{url}");
        }
    }

    #[test]
    fn a_url_with_no_scheme_has_no_origin_and_is_never_assumed_to_be_https() {
        // Fail closed: `None` is itself a deviation in the comparator.
        for url in ["www.example.com", "example.com/path", "//example.com", ""] {
            assert_eq!(effect_origin(url), None, "{url:?}");
        }
    }

    #[test]
    fn other_actions_act_at_the_active_tabs_origin() {
        let effects = action_effects(
            &AgentAction::Click {
                selector: "#buy".into(),
            },
            HERE,
            None,
        );
        assert_eq!(
            effects,
            vec![(Primitive::Click, Some("https://shop.example".to_string()))]
        );
    }

    #[test]
    fn a_click_on_a_cross_origin_link_is_also_a_navigation_there() {
        let digest = page_with_link("https://evil.example/landing");
        let effects = action_effects(
            &AgentAction::Click {
                selector: "@7".into(),
            },
            HERE,
            Some(&digest),
        );
        assert_eq!(
            effects,
            vec![
                (Primitive::Click, Some("https://shop.example".to_string())),
                (
                    Primitive::Navigate,
                    Some("https://evil.example".to_string())
                ),
            ]
        );
        // A same-origin link, an unknown ref and a CSS selector add nothing.
        for (action, digest) in [
            (
                AgentAction::Click {
                    selector: "@7".into(),
                },
                page_with_link("https://shop.example/other"),
            ),
            (
                AgentAction::Click {
                    selector: "@99".into(),
                },
                page_with_link("https://evil.example/"),
            ),
            (
                AgentAction::Click {
                    selector: "a.next".into(),
                },
                page_with_link("https://evil.example/"),
            ),
        ] {
            assert_eq!(action_effects(&action, HERE, Some(&digest)).len(), 1);
        }
    }

    #[test]
    fn terminal_actions_have_no_effects_and_every_other_action_has_a_primitive() {
        for action in [
            AgentAction::Finish { answer: "x".into() },
            AgentAction::AskUser {
                question: "x".into(),
            },
        ] {
            assert_eq!(primitive_of_action(&action), None);
            assert!(action_effects(&action, HERE, None).is_empty());
        }
        let js = AgentAction::JsExecute { script: "1".into() };
        assert_eq!(primitive_of_action(&js), Some(Primitive::JsExecute));
        assert_eq!(
            primitive_of_action(&AgentAction::FillForm { fields: vec![] }),
            Some(Primitive::FormFill)
        );
        assert_eq!(
            primitive_of_action(&AgentAction::SubmitForm { selector: None }),
            Some(Primitive::Click),
            "a submission is never weaker than a click"
        );
    }

    #[test]
    fn the_refusal_text_carries_nothing_from_the_page_or_the_action() {
        assert!(BLOCKED_OBSERVATION.starts_with("blocked:"));
        for needle in ["http", "@", "<", "{"] {
            assert!(!BLOCKED_OBSERVATION.contains(needle), "{needle}");
        }
    }
}
