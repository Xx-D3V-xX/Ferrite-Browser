//! Tests for the chat-driven agent panel's state machine: the run-ending
//! funnel (`conclude_run`), chat persistence through a temp-dir `ChatStore`,
//! New/Open/Delete chat, step recording, and (as later commits add them) the
//! context/IPI wiring, tab actions and the fast lane.
//!
//! All offline (R7): the only filesystem use is a per-test temp directory, and
//! `FerriteBrowser::default()` never touches disk or network.

use super::*;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// ── Fixtures ─────────────────────────────────────────────────────────────

/// A unique, self-cleaning temp directory (the repo has no `tempfile`
/// dependency; same approach as `ferrite-agent`'s chat tests).
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("ferrite_ui_chat_{name}_{nanos}_{n}")))
    }
    fn store(&self) -> ChatStore {
        ChatStore::new(&self.0)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A browser with a real temp-dir chat store and one turn in flight
/// (run 1), as `submit_task` leaves it.
fn running_state(dir: &TempDir, prompt: &str) -> FerriteBrowser {
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        run_id: 1,
        agent_is_running: true,
        ..FerriteBrowser::default()
    };
    state.chat.begin_turn(prompt, None);
    state.live_loop = Some(live());
    state
}

fn live() -> LiveAgentLoop {
    LiveAgentLoop {
        messages: vec![Message::user("go")],
        actions_taken: Vec::new(),
        started_at: std::time::Instant::now(),
        budget: LoopBudget::default(),
        rejected: Default::default(),
        rejected_origins: Default::default(),
        consecutive_malformed: 0,
    }
}

fn step(state: &mut FerriteBrowser, action: Result<AgentAction, StepFailure>) {
    let run_id = state.run_id;
    let _ = update(
        state,
        FerriteBrowserMessage::AgentStepReady { run_id, action },
    );
}

fn last_outcome(state: &FerriteBrowser) -> Outcome {
    state.chat.turns.last().expect("a turn").outcome.clone()
}

/// The chat as it is on disk, if it can be loaded.
fn on_disk(dir: &TempDir, chat: &Chat) -> Option<Chat> {
    dir.store().load(&chat.id).ok()
}

// ── Default is test-safe ─────────────────────────────────────────────────

#[test]
fn default_touches_no_disk_and_starts_with_an_empty_unsaved_chat() {
    let state = FerriteBrowser::default();
    assert!(state.chat_store.is_none(), "Default must not open a store");
    assert!(state.chat_list.is_empty());
    assert!(state.chat.turns.is_empty());
    assert_eq!(state.sidebar_view, SidebarView::Thread);
    assert!(state.panel_notice.is_none());
}

#[test]
fn only_launch_resolves_the_real_chats_directory() {
    // Same technique as the try_real_model_provider guard: scan this crate's
    // non-test source for call sites of `default_chats_dir(` and require the
    // single real one (launch), so a `Default`-reachable call cannot creep in.
    let src = include_str!("lib.rs");
    let end = src.find("#[cfg(test)]\nmod tests").unwrap_or(src.len());
    let calls = src[..end]
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("//") && l.contains("default_chats_dir(") && !l.contains("use ")
        })
        .count();
    assert_eq!(calls, 1, "exactly one call site (launch)");
}

// ── The run-ending funnel ────────────────────────────────────────────────

/// Every way a run can end records the right outcome on the chat, saves it
/// atomically, and shows it in the history list.
#[tokio::test]
async fn every_ending_path_records_its_outcome_and_saves() {
    type Setup = fn(&mut FerriteBrowser);
    type Expect = fn(&Outcome) -> bool;
    let cases: Vec<(&str, Setup, Expect)> = vec![
        (
            "finish",
            |s| {
                step(
                    s,
                    Ok(AgentAction::Finish {
                        answer: "all done".into(),
                    }),
                )
            },
            |o| *o == Outcome::Answered("all done".into()),
        ),
        (
            "ask_user",
            |s| {
                step(
                    s,
                    Ok(AgentAction::AskUser {
                        question: "which one?".into(),
                    }),
                )
            },
            |o| *o == Outcome::AskedUser("which one?".into()),
        ),
        (
            "model error",
            |s| step(s, Err(StepFailure::Model("no provider".into()))),
            |o| *o == Outcome::Failed("no provider".into()),
        ),
        (
            "malformed responses exhausted",
            |s| {
                s.live_loop.as_mut().unwrap().consecutive_malformed =
                    MAX_CONSECUTIVE_MALFORMED_STEPS;
                step(
                    s,
                    Err(StepFailure::Malformed {
                        raw: "junk".into(),
                        message: "EOF".into(),
                    }),
                )
            },
            |o| matches!(o, Outcome::Failed(t) if t.contains("EOF")),
        ),
        (
            "repeated action",
            |s| {
                let a = AgentAction::Click {
                    selector: "#x".into(),
                };
                s.live_loop.as_mut().unwrap().actions_taken = vec![a.clone(), a.clone()];
                step(s, Ok(a));
            },
            |o| matches!(o, Outcome::Stopped(t) if t.contains("repeat")),
        ),
        (
            "step budget",
            |s| {
                s.live_loop.as_mut().unwrap().budget.max_steps = 1;
                step(s, Ok(AgentAction::ReadDom));
            },
            |o| matches!(o, Outcome::Stopped(t) if t.contains("step budget")),
        ),
        (
            "wall clock budget",
            |s| {
                s.live_loop.as_mut().unwrap().budget.max_wall_clock = std::time::Duration::ZERO;
                step(s, Ok(AgentAction::ReadDom));
            },
            |o| matches!(o, Outcome::Stopped(t) if t.contains("wall-clock")),
        ),
        (
            "stop button",
            |s| {
                let _ = update(s, FerriteBrowserMessage::StopAgent);
            },
            |o| *o == Outcome::Cancelled,
        ),
        (
            "dry run failed",
            |s| {
                let _ = update(
                    s,
                    FerriteBrowserMessage::AgentFailed("dry run failed".into()),
                );
            },
            |o| *o == Outcome::Failed("dry run failed".into()),
        ),
        (
            "completed message",
            |s| {
                let _ = update(s, FerriteBrowserMessage::AgentCompleted("fine".into()));
            },
            |o| *o == Outcome::Answered("fine".into()),
        ),
    ];

    for (name, setup, expect) in cases {
        let dir = TempDir::new("ending");
        let mut state = running_state(&dir, "do the thing");
        setup(&mut state);

        let outcome = last_outcome(&state);
        assert!(expect(&outcome), "{name}: unexpected outcome {outcome:?}");
        assert!(!state.agent_is_running, "{name}: still running");
        assert!(state.live_loop.is_none(), "{name}: live loop kept");
        assert!(!state.chat.has_running_turn(), "{name}: turn left running");
        let saved =
            on_disk(&dir, &state.chat).unwrap_or_else(|| panic!("{name}: the chat was not saved"));
        assert_eq!(saved.turns.last().unwrap().outcome, outcome, "{name}");
        assert!(
            state.chat_list.iter().any(|s| s.id == state.chat.id),
            "{name}: chat missing from the list"
        );
        assert!(
            state.panel_notice.is_none(),
            "{name}: {:?}",
            state.panel_notice
        );
    }
}

#[test]
fn consent_cancelled_cancels_the_turn_and_saves() {
    let dir = TempDir::new("consent_cancel");
    let mut state = running_state(&dir, "check my inbox");
    let task = IpiTask::new("check my inbox", None);
    state.pending_task = Some("check my inbox".into());
    let mut diff = FingerprintDiff::default();
    diff.extra_primitives.insert(ToolId::new("js.execute"));
    let _ = update(
        &mut state,
        FerriteBrowserMessage::ConsentRequired {
            diff,
            expected: ExpectedFingerprint::empty(),
            evidence: Box::new(DryRunRecord::new(task.session_id, task.task_id)),
        },
    );
    assert!(
        state.chat.has_running_turn(),
        "the turn stays open while the user reviews"
    );

    let _ = update(&mut state, FerriteBrowserMessage::ConsentCancelled);

    assert_eq!(last_outcome(&state), Outcome::Cancelled);
    assert_eq!(
        on_disk(&dir, &state.chat).unwrap().turns[0].outcome,
        Outcome::Cancelled
    );
    assert!(state.pending_task.is_none());
}

#[test]
fn a_late_ending_after_the_turn_finished_cannot_overwrite_its_outcome() {
    let dir = TempDir::new("late");
    let mut state = running_state(&dir, "q");
    step(
        &mut state,
        Ok(AgentAction::Finish {
            answer: "first".into(),
        }),
    );
    let _ = update(&mut state, FerriteBrowserMessage::StopAgent);
    let _ = update(
        &mut state,
        FerriteBrowserMessage::AgentFailed("late".into()),
    );
    assert_eq!(last_outcome(&state), Outcome::Answered("first".into()));
}

#[test]
fn a_save_failure_is_a_notice_not_a_crash_and_the_run_still_ends() {
    // A store whose directory sits *under a regular file* cannot be created.
    let dir = TempDir::new("badstore");
    std::fs::create_dir_all(&dir.0).unwrap();
    let blocker = dir.0.join("not_a_dir");
    std::fs::write(&blocker, b"x").unwrap();
    let mut state = FerriteBrowser {
        chat_store: Some(ChatStore::new(blocker.join("chats"))),
        run_id: 1,
        agent_is_running: true,
        ..FerriteBrowser::default()
    };
    state.chat.begin_turn("q", None);
    state.live_loop = Some(live());

    step(
        &mut state,
        Ok(AgentAction::Finish {
            answer: "done".into(),
        }),
    );

    assert_eq!(last_outcome(&state), Outcome::Answered("done".into()));
    assert!(!state.agent_is_running);
    let notice = state.panel_notice.as_deref().unwrap_or_default();
    assert!(notice.contains("Couldn't save"), "{notice:?}");
    assert!(state.chat_list.is_empty(), "an unsaved chat is not listed");
}

// ── Step recording ───────────────────────────────────────────────────────

#[tokio::test]
async fn every_executed_step_is_recorded_on_the_turn() {
    let dir = TempDir::new("steps");
    let mut state = running_state(&dir, "q");
    step(
        &mut state,
        Ok(AgentAction::Click {
            selector: "@3".into(),
        }),
    );
    let mut live = state.live_loop.take().unwrap();
    live.rejected = [ToolId::new("js.execute")].into_iter().collect();
    state.live_loop = Some(live);
    step(
        &mut state,
        Ok(AgentAction::JsExecute {
            script: "1+1".into(),
        }),
    );

    let steps = &state.chat.turns[0].steps;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].label, "Click");
    assert_eq!(steps[0].detail, "@3");
    assert_eq!(steps[0].result, "error: no active browser session");
    assert!(!steps[0].blocked);
    assert_eq!(steps[1].label, "Run JavaScript");
    assert!(steps[1].blocked);
    assert_eq!(steps[1].result, "blocked by user consent");
}

#[tokio::test]
async fn typed_text_is_never_persisted_only_its_length() {
    let dir = TempDir::new("redact");
    let mut state = running_state(&dir, "log me in");
    step(
        &mut state,
        Ok(AgentAction::TypeText {
            selector: "@5".into(),
            text: "hunter2".into(),
        }),
    );
    let detail = &state.chat.turns[0].steps[0].detail;
    assert!(!detail.contains("hunter2"), "{detail}");
    assert!(detail.contains("7 chars"), "{detail}");
    // The live card (this run only, in memory) still shows what was typed.
    match &state.agent_log[0] {
        AgentLogEntry::Step { detail, .. } => assert!(detail.contains("hunter2")),
        other => panic!("{other:?}"),
    }
}

// ── Submit ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn submit_starts_a_turn_saves_it_and_clears_the_composer() {
    let dir = TempDir::new("submit");
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        agent_task_input: "  summarize this  ".into(),
        ..FerriteBrowser::default()
    };

    let _ = update(&mut state, FerriteBrowserMessage::AgentTaskSubmitted);

    assert!(state.agent_is_running);
    assert!(state.agent_task_input.is_empty(), "the composer is cleared");
    assert_eq!(state.chat.turns.len(), 1);
    assert_eq!(state.chat.turns[0].user, "summarize this");
    assert!(state.chat.has_running_turn());
    assert_eq!(state.chat.title, "summarize this");
    // Saved at the start of the turn: it survives a crash and is listed.
    assert!(on_disk(&dir, &state.chat).is_some());
    assert_eq!(state.chat_list.len(), 1);
}

#[tokio::test]
async fn submit_is_ignored_when_blank_running_or_awaiting_consent() {
    let mut state = FerriteBrowser {
        agent_task_input: "   ".into(),
        ..FerriteBrowser::default()
    };
    let _ = update(&mut state, FerriteBrowserMessage::AgentTaskSubmitted);
    assert!(state.chat.turns.is_empty() && !state.agent_is_running);

    state.agent_task_input = "go".into();
    state.agent_is_running = true;
    let _ = update(&mut state, FerriteBrowserMessage::AgentTaskSubmitted);
    assert!(state.chat.turns.is_empty());

    state.agent_is_running = false;
    state.pending_diff = Some(FingerprintDiff::default());
    let _ = update(&mut state, FerriteBrowserMessage::AgentTaskSubmitted);
    assert!(state.chat.turns.is_empty());
    assert_eq!(state.agent_task_input, "go", "the draft is kept");
}

#[tokio::test]
async fn a_second_message_continues_the_same_chat() {
    let dir = TempDir::new("multi");
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        agent_task_input: "first".into(),
        ..FerriteBrowser::default()
    };
    let _ = update(&mut state, FerriteBrowserMessage::AgentTaskSubmitted);
    let id = state.chat.id.clone();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::AgentCompleted("one".into()),
    );
    state.agent_task_input = "second".into();
    let _ = update(&mut state, FerriteBrowserMessage::AgentTaskSubmitted);

    assert_eq!(state.chat.id, id);
    assert_eq!(state.chat.turns.len(), 2);
    assert_eq!(state.chat.turns[1].user, "second");
    assert_eq!(state.chat_list.len(), 1, "still one chat, not two");
}

// ── New chat ─────────────────────────────────────────────────────────────

#[test]
fn new_chat_starts_empty_and_is_not_saved_until_its_first_message() {
    let dir = TempDir::new("newchat");
    let mut state = running_state(&dir, "old");
    step(&mut state, Ok(AgentAction::Finish { answer: "a".into() }));
    let old_id = state.chat.id.clone();
    state.expanded.insert("k".into());
    state.sidebar_view = SidebarView::History;

    let _ = update(&mut state, FerriteBrowserMessage::NewChat);

    assert_ne!(state.chat.id, old_id);
    assert!(state.chat.turns.is_empty());
    assert!(state.agent_log.is_empty() && state.agent_response.is_none());
    assert!(state.expanded.is_empty());
    assert_eq!(state.sidebar_view, SidebarView::Thread);
    assert!(state.show_agent_sidebar, "New chat opens the panel");
    assert!(on_disk(&dir, &state.chat).is_none(), "nothing written yet");
    assert_eq!(state.chat_list.len(), 1, "only the old chat is listed");
    // The old chat is intact on disk.
    assert!(on_disk(
        &dir,
        &Chat {
            id: old_id,
            ..Chat::new()
        }
    )
    .is_some());
}

#[test]
fn new_chat_is_refused_while_a_run_is_active_with_a_notice() {
    let dir = TempDir::new("newchat_busy");
    let mut state = running_state(&dir, "busy");
    let id = state.chat.id.clone();

    let _ = update(&mut state, FerriteBrowserMessage::NewChat);

    assert_eq!(state.chat.id, id, "the running chat is untouched");
    assert!(state.agent_is_running);
    assert!(state
        .panel_notice
        .as_deref()
        .unwrap_or_default()
        .contains("Stop the current run"));
}

#[test]
fn new_chat_is_refused_while_a_consent_decision_is_pending() {
    let mut state = FerriteBrowser::default();
    state.chat.begin_turn("q", None);
    state.pending_diff = Some(FingerprintDiff::default());
    let id = state.chat.id.clone();
    let _ = update(&mut state, FerriteBrowserMessage::NewChat);
    assert_eq!(state.chat.id, id);
    assert!(state.panel_notice.is_some());
}

// ── Open / delete ────────────────────────────────────────────────────────

/// Saves a finished one-turn chat through the store and returns it.
fn saved_chat(dir: &TempDir, prompt: &str, answer: &str) -> Chat {
    let mut chat = Chat::new();
    chat.begin_turn(prompt, None);
    chat.finish_turn(Outcome::Answered(answer.into()));
    dir.store().save(&chat).unwrap();
    chat
}

#[test]
fn persistence_round_trips_across_a_restart() {
    let dir = TempDir::new("restart");
    let mut first = running_state(&dir, "find flights");
    step(
        &mut first,
        Ok(AgentAction::Finish {
            answer: "three options".into(),
        }),
    );

    // "Restart": a new app reads the list and opens the chat, as launch()
    // and the history list do.
    let (list, warnings) = dir.store().list();
    assert!(warnings.is_empty());
    let mut second = FerriteBrowser {
        chat_store: Some(dir.store()),
        chat_list: list,
        ..FerriteBrowser::default()
    };
    assert_eq!(second.chat_list.len(), 1);
    assert_eq!(second.chat_list[0].title, "find flights");
    let id = second.chat_list[0].id.clone();

    let _ = update(&mut second, FerriteBrowserMessage::OpenChat(id));

    assert_eq!(second.chat, first.chat, "the whole chat, steps and all");
    assert_eq!(second.chat.turns[0].user, "find flights");
}

#[test]
fn a_corrupt_chat_file_never_breaks_the_list() {
    let dir = TempDir::new("corrupt");
    let good = saved_chat(&dir, "good one", "yes");
    std::fs::write(
        dir.0.join("ffffffff-ffff-ffff-ffff-ffffffffffff.json"),
        b"{ not json",
    )
    .unwrap();

    let (list, warnings) = dir.store().list();

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, good.id);
    assert_eq!(
        warnings.len(),
        1,
        "one log line for the bad file: {warnings:?}"
    );
}

#[test]
fn open_chat_loads_the_chat_and_switches_to_the_thread() {
    let dir = TempDir::new("open");
    let saved = saved_chat(&dir, "earlier", "answer");
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        sidebar_view: SidebarView::History,
        ..FerriteBrowser::default()
    };
    state.agent_response = Some("stale".into());
    state.expanded.insert("x".into());

    let _ = update(
        &mut state,
        FerriteBrowserMessage::OpenChat(saved.id.clone()),
    );

    assert_eq!(state.chat, saved);
    assert_eq!(state.sidebar_view, SidebarView::Thread);
    assert!(state.agent_response.is_none() && state.expanded.is_empty());
}

#[tokio::test]
async fn open_chat_is_blocked_while_a_run_is_active_and_never_reloads_the_running_chat() {
    let dir = TempDir::new("open_busy");
    let other = saved_chat(&dir, "other", "o");
    let mut state = running_state(&dir, "running one");
    let running_id = state.chat.id.clone();
    state.sidebar_view = SidebarView::History;

    let _ = update(
        &mut state,
        FerriteBrowserMessage::OpenChat(other.id.clone()),
    );
    assert_eq!(state.chat.id, running_id, "blocked: still the running chat");
    assert!(state.chat.has_running_turn());
    assert!(state.panel_notice.is_some());

    // Opening the *running* chat itself must not swap in the on-disk copy
    // (which, being mid-run, lacks the in-memory steps).
    step(
        &mut state,
        Ok(AgentAction::Click {
            selector: "@1".into(),
        }),
    );
    let steps_in_memory = state.chat.turns[0].steps.len();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::OpenChat(running_id.clone()),
    );
    assert_eq!(state.chat.turns[0].steps.len(), steps_in_memory);
    assert!(state.chat.has_running_turn());
    assert_eq!(state.sidebar_view, SidebarView::Thread);
}

#[test]
fn open_chat_reports_an_unreadable_chat_without_replacing_the_current_one() {
    let dir = TempDir::new("open_bad");
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        ..FerriteBrowser::default()
    };
    let current = state.chat.id.clone();
    let ghost = Chat::new();
    state.chat_list = vec![{
        let mut c = ghost.clone();
        c.begin_turn("gone", None);
        c.summary()
    }];
    let ghost_id = state.chat_list[0].id.clone();

    let _ = update(&mut state, FerriteBrowserMessage::OpenChat(ghost_id));

    assert_eq!(state.chat.id, current);
    assert!(
        state.chat_list.is_empty(),
        "a vanished file leaves the list"
    );
    assert!(state
        .panel_notice
        .as_deref()
        .unwrap_or_default()
        .contains("open"));
}

#[test]
fn delete_needs_a_confirm_and_can_be_cancelled() {
    let dir = TempDir::new("delete");
    let a = saved_chat(&dir, "a", "1");
    let b = saved_chat(&dir, "b", "2");
    let (list, _) = dir.store().list();
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        chat_list: list,
        sidebar_view: SidebarView::History,
        ..FerriteBrowser::default()
    };

    // One click only arms it: nothing is deleted.
    let _ = update(
        &mut state,
        FerriteBrowserMessage::RequestDeleteChat(a.id.clone()),
    );
    assert_eq!(state.pending_chat_delete, Some(a.id.clone()));
    assert!(on_disk(&dir, &a).is_some());
    assert_eq!(state.chat_list.len(), 2);

    // Cancel disarms.
    let _ = update(&mut state, FerriteBrowserMessage::CancelDeleteChat);
    assert!(state.pending_chat_delete.is_none());
    let _ = update(&mut state, FerriteBrowserMessage::ConfirmDeleteChat);
    assert!(
        on_disk(&dir, &a).is_some(),
        "confirm without an armed row is a no-op"
    );

    // Arm then confirm deletes exactly that chat.
    let _ = update(
        &mut state,
        FerriteBrowserMessage::RequestDeleteChat(a.id.clone()),
    );
    let _ = update(&mut state, FerriteBrowserMessage::ConfirmDeleteChat);
    assert!(on_disk(&dir, &a).is_none());
    assert!(on_disk(&dir, &b).is_some());
    assert_eq!(state.chat_list.len(), 1);
    assert_eq!(state.chat_list[0].id, b.id);
    assert!(state.pending_chat_delete.is_none());
}

#[test]
fn deleting_the_open_chat_when_idle_replaces_it_with_a_fresh_one() {
    let dir = TempDir::new("delete_current");
    let saved = saved_chat(&dir, "mine", "m");
    let (list, _) = dir.store().list();
    let mut state = FerriteBrowser {
        chat_store: Some(dir.store()),
        chat_list: list,
        chat: saved.clone(),
        ..FerriteBrowser::default()
    };
    let _ = update(
        &mut state,
        FerriteBrowserMessage::RequestDeleteChat(saved.id.clone()),
    );
    let _ = update(&mut state, FerriteBrowserMessage::ConfirmDeleteChat);
    assert_ne!(state.chat.id, saved.id);
    assert!(state.chat.turns.is_empty());
    assert!(on_disk(&dir, &saved).is_none());
}

#[tokio::test]
async fn the_running_chat_cannot_be_deleted() {
    let dir = TempDir::new("delete_running");
    let mut state = running_state(&dir, "busy");
    step(
        &mut state,
        Ok(AgentAction::Click {
            selector: "@1".into(),
        }),
    );
    persist_chat(&mut state);
    let id = state.chat.id.clone();
    let _ = update(
        &mut state,
        FerriteBrowserMessage::RequestDeleteChat(id.clone()),
    );
    let _ = update(&mut state, FerriteBrowserMessage::ConfirmDeleteChat);
    assert_eq!(state.chat.id, id);
    assert!(state.chat.has_running_turn());
    assert!(dir.store().load(&id).is_ok(), "the file is still there");
    assert!(state.panel_notice.is_some());
}

// ── Small handlers ───────────────────────────────────────────────────────

#[test]
fn notice_can_be_dismissed_and_expand_toggles() {
    let mut state = FerriteBrowser {
        panel_notice: Some("x".into()),
        ..FerriteBrowser::default()
    };
    let _ = update(&mut state, FerriteBrowserMessage::DismissNotice);
    assert!(state.panel_notice.is_none());

    let _ = update(&mut state, FerriteBrowserMessage::ToggleExpand("k".into()));
    assert!(state.expanded.contains("k"));
    let _ = update(&mut state, FerriteBrowserMessage::ToggleExpand("k".into()));
    assert!(!state.expanded.contains("k"));
}

#[test]
fn set_sidebar_view_switches_and_disarms_a_pending_delete() {
    let mut state = FerriteBrowser {
        pending_chat_delete: Some(ChatId::new()),
        ..FerriteBrowser::default()
    };
    let _ = update(
        &mut state,
        FerriteBrowserMessage::SetSidebarView(SidebarView::History),
    );
    assert_eq!(state.sidebar_view, SidebarView::History);
    assert!(state.pending_chat_delete.is_none());
}

#[test]
fn cmd_or_ctrl_shift_o_is_new_chat_and_a_bare_o_is_not() {
    let modifier = if cfg!(target_os = "macos") {
        keyboard::Modifiers::LOGO
    } else {
        keyboard::Modifiers::CTRL
    };
    for c in ["o", "O"] {
        let key = keyboard::Key::Character(c.into());
        assert!(
            matches!(
                handle_key_press(key, modifier | keyboard::Modifiers::SHIFT),
                Some(FerriteBrowserMessage::NewChat)
            ),
            "{c}"
        );
    }
    assert!(handle_key_press(keyboard::Key::Character("o".into()), modifier).is_none());
}
