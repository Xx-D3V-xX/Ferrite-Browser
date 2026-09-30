//! Full action-suite conformance tests against [`MockEngine`] — the
//! directive's exit gate #1 for A9/T-109: every `BrowserEngine` trait
//! method exercised at least once, origin-tracking asserted (not just that
//! a call succeeds), `js_execute`'s special status noted, and
//! `cookies_read`/`storage_read`'s scoping actually shown to restrict what
//! comes back.

use ferrite_core::Origin;
use ferrite_engine::{
    conformance, BrowserEngine, Cookie, DomNode, DomSnapshot, ElementHandle, EngineError,
    MockEngine, TabId, WaitCondition,
};

fn origin(url: &str) -> Origin {
    Origin::parse(url).expect("fixture origin must be http(s)")
}

// ── Shared, engine-agnostic assertions (see `ferrite_engine::conformance`) ──

#[test]
fn shared_navigate_updates_current_url_and_origin() {
    let mut engine = MockEngine::new();
    conformance::navigate_updates_current_url_and_origin(&mut engine, "https://a.example/page");
}

#[test]
fn shared_unknown_tab_id_is_a_typed_error() {
    let mut engine = MockEngine::new();
    conformance::unknown_tab_id_is_a_typed_error(&mut engine);
}

#[test]
fn shared_wait_idle_completes() {
    let mut engine = MockEngine::new();
    conformance::wait_idle_completes(&mut engine);
}

#[test]
fn shared_page_digest_and_observation_agree() {
    let mut engine = MockEngine::new();
    conformance::page_digest_and_observation_agree(&mut engine, "https://a.example/page");
}

#[test]
fn shared_find_text_of_absent_text_finds_nothing() {
    let mut engine = MockEngine::new();
    conformance::find_text_of_absent_text_finds_nothing(&mut engine);
}

#[test]
fn shared_list_tabs_reports_exactly_one_active_tab() {
    let mut engine = MockEngine::new();
    conformance::list_tabs_reports_exactly_one_active_tab(&mut engine);
    engine.open_tab(Some("https://b.example/")).unwrap();
    conformance::list_tabs_reports_exactly_one_active_tab(&mut engine);
}

#[test]
fn shared_optional_interactions_fail_typed_not_by_panicking() {
    let mut engine = MockEngine::new();
    conformance::optional_interactions_fail_typed_not_by_panicking(&mut engine);
}

// ── An engine that implements ONLY the required methods: the new surface's
//    default implementations, exercised through the same shared assertions ──

/// Forwards every *required* method to a [`MockEngine`] and overrides none of
/// the new ones, so everything it does for them is the trait's default.
struct DefaultsOnly(MockEngine);

impl BrowserEngine for DefaultsOnly {
    fn navigate(&mut self, url: &str) -> Result<((), Origin), EngineError> {
        self.0.navigate(url)
    }
    fn go_back(&mut self) -> Result<((), Origin), EngineError> {
        self.0.go_back()
    }
    fn go_forward(&mut self) -> Result<((), Origin), EngineError> {
        self.0.go_forward()
    }
    fn reload(&mut self) -> Result<((), Origin), EngineError> {
        self.0.reload()
    }
    fn current_url(&mut self) -> Result<(String, Origin), EngineError> {
        self.0.current_url()
    }
    fn dom_snapshot(&mut self) -> Result<(DomSnapshot, Origin), EngineError> {
        self.0.dom_snapshot()
    }
    fn query(&mut self, s: &str) -> Result<(Vec<ElementHandle>, Origin), EngineError> {
        self.0.query(s)
    }
    fn read_text(&mut self, s: &str) -> Result<(String, Origin), EngineError> {
        self.0.read_text(s)
    }
    fn click(&mut self, s: &str) -> Result<((), Origin), EngineError> {
        self.0.click(s)
    }
    fn type_text(&mut self, s: &str, t: &str) -> Result<((), Origin), EngineError> {
        self.0.type_text(s, t)
    }
    fn fill_form(&mut self, f: &[(String, String)]) -> Result<((), Origin), EngineError> {
        self.0.fill_form(f)
    }
    fn select_option(&mut self, s: &str, v: &str) -> Result<((), Origin), EngineError> {
        self.0.select_option(s, v)
    }
    fn scroll(&mut self, dx: i64, dy: i64) -> Result<((), Origin), EngineError> {
        self.0.scroll(dx, dy)
    }
    fn wait_for(&mut self, c: WaitCondition) -> Result<((), Origin), EngineError> {
        self.0.wait_for(c)
    }
    fn screenshot(&mut self) -> Result<(ferrite_engine::Frame, Origin), EngineError> {
        self.0.screenshot()
    }
    fn download(&mut self, u: &str) -> Result<(String, Origin), EngineError> {
        self.0.download(u)
    }
    fn open_tab(&mut self, u: Option<&str>) -> Result<(TabId, Origin), EngineError> {
        self.0.open_tab(u)
    }
    fn close_tab(&mut self, t: TabId) -> Result<((), Origin), EngineError> {
        self.0.close_tab(t)
    }
    fn switch_tab(&mut self, t: TabId) -> Result<((), Origin), EngineError> {
        self.0.switch_tab(t)
    }
    fn cookies_read(&mut self, s: &Origin) -> Result<(Vec<Cookie>, Origin), EngineError> {
        self.0.cookies_read(s)
    }
    fn storage_read(&mut self, s: &Origin) -> Result<(Vec<(String, String)>, Origin), EngineError> {
        self.0.storage_read(s)
    }
    fn clipboard_read(&mut self) -> Result<(String, Origin), EngineError> {
        self.0.clipboard_read()
    }
    fn clipboard_write(&mut self, t: &str) -> Result<((), Origin), EngineError> {
        self.0.clipboard_write(t)
    }
    fn js_execute(&mut self, s: &str) -> Result<(String, Origin), EngineError> {
        self.0.js_execute(s)
    }
}

#[test]
fn defaults_only_engine_passes_the_shared_new_surface_assertions() {
    let mut engine = DefaultsOnly(MockEngine::new());
    conformance::page_digest_and_observation_agree(&mut engine, "https://a.example/page");
    conformance::find_text_of_absent_text_finds_nothing(&mut engine);
    conformance::list_tabs_reports_exactly_one_active_tab(&mut engine);
    conformance::optional_interactions_fail_typed_not_by_panicking(&mut engine);
}

#[test]
fn default_impls_derive_from_the_snapshot_and_are_honestly_unsupported_otherwise() {
    let a = origin("https://a.example/");
    let mut mock = MockEngine::new();
    mock.navigate(a.as_str()).unwrap();
    mock.seed_dom_snapshot(
        &a,
        DomSnapshot {
            root: DomNode {
                role: "generic".into(),
                text: Some("Order total is 42 dollars".into()),
                children: vec![DomNode {
                    role: "button".into(),
                    label: Some("Pay".into()),
                    ..DomNode::default()
                }],
                ..DomNode::default()
            },
        },
    );
    let mut engine = DefaultsOnly(mock);

    let (digest, o) = engine.page_digest().unwrap();
    assert_eq!(o, a);
    assert_eq!(digest.url, a.as_str());
    assert_eq!(digest.elements[0].label, "Pay");

    let (m, _) = engine.find_text("42").unwrap();
    assert_eq!(m.count, 1);
    assert!(m.snippets[0].contains("[42]"));

    // observe_page defaults to page_digest.
    assert_eq!(engine.observe_page().unwrap().0, digest);

    // No DOM behind the default engine: acting is honestly unsupported, and a
    // selector-scoped link extraction cannot be faked.
    assert!(matches!(
        engine.press_key(None, "Enter"),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        engine.hover("@1"),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        engine.set_checked("@1", true),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        engine.scroll_to("@1"),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        engine.submit_form(None),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        engine.extract_links(Some("nav")),
        Err(EngineError::Unsupported(_))
    ));
    assert!(engine.extract_links(None).unwrap().0.is_empty());
}

// ── Element refs and the new action surface on MockEngine ──

#[test]
fn every_selector_taking_op_normalizes_refs_before_logging() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    let canon = |n: u32| ferrite_engine::ref_selector(n);

    engine.click("@1").unwrap();
    engine.type_text("[2]", "x").unwrap();
    engine
        .fill_form(&[
            ("@3".to_string(), "v".to_string()),
            ("#css".into(), "w".into()),
        ])
        .unwrap();
    engine.select_option("ref:4", "b").unwrap();
    let _ = engine.query("@5");
    let _ = engine.read_text("@6");
    let _ = engine.wait_for(WaitCondition::Selector("@7".into()));
    engine.press_key(Some("@8"), "Enter").unwrap();
    engine.hover("@9").unwrap();
    engine.set_checked("@10", true).unwrap();
    engine.scroll_to("@11").unwrap();
    engine.submit_form(Some("@12")).unwrap();
    engine.extract_links(Some("@13")).unwrap();

    use ferrite_engine::{Call, WaitConditionKind};
    let calls = engine.calls();
    assert_eq!(calls[1], Call::Click(canon(1)));
    assert_eq!(calls[2], Call::TypeText(canon(2), "x".into()));
    assert_eq!(
        calls[3],
        Call::FillForm(vec![
            (canon(3), "v".into()),
            ("#css".into(), "w".into()) // a real CSS selector passes through untouched
        ])
    );
    assert_eq!(calls[4], Call::SelectOption(canon(4), "b".into()));
    assert_eq!(calls[5], Call::Query(canon(5)));
    assert_eq!(calls[6], Call::ReadText(canon(6)));
    assert_eq!(
        calls[7],
        Call::WaitFor(WaitConditionKind::Selector(canon(7)))
    );
    assert_eq!(calls[8], Call::PressKey(Some(canon(8)), "Enter".into()));
    assert_eq!(calls[9], Call::Hover(canon(9)));
    assert_eq!(calls[10], Call::SetChecked(canon(10), true));
    assert_eq!(calls[11], Call::ScrollTo(canon(11)));
    assert_eq!(calls[12], Call::SubmitForm(Some(canon(12))));
    assert_eq!(calls[13], Call::ExtractLinks(Some(canon(13))));
}

#[test]
fn seeds_match_a_ref_written_either_way_and_a_dead_ref_explains_itself() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.seed_read_text(&a, "@5", "five");
    assert_eq!(engine.read_text("[5]").unwrap().0, "five");
    assert_eq!(
        engine.read_text("@5").unwrap().0,
        "five",
        "a single seeded reply repeats"
    );

    match engine.read_text("@77").unwrap_err() {
        EngineError::ElementNotFound(msg) => {
            assert!(msg.contains("@77") && msg.contains("read_page"), "{msg}");
        }
        other => panic!("expected ElementNotFound, got {other:?}"),
    }
}

#[test]
fn page_digest_is_logged_but_observe_page_is_not_and_does_not_consume_the_queue() {
    use ferrite_engine::{Call, DigestElement, PageDigest};
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    let digest = |label: &str| PageDigest {
        elements: vec![DigestElement {
            ref_id: 1,
            role: "button".into(),
            label: label.into(),
            in_viewport: true,
            ..DigestElement::default()
        }],
        ..PageDigest::default()
    };
    engine.seed_page_digest(&a, digest("first"));
    engine.seed_page_digest(&a, digest("second"));
    let logged_before = engine.calls().len();

    // The harness's observation peeks: same answer twice, nothing logged.
    assert_eq!(engine.observe_page().unwrap().0.elements[0].label, "first");
    assert_eq!(engine.observe_page().unwrap().0.elements[0].label, "first");
    assert_eq!(engine.calls().len(), logged_before);

    // The agent-initiated read consumes and is logged exactly once each.
    assert_eq!(engine.page_digest().unwrap().0.elements[0].label, "first");
    assert_eq!(engine.page_digest().unwrap().0.elements[0].label, "second");
    let new_calls = &engine.calls()[logged_before..];
    assert_eq!(new_calls, &[Call::PageDigest, Call::PageDigest]);
    // The digest's url is filled in from the tab when the seed left it empty.
    assert_eq!(engine.page_digest().unwrap().0.url, a.as_str());
}

#[test]
fn find_text_and_extract_links_read_the_seeded_digest() {
    use ferrite_engine::{DigestElement, PageDigest};
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.seed_page_digest(
        &a,
        PageDigest {
            text: "The refund window is 30 days. Refunds take 5 days.".into(),
            elements: vec![DigestElement {
                ref_id: 1,
                role: "link".into(),
                label: "Policy".into(),
                href: Some("https://a.example/policy".into()),
                in_viewport: true,
                ..DigestElement::default()
            }],
            ..PageDigest::default()
        },
    );
    let (m, _) = engine.find_text("refund").unwrap();
    assert_eq!(m.count, 2);
    let (links, _) = engine.extract_links(None).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].href, "https://a.example/policy");
}

#[test]
fn list_tabs_reports_every_open_tab_and_marks_the_active_one() {
    let mut engine = MockEngine::new();
    engine.navigate("https://a.example/").unwrap();
    let (tab_b, _) = engine.open_tab(Some("https://b.example/")).unwrap();
    let (tabs, o) = engine.list_tabs().unwrap();
    assert_eq!(o, origin("https://b.example/"));
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].url, "https://a.example/");
    assert!(!tabs[0].active);
    assert_eq!(tabs[1].id, tab_b);
    assert!(tabs[1].active);
}

#[test]
fn every_new_trait_method_is_visible_in_the_call_log() {
    use ferrite_engine::Call;
    let mut engine = MockEngine::new();
    let _ = engine.page_digest();
    let _ = engine.press_key(None, "Tab");
    let _ = engine.hover("#x");
    let _ = engine.set_checked("#x", false);
    let _ = engine.scroll_to("#x");
    let _ = engine.find_text("x");
    let _ = engine.extract_links(None);
    let _ = engine.submit_form(None);
    let _ = engine.list_tabs();
    assert_eq!(
        engine.calls(),
        &[
            Call::PageDigest,
            Call::PressKey(None, "Tab".into()),
            Call::Hover("#x".into()),
            Call::SetChecked("#x".into(), false),
            Call::ScrollTo("#x".into()),
            Call::FindText("x".into()),
            Call::ExtractLinks(None),
            Call::SubmitForm(None),
            Call::ListTabs,
        ]
    );
}

// ── Navigation: navigate / go_back / go_forward / reload / current_url ──

#[test]
fn navigate_go_back_go_forward_and_reload_track_origin_through_history() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    let b = origin("https://b.example/");

    let (_, o) = engine.navigate("https://a.example/").unwrap();
    assert_eq!(o, a);

    let (_, o) = engine.navigate("https://b.example/").unwrap();
    assert_eq!(o, b);

    let (_, o) = engine.go_back().unwrap();
    assert_eq!(o, a, "go_back must return to the prior origin");

    let (_, o) = engine.go_forward().unwrap();
    assert_eq!(o, b, "go_forward must return to the origin it left");

    let (_, o) = engine.reload().unwrap();
    assert_eq!(o, b, "reload does not change the origin");
}

#[test]
fn go_back_past_the_start_of_history_is_a_typed_error() {
    let mut engine = MockEngine::new();
    assert!(matches!(engine.go_back(), Err(EngineError::Internal(_))));
}

#[test]
fn navigating_again_truncates_forward_history() {
    let mut engine = MockEngine::new();
    engine.navigate("https://a.example/").unwrap();
    engine.navigate("https://b.example/").unwrap();
    engine.go_back().unwrap();
    // Forward history to b.example is now discarded by this new navigation.
    engine.navigate("https://c.example/").unwrap();
    assert!(matches!(engine.go_forward(), Err(EngineError::Internal(_))));
}

// ── dom_snapshot / query / read_text ──

#[test]
fn dom_snapshot_returns_the_seeded_tree_for_the_current_origin() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.seed_dom_snapshot(
        &a,
        DomSnapshot {
            root: DomNode {
                role: "document".into(),
                label: Some("Example".into()),
                ..DomNode::default()
            },
        },
    );

    let (snap, o) = engine.dom_snapshot().unwrap();
    assert_eq!(o, a);
    assert_eq!(snap.root.role, "document");
    assert_eq!(snap.root.label.as_deref(), Some("Example"));
}

#[test]
fn query_resolves_seeded_selectors_scoped_to_the_current_origin() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    let b = origin("https://b.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.seed_query(
        &a,
        "#button",
        vec![ElementHandle {
            selector: "#button".into(),
            role: Some("button".into()),
            text: Some("Go".into()),
        }],
    );

    let (handles, o) = engine.query("#button").unwrap();
    assert_eq!(o, a);
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].role.as_deref(), Some("button"));

    // Same selector at a *different* origin is not the same query result —
    // proves query results are origin-scoped, not global by selector text.
    engine.navigate(b.as_str()).unwrap();
    let (handles, _) = engine.query("#button").unwrap();
    assert!(
        handles.is_empty(),
        "unseeded selector at a new origin must resolve to nothing"
    );
}

#[test]
fn read_text_of_an_unseeded_selector_is_element_not_found() {
    let mut engine = MockEngine::new();
    let err = engine.read_text("#missing").unwrap_err();
    assert!(matches!(err, EngineError::ElementNotFound(sel) if sel == "#missing"));
}

#[test]
fn dom_response_queue_serves_each_entry_then_repeats_the_last() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.seed_read_text(&a, "#status", "loading");
    engine.seed_read_text(&a, "#status", "done");

    assert_eq!(engine.read_text("#status").unwrap().0, "loading");
    assert_eq!(engine.read_text("#status").unwrap().0, "done");
    assert_eq!(
        engine.read_text("#status").unwrap().0,
        "done",
        "once the queue is down to one entry it repeats rather than erroring"
    );
}

// ── click / type_text / fill_form / select_option / scroll ──

#[test]
fn click_type_text_fill_form_select_option_and_scroll_all_report_the_current_origin() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();

    assert_eq!(engine.click("#button").unwrap().1, a);
    assert_eq!(engine.type_text("#input", "hello").unwrap().1, a);
    assert_eq!(
        engine
            .fill_form(&[("#a".to_string(), "1".to_string())])
            .unwrap()
            .1,
        a
    );
    assert_eq!(engine.select_option("#select", "b").unwrap().1, a);
    assert_eq!(engine.scroll(0, 100).unwrap().1, a);
}

// ── wait_for ──

#[test]
fn wait_for_selector_succeeds_once_seeded_and_times_out_otherwise() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();

    assert!(matches!(
        engine.wait_for(WaitCondition::Selector("#late".into())),
        Err(EngineError::WaitTimedOut)
    ));

    engine.seed_query(
        &a,
        "#late",
        vec![ElementHandle {
            selector: "#late".into(),
            role: None,
            text: None,
        }],
    );
    assert!(engine
        .wait_for(WaitCondition::Selector("#late".into()))
        .is_ok());
}

#[test]
fn wait_for_timeout_completes_without_a_real_sleep() {
    // R8/no-real-time-in-tests: MockEngine must not actually block for the
    // requested duration.
    let mut engine = MockEngine::new();
    let started = std::time::Instant::now();
    engine
        .wait_for(WaitCondition::Timeout(std::time::Duration::from_secs(30)))
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

// ── screenshot / download ──

#[test]
fn screenshot_returns_the_configured_frame_and_current_origin() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.set_screenshot(2, 2, vec![1, 2, 3, 4]);

    let ((w, h, bytes), o) = engine.screenshot().unwrap();
    assert_eq!((w, h), (2, 2));
    assert_eq!(bytes, vec![1, 2, 3, 4]);
    assert_eq!(o, a);
}

#[test]
fn download_reports_the_seeded_path_and_current_origin() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    engine.navigate(a.as_str()).unwrap();
    engine.seed_download("https://a.example/report.pdf", "/tmp/report.pdf");

    let (path, o) = engine.download("https://a.example/report.pdf").unwrap();
    assert_eq!(path, "/tmp/report.pdf");
    assert_eq!(o, a);
}

// ── tabs: open / close / switch ──

#[test]
fn open_close_and_switch_tab_move_the_active_origin() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    let b = origin("https://b.example/");
    engine.navigate(a.as_str()).unwrap();

    let (tab_b, o) = engine.open_tab(Some(b.as_str())).unwrap();
    assert_eq!(
        o, b,
        "opening a tab with a URL makes it active at that origin"
    );

    engine.switch_tab(TabId(0)).unwrap();
    assert_eq!(engine.current_url().unwrap().1, a);

    engine.switch_tab(tab_b).unwrap();
    assert_eq!(engine.current_url().unwrap().1, b);

    let (_, o) = engine.close_tab(tab_b).unwrap();
    assert_eq!(
        o, a,
        "closing the active tab falls back to a remaining tab, here tab 0's origin"
    );
}

#[test]
fn open_tab_with_no_url_starts_at_the_mock_home_origin() {
    let mut engine = MockEngine::new();
    let (_, o) = engine.open_tab(None).unwrap();
    assert_eq!(o, origin(ferrite_engine::MOCK_HOME));
}

// ── cookies_read / storage_read: scoping actually restricts what comes back ──

#[test]
fn cookies_read_is_scoped_and_does_not_leak_other_origins_cookies() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    let b = origin("https://b.example/");
    engine.seed_cookie(
        &a,
        Cookie {
            name: "session".into(),
            value: "a-secret".into(),
        },
    );
    engine.seed_cookie(
        &b,
        Cookie {
            name: "session".into(),
            value: "b-secret".into(),
        },
    );

    let (a_cookies, _) = engine.cookies_read(&a).unwrap();
    assert_eq!(a_cookies.len(), 1);
    assert_eq!(a_cookies[0].value, "a-secret");

    let (b_cookies, _) = engine.cookies_read(&b).unwrap();
    assert_eq!(b_cookies.len(), 1);
    assert_eq!(b_cookies[0].value, "b-secret");

    let unrelated = origin("https://c.example/");
    let (c_cookies, _) = engine.cookies_read(&unrelated).unwrap();
    assert!(
        c_cookies.is_empty(),
        "an origin with no seeded cookies must get none, not every origin's cookies"
    );
}

#[test]
fn storage_read_is_scoped_and_does_not_leak_other_origins_storage() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");
    let b = origin("https://b.example/");
    engine.seed_storage(&a, "token", "a-token");
    engine.seed_storage(&b, "token", "b-token");

    let (a_kv, _) = engine.storage_read(&a).unwrap();
    assert_eq!(a_kv, vec![("token".to_string(), "a-token".to_string())]);

    let (b_kv, _) = engine.storage_read(&b).unwrap();
    assert_eq!(b_kv, vec![("token".to_string(), "b-token".to_string())]);
}

// ── clipboard ──

#[test]
fn clipboard_write_then_read_round_trips() {
    let mut engine = MockEngine::new();
    engine.clipboard_write("copied text").unwrap();
    let (text, _) = engine.clipboard_read().unwrap();
    assert_eq!(text, "copied text");
}

// ── js_execute: exercised, and its privileged status documented ──

#[test]
fn js_execute_returns_the_seeded_result_for_the_exact_script() {
    let mut engine = MockEngine::new();
    engine.navigate("https://a.example/").unwrap();
    engine.seed_js("document.title", Ok("Example Domain".to_string()));

    let (result, origin_now) = engine.js_execute("document.title").unwrap();
    assert_eq!(result, "Example Domain");
    assert_eq!(origin_now, origin("https://a.example/"));

    // js_execute is on the trait — the trait itself does not gate it; that
    // is `ferrite-ipi::comparator`'s job upstream (see the crate's module
    // docs). This test asserts only that the method runs, not that it is
    // "safe" to call — no code path in this crate makes that claim.
}

#[test]
fn js_execute_failure_is_reported_not_panicked() {
    let mut engine = MockEngine::new();
    engine.seed_js(
        "throw 1",
        Err("ReferenceError: x is not defined".to_string()),
    );
    let err = engine.js_execute("throw 1").unwrap_err();
    assert!(matches!(err, EngineError::Internal(_)));
}

// ── Opaque origins: a real, typed outcome, not a panic ──

#[test]
fn navigating_to_an_opaque_scheme_reports_a_typed_error_not_a_fabricated_origin() {
    let mut engine = MockEngine::new();
    // navigate() itself must resolve the new origin to report it (the
    // directive's "every action returns (result, origin)" contract), so
    // the opaque-scheme failure surfaces immediately, not on some later
    // call.
    let err = engine.navigate("data:text/plain,hello").unwrap_err();
    assert!(matches!(err, EngineError::OpaqueOrigin(_)));
    // The tab's history still recorded the navigation attempt (state
    // change is not rolled back just because the origin can't be
    // reported) — the next *http(s)* navigation resolves normally.
    let (_, o) = engine.navigate("https://a.example/").unwrap();
    assert_eq!(o, origin("https://a.example/"));
}

// ── Call log: every action is actually recorded ──

#[test]
fn every_trait_method_is_visible_in_the_call_log() {
    let mut engine = MockEngine::new();
    let a = origin("https://a.example/");

    let _ = engine.navigate(a.as_str());
    let _ = engine.go_back();
    let _ = engine.go_forward();
    let _ = engine.reload();
    let _ = engine.current_url();
    let _ = engine.dom_snapshot();
    let _ = engine.query("#x");
    let _ = engine.read_text("#x");
    let _ = engine.click("#x");
    let _ = engine.type_text("#x", "y");
    let _ = engine.fill_form(&[]);
    let _ = engine.select_option("#x", "y");
    let _ = engine.scroll(0, 0);
    let _ = engine.wait_for(WaitCondition::Idle);
    let _ = engine.screenshot();
    let _ = engine.download("https://a.example/f");
    let (tab, _) = engine.open_tab(None).unwrap();
    let _ = engine.switch_tab(TabId(0));
    let _ = engine.close_tab(tab);
    let _ = engine.cookies_read(&a);
    let _ = engine.storage_read(&a);
    let _ = engine.clipboard_read();
    let _ = engine.clipboard_write("z");
    let _ = engine.js_execute("1+1");

    let calls = engine.calls();
    // 24 distinct trait methods called above, each pushed exactly once.
    assert_eq!(
        calls.len(),
        24,
        "every call above must be logged exactly once: {calls:?}"
    );
}
