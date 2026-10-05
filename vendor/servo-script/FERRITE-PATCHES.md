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
3. `dom/document/document.rs`, `dom/window/window.rs`: the document's visibility
   state follows the embedder's throttle state. The engine set `page_showing`
   when a page finished loading but never made the document `visible`, so every
   ordinary page reported `document.visibilityState == "hidden"` and
   `document.hidden == true`, and the wake-lock API refused it ("The requesting
   page is not visible", seen in the owner's Speedometer log). Now a page that is
   showing and not throttled is `visible` (set at the end of the load, and again
   when `set_throttled` changes), and a throttled one is `hidden`, each with the
   `visibilitychange` event. Ferrite's session throttles the tabs that are not the
   active one.
4. `dom/globalscope/globalscope.rs`, `evaluate_js_on_global`: return
   `JavaScriptEvaluationError::WebViewNotReady` when the document cannot run script
   (not fully active, or sandboxed by `Content-Security-Policy: sandbox`) instead
   of `assert!(self.can_run_script())`. This is upstream issue servo/servo#47331.
   The assertion panicked the page's script thread, and the embedder's callback
   never ran. Embedder JavaScript, user scripts (Ferrite's SVG and compatibility
   scripts) and the debugger all go through this function. Seen on YouTube with
   Ferrite's script-thread probe (`ScriptWatch`), which evaluates `0` every 2 s.
5. `dom/document/document.rs`, `gather_active_resize_observations_at_depth`: copy
   the list of resize observers first, then let go of the borrow before asking each
   observer to measure. The old code kept `resize_observers` mutably borrowed while
   measuring, and measuring runs a reflow. The reflow can copy an inline SVG, which
   allocates, which can start a garbage collection, which reads the same field in
   `Document::trace` and panics with "already mutably borrowed". The sibling
   function `broadcast_active_resize_observations` already copies the list; this
   makes the two match. Seen on YouTube (script-thread panic, page stops
   responding).
