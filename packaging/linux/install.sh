#!/bin/sh
# Installs Ferrite for the current user: the app, a launcher entry and the icon.
#
# The app goes in $prefix/lib/ferrite, whole: the media build carries its own GStreamer
# in lib/ beside the binary, found from where the binary really is, so the binary is
# linked into $prefix/bin rather than copied there alone.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix="${PREFIX:-$HOME/.local}"
app="$prefix/lib/ferrite"
mkdir -p "$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/512x512/apps"
rm -rf "$app"
mkdir -p "$app"
install -m 755 "$here/ferrite" "$app/ferrite"
[ -d "$here/lib" ] && cp -R "$here/lib" "$app/lib"
rm -f "$prefix/bin/ferrite"
ln -s "$app/ferrite" "$prefix/bin/ferrite"
install -m 644 "$here/ferrite.png" "$prefix/share/icons/hicolor/512x512/apps/ferrite.png"
sed "s|^Exec=ferrite|Exec=$app/ferrite|" "$here/ferrite.desktop" > "$prefix/share/applications/ferrite.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
echo "Installed to $app (run it as $prefix/bin/ferrite)"
echo "To remove it: rm -rf \"$app\" \"$prefix/bin/ferrite\" \"$prefix/share/applications/ferrite.desktop\""
