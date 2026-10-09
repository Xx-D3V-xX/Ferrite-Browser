# Ferrite's changes to servo-pixels 0.6.0

This directory is `servo-pixels` 0.6.0 from crates.io (MPL-2.0, see the headers of
its files) with the changes below. It is used through `[patch.crates-io]` in the
workspace `Cargo.toml`. Drop the patch (and this directory) when the engine is
upgraded to a release that decodes these images itself.

1. `lib.rs`, `load_from_memory` (`with_default_gif_palette`): **a GIF with no colour
   table decodes.** The `gif` crate refuses a frame that has neither a local nor a
   global colour table ("no color table available for current frame"), so the `<img>`
   fired `error` and drew nothing; the CI site check found one on apple.com (T-332), a
   1x1 tracking pixel. A GIF with no global table now gets a two-entry black one before
   decoding, as Firefox uses black for a frame with no table; frames with their own
   table are unaffected, and a transparent index in a frame's graphic control extension
   still applies, so such a pixel is transparent. `tests/pixels.rs` has the pixel (it
   fails without the change) and a GIF with a table, which decodes as before.
