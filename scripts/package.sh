#!/usr/bin/env bash
# Packages the release binary for one platform into <out>/ (default: dist/).
#
#   scripts/package.sh <macos|windows|linux> <label> <path-to-binary> [out-dir]
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
    # Ad-hoc signature: not a Developer ID, but it lets the bundle run after the
    # one-time "open anyway" approval instead of being reported as damaged.
    if command -v codesign >/dev/null 2>&1; then
      codesign --force --deep --sign - "$app"
    fi
    zip_name="ferrite-$label-macos-arm64.zip"
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
    zip_name="ferrite-$label-windows-x64.zip"
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
    tar_name="ferrite-$label-linux-x64.tar.gz"
    tar -C "$work" -czf "$out/$tar_name" "ferrite-$label-linux-x64"
    echo "$out/$tar_name"
    ;;
  *)
    echo "unknown platform: $platform (macos|windows|linux)" >&2
    exit 2
    ;;
esac
