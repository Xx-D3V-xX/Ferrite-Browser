#!/usr/bin/env bash
# Gathers what is needed to diagnose a crash or a hang into one zip you can send:
#
#   bash scripts/collect-logs.sh            # from a checkout
#   bash /Applications/Ferrite.app/Contents/Resources/collect-logs.sh   # from the app, if bundled
#
# It collects Ferrite's own log (current and previous run), the newest macOS crash
# reports for it, a 5 second stack sample if Ferrite is running right now (run this
# while it is frozen), and basic system and GPU information. Nothing is uploaded.
set -u

stamp="$(date +%Y%m%d-%H%M%S)"
work="$(mktemp -d)"
out_dir="${HOME}/Desktop"
[ -d "$out_dir" ] || out_dir="$HOME"
bundle="$work/ferrite-logs-$stamp"
mkdir -p "$bundle"

case "$(uname -s)" in
  Darwin) log_dir="$HOME/Library/Logs/Ferrite" ;;
  *)      log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/ferrite" ;;
esac

for f in ferrite.log ferrite.previous.log; do
  [ -f "$log_dir/$f" ] && cp "$log_dir/$f" "$bundle/"
done
[ -f "$log_dir/ferrite.log" ] || echo "no $log_dir/ferrite.log (run the app once from Finder or a terminal)" >"$bundle/NO-LOG.txt"

if [ "$(uname -s)" = "Darwin" ]; then
  # Crash reports: the three newest that name Ferrite.
  ls -t "$HOME"/Library/Logs/DiagnosticReports/ferrite*.ips \
        "$HOME"/Library/Logs/DiagnosticReports/Ferrite*.ips 2>/dev/null | head -3 |
    while read -r report; do cp "$report" "$bundle/"; done
  {
    echo "== sw_vers";   sw_vers
    echo "== uname";     uname -a
    echo "== hardware";  sysctl -n machdep.cpu.brand_string hw.memsize hw.ncpu 2>/dev/null
    echo "== displays";  system_profiler SPDisplaysDataType 2>/dev/null | head -40
  } >"$bundle/system.txt" 2>&1
  # A frozen app: sample it now.
  pid="$(pgrep -x ferrite | head -1)"
  [ -z "$pid" ] && pid="$(pgrep -x ferrite-shell | head -1)"
  if [ -n "$pid" ]; then
    echo "Ferrite is running (pid $pid): sampling it for 5 seconds..."
    sample "$pid" 5 -file "$bundle/sample.txt" >/dev/null 2>&1 || echo "sample failed (try with sudo)" >"$bundle/sample-failed.txt"
  fi
else
  { uname -a; lscpu 2>/dev/null | head -20; } >"$bundle/system.txt" 2>&1
fi

zip_path="$out_dir/ferrite-logs-$stamp.zip"
(cd "$work" && zip -qr "$zip_path" "ferrite-logs-$stamp")
rm -rf "$work"
echo "Wrote $zip_path"
echo "Send that file. It contains Ferrite's log (page URLs and console errors can appear in it), crash reports and system info."
