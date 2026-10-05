/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The source element for a `MediaSource` (Media Source Extensions).
//!
//! A page hands encoded frames to `SourceBuffer`s; `ferrite-mse` parses and stores them
//! (see its crate documentation). This element, `servomsesrc`, is the player's end of
//! that store: playbin3 asks it for `servomse://<id>`, and it exposes one elementary
//! stream per track (`video_N`, `audio_N` pads, each an `appsrc` fed by a thread that
//! reads the track's frames in decode order). playbin3 demuxes nothing here; it only
//! parses and decodes, as it does for any elementary stream.
//!
//! What this element does that a plain `appsrc` would not:
//!
//! * It waits for every track to have a frame before it creates any pad, so the streams
//!   have one common start and the first segment can begin there (a stream that starts at
//!   an hour into the timeline would otherwise wait an hour for its first frame).
//! * All streams share one group id, so playbin3 plays them together.
//! * It follows seeks: a seek on any stream restarts all feeders at the sync sample
//!   before the target, and a frame fetched before the seek is never pushed after it.
//! * A track whose configuration changes (a new initialization segment with another
//!   resolution) gets new caps in front of the first frame that needs them.
//! * When a feeder has nothing left to push and its queue is empty, the pipeline is told
//!   it is buffering, and told it is done when frames flow again, so a stalled player
//!   pauses instead of racing ahead of its data.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use ferrite_mse::{Codec, Next, Sample, Shared, TrackInfo, TrackKind};
use glib::subclass::prelude::*;
use gstreamer::prelude::*;
use gstreamer::subclass::prelude::*;
use url::Url;

/// How long a feeder waits for a frame before it looks at the world again.
const POLL: Duration = Duration::from_millis(100);

/// How long the pad builder waits for the page between looks at whether it was stopped.
const START_POLL: Duration = Duration::from_millis(100);

/// A feeder that has nothing more to push and whose data ends less than this ahead of the
/// playhead tells the pipeline it is buffering...
const STARVE_AHEAD: i64 = 80_000_000;

/// ...and says it is done once the data reaches this far ahead again.
const RESUME_AHEAD: i64 = 250_000_000;

/// What each track's `appsrc` may queue before the feeder blocks.
const QUEUE_BYTES: u64 = 24 * 1024 * 1024;

mod imp {
    use super::*;

    pub(super) static VIDEO_PAD_TEMPLATE: LazyLock<gstreamer::PadTemplate> = LazyLock::new(|| {
        gstreamer::PadTemplate::new(
            "video_%u",
            gstreamer::PadDirection::Src,
            gstreamer::PadPresence::Sometimes,
            &gstreamer::Caps::new_any(),
        )
        .expect("Could not create video src pad template")
    });

    pub(super) static AUDIO_PAD_TEMPLATE: LazyLock<gstreamer::PadTemplate> = LazyLock::new(|| {
        gstreamer::PadTemplate::new(
            "audio_%u",
            gstreamer::PadDirection::Src,
            gstreamer::PadPresence::Sometimes,
            &gstreamer::Caps::new_any(),
        )
        .expect("Could not create audio src pad template")
    });

    #[derive(Default)]
    struct State {
        shared: Option<Arc<Shared>>,
        stop: Option<Arc<AtomicBool>>,
    }

    #[derive(Default)]
    pub struct ServoMseSrc {
        state: Mutex<State>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ServoMseSrc {
        const NAME: &'static str = "ServoMseSrc";
        type Type = super::ServoMseSrc;
        type ParentType = gstreamer::Bin;
        type Interfaces = (gstreamer::URIHandler,);
    }

    impl ObjectImpl for ServoMseSrc {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj()
                .set_element_flags(gstreamer::ElementFlags::SOURCE);
        }
    }

    impl GstObjectImpl for ServoMseSrc {}

    impl ElementImpl for ServoMseSrc {
        fn metadata() -> Option<&'static gstreamer::subclass::ElementMetadata> {
            static ELEMENT_METADATA: LazyLock<gstreamer::subclass::ElementMetadata> =
                LazyLock::new(|| {
                    gstreamer::subclass::ElementMetadata::new(
                        "Servo Media Source Extensions Source",
                        "Source/Audio/Video",
                        "Feed the player with the frames a page appended to a MediaSource",
                        "Ferrite",
                    )
                });
            Some(&*ELEMENT_METADATA)
        }

        fn pad_templates() -> &'static [gstreamer::PadTemplate] {
            static PAD_TEMPLATES: LazyLock<Vec<gstreamer::PadTemplate>> =
                LazyLock::new(|| vec![VIDEO_PAD_TEMPLATE.clone(), AUDIO_PAD_TEMPLATE.clone()]);
            PAD_TEMPLATES.as_ref()
        }

        fn change_state(
            &self,
            transition: gstreamer::StateChange,
        ) -> Result<gstreamer::StateChangeSuccess, gstreamer::StateChangeError> {
            match transition {
                gstreamer::StateChange::ReadyToPaused => self.start()?,
                gstreamer::StateChange::PausedToReady => self.stop(),
                _ => {},
            }
            let result = self.parent_change_state(transition);
            if transition == gstreamer::StateChange::ReadyToNull {
                self.stop();
            }
            result
        }
    }

    impl BinImpl for ServoMseSrc {}

    impl ServoMseSrc {
        fn start(&self) -> Result<(), gstreamer::StateChangeError> {
            let mut state = self.state.lock().unwrap();
            let Some(shared) = state.shared.clone() else {
                gstreamer::element_imp_error!(
                    self,
                    gstreamer::ResourceError::NotFound,
                    ["No MediaSource for this URI"]
                );
                return Err(gstreamer::StateChangeError);
            };
            if let Some(old) = state.stop.take() {
                old.store(true, Ordering::Relaxed);
            }
            let stop = Arc::new(AtomicBool::new(false));
            state.stop = Some(stop.clone());
            let bin = self.obj().downgrade();
            std::thread::Builder::new()
                .name("mse-pads".into())
                .spawn(move || build_pads(bin, shared, stop))
                .map_err(|_| gstreamer::StateChangeError)?;
            Ok(())
        }

        fn stop(&self) {
            if let Some(stop) = self.state.lock().unwrap().stop.take() {
                stop.store(true, Ordering::Relaxed);
            }
        }
    }

    impl URIHandlerImpl for ServoMseSrc {
        const URI_TYPE: gstreamer::URIType = gstreamer::URIType::Src;

        fn protocols() -> &'static [&'static str] {
            &["servomse"]
        }

        fn uri(&self) -> Option<String> {
            Some("servomse://".to_string())
        }

        fn set_uri(&self, uri: &str) -> Result<(), glib::Error> {
            let bad = || {
                glib::Error::new(
                    gstreamer::URIError::BadUri,
                    format!("Invalid URI '{uri}'").as_str(),
                )
            };
            let parsed = Url::parse(uri).map_err(|_| bad())?;
            if parsed.scheme() != "servomse" {
                return Err(bad());
            }
            let id: u64 = parsed.host_str().and_then(|h| h.parse().ok()).ok_or_else(bad)?;
            let shared = ferrite_mse::lookup(id).ok_or_else(bad)?;
            self.state.lock().unwrap().shared = Some(shared);
            Ok(())
        }
    }
}

glib::wrapper! {
    pub struct ServoMseSrc(ObjectSubclass<imp::ServoMseSrc>)
        @extends gstreamer::Bin, gstreamer::Element, gstreamer::Object,
        @implements gstreamer::URIHandler;
}

unsafe impl Send for ServoMseSrc {}
unsafe impl Sync for ServoMseSrc {}

/// Registers the element under the name `servomsesrc`.
pub fn register_servo_mse_src() -> Result<(), glib::BoolError> {
    gstreamer::Element::register(
        None,
        "servomsesrc",
        gstreamer::Rank::NONE,
        ServoMseSrc::static_type(),
    )
}

/// Tells the pipeline whether the source ran dry: any feeder with an empty queue and
/// nothing left to push is "starved", and while any is, the pipeline hears
/// `Buffering(0)`.
struct Starvation {
    bin: glib::WeakRef<gstreamer::Element>,
    starved: Mutex<u64>,
}

impl Starvation {
    /// Where the pipeline is playing, as the sinks see it (nanoseconds).
    fn position(&self) -> Option<i64> {
        let mut object: gstreamer::Object = self.bin.upgrade()?.upcast();
        while let Some(parent) = object.parent() {
            object = parent;
        }
        let pipeline = object.downcast::<gstreamer::Element>().ok()?;
        pipeline
            .query_position::<gstreamer::ClockTime>()
            .map(|p| p.nseconds() as i64)
    }

    fn post(&self, percent: i32) {
        if let Some(bin) = self.bin.upgrade() {
            let message = gstreamer::message::Buffering::builder(percent)
                .src(&bin)
                .build();
            let _ = bin.post_message(message);
        }
    }

    fn starved(&self, slot: usize) {
        let mut mask = self.starved.lock().unwrap();
        let was = *mask;
        *mask |= 1 << slot.min(63);
        if was == 0 {
            self.post(0);
        }
    }

    fn fed(&self, slot: usize) {
        let mut mask = self.starved.lock().unwrap();
        if *mask == 0 {
            return;
        }
        *mask &= !(1 << slot.min(63));
        if *mask == 0 {
            self.post(100);
        }
    }
}

/// Everything one track's feeder thread needs.
struct Feeder {
    appsrc: gstreamer_app::AppSrc,
    shared: Arc<Shared>,
    slot: usize,
    stop: Arc<AtomicBool>,
    /// Held while a frame is checked and pushed, and while a seek is recorded, so a
    /// frame fetched before a seek cannot be pushed after it.
    push_lock: Arc<Mutex<()>>,
    starvation: Arc<Starvation>,
}

/// The pad builder: waits for the page to have appended something for every track, then
/// makes a pad and a feeder for each.
fn build_pads(bin: glib::WeakRef<ServoMseSrc>, shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let start = loop {
        if stop.load(Ordering::Relaxed) || shared.closed() {
            return;
        }
        if let Some(start) = shared.wait_start(START_POLL) {
            break start;
        }
    };
    let Some(bin) = bin.upgrade() else { return };
    let group_id = gstreamer::GroupId::next();
    let push_lock = Arc::new(Mutex::new(()));
    let starvation = Arc::new(Starvation {
        bin: bin.upcast_ref::<gstreamer::Element>().downgrade(),
        starved: Mutex::new(0),
    });
    let mut feeders = Vec::new();
    for slot in 0..shared.slot_count() {
        let Some(info) = shared.track_info(slot) else {
            continue;
        };
        if matches!(info.codec, Codec::Unknown(_)) || caps_for(&info).is_none() {
            gstreamer::warning!(
                gstreamer::CAT_DEFAULT,
                obj = &bin,
                "No decoder description for track {}: {:?}",
                info.id,
                info.codec
            );
            continue;
        }
        match make_pad(&bin, &shared, slot, &info, group_id, start, &push_lock) {
            Ok(appsrc) => feeders.push(Feeder {
                appsrc,
                shared: shared.clone(),
                slot,
                stop: stop.clone(),
                push_lock: push_lock.clone(),
                starvation: starvation.clone(),
            }),
            Err(error) => {
                gstreamer::element_error!(
                    bin,
                    gstreamer::CoreError::Failed,
                    ["Could not create a stream for track {}: {}", info.id, error]
                );
                return;
            },
        }
    }
    if feeders.is_empty() {
        gstreamer::element_error!(
            bin,
            gstreamer::StreamError::CodecNotFound,
            ["The media source holds no stream this player can decode"]
        );
        return;
    }
    bin.no_more_pads();
    for feeder in feeders {
        let name = format!("mse-feed-{}", feeder.slot);
        let _ = std::thread::Builder::new()
            .name(name)
            .spawn(move || feed(feeder));
    }
}

fn make_pad(
    bin: &ServoMseSrc,
    shared: &Arc<Shared>,
    slot: usize,
    info: &TrackInfo,
    group_id: gstreamer::GroupId,
    start: i64,
    push_lock: &Arc<Mutex<()>>,
) -> Result<gstreamer_app::AppSrc, glib::BoolError> {
    let (prefix, template) = match info.kind {
        TrackKind::Video => ("video", &*imp::VIDEO_PAD_TEMPLATE),
        TrackKind::Audio => ("audio", &*imp::AUDIO_PAD_TEMPLATE),
    };
    let appsrc = gstreamer_app::AppSrc::builder()
        .name(format!("{prefix}-src-{slot}"))
        .format(gstreamer::Format::Time)
        .stream_type(gstreamer_app::AppStreamType::Seekable)
        .max_bytes(QUEUE_BYTES)
        .block(true)
        .is_live(false)
        .build();
    if let Some(duration) = shared.duration() {
        appsrc.set_duration(gstreamer::ClockTime::from_nseconds(duration.max(0) as u64));
    }
    {
        let shared = shared.clone();
        let push_lock = push_lock.clone();
        appsrc.set_callbacks(
            gstreamer_app::AppSrcCallbacks::builder()
                .seek_data(move |_, offset| {
                    let _guard = push_lock.lock().unwrap();
                    shared.seek(offset as i64);
                    true
                })
                .build(),
        );
    }

    let stream_id = format!("servomse/{prefix}/{slot}");
    let first_segment = AtomicBool::new(true);
    let src_pad = appsrc
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("appsrc has no src pad"))?;
    src_pad.add_probe(gstreamer::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
        let Some(gstreamer::PadProbeData::Event(ref event)) = info.data else {
            return gstreamer::PadProbeReturn::Ok;
        };
        let replacement = match event.view() {
            // One group for every stream, with ids that stay the same across seeks.
            gstreamer::EventView::StreamStart(_) => Some(
                gstreamer::event::StreamStart::builder(&stream_id)
                    .group_id(group_id)
                    .seqnum(event.seqnum())
                    .build(),
            ),
            // The first segment begins where the data begins, not at zero.
            gstreamer::EventView::Segment(segment) => {
                if first_segment.swap(false, Ordering::Relaxed) {
                    segment
                        .segment()
                        .clone()
                        .downcast::<gstreamer::ClockTime>()
                        .ok()
                        .filter(|s| s.start() == Some(gstreamer::ClockTime::ZERO))
                        .map(|mut s| {
                            let at = gstreamer::ClockTime::from_nseconds(start.max(0) as u64);
                            s.set_start(at);
                            s.set_position(at);
                            s.set_time(at);
                            gstreamer::event::Segment::builder(&s)
                                .seqnum(event.seqnum())
                                .build()
                        })
                } else {
                    None
                }
            },
            _ => None,
        };
        if let Some(replacement) = replacement {
            info.data = Some(gstreamer::PadProbeData::Event(replacement));
        }
        gstreamer::PadProbeReturn::Ok
    });

    bin.add(&appsrc)?;
    let ghost = gstreamer::GhostPad::builder_from_template(template)
        .name(format!("{prefix}_{slot}"))
        .build();
    ghost.set_target(Some(&src_pad))?;
    ghost.set_active(true)?;
    bin.add_pad(&ghost)?;
    appsrc.sync_state_with_parent()?;
    Ok(appsrc)
}

/// The feeding thread of one track.
fn feed(feeder: Feeder) {
    let Feeder {
        appsrc,
        shared,
        slot,
        stop,
        push_lock,
        starvation,
    } = feeder;
    let mut handle = shared.handle(slot);
    let mut config: Option<u32> = None;
    let mut discont = true;
    let mut ended = false;
    // The end of the latest frame pushed since the last flush.
    let mut pushed_end: Option<i64> = None;
    while !stop.load(Ordering::Relaxed) {
        match handle.next(POLL) {
            Next::Sample(sample) => {
                ended = false;
                let _guard = push_lock.lock().unwrap();
                if handle.stale() {
                    continue;
                }
                if config != Some(sample.config) {
                    if let Some(info) = shared.track_info_at(slot, sample.config)
                        && let Some(caps) = caps_for(&info)
                    {
                        appsrc.set_caps(Some(&caps));
                    }
                    config = Some(sample.config);
                }
                match appsrc.push_buffer(buffer_for(&sample, discont)) {
                    Ok(_) => {
                        discont = false;
                        pushed_end = Some(pushed_end.map_or(sample.end(), |e| e.max(sample.end())));
                        let ahead = starvation.position().map(|p| pushed_end.unwrap_or(p) - p);
                        if ahead.is_none_or(|a| a >= RESUME_AHEAD) {
                            starvation.fed(slot);
                        }
                    },
                    // A seek is under way: the next call to `next` reports it.
                    Err(gstreamer::FlowError::Flushing) => {},
                    Err(_) => break,
                }
            },
            Next::Flush { .. } => {
                discont = true;
                ended = false;
                pushed_end = None;
            },
            Next::Wait => {
                if !ended && appsrc.current_level_bytes() == 0 {
                    let ahead = match pushed_end {
                        // Nothing buffered at the place the player is going to.
                        None => Some(0),
                        Some(end) => starvation.position().map(|p| end - p),
                    };
                    if ahead.is_some_and(|a| a < STARVE_AHEAD) {
                        starvation.starved(slot);
                    }
                }
            },
            Next::Eos => {
                ended = true;
                let _ = appsrc.end_of_stream();
                starvation.fed(slot);
            },
            Next::Closed => break,
        }
    }
}

fn clock(ns: i64) -> gstreamer::ClockTime {
    gstreamer::ClockTime::from_nseconds(ns.max(0) as u64)
}

fn buffer_for(sample: &Sample, discont: bool) -> gstreamer::Buffer {
    let mut buffer = gstreamer::Buffer::from_slice(sample.data.clone());
    {
        let b = buffer.get_mut().expect("a new buffer is writable");
        b.set_pts(clock(sample.pts));
        b.set_dts(clock(sample.dts));
        b.set_duration(clock(sample.duration));
        let mut flags = gstreamer::BufferFlags::empty();
        if !sample.key {
            flags |= gstreamer::BufferFlags::DELTA_UNIT;
        }
        if discont {
            flags |= gstreamer::BufferFlags::DISCONT;
        }
        b.set_flags(flags);
    }
    buffer
}

/// The caps a decoder needs for a track, from what the initialization segment said.
/// `None` for a codec this crate does not describe.
fn caps_for(info: &TrackInfo) -> Option<gstreamer::Caps> {
    let sized = |mut builder: gstreamer::caps::Builder<gstreamer::caps::NoFeature>| {
        if info.width > 0 && info.height > 0 {
            builder = builder
                .field("width", info.width as i32)
                .field("height", info.height as i32);
        }
        builder
    };
    let audio = |builder: gstreamer::caps::Builder<gstreamer::caps::NoFeature>| {
        let mut builder = builder;
        if info.sample_rate > 0 {
            builder = builder.field("rate", info.sample_rate as i32);
        }
        if info.channels > 0 {
            builder = builder.field("channels", info.channels as i32);
        }
        builder
    };
    let buffer = |bytes: &[u8]| gstreamer::Buffer::from_slice(bytes.to_vec());
    let caps = match &info.codec {
        Codec::H264 { avcc } if !avcc.is_empty() => sized(
            gstreamer::Caps::builder("video/x-h264")
                .field("stream-format", "avc")
                .field("alignment", "au")
                .field("codec_data", buffer(avcc)),
        )
        .build(),
        Codec::H265 { hvcc } if !hvcc.is_empty() => sized(
            gstreamer::Caps::builder("video/x-h265")
                .field("stream-format", "hvc1")
                .field("alignment", "au")
                .field("codec_data", buffer(hvcc)),
        )
        .build(),
        Codec::Vp8 => sized(gstreamer::Caps::builder("video/x-vp8")).build(),
        Codec::Vp9 => sized(gstreamer::Caps::builder("video/x-vp9")).build(),
        Codec::Av1 { av1c } => {
            let mut builder = gstreamer::Caps::builder("video/x-av1")
                .field("stream-format", "obu-stream")
                .field("alignment", "tu");
            if !av1c.is_empty() {
                builder = builder.field("codec_data", buffer(av1c));
            }
            sized(builder).build()
        },
        Codec::Aac { asc } if !asc.is_empty() => audio(
            gstreamer::Caps::builder("audio/mpeg")
                .field("mpegversion", 4i32)
                .field("stream-format", "raw")
                .field("codec_data", buffer(asc)),
        )
        .build(),
        Codec::Opus { head } => {
            let mut builder = audio(gstreamer::Caps::builder("audio/x-opus"));
            // The `OpusHead`: version, channels, pre-skip, rate, gain, mapping family and,
            // for families other than 0, the stream layout.
            let family = head.get(18).copied().unwrap_or(0);
            builder = builder.field("channel-mapping-family", family as i32);
            if family != 0 && head.len() >= 21 {
                let channels = head[9] as usize;
                if head.len() >= 21 + channels {
                    builder = builder
                        .field("stream-count", head[19] as i32)
                        .field("coupled-count", head[20] as i32)
                        .field(
                            "channel-mapping",
                            gstreamer::Array::new(
                                head[21..21 + channels].iter().map(|&c| c as i32),
                            ),
                        );
                }
            }
            builder.build()
        },
        Codec::Vorbis { headers } if headers.len() == 3 => audio(
            gstreamer::Caps::builder("audio/x-vorbis").field(
                "streamheader",
                gstreamer::Array::new(headers.iter().map(|h| buffer(h))),
            ),
        )
        .build(),
        Codec::Mp3 => audio(
            gstreamer::Caps::builder("audio/mpeg")
                .field("mpegversion", 1i32)
                .field("layer", 3i32)
                .field("parsed", true),
        )
        .build(),
        _ => return None,
    };
    Some(caps)
}
