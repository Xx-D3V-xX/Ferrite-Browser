#!/bin/sh
# scripts/check_no_archive_links.sh — fails if any LIVE doc links into
# docs/archive/. The archive is banner-marked HISTORICAL specifically so it
# doesn't get read as current context; a live doc pointing into it would
# undermine that. docs/archive/README.md is the one permitted exception
# (something has to be able to describe the archive's contents).
#
# Also checked: CLAUDE.md must never contain a status/"not yet implemented"
# section (D11's root cause) — folded in here since both checks exist for
# the same reason (doc drift becoming silent context poisoning) and both
# belong in the same CI step per docs/REBUILD_DIRECTIVE.md §12.
set -e
cd "$(git rev-parse --show-toplevel)"

FAIL=0

# ── No live doc cites the archive as a markdown link (a live, clickable
# reference implies "go read this for real information" — a plain prose
# mention like "see docs/archive/README.md, it's historical" is fine and
# is how CLAUDE.md/README.md/docs/PROGRESS.md point AWAY from the archive).
# docs/archive/README.md itself is excepted since describing its own
# contents is its whole job.
HITS=$(git grep -lE '\]\(\.{0,2}/?docs/archive/' -- ':!docs/archive/**' 2>/dev/null || true)
if [ -n "$HITS" ]; then
  echo "check_no_archive_links: live files cite docs/archive/ as a markdown link (only docs/archive/README.md may):" >&2
  echo "$HITS" >&2
  FAIL=1
fi

# ── CLAUDE.md carries no status claims ───────────────────────────────────
# Matches a HEADING naming a planned/status section (the D11 failure shape:
# "## Planned Near-Term Work (not yet implemented)"), not bare prose that
# merely discusses the rule — this file is required to state the rule
# using those exact words, so a bare phrase match would flag itself.
if [ -f CLAUDE.md ] && grep -qiE '^#+.*(not yet implemented|planned|to.?do)' CLAUDE.md; then
  echo "check_no_archive_links: CLAUDE.md has a heading naming a status/planned section — status belongs only in docs/PROGRESS.md and docs/TO-DO.md (see D11)." >&2
  FAIL=1
fi

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

echo "check_no_archive_links: clean."
