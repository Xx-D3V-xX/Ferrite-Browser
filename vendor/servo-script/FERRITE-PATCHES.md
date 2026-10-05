# Ferrite's changes to servo-script 0.6.0

This directory is `servo-script` 0.6.0 from crates.io (MPL-2.0, see the
headers of its files) with the changes below. It is used through
`[patch.crates-io]` in the workspace `Cargo.toml`. Drop the patch (and this
directory) when the engine is upgraded to a release that includes the fixes.

1. `dom/window/location.rs`, `GetAncestorOrigins`: return an empty list when
   the document has none, instead of `expect("Must always have ancestor origins
   initialized")`. A page that read `location.ancestorOrigins` in a document the
   parser had not created panicked the script thread and the page stopped
   responding (seen on GitHub pages).
2. `dom/webgl/webglrenderingcontext.rs`: a WebGL thread that has died no longer
   panics the page's script thread. Creating a context returns an error
   (`getContext` then returns `null` and the page falls back) instead of calling
   `unwrap` on the channel, and `send_with_fallibility` logs a warning instead of
   `expect("Operation failed")`. Seen on macOS: the WebGL thread panicked with
   `SurfaceCreationFailed(Failed)`, the next WebGL call panicked the script thread
   with `Disconnected`, and the page (GitHub, Google) stopped responding. About 24
   calls that wait for an answer (`receiver.recv().unwrap()`, for example reading
   pixels back) are not changed; a page that uses them after the thread has died
   can still panic.
