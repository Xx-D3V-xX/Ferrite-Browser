# Ferrite's changes to servo-media-gstreamer 0.6.0

`servo-media-gstreamer` 0.6.0 from crates.io (MPL-2.0) with the changes below, used
through `[patch.crates-io]` together with `vendor/servo-media-streams` (where the
trait methods are declared) and `vendor/servo-embedder-traits`.

1. `media_capture.rs`: **screen capture.** `MediaTrackConstraintSet.screen` asks for
   the whole screen: `ximagesrc` on Linux (X11 only; Wayland would need the portal and
   PipeWire), `avfvideosrc capture-screen=true` on macOS, `d3d11screencapturesrc`,
   then `dx9screencapsrc`, then `gdiscreencapsrc` on Windows, each followed by a 30 fps
   capsfilter.
2. `media_stream.rs`: **`stop`, `set_enabled`, `video_settings`.** `stop` sets the
   pipeline and every element to NULL, which closes the camera, microphone or screen
   source (`MediaStreamTrack.stop()`). Every stream ends in a `valve`; closing it is
   `enabled = false`. `video_settings` reads the negotiated size and frame rate.
3. `media_stream.rs`: **`encoded()` set `vp8enc`'s `error-resilient` (a flags type) from
   a string**, which GStreamer 1.24 refuses (`property 'error-resilient' of type
   'GstVP8Enc' can't be set from the given type`), so attaching a video stream to a
   `<video>` (or a peer connection) panicked the script thread. Both properties are
   set with `property_from_str` now.
