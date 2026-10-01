#!/bin/sh
# Installs Ferrite for the current user: binary, launcher entry and icon.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix="${PREFIX:-$HOME/.local}"
mkdir -p "$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/512x512/apps"
install -m 755 "$here/ferrite" "$prefix/bin/ferrite"
install -m 644 "$here/ferrite.png" "$prefix/share/icons/hicolor/512x512/apps/ferrite.png"
sed "s|^Exec=ferrite|Exec=$prefix/bin/ferrite|" "$here/ferrite.desktop" > "$prefix/share/applications/ferrite.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
echo "Installed to $prefix (binary: $prefix/bin/ferrite)"
