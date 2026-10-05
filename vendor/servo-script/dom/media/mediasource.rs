/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Media Source Extensions: a page builds a media stream out of byte buffers and plays it
//! through a `<video>`. See `ferrite-mse` for the data path (parsing, the frame store) and
//! `servo-media-gstreamer`'s `mse_source.rs` for the player's end of it; this is the DOM.

use std::cell::Cell;
use std::sync::Arc;

use dom_struct::dom_struct;
use ferrite_mse::{Container, Shared};
use js::context::JSContext;
use js::rust::HandleObject;
use script_bindings::cell::DomRefCell;
use script_bindings::reflector::{DomObject, reflect_dom_object_with_proto};
use servo_media::{ServoMedia, SupportsMediaType};
use stylo_atoms::Atom;

use crate::dom::bindings::codegen::Bindings::MediaErrorBinding::MediaErrorConstants::{
    MEDIA_ERR_DECODE, MEDIA_ERR_NETWORK,
};
use crate::dom::bindings::codegen::Bindings::MediaSourceBinding::{
    EndOfStreamError, MediaSourceMethods, MediaSourceReadyState,
};
use crate::dom::bindings::error::{Error, ErrorResult, Fallible};
use crate::dom::bindings::conversions::DerivedFrom;
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot, MutNullableDom};
use crate::dom::bindings::str::DOMString;
use crate::dom::eventtarget::EventTarget;
use crate::dom::html::htmlmediaelement::HTMLMediaElement;
use crate::dom::sourcebuffer::SourceBuffer;
use crate::dom::sourcebufferlist::SourceBufferList;
use crate::dom::window::Window;

/// Queues a task that fires `name` at `target`, on the media element task source: the
/// spec's "queue a task to fire an event" for everything in this module.
pub(crate) fn queue_event<T>(target: &T, name: &'static str)
where
    T: DomObject + Castable + DerivedFrom<EventTarget>,
{
    let trusted = Trusted::new(target);
    target
        .global()
        .task_manager()
        .media_element_task_source()
        .queue(task!(media_source_event: move |cx| {
            trusted
                .root()
                .upcast::<EventTarget>()
                .fire_event(cx, Atom::from(name));
        }));
}

/// Seconds to the nanoseconds the data path counts in.
pub(crate) fn seconds_to_ns(seconds: f64) -> i64 {
    if seconds.is_nan() {
        0
    } else if seconds >= (i64::MAX / 1_000_000_000) as f64 {
        i64::MAX
    } else if seconds <= (i64::MIN / 1_000_000_000) as f64 {
        i64::MIN
    } else {
        (seconds * 1e9) as i64
    }
}

pub(crate) fn ns_to_seconds(ns: i64) -> f64 {
    ns as f64 / 1e9
}

#[derive(Clone, Copy, Debug, JSTraceable, MallocSizeOf, PartialEq)]
pub(crate) enum State {
    Closed,
    Open,
    Ended,
}

/// <https://w3c.github.io/media-source/#idl-def-mediasource>
#[dom_struct]
pub(crate) struct MediaSource {
    eventtarget: EventTarget,
    state: Cell<State>,
    /// Replaced by a new one when the source is detached, so it can be attached again.
    #[ignore_malloc_size_of = "shared with the media player"]
    #[no_trace]
    shared: DomRefCell<Arc<Shared>>,
    /// The number the player is given to find `shared` (`ferrite_mse::lookup`).
    registry_id: Cell<u64>,
    source_buffers: Dom<SourceBufferList>,
    active_source_buffers: Dom<SourceBufferList>,
    element: MutNullableDom<HTMLMediaElement>,
    /// <https://w3c.github.io/media-source/#dom-mediasource-duration>
    duration: Cell<f64>,
    live_seekable: DomRefCell<Option<(f64, f64)>>,
}

impl MediaSource {
    fn new_with_proto(
        cx: &mut JSContext,
        global: &Window,
        proto: Option<HandleObject>,
    ) -> DomRoot<MediaSource> {
        let global = global.upcast::<crate::dom::globalscope::GlobalScope>();
        let shared = Shared::new();
        let registry_id = ferrite_mse::register(&shared);
        // Rooted here until the `MediaSource` that holds them exists.
        let source_buffers = SourceBufferList::new(cx, global);
        let active_source_buffers = SourceBufferList::new(cx, global);
        reflect_dom_object_with_proto(
            cx,
            Box::new(MediaSource {
                eventtarget: EventTarget::new_inherited(),
                state: Cell::new(State::Closed),
                shared: DomRefCell::new(shared),
                registry_id: Cell::new(registry_id),
                source_buffers: Dom::from_ref(&*source_buffers),
                active_source_buffers: Dom::from_ref(&*active_source_buffers),
                element: MutNullableDom::new(None),
                duration: Cell::new(f64::NAN),
                live_seekable: DomRefCell::new(None),
            }),
            global,
            proto,
        )
    }

    pub(crate) fn shared(&self) -> Arc<Shared> {
        self.shared.borrow().clone()
    }

    pub(crate) fn registry_id(&self) -> u64 {
        self.registry_id.get()
    }

    pub(crate) fn state(&self) -> State {
        self.state.get()
    }

    pub(crate) fn element(&self) -> Option<DomRoot<HTMLMediaElement>> {
        self.element.get()
    }

    pub(crate) fn duration_seconds(&self) -> f64 {
        self.duration.get()
    }

    /// What the media element reports as `buffered`, in seconds.
    pub(crate) fn buffered_seconds(&self) -> Vec<(f64, f64)> {
        self.shared()
            .buffered_all()
            .into_iter()
            .map(|(start, end)| (ns_to_seconds(start), ns_to_seconds(end)))
            .collect()
    }

    /// The size of the first video track an initialization segment described.
    pub(crate) fn video_size(&self) -> Option<(u32, u32)> {
        (0..self.shared().slot_count())
            .filter_map(|slot| self.shared().track_info(slot))
            .find(|track| track.kind == ferrite_mse::TrackKind::Video && track.width > 0)
            .map(|track| (track.width, track.height))
    }

    pub(crate) fn live_seekable(&self) -> Option<(f64, f64)> {
        *self.live_seekable.borrow()
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.state.get() == State::Ended
    }

    fn any_updating(&self) -> bool {
        self.source_buffers
            .snapshot()
            .iter()
            .any(|buffer| buffer.updating())
    }

    /// <https://w3c.github.io/media-source/#mediasource-attach>: the media element
    /// starts using this `MediaSource`. False if it is already in use.
    pub(crate) fn attach(&self, element: &HTMLMediaElement) -> bool {
        if self.state.get() != State::Closed || self.element.get().is_some() {
            return false;
        }
        self.element.set(Some(element));
        self.state.set(State::Open);
        queue_event(self, "sourceopen");
        true
    }

    /// <https://w3c.github.io/media-source/#mediasource-detach>
    pub(crate) fn detach(&self) {
        if self.state.get() == State::Closed {
            return;
        }
        self.state.set(State::Closed);
        self.duration.set(f64::NAN);
        *self.live_seekable.borrow_mut() = None;
        for buffer in self.source_buffers.snapshot() {
            buffer.mark_removed();
            for slot in buffer.slots() {
                self.shared().retire(slot);
            }
        }
        for buffer in self.active_source_buffers.snapshot() {
            self.active_source_buffers.remove(&buffer);
        }
        let removed: Vec<_> = self.source_buffers.snapshot();
        for buffer in &removed {
            self.source_buffers.remove(buffer);
        }
        if !removed.is_empty() {
            queue_event(&*self.active_source_buffers, "removesourcebuffer");
            queue_event(&*self.source_buffers, "removesourcebuffer");
        }
        // The player's feeders stop with the old state; a new attach starts afresh.
        let fresh = Shared::new();
        let old = std::mem::replace(&mut *self.shared.borrow_mut(), fresh.clone());
        old.close();
        ferrite_mse::unregister(self.registry_id.get());
        self.registry_id.set(ferrite_mse::register(&fresh));
        self.element.set(None);
        queue_event(self, "sourceclose");
    }

    /// The page appended to a buffer, or something else changed what is buffered.
    pub(crate) fn buffers_changed(&self) {
        if let Some(element) = self.element.get() {
            element.media_source_changed();
        }
    }

    /// A `SourceBuffer` got its first initialization segment.
    pub(crate) fn buffer_initialized(&self, buffer: &SourceBuffer, duration: Option<i64>) {
        if !self.active_source_buffers.contains(buffer) {
            self.active_source_buffers.add(buffer);
            queue_event(&*self.active_source_buffers, "addsourcebuffer");
        }
        // "If the duration is NaN: the initialization segment's duration, else infinity."
        if self.duration.get().is_nan() {
            let seconds = duration.map_or(f64::INFINITY, ns_to_seconds);
            self.set_duration_value(seconds);
        }
        self.buffers_changed();
    }

    /// Changes `duration` and tells the element and the player.
    fn set_duration_value(&self, seconds: f64) {
        self.duration.set(seconds);
        self.shared()
            .set_duration(seconds.is_finite().then(|| seconds_to_ns(seconds)));
        if let Some(element) = self.element.get() {
            element.media_source_duration_changed(seconds);
        }
    }

    /// Frames reached `end_ns`: a duration below that grows to it.
    pub(crate) fn frames_reach(&self, end_ns: i64) {
        let seconds = ns_to_seconds(end_ns);
        let duration = self.duration.get();
        if duration.is_finite() && seconds > duration {
            self.set_duration_value(seconds);
        }
    }

    /// An ended stream gets more data, or the element seeks: back to "open".
    pub(crate) fn reopen_if_ended(&self) {
        if self.state.get() == State::Ended {
            self.state.set(State::Open);
            self.shared().set_ended(false);
            queue_event(self, "sourceopen");
        }
    }

    /// <https://w3c.github.io/media-source/#end-of-stream-algorithm>
    fn end_of_stream_algorithm(&self, error: Option<EndOfStreamError>) {
        self.state.set(State::Ended);
        queue_event(self, "sourceended");
        match error {
            None => {
                // The duration becomes the end of what is buffered: the highest end of any
                // track, so a track that stops short does not cut the other one off.
                let shared = self.shared();
                let highest = (0..shared.slot_count())
                    .filter_map(|slot| shared.buffered(slot).last().map(|r| r.1))
                    .max();
                if let Some(end) = highest {
                    let seconds = ns_to_seconds(end);
                    if seconds != self.duration.get() {
                        self.set_duration_value(seconds);
                    }
                }
                self.shared().set_ended(true);
                self.buffers_changed();
            },
            Some(error) => {
                self.shared().set_ended(true);
                if let Some(element) = self.element.get() {
                    let code = match error {
                        EndOfStreamError::Network => MEDIA_ERR_NETWORK,
                        EndOfStreamError::Decode => MEDIA_ERR_DECODE,
                    };
                    element.media_source_error(code);
                }
            },
        }
    }

    /// A buffer could not parse what the page appended.
    pub(crate) fn decode_error(&self) {
        if self.state.get() == State::Open {
            self.end_of_stream_algorithm(Some(EndOfStreamError::Decode));
        }
    }

    pub(crate) fn supports_type(mime: &str) -> bool {
        ferrite_mse::is_type_supported(mime) &&
            ServoMedia::get().can_play_type(mime) == SupportsMediaType::Probably
    }
}

impl MediaSourceMethods<crate::DomTypeHolder> for MediaSource {
    /// <https://w3c.github.io/media-source/#dom-mediasource-constructor>
    fn Constructor(
        cx: &mut JSContext,
        global: &Window,
        proto: Option<HandleObject>,
    ) -> Fallible<DomRoot<MediaSource>> {
        Ok(MediaSource::new_with_proto(cx, global, proto))
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-sourcebuffers>
    fn SourceBuffers(&self) -> DomRoot<SourceBufferList> {
        DomRoot::from_ref(&*self.source_buffers)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-activesourcebuffers>
    fn ActiveSourceBuffers(&self) -> DomRoot<SourceBufferList> {
        DomRoot::from_ref(&*self.active_source_buffers)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-readystate>
    fn ReadyState(&self) -> MediaSourceReadyState {
        match self.state.get() {
            State::Closed => MediaSourceReadyState::Closed,
            State::Open => MediaSourceReadyState::Open,
            State::Ended => MediaSourceReadyState::Ended,
        }
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-duration>
    fn Duration(&self) -> f64 {
        if self.state.get() == State::Closed {
            f64::NAN
        } else {
            self.duration.get()
        }
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-duration>
    fn SetDuration(&self, value: f64) -> ErrorResult {
        if value.is_nan() || value < 0.0 {
            return Err(Error::Type(c"The duration must be a number, not negative.".to_owned()));
        }
        if self.state.get() != State::Open || self.any_updating() {
            return Err(Error::InvalidState(None));
        }
        // The duration may not cut off what is buffered.
        let highest = (0..self.shared().slot_count())
            .filter_map(|slot| self.shared().buffered(slot).last().map(|r| r.1))
            .max();
        if highest.is_some_and(|end| ns_to_seconds(end) > value) {
            return Err(Error::InvalidState(Some(
                "The duration is below the end of the buffered data.".to_owned(),
            )));
        }
        if value != self.duration.get() {
            self.set_duration_value(value);
        }
        Ok(())
    }

    event_handler!(sourceopen, GetOnsourceopen, SetOnsourceopen);
    event_handler!(sourceended, GetOnsourceended, SetOnsourceended);
    event_handler!(sourceclose, GetOnsourceclose, SetOnsourceclose);

    /// <https://w3c.github.io/media-source/#dom-mediasource-addsourcebuffer>
    fn AddSourceBuffer(&self, cx: &mut JSContext, type_: DOMString) -> Fallible<DomRoot<SourceBuffer>> {
        let mime = type_.str().to_string();
        if mime.is_empty() {
            return Err(Error::Type(c"The type must not be empty.".to_owned()));
        }
        let Some(container) = Container::from_mime(&mime) else {
            return Err(Error::NotSupported(None));
        };
        if !MediaSource::supports_type(&mime) {
            return Err(Error::NotSupported(None));
        }
        if self.state.get() != State::Open {
            return Err(Error::InvalidState(None));
        }
        let buffer = SourceBuffer::new(cx, &self.global(), self, mime, container);
        self.shared().register_buffer();
        self.source_buffers.add(&buffer);
        queue_event(&*self.source_buffers, "addsourcebuffer");
        Ok(buffer)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-removesourcebuffer>
    fn RemoveSourceBuffer(&self, buffer: &SourceBuffer) -> ErrorResult {
        if !self.source_buffers.contains(buffer) {
            return Err(Error::NotFound(None));
        }
        buffer.abort_for_removal();
        let was_active = self.active_source_buffers.remove(buffer);
        self.source_buffers.remove(buffer);
        buffer.mark_removed();
        for slot in buffer.slots() {
            self.shared().retire(slot);
        }
        self.shared().unregister_buffer(buffer.initialized());
        if was_active {
            queue_event(&*self.active_source_buffers, "removesourcebuffer");
        }
        queue_event(&*self.source_buffers, "removesourcebuffer");
        self.buffers_changed();
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-endofstream>
    fn EndOfStream(&self, error: Option<EndOfStreamError>) -> ErrorResult {
        if self.state.get() != State::Open || self.any_updating() {
            return Err(Error::InvalidState(None));
        }
        self.end_of_stream_algorithm(error);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-setliveseekablerange>
    fn SetLiveSeekableRange(&self, start: Finite<f64>, end: Finite<f64>) -> ErrorResult {
        let (start, end) = (*start, *end);
        if self.state.get() != State::Open {
            return Err(Error::InvalidState(None));
        }
        if start < 0.0 || start > end {
            return Err(Error::Type(c"The range is not valid.".to_owned()));
        }
        *self.live_seekable.borrow_mut() = Some((start, end));
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-clearliveseekablerange>
    fn ClearLiveSeekableRange(&self) -> ErrorResult {
        if self.state.get() != State::Open {
            return Err(Error::InvalidState(None));
        }
        *self.live_seekable.borrow_mut() = None;
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-istypesupported>
    fn IsTypeSupported(_: &Window, type_: DOMString) -> bool {
        MediaSource::supports_type(&type_.str())
    }
}
