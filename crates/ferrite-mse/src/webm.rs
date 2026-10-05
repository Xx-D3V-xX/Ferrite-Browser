//! WebM (Matroska): an initialization segment (EBML header, `Segment`, `Info`, `Tracks`)
//! and media segments (`Cluster`s of blocks), possibly split across any number of appends,
//! and with the unknown sizes live streams use for the segment and for clusters.

use std::collections::HashMap;

use crate::{Codec, Event, ParseError, Sample, TrackInfo, TrackKind};

const ID_EBML: u64 = 0x1A45_DFA3;
const ID_SEGMENT: u64 = 0x1853_8067;
const ID_INFO: u64 = 0x1549_A966;
const ID_TIMECODE_SCALE: u64 = 0x2A_D7B1;
const ID_DURATION: u64 = 0x4489;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_TYPE: u64 = 0x83;
const ID_CODEC_ID: u64 = 0x86;
const ID_CODEC_PRIVATE: u64 = 0x63A2;
const ID_DEFAULT_DURATION: u64 = 0x23_E383;
const ID_VIDEO: u64 = 0xE0;
const ID_PIXEL_WIDTH: u64 = 0xB0;
const ID_PIXEL_HEIGHT: u64 = 0xBA;
const ID_AUDIO: u64 = 0xE1;
const ID_SAMPLING_FREQUENCY: u64 = 0xB5;
const ID_CHANNELS: u64 = 0x9F;
const ID_CLUSTER: u64 = 0x1F43_B675;
const ID_CLUSTER_TIMECODE: u64 = 0xE7;
const ID_SIMPLE_BLOCK: u64 = 0xA3;
const ID_BLOCK_GROUP: u64 = 0xA0;
const ID_BLOCK: u64 = 0xA1;
const ID_BLOCK_DURATION: u64 = 0x9B;
const ID_REFERENCE_BLOCK: u64 = 0xFB;

/// Elements that sit next to `Cluster` in the segment: one of these ends a cluster of
/// unknown size.
const LEVEL1: [u64; 8] = [
    ID_CLUSTER,
    ID_INFO,
    ID_TRACKS,
    0x114D_9B74, // SeekHead
    0x1C53_BB6B, // Cues
    0x1043_A770, // Chapters
    0x1254_C367, // Tags
    0x1941_A469, // Attachments
];

#[derive(Default)]
struct Track {
    kind: Option<TrackKind>,
    default_duration: i64,
}

enum State {
    /// Before the `Segment`: the EBML header, which is skipped.
    Top,
    /// Inside the segment, between top-level elements.
    Segment,
    /// Inside a cluster; `end` is where it ends (None: unknown size).
    Cluster { end: Option<u64> },
}

pub(crate) struct WebmParser {
    buf: Vec<u8>,
    /// Stream offset of `buf[0]`.
    pos: u64,
    state: State,
    /// Nanoseconds per timecode tick (default 1 ms).
    scale: i64,
    duration: Option<i64>,
    tracks: HashMap<u32, Track>,
    cluster_timecode: i64,
    /// Bytes of an element this parser does not read that are still to arrive.
    skip: u64,
    /// Frames of the current cluster waiting for the one after them (a block that has no
    /// duration of its own is as long as the gap to the next block of its track).
    held: HashMap<u32, Sample>,
    out: HashMap<u32, Vec<Sample>>,
    order: Vec<u32>,
    last_duration: HashMap<u32, i64>,
    failed: bool,
}

impl Default for WebmParser {
    fn default() -> Self {
        WebmParser {
            buf: Vec::new(),
            pos: 0,
            state: State::Top,
            scale: 1_000_000,
            duration: None,
            tracks: HashMap::new(),
            cluster_timecode: 0,
            skip: 0,
            held: HashMap::new(),
            out: HashMap::new(),
            order: Vec::new(),
            last_duration: HashMap::new(),
            failed: false,
        }
    }
}

/// A variable-length integer: (value with its length marker removed, length in bytes,
/// all value bits set). `None` if more bytes are needed.
fn vint(b: &[u8]) -> Option<(u64, usize, bool)> {
    let first = *b.first()?;
    if first == 0 {
        return Some((0, 9, false)); // invalid; the caller treats a length 9 as an error
    }
    let len = first.leading_zeros() as usize + 1;
    if b.len() < len {
        return None;
    }
    let mut value = (first as u64) & ((1u64 << (8 - len)) - 1);
    let mut all_ones = value == (1u64 << (8 - len)) - 1;
    for byte in &b[1..len] {
        value = (value << 8) | *byte as u64;
        if *byte != 0xff {
            all_ones = false;
        }
    }
    Some((value, len, all_ones))
}

/// An element id keeps its length marker.
fn element_id(b: &[u8]) -> Option<(u64, usize)> {
    let first = *b.first()?;
    if first == 0 {
        return Some((0, 9));
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 4 {
        return Some((0, 9));
    }
    if b.len() < len {
        return None;
    }
    let mut id = 0u64;
    for byte in &b[..len] {
        id = (id << 8) | *byte as u64;
    }
    Some((id, len))
}

struct Header {
    id: u64,
    /// Bytes of id and size.
    header: usize,
    /// None: unknown size.
    size: Option<u64>,
}

fn header(b: &[u8]) -> Result<Option<Header>, ParseError> {
    let Some((id, id_len)) = element_id(b) else {
        return Ok(None);
    };
    if id_len > 4 {
        return Err(ParseError::Malformed(
            "an element id longer than four bytes",
        ));
    }
    let Some((size, size_len, all_ones)) = vint(&b[id_len..]) else {
        return Ok(None);
    };
    if size_len > 8 {
        return Err(ParseError::Malformed(
            "an element size longer than eight bytes",
        ));
    }
    Ok(Some(Header {
        id,
        header: id_len + size_len,
        size: if all_ones { None } else { Some(size) },
    }))
}

fn read_uint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |v, byte| (v << 8) | *byte as u64)
}

fn read_float(b: &[u8]) -> f64 {
    match b.len() {
        4 => f32::from_be_bytes(b.try_into().unwrap()) as f64,
        8 => f64::from_be_bytes(b.try_into().unwrap()),
        _ => 0.0,
    }
}

/// The child elements of a master element's body, each as (id, contents).
fn children(body: &[u8]) -> Result<Vec<(u64, &[u8])>, ParseError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < body.len() {
        let Some(h) = header(&body[at..])? else {
            return Err(ParseError::Malformed(
                "an element is cut off inside its parent",
            ));
        };
        let size = h.size.ok_or(ParseError::Unsupported(
            "an unknown-size element in a header",
        ))? as usize;
        if at + h.header + size > body.len() {
            return Err(ParseError::Malformed(
                "an element is larger than its parent",
            ));
        }
        out.push((h.id, &body[at + h.header..at + h.header + size]));
        at += h.header + size;
    }
    Ok(out)
}

impl WebmParser {
    pub(crate) fn pending_bytes(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn reset(&mut self) {
        self.buf.clear();
        self.pos = 0;
        self.skip = 0;
        self.failed = false;
        self.held.clear();
        self.out.clear();
        // Expect either a new initialization segment or a bare media segment: the
        // latter starts at a `Cluster`, which both states accept.
        self.state = State::Segment;
    }

    pub(crate) fn append(&mut self, data: &[u8]) -> Result<Vec<Event>, ParseError> {
        if self.failed {
            return Err(ParseError::Malformed("an earlier error: reset the parser"));
        }
        self.buf.extend_from_slice(data);
        let mut events = Vec::new();
        match self.drain(&mut events) {
            Ok(()) => {
                self.flush_out(&mut events, false);
                Ok(events)
            }
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }

    fn consume(&mut self, n: usize) {
        self.buf.drain(..n);
        self.pos += n as u64;
    }

    fn drain(&mut self, events: &mut Vec<Event>) -> Result<(), ParseError> {
        loop {
            if self.skip > 0 {
                let n = (self.skip as usize).min(self.buf.len());
                self.consume(n);
                self.skip -= n as u64;
                if self.skip > 0 {
                    return Ok(());
                }
            }
            if let State::Cluster { end: Some(end) } = self.state
                && self.pos >= end
            {
                self.close_cluster(events);
                self.state = State::Segment;
                continue;
            }
            let Some(h) = header(&self.buf)? else {
                return Ok(());
            };
            match self.state {
                State::Top => match h.id {
                    ID_EBML => {
                        let size = h
                            .size
                            .ok_or(ParseError::Malformed("an EBML header of unknown size"))?
                            as usize;
                        if self.buf.len() < h.header + size {
                            return Ok(());
                        }
                        self.consume(h.header + size);
                    }
                    ID_SEGMENT => {
                        self.consume(h.header);
                        self.state = State::Segment;
                    }
                    _ => return Err(ParseError::Malformed("not a WebM: no EBML header")),
                },
                State::Segment => match h.id {
                    ID_SEGMENT => {
                        self.consume(h.header);
                    }
                    ID_EBML => {
                        // A second initialization segment: start over from its header.
                        self.state = State::Top;
                    }
                    ID_CLUSTER => {
                        let end = h.size.map(|s| self.pos + h.header as u64 + s);
                        self.consume(h.header);
                        self.cluster_timecode = 0;
                        self.state = State::Cluster { end };
                    }
                    ID_INFO | ID_TRACKS => {
                        let size = h.size.ok_or(ParseError::Unsupported(
                            "an unknown-size `Info` or `Tracks`",
                        ))? as usize;
                        if self.buf.len() < h.header + size {
                            return Ok(());
                        }
                        let body = self.buf[h.header..h.header + size].to_vec();
                        self.consume(h.header + size);
                        if h.id == ID_INFO {
                            self.info(&body)?;
                        } else {
                            self.tracks_element(&body, events)?;
                        }
                    }
                    _ => {
                        // SeekHead, Cues, Tags, Void, Chapters, Attachments: not needed.
                        let size = h.size.ok_or(ParseError::Unsupported(
                            "an unknown-size element I do not read",
                        ))? as usize;
                        if self.buf.len() < h.header + size {
                            // Skip what has arrived; the rest is dropped as it comes.
                            let have = self.buf.len();
                            self.skip = (h.header + size - have) as u64;
                            self.consume(have);
                            return Ok(());
                        }
                        self.consume(h.header + size);
                    }
                },
                State::Cluster { end } => {
                    if end.is_some_and(|e| self.pos >= e)
                        || (end.is_none() && LEVEL1.contains(&h.id))
                    {
                        self.close_cluster(events);
                        self.state = State::Segment;
                        continue;
                    }
                    let size = h.size.ok_or(ParseError::Unsupported(
                        "an unknown-size element inside a cluster",
                    ))? as usize;
                    if self.buf.len() < h.header + size {
                        return Ok(());
                    }
                    let body = self.buf[h.header..h.header + size].to_vec();
                    self.consume(h.header + size);
                    match h.id {
                        ID_CLUSTER_TIMECODE => self.cluster_timecode = read_uint(&body) as i64,
                        ID_SIMPLE_BLOCK => self.block(&body, None, None)?,
                        ID_BLOCK_GROUP => {
                            let mut block: Option<&[u8]> = None;
                            let mut duration = None;
                            let mut has_reference = false;
                            for (id, b) in children(&body)? {
                                match id {
                                    ID_BLOCK => block = Some(b),
                                    ID_BLOCK_DURATION => duration = Some(read_uint(b) as i64),
                                    ID_REFERENCE_BLOCK => has_reference = true,
                                    _ => {}
                                }
                            }
                            if let Some(b) = block {
                                self.block(b, duration, Some(!has_reference))?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn info(&mut self, body: &[u8]) -> Result<(), ParseError> {
        let mut scale = 1_000_000i64;
        let mut duration_ticks = None;
        for (id, b) in children(body)? {
            match id {
                ID_TIMECODE_SCALE => scale = read_uint(b) as i64,
                ID_DURATION => duration_ticks = Some(read_float(b)),
                _ => {}
            }
        }
        self.scale = scale.max(1);
        self.duration = duration_ticks.map(|d| (d * self.scale as f64) as i64);
        Ok(())
    }

    fn tracks_element(&mut self, body: &[u8], events: &mut Vec<Event>) -> Result<(), ParseError> {
        let mut tracks = HashMap::new();
        let mut infos = Vec::new();
        for (id, entry) in children(body)? {
            if id != ID_TRACK_ENTRY {
                continue;
            }
            let (mut number, mut kind, mut codec_id, mut private) =
                (0u32, 0u64, String::new(), Vec::new());
            let (mut width, mut height, mut rate, mut channels, mut default_duration) =
                (0u32, 0u32, 0f64, 0u32, 0i64);
            for (id, b) in children(entry)? {
                match id {
                    ID_TRACK_NUMBER => number = read_uint(b) as u32,
                    ID_TRACK_TYPE => kind = read_uint(b),
                    ID_CODEC_ID => {
                        codec_id = String::from_utf8_lossy(b)
                            .trim_end_matches('\0')
                            .to_string()
                    }
                    ID_CODEC_PRIVATE => private = b.to_vec(),
                    ID_DEFAULT_DURATION => default_duration = read_uint(b) as i64,
                    ID_VIDEO => {
                        for (id, b) in children(b)? {
                            match id {
                                ID_PIXEL_WIDTH => width = read_uint(b) as u32,
                                ID_PIXEL_HEIGHT => height = read_uint(b) as u32,
                                _ => {}
                            }
                        }
                    }
                    ID_AUDIO => {
                        for (id, b) in children(b)? {
                            match id {
                                ID_SAMPLING_FREQUENCY => rate = read_float(b),
                                ID_CHANNELS => channels = read_uint(b) as u32,
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            let kind = match kind {
                1 => TrackKind::Video,
                2 => TrackKind::Audio,
                _ => continue, // subtitles, buttons, ...
            };
            let codec = match codec_id.as_str() {
                "V_VP8" => Codec::Vp8,
                "V_VP9" => Codec::Vp9,
                "V_AV1" => Codec::Av1 { av1c: private },
                "V_MPEG4/ISO/AVC" => Codec::H264 { avcc: private },
                "V_MPEGH/ISO/HEVC" => Codec::H265 { hvcc: private },
                "A_OPUS" => Codec::Opus { head: private },
                "A_VORBIS" => Codec::Vorbis {
                    headers: vorbis_headers(&private)?,
                },
                "A_AAC" => Codec::Aac { asc: private },
                "A_MPEG/L3" => Codec::Mp3,
                other => Codec::Unknown(other.to_string()),
            };
            let info = TrackInfo {
                id: number,
                kind,
                codec,
                width,
                height,
                sample_rate: rate as u32,
                channels,
            };
            infos.push(info.clone());
            tracks.insert(
                number,
                Track {
                    kind: Some(kind),
                    default_duration,
                },
            );
        }
        if infos.is_empty() {
            return Err(ParseError::Malformed(
                "an initialization segment with no audio or video track",
            ));
        }
        self.tracks = tracks;
        events.push(Event::Init {
            tracks: infos,
            duration: self.duration,
        });
        Ok(())
    }

    /// A `SimpleBlock` or the `Block` of a `BlockGroup`. `key` is known for the latter.
    fn block(
        &mut self,
        body: &[u8],
        duration: Option<i64>,
        key: Option<bool>,
    ) -> Result<(), ParseError> {
        let Some((number, n, _)) = vint(body) else {
            return Err(ParseError::Malformed("a block with no track number"));
        };
        if body.len() < n + 3 {
            return Err(ParseError::Malformed("a block shorter than its header"));
        }
        let relative = i16::from_be_bytes([body[n], body[n + 1]]) as i64;
        let flags = body[n + 2];
        if flags & 0x06 != 0 {
            return Err(ParseError::Unsupported("a laced block"));
        }
        let track = number as u32;
        let Some(info) = self.tracks.get(&track) else {
            return Ok(()); // a track that is not played
        };
        let default_duration = info.default_duration;
        // Every audio packet is a sync sample, whatever the muxer flagged.
        let key = info.kind == Some(TrackKind::Audio) || key.unwrap_or(flags & 0x80 != 0);
        let pts = (self.cluster_timecode + relative) * self.scale;
        let mut sample = Sample {
            pts,
            dts: pts,
            duration: duration.map(|d| d * self.scale).unwrap_or(default_duration),
            key,
            data: body[n + 3..].to_vec(),
            config: 0,
        };
        // A frame with no duration of its own: the gap to the next one of its track
        // (found when that one arrives).
        if let Some(mut earlier) = self.held.remove(&track) {
            if earlier.duration == 0 && pts > earlier.pts {
                earlier.duration = pts - earlier.pts;
            }
            // A frame that takes no time and shares its time with the next one cannot be
            // told apart from it in a buffer; it is dropped.
            if earlier.duration > 0 {
                self.last_duration.insert(track, earlier.duration);
                self.push_out(track, earlier);
            }
        }
        if sample.duration == 0 {
            self.held.insert(track, sample);
        } else {
            self.last_duration.insert(track, sample.duration);
            sample.dts = sample.pts;
            self.push_out(track, sample);
        }
        Ok(())
    }

    fn push_out(&mut self, track: u32, sample: Sample) {
        if !self.order.contains(&track) {
            self.order.push(track);
        }
        self.out.entry(track).or_default().push(sample);
    }

    /// The end of a cluster: a frame still waiting for a duration takes the one the
    /// track last had.
    fn close_cluster(&mut self, events: &mut Vec<Event>) {
        let held: Vec<u32> = self.held.keys().copied().collect();
        for track in held {
            if let Some(mut s) = self.held.remove(&track) {
                s.duration = self
                    .last_duration
                    .get(&track)
                    .copied()
                    .unwrap_or(20_000_000);
                self.push_out(track, s);
            }
        }
        self.flush_out(events, true);
    }

    fn flush_out(&mut self, events: &mut Vec<Event>, _cluster_closed: bool) {
        let order = std::mem::take(&mut self.order);
        for track in order {
            if let Some(samples) = self.out.remove(&track)
                && !samples.is_empty()
            {
                events.push(Event::Samples { track, samples });
            }
        }
    }
}

/// The three Vorbis headers out of a `CodecPrivate` (Xiph lacing).
fn vorbis_headers(private: &[u8]) -> Result<Vec<Vec<u8>>, ParseError> {
    if private.is_empty() {
        return Ok(Vec::new());
    }
    if private[0] != 2 {
        return Err(ParseError::Malformed(
            "a Vorbis `CodecPrivate` that does not have three headers",
        ));
    }
    let mut at = 1usize;
    let mut sizes = [0usize; 2];
    for size in &mut sizes {
        loop {
            let Some(byte) = private.get(at) else {
                return Err(ParseError::Malformed(
                    "a Vorbis `CodecPrivate` cut off in its sizes",
                ));
            };
            at += 1;
            *size += *byte as usize;
            if *byte != 255 {
                break;
            }
        }
    }
    let total = at + sizes[0] + sizes[1];
    if private.len() < total {
        return Err(ParseError::Malformed(
            "a Vorbis `CodecPrivate` shorter than its sizes",
        ));
    }
    Ok(vec![
        private[at..at + sizes[0]].to_vec(),
        private[at + sizes[0]..total].to_vec(),
        private[total..].to_vec(),
    ])
}
