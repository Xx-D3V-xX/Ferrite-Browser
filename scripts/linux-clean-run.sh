#!/usr/bin/env bash
# Runs a Linux package, and optionally the engine's media probes, on a clean Ubuntu: a
# container with no GStreamer and nothing else beyond what every desktop has (the
# libraries scripts/bundle-gstreamer.py leaves to the machine).
#
#   scripts/linux-clean-run.sh <dir>
#
#   <dir>/package/<one folder>/   an unpacked release (ferrite, and lib/ for a media build)
#   <dir>/probes/                 optional: media_probe and mse_probe, bundled the same way
#
# Fails if a program or library in the package names a library the container cannot find,
# if the container has any GStreamer of its own (the test would prove nothing), or if a
# probe fails. Needs docker (GitHub's Linux runners have it).
set -euo pipefail

dir="$(cd "${1:?directory with package/ and optionally probes/}" && pwd)"
image="${FERRITE_CLEAN_IMAGE:-ubuntu:24.04}"

docker run --rm -v "$dir:/e2e" "$image" bash -s <<'INSIDE'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
# A desktop's libraries: C and C++ runtimes and GLib come with the image or these.
apt-get install -y -qq --no-install-recommends \
  libglib2.0-0t64 libfontconfig1 libfreetype6 libharfbuzz0b \
  libx11-6 libx11-xcb1 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libxext6 libxrandr2 \
  libxi6 libxcursor1 libxfixes3 libxdamage1 libxrender1 libxtst6 libxv1 \
  libwayland-client0 libwayland-egl1 libwayland-cursor0 \
  libegl1 libgl1 libgles2 libgbm1 libdrm2 libegl-mesa0 libgl1-mesa-dri libvulkan1 \
  libasound2t64 libpulse0 libdbus-1-3 libudev1 libssl3t64 ca-certificates \
  xvfb xauth > /dev/null
if ldconfig -p | grep -q 'libgst'; then
  echo "the clean container has a GStreamer of its own; the test would prove nothing"
  ldconfig -p | grep libgst | head
  exit 1
fi
echo "--- the container has no GStreamer"
bad=""
missing() {   # every ELF file under $1: no library "not found"
  local n=0
  while IFS= read -r f; do
    head -c 4 "$f" | grep -q 'ELF' || continue
    n=$((n+1))
    if ldd "$f" 2>&1 | grep -q 'not found'; then
      echo "MISSING for ${f#/e2e/}:"; ldd "$f" | grep 'not found'; bad=1
    fi
  done < <(find "$1" -type f)
  echo "--- $1: $n programs and libraries checked"
}
for pkg in /e2e/package/*/; do missing "$pkg"; done
if [ -d /e2e/probes ]; then
  missing /e2e/probes
  [ -z "$bad" ] || exit 1
  run_probe() {   # $1 = probe, then how to run it
    local probe="$1"; shift
    set +e
    "$@" > "/tmp/$probe.log" 2>&1
    local code=$?
    set -e
    grep -E "^(PASS|FAIL)|passed, |panicked|PANIC|GStreamer bundle" "/tmp/$probe.log" | tail -60
    if grep -qE "panicked|PANIC" "/tmp/$probe.log"; then echo "$probe: A PANIC"; bad=1; fi
    if [ "$code" -ne 0 ]; then echo "$probe exited with $code"; tail -30 "/tmp/$probe.log"; bad=1; fi
    echo "=== $probe exit $code"
  }
  run_probe media_probe /e2e/probes/media_probe
  run_probe mse_probe xvfb-run -a /e2e/probes/mse_probe
fi
[ -z "$bad" ]
echo "--- clean run ok"
INSIDE
