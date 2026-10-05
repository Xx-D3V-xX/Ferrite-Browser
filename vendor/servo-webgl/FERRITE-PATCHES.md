# Ferrite's changes to servo-webgl 0.6.0

This directory is `servo-webgl` 0.6.0 from crates.io (MPL-2.0, see the headers of
its files) with the changes below. It is used through `[patch.crates-io]` in the
workspace `Cargo.toml`. Drop the patch (and this directory) when the engine is
upgraded to a release that includes the upstream fix (servo/servo#48620).

1. `webgl_thread.rs`, `handle_swap_buffers`: drain pending GL errors before the
   swap instead of `debug_assert_eq!(get_error(), NO_ERROR)`, which compiles away
   in release builds. This is the upstream fix, servo/servo#48620 (issue #48550,
   "Panic when swapping buffers with webgl2"): a stale error stayed pending until
   the swap, where surfman failed to create the next surface on macOS.
2. The same function: a failed `swap_buffers` is logged and skipped instead of
   `unwrap()`, and a failed `clear_surface` is logged. On the owner's Apple M1
   the unwrap panicked the WebGL thread with `SurfaceCreationFailed(Failed)`
   (`webgl_thread.rs:879`), and the next WebGL call panicked the page's script
   thread (see `vendor/servo-script/FERRITE-PATCHES.md` item 2).

Not ported: the other half of upstream's fix, which maps `BACK` to
`COLOR_ATTACHMENT0` for `drawBuffers` and `readBuffer` on the default
framebuffer. Ferrite leaves WebGL 2 off by default instead (`FERRITE_WEBGL`),
since those two calls are WebGL 2 only.
