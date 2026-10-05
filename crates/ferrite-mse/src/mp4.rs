//! Fragmented MP4 (ISO BMFF): an initialization segment (`ftyp`, `moov` with `mvex`) and
//! media segments (`moof` + `mdat`), possibly split across any number of appends.

use std::collections::HashMap;

use crate::{Codec, Event, ParseError, Sample, TrackInfo, TrackKind, to_ns};

const FLAG_NON_SYNC: u32 = 0x0001_0000;

#[derive(Default, Clone, Copy)]
struct Trex {
    duration: u32,
    size: u32,
    flags: u32,
}

struct Track {
    info: TrackInfo,
    timescale: u32,
    trex: Trex,
}

#[derive(Default)]
pub(crate) struct Mp4Parser {
    buf: Vec<u8>,
    /// Stream offset of `buf[0]`.
    pos: u64,
    tracks: HashMap<u32, Track>,
    /// Where each track's next frame is decoded, in its own timescale, for a fragment
    /// that has no `tfdt`.
    next_dts: HashMap<u32, i64>,
    failed: bool,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, at: 0 }
    }
    fn left(&self) -> usize {
        self.b.len() - self.at
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        if self.left() < n {
            return Err(ParseError::Malformed("a box is shorter than its fields"));
        }
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, ParseError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, ParseError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ParseError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        self.take(n).map(|_| ())
    }
    fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.at..];
        self.at = self.b.len();
        s
    }
}

/// The boxes in `body`, each as (type, its contents).
/// A box's type and contents.
type BoxEntry<'a> = ([u8; 4], &'a [u8]);

fn boxes(body: &[u8]) -> Result<Vec<BoxEntry<'_>>, ParseError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= body.len() {
        let size32 = u32::from_be_bytes(body[at..at + 4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = body[at + 4..at + 8].try_into().unwrap();
        let (header, size) = match size32 {
            1 => {
                if at + 16 > body.len() {
                    return Err(ParseError::Malformed("a box's large size is cut off"));
                }
                (
                    16usize,
                    u64::from_be_bytes(body[at + 8..at + 16].try_into().unwrap()),
                )
            }
            0 => (8usize, (body.len() - at) as u64),
            n => (8usize, n),
        };
        if size < header as u64 || at as u64 + size > body.len() as u64 {
            return Err(ParseError::Malformed("a box is larger than its parent"));
        }
        out.push((kind, &body[at + header..at + size as usize]));
        at += size as usize;
    }
    Ok(out)
}

fn find<'a>(list: &[([u8; 4], &'a [u8])], kind: &[u8; 4]) -> Option<&'a [u8]> {
    list.iter().find(|(k, _)| k == kind).map(|(_, b)| *b)
}

impl Mp4Parser {
    pub(crate) fn pending_bytes(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn reset(&mut self) {
        self.buf.clear();
        self.pos = 0;
        self.failed = false;
    }

    pub(crate) fn append(&mut self, data: &[u8]) -> Result<Vec<Event>, ParseError> {
        if self.failed {
            return Err(ParseError::Malformed("an earlier error: reset the parser"));
        }
        self.buf.extend_from_slice(data);
        let mut events = Vec::new();
        match self.drain(&mut events) {
            Ok(()) => Ok(events),
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }

    fn drain(&mut self, events: &mut Vec<Event>) -> Result<(), ParseError> {
        loop {
            if self.buf.len() < 8 {
                return Ok(());
            }
            let size32 = u32::from_be_bytes(self.buf[0..4].try_into().unwrap()) as u64;
            let kind: [u8; 4] = self.buf[4..8].try_into().unwrap();
            let (header, size) = match size32 {
                1 => {
                    if self.buf.len() < 16 {
                        return Ok(());
                    }
                    (
                        16usize,
                        u64::from_be_bytes(self.buf[8..16].try_into().unwrap()),
                    )
                }
                0 => {
                    return Err(ParseError::Unsupported(
                        "a box that runs to the end of the stream",
                    ));
                }
                n => (8usize, n),
            };
            if size < header as u64 {
                return Err(ParseError::Malformed("a box smaller than its own header"));
            }
            let total = size as usize;
            match &kind {
                b"moof" => {
                    if self.buf.len() < total {
                        return Ok(());
                    }
                    // The data it describes is in the `mdat` that follows: both or neither.
                    let rest = &self.buf[total..];
                    if rest.len() < 8 {
                        return Ok(());
                    }
                    let msize = u32::from_be_bytes(rest[0..4].try_into().unwrap()) as u64;
                    let mkind: [u8; 4] = rest[4..8].try_into().unwrap();
                    if &mkind != b"mdat" {
                        return Err(ParseError::Unsupported(
                            "a `moof` that is not followed by its `mdat`",
                        ));
                    }
                    let (mheader, msize) = match msize {
                        1 => {
                            if rest.len() < 16 {
                                return Ok(());
                            }
                            (16usize, u64::from_be_bytes(rest[8..16].try_into().unwrap()))
                        }
                        0 => {
                            return Err(ParseError::Unsupported(
                                "an `mdat` that runs to the end of the stream",
                            ));
                        }
                        n => (8usize, n),
                    };
                    if msize < mheader as u64 {
                        return Err(ParseError::Malformed(
                            "an `mdat` smaller than its own header",
                        ));
                    }
                    let both = total + msize as usize;
                    if self.buf.len() < both {
                        return Ok(());
                    }
                    let moof = self.buf[header..total].to_vec();
                    let window = self.buf[..both].to_vec();
                    self.fragment(&moof, &window, total + mheader, events)?;
                    self.consume(both);
                }
                _ => {
                    if self.buf.len() < total {
                        // A big box we will skip anyway: do not hold it all.
                        if matches!(&kind, b"moov") {
                            return Ok(());
                        }
                        if matches!(&kind, b"mdat") {
                            return Err(ParseError::Unsupported(
                                "an `mdat` with no `moof` before it (not a fragmented MP4)",
                            ));
                        }
                        return Ok(());
                    }
                    if &kind == b"moov" {
                        let body = self.buf[header..total].to_vec();
                        self.moov(&body, events)?;
                    } else if &kind == b"mdat" {
                        return Err(ParseError::Unsupported(
                            "an `mdat` with no `moof` before it (not a fragmented MP4)",
                        ));
                    }
                    // ftyp, styp, sidx, emsg, free, skip, pssh, prft, mfra, ... are not needed.
                    self.consume(total);
                }
            }
        }
    }

    fn consume(&mut self, n: usize) {
        self.buf.drain(..n);
        self.pos += n as u64;
    }

    // ---------------------------------------------------------------- moov

    fn moov(&mut self, body: &[u8], events: &mut Vec<Event>) -> Result<(), ParseError> {
        let top = boxes(body)?;
        let mut duration = None;
        if let Some(mvhd) = find(&top, b"mvhd") {
            let mut r = Reader::new(mvhd);
            let version = r.u8()?;
            r.skip(3)?;
            let (timescale, dur) = if version == 1 {
                r.skip(16)?;
                (r.u32()? as u64, r.u64()?)
            } else {
                r.skip(8)?;
                (r.u32()? as u64, r.u32()? as u64)
            };
            if timescale > 0 && dur != 0 && dur != u64::MAX && dur != u32::MAX as u64 {
                duration = Some(to_ns(dur as i64, timescale));
            }
        }
        let mut trex: HashMap<u32, Trex> = HashMap::new();
        let fragmented = if let Some(mvex) = find(&top, b"mvex") {
            for (kind, b) in boxes(mvex)? {
                if &kind == b"trex" {
                    let mut r = Reader::new(b);
                    r.skip(4)?;
                    let id = r.u32()?;
                    r.skip(4)?;
                    trex.insert(
                        id,
                        Trex {
                            duration: r.u32()?,
                            size: r.u32()?,
                            flags: r.u32()?,
                        },
                    );
                }
            }
            true
        } else {
            false
        };
        let mut tracks = HashMap::new();
        let mut infos = Vec::new();
        for (kind, b) in &top {
            if kind == b"trak"
                && let Some((track, has_samples)) = parse_trak(b, &trex)?
            {
                if has_samples && !fragmented {
                    return Err(ParseError::Unsupported(
                        "a progressive MP4 (no `mvex`): MSE needs a fragmented one",
                    ));
                }
                infos.push(track.info.clone());
                tracks.insert(track.info.id, track);
            }
        }
        if infos.is_empty() {
            return Err(ParseError::Malformed(
                "an initialization segment with no track",
            ));
        }
        self.tracks = tracks;
        self.next_dts.clear();
        events.push(Event::Init {
            tracks: infos,
            duration,
        });
        Ok(())
    }

    // ---------------------------------------------------------------- moof + mdat

    /// `moof_body`: the contents of the `moof` box; `window`: the `moof` and `mdat` as
    /// they sit in the stream (the `moof` starts at 0); `mdat_data`: where the `mdat`'s
    /// payload starts in `window`.
    fn fragment(
        &mut self,
        moof_body: &[u8],
        window: &[u8],
        _mdat_data: usize,
        events: &mut Vec<Event>,
    ) -> Result<(), ParseError> {
        let top = boxes(moof_body)?;
        let mut previous_end: Option<i64> = None;
        for (kind, traf) in top {
            if &kind != b"traf" {
                continue;
            }
            let parts = boxes(traf)?;
            let tfhd =
                find(&parts, b"tfhd").ok_or(ParseError::Malformed("a `traf` with no `tfhd`"))?;
            let mut r = Reader::new(tfhd);
            let flags = r.u32()? & 0x00ff_ffff;
            let track_id = r.u32()?;
            let base_data_offset = if flags & 0x01 != 0 {
                Some(r.u64()?)
            } else {
                None
            };
            if flags & 0x02 != 0 {
                r.skip(4)?;
            }
            let default_duration = if flags & 0x08 != 0 {
                Some(r.u32()?)
            } else {
                None
            };
            let default_size = if flags & 0x10 != 0 {
                Some(r.u32()?)
            } else {
                None
            };
            let default_flags = if flags & 0x20 != 0 {
                Some(r.u32()?)
            } else {
                None
            };
            let default_base_is_moof = flags & 0x2_0000 != 0;
            let duration_is_empty = flags & 0x1_0000 != 0;

            let Some(track) = self.tracks.get(&track_id) else {
                continue;
            };
            let timescale = track.timescale as u64;
            let trex = track.trex;

            let mut base_dts = None;
            if let Some(tfdt) = find(&parts, b"tfdt") {
                let mut r = Reader::new(tfdt);
                let version = r.u8()?;
                r.skip(3)?;
                base_dts = Some(if version == 1 {
                    r.u64()? as i64
                } else {
                    r.u32()? as i64
                });
            }
            let mut dts = base_dts.unwrap_or_else(|| *self.next_dts.get(&track_id).unwrap_or(&0));

            // Where the sample data is counted from.
            let base: i64 = match base_data_offset {
                Some(abs) => abs as i64 - self.pos as i64,
                None if default_base_is_moof => 0,
                None => previous_end.unwrap_or(0),
            };
            let mut cursor = base;
            let mut samples = Vec::new();
            if !duration_is_empty {
                for (kind, run) in &parts {
                    if kind != b"trun" {
                        continue;
                    }
                    let mut r = Reader::new(run);
                    let version = r.u8()?;
                    let flags = (r.u8()? as u32) << 16 | (r.u8()? as u32) << 8 | r.u8()? as u32;
                    let count = r.u32()?;
                    if flags & 0x01 != 0 {
                        cursor = base + r.i32()? as i64;
                    }
                    let first_flags = if flags & 0x04 != 0 {
                        Some(r.u32()?)
                    } else {
                        None
                    };
                    for index in 0..count {
                        let duration = if flags & 0x100 != 0 {
                            r.u32()?
                        } else {
                            default_duration.unwrap_or(trex.duration)
                        };
                        let size = if flags & 0x200 != 0 {
                            r.u32()?
                        } else {
                            default_size.unwrap_or(trex.size)
                        };
                        let sflags = if flags & 0x400 != 0 {
                            r.u32()?
                        } else if index == 0 && first_flags.is_some() {
                            first_flags.unwrap_or(0)
                        } else {
                            default_flags.unwrap_or(trex.flags)
                        };
                        let cto: i64 = if flags & 0x800 != 0 {
                            if version == 0 {
                                r.u32()? as i64
                            } else {
                                r.i32()? as i64
                            }
                        } else {
                            0
                        };
                        if cursor < 0 || cursor as usize + size as usize > window.len() {
                            return Err(ParseError::Malformed(
                                "a sample lies outside the segment's `mdat`",
                            ));
                        }
                        let data =
                            window[cursor as usize..cursor as usize + size as usize].to_vec();
                        cursor += size as i64;
                        samples.push(Sample {
                            pts: to_ns(dts + cto, timescale),
                            dts: to_ns(dts, timescale),
                            duration: to_ns(duration as i64, timescale),
                            key: sflags & FLAG_NON_SYNC == 0,
                            data,
                            config: 0,
                        });
                        dts += duration as i64;
                    }
                }
            }
            previous_end = Some(cursor);
            self.next_dts.insert(track_id, dts);
            if !samples.is_empty() {
                events.push(Event::Samples {
                    track: track_id,
                    samples,
                });
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------- moov: a track

/// Returns the track and whether its sample table (`stbl`) lists samples (which a
/// fragmented file does not).
fn parse_trak(trak: &[u8], trex: &HashMap<u32, Trex>) -> Result<Option<(Track, bool)>, ParseError> {
    let parts = boxes(trak)?;
    let Some(tkhd) = find(&parts, b"tkhd") else {
        return Ok(None);
    };
    let mut r = Reader::new(tkhd);
    let version = r.u8()?;
    r.skip(3)?;
    let (id, width, height) = if version == 1 {
        r.skip(16)?;
        let id = r.u32()?;
        r.skip(4 + 8 + 8 + 2 + 2 + 2 + 2 + 36)?;
        (id, r.u32()? >> 16, r.u32()? >> 16)
    } else {
        r.skip(8)?;
        let id = r.u32()?;
        r.skip(4 + 4 + 8 + 2 + 2 + 2 + 2 + 36)?;
        (id, r.u32()? >> 16, r.u32()? >> 16)
    };
    let Some(mdia) = find(&parts, b"mdia") else {
        return Ok(None);
    };
    let mdia = boxes(mdia)?;
    let mdhd = find(&mdia, b"mdhd").ok_or(ParseError::Malformed("a `trak` with no `mdhd`"))?;
    let mut r = Reader::new(mdhd);
    let version = r.u8()?;
    r.skip(3)?;
    let timescale = if version == 1 {
        r.skip(16)?;
        r.u32()?
    } else {
        r.skip(8)?;
        r.u32()?
    };
    if timescale == 0 {
        return Err(ParseError::Malformed("a track with timescale 0"));
    }
    let hdlr = find(&mdia, b"hdlr").ok_or(ParseError::Malformed("a `trak` with no `hdlr`"))?;
    let mut r = Reader::new(hdlr);
    r.skip(8)?;
    let handler = r.take(4)?;
    let kind = match handler {
        b"vide" => TrackKind::Video,
        b"soun" => TrackKind::Audio,
        _ => return Ok(None), // text, hint, metadata: not played
    };
    let minf =
        boxes(find(&mdia, b"minf").ok_or(ParseError::Malformed("a `trak` with no `minf`"))?)?;
    let stbl =
        boxes(find(&minf, b"stbl").ok_or(ParseError::Malformed("a `trak` with no `stbl`"))?)?;
    let has_samples = match find(&stbl, b"stsz") {
        Some(stsz) => {
            let mut r = Reader::new(stsz);
            r.skip(4)?;
            let fixed = r.u32()?;
            let count = r.u32()?;
            count > 0 && (fixed != 0 || count > 0)
        }
        None => false,
    };
    let stsd = find(&stbl, b"stsd").ok_or(ParseError::Malformed("a `trak` with no `stsd`"))?;
    let mut r = Reader::new(stsd);
    r.skip(4)?;
    let entries = r.u32()?;
    if entries == 0 {
        return Err(ParseError::Malformed("an empty `stsd`"));
    }
    let entry_box = boxes(r.rest())?
        .into_iter()
        .next()
        .ok_or(ParseError::Malformed("an `stsd` with no sample entry"))?;
    let (codec, sample_rate, channels) = parse_entry(kind, entry_box.0, entry_box.1)?;
    let info = TrackInfo {
        id,
        kind,
        codec,
        width,
        height,
        sample_rate,
        channels,
    };
    Ok(Some((
        Track {
            info,
            timescale,
            trex: trex.get(&id).copied().unwrap_or_default(),
        },
        has_samples,
    )))
}

fn parse_entry(
    kind: TrackKind,
    fourcc: [u8; 4],
    body: &[u8],
) -> Result<(Codec, u32, u32), ParseError> {
    match kind {
        TrackKind::Video => {
            // 6 reserved + 2 data reference index + 16 + width/height... = 78 bytes, then boxes.
            if body.len() < 78 {
                return Err(ParseError::Malformed(
                    "a video sample entry that is too short",
                ));
            }
            let children = boxes(&body[78..]).unwrap_or_default();
            let codec = match &fourcc {
                b"avc1" | b"avc3" => Codec::H264 {
                    avcc: find(&children, b"avcC").unwrap_or_default().to_vec(),
                },
                b"hvc1" | b"hev1" => Codec::H265 {
                    hvcc: find(&children, b"hvcC").unwrap_or_default().to_vec(),
                },
                b"vp09" => Codec::Vp9,
                b"vp08" => Codec::Vp8,
                b"av01" => Codec::Av1 {
                    av1c: find(&children, b"av1C").unwrap_or_default().to_vec(),
                },
                other => Codec::Unknown(String::from_utf8_lossy(other).trim().to_string()),
            };
            Ok((codec, 0, 0))
        }
        TrackKind::Audio => {
            // 6 reserved + 2 data reference index, then version(2) revision(2) vendor(4),
            // channels(2) sample size(2) pre-defined(2) reserved(2) sample rate(16.16).
            if body.len() < 28 {
                return Err(ParseError::Malformed(
                    "an audio sample entry that is too short",
                ));
            }
            let version = u16::from_be_bytes([body[8], body[9]]);
            let channels = u16::from_be_bytes([body[16], body[17]]) as u32;
            let rate = u32::from_be_bytes(body[24..28].try_into().unwrap()) >> 16;
            // QuickTime version 1 and 2 entries carry extra fields before the child boxes.
            let skip = match version {
                1 => 16,
                2 => 36,
                _ => 0,
            };
            let children = if body.len() >= 28 + skip {
                boxes(&body[28 + skip..]).unwrap_or_default()
            } else {
                Vec::new()
            };
            match &fourcc {
                b"mp4a" => {
                    let asc = find(&children, b"esds")
                        .and_then(esds_asc)
                        .unwrap_or_default();
                    let (r, c) = asc_params(&asc).unwrap_or((rate, channels));
                    Ok((Codec::Aac { asc }, r, c))
                }
                b"Opus" => {
                    let head = find(&children, b"dOps").map(opus_head).unwrap_or_default();
                    let c = head.get(9).map(|c| *c as u32).unwrap_or(channels);
                    Ok((Codec::Opus { head }, 48_000, c))
                }
                b".mp3" | b"mp3 " => Ok((Codec::Mp3, rate, channels)),
                other => Ok((
                    Codec::Unknown(String::from_utf8_lossy(other).trim().to_string()),
                    rate,
                    channels,
                )),
            }
        }
    }
}

/// The AudioSpecificConfig inside an `esds` box.
fn esds_asc(esds: &[u8]) -> Option<Vec<u8>> {
    // 4 bytes of version and flags, then descriptors.
    descriptor(esds.get(4..)?, 0x05)
}

/// Finds the payload of descriptor `wanted`, looking through the nesting MPEG-4 uses.
fn descriptor(mut b: &[u8], wanted: u8) -> Option<Vec<u8>> {
    while !b.is_empty() {
        let tag = *b.first()?;
        let mut at = 1usize;
        let mut len = 0usize;
        for _ in 0..4 {
            let byte = *b.get(at)?;
            at += 1;
            len = (len << 7) | (byte & 0x7f) as usize;
            if byte & 0x80 == 0 {
                break;
            }
        }
        let body = b.get(at..at + len.min(b.len() - at))?;
        if tag == wanted {
            return Some(body.to_vec());
        }
        // ES_Descriptor: ES_ID(2), flags(1), optional fields, then a descriptor.
        // DecoderConfigDescriptor: 13 bytes, then descriptors.
        let inner = match tag {
            0x03 => {
                let flags = *body.get(2)?;
                let mut skip = 3usize;
                if flags & 0x80 != 0 {
                    skip += 2;
                }
                if flags & 0x40 != 0 {
                    skip += 1 + *body.get(skip)? as usize;
                }
                if flags & 0x20 != 0 {
                    skip += 2;
                }
                body.get(skip..)
            }
            0x04 => body.get(13..),
            _ => None,
        };
        if let Some(found) = inner.and_then(|i| descriptor(i, wanted)) {
            return Some(found);
        }
        b = &b[at + len.min(b.len() - at)..];
    }
    None
}

const AAC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// (sample rate, channels) from an AudioSpecificConfig.
fn asc_params(asc: &[u8]) -> Option<(u32, u32)> {
    if asc.len() < 2 {
        return None;
    }
    let bits = u32::from(asc[0]) << 8 | u32::from(asc[1]);
    let index = ((bits >> 7) & 0x0f) as usize;
    let channels = (bits >> 3) & 0x0f;
    let rate = *AAC_RATES.get(index)?;
    Some((rate, channels))
}

/// An `OpusHead` header from an ISO `dOps` box (big-endian fields to little-endian).
fn opus_head(dops: &[u8]) -> Vec<u8> {
    let mut head = Vec::with_capacity(19 + dops.len());
    head.extend_from_slice(b"OpusHead");
    if dops.len() < 11 {
        return Vec::new();
    }
    head.push(1); // version
    head.push(dops[1]); // channel count
    head.extend_from_slice(&u16::from_be_bytes([dops[2], dops[3]]).to_le_bytes()); // pre-skip
    head.extend_from_slice(&u32::from_be_bytes(dops[4..8].try_into().unwrap()).to_le_bytes()); // input rate
    head.extend_from_slice(&i16::from_be_bytes([dops[8], dops[9]]).to_le_bytes()); // output gain
    head.push(dops[10]); // mapping family
    if dops[10] != 0 {
        head.extend_from_slice(&dops[11..]);
    }
    head
}
