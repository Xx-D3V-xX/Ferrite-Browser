/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::Cell;
use std::rc::Rc;

use dom_struct::dom_struct;
use crate::dom::bindings::num::Finite;
use script_bindings::reflector::reflect_dom_object_with_cx;
use servo_media::streams::MediaStreamType;
use servo_media::streams::registry::{MediaStreamId, get_stream};

use crate::dom::bindings::codegen::Bindings::MediaStreamTrackBinding::{
    MediaStreamTrackMethods, MediaStreamTrackState, MediaTrackSettings,
};
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::str::DOMString;
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;

/// What a track was made from, for `label` and `getSettings()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TrackSource {
    /// Not a capture track (a remote WebRTC track, an audio node).
    Other,
    Camera,
    Microphone,
    /// `getDisplayMedia`, the whole screen.
    Screen,
}

/// A count of the tracks (a track and its clones) that share one device.
#[derive(Clone, Default)]
pub(crate) struct Sharing(Rc<Cell<u32>>);

#[dom_struct]
pub(crate) struct MediaStreamTrack {
    eventtarget: EventTarget,
    #[ignore_malloc_size_of = "defined in servo-media"]
    #[no_trace]
    id: MediaStreamId,
    #[ignore_malloc_size_of = "defined in servo-media"]
    #[no_trace]
    ty: MediaStreamType,
    #[ignore_malloc_size_of = "a plain enum"]
    #[no_trace]
    source: TrackSource,
    enabled: Cell<bool>,
    ended: Cell<bool>,
    /// How many live tracks (this one and its clones) share the device. The device
    /// is released when the last of them stops.
    #[ignore_malloc_size_of = "a counter"]
    #[no_trace]
    sharing: Sharing,
}

impl MediaStreamTrack {
    pub(crate) fn new_inherited(
        id: MediaStreamId,
        ty: MediaStreamType,
        source: TrackSource,
        sharing: Sharing,
    ) -> MediaStreamTrack {
        sharing.0.set(sharing.0.get() + 1);
        MediaStreamTrack {
            eventtarget: EventTarget::new_inherited(),
            id,
            ty,
            source,
            enabled: Cell::new(true),
            ended: Cell::new(false),
            sharing,
        }
    }

    pub(crate) fn new(
        cx: &mut js::context::JSContext,
        global: &GlobalScope,
        id: MediaStreamId,
        ty: MediaStreamType,
    ) -> DomRoot<MediaStreamTrack> {
        Self::new_from(cx, global, id, ty, TrackSource::Other)
    }

    /// A track that captures a camera, a microphone or the screen.
    pub(crate) fn new_from(
        cx: &mut js::context::JSContext,
        global: &GlobalScope,
        id: MediaStreamId,
        ty: MediaStreamType,
        source: TrackSource,
    ) -> DomRoot<MediaStreamTrack> {
        reflect_dom_object_with_cx(
            Box::new(MediaStreamTrack::new_inherited(
                id,
                ty,
                source,
                Sharing::default(),
            )),
            global,
            cx,
        )
    }

    pub(crate) fn id(&self) -> MediaStreamId {
        self.id
    }

    pub(crate) fn ty(&self) -> MediaStreamType {
        self.ty
    }

    pub(crate) fn is_live(&self) -> bool {
        !self.ended.get()
    }
}

impl MediaStreamTrackMethods<crate::DomTypeHolder> for MediaStreamTrack {
    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-kind>
    fn Kind(&self) -> DOMString {
        match self.ty {
            MediaStreamType::Video => "video".into(),
            MediaStreamType::Audio => "audio".into(),
        }
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-id>
    fn Id(&self) -> DOMString {
        self.id.id().to_string().into()
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-label>
    fn Label(&self) -> DOMString {
        match self.source {
            TrackSource::Camera => "Camera".into(),
            TrackSource::Microphone => "Microphone".into(),
            TrackSource::Screen => "Screen".into(),
            TrackSource::Other => "".into(),
        }
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-enabled>
    fn Enabled(&self) -> bool {
        self.enabled.get()
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-enabled>
    fn SetEnabled(&self, value: bool) {
        self.enabled.set(value);
        if self.ended.get() {
            return;
        }
        if let Some(stream) = get_stream(&self.id) {
            if let Ok(mut stream) = stream.lock() {
                stream.set_enabled(value);
            }
        }
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-muted>
    fn Muted(&self) -> bool {
        false
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-readystate>
    fn ReadyState(&self) -> MediaStreamTrackState {
        if self.ended.get() {
            MediaStreamTrackState::Ended
        } else {
            MediaStreamTrackState::Live
        }
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-clone>
    fn Clone(&self, cx: &mut js::context::JSContext) -> DomRoot<MediaStreamTrack> {
        let clone = reflect_dom_object_with_cx(
            Box::new(MediaStreamTrack::new_inherited(
                self.id,
                self.ty,
                self.source,
                self.sharing.clone(),
            )),
            &*self.global(),
            cx,
        );
        clone.enabled.set(self.enabled.get());
        clone.ended.set(self.ended.get());
        clone
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-stop>
    fn Stop(&self, _cx: &mut js::context::JSContext) {
        if self.ended.replace(true) {
            return;
        }
        // The device is let go when the last track that shares it has stopped.
        self.sharing.0.set(self.sharing.0.get().saturating_sub(1));
        if self.sharing.0.get() == 0 {
            if let Some(stream) = get_stream(&self.id) {
                if let Ok(mut stream) = stream.lock() {
                    stream.stop();
                }
            }
        }
    }

    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-getsettings>
    fn GetSettings(&self) -> MediaTrackSettings {
        let measured = get_stream(&self.id)
            .and_then(|stream| stream.lock().ok().and_then(|s| s.video_settings()));
        let (width, height, rate) = measured.unwrap_or(match self.source {
            TrackSource::Screen => (1280, 720, 30.0),
            _ => (640, 480, 30.0),
        });
        let video = self.ty == MediaStreamType::Video;
        MediaTrackSettings {
            deviceId: Some(self.id.id().to_string().into()),
            groupId: Some("".into()),
            width: video.then_some(width as i32),
            height: video.then_some(height as i32),
            aspectRatio: video.then(|| Finite::wrap(width as f64 / height.max(1) as f64)),
            frameRate: video.then(|| Finite::wrap(if rate > 0.0 { rate } else { 30.0 })),
            sampleRate: (!video).then_some(48000),
            channelCount: (!video).then_some(1),
            displaySurface: (self.source == TrackSource::Screen).then(|| "monitor".into()),
        }
    }

    event_handler!(ended, GetOnended, SetOnended);
    event_handler!(mute, GetOnmute, SetOnmute);
    event_handler!(unmute, GetOnunmute, SetOnunmute);
}
