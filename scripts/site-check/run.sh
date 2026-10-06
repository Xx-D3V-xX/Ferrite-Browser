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
  echo "| site | title | load | errors | warnings | crash | page answer |"
  echo "|---|---|---|---|---|---|---|"
} > "$summary"
list="${SITE_CHECK_LIST:-$(dirname "$0")/sites.txt}"
grep -vE '^\s*(#|$)' "$list" | while IFS= read -r line; do
  name="$(echo "$line" | awk -F' \\| ' '{print $1}' | xargs)"
  url="$(echo "$line" | awk -F' \\| ' '{print $2}' | xargs)"
  js="$(echo "$line" | awk -F' \\| ' '{print $3}')"
  if [ "$#" -gt 0 ] && ! printf '%s\n' "$@" | grep -qx "$name"; then continue; fi
  echo "== $name $url"
  log="$out/$name.log"
  PAGE_SHOT_JS="${js:-document.readyState}" timeout 120 "$bin" "$url" "$wait_ms" "$out/$name.png" 1280 800 > "$log" 2>&1
  code=$?
  title="$(grep -m1 '^TITLE' "$log" | cut -c9- | tr '|' '/' | cut -c1-60)"
  load="$(grep -m1 '^LOADED' "$log" | cut -c9- | cut -c1-30)"
  errors="$(grep -c '^CONSOLE error' "$log")"
  warnings="$(grep -c '^CONSOLE warn' "$log")"
  crash="$(grep -m1 '^CRASH' "$log" | cut -c9- | cut -c1-60)"
  [ "$code" -ne 0 ] && crash="${crash} exit $code"
  answer="$(grep -m1 '^JS' "$log" | cut -c9- | tr '|' '/' | cut -c1-80)"
  echo "| $name | $title | $load | $errors | $warnings | ${crash:-} | $answer |" >> "$summary"
  grep -E '^(TITLE|LOADED|JS|CRASH|CONSOLE error)' "$log" | head -15
done
echo
cat "$summary"
