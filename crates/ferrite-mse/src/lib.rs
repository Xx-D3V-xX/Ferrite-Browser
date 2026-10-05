//! The data path of Media Source Extensions, without any engine in it.
//!
//! A page appends bytes to a `SourceBuffer`; those bytes are a piece of a fragmented MP4
//! or a WebM. This crate turns them into what a player needs and what the page can ask
//! about:
//!
//! * [`Parser`] reads the bytes as they arrive, in any chunking, and yields the *initial
//!   segment* ([`Event::Init`]: the tracks, their codecs and sizes) and, for every media
//!   segment, the encoded frames of each track with their presentation and decode times
//!   ([`Event::Samples`]).
//! * [`TrackBuffer`] is a track's *coded frame store*: frames sorted by decode time,
//!   replaced when a later append covers the same time, removable by range, with the
//!   buffered ranges the page reads as `SourceBuffer.buffered`.
//! * [`Shared`] is what several `SourceBuffer`s and the player's feeding threads hold
//!   together: the tracks, the end of the stream, the duration, and a way to wait for the
//!   next frame to play.
//!
//! All times are nanoseconds in an `i64`, as GStreamer counts them.

mod mp4;
mod registry;
mod shared;
mod store;
mod webm;

pub use registry::{lookup, register, unregister};
pub use shared::{Next, QuotaExceeded, Shared, TrackHandle};
pub use store::TrackBuffer;

use std::fmt;

/// A track's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// What an encoded frame is, with whatever the decoder needs besides the frames.
#[derive(Debug, Clone, PartialEq)]
pub enum Codec {
    /// `avcC` box contents.
    H264 {
        avcc: Vec<u8>,
    },
    /// `hvcC` box contents.
    H265 {
        hvcc: Vec<u8>,
    },
    Vp8,
    Vp9,
    /// `av1C` box contents (may be empty for WebM that omits it).
    Av1 {
        av1c: Vec<u8>,
    },
    /// An AudioSpecificConfig.
    Aac {
        asc: Vec<u8>,
    },
    /// An `OpusHead` header.
    Opus {
        head: Vec<u8>,
    },
    /// The three Vorbis headers: identification, comment, setup.
    Vorbis {
        headers: Vec<Vec<u8>>,
    },
    Mp3,
    /// A codec the container named and this crate does not know.
    Unknown(String),
}

/// A track named by an initialization segment.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    /// The container's number for the track (the MP4 `track_ID`, the WebM track number).
    pub id: u32,
    pub kind: TrackKind,
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
    pub channels: u32,
}

impl TrackInfo {
    /// The codec as the `codecs=` parameter of a MIME type would say it (`avc1.42c00d`,
    /// `mp4a.40.2`, `vp09.00.10.08`, `opus`), so `isTypeSupported` and the page's own
    /// bookkeeping can use it.
    pub fn codec_string(&self) -> String {
        match &self.codec {
            Codec::H264 { avcc } if avcc.len() >= 4 => {
                format!("avc1.{:02x}{:02x}{:02x}", avcc[1], avcc[2], avcc[3])
            }
            Codec::H264 { .. } => "avc1".to_string(),
            Codec::H265 { .. } => "hvc1".to_string(),
            Codec::Vp8 => "vp8".to_string(),
            Codec::Vp9 => "vp09.00.10.08".to_string(),
            Codec::Av1 { .. } => "av01.0.04M.08".to_string(),
            Codec::Aac { asc } if !asc.is_empty() => format!("mp4a.40.{}", asc[0] >> 3),
            Codec::Aac { .. } => "mp4a.40.2".to_string(),
            Codec::Opus { .. } => "opus".to_string(),
            Codec::Vorbis { .. } => "vorbis".to_string(),
            Codec::Mp3 => "mp3".to_string(),
            Codec::Unknown(name) => name.clone(),
        }
    }
}

/// One encoded frame (or audio packet).
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Presentation time, nanoseconds.
    pub pts: i64,
    /// Decode time, nanoseconds.
    pub dts: i64,
    pub duration: i64,
    /// A sync sample: decoding can start here.
    pub key: bool,
    pub data: Vec<u8>,
    /// Which initialization segment's configuration the frame was appended under (see
    /// [`Shared::track_info_at`]). The parsers leave it 0; [`Shared::append`] sets it.
    pub config: u32,
}

impl Sample {
    /// The end of the frame's presentation interval.
    pub fn end(&self) -> i64 {
        self.pts + self.duration
    }
}

/// What a [`Parser`] found in the bytes it was given.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// An initialization segment: the tracks, and the duration if the container says.
    Init {
        tracks: Vec<TrackInfo>,
        duration: Option<i64>,
    },
    /// The frames of one track from one media segment (or part of one).
    Samples { track: u32, samples: Vec<Sample> },
}

/// A container this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    WebM,
}

impl Container {
    /// The container a MIME type names (`video/mp4`, `audio/webm; codecs="opus"`).
    pub fn from_mime(mime: &str) -> Option<Container> {
        let kind = mime.split(';').next()?.trim().to_ascii_lowercase();
        match kind.as_str() {
            "video/mp4" | "audio/mp4" | "audio/x-m4a" | "video/x-m4v" => Some(Container::Mp4),
            "video/webm" | "audio/webm" => Some(Container::WebM),
            _ => None,
        }
    }
}

/// Whether `MediaSource.isTypeSupported(mime)` should say yes, as far as this crate can
/// tell: a container it reads, and only codecs it can describe to a decoder. Whether the
/// machine has the decoder is for the player to say.
pub fn is_type_supported(mime: &str) -> bool {
    if Container::from_mime(mime).is_none() {
        return false;
    }
    let Some(codecs) = codecs_of(mime) else {
        // No `codecs` parameter: the container alone is not enough for MSE.
        return false;
    };
    !codecs.is_empty() && codecs.iter().all(|c| known_codec(c))
}

/// The entries of a MIME type's `codecs` parameter.
pub fn codecs_of(mime: &str) -> Option<Vec<String>> {
    for part in mime.split(';').skip(1) {
        let part = part.trim();
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("codecs") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            return Some(
                value
                    .split(',')
                    .map(|c| c.trim().to_ascii_lowercase())
                    .filter(|c| !c.is_empty())
                    .collect(),
            );
        }
    }
    None
}

fn known_codec(codec: &str) -> bool {
    codec.starts_with("avc1.")
        || codec.starts_with("avc3.")
        || codec.starts_with("hvc1.")
        || codec.starts_with("hev1.")
        || codec.starts_with("mp4a.40.")
        || codec.starts_with("mp4a.69")
        || codec.starts_with("mp4a.6b")
        || codec.starts_with("vp09.")
        || codec.starts_with("av01.")
        || matches!(
            codec,
            "vp8" | "vp9" | "opus" | "vorbis" | "mp3" | "avc1" | "av1" | "flac"
        )
}

/// Why bytes could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The bytes are not what the container allows (a box too small, a bad size, a bad id).
    Malformed(&'static str),
    /// Something the container allows that this crate does not read.
    Unsupported(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Malformed(m) => write!(f, "malformed media segment: {m}"),
            ParseError::Unsupported(m) => write!(f, "unsupported media segment: {m}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// An incremental parser for one `SourceBuffer`'s byte stream.
pub struct Parser {
    inner: Inner,
}

enum Inner {
    Mp4(mp4::Mp4Parser),
    WebM(webm::WebmParser),
}

impl Parser {
    pub fn new(container: Container) -> Parser {
        Parser {
            inner: match container {
                Container::Mp4 => Inner::Mp4(mp4::Mp4Parser::default()),
                Container::WebM => Inner::WebM(webm::WebmParser::default()),
            },
        }
    }

    /// Feeds bytes. Anything not yet a whole box or element is kept for the next call.
    /// An error leaves the parser unusable until [`Parser::reset`].
    pub fn append(&mut self, data: &[u8]) -> Result<Vec<Event>, ParseError> {
        match &mut self.inner {
            Inner::Mp4(p) => p.append(data),
            Inner::WebM(p) => p.append(data),
        }
    }

    /// The "reset parser state" algorithm: drop partial data and expect a new segment
    /// (`abort()`, or an error). Known tracks are kept: a media segment may follow
    /// without a new initialization segment.
    pub fn reset(&mut self) {
        match &mut self.inner {
            Inner::Mp4(p) => p.reset(),
            Inner::WebM(p) => p.reset(),
        }
    }

    /// The bytes held back, waiting for the rest of a box or element.
    pub fn pending_bytes(&self) -> usize {
        match &self.inner {
            Inner::Mp4(p) => p.pending_bytes(),
            Inner::WebM(p) => p.pending_bytes(),
        }
    }
}

/// Converts `ticks` of `timescale` per second to nanoseconds without overflow.
pub(crate) fn to_ns(ticks: i64, timescale: u64) -> i64 {
    if timescale == 0 {
        return 0;
    }
    ((ticks as i128 * 1_000_000_000) / timescale as i128) as i64
}
