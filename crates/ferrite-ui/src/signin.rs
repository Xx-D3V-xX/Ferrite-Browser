//! The sign-in handoff: when the agent reaches a page that wants a password
//! (or another secret), it stops and asks *you* to deal with it.
//!
//! Two reasons, one of them practical and one of them principle:
//!
//! * **Principle.** An agent should not be the thing that types credentials.
//!   Anything it types came from a model, and anything that watches the page
//!   (an injected instruction, a look-alike login form) is an attempt to make
//!   it do exactly that. Ferrite never reads a secret field's value into a page
//!   digest, and its page script refuses to type into one; this module is the
//!   user-facing half: the run pauses, before any model call, and a person signs
//!   in with their own hands.
//! * **Practical.** Providers such as Google and GitHub decide whether a sign-in
//!   is a person from the whole session. A human typing into the page is the
//!   only version of that which is both honest and likely to work.
//!
//! Detection is pure and conservative: a *visible password field* on the page,
//! or a well-known sign-in host. A page with no such signal is never paused, so
//! this cannot stall ordinary browsing; the worst a false positive costs is one
//! click on *Continue*.

use ferrite_engine::PageDigest;

/// Hosts that are sign-in pages by themselves, before their form has rendered.
/// Exact host matches only: a look-alike (`accounts.google.com.evil.example`) is
/// not one of these, and is caught, if at all, by its password field.
const SIGN_IN_HOSTS: &[&str] = &[
    "accounts.google.com",
    "login.microsoftonline.com",
    "login.live.com",
    "appleid.apple.com",
    "idmsa.apple.com",
];

/// What kind of secret the page is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallKind {
    /// A password field is on the page, or this is a known sign-in host.
    SignIn,
    /// Some other secret field: a one-time code, a card number, an ID number.
    Secret,
}

/// A page that needs the person, not the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInWall {
    /// What it is asking for.
    pub kind: WallKind,
    /// The page's host, shown to the person and remembered once they continue.
    pub host: String,
}

impl SignInWall {
    /// The card's title.
    #[must_use]
    pub fn title(&self) -> String {
        match self.kind {
            WallKind::SignIn => format!("Sign in to {}", self.host),
            WallKind::Secret => format!("{} is asking for a secret", self.host),
        }
    }

    /// The card's explanation.
    #[must_use]
    pub const fn body(&self) -> &'static str {
        match self.kind {
            WallKind::SignIn => {
                "Ferrite does not type passwords. Sign in on the page yourself, then press \
                 Continue and the agent picks up where it stopped."
            }
            WallKind::Secret => {
                "Ferrite does not type codes, card numbers or other secrets. Enter it on the \
                 page yourself, then press Continue."
            }
        }
    }
}

/// Whether the active page is one the agent must hand to the person.
///
/// `url` is the tab's address; `digest` is the page as the agent sees it, when
/// it could be read. `None` means nothing needs the person.
#[must_use]
pub fn detect(url: &str, digest: Option<&PageDigest>) -> Option<SignInWall> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();

    let has_password = digest.is_some_and(|d| {
        d.elements
            .iter()
            .any(|e| e.sensitive && !e.disabled && e.input_type.as_deref() == Some("password"))
    });
    if has_password || SIGN_IN_HOSTS.contains(&host.as_str()) {
        return Some(SignInWall {
            kind: WallKind::SignIn,
            host,
        });
    }
    let has_other_secret =
        digest.is_some_and(|d| d.elements.iter().any(|e| e.sensitive && !e.disabled));
    has_other_secret.then_some(SignInWall {
        kind: WallKind::Secret,
        host,
    })
}

/// What to do at the start of a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// Nothing needs the person; take the step.
    Proceed,
    /// Stop and ask.
    Pause(SignInWall),
}

/// Decides the gate for one step, and keeps `cleared_host` honest.
///
/// `cleared_host` is the host the person already pressed *Continue* on. While
/// the agent stays on that host it is not paused again, so a multi-page sign-in
/// does not ask at every page. The moment a step finds a page that needs no
/// one, it is forgotten, so coming back later asks again.
pub fn gate(cleared_host: &mut Option<String>, url: &str, digest: Option<&PageDigest>) -> Gate {
    match detect(url, digest) {
        None => {
            *cleared_host = None;
            Gate::Proceed
        }
        Some(wall) if cleared_host.as_deref() == Some(wall.host.as_str()) => Gate::Proceed,
        Some(wall) => Gate::Pause(wall),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_engine::DigestElement;

    fn element(role: &str, input_type: Option<&str>, sensitive: bool) -> DigestElement {
        DigestElement {
            ref_id: 1,
            role: role.to_string(),
            input_type: input_type.map(str::to_string),
            sensitive,
            in_viewport: true,
            ..Default::default()
        }
    }

    fn digest(elements: Vec<DigestElement>) -> PageDigest {
        PageDigest {
            url: String::new(),
            title: String::new(),
            text: String::new(),
            elements,
            ..Default::default()
        }
    }

    #[test]
    fn an_ordinary_page_is_never_paused() {
        let d = digest(vec![
            element("textbox", Some("text"), false),
            element("link", None, false),
        ]);
        assert_eq!(detect("https://news.ycombinator.com/", Some(&d)), None);
        assert_eq!(detect("https://example.com", None), None);
    }

    #[test]
    fn a_visible_password_field_is_a_sign_in_on_any_site() {
        let d = digest(vec![element("textbox", Some("password"), true)]);
        let wall = detect("https://github.com/login", Some(&d)).expect("wall");
        assert_eq!(wall.kind, WallKind::SignIn);
        assert_eq!(wall.host, "github.com");
    }

    #[test]
    fn a_well_known_sign_in_host_is_a_sign_in_before_its_form_renders() {
        let wall = detect("https://accounts.google.com/v3/signin/identifier", None).expect("wall");
        assert_eq!(
            (wall.kind, wall.host.as_str()),
            (WallKind::SignIn, "accounts.google.com")
        );
    }

    #[test]
    fn a_look_alike_host_is_not_mistaken_for_a_known_one() {
        assert_eq!(
            detect("https://accounts.google.com.evil.example/", None),
            None
        );
        assert_eq!(detect("https://notaccounts.google.com/", None), None);
    }

    #[test]
    fn another_secret_field_is_a_secret_wall_and_a_password_wins_over_it() {
        let code = digest(vec![element("textbox", Some("text"), true)]);
        assert_eq!(
            detect("https://shop.example/pay", Some(&code)).map(|w| w.kind),
            Some(WallKind::Secret)
        );
        let both = digest(vec![
            element("textbox", Some("text"), true),
            element("textbox", Some("password"), true),
        ]);
        assert_eq!(
            detect("https://shop.example/pay", Some(&both)).map(|w| w.kind),
            Some(WallKind::SignIn)
        );
    }

    #[test]
    fn a_disabled_secret_field_does_not_pause_anything() {
        let mut e = element("textbox", Some("password"), true);
        e.disabled = true;
        assert_eq!(detect("https://example.com/", Some(&digest(vec![e]))), None);
    }

    #[test]
    fn only_web_pages_are_considered() {
        let d = digest(vec![element("textbox", Some("password"), true)]);
        assert_eq!(detect("about:blank", Some(&d)), None);
        assert_eq!(detect("file:///tmp/x.html", Some(&d)), None);
        assert_eq!(detect("not a url", Some(&d)), None);
    }

    #[test]
    fn host_matching_ignores_case() {
        assert!(detect("https://ACCOUNTS.GOOGLE.COM/", None).is_some());
    }

    #[test]
    fn continuing_clears_the_wall_for_that_host_until_the_agent_leaves_it() {
        let d = digest(vec![element("textbox", Some("password"), true)]);
        let mut cleared = None;
        let Gate::Pause(wall) = gate(&mut cleared, "https://github.com/login", Some(&d)) else {
            panic!("should pause");
        };
        cleared = Some(wall.host);
        // The same host again (the next page of the sign-in): no second pause.
        assert_eq!(
            gate(
                &mut cleared,
                "https://github.com/sessions/two-factor",
                Some(&d)
            ),
            Gate::Proceed
        );
        // A page that needs no one: forgotten, so a later sign-in asks again.
        let plain = digest(vec![element("link", None, false)]);
        assert_eq!(
            gate(&mut cleared, "https://github.com/", Some(&plain)),
            Gate::Proceed
        );
        assert_eq!(cleared, None);
        assert!(matches!(
            gate(&mut cleared, "https://github.com/login", Some(&d)),
            Gate::Pause(_)
        ));
    }

    #[test]
    fn a_different_host_asks_even_after_another_was_cleared() {
        let d = digest(vec![element("textbox", Some("password"), true)]);
        let mut cleared = Some("github.com".to_string());
        assert!(matches!(
            gate(&mut cleared, "https://gitlab.com/users/sign_in", Some(&d)),
            Gate::Pause(_)
        ));
    }

    #[test]
    fn the_card_text_never_claims_more_than_ferrite_does() {
        let wall = SignInWall {
            kind: WallKind::SignIn,
            host: "example.com".into(),
        };
        assert_eq!(wall.title(), "Sign in to example.com");
        assert!(wall.body().contains("does not type passwords"));
        let secret = SignInWall {
            kind: WallKind::Secret,
            host: "shop.example".into(),
        };
        assert!(secret.title().contains("shop.example"));
    }

    // ── Wired into the live agent loop ──────────────────────────────────

    use crate::{update, FerriteBrowser, FerriteBrowserMessage, LiveAgentLoop};

    /// A browser whose active tab is at `url`, with a run in progress.
    fn running_at(url: &str) -> FerriteBrowser {
        let mut state = FerriteBrowser::default();
        let tab = state.active_tab;
        state.tab_urls[tab] = url.to_string();
        state.agent_is_running = true;
        state
    }

    fn fresh_loop() -> LiveAgentLoop {
        LiveAgentLoop::new(
            "seed".to_string(),
            "goal".to_string(),
            Default::default(),
            Default::default(),
        )
    }

    #[tokio::test]
    async fn the_agent_stops_on_a_sign_in_page_before_any_model_call() {
        let mut state = running_at("https://accounts.google.com/v3/signin/identifier");
        let run_id = state.run_id;
        let _ = crate::spawn_next_step(&mut state, run_id, fresh_loop());

        let wall = state.signin_handoff.as_ref().expect("paused");
        assert_eq!(
            (wall.kind, wall.host.as_str()),
            (WallKind::SignIn, "accounts.google.com")
        );
        assert!(
            state.agent_handle.is_none(),
            "no model call may be spawned while it waits"
        );
        assert!(
            state.live_loop.is_some(),
            "the loop is kept so Continue can resume it"
        );
        assert!(state.agent_is_running, "the run is paused, not finished");
    }

    #[tokio::test]
    async fn an_ordinary_page_is_never_paused_and_the_step_proceeds() {
        let mut state = running_at("https://news.ycombinator.com/");
        let run_id = state.run_id;
        let _ = crate::spawn_next_step(&mut state, run_id, fresh_loop());
        assert!(state.signin_handoff.is_none());
        assert!(state.agent_handle.is_some(), "the model call was spawned");
    }

    #[tokio::test]
    async fn continuing_resumes_the_same_run_tells_the_model_and_does_not_ask_again() {
        let mut state = running_at("https://accounts.google.com/signin");
        let run_id = state.run_id;
        let _ = crate::spawn_next_step(&mut state, run_id, fresh_loop());
        assert!(state.signin_handoff.is_some());

        let _ = update(&mut state, FerriteBrowserMessage::SigninContinue);
        assert!(state.signin_handoff.is_none(), "the card goes away");
        assert!(state.agent_handle.is_some(), "the next step was spawned");
        assert_eq!(state.run_id, run_id, "it is the same run");
        let live = state.live_loop.as_ref().expect("loop is back");
        assert_eq!(
            live.signin_cleared_host.as_deref(),
            Some("accounts.google.com")
        );
        let last = live.messages.last().expect("a message");
        assert!(
            last.content.contains("pressed Continue"),
            "{}",
            last.content
        );
        assert!(
            last.content.starts_with("seed"),
            "the note is appended to the observation, not a new turn"
        );
    }

    #[tokio::test]
    async fn continue_with_nothing_waiting_does_nothing() {
        let mut state = running_at("https://example.com/");
        let _ = update(&mut state, FerriteBrowserMessage::SigninContinue);
        assert!(state.agent_handle.is_none() && state.live_loop.is_none());
    }

    #[tokio::test]
    async fn stopping_the_task_while_it_waits_ends_the_run_and_clears_the_card() {
        let mut state = running_at("https://accounts.google.com/signin");
        let run_id = state.run_id;
        let _ = crate::spawn_next_step(&mut state, run_id, fresh_loop());
        assert!(state.signin_handoff.is_some());

        let _ = update(&mut state, FerriteBrowserMessage::StopAgent);
        assert!(state.signin_handoff.is_none());
        assert!(!state.agent_is_running);
        assert!(state.live_loop.is_none());
    }
}
