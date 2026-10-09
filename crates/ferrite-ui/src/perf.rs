//! `FERRITE_PERF=1`: one line on stderr every five seconds saying where the UI
//! thread's engine work went, for a machine where a profiler is not at hand.
//!
//! It answers the first question about a busy page: is the engine making new
//! pictures all the time (an animation, a video, a page that keeps changing),
//! and how long does the UI thread spend driving the engine and copying its
//! pixels. Time on the engine's own threads (script, layout, painting) shows
//! up in the process's CPU, not here.

use std::time::{Duration, Instant};

/// How often a summary is printed.
const WINDOW: Duration = Duration::from_secs(5);

/// Counts for the current window.
pub(crate) struct PerfStats {
    since: Instant,
    ticks: u32,
    pictures: u32,
    pump: Duration,
    read: Duration,
    tick: Duration,
    size: (u32, u32),
}

impl PerfStats {
    /// Counting is on only when `FERRITE_PERF` is set to something other than
    /// `0` or `off`.
    pub(crate) fn from_env() -> Option<Self> {
        let on = std::env::var("FERRITE_PERF").is_ok_and(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("off")
        });
        on.then(|| Self::starting(Instant::now()))
    }

    fn starting(now: Instant) -> Self {
        Self {
            since: now,
            ticks: 0,
            pictures: 0,
            pump: Duration::ZERO,
            read: Duration::ZERO,
            tick: Duration::ZERO,
            size: (0, 0),
        }
    }

    /// Adds one engine tick; returns the summary line when a window ends.
    pub(crate) fn record(
        &mut self,
        now: Instant,
        pump: Duration,
        read: Duration,
        tick: Duration,
        new_picture: Option<(u32, u32)>,
    ) -> Option<String> {
        self.ticks += 1;
        self.pump += pump;
        self.read += read;
        self.tick += tick;
        if let Some(size) = new_picture {
            self.pictures += 1;
            self.size = size;
        }
        let span = now.duration_since(self.since);
        if span < WINDOW {
            return None;
        }
        let secs = span.as_secs_f64();
        let line = format!(
            "[ferrite-perf] {secs:.1} s: {} ticks ({:.0}/s), {} new pictures ({:.0}/s, {}x{} px); \
             UI thread: engine pump {} ms, page sync and pixel read {} ms, whole tick {} ms ({:.0}% of one core)",
            self.ticks,
            f64::from(self.ticks) / secs,
            self.pictures,
            f64::from(self.pictures) / secs,
            self.size.0,
            self.size.1,
            self.pump.as_millis(),
            self.read.as_millis(),
            self.tick.as_millis(),
            self.tick.as_secs_f64() / secs * 100.0,
        );
        *self = Self::starting(now);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_comes_once_per_window_and_counts_restart() {
        let start = Instant::now();
        let mut stats = PerfStats::starting(start);
        let ms = Duration::from_millis;
        assert!(stats
            .record(start + ms(16), ms(2), ms(3), ms(6), Some((800, 600)))
            .is_none());
        let line = stats
            .record(start + ms(5000), ms(2), ms(3), ms(6), None)
            .expect("a summary after five seconds");
        assert!(line.contains("2 ticks"), "{line}");
        assert!(line.contains("1 new pictures"), "{line}");
        assert!(line.contains("800x600"), "{line}");
        assert!(line.contains("engine pump 4 ms"), "{line}");
        assert!(line.contains("pixel read 6 ms"), "{line}");
        // The next window starts empty.
        assert!(stats
            .record(start + ms(5016), ms(1), ms(1), ms(1), None)
            .is_none());
        assert_eq!(stats.ticks, 1);
    }
}
