/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use gstreamer;
use gstreamer::caps::NoFeature;
use gstreamer::prelude::*;
use servo_media_streams::MediaStreamType;
use servo_media_streams::capture::*;
use servo_media_streams::registry::MediaStreamId;

use crate::media_stream::GStreamerMediaStream;

trait AddToCaps {
    type Bound;
    fn add_to_caps(
        &self,
        name: &str,
        min: Self::Bound,
        max: Self::Bound,
        builder: gstreamer::caps::Builder<NoFeature>,
    ) -> Option<gstreamer::caps::Builder<NoFeature>>;
}

impl AddToCaps for Constrain<u32> {
    type Bound = u32;
    fn add_to_caps(
        &self,
        name: &str,
        min: u32,
        max: u32,
        builder: gstreamer::caps::Builder<NoFeature>,
    ) -> Option<gstreamer::caps::Builder<NoFeature>> {
        match self {
            Constrain::Value(v) => Some(builder.field(name, v)),
            Constrain::Range(r) => {
                let min = into_i32(r.min.unwrap_or(min));
                let max = into_i32(r.max.unwrap_or(max));
                let range = gstreamer::IntRange::<i32>::new(min, max);

                // TODO: Include the ideal caps value in the caps, needs a refactor
                //       of the AddToCaps trait
                Some(builder.field(name, range))
            },
        }
    }
}

fn into_i32(x: u32) -> i32 {
    if x > i32::MAX as u32 {
        i32::MAX
    } else {
        x as i32
    }
}

impl AddToCaps for Constrain<f64> {
    type Bound = i32;
    fn add_to_caps<'a>(
        &self,
        name: &str,
        min: i32,
        max: i32,
        builder: gstreamer::caps::Builder<NoFeature>,
    ) -> Option<gstreamer::caps::Builder<NoFeature>> {
        match self {
            Constrain::Value(v) => {
                Some(builder.field("name", gstreamer::Fraction::approximate_f64(*v)?))
            },
            Constrain::Range(r) => {
                let min = r
                    .min
                    .and_then(gstreamer::Fraction::approximate_f64)
                    .unwrap_or(gstreamer::Fraction::new(min, 1));
                let max = r
                    .max
                    .and_then(gstreamer::Fraction::approximate_f64)
                    .unwrap_or(gstreamer::Fraction::new(max, 1));
                let range = gstreamer::FractionRange::new(min, max);
                // TODO: Include the ideal caps value in the caps, needs a refactor
                //       of the AddToCaps trait
                Some(builder.field(name, range))
            },
        }
    }
}

// TODO(Manishearth): Should support a set of constraints
fn into_caps(set: MediaTrackConstraintSet, format: &str) -> Option<gstreamer::Caps> {
    let mut builder = gstreamer::Caps::builder(format);
    if let Some(w) = set.width {
        builder = w.add_to_caps("width", 0, 1000000, builder)?;
    }
    if let Some(h) = set.height {
        builder = h.add_to_caps("height", 0, 1000000, builder)?;
    }
    if let Some(aspect) = set.aspect {
        builder = aspect.add_to_caps("pixel-aspect-ratio", 0, 1000000, builder)?;
    }
    if let Some(fr) = set.frame_rate {
        builder = fr.add_to_caps("framerate", 0, 1000000, builder)?;
    }
    if let Some(sr) = set.sample_rate {
        builder = sr.add_to_caps("rate", 0, 1000000, builder)?;
    }
    Some(builder.build())
}

struct GstMediaDevices {
    monitor: gstreamer::DeviceMonitor,
}

impl GstMediaDevices {
    pub fn new() -> Self {
        Self {
            monitor: gstreamer::DeviceMonitor::new(),
        }
    }

    pub fn get_track(
        &self,
        video: bool,
        constraints: MediaTrackConstraintSet,
    ) -> Option<GstMediaTrack> {
        let (format, filter) = if video {
            ("video/x-raw", "Video/Source")
        } else {
            ("audio/x-raw", "Audio/Source")
        };
        let caps = into_caps(constraints, format)?;
        let f = self.monitor.add_filter(Some(filter), Some(&caps));
        let devices = self.monitor.devices();
        if let Some(f) = f {
            let _ = self.monitor.remove_filter(f);
        }
        match devices.front() {
            Some(d) => {
                let element = d.create_element(None).ok()?;
                Some(GstMediaTrack { element })
            },
            _ => None,
        }
    }
}

pub struct GstMediaTrack {
    element: gstreamer::Element,
}

/// Ferrite: the elements that capture the whole screen on this operating system, or
/// `None` when the machine has no way to (no X11 on Linux, a GStreamer without the
/// plugin). The first source that exists wins. A capsfilter follows it because a
/// screen source otherwise runs as fast as it can.
fn screen_source() -> Option<Vec<gstreamer::Element>> {
    let candidates: &[(&str, &[(&str, &str)])] = if cfg!(target_os = "macos") {
        &[("avfvideosrc", &[("capture-screen", "true")])]
    } else if cfg!(target_os = "windows") {
        &[
            ("d3d11screencapturesrc", &[]),
            ("dx9screencapsrc", &[]),
            ("gdiscreencapsrc", &[]),
        ]
    } else {
        &[("ximagesrc", &[("use-damage", "false")])]
    };
    for (name, properties) in candidates {
        let mut builder = gstreamer::ElementFactory::make(name);
        for (key, value) in *properties {
            builder = builder.property_from_str(key, value);
        }
        let Ok(source) = builder.build() else { continue };
        let Ok(rate) = gstreamer::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gstreamer::Caps::builder("video/x-raw")
                    .field("framerate", gstreamer::Fraction::new(30, 1))
                    .build(),
            )
            .build()
        else {
            return None;
        };
        return Some(vec![source, rate]);
    }
    None
}

fn create_screen_stream() -> Option<MediaStreamId> {
    screen_source().map(GStreamerMediaStream::create_video_from_chain)
}

fn create_input_stream(
    stream_type: MediaStreamType,
    constraint_set: MediaTrackConstraintSet,
) -> Option<MediaStreamId> {
    if constraint_set.screen {
        return if stream_type == MediaStreamType::Video {
            create_screen_stream()
        } else {
            None
        };
    }
    let devices = GstMediaDevices::new();
    devices
        .get_track(stream_type == MediaStreamType::Video, constraint_set)
        .map(|track| {
            let f = match stream_type {
                MediaStreamType::Audio => GStreamerMediaStream::create_audio_from,
                MediaStreamType::Video => GStreamerMediaStream::create_video_from,
            };
            f(track.element)
        })
}

pub fn create_audioinput_stream(constraint_set: MediaTrackConstraintSet) -> Option<MediaStreamId> {
    create_input_stream(MediaStreamType::Audio, constraint_set)
}

pub fn create_videoinput_stream(constraint_set: MediaTrackConstraintSet) -> Option<MediaStreamId> {
    create_input_stream(MediaStreamType::Video, constraint_set)
}
