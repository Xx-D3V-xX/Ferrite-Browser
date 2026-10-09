//! A track's coded frame store.

use crate::Sample;

/// How far a frame's decode time can lie before its presentation time (the reorder
/// delay of B-frames), as far as overlap detection cares. Anything a real stream does
/// is well inside this.
const REORDER_WINDOW: i64 = 2_000_000_000;

/// A seek to a time slightly before the first buffered frame still finds it: streams
/// rarely start at exactly zero.
const START_SLACK: i64 = 250_000_000;

/// The frames of one track, sorted by decode time.
#[derive(Debug, Default)]
pub struct TrackBuffer {
    frames: Vec<Sample>,
    bytes: usize,
}

impl TrackBuffer {
    pub fn new() -> TrackBuffer {
        TrackBuffer::default()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The encoded bytes held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub fn get(&self, index: usize) -> Option<&Sample> {
        self.frames.get(index)
    }

    /// Adds frames from a media segment. A frame replaces whatever is already buffered
    /// for its presentation interval; frames that were left without the frame they
    /// depended on are dropped up to the next sync sample. Returns how many frames were
    /// replaced or dropped.
    pub fn append(&mut self, samples: Vec<Sample>) -> usize {
        let mut removed = 0;
        let mut last_dts = None;
        for sample in samples {
            let start = sample.pts;
            let end = sample.end().max(sample.pts + 1);
            let lo = self
                .frames
                .partition_point(|f| f.dts < sample.dts - REORDER_WINDOW);
            let hi = self
                .frames
                .partition_point(|f| f.dts <= sample.dts + REORDER_WINDOW);
            let mut i = lo;
            let mut stop = hi;
            while i < stop {
                if self.frames[i].pts >= start && self.frames[i].pts < end {
                    self.bytes -= self.frames[i].data.len();
                    self.frames.remove(i);
                    removed += 1;
                    stop -= 1;
                } else {
                    i += 1;
                }
            }
            let at = self.frames.partition_point(|f| f.dts <= sample.dts);
            self.bytes += sample.data.len();
            last_dts = Some(sample.dts);
            self.frames.insert(at, sample);
        }
        if removed > 0
            && let Some(dts) = last_dts
        {
            let at = self.frames.partition_point(|f| f.dts <= dts);
            removed += self.drop_orphans(at);
        }
        removed
    }

    /// Removes the frames presented in `[start, end)`, and the frames after them that
    /// could not be decoded without them.
    pub fn remove(&mut self, start: i64, end: i64) {
        let mut kept = Vec::with_capacity(self.frames.len());
        let mut at = None;
        for frame in std::mem::take(&mut self.frames) {
            if frame.pts >= start && frame.pts < end {
                self.bytes -= frame.data.len();
                at.get_or_insert(kept.len());
            } else {
                kept.push(frame);
            }
        }
        self.frames = kept;
        if let Some(at) = at {
            self.drop_orphans(at);
        }
    }

    fn drop_orphans(&mut self, at: usize) -> usize {
        let mut n = 0;
        while at < self.frames.len() && !self.frames[at].key {
            self.bytes -= self.frames[at].data.len();
            self.frames.remove(at);
            n += 1;
        }
        n
    }

    /// The buffered time ranges, in nanoseconds, sorted and merged. Frames closer
    /// together than the longest frame count as contiguous.
    pub fn buffered(&self) -> Vec<(i64, i64)> {
        if self.frames.is_empty() {
            return Vec::new();
        }
        let tolerance = self.frames.iter().map(|f| f.duration).max().unwrap_or(0);
        let mut spans: Vec<(i64, i64)> = self.frames.iter().map(|f| (f.pts, f.end())).collect();
        spans.sort_unstable();
        let mut out: Vec<(i64, i64)> = Vec::new();
        for (start, end) in spans {
            match out.last_mut() {
                Some(last) if start <= last.1 + tolerance => last.1 = last.1.max(end),
                _ => out.push((start, end)),
            }
        }
        out
    }

    /// The latest presentation time of any frame (its start, not its end): what a new
    /// duration may not go below.
    pub fn highest_pts(&self) -> Option<i64> {
        self.frames.iter().map(|f| f.pts).max()
    }

    /// The frame after the one decoded at `after` (or the first, with `None`).
    pub fn next_after(&self, after: Option<i64>) -> Option<&Sample> {
        match after {
            None => self.frames.first(),
            Some(dts) => {
                let at = self.frames.partition_point(|f| f.dts <= dts);
                self.frames.get(at)
            }
        }
    }

    /// Where feeding must start to play from `time`: the decode time just before the
    /// last sync sample presented at or before it. `None` if nothing buffered covers
    /// `time`.
    pub fn seek_cursor(&self, time: i64) -> Option<i64> {
        let covered = self
            .buffered()
            .iter()
            .any(|&(start, end)| time < end && time >= start - START_SLACK);
        if !covered {
            return None;
        }
        let end = self
            .frames
            .partition_point(|f| f.dts <= time + REORDER_WINDOW);
        let key = self.frames[..end]
            .iter()
            .rev()
            .find(|f| f.key && f.pts <= time)
            // `time` is just before the first frame: start from the first sync sample.
            .or_else(|| self.frames.iter().find(|f| f.key))?;
        Some(key.dts - 1)
    }

    /// Drops whole groups of frames that end before `before`, oldest first, until
    /// `bytes` or more were freed (or nothing more can go). Never splits a group of
    /// pictures: removal stops at a sync sample. Returns the bytes freed.
    pub fn evict_before(&mut self, before: i64, bytes: usize) -> usize {
        let mut freed = 0;
        // The cut is the last sync sample whose predecessors all end before `before`.
        let mut cut = 0;
        for (i, f) in self.frames.iter().enumerate() {
            if f.end() > before {
                break;
            }
            if f.key && i > 0 {
                cut = i;
            }
        }
        // Keep from the first sync sample at or after the point enough was freed.
        let mut upto = 0;
        for i in 0..cut {
            freed += self.frames[i].data.len();
            if self.frames.get(i + 1).is_some_and(|f| f.key) {
                upto = i + 1;
                if freed >= bytes {
                    break;
                }
            }
        }
        if upto == 0 {
            return 0;
        }
        let gone: usize = self.frames[..upto].iter().map(|f| f.data.len()).sum();
        self.frames.drain(..upto);
        self.bytes -= gone;
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000_000;

    fn frame(pts_ms: i64, dur_ms: i64, key: bool, size: usize) -> Sample {
        Sample {
            pts: pts_ms * MS,
            dts: pts_ms * MS,
            duration: dur_ms * MS,
            key,
            data: vec![0; size],
            config: 0,
        }
    }

    /// Frames every 40 ms with a sync sample every `gop`.
    fn run(from: i64, count: i64, gop: i64) -> Vec<Sample> {
        (0..count)
            .map(|i| frame(from + i * 40, 40, i % gop == 0, 10))
            .collect()
    }

    #[test]
    fn buffered_merges_contiguous_frames() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 10, 5));
        assert_eq!(b.buffered(), vec![(0, 400 * MS)]);
        b.append(run(1000, 5, 5));
        assert_eq!(b.buffered(), vec![(0, 400 * MS), (1000 * MS, 1200 * MS)]);
    }

    #[test]
    fn highest_pts_is_the_last_frame_start_not_its_end() {
        let mut b = TrackBuffer::new();
        assert_eq!(b.highest_pts(), None);
        b.append(run(0, 10, 5));
        assert_eq!(b.highest_pts(), Some(360 * MS));
        assert_eq!(b.buffered().last().map(|r| r.1), Some(400 * MS));
    }

    #[test]
    fn append_replaces_an_overlap() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 10, 5)); // keys at 0 and 200
        // A new group covering 80..200 replaces three frames; the key at 200 stays.
        let removed = b.append(run(80, 3, 3));
        assert_eq!(removed, 3);
        assert_eq!(b.len(), 10);
        assert_eq!(b.bytes(), 100);
    }

    #[test]
    fn replacing_a_frame_drops_what_depended_on_it() {
        // The spec's rule: frames after a replaced frame, up to the next sync sample,
        // are removed with it.
        let mut b = TrackBuffer::new();
        b.append(run(0, 10, 5)); // keys at 0 and 200
        let removed = b.append(vec![frame(0, 40, true, 10)]);
        assert_eq!(removed, 5);
        assert_eq!(b.len(), 6);
        assert_eq!(b.buffered(), vec![(0, 40 * MS), (200 * MS, 400 * MS)]);
    }

    #[test]
    fn replacing_the_middle_of_a_group_drops_the_rest_of_it() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 10, 5));
        b.append(vec![frame(80, 40, false, 10)]); // drops 120 and 160
        assert_eq!(b.len(), 8);
        assert_eq!(b.buffered(), vec![(0, 120 * MS), (200 * MS, 400 * MS)]);
    }

    #[test]
    fn remove_drops_orphaned_frames_to_the_next_sync_sample() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 10, 5)); // keys at 0 and 200
        b.remove(0, 40 * MS); // removes the key at 0 and its four dependents
        assert_eq!(b.len(), 5);
        assert_eq!(b.buffered(), vec![(200 * MS, 400 * MS)]);
        assert_eq!(b.bytes(), 50);
    }

    #[test]
    fn remove_a_range_in_the_middle() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 15, 5)); // keys at 0, 200, 400
        b.remove(200 * MS, 400 * MS);
        assert_eq!(b.buffered(), vec![(0, 200 * MS), (400 * MS, 600 * MS)]);
    }

    #[test]
    fn seek_finds_the_sync_sample_before() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 15, 5));
        // 250 is in the group that starts at 200.
        let cursor = b.seek_cursor(250 * MS).unwrap();
        assert_eq!(b.next_after(Some(cursor)).unwrap().pts, 200 * MS);
        // The start.
        let cursor = b.seek_cursor(0).unwrap();
        assert_eq!(b.next_after(Some(cursor)).unwrap().pts, 0);
        // Nothing buffered at 5 s.
        assert_eq!(b.seek_cursor(5000 * MS), None);
    }

    #[test]
    fn seek_to_just_before_the_first_frame_still_works() {
        let mut b = TrackBuffer::new();
        b.append(run(66, 5, 5));
        assert!(b.seek_cursor(0).is_some());
        assert!(b.seek_cursor(-1000 * MS).is_none());
    }

    #[test]
    fn next_after_walks_in_decode_order() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 3, 3));
        let mut cursor = None;
        let mut seen = Vec::new();
        while let Some(s) = b.next_after(cursor) {
            seen.push(s.pts / MS);
            cursor = Some(s.dts);
        }
        assert_eq!(seen, vec![0, 40, 80]);
    }

    #[test]
    fn b_frames_are_ordered_by_decode_time() {
        // pts order differs from dts order: I P B (decode) is I B P (presentation).
        let mk = |pts: i64, dts: i64, key: bool| Sample {
            pts: pts * MS,
            dts: dts * MS,
            duration: 40 * MS,
            key,
            data: vec![0; 4],
            config: 0,
        };
        let mut b = TrackBuffer::new();
        b.append(vec![mk(40, 0, true), mk(120, 40, false), mk(80, 80, false)]);
        assert_eq!(b.get(0).unwrap().dts, 0);
        assert_eq!(b.get(1).unwrap().dts, 40 * MS);
        assert_eq!(b.get(2).unwrap().dts, 80 * MS);
        assert_eq!(b.buffered(), vec![(40 * MS, 160 * MS)]);
    }

    #[test]
    fn eviction_frees_whole_groups_behind_the_playhead() {
        let mut b = TrackBuffer::new();
        b.append(run(0, 20, 5)); // keys at 0, 200, 400, 600
        let freed = b.evict_before(500 * MS, 1);
        assert_eq!(freed, 50);
        assert_eq!(b.buffered(), vec![(200 * MS, 800 * MS)]);
        // Nothing before the playhead any more: a group is never split.
        assert_eq!(b.evict_before(0, 1000), 0);
    }
}
