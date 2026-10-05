# Ferrite's changes to servo-script-bindings 0.6.0

This directory is `servo-script-bindings` 0.6.0 from crates.io (MPL-2.0, see the
headers of its files) with the changes below. It is used through `[patch.crates-io]`
in the workspace `Cargo.toml`. It is vendored because the web interface
definitions (`webidls/*.webidl`) are compiled into the crate when it is built, so
they cannot be changed from outside. Drop the patch (and this directory) when the
engine is upgraded to a release that has these interfaces.

1. `webidls/IDBCursor.webidl`: `advance`, `continue`, `continuePrimaryKey`,
   `update` and `delete` are switched on (they were commented out). Without
   `continue` a cursor can return only its first record, which broke the page
   scripts of YouTube and Google (`this.cursor.continue is not a function`).
2. `webidls/IDBIndex.webidl`: `get`, `getKey`, `getAll`, `getAllKeys`, `count`,
   `openCursor` and `openKeyCursor` are switched on (they were commented out; an
   index could not be queried at all: `S.wrapped.openCursor is not a function`).
   `getAllRecords` is still off.
3. `webidls/IDBRequest.webidl`: `source` is `(IDBObjectStore or IDBIndex or
   IDBCursor)?`, as the specification says; it was an `IDBObjectStore?` only, so a
   request made on an index or a cursor could not name its source.
4. `codegen/Bindings.conf`: the new methods are listed in the `cx` lists of
   `IDBCursor` and `IDBIndex`, so the engine's implementation (in
   `vendor/servo-script`, `dom/indexeddb/`; see its `FERRITE-PATCHES.md`) gets a
   JavaScript context like the other IndexedDB methods do.
5. `interface.rs`: `SharedArrayBuffer` and `Atomics` are on in every realm
   (`sharedMemoryAndAtomics_`); they were hard-wired off. `FERRITE_SHARED_MEMORY=off` turns
   them off. See `vendor/servo-script/FERRITE-PATCHES.md`, item 7.
6. `webidls/MediaDevices.webidl`, `MediaStreamTrack.webidl`, `MediaStream.webidl`,
   `codegen/Bindings.conf`: `getDisplayMedia`, `getSupportedConstraints`, the track's
   `label`, `enabled`, `muted`, `readyState`, `stop()`, `getSettings()` and event handlers,
   the stream's `id` and `active`. See `vendor/servo-script/FERRITE-PATCHES.md`, item 8.
