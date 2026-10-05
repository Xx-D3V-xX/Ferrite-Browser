//! What a `MediaSource`'s buffers and its player share.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::{Sample, TrackBuffer, TrackInfo};

/// The most encoded data a `MediaSource` keeps, as Chrome's order of magnitude.
pub const DEFAULT_QUOTA: usize = 200 * 1024 * 1024;

/// Seeks to the same time closer together than this are one seek.
const SEEK_COALESCE: Duration = Duration::from_millis(250);

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
    /// The track as each initialization segment described it; the last is current.
    infos: Vec<TrackInfo>,
    buf: TrackBuffer,
    /// Its `SourceBuffer` was removed: nothing reads or waits for it any more.
    retired: bool,
}

struct State {
    slots: Vec<Slot>,
    ended: bool,
    closed: bool,
    duration: Option<i64>,
    /// `SourceBuffer`s added, and how many of them have had an initialization segment.
    buffers: usize,
    initialized: usize,
    /// Bumped by every seek, so feeders know to restart.
    epoch: u64,
    seek_to: i64,
    seek_at: Option<Instant>,
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
                buffers: 0,
                initialized: 0,
                epoch: 0,
                seek_to: 0,
                seek_at: None,
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
    /// that is already known (the same container id and kind) keeps its slot and its
    /// frames, as a repeated initialization segment does; if its configuration changed,
    /// frames appended from now on carry the new one.
    pub fn add_track(&self, info: TrackInfo) -> usize {
        let mut st = self.lock();
        if let Some(i) = st.slots.iter().position(|s| {
            let last = &s.infos[s.infos.len() - 1];
            last.id == info.id && last.kind == info.kind
        }) {
            if st.slots[i].infos.last() != Some(&info) {
                st.slots[i].infos.push(info);
            }
            return i;
        }
        st.slots.push(Slot {
            infos: vec![info],
            buf: TrackBuffer::new(),
            retired: false,
        });
        self.wake.notify_all();
        st.slots.len() - 1
    }

    /// A `SourceBuffer` was added.
    pub fn register_buffer(&self) {
        self.lock().buffers += 1;
        self.wake.notify_all();
    }

    /// A `SourceBuffer` got its first initialization segment.
    pub fn buffer_initialized(&self) {
        self.lock().initialized += 1;
        self.wake.notify_all();
    }

    /// A `SourceBuffer` was removed; say whether it had been initialized.
    pub fn unregister_buffer(&self, initialized: bool) {
        let mut st = self.lock();
        st.buffers = st.buffers.saturating_sub(1);
        if initialized {
            st.initialized = st.initialized.saturating_sub(1);
        }
        self.wake.notify_all();
    }

    /// Every `SourceBuffer` has its tracks: the player can build its streams.
    pub fn ready(&self) -> bool {
        let st = self.lock();
        st.buffers > 0 && st.initialized >= st.buffers
    }

    /// Waits until [`Shared::ready`] and every track holds a frame, or the stream
    /// ended, up to `timeout`. Returns the earliest presentation time buffered then:
    /// where the player's timeline starts.
    pub fn wait_start(&self, timeout: Duration) -> Option<i64> {
        let deadline = Instant::now() + timeout;
        let mut st = self.lock();
        loop {
            if st.closed {
                return None;
            }
            let ready = st.buffers > 0 && st.initialized >= st.buffers;
            let live = || st.slots.iter().filter(|s| !s.retired);
            let all_have_frames = live().next().is_some() && live().all(|s| !s.buf.is_empty());
            if ready && (all_have_frames || (st.ended && live().any(|s| !s.buf.is_empty()))) {
                return live()
                    .filter_map(|s| s.buf.buffered().first().map(|r| r.0))
                    .min();
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            st = self
                .wake
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    pub fn slot_count(&self) -> usize {
        self.lock().slots.len()
    }

    pub fn track_info(&self, slot: usize) -> Option<TrackInfo> {
        self.lock()
            .slots
            .get(slot)
            .map(|s| s.infos[s.infos.len() - 1].clone())
    }

    /// The track as the initialization segment numbered `config` described it (the
    /// number a [`Sample`] carries).
    pub fn track_info_at(&self, slot: usize, config: u32) -> Option<TrackInfo> {
        let st = self.lock();
        let s = st.slots.get(slot)?;
        s.infos.get(config as usize).or(s.infos.last()).cloned()
    }

    fn make_room(&self, st: &mut State, incoming: usize) -> Result<(), QuotaExceeded> {
        let used: usize = st.slots.iter().map(|s| s.buf.bytes()).sum();
        if used + incoming <= self.quota {
            return Ok(());
        }
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
        Ok(())
    }

    /// Makes room for `bytes` more (evicting what was played), or says there is none:
    /// `appendBuffer` throws `QuotaExceededError` then, before it takes the data.
    pub fn reserve(&self, bytes: usize) -> Result<(), QuotaExceeded> {
        let mut st = self.lock();
        self.make_room(&mut st, bytes)
    }

    /// Adds frames to a track, evicting played data first if the quota needs it.
    pub fn append(&self, slot: usize, mut samples: Vec<Sample>) -> Result<(), QuotaExceeded> {
        let incoming: usize = samples.iter().map(|s| s.data.len()).sum();
        let mut st = self.lock();
        if slot >= st.slots.len() {
            return Ok(());
        }
        self.make_room(&mut st, incoming)?;
        let config = (st.slots[slot].infos.len() - 1) as u32;
        for s in &mut samples {
            s.config = config;
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

    /// A track whose `SourceBuffer` was removed: its frames go, and nothing reads or
    /// waits for it from now on.
    pub fn retire(&self, slot: usize) {
        let mut st = self.lock();
        if let Some(s) = st.slots.get_mut(slot) {
            s.buf = TrackBuffer::new();
            s.retired = true;
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

    /// What a `SourceBuffer` reports as `buffered`, and the media element with every
    /// track: the time all of `slots` have. After the stream ends, a track that stops
    /// short of the longest one counts as reaching it.
    pub fn buffered_of(&self, slots: &[usize]) -> Vec<(i64, i64)> {
        let st = self.lock();
        Self::intersection(&st, slots.iter().copied())
    }

    /// What the media element reports as `buffered`: the time every track has.
    pub fn buffered_all(&self) -> Vec<(i64, i64)> {
        let st = self.lock();
        let live: Vec<usize> = (0..st.slots.len())
            .filter(|&i| !st.slots[i].retired)
            .collect();
        Self::intersection(&st, live.into_iter())
    }

    fn intersection(st: &State, slots: impl Iterator<Item = usize>) -> Vec<(i64, i64)> {
        let mut all: Vec<Vec<(i64, i64)>> = slots
            .filter_map(|i| st.slots.get(i))
            .map(|s| s.buf.buffered())
            .collect();
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

    /// The page (or the user) seeked to `time` nanoseconds: feeders restart there. The
    /// player reports one seek once per track; a repeat of the same time right after is
    /// that, not a second seek.
    pub fn seek(&self, time: i64) {
        let mut st = self.lock();
        let now = Instant::now();
        if st.seek_to == time
            && st.epoch > 0
            && st
                .seek_at
                .is_some_and(|at| now.duration_since(at) < SEEK_COALESCE)
        {
            return;
        }
        st.epoch += 1;
        st.seek_to = time;
        st.seek_at = Some(now);
        st.position = time;
        self.wake.notify_all();
    }

    /// Counts seeks; a feeder compares it with the one it last saw.
    pub fn epoch(&self) -> u64 {
        self.lock().epoch
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
            eos_sent: false,
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
    /// `Next::Eos` was returned and nothing has changed since.
    eos_sent: bool,
}

impl TrackHandle {
    /// A seek happened since the last [`TrackHandle::next`]: a frame it returned, not
    /// yet handed to the player, belongs to the time before the seek.
    pub fn stale(&self) -> bool {
        self.shared.lock().epoch != self.epoch
    }

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
                self.eos_sent = false;
                return Next::Flush {
                    position: st.seek_to,
                };
            }
            let Some(slot) = st.slots.get(self.slot).filter(|s| !s.retired) else {
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
                self.eos_sent = false;
                return Next::Sample(sample.clone());
            }
            if st.ended && !self.eos_sent {
                self.eos_sent = true;
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
                config: 0,
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
        // Once only: the stream stays ended, the feeder has nothing more to do.
        assert_eq!(h.next(SHORT), Next::Wait);
        // More data after an end restarts the stream.
        shared.append(slot, run(160, 1, 1)).unwrap();
        assert!(matches!(h.next(SHORT), Next::Sample(s) if s.pts == 160 * MS));
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
    fn frames_carry_the_configuration_they_were_appended_under() {
        let shared = Shared::new();
        let mut a = info(1, TrackKind::Video);
        a.width = 320;
        let slot = shared.add_track(a.clone());
        shared.append(slot, run(0, 2, 2)).unwrap();
        let mut b = a.clone();
        b.width = 640;
        assert_eq!(shared.add_track(b.clone()), slot);
        // The same configuration again adds nothing.
        shared.add_track(b.clone());
        shared.append(slot, run(80, 2, 2)).unwrap();
        let mut h = shared.handle(slot);
        let mut widths = Vec::new();
        while let Next::Sample(s) = h.next(SHORT) {
            widths.push(shared.track_info_at(slot, s.config).unwrap().width);
        }
        assert_eq!(widths, vec![320, 320, 640, 640]);
        assert_eq!(shared.track_info(slot).unwrap().width, 640);
    }

    #[test]
    fn ready_waits_for_every_buffer_and_the_start_for_every_track() {
        let shared = Shared::new();
        assert!(!shared.ready());
        shared.register_buffer();
        shared.register_buffer();
        let v = shared.add_track(info(1, TrackKind::Video));
        shared.buffer_initialized();
        assert!(!shared.ready());
        let a = shared.add_track(info(2, TrackKind::Audio));
        shared.buffer_initialized();
        assert!(shared.ready());
        // No frames yet.
        assert_eq!(shared.wait_start(SHORT), None);
        shared.append(v, run(66, 5, 5)).unwrap();
        assert_eq!(shared.wait_start(SHORT), None);
        shared.append(a, run(0, 5, 1)).unwrap();
        // The timeline starts at the earliest frame of any track.
        assert_eq!(shared.wait_start(SHORT), Some(0));
        shared.unregister_buffer(true);
        shared.unregister_buffer(true);
        assert!(!shared.ready());
    }

    #[test]
    fn a_repeated_seek_report_is_one_seek() {
        let shared = Shared::new();
        shared.seek(1000 * MS);
        let epoch = shared.epoch();
        shared.seek(1000 * MS);
        assert_eq!(shared.epoch(), epoch);
        shared.seek(2000 * MS);
        assert_eq!(shared.epoch(), epoch + 1);
    }

    #[test]
    fn a_frame_fetched_before_a_seek_is_stale_after_it() {
        let shared = Shared::new();
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 5, 5)).unwrap();
        let mut h = shared.handle(slot);
        assert!(matches!(h.next(SHORT), Next::Sample(_)));
        assert!(!h.stale());
        shared.seek(80 * MS);
        assert!(h.stale());
        assert!(matches!(h.next(SHORT), Next::Flush { .. }));
        assert!(!h.stale());
    }

    #[test]
    fn a_retired_track_is_left_out_and_stops_its_feeder() {
        let shared = Shared::new();
        shared.register_buffer();
        shared.register_buffer();
        let v = shared.add_track(info(1, TrackKind::Video));
        let a = shared.add_track(info(2, TrackKind::Audio));
        shared.buffer_initialized();
        shared.buffer_initialized();
        shared.append(v, run(0, 25, 5)).unwrap();
        shared.append(a, run(0, 10, 1)).unwrap();
        assert_eq!(shared.buffered_all(), vec![(0, 400 * MS)]);
        assert_eq!(shared.buffered_of(&[v]), vec![(0, 1000 * MS)]);
        let mut h = shared.handle(a);
        shared.retire(a);
        assert_eq!(h.next(SHORT), Next::Closed);
        assert_eq!(shared.buffered_all(), vec![(0, 1000 * MS)]);
    }

    #[test]
    fn reserve_makes_room_or_refuses() {
        let shared = Shared::with_quota(2000);
        let slot = shared.add_track(info(1, TrackKind::Video));
        shared.append(slot, run(0, 15, 5)).unwrap(); // 1500 bytes
        assert!(shared.reserve(400).is_ok());
        assert_eq!(shared.reserve(1000), Err(QuotaExceeded));
        shared.set_position(3000 * MS);
        assert!(shared.reserve(1000).is_ok());
    }

    #[test]
    fn intersecting_ranges() {
        let a = vec![(0, 10), (20, 30)];
        let b = vec![(5, 25)];
        assert_eq!(intersect(&a, &b), vec![(5, 10), (20, 25)]);
        assert!(intersect(&a, &[]).is_empty());
    }
}
