/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `SourceBuffer`: bytes in, frames out. The bytes a page appends go through
//! `ferrite_mse::Parser` and land in the `MediaSource`'s shared frame store.

use std::cell::Cell;

use dom_struct::dom_struct;
use ferrite_mse::{Container, Event, Parser, Sample, TrackInfo};
use js::context::JSContext;
use script_bindings::cell::DomRefCell;
use script_bindings::reflector::reflect_dom_object_with_cx;
use stylo_atoms::Atom;

use crate::dom::bindings::buffer_source::get_buffer_source_copy;
use crate::dom::bindings::codegen::Bindings::SourceBufferBinding::{AppendMode, SourceBufferMethods};
use crate::dom::bindings::codegen::UnionTypes::ArrayBufferViewOrArrayBuffer;
use crate::dom::bindings::error::{Error, ErrorResult, Fallible};
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot};
use crate::dom::bindings::str::DOMString;
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;
use crate::dom::mediasource::{MediaSource, State, ns_to_seconds, queue_event, seconds_to_ns};
use crate::dom::timeranges::{TimeRanges, TimeRangesContainer};

/// <https://w3c.github.io/media-source/#idl-def-sourcebuffer>
#[dom_struct]
pub(crate) struct SourceBuffer {
    eventtarget: EventTarget,
    media_source: Dom<MediaSource>,
    #[ignore_malloc_size_of = "defined in ferrite-mse"]
    #[no_trace]
    parser: DomRefCell<Parser>,
    #[ignore_malloc_size_of = "a plain enum"]
    #[no_trace]
    container: Cell<Container>,
    /// The track numbers the container uses, and the slots the tracks have in the
    /// `MediaSource`'s shared store.
    slots: DomRefCell<Vec<(u32, usize)>>,
    /// Slots whose next frame must be a sync sample (after a frame was dropped).
    need_sync: DomRefCell<Vec<usize>>,
    sequence_mode: Cell<bool>,
    updating: Cell<bool>,
    initialized: Cell<bool>,
    removed: Cell<bool>,
    timestamp_offset: Cell<f64>,
    append_window_start: Cell<f64>,
    append_window_end: Cell<f64>,
    /// Bumped by `abort()` so a task queued by an append that was aborted does nothing.
    generation: Cell<u32>,
    /// In sequence mode, the next frames start a group and fix the offset.
    need_group_start: Cell<bool>,
    group_end: Cell<Option<i64>>,
}

impl SourceBuffer {
    pub(crate) fn new(
        cx: &mut JSContext,
        global: &GlobalScope,
        media_source: &MediaSource,
        _mime: String,
        container: Container,
    ) -> DomRoot<SourceBuffer> {
        reflect_dom_object_with_cx(
            Box::new(SourceBuffer {
                eventtarget: EventTarget::new_inherited(),
                media_source: Dom::from_ref(media_source),
                parser: DomRefCell::new(Parser::new(container)),
                container: Cell::new(container),
                slots: DomRefCell::new(Vec::new()),
                need_sync: DomRefCell::new(Vec::new()),
                sequence_mode: Cell::new(false),
                updating: Cell::new(false),
                initialized: Cell::new(false),
                removed: Cell::new(false),
                timestamp_offset: Cell::new(0.0),
                append_window_start: Cell::new(0.0),
                append_window_end: Cell::new(f64::INFINITY),
                generation: Cell::new(0),
                need_group_start: Cell::new(true),
                group_end: Cell::new(None),
            }),
            global,
            cx,
        )
    }

    pub(crate) fn updating(&self) -> bool {
        self.updating.get()
    }

    pub(crate) fn initialized(&self) -> bool {
        self.initialized.get()
    }

    pub(crate) fn slots(&self) -> Vec<usize> {
        self.slots.borrow().iter().map(|&(_, slot)| slot).collect()
    }

    pub(crate) fn mark_removed(&self) {
        self.removed.set(true);
    }

    /// `removeSourceBuffer` on a buffer in the middle of an append: the append ends.
    pub(crate) fn abort_for_removal(&self) {
        if self.updating.replace(false) {
            self.generation.set(self.generation.get().wrapping_add(1));
            queue_event(self, "abort");
            queue_event(self, "updateend");
        }
        self.parser.borrow_mut().reset();
    }

    fn fire(&self, cx: &mut JSContext, name: &'static str) {
        self.upcast::<EventTarget>()
            .fire_event(cx, Atom::from(name));
    }

    /// The checks every method that changes the buffer starts with.
    fn check_usable(&self) -> ErrorResult {
        if self.removed.get() || self.updating.get() {
            return Err(Error::InvalidState(None));
        }
        Ok(())
    }

    /// Starts an operation that takes a task: `updating` is true until
    /// `finish_update` runs.
    fn start_update(&self) -> u32 {
        self.updating.set(true);
        self.generation.get()
    }

    /// <https://w3c.github.io/media-source/#sourcebuffer-append-error>
    fn append_error(&self, cx: &mut JSContext) {
        self.parser.borrow_mut().reset();
        self.updating.set(false);
        self.fire(cx, "error");
        self.fire(cx, "updateend");
        self.media_source.decode_error();
    }

    fn finish_update(&self, cx: &mut JSContext) {
        self.updating.set(false);
        self.media_source.buffers_changed();
        self.fire(cx, "update");
        self.fire(cx, "updateend");
    }

    /// <https://w3c.github.io/media-source/#sourcebuffer-segment-parser-loop>
    fn run_append(&self, cx: &mut JSContext, bytes: &[u8]) {
        // The borrow ends here: the error path resets the parser.
        let parsed = self.parser.borrow_mut().append(bytes);
        let events = match parsed {
            Ok(events) => events,
            Err(error) => {
                warn!("SourceBuffer append failed: {error}");
                self.append_error(cx);
                return;
            },
        };
        for event in events {
            let ok = match event {
                Event::Init { tracks, duration } => {
                    self.initialization_segment(tracks, duration);
                    true
                },
                Event::Samples { track, samples } => self.coded_frames(track, samples),
            };
            if !ok {
                self.append_error(cx);
                return;
            }
        }
        self.finish_update(cx);
    }

    /// <https://w3c.github.io/media-source/#sourcebuffer-init-segment-received>
    fn initialization_segment(&self, tracks: Vec<TrackInfo>, duration: Option<i64>) {
        let shared = self.media_source.shared();
        {
            let mut slots = self.slots.borrow_mut();
            for track in tracks {
                let id = track.id;
                let slot = shared.add_track(track);
                match slots.iter_mut().find(|(known, _)| *known == id) {
                    Some(entry) => entry.1 = slot,
                    None => slots.push((id, slot)),
                }
                let mut need_sync = self.need_sync.borrow_mut();
                if !need_sync.contains(&slot) {
                    need_sync.push(slot);
                }
            }
        }
        if !self.initialized.replace(true) {
            shared.buffer_initialized();
            self.media_source.buffer_initialized(self, duration);
        }
    }

    /// <https://w3c.github.io/media-source/#sourcebuffer-coded-frame-processing>
    /// Returns false if the frames could not be kept.
    fn coded_frames(&self, track: u32, mut samples: Vec<Sample>) -> bool {
        let Some(slot) = self
            .slots
            .borrow()
            .iter()
            .find(|&&(id, _)| id == track)
            .map(|&(_, slot)| slot)
        else {
            return true;
        };
        if samples.is_empty() {
            return true;
        }
        // Sequence mode: the first frames of a group are placed where the last group
        // ended, whatever times they carry.
        if self.sequence_mode.get() && self.need_group_start.get() {
            let first = samples.iter().map(|s| s.pts).min().unwrap_or(0);
            let end = self.group_end.get().unwrap_or(0);
            self.timestamp_offset.set(ns_to_seconds(end - first));
            self.need_group_start.set(false);
        }
        let offset = seconds_to_ns(self.timestamp_offset.get());
        let window_start = seconds_to_ns(self.append_window_start.get());
        let window_end = seconds_to_ns(self.append_window_end.get());
        let mut kept = Vec::with_capacity(samples.len());
        {
            let mut need_sync = self.need_sync.borrow_mut();
            for mut sample in samples.drain(..) {
                sample.pts = sample.pts.saturating_add(offset);
                sample.dts = sample.dts.saturating_add(offset);
                let outside = sample.pts < window_start || sample.end() > window_end;
                if outside {
                    if !need_sync.contains(&slot) {
                        need_sync.push(slot);
                    }
                    continue;
                }
                if need_sync.contains(&slot) {
                    if !sample.key {
                        continue;
                    }
                    need_sync.retain(|s| *s != slot);
                }
                kept.push(sample);
            }
        }
        let Some(end) = kept.iter().map(Sample::end).max() else {
            return true;
        };
        if self.media_source.shared().append(slot, kept).is_err() {
            return false;
        }
        self.group_end
            .set(Some(self.group_end.get().map_or(end, |g| g.max(end))));
        self.media_source.frames_reach(end);
        true
    }
}

impl SourceBufferMethods<crate::DomTypeHolder> for SourceBuffer {
    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-mode>
    fn Mode(&self) -> AppendMode {
        if self.sequence_mode.get() {
            AppendMode::Sequence
        } else {
            AppendMode::Segments
        }
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-mode>
    fn SetMode(&self, mode: AppendMode) -> ErrorResult {
        self.check_usable()?;
        self.media_source.reopen_if_ended();
        let sequence = mode == AppendMode::Sequence;
        if sequence && !self.sequence_mode.get() {
            // The group starts where what is buffered ends.
            self.need_group_start.set(true);
        }
        self.sequence_mode.set(sequence);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-updating>
    fn Updating(&self) -> bool {
        self.updating.get()
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-buffered>
    fn GetBuffered(&self, cx: &mut JSContext) -> Fallible<DomRoot<TimeRanges>> {
        if self.removed.get() {
            return Err(Error::InvalidState(None));
        }
        let mut container = TimeRangesContainer::default();
        let slots = self.slots();
        for (start, end) in self.media_source.shared().buffered_of(&slots) {
            let _ = container.add(ns_to_seconds(start), ns_to_seconds(end));
        }
        Ok(TimeRanges::new(cx, self.global().as_window(), container))
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-timestampoffset>
    fn TimestampOffset(&self) -> Finite<f64> {
        Finite::wrap(self.timestamp_offset.get())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-timestampoffset>
    fn SetTimestampOffset(&self, value: Finite<f64>) -> ErrorResult {
        self.check_usable()?;
        self.media_source.reopen_if_ended();
        self.timestamp_offset.set(*value);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowstart>
    fn AppendWindowStart(&self) -> Finite<f64> {
        Finite::wrap(self.append_window_start.get())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowstart>
    fn SetAppendWindowStart(&self, value: Finite<f64>) -> ErrorResult {
        self.check_usable()?;
        let value = *value;
        if value < 0.0 || value >= self.append_window_end.get() {
            return Err(Error::Type(c"The append window start is not valid.".to_owned()));
        }
        self.append_window_start.set(value);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowend>
    fn AppendWindowEnd(&self) -> f64 {
        self.append_window_end.get()
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowend>
    fn SetAppendWindowEnd(&self, value: f64) -> ErrorResult {
        self.check_usable()?;
        if value.is_nan() || value <= self.append_window_start.get() {
            return Err(Error::Type(c"The append window end is not valid.".to_owned()));
        }
        self.append_window_end.set(value);
        Ok(())
    }

    event_handler!(updatestart, GetOnupdatestart, SetOnupdatestart);
    event_handler!(update, GetOnupdate, SetOnupdate);
    event_handler!(updateend, GetOnupdateend, SetOnupdateend);
    event_handler!(error, GetOnerror, SetOnerror);
    event_handler!(abort, GetOnabort, SetOnabort);

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendbuffer>
    fn AppendBuffer(&self, data: ArrayBufferViewOrArrayBuffer) -> ErrorResult {
        self.check_usable()?;
        let bytes = get_buffer_source_copy((&data).into());
        self.media_source.reopen_if_ended();
        // Room first, as `QuotaExceededError` is thrown here, not reported later.
        if self.media_source.shared().reserve(bytes.len()).is_err() {
            return Err(Error::QuotaExceeded {
                quota: None,
                requested: None,
            });
        }
        let generation = self.start_update();
        let this = Trusted::new(self);
        self.global()
            .task_manager()
            .media_element_task_source()
            .queue(task!(source_buffer_append: move |cx| {
                let this = this.root();
                if generation != this.generation.get() {
                    return;
                }
                this.fire(cx, "updatestart");
                // A handler may have aborted the append.
                if generation != this.generation.get() {
                    return;
                }
                this.run_append(cx, &bytes);
            }));
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-abort>
    fn Abort(&self) -> ErrorResult {
        if self.removed.get() || self.media_source.state() != State::Open {
            return Err(Error::InvalidState(None));
        }
        if self.updating.replace(false) {
            self.generation.set(self.generation.get().wrapping_add(1));
            queue_event(self, "abort");
            queue_event(self, "updateend");
        }
        self.parser.borrow_mut().reset();
        self.append_window_start.set(0.0);
        self.append_window_end.set(f64::INFINITY);
        self.need_group_start.set(true);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-changetype>
    fn ChangeType(&self, type_: DOMString) -> ErrorResult {
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
        self.check_usable()?;
        self.media_source.reopen_if_ended();
        self.container.set(container);
        *self.parser.borrow_mut() = Parser::new(container);
        self.need_group_start.set(true);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-remove>
    fn Remove(&self, start: Finite<f64>, end: f64) -> ErrorResult {
        let start = *start;
        if self.removed.get() || self.updating.get() {
            return Err(Error::InvalidState(None));
        }
        let duration = self.media_source.duration_seconds();
        if duration.is_nan() || start < 0.0 || start > duration {
            return Err(Error::Type(c"The range to remove is not valid.".to_owned()));
        }
        if end.is_nan() || end <= start {
            return Err(Error::Type(c"The range to remove is not valid.".to_owned()));
        }
        self.media_source.reopen_if_ended();
        let generation = self.start_update();
        let this = Trusted::new(self);
        self.global()
            .task_manager()
            .media_element_task_source()
            .queue(task!(source_buffer_remove: move |cx| {
                let this = this.root();
                if generation != this.generation.get() {
                    return;
                }
                this.fire(cx, "updatestart");
                if generation != this.generation.get() {
                    return;
                }
                let (start, end) = (seconds_to_ns(start), seconds_to_ns(end));
                let shared = this.media_source.shared().clone();
                for slot in this.slots() {
                    shared.remove(slot, start, end);
                }
                this.finish_update(cx);
            }));
        Ok(())
    }
}
