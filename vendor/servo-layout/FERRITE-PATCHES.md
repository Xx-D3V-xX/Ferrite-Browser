# Ferrite's changes to servo-layout 0.6.0

This directory is `servo-layout` 0.6.0 from crates.io (MPL-2.0, see the headers of
its files) with the change below. It is used through `[patch.crates-io]` in the
workspace `Cargo.toml`. Drop the patch (and this directory) when the engine is
upgraded to a release that has the fix.

1. `query.rs`, `containing_block_for_node`: when an ancestor has no layout box
   (for example a `display: contents` wrapper), move on to that ancestor's parent
   instead of `continue`-ing without advancing. The original asked for the same
   parent's style again on every turn, an infinite loop. It runs on a page's
   script thread (an IntersectionObserver with an explicit `root` reaches it from
   `update_the_rendering`), so the page stopped responding, and with the load
   never completing, scrolling and clicking dead, and one CPU core at 100%. Seen
   on Google's results page on an Apple M1: a 5-second `sample` of the frozen app
   was entirely inside `layout::query::style_and_flags_for_node`. Reproduced with
   a test page: a scroll container as the observer's `root`, the observed element
   inside two `display: contents` wrappers.
