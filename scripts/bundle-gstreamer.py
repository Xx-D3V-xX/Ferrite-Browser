#!/usr/bin/env python3
"""Puts a GStreamer next to the Ferrite app, for a release of the `media` build.

    scripts/bundle-gstreamer.py macos   Ferrite.app [--prefix /opt/homebrew]
    scripts/bundle-gstreamer.py windows STAGE_DIR   [--root "C:\\Program Files\\gstreamer\\1.0\\msvc_x86_64"]

The layout is the one `crates/ferrite-servo/src/bundle.rs` looks for:

    macOS    Contents/Frameworks/*.dylib            libraries, with @rpath names
             Contents/Resources/gstreamer/plugins   the plugins
             Contents/Resources/gstreamer/gst-plugin-scanner
    Windows  STAGE_DIR/*.dll                        libraries, next to ferrite.exe
             STAGE_DIR/gstreamer/plugins            the plugins
             STAGE_DIR/gstreamer/gst-plugin-scanner.exe

Only the plugins in PLUGINS are copied (a playbin3 pipeline, the codecs and containers a
page's media uses, WebRTC, capture), and the libraries those need. A plugin that is not
installed is reported and left out; the build fails only if the core ones are missing.

It has not been run on a Mac or on Windows by its author (nothing there to run it on);
`--self-test` checks the parts that do not need one. The release workflow builds these
packages in a job that cannot hold a release up, so a failure here costs the media
package, not the release.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

# Plugin names (the file without `lib` and the extension). `CORE` must be there.
CORE = ["gstcoreelements", "gstapp", "gstplayback", "gsttypefindfunctions"]
PLUGINS = CORE + [
    # Conversion and plumbing.
    "gstvideoconvertscale", "gstvideoscale", "gstvideoconvert", "gstvideorate", "gstaudioconvert",
    "gstaudioresample", "gstaudiorate", "gstvolume", "gstautodetect", "gstvideofilter",
    "gstaudiotestsrc", "gstvideotestsrc", "gsttcp", "gstmultifile", "gstqueue2",
    # Containers and parsers (MSE feeds elementary streams; files and capture need the rest).
    "gstisomp4", "gstmatroska", "gstogg", "gstwavparse", "gstid3demux", "gstaudioparsers",
    "gstvideoparsersbad", "gstmpegtsdemux", "gstmpegpsdemux", "gstadaptivedemux2", "gstapetag",
    "gsticydemux", "gstflac", "gstlame", "gstmpg123", "gstaiff", "gstavi", "gstflv",
    # Decoders and encoders.
    "gstlibav", "gstopenh264", "gstvpx", "gstopus", "gstvorbis", "gsttheora", "gstdav1d",
    "gstaom", "gstx264", "gstopusparse", "gstdvdsub", "gstsubparse", "gstvideoparsersbad",
    # WebRTC.
    "gstwebrtc", "gstdtls", "gstsrtp", "gstnice", "gstrtp", "gstrtpmanager", "gstsctp", "gstrtsp",
    "gstsdpelem", "gstudp", "gstvideo4linux2",
    # macOS.
    "gstapplemedia", "gstosxaudio", "gstosxvideo", "gstvideotoolbox",
    # Windows.
    "gstwasapi", "gstwasapi2", "gstd3d11", "gstmediafoundation", "gstwinscreencap",
    "gstdirectsound", "gstdirectshow", "gstd3d12", "gstwinks",
    # OpenGL (the video sink the engine uses when it can).
    "gstopengl", "gstgl", "gstglimagesink", "gstgtk",
]


def plugin_name(path: str) -> str:
    """`libgstisomp4.dylib` / `gstisomp4.dll` / `libgstisomp4.so` -> `gstisomp4`."""
    name = os.path.basename(path)
    name = re.sub(r"\.(dylib|so|dll)$", "", name)
    return name[3:] if name.startswith("lib") else name


def wanted_plugins(paths: list[str]) -> list[str]:
    """The files among `paths` that are plugins this bundle carries, in a stable order."""
    allow = set(PLUGINS)
    return sorted(p for p in paths if plugin_name(p) in allow)


def missing_core(paths: list[str]) -> list[str]:
    have = {plugin_name(p) for p in paths}
    return [c for c in CORE if c not in have]


def parse_otool(output: str, own_name: str) -> list[str]:
    """The libraries an `otool -L` listing names, without the file itself and without
    the ones every Mac has (/usr/lib, /System)."""
    deps = []
    for line in output.splitlines()[1:]:
        line = line.strip()
        if not line:
            continue
        path = line.split(" (compatibility")[0].strip()
        if path.startswith(("/usr/lib/", "/System/")):
            continue
        if os.path.basename(path) == own_name:
            continue
        deps.append(path)
    return deps


def parse_rpaths(otool_l: str) -> list[str]:
    """The LC_RPATH entries an `otool -l` listing shows."""
    found, want = [], False
    for line in otool_l.splitlines():
        text = line.strip()
        if text == "cmd LC_RPATH":
            want = True
        elif want and text.startswith("path "):
            found.append(text[len("path "):].split(" (offset")[0])
            want = False
    return found


def rpath_name(path: str) -> str:
    return "@rpath/" + os.path.basename(path)


def resolve_dependency(dep: str, search: list[Path], loader_dir: Path) -> Path | None:
    """Where a dependency named in a Mach-O file really is."""
    if dep.startswith("@loader_path/"):
        candidate = loader_dir / dep[len("@loader_path/"):]
        return candidate if candidate.exists() else None
    if dep.startswith("@rpath/") or dep.startswith("@executable_path/"):
        name = dep.split("/", 1)[1]
        for directory in search:
            if (directory / name).exists():
                return directory / name
        return None
    path = Path(dep)
    return path if path.exists() else None


# ---------------------------------------------------------------------------- macOS


def run(cmd: list[str], **kw) -> str:
    result = subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)
    return result.stdout


def brew_dirs(prefix: Path) -> tuple[list[Path], list[Path]]:
    """(plugin directories, library directories) under a Homebrew prefix."""
    plugin_dirs = sorted(prefix.glob("lib/gstreamer-1.0")) + sorted(prefix.glob("opt/*/lib/gstreamer-1.0"))
    lib_dirs = [prefix / "lib"] + sorted(prefix.glob("opt/*/lib"))
    return plugin_dirs, lib_dirs


def bundle_macos(app: Path, prefix: Path, main_name: str = "ferrite") -> None:
    frameworks = app / "Contents" / "Frameworks"
    gst = app / "Contents" / "Resources" / "gstreamer"
    plugins_out = gst / "plugins"
    frameworks.mkdir(parents=True, exist_ok=True)
    plugins_out.mkdir(parents=True, exist_ok=True)
    plugin_dirs, lib_dirs = brew_dirs(prefix)

    found: dict[str, Path] = {}
    for d in plugin_dirs:
        for f in d.glob("*.dylib"):
            # Homebrew leaves a link behind for a plugin whose package is gone: skip it.
            if not f.exists():
                print(f"warning: {f} points at nothing, left out", file=sys.stderr)
                continue
            found.setdefault(f.name, f)
    chosen = wanted_plugins([str(p) for p in found.values()])
    gone = missing_core([str(p) for p in found.values()])
    if gone:
        sys.exit(f"missing core GStreamer plugins under {prefix}: {gone}")
    scanner = next(iter(prefix.glob("opt/gstreamer/libexec/gstreamer-1.0/gst-plugin-scanner")), None) or \
        next(iter(prefix.glob("libexec/gstreamer-1.0/gst-plugin-scanner")), None)

    main = app / "Contents" / "MacOS" / main_name
    todo: list[tuple[Path, Path, str]] = []  # (source, destination, rpath to add)
    for p in chosen:
        todo.append((Path(p), plugins_out / Path(p).name, "@loader_path/../../../Frameworks"))
    if scanner:
        todo.append((scanner, gst / "gst-plugin-scanner", "@loader_path/../../Frameworks"))

    copied: dict[str, Path] = {}  # basename -> destination in Frameworks
    queue: list[Path] = [main] + [d for _, d, _ in todo]
    for src, dst, _ in todo:
        shutil.copy2(src, dst)
        os.chmod(dst, 0o755)
    seen: set[str] = set()
    changes: dict[Path, list[tuple[str, str]]] = {}
    while queue:
        current = queue.pop()
        if str(current) in seen:
            continue
        seen.add(str(current))
        for dep in parse_otool(run(["otool", "-L", str(current)]), current.name):
            real = resolve_dependency(dep, lib_dirs, current.parent)
            if real is None:
                print(f"warning: {current.name} wants {dep}, which was not found", file=sys.stderr)
                continue
            name = real.name
            if name not in copied:
                dest = frameworks / name
                shutil.copy2(real, dest)
                os.chmod(dest, 0o755)
                copied[name] = dest
                queue.append(dest)
            changes.setdefault(current, []).append((dep, rpath_name(str(real))))
    # Rewrite the names, then give every file somewhere to look and a signature again (a
    # changed Mach-O file without one does not start on Apple silicon).
    rpaths = {main: "@executable_path/../Frameworks"}
    rpaths.update({d: r for _, d, r in todo})
    for name, dest in copied.items():
        rpaths[dest] = "@loader_path"
        run(["install_name_tool", "-id", rpath_name(name), str(dest)])
    # A plugin's own name should not point at the machine that built the bundle either.
    for _, dest, _ in todo:
        if dest.parent == plugins_out:
            run(["install_name_tool", "-id", rpath_name(dest.name), str(dest)])
    for target, pairs in changes.items():
        for old, new in pairs:
            run(["install_name_tool", "-change", old, new, str(target)])
    # Take out the search paths the file was built with. They are searched before ours, so
    # on a machine that has Homebrew's GStreamer they would load Homebrew's copy of every
    # library next to ours (two GLibs in one process: spurious failures and silent exits).
    for target in rpaths:
        for old in parse_rpaths(run(["otool", "-l", str(target)])):
            run(["install_name_tool", "-delete_rpath", old, str(target)])
    for target, rpath in rpaths.items():
        try:
            run(["install_name_tool", "-add_rpath", rpath, str(target)])
        except subprocess.CalledProcessError:
            pass  # already there
    for target in list(rpaths):
        run(["codesign", "--force", "--sign", "-", str(target)])
    print(f"bundled {len(chosen)} plugins and {len(copied)} libraries into {app}")


# -------------------------------------------------------------------------- Windows


def bundle_windows(stage: Path, root: Path) -> None:
    gst = stage / "gstreamer"
    plugins_out = gst / "plugins"
    plugins_out.mkdir(parents=True, exist_ok=True)
    plugin_files = [str(p) for p in (root / "lib" / "gstreamer-1.0").glob("*.dll")]
    gone = missing_core(plugin_files)
    if gone:
        sys.exit(f"missing core GStreamer plugins under {root}: {gone}")
    for p in wanted_plugins(plugin_files):
        shutil.copy2(p, plugins_out)
    # Windows resolves a plugin's libraries from the application's directory.
    libs = 0
    for dll in (root / "bin").glob("*.dll"):
        shutil.copy2(dll, stage)
        libs += 1
    scanner = root / "libexec" / "gstreamer-1.0" / "gst-plugin-scanner.exe"
    if scanner.exists():
        shutil.copy2(scanner, gst)
    print(f"bundled {len(wanted_plugins(plugin_files))} plugins and {libs} libraries into {stage}")


# ----------------------------------------------------------------------------- tests

SAMPLE_OTOOL = """/opt/homebrew/opt/gstreamer/lib/gstreamer-1.0/libgstisomp4.dylib:
\t/opt/homebrew/opt/gstreamer/lib/gstreamer-1.0/libgstisomp4.dylib (compatibility version 0.0.0, current version 0.0.0)
\t/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib (compatibility version 8401.0.0, current version 8401.5.0)
\t@rpath/libgstbase-1.0.0.dylib (compatibility version 2401.0.0, current version 2401.1.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)
\t/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 3000.0.0)
"""


def self_test() -> int:
    import tempfile

    assert plugin_name("/x/lib/gstreamer-1.0/libgstisomp4.dylib") == "gstisomp4"
    assert plugin_name("gstisomp4.dll") == "gstisomp4"
    assert plugin_name("libgstlibav.so") == "gstlibav"
    paths = ["a/libgstisomp4.dylib", "a/libgstsomethingelse.dylib", "a/libgstcoreelements.dylib"]
    assert wanted_plugins(paths) == ["a/libgstcoreelements.dylib", "a/libgstisomp4.dylib"]
    assert missing_core(paths) == ["gstapp", "gstplayback", "gsttypefindfunctions"]
    deps = parse_otool(SAMPLE_OTOOL, "libgstisomp4.dylib")
    assert deps == [
        "/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib",
        "@rpath/libgstbase-1.0.0.dylib",
    ], deps
    assert rpath_name("/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib") == "@rpath/libglib-2.0.0.dylib"
    with tempfile.TemporaryDirectory() as t:
        d = Path(t)
        (d / "libfoo.dylib").write_bytes(b"")
        assert resolve_dependency("@rpath/libfoo.dylib", [d], d) == d / "libfoo.dylib"
        assert resolve_dependency("@rpath/libbar.dylib", [d], d) is None
        assert resolve_dependency("@loader_path/libfoo.dylib", [], d) == d / "libfoo.dylib"
        assert resolve_dependency(str(d / "libfoo.dylib"), [], d) == d / "libfoo.dylib"
        # A fake Windows install: bundling copies the wanted plugins and every DLL.
        root = d / "gst"
        (root / "lib" / "gstreamer-1.0").mkdir(parents=True)
        (root / "bin").mkdir()
        for n in CORE + ["gstisomp4", "gstunrelated"]:
            (root / "lib" / "gstreamer-1.0" / f"{n}.dll").write_bytes(b"x")
        (root / "bin" / "gstreamer-1.0-0.dll").write_bytes(b"x")
        (root / "bin" / "gst-launch-1.0.exe").write_bytes(b"x")
        stage = d / "stage"
        stage.mkdir()
        bundle_windows(stage, root)
        assert (stage / "gstreamer" / "plugins" / "gstisomp4.dll").exists()
        assert not (stage / "gstreamer" / "plugins" / "gstunrelated.dll").exists()
        assert (stage / "gstreamer-1.0-0.dll").exists()
        assert not (stage / "gst-launch-1.0.exe").exists()
        # A dangling plugin link (Homebrew leaves one for libnice) is skipped, not copied.
        brew = d / "brew"
        plug = brew / "lib" / "gstreamer-1.0"
        plug.mkdir(parents=True)
        (plug / "libgstcoreelements.dylib").write_bytes(b"x")
        (plug / "libgstnice.dylib").symlink_to(d / "nowhere.dylib")
        listed = [f.name for f in plug.glob("*.dylib") if f.exists()]
        assert listed == ["libgstcoreelements.dylib"], listed
    sample = """Load command 12
          cmd LC_RPATH
      cmdsize 48
         path /opt/homebrew/lib (offset 12)
Load command 13
          cmd LC_LOAD_DYLIB
         name /usr/lib/libSystem.B.dylib (offset 24)
Load command 14
          cmd LC_RPATH
      cmdsize 64
         path /opt/homebrew/Cellar/gstreamer/1.26.0/lib (offset 12)
"""
    assert parse_rpaths(sample) == ["/opt/homebrew/lib", "/opt/homebrew/Cellar/gstreamer/1.26.0/lib"], parse_rpaths(sample)
    assert parse_rpaths("") == []
    print("self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("platform", choices=["macos", "windows", "self-test"])
    ap.add_argument("target", nargs="?", help="Ferrite.app (macos) or the stage directory (windows)")
    ap.add_argument("--main", default="ferrite", help="name of the executable in Contents/MacOS (the smoke test uses another)")
    ap.add_argument("--prefix", default=os.environ.get("HOMEBREW_PREFIX", "/opt/homebrew"))
    ap.add_argument("--root", default=os.environ.get("GSTREAMER_1_0_ROOT_MSVC_X86_64", r"C:\Program Files\gstreamer\1.0\msvc_x86_64"))
    args = ap.parse_args()
    if args.platform == "self-test":
        return self_test()
    if not args.target:
        ap.error("the target directory is required")
    if args.platform == "macos":
        bundle_macos(Path(args.target), Path(args.prefix), args.main)
    else:
        bundle_windows(Path(args.target), Path(args.root))
    return 0


if __name__ == "__main__":
    sys.exit(main())
