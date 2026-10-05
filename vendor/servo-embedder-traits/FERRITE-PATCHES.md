# Ferrite's changes to servo-embedder-traits 0.6.0

`servo-embedder-traits` 0.6.0 from crates.io (MPL-2.0) with one change, used through
`[patch.crates-io]`.

1. `lib.rs`: `PermissionFeature::ScreenCapture`, so that a page's `getDisplayMedia`
   reaches the embedder as its own kind of permission request (the engine's list of
   permission names, which the other variants mirror, has no name for it).
