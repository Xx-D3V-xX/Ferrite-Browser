//! Tests for the optional Laya fast lane's plumbing in the UI: the background
//! half (`try_fast_lane`, against a loopback mock Laya server owned by the
//! test — no external network, R7), the shared step handler's treatment of a
//! fast-lane step (consent, loop safety, recording) and startup resolution.

use super::*;

use std::sync::Mutex;
use std::time::Duration;

use ferrite_agent::decider::{build_step_request, StepRequest};
use ferrite_engine::DigestElement;
use ferrite_model::laya::LayaConfig;
use ferrite_model::{MapEnv, MockProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::agent_run::{self, FastLaneInputs};

// ── Fixtures ─────────────────────────────────────────────────────────────

fn element(ref_id: u32, role: &str, label: &str) -> DigestElement {
    DigestElement {
        ref_id,
        role: role.into(),
        label: label.into(),
        in_viewport: true,
        ..DigestElement::default()
    }
}

/// A search page: a text box (ref 12), a Search button (ref 15) and a Cart
/// button (ref 31). Non-contiguous refs on purpose: Laya sees 1..n.
fn search_page() -> PageDigest {
    let mut search_box = element(12, "textbox", "Search products");
    search_box.value = Some(String::new());
    PageDigest {
        url: "https://shop.example/".into(),
        title: "Shop".into(),
        text: "Find anything. Search products below.".into(),
        elements: vec![
            search_box,
            element(15, "button", "Search"),
            element(31, "button", "Cart"),
        ],
        ..PageDigest::default()
    }
}

const GOAL: &str = "search for red shoes";

fn cfg() -> LayaConfig {
    LayaConfig::new("http://127.0.0.1:1")
}

/// The option id (1-based index) Laya must answer with to pick `ref_id`.
fn index_for(step: &StepRequest, ref_id: u32) -> String {
    (1..=step.candidate_count())
        .find(|i| step.ref_for_index(*i) == Some(ref_id))
        .unwrap_or_else(|| panic!("ref {ref_id} is not a candidate"))
        .to_string()
}

fn probs(ids: &[&str], choice: &str, p: f64) -> serde_json::Value {
    // A question with a single option must put all its probability on it.
    let p = if ids.len() == 1 { 1.0 } else { p };
    let rest = (1.0 - p) / (ids.len() - 1).max(1) as f64;
    let map: serde_json::Map<String, serde_json::Value> = ids
        .iter()
        .map(|id| {
            (
                (*id).to_string(),
                serde_json::json!(if *id == choice { p } else { rest }),
            )
        })
        .collect();
    serde_json::json!({"choice": choice, "probabilities": map, "confidence": p})
}

/// A response body answering the operation question and, if given, the
/// target question `qid`.
fn body_for(step: &StepRequest, op: (&str, f64), target: Option<(&str, String, f64)>) -> String {
    let mut answers = serde_json::Map::new();
    let q = step.request().question("operation").expect("operation q");
    answers.insert("operation".into(), probs(&q.ids(), op.0, op.1));
    if let Some((qid, choice, p)) = target {
        let q = step.request().question(qid).expect("target q");
        answers.insert(qid.into(), probs(&q.ids(), &choice, p));
    }
    serde_json::json!({"model": "typed-decisions", "answers": answers, "usage": {}}).to_string()
}

struct Server {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

/// A one-shot-per-connection HTTP/1.1 responder on 127.0.0.1 (a port the test
/// itself binds): records each request, waits `delay`, answers `status`.
async fn serve(status: u16, body: String, delay: Duration) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let base_url = format!("http://{}", listener.local_addr().expect("addr"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (log, body) = (Arc::clone(&log), body.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                        let want = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if rest.len() >= want {
                            break;
                        }
                    }
                }
                log.lock()
                    .expect("log")
                    .push(String::from_utf8_lossy(&buf).to_string());
                tokio::time::sleep(delay).await;
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Server { base_url, requests }
}

fn inputs(server: &Server, provider: MockProvider) -> FastLaneInputs {
    inputs_with(LayaConfig::new(&server.base_url), provider)
}

fn inputs_with(config: LayaConfig, provider: MockProvider) -> FastLaneInputs {
    FastLaneInputs {
        decider: Arc::new(LayaStepDecider::new(config)),
        digest: search_page(),
        goal: GOAL.into(),
        history: Vec::new(),
        previous: None,
        provider: Arc::new(provider),
        small_model_tag: "small-model".into(),
    }
}

fn step_for(digest: &PageDigest) -> StepRequest {
    build_step_request(GOAL, digest, &[], &cfg())
}

// ── try_fast_lane: the background half ───────────────────────────────────

#[tokio::test]
async fn a_confident_click_becomes_a_fast_action_with_a_history_item() {
    let step = step_for(&search_page());
    let target = index_for(&step, 15);
    let server = serve(
        200,
        body_for(&step, ("CLICK", 0.93), Some(("click_target", target, 0.85))),
        Duration::ZERO,
    )
    .await;

    let picked = agent_run::try_fast_lane(&inputs(&server, MockProvider::new())).await;

    let (fast, history) = picked.expect("a confident click clears both gates");
    assert_eq!(fast, FastAction::Click { target_ref: 15 });
    assert_eq!(
        fast.to_agent_action(),
        AgentAction::Click {
            selector: "@15".into()
        }
    );
    assert_eq!(history.action, "Search", "the acted-on element's label");
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn low_confidence_or_done_falls_back_to_the_llm() {
    let step = step_for(&search_page());
    let target = index_for(&step, 15);
    let unsure_target = serve(
        200,
        body_for(&step, ("CLICK", 0.95), Some(("click_target", target, 0.10))),
        Duration::ZERO,
    )
    .await;
    assert!(
        agent_run::try_fast_lane(&inputs(&unsure_target, MockProvider::new()))
            .await
            .is_none()
    );

    let done = serve(200, body_for(&step, ("DONE", 0.99), None), Duration::ZERO).await;
    assert!(
        agent_run::try_fast_lane(&inputs(&done, MockProvider::new()))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_slow_laya_pauses_itself_and_later_steps_send_no_request() {
    // Reads the shared trace back at the end; see `TRACE_LOCK`.
    let _trace = crate::activity_tests::TRACE_LOCK.lock().await;
    // Every request outlasts the timeout, as on the owner's machine.
    let slow = serve(200, "{}".into(), Duration::from_millis(400)).await;
    let mut config = LayaConfig::new(&slow.base_url);
    config.timeout = Duration::from_millis(100);
    let shared = inputs_with(config, MockProvider::new());

    // Two timeouts in a row open the breaker ...
    for _ in 0..2 {
        assert!(agent_run::try_fast_lane(&shared).await.is_none());
    }
    let sent = slow.requests.lock().unwrap().len();
    assert_eq!(sent, 2);

    // ... and the next steps skip Laya entirely: no request, straight to the LLM.
    for _ in 0..3 {
        assert!(agent_run::try_fast_lane(&shared).await.is_none());
    }
    assert_eq!(
        slow.requests.lock().unwrap().len(),
        sent,
        "a paused fast lane must not send requests"
    );
    // The pause is explained in the activity trace, not silent.
    assert!(ferrite_model::trace::global()
        .snapshot()
        .iter()
        .any(|e| e.stage == "fast lane paused" && e.note.contains("timed out")));
}

#[tokio::test]
async fn every_laya_failure_is_none_never_an_error_or_a_panic() {
    // HTTP failure.
    let down = serve(503, "{}".into(), Duration::ZERO).await;
    assert!(
        agent_run::try_fast_lane(&inputs(&down, MockProvider::new()))
            .await
            .is_none()
    );
    // Garbage body.
    let junk = serve(200, "not json".into(), Duration::ZERO).await;
    assert!(
        agent_run::try_fast_lane(&inputs(&junk, MockProvider::new()))
            .await
            .is_none()
    );
    // Nothing listening at all.
    let dead = LayaConfig::new("http://127.0.0.1:1");
    assert!(
        agent_run::try_fast_lane(&inputs_with(dead, MockProvider::new()))
            .await
            .is_none()
    );
    // A server slower than the configured timeout.
    let step = step_for(&search_page());
    let target = index_for(&step, 15);
    let slow = serve(
        200,
        body_for(&step, ("CLICK", 0.95), Some(("click_target", target, 0.9))),
        Duration::from_millis(600),
    )
    .await;
    let mut config = LayaConfig::new(&slow.base_url);
    config.timeout = Duration::from_millis(100);
    assert!(
        agent_run::try_fast_lane(&inputs_with(config, MockProvider::new()))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_repeat_of_the_previous_fast_action_is_suppressed() {
    let step = step_for(&search_page());
    let target = index_for(&step, 15);
    let server = serve(
        200,
        body_for(&step, ("CLICK", 0.95), Some(("click_target", target, 0.9))),
        Duration::ZERO,
    )
    .await;
    let mut repeat = inputs(&server, MockProvider::new());
    repeat.previous = Some(FastAction::Click { target_ref: 15 });
    assert!(
        agent_run::try_fast_lane(&repeat).await.is_none(),
        "Laya may not click the same thing twice in a row"
    );
}

#[tokio::test]
async fn type_text_gets_its_text_from_the_small_model_or_falls_back() {
    let step = step_for(&search_page());
    let target = index_for(&step, 12);
    let body = body_for(
        &step,
        ("TYPE_TEXT", 0.95),
        Some(("type_text_target", target, 0.9)),
    );

    // The helper LLM supplies the text.
    let server = serve(200, body.clone(), Duration::ZERO).await;
    let provider = MockProvider::new().push_content(r#"{"text":"red shoes"}"#);
    let (fast, history) = agent_run::try_fast_lane(&inputs(&server, provider))
        .await
        .expect("typed");
    assert_eq!(
        fast,
        FastAction::TypeText {
            target_ref: 12,
            text: "red shoes".into()
        }
    );
    assert_eq!(history.detail, "red shoes");

    // A helper that declines (`null`: a needed value is missing) => LLM step.
    let server = serve(200, body.clone(), Duration::ZERO).await;
    let declined = MockProvider::new().push_content(r#"{"text":null}"#);
    assert!(agent_run::try_fast_lane(&inputs(&server, declined))
        .await
        .is_none());

    // A helper that errors => LLM step, not a failed run.
    let server = serve(200, body, Duration::ZERO).await;
    let broken = MockProvider::new(); // nothing scripted: every call errors
    assert!(agent_run::try_fast_lane(&inputs(&server, broken))
        .await
        .is_none());
}

// ── The shared step handler treats a fast step like any other ────────────

fn fast_click() -> (AgentAction, FastAction, HistoryItem) {
    let fast = FastAction::Click { target_ref: 15 };
    (
        fast.to_agent_action(),
        fast.clone(),
        HistoryItem::new("Search", "", None),
    )
}

fn fast_step(state: &mut FerriteBrowser, parts: (AgentAction, FastAction, HistoryItem)) {
    let run_id = state.run_id;
    let (action, fast, history) = parts;
    let _ = update(
        state,
        FerriteBrowserMessage::FastStepReady {
            run_id,
            action,
            fast,
            history,
        },
    );
}

fn running_state() -> FerriteBrowser {
    let mut state = FerriteBrowser {
        run_id: 1,
        agent_is_running: true,
        ..FerriteBrowser::default()
    };
    state.chat.begin_turn("search for red shoes", None);
    state.live_loop = Some(LiveAgentLoop::new(
        "go".into(),
        GOAL.into(),
        Default::default(),
        Default::default(),
    ));
    state
}

#[tokio::test]
async fn a_fast_step_is_executed_logged_marked_and_remembered() {
    let mut state = running_state();
    fast_step(&mut state, fast_click());

    match &state.agent_log[0] {
        AgentLogEntry::Step {
            label,
            detail,
            fast,
            blocked,
            ..
        } => {
            assert_eq!(*label, "Click");
            assert_eq!(detail, "@15");
            assert!(*fast, "the step card is badged");
            assert!(!blocked);
        }
        other => panic!("{other:?}"),
    }
    let recorded = &state.chat.turns[0].steps[0];
    assert!(recorded.detail.starts_with("@15"), "{}", recorded.detail);
    assert!(
        recorded.detail.ends_with("fast lane"),
        "the record says so: {}",
        recorded.detail
    );

    let live = state.live_loop.as_ref().expect("the run continues");
    assert_eq!(
        live.actions_taken,
        [AgentAction::Click {
            selector: "@15".into()
        }]
    );
    assert_eq!(
        live.previous_fast,
        Some(FastAction::Click { target_ref: 15 })
    );
    assert_eq!(live.history.len(), 1);
    assert_eq!(live.history[0].action, "Search");
    // The model still sees the step in its own transcript.
    assert!(live
        .messages
        .iter()
        .any(|m| m.content.contains("\"click\"")));
}

#[tokio::test]
async fn an_llm_step_clears_previous_fast_and_is_not_badged() {
    let mut state = running_state();
    fast_step(&mut state, fast_click());
    step_llm(&mut state, AgentAction::ReadDom);

    let live = state.live_loop.as_ref().unwrap();
    assert_eq!(
        live.previous_fast, None,
        "an LLM step resets repeat suppression"
    );
    assert_eq!(live.history.len(), 2);
    match &state.agent_log[1] {
        AgentLogEntry::Step { fast, .. } => assert!(!fast),
        other => panic!("{other:?}"),
    }
    assert!(!state.chat.turns[0].steps[1].detail.contains("fast lane"));
}

fn step_llm(state: &mut FerriteBrowser, action: AgentAction) {
    let run_id = state.run_id;
    let _ = update(
        state,
        FerriteBrowserMessage::AgentStepReady {
            run_id,
            action: Ok(action),
        },
    );
}

#[tokio::test]
async fn a_rejected_tool_id_blocks_a_fast_lane_click_too() {
    let mut state = running_state();
    let click_id = action_tool_id(&AgentAction::Click {
        selector: "@15".into(),
    });
    state.live_loop.as_mut().unwrap().rejected = [click_id].into_iter().collect();

    fast_step(&mut state, fast_click());

    match &state.agent_log[0] {
        AgentLogEntry::Step {
            blocked,
            result,
            fast,
            ..
        } => {
            assert!(*blocked, "the consent decision applies to Laya's actions");
            assert_eq!(result, "blocked by user consent");
            assert!(*fast);
        }
        other => panic!("{other:?}"),
    }
    assert!(state.chat.turns[0].steps[0].blocked);
    // A blocked fast step still counts as the previous one, so Laya cannot
    // propose the very same blocked click again.
    assert_eq!(
        state.live_loop.as_ref().unwrap().previous_fast,
        Some(FastAction::Click { target_ref: 15 })
    );
}

#[tokio::test]
async fn the_step_budget_and_repeat_stop_apply_to_fast_steps() {
    // Budget: one allowed step, taken by a fast action, ends the run.
    let mut state = running_state();
    state.live_loop.as_mut().unwrap().budget.max_steps = 1;
    fast_step(&mut state, fast_click());
    assert!(matches!(
        state.chat.turns[0].outcome,
        Outcome::Stopped(ref why) if why.contains("step budget")
    ));
    assert!(!state.agent_is_running);

    // Repeat stop: the same action already twice in a row is not executed a
    // third time, fast lane or not.
    let mut state = running_state();
    let click = AgentAction::Click {
        selector: "@15".into(),
    };
    state.live_loop.as_mut().unwrap().actions_taken = vec![click.clone(), click];
    fast_step(&mut state, fast_click());
    assert!(matches!(
        state.chat.turns[0].outcome,
        Outcome::Stopped(ref why) if why.contains("repeat")
    ));
    assert!(state.agent_log.is_empty(), "nothing was executed");
}

#[tokio::test]
async fn a_fast_step_for_a_stale_run_is_ignored() {
    let mut state = running_state();
    state.run_id = 2; // the user stopped run 1 and started run 2
    let (action, fast, history) = fast_click();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::FastStepReady {
            run_id: 1,
            action,
            fast,
            history,
        },
    );
    let live = state
        .live_loop
        .as_ref()
        .expect("the current run is untouched");
    assert!(live.actions_taken.is_empty() && state.agent_log.is_empty());
    assert!(state.agent_is_running);
}

#[tokio::test]
async fn without_a_readable_page_or_laya_the_next_step_is_the_normal_llm_step() {
    for laya in [None, Some(Arc::new(LayaStepDecider::new(cfg())))] {
        let mut state = FerriteBrowser {
            run_id: 1,
            agent_is_running: true,
            laya,
            ..FerriteBrowser::default()
        };
        state.chat.begin_turn("q", None);
        let live = LiveAgentLoop::new(
            "go".into(),
            GOAL.into(),
            Default::default(),
            Default::default(),
        );
        let _ = spawn_next_step(&mut state, 1, live);
        // No session in a unit test => no digest => no fast lane; the normal
        // step was spawned and the run is alive.
        assert!(state.agent_handle.is_some());
        assert!(state.live_loop.is_some());
        assert!(state.agent_is_running);
    }
}

// ── Startup resolution ───────────────────────────────────────────────────

#[test]
fn laya_is_off_by_default_and_in_default_state() {
    assert!(FerriteBrowser::default().laya.is_none());
    let (laya, log) = agent_run::resolve_laya(&MapEnv::new());
    assert!(laya.is_none());
    assert_eq!(log.len(), 1);
    assert!(log[0].contains("disabled"), "{log:?}");
}

#[test]
fn a_loopback_laya_is_enabled_without_a_privacy_warning() {
    let env = MapEnv::new().with("FERRITE_LAYA_URL", "http://127.0.0.1:8000");
    let (laya, log) = agent_run::resolve_laya(&env);
    assert!(laya.is_some());
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].contains("enabled") && log[0].contains("127.0.0.1:8000"));
}

#[test]
fn a_remote_laya_warns_once_that_page_data_leaves_the_machine() {
    let env = MapEnv::new().with(
        "FERRITE_LAYA_URL",
        "https://user:secret@laya.example.com:9000/x",
    );
    let (laya, log) = agent_run::resolve_laya(&env);
    assert!(laya.is_some());
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[1].contains("WARNING") && log[1].contains("laya.example.com:9000"));
    assert!(
        log.iter()
            .all(|l| !l.contains("secret") && !l.contains("user:")),
        "credentials never reach the log: {log:?}"
    );
}

#[test]
fn a_bad_laya_configuration_disables_it_once_and_never_aborts_startup() {
    let env = MapEnv::new().with("FERRITE_LAYA_URL", "localhost:8000"); // no scheme
    let (laya, log) = agent_run::resolve_laya(&env);
    assert!(laya.is_none());
    assert_eq!(log.len(), 1);
    assert!(
        log[0].contains("disabled") && log[0].contains("invalid"),
        "{log:?}"
    );
}

#[test]
fn loopback_detection_is_a_whole_host_match() {
    for local in [
        "http://localhost:8000",
        "http://LOCALHOST",
        "http://127.0.0.1",
        "http://127.8.9.10:1",
        "http://[::1]:8000",
    ] {
        assert!(agent_run::laya_url_is_loopback(local), "{local}");
    }
    for remote in [
        "http://localhost.evil.example",
        "http://10.0.0.5:8000",
        "https://laya.example.com",
        "http://127.0.0.1.evil.example",
        "not a url",
    ] {
        assert!(!agent_run::laya_url_is_loopback(remote), "{remote}");
    }
}

#[test]
fn only_launch_resolves_laya() {
    let src = include_str!("lib.rs");
    let end = src
        .find("#[cfg(test)]\nmod chat_tests")
        .unwrap_or(src.len());
    let calls = src[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//") && l.contains("resolve_laya("))
        .count();
    assert_eq!(calls, 1, "exactly one call site (launch)");
}

// ── Small pure helpers ───────────────────────────────────────────────────

#[test]
fn the_page_signature_moves_when_the_page_changes_and_only_then() {
    let a = search_page();
    let same = search_page();
    assert_eq!(
        agent_run::page_signature(&a),
        agent_run::page_signature(&same)
    );

    let mut retitled = search_page();
    retitled.text.push_str(" New results loaded.");
    assert_ne!(
        agent_run::page_signature(&a),
        agent_run::page_signature(&retitled)
    );

    let mut typed = search_page();
    typed.elements[0].value = Some("red shoes".into());
    assert_ne!(
        agent_run::page_signature(&a),
        agent_run::page_signature(&typed)
    );
}

#[test]
fn llm_history_items_never_carry_typed_text() {
    let item = agent_run::llm_history_item(
        &AgentAction::TypeText {
            selector: "@5".into(),
            text: "hunter2".into(),
        },
        "Type text",
    );
    assert_eq!(item.action, "Type text");
    assert!(item.detail.is_empty() && !item.action.contains("hunter2"));
    let nav = agent_run::llm_history_item(
        &AgentAction::Navigate {
            url: "https://a.example".into(),
        },
        "Navigate",
    );
    assert_eq!(nav.action, "Navigate https://a.example");
}

#[test]
fn the_fast_mark_is_appended_after_bounding_the_detail() {
    let long = "x".repeat(1000);
    let marked = agent_run::mark_fast(&long);
    assert!(marked.ends_with("fast lane"));
    assert!(marked.chars().count() <= 260, "{}", marked.chars().count());
    assert_eq!(agent_run::mark_fast("@3"), "@3 \u{b7} fast lane");
}
