#!/usr/bin/env bash
# Loads every site in sites.txt in the real engine (page_shot) and writes, per site,
# its log and a screenshot, plus summary.md: title, load time, console errors, crash.
# Needs the network, so it runs on a CI machine (ci.yml, `only_sites`).
# SITE_CHECK_LIST names another list (a local test).
#
#   scripts/site-check/run.sh <page_shot binary> <out dir> [name ...]
set -u
bin="$1"; out="$2"; shift 2
mkdir -p "$out"
wait_ms="${SITE_CHECK_WAIT_MS:-15000}"
summary="$out/summary.md"
{
  echo "| site | title | load | painted | errors | warnings | crash | page answer |"
  echo "|---|---|---|---|---|---|---|---|"
} > "$summary"
list="${SITE_CHECK_LIST:-$(dirname "$0")/sites.txt}"
grep -vE '^\s*(#|$)' "$list" | while IFS= read -r line; do
  name="$(echo "$line" | awk -F' \\| ' '{print $1}' | xargs)"
  url="$(echo "$line" | awk -F' \\| ' '{print $2}' | xargs)"
  js="$(echo "$line" | awk -F' \\| ' '{print $3}')"
  wait="$wait_ms"
  compat="${FERRITE_COMPAT:-}"
  # Suffixes on the name: `@slow` (a bot check that takes a while) and `@compat=<setting>`
  # (FERRITE_COMPAT for this load only, to bisect a page against the compatibility scripts).
  while :; do
    case "$name" in
      *@slow) name="${name%@slow}"; wait="${SITE_CHECK_SLOW_MS:-40000}" ;;
      *@compat=*) compat="${name##*@compat=}"; name="${name%@compat=*}" ;;
      *) break ;;
    esac
  done
  if [ "$#" -gt 0 ] && ! printf '%s\n' "$@" | grep -qx "$name"; then continue; fi
  echo "== $name $url"
  log="$out/$name.log"
  FERRITE_COMPAT="$compat" PAGE_SHOT_JS="${js:-document.readyState}" timeout 150 "$bin" "$url" "$wait" "$out/$name.png" 1280 800 > "$log" 2>&1
  code=$?
  # A crash (a signal, not a failed check): load it again under gdb for a backtrace.
  if [ "$code" -ge 128 ] && [ "$code" -ne 143 ] && command -v gdb > /dev/null; then
    echo "-- exit $code: again under gdb" >> "$log"
    FERRITE_COMPAT="$compat" PAGE_SHOT_JS="${js:-document.readyState}" timeout 300 gdb -batch -q -ex run -ex "bt 40" -ex "info threads" -ex "thread apply all bt 12" \
      --args "$bin" "$url" "$wait" "$out/$name-gdb.png" 1280 800 >> "$log" 2>&1
  fi
  title="$(grep -m1 '^TITLE' "$log" | cut -c9- | tr '|' '/' | cut -c1-60)"
  load="$(grep -m1 '^LOADED' "$log" | cut -c9- | cut -c1-30)"
  painted="$(grep -m1 '^PAINT' "$log" | cut -c9- | cut -d' ' -f1)"
  errors="$(grep -c '^CONSOLE error' "$log")"
  warnings="$(grep -c '^CONSOLE warn' "$log")"
  crash="$(grep -m1 '^CRASH' "$log" | cut -c9- | cut -c1-60)"
  [ "$code" -ne 0 ] && crash="${crash} exit $code"
  answer="$(grep -m1 '^JS' "$log" | cut -c9- | tr '|' '/' | cut -c1-80)"
  echo "| $name | $title | $load | ${painted:-} | $errors | $warnings | ${crash:-} | $answer |" >> "$summary"
  grep -E '^(TITLE|LOADED|PAINT|JS|CRASH|CONSOLE (error|warn))' "$log" | head -25
  # The backtrace of the crashing thread, if gdb ran.
  grep -A50 'received signal' "$log" | head -60
done
echo
cat "$summary"
