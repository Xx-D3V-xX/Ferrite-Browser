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

/// One sentence answering "is Laya making things faster?" from the events held.
pub(crate) fn laya_effect_summary(events: &[TraceEvent]) -> String {
    let llm_step = mean_ms(
        events
            .iter()
            .filter(|e| e.backend == TraceBackend::Llm && e.stage == "agent step"),
    );
    let laya_call = mean_ms(
        events
            .iter()
            .filter(|e| e.backend == TraceBackend::Laya && e.stage == "browser step"),
    );
    let verdicts: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| e.backend == TraceBackend::Laya && e.stage == "fast-lane verdict")
        .collect();
    if verdicts.is_empty() {
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
    let used = verdicts
        .iter()
        .filter(|e| e.note.starts_with("accepted"))
        .count();
    let fell_back = verdicts.len() - used;
    let mut line = format!(
        "Laya was asked {} time(s) and its answer was used {used} time(s); {fell_back} fell back to the LLM.",
        verdicts.len()
    );
    if let Some(ms) = laya_call {
        line.push_str(&format!(" A Laya round trip averages {}.", format_ms(ms)));
    }
    match (llm_step, laya_call) {
        (Some(llm), Some(laya)) if llm > laya && used > 0 => line.push_str(&format!(
            " An LLM step averages {}, so the fast lane saved about {}.",
            format_ms(llm),
            format_ms((llm - laya) * used as u64)
        )),
        (Some(llm), Some(_)) => line.push_str(&format!(
            " An LLM step averages {}; no time was saved on average.",
            format_ms(llm)
        )),
        (None, _) => line.push_str(" No LLM agent step has run yet to compare against."),
        (_, None) => {}
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

    #[test]
    fn the_summary_counts_used_and_fallback_verdicts_and_the_time_saved() {
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
        assert!(text.contains("used 1 time(s); 1 fell back"), "{text}");
        assert!(text.contains("averages 30 ms"), "{text}");
        // (3000 - 30) * 1 used step = 2970 ms
        assert!(text.contains("saved about 3.0 s"), "{text}");
    }

    #[test]
    fn a_slower_laya_is_reported_as_no_saving_not_a_negative_one() {
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
        assert!(laya_effect_summary(&events).contains("no time was saved"));
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
