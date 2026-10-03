//! What each tab collects about itself between ticks: its console and network
//! log, a control the page is waiting on, and a crash notice. One entry per tab,
//! kept in step with the tab strip (`push_tab_state`, `close_tab_at`).
//!
//! [`drain_all`] is the per-tick work. It is O(new entries): each session's
//! queues are emptied (an empty queue is one `RefCell` borrow), the entries are
//! appended to that tab's log, and nothing is cloned when nothing happened. It
//! asks a session for its pending control only while the tab has none (the
//! engine hands over a copy each time), so a long `<select>` is not copied sixty
//! times a second while it is open.

use iced::widget::scrollable;
use iced::Task;

use crate::controls::{self, PendingControl};
use crate::crash::CrashState;
use crate::devtools::{self, DevTab, TabLog, MIRROR_PER_TICK};
use crate::{FerriteBrowser, FerriteBrowserMessage};

/// One tab's diagnostics and pending page controls.
#[derive(Default)]
pub(crate) struct TabDiag {
    pub log: TabLog,
    pub control: Option<PendingControl>,
    pub crash: Option<CrashState>,
}

/// Empties every session's queues into the tabs' logs; shows crashes and page
/// controls; mirrors warnings and errors to the log file. Returns a task that
/// keeps the Console pinned to its newest message or focuses a new prompt.
pub(crate) fn drain_all(state: &mut FerriteBrowser) -> Task<FerriteBrowserMessage> {
    let mut tasks: Vec<Task<FerriteBrowserMessage>> = Vec::new();

    for panic in ferrite_servo::diag::take_panics() {
        state.engine_log.push_panic(panic);
    }

    let active = state.active_tab;
    let preserve = state.devtools.preserve;
    let area = state.content_area_size.get();
    let mut mirrored = 0usize;
    let mut skipped = 0usize;
    let mut new_console_on_active = false;

    for (&index, session) in state.servo_sessions.iter_mut() {
        let Some(diag) = state.tab_diag.get_mut(index) else {
            continue;
        };
        // The engine's separate error-only queue is superseded by the full
        // console queue; empty it so it cannot grow without bound.
        let _ = session.take_console_errors();
        let console = session.take_console_entries();
        let net = session.take_net_events();
        if !console.is_empty() || !net.is_empty() {
            for entry in &console {
                if let Some(line) = devtools::stderr_line(index, entry.level, &entry.message) {
                    if mirrored < MIRROR_PER_TICK {
                        eprintln!("{line}");
                        mirrored += 1;
                    } else {
                        skipped += 1;
                    }
                }
            }
            let added = diag.log.ingest(console, net, preserve);
            if index == active && added.console > 0 {
                new_console_on_active = true;
            }
        }

        if let Some(note) = session.take_crash() {
            eprintln!(
                "[ferrite-ui] tab {}: the page crashed: {}",
                index + 1,
                crate::truncate(&note.reason, 400)
            );
            state.engine_log.push_crash(index, &note);
            diag.crash = Some(CrashState::new(note));
            crate::wake_flag(&mut state.busy_ticks);
        }

        if diag.control.is_none() {
            if let Some(control) = session.page_control() {
                let pending = PendingControl::new(control, area);
                if index == active {
                    tasks.push(controls::focus_task(&pending));
                }
                diag.control = Some(pending);
                crate::wake_flag(&mut state.busy_ticks);
            }
        }
    }
    if skipped > 0 {
        eprintln!("[console] {skipped} more warnings or errors this tick were not written to this log (see the DevTools Console)");
    }

    if new_console_on_active
        && state.show_js_console
        && state.devtools.tab == DevTab::Console
        && state.devtools.pinned
    {
        tasks.push(scrollable::snap_to(
            devtools::console_scroll_id(),
            scrollable::RelativeOffset::END,
        ));
    }
    Task::batch(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_sessions_a_tick_does_nothing_and_costs_nothing() {
        let mut state = FerriteBrowser::default();
        let _ = drain_all(&mut state);
        assert_eq!(state.tab_diag[0].log.console_len(), 0);
        assert!(state.tab_diag[0].control.is_none());
        assert!(state.tab_diag[0].crash.is_none());
    }

    #[test]
    fn a_new_tab_gets_its_own_diagnostics_and_closing_one_drops_them() {
        let mut state = FerriteBrowser::default();
        crate::push_tab_state(&mut state);
        assert_eq!(state.tab_diag.len(), state.tabs.len());
        state.tab_diag[0].log.push_input("a");
        state.tab_diag[1].log.push_input("b");
        crate::close_tab_at(&mut state, 0);
        assert_eq!(state.tab_diag.len(), 1);
        assert_eq!(
            state.tab_diag[0]
                .log
                .console_rows()
                .next()
                .map(|r| r.message.as_str()),
            Some("b"),
            "the surviving tab keeps its own log"
        );
    }
}
