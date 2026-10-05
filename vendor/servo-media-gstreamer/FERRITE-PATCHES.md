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
4. `mse_source.rs`, `player.rs`, `lib.rs`, `Cargo.toml`: **Media Source Extensions.**
   `StreamType::MediaSource(id)` (declared in `vendor/servo-media-player`) makes the
   player ask playbin3 for `servomse://<id>`, which is `servomsesrc`, a source bin that
   reads the frames `ferrite-mse` holds for that `MediaSource`. It makes one `appsrc` pad
   per track once every track has a frame, with one group id and a first segment that
   begins where the data begins; follows seeks (a frame fetched before a seek is never
   pushed after it); sends new caps when an initialization segment changes a track's
   configuration; and posts `Buffering(0)` when the data ends within 80 ms of the
   playhead and `Buffering(100)` once it is 250 ms ahead again, so a stalled player
   pauses. The crate depends on `crates/ferrite-mse` by path.
