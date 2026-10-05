#!/usr/bin/env bash
# Plays a stream with hls.js, dash.js and Shaka Player in a headless Servo session, to check
# Media Source Extensions with the libraries pages really use. Needs GStreamer (with its
# tools and the x264-free encoders `openh264enc` and `avenc_aac`), Node's npm (to fetch the
# libraries), xvfb, and the media build of the engine. See docs/COMMANDS.md.
#
#   scripts/mse-libs/run.sh            # all three
#   scripts/mse-libs/run.sh hls        # one of hls, dash, shaka
set -euo pipefail
cd "$(dirname "$0")/../.."
work="${MSE_LIBS_DIR:-target/mse-libs}"
mkdir -p "$work/site" "$work/pack"
site="$(cd "$work/site" && pwd)"

if [ ! -f "$site/av.mp4" ]; then
  echo "== making the test streams"
  gst-launch-1.0 -q videotestsrc num-buffers=360 pattern=ball ! video/x-raw,width=320,height=240,framerate=30/1 \
    ! openh264enc gop-size=60 ! h264parse ! mp4mux fragment-duration=2000 streamable=true ! filesink location="$site/v.mp4"
  gst-launch-1.0 -q audiotestsrc num-buffers=516 samplesperbuffer=1024 wave=sine freq=330 \
    ! audio/x-raw,rate=44100,channels=2 ! avenc_aac ! aacparse ! mp4mux fragment-duration=2000 streamable=true \
    ! filesink location="$site/a.mp4"
  gst-launch-1.0 -q videotestsrc num-buffers=360 pattern=ball ! video/x-raw,width=320,height=240,framerate=30/1 \
    ! openh264enc gop-size=60 ! h264parse ! mux. \
    audiotestsrc num-buffers=516 samplesperbuffer=1024 wave=sine freq=330 ! audio/x-raw,rate=44100,channels=2 \
    ! avenc_aac ! aacparse ! mux. mp4mux name=mux fragment-duration=2000 streamable=true ! filesink location="$site/av.mp4"
  python3 scripts/mse-libs/gen.py "$site"
fi

if [ ! -f "$site/hls.min.js" ]; then
  echo "== fetching the libraries"
  (cd "$work/pack" && npm pack hls.js@1.7.3 dashjs@5.2.1 shaka-player@5.2.12 >/dev/null)
  for t in "$work"/pack/*.tgz; do d="$work/pack/$(basename "$t" .tgz)"; mkdir -p "$d"; tar xzf "$t" -C "$d"; done
  cp "$work"/pack/hls.js-*/package/dist/hls.min.js "$site/"
  cp "$work"/pack/dashjs-*/package/dist/modern/umd/dash.all.min.js "$site/"
  cp "$work"/pack/shaka-player-*/package/dist/shaka-player.compiled.js "$site/"
fi
cp scripts/mse-libs/*.html scripts/mse-libs/common.js "$site/"

cargo build -p ferrite-servo --features servo,media --example mse_libs_probe
if [ "$#" -eq 0 ]; then set -- hls dash shaka; fi
status=0
for page in "$@"; do
  echo "== $page"
  out="$(xvfb-run -a target/debug/examples/mse_libs_probe "$site" "$page.html" 2>&1 || true)"
  echo "$out" | grep -E "^(PASS|FAIL|EXCEPTION|PAGE ERROR|INFO|[0-9]+ passed)" || true
  echo "$out" | grep -qE "^[0-9]+ passed, 0 failed$" || status=1
done
exit $status
