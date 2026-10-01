//! Engine-agnostic assertions shared between `MockEngine`'s own test suite
//! (`tests/conformance.rs`, always run) and `ferrite-engine-servo`'s
//! `ServoEngine` tests (feature `engine-servo`, `#[ignore]`d unless a real
//! Servo build is present).
//!
//! This is a genuinely small, shared subset — not the entire action
//! surface — by design. `MockEngine` can be scripted with exact DOM
//! content, cookies, and JS results with no page ever having existed;
//! `ServoEngine` needs a real page actually loaded (over real, if only
//! loopback, HTTP) before `dom_snapshot`/`query`/`cookies_read` mean
//! anything. Forcing both through one parametrized fixture trait would cost
//! more engineering than this charter's remaining budget justifies for the
//! genuinely-overlapping behavior, so the two engines' richer,
//! implementation-specific scripts live in their own crates' test files —
//! see `docs/handoffs/a09.md` for the reasoning. What lives here is what is
//! true of *every* `BrowserEngine`, regardless of implementation:
//! navigation updates the reported origin, an unknown tab id is a real
//! error (not a panic), and an idle wait completes without blocking
//! forever.

use ferrite_core::Origin;

use crate::{BrowserEngine, EngineError, TabId, WaitCondition};

/// Navigating to `url` makes `current_url()` report it and both calls agree
/// on `url`'s origin.
pub fn navigate_updates_current_url_and_origin<E: BrowserEngine>(engine: &mut E, url: &str) {
    let expected_origin = Origin::parse(url).expect("test fixture URL must be http(s)");

    let (_, nav_origin) = engine.navigate(url).expect("navigate must succeed");
    assert_eq!(
        nav_origin, expected_origin,
        "navigate's reported origin must match the URL navigated to"
    );

    let (reported_url, read_origin) = engine.current_url().expect("current_url must succeed");
    assert_eq!(
        reported_url, url,
        "current_url must reflect the last navigation"
    );
    assert_eq!(
        read_origin, expected_origin,
        "current_url's reported origin must match navigate's"
    );
}

/// Closing/switching to a tab id that was never opened is a typed error,
/// never a panic, for every `BrowserEngine` implementation.
pub fn unknown_tab_id_is_a_typed_error<E: BrowserEngine>(engine: &mut E) {
    let bogus = TabId(u64::MAX);
    assert!(
        matches!(engine.close_tab(bogus), Err(EngineError::NoSuchTab(_))),
        "closing an unopened tab must be EngineError::NoSuchTab, not a panic"
    );
    assert!(
        matches!(engine.switch_tab(bogus), Err(EngineError::NoSuchTab(_))),
        "switching to an unopened tab must be EngineError::NoSuchTab, not a panic"
    );
}

/// After navigating to `url`, `page_digest()` and `observe_page()` both
/// succeed, report the same origin as `current_url()`, and describe the same
/// page — the per-step observation must never disagree with the read the
/// agent asks for. Holds for engines using the trait's default
/// implementations as well as for real page scripts.
pub fn page_digest_and_observation_agree<E: BrowserEngine>(engine: &mut E, url: &str) {
    let (_, nav_origin) = engine.navigate(url).expect("navigate must succeed");
    let (digest, digest_origin) = engine.page_digest().expect("page_digest must succeed");
    let (observed, observed_origin) = engine.observe_page().expect("observe_page must succeed");
    assert_eq!(
        digest_origin, nav_origin,
        "page_digest reports the page's origin"
    );
    assert_eq!(
        observed_origin, nav_origin,
        "observe_page reports the page's origin"
    );
    assert_eq!(digest.url, observed.url, "both views name the same page");
    for e in digest.elements.iter().chain(observed.elements.iter()) {
        assert!(
            !(e.sensitive && e.value.is_some()),
            "a sensitive element's value must never appear in a digest"
        );
    }
}

/// Text that cannot be on any page has zero matches and no snippets.
pub fn find_text_of_absent_text_finds_nothing<E: BrowserEngine>(engine: &mut E) {
    let (matches, _) = engine
        .find_text("\u{1F9EA}-text-that-is-on-no-page-\u{1F9EA}")
        .expect("find_text must succeed");
    assert_eq!(matches.count, 0);
    assert!(matches.snippets.is_empty());
}

/// `list_tabs` reports at least one tab and exactly one active tab, whose
/// URL is the one `current_url` reports.
pub fn list_tabs_reports_exactly_one_active_tab<E: BrowserEngine>(engine: &mut E) {
    let (url, _) = engine.current_url().expect("current_url must succeed");
    let (tabs, _) = engine.list_tabs().expect("list_tabs must succeed");
    assert!(!tabs.is_empty(), "an engine always has at least one tab");
    let active: Vec<_> = tabs.iter().filter(|t| t.active).collect();
    assert_eq!(active.len(), 1, "exactly one tab is active: {tabs:?}");
    assert_eq!(active[0].url, url);
}

/// The richer interaction surface (`press_key`, `hover`, `set_checked`,
/// `scroll_to`, `submit_form`, scoped `extract_links`) is either implemented
/// or fails with a typed, retry-is-pointless / not-found error — never a
/// panic and never a misleading success on an element that is not there.
pub fn optional_interactions_fail_typed_not_by_panicking<E: BrowserEngine>(engine: &mut E) {
    fn typed<T>(what: &str, r: Result<T, EngineError>) {
        match r {
            Ok(_)
            | Err(
                EngineError::Unsupported(_)
                | EngineError::ElementNotFound(_)
                | EngineError::Internal(_),
            ) => {}
            Err(other) => panic!("{what}: unexpected error class {other:?}"),
        }
    }
    typed("press_key", engine.press_key(None, "Enter"));
    typed("hover", engine.hover("@999999"));
    typed("set_checked", engine.set_checked("@999999", true));
    typed("scroll_to", engine.scroll_to("@999999"));
    typed("submit_form", engine.submit_form(None));
    typed("extract_links", engine.extract_links(Some("#nothing-here")));
}

/// `wait_for(Idle)` completes without blocking indefinitely, for every
/// `BrowserEngine` implementation (a page that never loads anything is, by
/// definition, idle).
pub fn wait_idle_completes<E: BrowserEngine>(engine: &mut E) {
    engine
        .wait_for(WaitCondition::Idle)
        .expect("wait_for(Idle) must complete, not hang or error");
}
