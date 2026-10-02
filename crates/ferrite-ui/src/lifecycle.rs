//! Keeping the app quittable and its stalls on the record.
//!
//! The engine runs pages on threads of its own. When one of them panics or
//! blocks, the window can stop answering and quitting can wait on it forever.
//! Two small guards, both of which only write to the log or end the process:
//!
//! * a **quit watchdog**: once the person asks to close the window, the
//!   process ends within a few seconds whatever the engine is doing;
//! * a **stall note**: if the UI has not ticked for a while, one line says so
//!   (and how long), so a log of a frozen run shows when it froze.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a clean shutdown gets before the process is ended anyway.
const QUIT_GRACE: Duration = Duration::from_secs(3);

/// How long without a tick before the UI counts as stalled.
const STALL_AFTER: Duration = Duration::from_secs(8);

static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Called on every UI tick: the app is alive.
pub(crate) fn heartbeat() {
    LAST_TICK_MS.store(now_ms(), Ordering::Relaxed);
}

/// Ends the process after the grace period unless it has already ended. Call
/// when the person asks to quit.
pub(crate) fn exit_soon() {
    let _ = std::thread::Builder::new()
        .name("ferrite-quit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(QUIT_GRACE);
            eprintln!("[ferrite] shutdown did not finish in {QUIT_GRACE:?}; ending the process");
            std::process::exit(0);
        });
}

/// Starts the stall note. Idempotent in effect: call once at launch.
pub(crate) fn start_stall_note() {
    let _ = std::thread::Builder::new()
        .name("ferrite-stall-note".into())
        .spawn(|| {
            let mut noted = false;
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let last = LAST_TICK_MS.load(Ordering::Relaxed);
                // No tick yet (the engine has not started): nothing to be stalled.
                let idle = if last == 0 { None } else { stalled_for(now_ms(), last) };
                match (idle, noted) {
                    (Some(d), false) => {
                        eprintln!(
                            "[ferrite] the UI has not responded for {}s (a page or the engine is probably stuck)",
                            d.as_secs()
                        );
                        noted = true;
                    }
                    (None, true) => {
                        eprintln!("[ferrite] the UI is responding again");
                        noted = false;
                    }
                    _ => {}
                }
            }
        });
}

/// How long the UI has been silent, when that is long enough to matter.
fn stalled_for(now: u64, last: u64) -> Option<Duration> {
    let idle = Duration::from_millis(now.saturating_sub(last));
    (idle >= STALL_AFTER).then_some(idle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stall_is_only_reported_once_it_is_long_enough_to_matter() {
        assert_eq!(stalled_for(10_000, 9_000), None);
        assert_eq!(stalled_for(20_000, 10_000), Some(Duration::from_secs(10)));
        // A clock that went backwards is not a stall.
        assert_eq!(stalled_for(5_000, 9_000), None);
    }
}
