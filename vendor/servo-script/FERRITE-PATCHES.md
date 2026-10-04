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
