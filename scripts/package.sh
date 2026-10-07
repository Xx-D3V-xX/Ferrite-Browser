#!/usr/bin/env bash
# Packages the release binary for one platform into <out>/ (default: dist/).
#
#   scripts/package.sh <macos|windows|linux> <label> <path-to-binary> [out-dir] [media]
#
# With `media` the binary is the `media` build (it links GStreamer): the package carries a
# GStreamer (scripts/bundle-gstreamer.py) and is named ferrite-<label>-<platform>-media.*.
# Every package is checked before it is made: on Windows that each DLL the program loads
# is in it or part of Windows (scripts/windows-runtime.py), on Linux that each library is
# in it or one every desktop has (bundle-gstreamer.py check-linux), on macOS that nothing
# is loaded from Homebrew or another place only the build machine has.
#
#   macos    Ferrite.app (icon + Info.plist, ad-hoc signed when codesign exists)
#            -> ferrite-<label>-macos-arm64.zip
#   windows  ferrite.exe (icon is embedded by crates/ferrite-shell/build.rs)
#            -> ferrite-<label>-windows-x64.zip
#   linux    ferrite + .desktop entry + icon + install.sh
#            -> ferrite-<label>-linux-x64.tar.gz
#
# Used by .github/workflows/ci.yml; runs under bash on all three runner images.
set -euo pipefail

platform="${1:?platform: macos|windows|linux}"
label="${2:?label (short commit sha)}"
binary="${3:?path to the built ferrite-shell binary}"
out="${4:-dist}"
media="${5:-}"
suffix=""
# Windows runners have `python`, not always `python3`.
py="$(command -v python3 || command -v python || echo python3)"
[ "$media" = "media" ] && suffix="-media"

root="$(cd "$(dirname "$0")/.." && pwd)"
[ -f "$binary" ] || { echo "binary not found: $binary" >&2; exit 1; }
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/ferrite-shell/Cargo.toml" | head -1)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

case "$platform" in
  macos)
    app="$work/Ferrite.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 755 "$binary" "$app/Contents/MacOS/ferrite"
    cp "$root/assets/icon/ferrite.icns" "$app/Contents/Resources/ferrite.icns"
    # For a crash or a freeze: bash Ferrite.app/Contents/Resources/collect-logs.sh
    install -m 755 "$root/scripts/collect-logs.sh" "$app/Contents/Resources/collect-logs.sh"
    sed -e "s/@VERSION@/$version/" -e "s/@LABEL@/$label/" \
        "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
    if [ -n "$suffix" ]; then
      "$py" "$root/scripts/bundle-gstreamer.py" macos "$app"
    fi
    # Nothing in the app may load a library from where the build machine kept it (Homebrew,
    # a home folder): a Mac without that would not start the app.
    if command -v otool >/dev/null 2>&1; then
      leak=""
      while IFS= read -r f; do
        file "$f" | grep -q 'Mach-O' || continue
        if otool -L "$f" | tail -n +2 | grep -E "/opt/homebrew|/usr/local|/Users/"; then
          echo "$f needs a library from outside the app"; leak=1
        fi
      done < <(find "$app/Contents" -type f)
      [ -z "$leak" ] || { echo "the app needs libraries that are not in it" >&2; exit 1; }
    fi
    # Ad-hoc signature: not a Developer ID, but it lets the bundle run after the
    # one-time "open anyway" approval instead of being reported as damaged.
    if command -v codesign >/dev/null 2>&1; then
      codesign --force --deep --sign - "$app"
    fi
    zip_name="ferrite-$label-macos-arm64$suffix.zip"
    if command -v ditto >/dev/null 2>&1; then
      ditto -c -k --sequesterRsrc --keepParent "$app" "$out/$zip_name"
    else
      (cd "$work" && zip -qry "$out/$zip_name" Ferrite.app)
    fi
    echo "$out/$zip_name"
    ;;
  windows)
    stage="$work/ferrite-$label-windows-x64"
    mkdir -p "$stage"
    cp "$binary" "$stage/ferrite.exe"
    if [ -n "$suffix" ]; then
      "$py" "$root/scripts/bundle-gstreamer.py" windows "$stage"
    fi
    # The Visual C++ runtime (MSVCP140.dll, VCRUNTIME140.dll...): not part of Windows, and
    # without it the app does not start ("VCRUNTIME140.dll was not found").
    "$py" "$root/scripts/windows-runtime.py" bundle "$stage"
    "$py" "$root/scripts/windows-runtime.py" check "$stage"
    zip_name="ferrite-$label-windows-x64$suffix.zip"
    if command -v 7z >/dev/null 2>&1; then
      (cd "$work" && 7z a -tzip -bso0 "$out/$zip_name" "ferrite-$label-windows-x64")
    elif command -v zip >/dev/null 2>&1; then
      (cd "$work" && zip -qr "$out/$zip_name" "ferrite-$label-windows-x64")
    else
      powershell -NoProfile -Command \
        "Compress-Archive -Path '$(cygpath -w "$stage" 2>/dev/null || echo "$stage")' -DestinationPath '$(cygpath -w "$out/$zip_name" 2>/dev/null || echo "$out/$zip_name")'"
    fi
    echo "$out/$zip_name"
    ;;
  linux)
    stage="$work/ferrite-$label-linux-x64"
    mkdir -p "$stage"
    install -m 755 "$binary" "$stage/ferrite"
    cp "$root/assets/icon/ferrite-512.png" "$stage/ferrite.png"
    cp "$root/packaging/linux/ferrite.desktop" "$root/packaging/linux/README.txt" "$stage/"
    install -m 755 "$root/packaging/linux/install.sh" "$stage/install.sh"
    if [ -n "$suffix" ]; then
      "$py" "$root/scripts/bundle-gstreamer.py" linux "$stage"
      printf '\nThis is the media build (video, audio, calls). It carries its own GStreamer in\nlib/: keep that folder next to the ferrite program (install.sh copies both).\n' >> "$stage/README.txt"
    fi
    "$py" "$root/scripts/bundle-gstreamer.py" check-linux "$stage"
    tar_name="ferrite-$label-linux-x64$suffix.tar.gz"
    tar -C "$work" -czf "$out/$tar_name" "ferrite-$label-linux-x64"
    echo "$out/$tar_name"
    ;;
  *)
    echo "unknown platform: $platform (macos|windows|linux)" >&2
    exit 2
    ;;
esac
