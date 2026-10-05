# Ferrite's changes to servo-media-player 0.6.0

`servo-media-player` 0.6.0 from crates.io (MPL-2.0) with the change below, used through
`[patch.crates-io]`. See `vendor/servo-media-gstreamer/FERRITE-PATCHES.md`.

1. `lib.rs`: `StreamType::MediaSource(u64)`, for a player fed by a page through Media
   Source Extensions instead of by a URL or a stream. The number names the
   `MediaSource`'s shared state in `ferrite-mse`.
