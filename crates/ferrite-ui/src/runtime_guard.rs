//! Live enforcement of the predicted fingerprint (ADR-014): turns an agent
//! action into the effects the comparator reasons about, and the verdict into
//! text for the log and for the agent.
//!
//! The decision itself is `ferrite_ipi::comparator::RuntimeGuard`; this module
//! only knows the UI's vocabulary (`AgentAction`, the active tab's URL, the
//! page digest).

use ferrite_core::Primitive;
use ferrite_engine::PageDigest;
use ferrite_ipi::comparator::GuardVerdict;

use super::{action_url, origin_of_url, primitive_of_action, AgentAction};

/// Blocked actions in one run after which the run stops instead of letting the
/// agent keep probing: an agent that keeps trying things the task never needed
/// is, at best, confused.
pub(crate) const MAX_GUARD_BLOCKS: u32 = 4;

/// What the agent sees when an action is blocked. Deliberately fixed text with
/// nothing from the page or from the action in it: an observation goes back to
/// the model, and anything attacker-chosen echoed there is a prompt-injection
/// channel.
pub(crate) const BLOCKED_OBSERVATION: &str =
    "blocked: this action is outside what the user's task was expected to need, so Ferrite did not \
     run it. Continue the task without it, or finish and tell the user what you could not do.";

/// The origin of a URL as the guard sees it: `scheme://host[:port]`, or just
/// `scheme:` for a URL with no host (`data:`, `javascript:`, `about:`), which
/// no scope can admit.
fn effect_origin(url: &str) -> Option<String> {
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

/// The `(primitive, origin)` effects of `action`, given the URL of the active
/// tab and (for a click by `@ref`) the page digest.
///
/// * An action carrying a URL (`navigate`, `open_tab`, `download`) acts at that
///   URL's origin.
/// * Every other action acts at the active tab's origin.
/// * A click on a link whose destination is another origin is **also** a
///   navigation to that origin: following a link is the most ordinary way to
///   leave the site a task was scoped to, and it needs no `navigate` action.
///   Only `@ref` selectors can be resolved to a link; a CSS selector is checked
///   at the current origin only (a stated limit).
pub(crate) fn action_effects(
    action: &AgentAction,
    active_tab_url: &str,
    digest: Option<&PageDigest>,
) -> Vec<(Primitive, Option<String>)> {
    let primitive = primitive_of_action(action);
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

/// A line for the activity trace and the step log: what was blocked and why,
/// in the guard's own words.
pub(crate) fn block_detail(verdict: &GuardVerdict) -> String {
    format!("Ferrite's guard: {}", verdict.describe())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_engine::DigestElement;

    fn link(ref_id: u32, href: &str) -> DigestElement {
        DigestElement {
            ref_id,
            role: "link".into(),
            label: "Next".into(),
            href: Some(href.into()),
            ..DigestElement::default()
        }
    }

    fn page(elements: Vec<DigestElement>) -> PageDigest {
        PageDigest {
            url: "https://shop.example/".into(),
            elements,
            ..PageDigest::default()
        }
    }

    const HERE: &str = "https://shop.example/cart?x=1";

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
            ("blob:https://shop.example/0e", "blob:"),
            ("file:///etc/passwd", "file:"),
        ] {
            let effects = action_effects(&AgentAction::Navigate { url: url.into() }, HERE, None);
            assert_eq!(effects[0].1.as_deref(), Some(want), "{url}");
        }
        // Not a URL at all: no origin, which the guard treats as a deviation too.
        assert_eq!(
            action_effects(
                &AgentAction::Navigate {
                    url: "just words".into()
                },
                HERE,
                None
            )[0]
            .1,
            None
        );
    }

    #[test]
    fn a_page_action_acts_at_the_active_tabs_origin() {
        let effects = action_effects(&AgentAction::ReadPage, HERE, None);
        assert_eq!(
            effects,
            vec![(Primitive::DomRead, Some("https://shop.example".to_string()))]
        );
        let blank = action_effects(&AgentAction::ReadPage, "about:blank", None);
        assert_eq!(blank, vec![(Primitive::DomRead, None)]);
    }

    #[test]
    fn a_click_on_a_cross_origin_link_is_also_a_navigation_there() {
        let digest = page(vec![link(3, "https://attacker.example/landing?x=1")]);
        let effects = action_effects(
            &AgentAction::Click {
                selector: "@3".into(),
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
                    Some("https://attacker.example".to_string())
                ),
            ]
        );
    }

    #[test]
    fn a_click_on_a_same_origin_link_or_a_non_link_is_just_a_click() {
        let digest = page(vec![
            link(3, "https://shop.example/next"),
            DigestElement {
                ref_id: 4,
                role: "button".into(),
                href: Some("https://attacker.example/".into()),
                ..DigestElement::default()
            },
        ]);
        for selector in ["@3", "@4", "@99", "a.next", "@x"] {
            let effects = action_effects(
                &AgentAction::Click {
                    selector: selector.into(),
                },
                HERE,
                Some(&digest),
            );
            assert_eq!(effects.len(), 1, "{selector}");
        }
        // No digest available: still just the click.
        assert_eq!(
            action_effects(
                &AgentAction::Click {
                    selector: "@3".into()
                },
                HERE,
                None
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_link_with_a_javascript_or_data_href_is_a_navigation_to_an_opaque_origin() {
        let digest = page(vec![
            link(1, "javascript:steal()"),
            link(2, "data:text/html,x"),
        ]);
        for n in ["@1", "@2"] {
            let effects = action_effects(
                &AgentAction::Click { selector: n.into() },
                HERE,
                Some(&digest),
            );
            assert_eq!(effects.len(), 2, "{n}");
        }
    }

    #[test]
    fn the_blocked_observation_carries_nothing_from_the_action_or_the_page() {
        assert!(!BLOCKED_OBSERVATION.contains("http") && !BLOCKED_OBSERVATION.contains('@'));
    }
}
