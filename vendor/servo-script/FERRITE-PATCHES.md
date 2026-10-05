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
   - **Not done.** A unique index is not enforced (a `put` that repeats a unique
     index key does not fail with `ConstraintError`), and `getAllRecords`.
