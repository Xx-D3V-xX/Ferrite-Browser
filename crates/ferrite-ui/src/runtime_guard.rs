//! Live enforcement of the predicted fingerprint (ADR-014): turns an agent
//! action into the effects the comparator reasons about, and the verdict into
//! text for the log and for the agent.
//!
//! The decision itself is `ferrite_ipi::comparator::RuntimeGuard`; this module
//! only knows the UI's vocabulary (`AgentAction`, the active tab's URL, the
//! page digest).

use ferrite_core::Primitive;
use ferrite_engine::PageDigest;
use ferrite_ipi::comparator::{EventVerdict, GuardVerdict, RuntimeGuard};

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

/// For a click by `@ref` on a link that leads to the origin the tab is already
/// at: the navigation that click amounts to. Such a click is **either** a click
/// or a navigation here (`RuntimeGuard::check_either`), so a task that was only
/// ever expected to navigate ("go to this site's docs page") can follow a link
/// there, as it could already `navigate` to the same address. `None` for
/// anything else, including every link to another origin (which stays a click
/// here *and* a navigation there) and a link with no known destination.
pub(crate) fn same_site_link_navigation(
    action: &AgentAction,
    active_tab_url: &str,
    digest: Option<&PageDigest>,
) -> Option<(Primitive, Option<String>)> {
    let AgentAction::Click { selector } = action else {
        return None;
    };
    let here = origin_of_url(active_tab_url)?;
    let element = digest?.element(selector.strip_prefix('@')?.parse::<u32>().ok()?)?;
    let there = (element.role == "link")
        .then_some(element.href.as_deref())
        .flatten()
        .and_then(effect_origin)?;
    (there == here).then_some((Primitive::Navigate, Some(here)))
}

/// Judges `action` against `guard`: the effects it has, and the verdict.
/// This is the one place the live loop and the tests decide an action, so
/// they cannot drift apart.
pub(crate) fn judge(
    guard: &RuntimeGuard,
    action: &AgentAction,
    active_tab_url: &str,
    digest: Option<&PageDigest>,
) -> (GuardVerdict, Vec<(Primitive, Option<String>)>) {
    let effects = action_effects(action, active_tab_url, digest);
    let verdict = match same_site_link_navigation(action, active_tab_url, digest) {
        // A link to the site the tab is already on is a navigation there as
        // well as a click, so a task expected only to navigate may follow it.
        Some(navigation) if effects.len() == 1 => guard.check_either(
            (effects[0].0, effects[0].1.as_deref()),
            (navigation.0, navigation.1.as_deref()),
        ),
        _ => guard.check_all(&effects),
    };
    (verdict, effects)
}

/// What the person has decided about the one action the guard stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GuardChoice {
    /// Nothing decided yet: a blocked action is put to the person.
    #[default]
    Ask,
    /// Run this one action although it is outside the prediction.
    AllowOnce,
    /// Do not run it; tell the agent it was blocked.
    Deny,
}

/// What to do with an action, given the guard's verdict and the person's choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settled {
    /// Run it. `approved` is true when it only runs because a person said so.
    Run { approved: bool },
    /// Stop the run here and ask the person.
    Ask,
    /// Do not run it.
    Block,
}

/// The single place that turns a verdict into a decision. A blocked action is
/// never run on its own: it either waits for a person (`Ask`), is refused, or
/// runs because that person allowed exactly this action.
pub(crate) fn settle(verdict: &GuardVerdict, choice: GuardChoice) -> Settled {
    match (verdict, choice) {
        (GuardVerdict::Expected(_), _) => Settled::Run { approved: false },
        (GuardVerdict::Approved(_), _) => Settled::Run { approved: true },
        (GuardVerdict::Block(_), GuardChoice::Ask) => Settled::Ask,
        (GuardVerdict::Block(_), GuardChoice::AllowOnce) => Settled::Run { approved: true },
        (GuardVerdict::Block(_), GuardChoice::Deny) => Settled::Block,
    }
}

/// What the person is asked, and what *Allow for this task* would approve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAsk {
    /// One plain sentence naming the action and the site, never anything a
    /// page wrote (the agent's own words and the page's labels stay out: this
    /// text is a prompt-injection target otherwise).
    pub summary: String,
    /// The tool to approve for the rest of the task, when the verdict is about
    /// a tool no capability names.
    pub approve_tool: Option<ferrite_ipi::tool_decision::ToolId>,
    /// The site to approve for the rest of the task, when the verdict is about
    /// a site no scope admits.
    pub approve_origin: Option<String>,
}

/// The question for a blocked verdict; `None` for one that is not blocked.
pub(crate) fn ask_for(verdict: &GuardVerdict, action_label: &str) -> Option<RuntimeAsk> {
    let GuardVerdict::Block(event) = verdict else {
        return None;
    };
    let (approve_tool, approve_origin) = match event {
        EventVerdict::ExtraPrimitive(tool) => (Some(tool.clone()), None),
        EventVerdict::OutOfScopeOrigin(origin) => (None, Some(origin.clone())),
        EventVerdict::Justified(_) => return None,
    };
    Some(RuntimeAsk {
        summary: format!(
            "{action_label}: {}",
            verdict.describe().replace(", and was not approved", "")
        ),
        approve_tool,
        approve_origin,
    })
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
    fn a_click_on_a_same_origin_link_is_also_a_navigation_here() {
        let digest = page(vec![
            link(3, "https://shop.example/docs/index.html"),
            link(4, "https://attacker.example/"),
            link(5, "javascript:steal()"),
            DigestElement {
                ref_id: 6,
                role: "button".into(),
                href: Some("https://shop.example/x".into()),
                ..DigestElement::default()
            },
        ]);
        let click = |s: &str| AgentAction::Click { selector: s.into() };
        assert_eq!(
            same_site_link_navigation(&click("@3"), HERE, Some(&digest)),
            Some((
                Primitive::Navigate,
                Some("https://shop.example".to_string())
            ))
        );
        // Another origin, an opaque scheme, a button, an unknown ref, a CSS
        // selector, no digest, a non-click: none of these is relaxed.
        for selector in ["@4", "@5", "@6", "@99", "a.next", "@x"] {
            assert_eq!(
                same_site_link_navigation(&click(selector), HERE, Some(&digest)),
                None,
                "{selector}"
            );
        }
        assert_eq!(same_site_link_navigation(&click("@3"), HERE, None), None);
        assert_eq!(
            same_site_link_navigation(&AgentAction::ReadPage, HERE, Some(&digest)),
            None
        );
        assert_eq!(
            same_site_link_navigation(&click("@3"), "about:blank", Some(&digest)),
            None
        );
    }

    /// The reported failure: "go to this site's docs page" is expected to need
    /// navigation at the current site only, and following the site's own Docs
    /// link was blocked as a click. It now runs; a link elsewhere, a button and
    /// a read-only task are still stopped.
    #[test]
    fn a_navigation_task_may_follow_a_link_on_its_own_site_and_nothing_else() {
        use ferrite_core::scope::OriginScope;
        use ferrite_core::{Capability, ExpectedCapability, ExpectedCapabilitySet, Origin};
        use ferrite_ipi::comparator::ExpectedFingerprint;

        let site = Origin::parse("https://shop.example").expect("origin");
        let guard_for = |cap| {
            RuntimeGuard::new(ExpectedFingerprint::from_capabilities(
                ExpectedCapabilitySet::new([ExpectedCapability::new(
                    cap,
                    OriginScope::exact([site.clone()]).expect("scope"),
                )])
                .expect("one capability"),
            ))
        };
        let digest = page(vec![
            link(4, "https://shop.example/docs/index.html"),
            link(5, "https://attacker.example/"),
            DigestElement {
                ref_id: 6,
                role: "button".into(),
                ..DigestElement::default()
            },
        ]);
        let verdict = |guard: &RuntimeGuard, n: &str| {
            let action = AgentAction::Click { selector: n.into() };
            judge(guard, &action, HERE, Some(&digest)).0
        };
        let navigate = guard_for(Capability::WebNavigate);
        assert!(verdict(&navigate, "@4").allows(), "own-site link");
        assert!(!verdict(&navigate, "@5").allows(), "link to another site");
        assert!(!verdict(&navigate, "@6").allows(), "a button is a click");
        assert!(verdict(&guard_for(Capability::WebInteract), "@4").allows());
        assert!(!verdict(&guard_for(Capability::WebRead), "@4").allows());
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
    fn a_blocked_action_is_asked_about_never_run_on_its_own() {
        use ferrite_ipi::tool_decision::ToolId;
        let block = GuardVerdict::Block(EventVerdict::ExtraPrimitive(ToolId::new("click")));
        assert_eq!(settle(&block, GuardChoice::Ask), Settled::Ask);
        assert_eq!(settle(&block, GuardChoice::Deny), Settled::Block);
        assert_eq!(
            settle(&block, GuardChoice::AllowOnce),
            Settled::Run { approved: true }
        );
        // An action that was already approved at the start of the task runs
        // whatever the choice says; one that is merely expected is not "approved".
        let approved = GuardVerdict::Approved(EventVerdict::ExtraPrimitive(ToolId::new("click")));
        for choice in [GuardChoice::Ask, GuardChoice::AllowOnce, GuardChoice::Deny] {
            assert_eq!(settle(&approved, choice), Settled::Run { approved: true });
        }
    }

    #[test]
    fn the_question_names_the_action_and_the_site_and_nothing_from_the_page() {
        use ferrite_ipi::tool_decision::ToolId;
        let tool = GuardVerdict::Block(EventVerdict::ExtraPrimitive(ToolId::new("click")));
        let ask = ask_for(&tool, "Click").expect("asked");
        assert!(ask.summary.starts_with("Click: "), "{}", ask.summary);
        assert!(!ask.summary.contains("not approved"), "{}", ask.summary);
        assert_eq!(ask.approve_tool, Some(ToolId::new("click")));
        assert_eq!(ask.approve_origin, None);

        let site = GuardVerdict::Block(EventVerdict::OutOfScopeOrigin(
            "https://other.example".into(),
        ));
        let ask = ask_for(&site, "Navigate").expect("asked");
        assert!(ask.summary.contains("https://other.example"));
        assert_eq!(ask.approve_origin.as_deref(), Some("https://other.example"));
        assert_eq!(ask.approve_tool, None);
    }

    #[test]
    fn the_blocked_observation_carries_nothing_from_the_action_or_the_page() {
        assert!(!BLOCKED_OBSERVATION.contains("http") && !BLOCKED_OBSERVATION.contains('@'));
    }
}
