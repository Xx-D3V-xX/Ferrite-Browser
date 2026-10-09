/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::Cell;
use std::rc::Rc;

use dom_struct::dom_struct;
use embedder_traits::{AllowOrDeny, EmbedderMsg, PermissionFeature};
use js::context::JSContext;
use js::realm::CurrentRealm;
use script_bindings::reflector::reflect_dom_object_with_cx;
use servo_base::generic_channel;
use servo_media::ServoMedia;
use servo_media::streams::MediaStreamType;
use servo_media::streams::capture::{Constrain, ConstrainRange, MediaTrackConstraintSet};

use crate::conversions::Convert;
use crate::dom::bindings::codegen::Bindings::MediaDevicesBinding::{
    DisplayMediaStreamOptions, MediaDevicesMethods, MediaStreamConstraints,
    MediaTrackSupportedConstraints,
};
use crate::dom::bindings::codegen::Bindings::MediaDeviceInfoBinding::MediaDeviceKind;
use crate::dom::bindings::codegen::UnionTypes::{
    BooleanOrMediaTrackConstraints, ClampedUnsignedLongOrConstrainULongRange as ConstrainULong,
    DoubleOrConstrainDoubleRange as ConstrainDouble,
};
use crate::dom::bindings::error::Error;
use crate::dom::bindings::refcounted::{Trusted, TrustedPromise};
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::DomRoot;
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;
use crate::dom::media::mediadeviceinfo::MediaDeviceInfo;
use crate::dom::media::mediastream::MediaStream;
use crate::dom::media::mediastreamtrack::{MediaStreamTrack, TrackSource};
use crate::dom::promise::Promise;

/// What a page asked to capture. Everything in it is `Send` so it can wait with the
/// permission prompt on another thread.
struct CaptureRequest {
    audio: Option<MediaTrackConstraintSet>,
    video: Option<MediaTrackConstraintSet>,
    /// `getDisplayMedia`: the video is the screen.
    screen: bool,
}

#[dom_struct]
pub(crate) struct MediaDevices {
    eventtarget: EventTarget,
    /// Ferrite: a page that has not been given a camera or a microphone must not learn
    /// what devices there are (their names identify the machine), so
    /// `enumerateDevices` shows labels only after a grant.
    granted_audio: Cell<bool>,
    granted_video: Cell<bool>,
}

impl MediaDevices {
    pub(crate) fn new_inherited() -> MediaDevices {
        MediaDevices {
            eventtarget: EventTarget::new_inherited(),
            granted_audio: Cell::new(false),
            granted_video: Cell::new(false),
        }
    }

    pub(crate) fn new(cx: &mut JSContext, global: &GlobalScope) -> DomRoot<MediaDevices> {
        reflect_dom_object_with_cx(Box::new(MediaDevices::new_inherited()), global, cx)
    }

    /// Ask the embedder (the person) for each feature, then make the streams or refuse.
    /// The page's script thread keeps running while the prompt is open: a thread waits
    /// for the answer and queues a task with it.
    fn request_capture(
        &self,
        cx: &mut CurrentRealm,
        features: Vec<PermissionFeature>,
        request: CaptureRequest,
    ) -> Rc<Promise> {
        let promise = Promise::new_in_realm(cx);
        let global = self.global();
        let Some(webview_id) = global.webview_id() else {
            promise.reject_error(
                cx,
                Error::NotAllowed(Some("There is no tab to ask.".to_string())),
            );
            return promise;
        };
        // Every prompt goes out before any answer is waited for, so the embedder can
        // show one card for the lot.
        let mut receivers = Vec::with_capacity(features.len());
        for feature in features {
            let Some((sender, receiver)) = generic_channel::channel() else {
                promise.reject_error(
                    cx,
                    Error::NotAllowed(Some("Could not ask for permission.".to_string())),
                );
                return promise;
            };
            global.send_to_embedder(EmbedderMsg::PromptPermission(webview_id, feature, sender));
            receivers.push(receiver);
        }
        let task_source = global
            .task_manager()
            .dom_manipulation_task_source()
            .to_sendable();
        let trusted_promise = TrustedPromise::new(promise.clone());
        let trusted_devices = Trusted::new(self);
        let spawned = std::thread::Builder::new()
            .name("capture-permission".to_string())
            .spawn(move || {
                let mut allowed = true;
                for receiver in receivers {
                    if !matches!(receiver.recv(), Ok(AllowOrDeny::Allow)) {
                        allowed = false;
                    }
                }
                task_source.queue(task!(capture_permission_answered: move |cx| {
                    let promise = trusted_promise.root();
                    trusted_devices.root().finish_capture(cx, allowed, request, &promise);
                }));
            });
        if spawned.is_err() {
            warn!("Could not start the capture permission thread");
        }
        promise
    }

    fn finish_capture(
        &self,
        cx: &mut JSContext,
        allowed: bool,
        request: CaptureRequest,
        promise: &Rc<Promise>,
    ) {
        if !allowed {
            promise.reject_error(
                cx,
                Error::NotAllowed(Some("Permission denied".to_string())),
            );
            return;
        }
        let media = ServoMedia::get();
        // A machine with no camera and no microphone to try (a build server, the
        // probe): `FERRITE_MOCK_CAPTURE` makes the engine's own test sources stand in.
        if std::env::var_os("FERRITE_MOCK_CAPTURE").is_some() {
            media.set_capture_mocking(true);
        }
        let global = self.global();
        let stream = MediaStream::new(cx, &global);
        let mut missing = false;
        if let Some(constraints) = request.audio {
            match media.create_audioinput_stream(constraints) {
                Some(audio) => {
                    let track = MediaStreamTrack::new_from(
                        cx,
                        &global,
                        audio,
                        MediaStreamType::Audio,
                        TrackSource::Microphone,
                    );
                    stream.add_track(&track);
                    self.granted_audio.set(true);
                },
                None => missing = true,
            }
        }
        if let Some(constraints) = request.video {
            match media.create_videoinput_stream(constraints) {
                Some(video) => {
                    let source = if request.screen {
                        TrackSource::Screen
                    } else {
                        TrackSource::Camera
                    };
                    let track = MediaStreamTrack::new_from(
                        cx,
                        &global,
                        video,
                        MediaStreamType::Video,
                        source,
                    );
                    stream.add_track(&track);
                    if !request.screen {
                        self.granted_video.set(true);
                    }
                },
                None => missing = true,
            }
        }
        if missing {
            // Devices already opened for this call are let go.
            for track in stream.get_tracks().iter() {
                if let Some(opened) = servo_media::streams::registry::get_stream(&track.id()) {
                    if let Ok(mut opened) = opened.lock() {
                        opened.stop();
                    }
                }
            }
            promise.reject_error(
                cx,
                Error::NotFound(Some("Requested device not found".to_string())),
            );
            return;
        }
        promise.resolve_native(cx, &stream);
    }
}

impl MediaDevicesMethods<crate::DomTypeHolder> for MediaDevices {
    /// <https://w3c.github.io/mediacapture-main/#dom-mediadevices-getusermedia>
    fn GetUserMedia(
        &self,
        cx: &mut CurrentRealm,
        constraints: &MediaStreamConstraints,
    ) -> Rc<Promise> {
        let audio = convert_constraints(&constraints.audio);
        let video = convert_constraints(&constraints.video);
        // Step 2. If neither is requested, reject with a TypeError.
        if audio.is_none() && video.is_none() {
            let promise = Promise::new_in_realm(cx);
            promise.reject_error(
                cx,
                Error::Type(c"At least one of audio and video must be requested".to_owned()),
            );
            return promise;
        }
        let mut features = vec![];
        if audio.is_some() {
            features.push(PermissionFeature::Microphone);
        }
        if video.is_some() {
            features.push(PermissionFeature::Camera);
        }
        self.request_capture(
            cx,
            features,
            CaptureRequest {
                audio,
                video,
                screen: false,
            },
        )
    }

    /// <https://w3c.github.io/mediacapture-screen-share/#dom-mediadevices-getdisplaymedia>
    fn GetDisplayMedia(
        &self,
        cx: &mut CurrentRealm,
        options: &DisplayMediaStreamOptions,
    ) -> Rc<Promise> {
        // The whole screen, as video. Audio capture of the system is not offered;
        // asking for it is not an error (the page gets a video-only stream).
        let video = convert_constraints(&options.video).map(|mut set| {
            set.screen = true;
            set
        });
        let Some(video) = video else {
            let promise = Promise::new_in_realm(cx);
            promise.reject_error(
                cx,
                Error::Type(c"getDisplayMedia needs video".to_owned()),
            );
            return promise;
        };
        self.request_capture(
            cx,
            vec![PermissionFeature::ScreenCapture],
            CaptureRequest {
                audio: None,
                video: Some(video),
                screen: true,
            },
        )
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediadevices-getsupportedconstraints>
    fn GetSupportedConstraints(&self) -> MediaTrackSupportedConstraints {
        MediaTrackSupportedConstraints {
            width: true,
            height: true,
            aspectRatio: true,
            frameRate: true,
            sampleRate: true,
        }
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediadevices-enumeratedevices>
    fn EnumerateDevices(&self, cx: &mut JSContext) -> Rc<Promise> {
        // Step 1.
        let mut realm = CurrentRealm::assert(cx);
        let p = Promise::new_in_realm(&mut realm);

        // Step 2.
        // XXX These steps should be run in parallel.
        // XXX Steps 2.1 - 2.4

        // Step 2.5
        let media = ServoMedia::get();
        let device_monitor = media.get_device_monitor();
        let (mut seen_audio, mut seen_video) = (false, false);
        let result_list = device_monitor
            .enumerate_devices()
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|device| {
                        let kind: MediaDeviceKind = device.kind.convert();
                        let (granted, seen) = match kind {
                            MediaDeviceKind::Videoinput => {
                                (self.granted_video.get(), &mut seen_video)
                            },
                            _ => (self.granted_audio.get(), &mut seen_audio),
                        };
                        // Without a grant a page sees one nameless device of a kind:
                        // enough to know there is one, not which.
                        if !granted {
                            if kind == MediaDeviceKind::Audiooutput || *seen {
                                return None;
                            }
                            *seen = true;
                            return Some(MediaDeviceInfo::new(
                                cx,
                                &self.global(),
                                "",
                                kind,
                                "",
                                "",
                            ));
                        }
                        // XXX The media backend has no way to group devices yet.
                        Some(MediaDeviceInfo::new(
                            cx,
                            &self.global(),
                            &device.device_id,
                            kind,
                            &device.label,
                            "",
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        p.resolve_native(cx, &result_list);

        // Step 3.
        p
    }
}

fn convert_constraints(js: &BooleanOrMediaTrackConstraints) -> Option<MediaTrackConstraintSet> {
    match js {
        BooleanOrMediaTrackConstraints::Boolean(false) => None,
        BooleanOrMediaTrackConstraints::Boolean(true) => Some(Default::default()),
        BooleanOrMediaTrackConstraints::MediaTrackConstraints(c) => Some(MediaTrackConstraintSet {
            height: c.parent.height.as_ref().and_then(convert_culong),
            width: c.parent.width.as_ref().and_then(convert_culong),
            aspect: c.parent.aspectRatio.as_ref().and_then(convert_cdouble),
            frame_rate: c.parent.frameRate.as_ref().and_then(convert_cdouble),
            sample_rate: c.parent.sampleRate.as_ref().and_then(convert_culong),
            screen: false,
        }),
    }
}

fn convert_culong(js: &ConstrainULong) -> Option<Constrain<u32>> {
    match js {
        ConstrainULong::ClampedUnsignedLong(val) => Some(Constrain::Value(*val)),
        ConstrainULong::ConstrainULongRange(range) => {
            if range.parent.min.is_some() || range.parent.max.is_some() {
                Some(Constrain::Range(ConstrainRange {
                    min: range.parent.min,
                    max: range.parent.max,
                    ideal: range.ideal,
                }))
            } else {
                range.exact.map(Constrain::Value)
            }
        },
    }
}

fn convert_cdouble(js: &ConstrainDouble) -> Option<Constrain<f64>> {
    match js {
        ConstrainDouble::Double(val) => Some(Constrain::Value(**val)),
        ConstrainDouble::ConstrainDoubleRange(range) => {
            if range.parent.min.is_some() || range.parent.max.is_some() {
                Some(Constrain::Range(ConstrainRange {
                    min: range.parent.min.map(|x| *x),
                    max: range.parent.max.map(|x| *x),
                    ideal: range.ideal.map(|x| *x),
                }))
            } else {
                range.exact.map(|exact| Constrain::Value(*exact))
            }
        },
    }
}
