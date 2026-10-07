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
5. `datachannel.rs`: **creating a data channel no longer panics.** It read the value of
   webrtcbin's `create-data-channel` signal with `emit_by_name::<WebRTCDataChannel>`,
   which panics if the value is not of the expected type. On a Mac where a second GLib
   had been loaded (Homebrew's, through GIO modules) that happened, the WebRTC thread died,
   and the script thread then panicked on its closed channel (`servo-media-webrtc`
   `thread.rs`). Now the failure is returned and the page's `createDataChannel` fails.
6. `mse_source.rs` (`make_pad`): **a stream starts only once its pad is linked.** The
   source bin can be going to PAUSED on another thread while its pads are made; that
   change started a new `appsrc` as soon as it was added to the bin, before `add_pad` had
   linked it, so its first push failed with `not-linked` and the player stopped. Under
   load, `mse_probe` hit it about one run in forty (a lost `seeked`, a lost `ended`, and
   once a `playbin3` assertion that ended the process). The `appsrc`'s state is locked
   until its ghost pad is added, then synced with the bin.
7. `player.rs`: **`not-linked` is reported as `PlayerEvent::StreamLost`**, not as an error
   (see `vendor/servo-script/FERRITE-PATCHES.md` item 15).
8. `mse_source.rs` (`make_pad`, `seek_data`): **a replaced player's seeks are ignored.**
   When the element replaces its player (a seek after the end, or a lost stream), the old
   pipeline stops asynchronously (`Play::stop` runs on GstPlay's thread) and could still
   report a seek of its own, at its end, after the new player's start was set; that
   replaced the start, the new player reported the old end as its position, and the
   element never reached `playing` (T-339, about one loaded `mse_probe` run in thirty).
   Each pipeline takes the `MediaSource`'s current run (`ferrite_mse::Shared::run`) when
   it builds its streams and reports its seeks with it; `Shared::seek_all` (a new player)
   starts a new run, and a seek from an older one is dropped.
