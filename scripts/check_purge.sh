#!/bin/sh
# scripts/check_purge.sh — fails if the dead broker/policy/sandbox/extension
# architecture has reappeared anywhere in the tracked tree. Run manually or
# from CI. Excludes docs/archive/ (its entire purpose is to hold historical
# references to the dead architecture, clearly banner-marked) and .git/.
#
# Pattern widened per the A0 exit-gate amendment to also catch the Wasm/
# Extism extension-sandbox remnants (wasm32, extism, hello-ext, and its two
# host-function names), not just the broker/policy/sandbox crate names and
# the old JSON-RPC agent port.
set -e
cd "$(git rev-parse --show-toplevel)"

PATTERN='capability.broker|regorus|extism|9222|ferrite-(types|network|a11y|cef|policy|sandbox|capability-broker)|wasm32|hello-ext|host_dom_read|host_network_fetch'

# Exclusions, each with a reason — this is a short, explicit allowlist, not
# a way to make the check pass by hiding things:
#   docs/archive/**          — historical, banner-marked, whole point is to name dead things
#   docs/REBUILD_DIRECTIVE.md, docs/AUDIT.md, docs/DECISIONS.md, docs/PROGRESS.md,
#   docs/TO-DO.md, docs/EVALUATION.md, docs/handoffs/**
#                             — the rebuild's own record; each explicitly discusses what
#                               was removed (or, for EVALUATION.md's §14 checklist and
#                               TO-DO.md's summary, verifies it's gone) and why, by name,
#                               as its stated job — added docs/EVALUATION.md at A13 (T-113)
#                               when its new §14 checklist named "9222"/the broker crates
#                               to confirm their absence, the same pattern this exclusion
#                               list already allowed for the other rebuild-record docs.
#   CLAUDE.md                — the "Never reference, import, or revive: ..." list must
#                               name the forbidden crates to forbid them
#   .devcontainer/Dockerfile — its comments explain *why* a target/COPY line is absent;
#                               obscuring the names would make the comment worse, not safer
#   Cargo.lock                — dependency checksums are long hex strings; "9222" and
#                               similar short numeric tokens appear in them by coincidence,
#                               not because a dead dependency is present (verified: no
#                               `regorus`/`extism*` package entries exist in this file)
#   scripts/check_purge.sh itself — its own source necessarily contains the
#                             pattern strings; a self-match here is not a hit
#
# The former exclusion for crates/ferrite-servo/src/shell.rs (a doc-comment
# that used to say "the capability broker") is gone as of A13 (T-113):
# T-207 made this crate cargo-fmt-clean at A1, which was the blocker A0's
# comment cited, so the comment was reworded instead of staying excluded.
HITS=$(git grep -ilE "$PATTERN" -- \
  ':!docs/archive/**' \
  ':!docs/REBUILD_DIRECTIVE.md' ':!docs/AUDIT.md' ':!docs/DECISIONS.md' ':!docs/PROGRESS.md' \
  ':!docs/TO-DO.md' ':!docs/EVALUATION.md' ':!docs/handoffs/**' \
  ':!CLAUDE.md' ':!.devcontainer/Dockerfile' ':!Cargo.lock' \
  ':!scripts/check_purge.sh' \
  || true)

if [ -n "$HITS" ]; then
  echo "check_purge: dead-architecture references found outside docs/archive/:" >&2
  echo "$HITS" >&2
  exit 1
fi

echo "check_purge: clean."
