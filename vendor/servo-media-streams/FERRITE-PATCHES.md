# Ferrite's changes to servo-media-streams 0.6.0

`servo-media-streams` 0.6.0 from crates.io (MPL-2.0) with the changes below, used
through `[patch.crates-io]`. See `vendor/servo-media-gstreamer/FERRITE-PATCHES.md`.

1. `capture.rs`: `MediaTrackConstraintSet.screen`.
2. `lib.rs`: `MediaStream::stop`, `set_enabled` and `video_settings`, with do-nothing
   defaults so a backend that has no use for them (the dummy one) needs no change.
