//! Agent-side entries for the activity trace: what the agent did and decided,
//! written next to the model calls that `ferrite_model` already records, so
//! the Audit panel reads as one timeline.

use ferrite_model::trace::{global, TraceBackend, TraceEvent};

/// Records one agent event. `latency_ms` is 0 when the entry is not a timed
/// operation (run start, consent request, run end).
pub(crate) fn record(
    stage: &str,
    request: &str,
    response: &str,
    ok: bool,
    note: &str,
    latency_ms: u64,
) {
    let mut event = TraceEvent::new(TraceBackend::Agent, stage, "");
    event.request = request.to_string();
    event.response = response.to_string();
    event.ok = ok;
    event.note = note.to_string();
    event.latency_ms = latency_ms;
    global().record(event);
}

/// `1250` -> `"1.3 s"`, `22` -> `"22 ms"`.
pub(crate) fn format_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else {
        format!("{:.1} s", ms as f64 / 1_000.0)
    }
}

fn mean_ms<'a>(events: impl Iterator<Item = &'a TraceEvent>) -> Option<u64> {
    let times: Vec<u64> = events
        .filter(|e| e.ok && !e.cached)
        .map(|e| e.latency_ms)
        .collect();
    (!times.is_empty()).then(|| times.iter().sum::<u64>() / times.len() as u64)
}

/// Answers "is Laya making things faster?" from the events held: what asking
/// Laya cost *in total* (answers, declined answers and failures all cost a
/// round trip, and the LLM step still follows when Laya does not decide) against
/// the LLM time the used answers avoided.
pub(crate) fn laya_effect_summary(events: &[TraceEvent]) -> String {
    let llm_step = mean_ms(
        events
            .iter()
            .filter(|e| e.backend == TraceBackend::Llm && e.stage == "agent step"),
    );
    let asked: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| e.backend == TraceBackend::Laya && e.stage == "browser step")
        .collect();
    let used = events
        .iter()
        .filter(|e| {
            e.backend == TraceBackend::Laya
                && e.stage == "fast-lane verdict"
                && e.note.starts_with("accepted")
        })
        .count();
    let pauses = events
        .iter()
        .filter(|e| e.backend == TraceBackend::Laya && e.stage == "fast lane paused")
        .count();
    if asked.is_empty() {
        return match llm_step {
            Some(ms) => format!(
                "Laya has made no decisions yet; every step ran on the LLM (about {} each).",
                format_ms(ms)
            ),
            None => {
                "No model calls yet. Start an agent task and every call appears here.".to_string()
            }
        };
    }
    let failed = asked.iter().filter(|e| !e.ok).count();
    let answered = asked.len() - failed;
    let declined = answered.saturating_sub(used);
    let spent: u64 = asked.iter().map(|e| e.latency_ms).sum();
    let mut line = format!(
        "Laya was asked {} time(s): its answer was used {used} time(s), declined {declined} time(s) \
         (the LLM decided instead) and failed {failed} time(s) (timed out or errored). Asking took {} in total.",
        asked.len(),
        format_ms(spent)
    );
    match llm_step {
        Some(llm) => {
            let avoided = llm * used as u64;
            if avoided > spent {
                line.push_str(&format!(
                    " The {used} used step(s) skipped about {} of LLM time, so the fast lane saved about {} net.",
                    format_ms(avoided),
                    format_ms(avoided - spent)
                ));
            } else {
                line.push_str(&format!(
                    " The {used} used step(s) skipped about {} of LLM time (an LLM step averages {}), \
                     so the fast lane cost about {} net: it is not making things faster here.",
                    format_ms(avoided),
                    format_ms(llm),
                    format_ms(spent - avoided)
                ));
            }
        }
        None => line.push_str(" No LLM agent step has run yet to compare against."),
    }
    if pauses > 0 {
        line.push_str(&format!(
            " It paused itself {pauses} time(s) because asking was not paying off."
        ));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(backend: TraceBackend, stage: &str, ms: u64, note: &str) -> TraceEvent {
        let mut e = TraceEvent::new(backend, stage, "m");
        e.latency_ms = ms;
        e.note = note.to_string();
        e
    }

    #[test]
    fn milliseconds_read_naturally() {
        assert_eq!(format_ms(22), "22 ms");
        assert_eq!(format_ms(999), "999 ms");
        assert_eq!(format_ms(1_260), "1.3 s");
    }

    #[test]
    fn the_summary_says_so_when_nothing_has_run() {
        assert!(laya_effect_summary(&[]).contains("No model calls yet"));
    }

    #[test]
    fn the_summary_says_so_when_only_the_llm_ran() {
        let events = [ev(TraceBackend::Llm, "agent step", 3_000, "")];
        let text = laya_effect_summary(&events);
        assert!(
            text.contains("no decisions yet") && text.contains("3.0 s"),
            "{text}"
        );
    }

    fn failed(mut e: TraceEvent) -> TraceEvent {
        e.ok = false;
        e
    }

    #[test]
    fn the_summary_reports_a_net_saving_when_laya_is_fast_and_used() {
        let events = [
            ev(TraceBackend::Llm, "agent step", 3_000, ""),
            ev(TraceBackend::Laya, "browser step", 20, ""),
            ev(TraceBackend::Laya, "browser step", 40, ""),
            ev(
                TraceBackend::Laya,
                "fast-lane verdict",
                20,
                "accepted: Click",
            ),
            ev(
                TraceBackend::Laya,
                "fast-lane verdict",
                40,
                "fell back to the LLM: low-op-confidence",
            ),
        ];
        let text = laya_effect_summary(&events);
        assert!(text.contains("asked 2 time(s)"), "{text}");
        assert!(
            text.contains("used 1 time(s), declined 1 time(s)"),
            "{text}"
        );
        assert!(text.contains("failed 0 time(s)"), "{text}");
        assert!(text.contains("took 60 ms in total"), "{text}");
        // 1 used x 3000 ms avoided - 60 ms spent = 2940 ms
        assert!(text.contains("saved about 2.9 s net"), "{text}");
    }

    #[test]
    fn the_owners_real_trace_is_reported_as_a_net_loss() {
        // 6 calls (3 timed out at 1.5 s, 3 answered in ~1.4 s), 1 used; an LLM
        // step averaged 2.2 s. The old summary said "saved about 831 ms".
        let mut events = vec![ev(TraceBackend::Llm, "agent step", 2_200, "")];
        for _ in 0..3 {
            events.push(failed(ev(TraceBackend::Laya, "browser step", 1_500, "")));
            events.push(ev(TraceBackend::Laya, "browser step", 1_400, ""));
        }
        events.push(ev(
            TraceBackend::Laya,
            "fast-lane verdict",
            1_400,
            "accepted: Click",
        ));
        for _ in 0..2 {
            events.push(ev(
                TraceBackend::Laya,
                "fast-lane verdict",
                1_400,
                "fell back to the LLM: low-op-confidence",
            ));
        }
        let text = laya_effect_summary(&events);
        assert!(text.contains("failed 3 time(s)"), "{text}");
        assert!(text.contains("declined 2 time(s)"), "{text}");
        assert!(text.contains("took 8.7 s in total"), "{text}");
        assert!(text.contains("cost about 6.5 s net"), "{text}");
        assert!(!text.contains("saved"), "{text}");
    }

    #[test]
    fn a_slower_laya_is_reported_as_a_cost() {
        let events = [
            ev(TraceBackend::Llm, "agent step", 10, ""),
            ev(TraceBackend::Laya, "browser step", 500, ""),
            ev(
                TraceBackend::Laya,
                "fast-lane verdict",
                500,
                "accepted: Click",
            ),
        ];
        let text = laya_effect_summary(&events);
        assert!(text.contains("not making things faster"), "{text}");
    }

    #[test]
    fn pauses_are_mentioned() {
        let events = [
            ev(TraceBackend::Llm, "agent step", 2_000, ""),
            failed(ev(TraceBackend::Laya, "browser step", 1_500, "")),
            ev(
                TraceBackend::Laya,
                "fast lane paused",
                0,
                "paused for 4 steps",
            ),
        ];
        assert!(laya_effect_summary(&events).contains("paused itself 1 time(s)"));
    }

    #[test]
    fn an_agent_event_lands_in_the_global_trace_with_its_fields() {
        record("action", "click @3", "clicked", true, "chosen by Laya", 12);
        let found = global()
            .snapshot()
            .into_iter()
            .rev()
            .find(|e| e.backend == TraceBackend::Agent && e.request == "click @3")
            .expect("recorded");
        assert_eq!(found.stage, "action");
        assert_eq!(found.response, "clicked");
        assert_eq!(found.note, "chosen by Laya");
        assert_eq!(found.latency_ms, 12);
        assert!(found.ok);
    }
}
