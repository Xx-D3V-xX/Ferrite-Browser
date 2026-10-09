#!/usr/bin/env bash
# Prints the lines of a log that explain a failure as one GitHub error annotation, so
# they can be read from the run's summary and through the API (a job's full log can
# only be downloaded, and not from everywhere).
#
#   scripts/ci-annotate.sh <title> <log file> [extended regex of the lines to keep]
#
# At most the last 40 matching lines; with none matching, the last 20 lines of the log.
set -uo pipefail
title="${1:?title}"
log="${2:?log file}"
pattern="${3:-FAIL|MISSING|not found|PANIC|panicked|error|exited with}"
[ -f "$log" ] || { echo "::error title=$title::no log at $log"; exit 0; }
lines="$(grep -E "$pattern" "$log" | tail -40)"
[ -n "$lines" ] || lines="$(tail -20 "$log")"
# An annotation is one line: newlines, and the characters that would end it, are escaped.
body="$(printf '%s' "$lines" | sed -e 's/%/%25/g' -e 's/\r//g' | awk 'BEGIN{ORS="%0A"} {print}')"
echo "::error title=$title::$body"
