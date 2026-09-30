//! Is the Laya fast lane paying for itself? (ADR-015)
//!
//! The fast lane asks Laya first and runs the normal LLM step only when Laya
//! declines, so **every Laya attempt that does not end in a used answer is
//! pure added latency**: a timeout costs the whole timeout, an answer that
//! falls below the confidence gates costs its round trip, and in both cases the
//! LLM call still follows. A Laya server that is slow (CPU inference, a cold
//! start, a busy machine) or rarely confident therefore makes the agent
//! *slower* than having no Laya at all — which is what the owner's first real
//! trace showed: half the calls timed out at the 1.5 s cap, the rest averaged
//! 1.4 s, and an LLM step averaged 2.2 s.
//!
//! This module is the pure decision logic for "keep asking?", with no I/O and
//! no clock, so it is testable down to the step. It looks at what the last
//! few attempts really cost and really bought:
//!
//! * **Breaker.** Two requests in a row that fail or time out pause the fast
//!   lane for a number of *steps* (not seconds: the agent loop is what
//!   advances).
//! * **Value rule.** With enough evidence, asking is worth it only while
//!   `accept_rate × mean LLM step > mean cost of an attempt`, where the cost
//!   counts failed and declined attempts too. Otherwise the lane pauses.
//! * **Recovery.** A pause ends by itself; the next attempts are judged on
//!   fresh evidence. Each consecutive pause doubles in length (capped), and a
//!   judgement that finds the lane profitable resets that.
//!
//! It never touches safety: a paused lane means the step runs on the LLM,
//! exactly as if Laya were not configured (ADR-009), and the LLM step goes
//! through the same rejection/consent path as ever.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Attempts (and LLM steps) remembered.
const WINDOW: usize = 10;
/// Attempts needed before the value rule may pause the lane.
const MIN_ATTEMPTS: usize = 3;
/// LLM step timings needed before the value rule has anything to compare to.
const MIN_LLM_SAMPLES: usize = 2;
/// Consecutive failed requests that open the breaker on their own.
const FAILURES_TO_PAUSE: u32 = 2;
/// Steps skipped by the first pause.
const BASE_PAUSE_STEPS: u32 = 4;
/// Longest pause, in steps.
const MAX_PAUSE_STEPS: u32 = 48;
/// Cap on the doubling exponent (`4 << 4` = 64, then capped by the above).
const MAX_LEVEL: u32 = 4;

/// Whether the next step should ask Laya at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Ask Laya.
    Ask,
    /// Skip Laya for this step; the LLM decides.
    Skip,
}

/// Why the lane just paused, for the activity trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paused {
    /// One sentence a person can read in the Activity panel.
    pub reason: String,
    /// How many agent steps will skip Laya.
    pub steps: u32,
}

#[derive(Debug, Clone, Copy)]
struct Attempt {
    /// Wall-clock cost of the request, whatever its outcome.
    ms: u64,
    /// The request itself failed (timeout, connection, HTTP, bad body).
    failed: bool,
    /// Its answer was executed instead of an LLM step.
    used: bool,
}

#[derive(Debug, Default)]
struct State {
    attempts: VecDeque<Attempt>,
    llm_ms: VecDeque<u64>,
    consecutive_failures: u32,
    pause_left: u32,
    level: u32,
}

/// Shared, thread-safe fast-lane governor. One per [`crate::laya::LayaClient`]
/// owner; cheap to share behind an `Arc`.
#[derive(Debug, Default)]
pub struct LaneGovernor {
    state: Mutex<State>,
}

fn push_capped<T>(queue: &mut VecDeque<T>, value: T) {
    if queue.len() == WINDOW {
        queue.pop_front();
    }
    queue.push_back(value);
}

impl LaneGovernor {
    /// A governor with no history: the lane is open.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A poisoned lock only means another thread panicked mid-update of
        // plain counters; keep going with what is there.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Called once per agent step that *would* send a request. A step that
    /// skips consumes one step of the current pause.
    pub fn permit(&self) -> Gate {
        let mut state = self.lock();
        if state.pause_left > 0 {
            state.pause_left -= 1;
            Gate::Skip
        } else {
            Gate::Ask
        }
    }

    /// How many more steps are currently paused (0 = the lane is open).
    #[cfg(test)]
    fn paused_steps_left(&self) -> u32 {
        self.lock().pause_left
    }

    /// Records the request itself: how long it took and whether it produced a
    /// valid answer. Returns `Some` when a failure just paused the lane.
    pub fn note_call(&self, ms: u64, ok: bool) -> Option<Paused> {
        let mut state = self.lock();
        push_capped(
            &mut state.attempts,
            Attempt {
                ms,
                failed: !ok,
                used: false,
            },
        );
        if ok {
            state.consecutive_failures = 0;
            // The verdict (used or declined) is what judges an answered call.
            return None;
        }
        state.consecutive_failures += 1;
        Self::judge(&mut state)
    }

    /// Records what happened to the answer of the call just noted: `used`
    /// means it cleared every gate and was executed instead of an LLM step.
    /// Returns `Some` when the value rule just paused the lane.
    pub fn note_verdict(&self, used: bool) -> Option<Paused> {
        let mut state = self.lock();
        if let Some(last) = state.attempts.back_mut().filter(|a| !a.failed) {
            last.used = used;
        }
        Self::judge(&mut state)
    }

    /// Records how long an LLM step took (the thing the fast lane competes
    /// with). Only steps that ran on the LLM count.
    pub fn note_llm_step(&self, ms: u64) {
        push_capped(&mut self.lock().llm_ms, ms);
    }

    fn judge(state: &mut State) -> Option<Paused> {
        if state.consecutive_failures >= FAILURES_TO_PAUSE {
            let reason = format!(
                "the last {} Laya requests failed or timed out",
                state.consecutive_failures
            );
            return Some(Self::open(state, reason));
        }
        if state.attempts.len() < MIN_ATTEMPTS || state.llm_ms.len() < MIN_LLM_SAMPLES {
            return None;
        }
        let n = state.attempts.len() as f64;
        let cost = state.attempts.iter().map(|a| a.ms as f64).sum::<f64>() / n;
        let used = state.attempts.iter().filter(|a| a.used).count() as f64;
        let llm = state.llm_ms.iter().map(|&ms| ms as f64).sum::<f64>() / state.llm_ms.len() as f64;
        let gain = used / n * llm;
        if gain > cost {
            // Earning its keep: a later pause starts short again.
            state.level = 0;
            return None;
        }
        let reason = format!(
            "each Laya attempt cost {cost:.0} ms and its answer was used {used:.0} of {n:.0} \
             times, saving about {gain:.0} ms per attempt against an LLM step of {llm:.0} ms"
        );
        Some(Self::open(state, reason))
    }

    fn open(state: &mut State, reason: String) -> Paused {
        let steps = (BASE_PAUSE_STEPS << state.level).min(MAX_PAUSE_STEPS);
        state.level = (state.level + 1).min(MAX_LEVEL);
        state.pause_left = steps;
        state.consecutive_failures = 0;
        // After the pause, judge on fresh evidence, not the run that caused it.
        state.attempts.clear();
        Paused { reason, steps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One answered call whose answer was (not) used.
    fn answered(g: &LaneGovernor, ms: u64, used: bool) -> Option<Paused> {
        assert!(g.note_call(ms, true).is_none());
        g.note_verdict(used)
    }

    fn with_llm(g: &LaneGovernor, ms: u64) {
        g.note_llm_step(ms);
        g.note_llm_step(ms);
    }

    #[test]
    fn a_new_governor_asks() {
        assert_eq!(LaneGovernor::new().permit(), Gate::Ask);
    }

    #[test]
    fn two_failures_in_a_row_pause_the_lane_for_a_few_steps() {
        let g = LaneGovernor::new();
        assert!(g.note_call(1_500, false).is_none());
        let paused = g.note_call(1_500, false).expect("second failure pauses");
        assert_eq!(paused.steps, BASE_PAUSE_STEPS);
        assert!(paused.reason.contains("failed or timed out"));
        for _ in 0..BASE_PAUSE_STEPS {
            assert_eq!(g.permit(), Gate::Skip);
        }
        assert_eq!(g.permit(), Gate::Ask, "the pause ends by itself");
    }

    #[test]
    fn an_answer_between_failures_resets_the_streak() {
        let g = LaneGovernor::new();
        g.note_call(1_500, false);
        assert!(answered(&g, 30, true).is_none());
        assert!(g.note_call(1_500, false).is_none());
    }

    #[test]
    fn the_owners_real_trace_is_paused() {
        // 6 attempts: 3 timed out at 1.5 s, 3 answered in ~1.4 s of which 1 was
        // used; an LLM step averaged 2.2 s. 1/6 x 2.2 s = 0.37 s gained against
        // ~1.45 s spent per attempt: it was a net loss, and must stop.
        let g = LaneGovernor::new();
        with_llm(&g, 2_200);
        let mut paused = None;
        for (ms, ok, used) in [
            (1_400, true, true),
            (1_500, false, false),
            (1_400, true, false),
            (1_500, false, false),
            (1_400, true, false),
        ] {
            let p = if ok {
                answered(&g, ms, used)
            } else {
                g.note_call(ms, false)
            };
            paused = paused.or(p);
        }
        assert!(paused.is_some(), "a lane slower than the LLM must pause");
    }

    #[test]
    fn a_fast_accurate_lane_is_never_paused() {
        let g = LaneGovernor::new();
        with_llm(&g, 2_200);
        for i in 0..40 {
            // 30 ms round trips, 2 of 3 answers used.
            assert!(
                answered(&g, 30, i % 3 != 0).is_none(),
                "paused a profitable lane at attempt {i}"
            );
            assert_eq!(g.permit(), Gate::Ask);
        }
    }

    #[test]
    fn a_fast_lane_that_is_always_declined_is_paused_on_cost() {
        // Cheap but never used: 100 ms spent, 0 gained.
        let g = LaneGovernor::new();
        with_llm(&g, 2_000);
        assert!(answered(&g, 100, false).is_none());
        assert!(answered(&g, 100, false).is_none());
        let paused = answered(&g, 100, false).expect("three useless attempts pause it");
        assert!(paused.reason.contains("used 0 of 3"));
    }

    #[test]
    fn the_value_rule_waits_for_evidence() {
        let g = LaneGovernor::new();
        // No LLM timings yet: nothing to compare to, so never pause on value.
        for _ in 0..6 {
            assert!(answered(&g, 900, false).is_none());
        }
        // Too few attempts, even with LLM timings.
        let g = LaneGovernor::new();
        with_llm(&g, 2_000);
        assert!(answered(&g, 900, false).is_none());
        assert!(answered(&g, 900, false).is_none());
    }

    #[test]
    fn consecutive_pauses_double_and_are_capped() {
        let g = LaneGovernor::new();
        let mut lengths = Vec::new();
        for _ in 0..7 {
            g.note_call(1_500, false);
            let p = g.note_call(1_500, false).expect("pauses");
            lengths.push(p.steps);
            while g.permit() == Gate::Skip {}
        }
        assert_eq!(lengths[..4], [4, 8, 16, 32]);
        assert!(lengths.iter().all(|&s| s <= MAX_PAUSE_STEPS));
        assert_eq!(*lengths.last().expect("some"), MAX_PAUSE_STEPS);
    }

    #[test]
    fn a_profitable_judgement_resets_the_backoff() {
        let g = LaneGovernor::new();
        g.note_call(1_500, false);
        g.note_call(1_500, false).expect("first pause");
        while g.permit() == Gate::Skip {}
        with_llm(&g, 2_200);
        for _ in 0..MIN_ATTEMPTS {
            assert!(answered(&g, 30, true).is_none());
        }
        g.note_call(1_500, false);
        let again = g.note_call(1_500, false).expect("pauses again");
        assert_eq!(again.steps, BASE_PAUSE_STEPS, "backoff restarted");
    }

    #[test]
    fn a_pause_forgets_the_run_that_caused_it() {
        let g = LaneGovernor::new();
        with_llm(&g, 2_000);
        for _ in 0..MIN_ATTEMPTS {
            answered(&g, 900, false);
        }
        assert!(g.paused_steps_left() > 0);
        while g.permit() == Gate::Skip {}
        // One fresh attempt is not enough to judge again.
        assert!(answered(&g, 900, false).is_none());
    }

    #[test]
    fn windows_are_bounded() {
        let g = LaneGovernor::new();
        for _ in 0..(WINDOW * 3) {
            g.note_llm_step(1_000);
        }
        assert_eq!(g.lock().llm_ms.len(), WINDOW);
    }
}
