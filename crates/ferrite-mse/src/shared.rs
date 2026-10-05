//! What a `MediaSource`'s buffers and its player share.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::{Sample, TrackBuffer, TrackInfo};

/// The most encoded data a `MediaSource` keeps, as Chrome's order of magnitude.
pub const DEFAULT_QUOTA: usize = 200 * 1024 * 1024;

/// Played data is kept this long behind the playhead when the quota forces eviction.
const KEEP_BEHIND: i64 = 1_000_000_000;

/// An append would take the buffers past their quota even after evicting what has been
/// played (`QuotaExceededError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaExceeded;

/// What the player's feeding thread for a track is told next.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// The next frame to hand to the decoder.
    Sample(Sample),
    /// The page seeked: drop whatever was queued and restart at `position` (nanoseconds).
    /// Frames follow once the buffers hold the time.
    Flush { position: i64 },
    /// Nothing to give yet (the wait ran out): check for shutdown and ask again.
    Wait,
    /// The page ended the stream and every frame was handed over.
    Eos,
    /// The `MediaSource` is detached or dropped: stop.
    Closed,
}

struct Slot {
    info: TrackInfo,
    buf: TrackBuffer,
}

struct State {
    slots: Vec<Slot>,
    ended: bool,
    closed: bool,
    duration: Option<i64>,
    /// Bumped by every seek, so feeders know to restart.
    epoch: u64,
    seek_to: i64,
    /// The playhead, for eviction.
    position: i64,
}

/// The tracks of one `MediaSource`, shared by its `SourceBuffer`s (which append and
/// remove) and the player's feeding threads (which read).
pub struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    quota: usize,
}

impl Shared {
    pub fn new() -> Arc<Shared> {
        Shared::with_quota(DEFAULT_QUOTA)
    }

    pub fn with_quota(quota: usize) -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State {
                slots: Vec::new(),
                ended: false,
                closed: false,
                duration: None,
                epoch: 0,
                seek_to: 0,
                position: 0,
            }),
            wake: Condvar::new(),
            quota,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves the buffers as they were; carry on.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers a track from an initialization segment; returns its slot. A track
    /// that is already known (the same container id, kind and codec) keeps its slot and
    /// its frames, as a repeated initialization segment does.
    pub fn add_track(&self, info: TrackInfo) -> usize {
        let mut st = self.lock();
        if let Some(i) = st
            .slots
            .iter()
            .position(|s| s.info.id == info.id && s.info.kind == info.kind)
        {
            st.slots[i].info = info;
            return i;
        }
        st.slots.push(Slot {
            info,
            buf: TrackBuffer::new(),
        });
        self.wake.notify_all();
        st.slots.len() - 1
    }

    pub fn slot_count(&self) -> usize {
        self.lock().slots.len()
    }

    pub fn track_info(&self, slot: usize) -> Option<TrackInfo> {
        self.lock().slots.get(slot).map(|s| s.info.clone())
    }

    /// Adds frames to a track, evicting played data first if the quota needs it.
    pub fn append(&self, slot: usize, samples: Vec<Sample>) -> Result<(), QuotaExceeded> {
        let incoming: usize = samples.iter().map(|s| s.data.len()).sum();
        let mut st = self.lock();
        if slot >= st.slots.len() {
            return Ok(());
        }
        let used: usize = st.slots.iter().map(|s| s.buf.bytes()).sum();
        if used + incoming > self.quota {
            let need = used + incoming - self.quota;
            let before = st.position - KEEP_BEHIND;
            let mut freed = 0;
            for s in st.slots.iter_mut() {
                freed += s.buf.evict_before(before, need.saturating_sub(freed));
                if freed >= need {
                    break;
                }
            }
            if freed < need {
                return Err(QuotaExceeded);
            }
        }
        st.slots[slot].buf.append(samples);
        st.ended = false;
        self.wake.notify_all();
        Ok(())
    }

    /// `SourceBuffer.remove(start, end)`.
    pub fn remove(&self, slot: usize, start: i64, end: i64) {
        let mut st = self.lock();
        if let Some(s) = st.slots.get_mut(slot) {
            s.buf.remove(start, end);
        }
        self.wake.notify_all();
    }

    /// One track's buffered ranges.
    pub fn buffered(&self, slot: usize) -> Vec<(i64, i64)> {
        self.lock()
            .slots
            .get(slot)
            .map(|s| s.buf.buffered())
            .unwrap_or_default()
    }

    /// What the media element reports as `buffered`: the time every track has. After
    /// the stream ends, a track that stops short of the longest one counts as reaching
    /// it.
    pub fn buffered_all(&self) -> Vec<(i64, i64)> {
        let st = self.lock();
        let mut all: Vec<Vec<(i64, i64)>> = st.slots.iter().map(|s| s.buf.buffered()).collect();
        if all.is_empty() {
            return Vec::new();
        }
        if st.ended {
            let highest = all
                .iter()
                .filter_map(|r| r.last().map(|l| l.1))
                .max()
                .unwrap_or(0);
            for ranges in all.iter_mut() {
                if let Some(last) = ranges.last_mut() {
                    last.1 = highest;
                }
            }
        }
        let mut out = all[0].clone();
        for other in &all[1..] {
            out = intersect(&out, other);
        }
        out
    }

    /// The bytes held across all tracks.
    pub fn bytes(&self) -> usize {
        self.lock().slots.iter().map(|s| s.buf.bytes()).sum()
    }

    /// `endOfStream()`.
    pub fn set_ended(&self, ended: bool) {
        self.lock().ended = ended;
        self.wake.notify_all();
    }

    pub fn ended(&self) -> bool {
        self.lock().ended
    }

    pub fn set_duration(&self, duration: Option<i64>) {
        self.lock().duration = duration;
    }

    pub fn duration(&self) -> Option<i64> {
        self.lock().duration
    }

    /// The playhead, which decides what may be evicted.
    pub fn set_position(&self, position: i64) {
        self.lock().position = position;
    }

    pub fn position(&self) -> i64 {
        self.lock().position
    }

    /// The page (or the user) seeked to `time` nanoseconds: feeders restart there.
    pub fn seek(&self, time: i64) {
        let mut st = self.lock();
        st.epoch += 1;
        st.seek_to = time;
        st.position = time;
        self.wake.notify_all();
    }

    /// Detaches: every feeder is told to stop.
    pub fn close(&self) {
        self.lock().closed = true;
        self.wake.notify_all();
    }

    pub fn closed(&self) -> bool {
        self.lock().closed
    }

    /// A feeder for a track, starting at the beginning of the buffered data.
    pub fn handle(self: &Arc<Shared>, slot: usize) -> TrackHandle {
        let epoch = self.lock().epoch;
        TrackHandle {
            shared: Arc::clone(self),
            slot,
            epoch,
            cursor: Cursor::Start,
        }
    }
}

fn intersect(a: &[(i64, i64)], b: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let start = a[i].0.max(b[j].0);
        let end = a[i].1.min(b[j].1);
        if start < end {
            out.push((start, end));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

enum Cursor {
    /// From the first buffered frame.
    Start,
    /// After a seek, until the buffers hold a sync sample to start from.
    Seek(i64),
    /// After the frame decoded at this time.
    After(i64),
}

/// A feeder's view of one track: hands the player frames in decode order, and follows
/// seeks.
pub struct TrackHandle {
    shared: Arc<Shared>,
    slot: usize,
    epoch: u64,
    cursor: Cursor,
}

impl TrackHandle {
    /// The next thing the player's thread must do, waiting up to `timeout` for it.
    pub fn next(&mut self, timeout: Duration) -> Next {
        let deadline = Instant::now() + timeout;
        let mut st = self.shared.lock();
        loop {
            if st.closed {
                return Next::Closed;
            }
            if st.epoch != self.epoch {
                self.epoch = st.epoch;
                self.cursor = Cursor::Seek(st.seek_to);
                return Next::Flush {
                    position: st.seek_to,
                };
            }
            let Some(slot) = st.slots.get(self.slot) else {
                return Next::Closed;
            };
            if let Cursor::Seek(time) = self.cursor
                && let Some(c) = slot.buf.seek_cursor(time)
            {
                self.cursor = Cursor::After(c);
            }
            let sample = match self.cursor {
                Cursor::Start => slot.buf.next_after(None),
                Cursor::After(dts) => slot.buf.next_after(Some(dts)),
                Cursor::Seek(_) => None,
            };
            if let Some(sample) = sample {
                self.cursor = Cursor::After(sample.dts);
                return Next::Sample(sample.clone());
            }
            if st.ended {
                return Next::Eos;
            }
            let now = Instant::now();
            if now >= deadline {
                return Next::Wait;
            }
            st = self
                .shared
                .wake
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// The slot this feeder reads.
    pub fn slot(&self) -> usize {
        self.slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Codec, TrackKind};

    const MS: i64 = 1_000_000;

    fn info(id: u32, kind: TrackKind) -> TrackInfo {
        TrackInfo {
            id,
            kind,
            codec: Codec::Vp8,
            width: 0,
            height: 0,
            sample_rate: 0,
            channels: 0,
        }
    }

    fn run(from: i64, count: i64, gop: i64) -> Vec<Sample> {
        (0..count)
            .map(|i| Sample {
                pts: (from + i * 40) * MS,
                dts: (from + i * 40) * MS,
                duration: 40 * MS,
                key: i % gop == 0,
                data: vec![0; 100],
            })
            .collect()
    }

    const SHORT: Duration = Duration::from_millis(20);

    #[test]
    fn a_feeder_gets_frames_in_order_then_waits_then_ends() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 3, 3)).unwrap();
        let mut h = shared.handle(slot);
        for want in [0, 40, 80] {
            match h.next(SHORT) {
                Next::Sample(s) => assert_eq!(s.pts, want * MS),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(h.next(SHORT), Next::Wait);
        shared.append(slot, run(120, 1, 1)).unwrap();
        assert!(matches!(h.next(SHORT), Next::Sample(s) if s.pts == 120 * MS));
        shared.set_ended(true);
        assert_eq!(h.next(SHORT), Next::Eos);
    }

    #[test]
    fn an_append_wakes_a_waiting_feeder() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        let mut h = shared.handle(slot);
        let t = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                shared.append(slot, run(0, 1, 1)).unwrap();
            })
        };
        let started = Instant::now();
        let next = h.next(Duration::from_secs(5));
        assert!(matches!(next, Next::Sample(_)), "{next:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
        t.join().unwrap();
    }

    #[test]
    fn a_seek_flushes_and_restarts_at_the_sync_sample() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 15, 5)).unwrap();
        let mut h = shared.handle(slot);
        assert!(matches!(h.next(SHORT), Next::Sample(_)));
        shared.seek(250 * MS);
        assert_eq!(h.next(SHORT), Next::Flush { position: 250 * MS });
        // Restarts at the key at 200, not at 250.
        assert!(matches!(h.next(SHORT), Next::Sample(s) if s.pts == 200 * MS && s.key));
    }

    #[test]
    fn a_seek_into_a_gap_waits_for_the_data() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 5, 5)).unwrap();
        let mut h = shared.handle(slot);
        shared.seek(10_000 * MS);
        assert!(matches!(h.next(SHORT), Next::Flush { .. }));
        assert_eq!(h.next(SHORT), Next::Wait);
        shared.append(slot, run(10_000, 5, 5)).unwrap();
        assert!(matches!(h.next(SHORT), Next::Sample(s) if s.pts == 10_000 * MS));
    }

    #[test]
    fn close_stops_feeders() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        let mut h = shared.handle(slot);
        shared.close();
        assert_eq!(h.next(SHORT), Next::Closed);
    }

    #[test]
    fn buffered_all_is_the_intersection_of_the_tracks() {
        let shared = Shared::new();
        let v = shared.add_track(info(1, TrackKind::Video));
        let a = shared.add_track(info(2, TrackKind::Audio));
        assert!(shared.buffered_all().is_empty());
        shared.append(v, run(0, 25, 5)).unwrap(); // 0..1000
        shared.append(a, run(0, 20, 1)).unwrap(); // 0..800
        assert_eq!(shared.buffered_all(), vec![(0, 800 * MS)]);
        // At the end of the stream the shorter track counts as reaching the longer.
        shared.set_ended(true);
        assert_eq!(shared.buffered_all(), vec![(0, 1000 * MS)]);
    }

    #[test]
    fn a_repeated_init_keeps_the_slot_and_its_frames() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 3, 3)).unwrap();
        assert_eq!(shared.add_track(info(1, TrackKind::Video)), slot);
        assert_eq!(shared.buffered(slot).len(), 1);
        assert_eq!(shared.slot_count(), 1);
    }

    #[test]
    fn the_quota_evicts_what_was_played_and_refuses_the_rest() {
        let shared = Shared::with_quota(2000);
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 15, 5)).unwrap(); // 1500 bytes
        // Nothing played: no room.
        assert_eq!(shared.append(slot, run(600, 10, 5)), Err(QuotaExceeded));
        // With the playhead at 3 s, the old groups can go.
        shared.set_position(3000 * MS);
        shared.append(slot, run(600, 10, 5)).unwrap();
        assert!(shared.bytes() <= 2000);
    }

    #[test]
    fn intersecting_ranges() {
        let a = vec![(0, 10), (20, 30)];
        let b = vec![(5, 25)];
        assert_eq!(intersect(&a, &b), vec![(5, 10), (20, 25)]);
        assert!(intersect(&a, &[]).is_empty());
    }
}
