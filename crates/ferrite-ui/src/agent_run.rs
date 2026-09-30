//! Agent run lifecycle helpers: the pure, unit-testable pieces behind the
//! chat-driven agent panel (how a finished step is recorded, how a run's
//! outcome is mirrored into the live state, how the chat list is kept in
//! order). Everything here is a plain function of plain data; the stateful
//! half (the `update()` handlers, `conclude_run`, the live loop) lives in
//! `lib.rs` and calls into this module. Nothing here touches
//! `FerriteBrowser`, the disk, the keyring or the network.

use ferrite_agent::browser_loop::AgentAction;
use ferrite_agent::chat::{ChatSummary, Outcome};

use super::action_detail;

// ---------------------------------------------------------------------------
// Recording a step
// ---------------------------------------------------------------------------

/// The action argument summary that is **persisted** in the chat file (and
/// echoed into later seeds). Like [`action_detail`], except that text the
/// agent typed or wrote to the clipboard is recorded as a length only: chat
/// files are plaintext on disk and get re-read into future prompts, and typed
/// values are exactly where a password or personal detail the user supplied
/// would live. The live step card still shows the full text for this run.
#[must_use]
pub(crate) fn persisted_step_detail(action: &AgentAction) -> String {
    match action {
        AgentAction::TypeText { selector, text } => {
            format!("{selector} \u{2192} {} chars", text.chars().count())
        }
        AgentAction::ClipboardWrite { text } => format!("{} chars", text.chars().count()),
        other => action_detail(other),
    }
}

/// The part of a step's observation worth keeping in the transcript: the
/// action's own result, without the fresh element table `execute_action`
/// appends after state-changing actions (that table is for the model, and it
/// is what would otherwise fill the 600-character record).
#[must_use]
pub(crate) fn step_result_text(observation: &str) -> String {
    observation
        .split("\n\nPAGE: ")
        .next()
        .unwrap_or(observation)
        .trim()
        .to_string()
}

/// `outcome`'s text as the run's live mirror (`FerriteBrowser::agent_response`)
/// has always shown it: answers and questions verbatim, budget/safety stops as
/// `[stopped: ...]`, failures as `[error] ...`, nothing for a cancel.
#[must_use]
pub(crate) fn outcome_mirror_text(outcome: &Outcome) -> Option<String> {
    match outcome {
        Outcome::Answered(text) | Outcome::AskedUser(text) => Some(text.clone()),
        Outcome::Stopped(why) => Some(format!("[stopped: {why}]")),
        Outcome::Failed(why) => Some(format!("[error] {why}")),
        Outcome::InProgress | Outcome::Cancelled => None,
    }
}

/// Inserts `summary` into the newest-first chat list, replacing an existing row
/// for the same chat, keeping `ChatStore::list`'s order (`updated_at`
/// descending, id as the tie-break). Lets a save refresh the list without
/// re-reading every chat file from disk.
pub(crate) fn upsert_summary(list: &mut Vec<ChatSummary>, summary: ChatSummary) {
    list.retain(|s| s.id != summary.id);
    list.push(summary);
    list.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use ferrite_agent::chat::Chat;

    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn summary(title: &str, updated: i64) -> ChatSummary {
        let mut chat = Chat::new();
        chat.title = title.to_string();
        chat.updated_at = at(updated);
        chat.summary()
    }

    #[test]
    fn outcome_mirror_text_keeps_the_live_panel_wording() {
        assert_eq!(
            outcome_mirror_text(&Outcome::Answered("a".into())).as_deref(),
            Some("a")
        );
        assert_eq!(
            outcome_mirror_text(&Outcome::AskedUser("q?".into())).as_deref(),
            Some("q?")
        );
        assert_eq!(
            outcome_mirror_text(&Outcome::Stopped("step budget exhausted".into())).as_deref(),
            Some("[stopped: step budget exhausted]")
        );
        assert_eq!(
            outcome_mirror_text(&Outcome::Failed("boom".into())).as_deref(),
            Some("[error] boom")
        );
        assert_eq!(outcome_mirror_text(&Outcome::Cancelled), None);
        assert_eq!(outcome_mirror_text(&Outcome::InProgress), None);
    }

    #[test]
    fn step_result_text_drops_the_appended_element_table() {
        let obs = "clicked @3 (origin https://a.example)\n\nPAGE: A \u{2014} https://a.example\nELEMENTS (1 of 1):\n[1] link \"x\"";
        assert_eq!(
            step_result_text(obs),
            "clicked @3 (origin https://a.example)"
        );
        assert_eq!(step_result_text("error: no tab 9"), "error: no tab 9");
        // A read_page result *is* the page: keep it (record_step bounds it).
        assert!(step_result_text("PAGE: T \u{2014} u\nTEXT: hi").starts_with("PAGE: T"));
    }

    #[test]
    fn persisted_step_detail_redacts_typed_and_clipboard_text_only() {
        let typed = AgentAction::TypeText {
            selector: "@5".into(),
            text: "s3cret \u{2713}".into(),
        };
        let d = persisted_step_detail(&typed);
        assert!(
            !d.contains("s3cret") && d.contains("@5") && d.contains("8 chars"),
            "{d}"
        );
        assert_eq!(
            persisted_step_detail(&AgentAction::ClipboardWrite {
                text: "abcd".into()
            }),
            "4 chars"
        );
        assert_eq!(
            persisted_step_detail(&AgentAction::Navigate {
                url: "https://a.example".into()
            }),
            "https://a.example"
        );
    }

    #[test]
    fn upsert_summary_inserts_replaces_and_keeps_newest_first() {
        let mut list = vec![summary("old", 0), summary("mid", 10)];
        list.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        let newest = summary("new", 20);
        upsert_summary(&mut list, newest.clone());
        assert_eq!(
            list.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            ["new", "mid", "old"]
        );

        // Updating an existing chat replaces its row and re-sorts it to the top.
        let mut touched = list[2].clone();
        touched.updated_at = at(30);
        touched.turn_count = 5;
        upsert_summary(&mut list, touched.clone());
        assert_eq!(list.len(), 3, "no duplicate row");
        assert_eq!(list[0], touched);
    }
}
