/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

pub mod capture;
pub mod device_monitor;
pub mod registry;

use std::any::Any;

pub use registry::*;

pub trait MediaStream: Any + Send {
    fn as_any(&self) -> &dyn Any;
    fn as_mut_any(&mut self) -> &mut dyn Any;
    fn set_id(&mut self, id: registry::MediaStreamId);
    fn ty(&self) -> MediaStreamType;

    /// Ferrite: release the device (`MediaStreamTrack.stop()`): the camera light
    /// goes out, the microphone and the screen capture end.
    fn stop(&mut self) {}

    /// Ferrite: `MediaStreamTrack.enabled`. A disabled track carries no media
    /// (silence, or no new frames) and keeps the device.
    fn set_enabled(&mut self, _enabled: bool) {}

    /// Ferrite: the size and frame rate the source is delivering, once known
    /// (`MediaStreamTrack.getSettings()`).
    fn video_settings(&self) -> Option<(u32, u32, f64)> {
        None
    }
}

/// A MediaSocket is a way for a backend to represent a
/// yet-to-be-connected source side of a MediaStream
pub trait MediaSocket: Any + Send {
    fn as_any(&self) -> &dyn Any;
}

/// This isn't part of the webrtc spec; it's a leaky abstaction while media streams
/// are under development and example consumers need to be able to inspect them.
pub trait MediaOutput: Send {
    fn add_stream(&mut self, stream: &registry::MediaStreamId);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaStreamType {
    Video,
    Audio,
}
