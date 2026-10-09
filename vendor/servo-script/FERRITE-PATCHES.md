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
6. `dom/indexeddb/` (with `vendor/servo-script-bindings`, which switches the
   interface methods on): indexes and cursors.
   - **Index queries.** `IDBIndex.get`, `getKey`, `getAll`, `getAllKeys`, `count`,
     `openCursor` and `openKeyCursor`. The storage backend keeps no index data (its
     index tables are created and never written), so an index is computed: the
     backend sends the store's records (the existing `Iterate` operation), each
     record's value is read and the index's key path evaluated on it
     (`key.rs`, `extract_index_keys`, including multiEntry arrays, which
     `extract_key` marked `unimplemented!`), and the records are put in index order
     (key, then primary key) (`idbindex.rs`, `index_records`). A record whose value
     gives no valid key is not in the index. The answer for the non-cursor queries
     is built in `idbrequest.rs` (`index_answer`).
   - **Cursor stepping.** `IDBCursor.advance`, `continue`, `continuePrimaryKey`,
     `update` and `delete` (`idbcursor.rs`). Moving a cursor runs "iterate a
     cursor" again on the cursor's own request, which becomes pending again
     (`IDBRequest::set_ready_state_pending`,
     `IDBTransaction::mark_request_pending_again`) and gets its next record as
     another `success` event. `update` stores under the cursor's effective key (and
     checks it against the in-line key path); `delete` removes that record.
   - **Fixes found on the way.** `iterate_cursor` kept the object store position in
     the cursor and then overwrote it with the stale starting value in step 11, so
     an index cursor could never get past its first record, and `advance(n)` with n
     above 1 repeated the same position. A cursor that ran off the end left its
     request's result `undefined`; the specification says `null`.
     `IDBRequest.source` can now be an index or a cursor (`RequestSource`).
   - **Compound key paths.** `evaluate_key_path_on_value` built the result of a key
     path list (`keyPath: ['a', 'b']`) with `JS_NewObject`, a plain object, which is
     not a valid key, so every `put` into a store (or index) with a compound key
     path failed with `DataError: Provided data is inadequate.` It is an Array now.
     Found by the Cache API stand-in (`crates/ferrite-servo/src/storage_compat.js`),
     which keys its entries on `['cache', 'url', 'method']`.
   - **Unique indexes.** `IDBObjectStore::put`, `add` and `IDBCursor::update` send the
     keys the record has in each unique index (`unique_index_keys`, `idbobjectstore.rs`)
     and the storage backend refuses a duplicate (`vendor/servo-storage`), which the
     request reports as `ConstraintError`.
   - **Not done.** A unique index made after records exist is not checked against them
     (see `vendor/servo-storage/FERRITE-PATCHES.md`), and `getAllRecords`.
7. `dom/bindings/structuredclone.rs`, `dom/workers/dedicatedworkerglobalscope.rs` (with
   `servo-script-bindings`, item 5): **`SharedArrayBuffer` can be shared with a worker.**
   The engine hard-wired `SharedArrayBuffer`/`Atomics` off, and its structured clone
   refused one (`sab_cloned_callback` returned false). Now `write_message` (used by
   `postMessage` to a worker, a window, a port, a broadcast channel) writes normally and,
   when SpiderMonkey refuses (a `SharedArrayBuffer`, a shared `WebAssembly.Memory`), writes
   again with the same-process scope and the policy that allows shared memory; the bytes
   carry a four-byte mark (`FSAB`) and `read` uses the same scope when it sees it. The
   clone buffer of such a message is leaked on purpose (it holds the reference that keeps
   the shared buffer alive until the receiver has read it); messages that share nothing are
   unchanged. What is stored (IndexedDB, history state) never takes this path. A worker
   may now call `Atomics.wait` (`JS_SetFutexCanWait`). **Not done:** a `WebAssembly.Module`
   still cannot be sent to a worker (the engine's clone has no hook for it, and recreating
   one needs a C++ call on `JS::WasmModule` that the Rust bindings do not expose), so
   emscripten's pthreads, which post the module to each worker, still fail.
8. `dom/media/` (with `servo-script-bindings`, item 6, and `vendor/servo-media-gstreamer`):
   **capture.** `getUserMedia` asks the embedder (`PromptPermission`) without blocking the
   page's script thread and rejects with `NotAllowedError` when refused, `NotFoundError`
   when there is no device; `getDisplayMedia` (the whole screen); `getSupportedConstraints`;
   `MediaStreamTrack.label`, `enabled`, `readyState`, `stop()`, `getSettings()`, `onended`;
   `MediaStream.id` and `active`; `enumerateDevices` shows names only after a grant;
   `FERRITE_MOCK_CAPTURE` makes the engine's test sources stand in for devices (probes).

9. `dom/webrtc/rtcdatachannel.rs`, `dom/webrtc/rtcpeerconnection.rs` (with
   `servo-script-bindings`, item 7): **`createDataChannel` throws instead of panicking.**
   When the media backend could not create the channel (it happened on macOS), the script
   thread panicked on `expect("Expected data channel id")` and the page died. Now the page
   gets an `OperationError`. The id was also read with `unwrap_or(...)`, which evaluates its
   argument first, so a channel the remote peer had opened made a second, local channel.
10. `dom/webgl/vertexarrayobject.rs`: **a VAO's finalizer no longer touches other objects.**
   `Drop` called `delete`, which walks the array's attached buffers; when the script thread
   shuts down, the final GC finalizes every object, so those buffers could already be gone,
   and the panic inside a finalizer (which cannot unwind) aborted the process. github.com
   did this on every exit (CI real-site run 37545369862, backtrace in
   `WebGLVertexArrayObjectOES_Binding::_finalize`). `Drop` now sends only
   `DeleteVertexArray` for its own id; a still-attached buffer is freed with its context.
   An explicit `deleteVertexArray()` call still takes the full path. Reproduced before the
   fix with 200 VAOs whose buffers the page deleted (`assertion failed: self.is_deleted()`
   at `webglbuffer.rs:85` from the VAO's drop, then exit 139, three runs of three).
   **Cost:** a buffer the page deleted while a collected VAO still held it is freed when
   its context goes, not at once, so its GPU memory lives as long as the context.
11. `dom/performance/performanceobserver.rs`: **the "no valid entry type" warning names the
   types.** github.com, google.com, amazon.com, nytimes.com and figma.com all ask for
   timings this engine does not record (such as `longtask` or `layout-shift`), and the
   console said only "No valid entry type provided to observe()." It now names what was
   asked for, as other browsers' warnings do. Still a warning, as the specification asks
   (performance-timeline, observe() steps 6.3 and 7.2); nothing else changes.
12. `dom/domexception.rs`: **a DOMException carries a `stack`.** One the engine made (for
   a rejected promise or a thrown error) had none, so a page's logging and Ferrite's own
   console could not say where it came from: YouTube showed "unhandled promise rejection:
   The object is in an invalid state." three times with no file. Like other browsers'
   exceptions, each now has an own `stack` property (the script position it was made at,
   at most 16 frames; non-enumerable, writable, configurable), including one a page makes
   with `new DOMException()`. The cost is one stack capture per exception.
13. `dom/html/embedded_content/htmlmediaelement.rs` (`media_source_ready_state`, item 8's
   Media Source Extensions): **the end of an ended `MediaSource` is the end, not data
   running out.** The player's clock runs a little past the last frame while its sinks
   drain, so the playhead (2.0124 s) passed the end of what was buffered (2.0120 s). The
   element took that for running out of data, dropped its ready state and paused the
   player, before the player could report the end of the stream: `ended` never fired.
   `mse_probe`'s "WebM ends" failed this way about one run in six, here and in CI's macOS
   media job (run 37561043209). Once the source has ended, a position at or past the end
   of the last buffered range keeps `HAVE_ENOUGH_DATA`.
14. `dom/globalscope/globalscope.rs`, `dom/userscripts.rs`: **the compatibility scripts run in
   a frame sandboxed without `allow-scripts`.** They stand in for interfaces other browsers
   build in (`requestIdleCallback`, `Element.animate`...), and a page can call those on a
   sandboxed frame's window from outside: youtube.com's scheduler did, on its
   `sandbox="allow-same-origin"` frame, and stopped on "window.cancelIdleCallback is not a
   function" (a CI site check showed that frame's window with neither function). User
   scripts now go through `evaluate_user_script_on_global`, which refuses only a document
   that is not fully active; the page's own scripts in such a frame still do not run
   (`web_api_probe` checks both).
15. `dom/html/embedded_content/htmlmediaelement.rs` (with `vendor/servo-media-player` and
   `vendor/servo-media-gstreamer`): **a `MediaSource` player that loses a stream starts
   again.** GStreamer's playsink can relink its audio chain while data flows (when the
   first audio output it tries cannot be opened); a source that pushes in that moment
   stops with `not-linked` and the player with it. The player now reports that case as
   `PlayerEvent::StreamLost` (the error's `flow-return` is `NOT_LINKED`), and the element
   makes a new player where the old one was, as after a seek past the end, at most three
   times per load. Other errors, and a stream that keeps failing, are errors as before.
16. `dom/media/mediasource.rs`, `SetDuration` (item 8's file): **a duration inside the last
   frame is raised, not refused.** It threw `InvalidStateError` for any value below the
   end of the buffered data. The specification's duration change algorithm throws only
   below the highest presentation timestamp (the start of the latest buffered frame), and
   otherwise raises a value below the buffered end to that end. YouTube's player sets the
   stream's nominal length right after appending, a few milliseconds inside the last
   frame, and stopped on the exception (T-342). The latest frame start comes from
   `ferrite_mse::Shared::highest_pts`.
17. `dom/html/embedded_content/htmlmediaelement.rs` (`restart_media_source_player`, with
   `vendor/servo-media-gstreamer` item 8): **a new `MediaSource` player's start is set after
   the old player is told to stop**, in one place for both callers (a seek after the end
   and a lost stream), not before; with the run check in the source, a seek the old
   pipeline still reports cannot replace it (T-339).
18. `dom/userscripts.rs` + `dom/trustedtypes/trustedscripturl.rs`: **Ferrite's own compat scripts are named `ferrite-user-script`, and a plain string passed to a `TrustedScriptURL` sink by that caller is accepted.** The service-worker support in `sw_compat.js` starts a worker from a blob URL; under a page policy of `require-trusted-types-for 'script'` (Google Meet) that threw `Cannot set value, expected trusted type` (T-348). Page scripts always have a URL as file name, so a page cannot pass as the caller.
