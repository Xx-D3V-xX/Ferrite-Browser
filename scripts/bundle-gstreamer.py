#!/usr/bin/env python3
"""Puts a GStreamer next to the Ferrite app, for a release of the `media` build.

    scripts/bundle-gstreamer.py macos   Ferrite.app [--prefix /opt/homebrew]
    scripts/bundle-gstreamer.py windows STAGE_DIR   [--root "C:\\Program Files\\gstreamer\\1.0\\msvc_x86_64"]
    scripts/bundle-gstreamer.py list    macos|windows     (the plugin files Servo loads, one a line)
    scripts/bundle-gstreamer.py self-test

The layout is the one the engine insists on. On macOS and Windows Servo (`servo.rs`,
`media_platform::init`) loads a fixed list of plugin files, by path, from one directory,
and if any one of them will not load it logs the error and calls `exit(1)`: the app ends
at start-up and says nothing. The list is `gstreamer_plugin_lists/` in the `servo` crate,
and this script reads it from there (through `cargo metadata`) so it cannot drift from
the engine in use.

    macOS    Contents/MacOS/lib/lib<name>.dylib     the plugins (Servo's directory)
             Contents/Frameworks/*.dylib            the libraries they need, @rpath names
    Windows  STAGE_DIR/<name>.dll                   the plugins (next to ferrite.exe)
             STAGE_DIR/*.dll                        the libraries they need

`crates/ferrite-servo/src/bundle.rs` finds this layout at start-up and keeps GStreamer from
scanning anywhere else (a scan of the same directory would register each plugin twice,
which Servo counts as a failure; a scan of the machine's own GStreamer would mix two).

On macOS every file also has the library search paths it was built with removed: they are
searched before the bundle's, and on a machine that has Homebrew's GStreamer they made the
app load Homebrew's libraries next to its own.

`self-test` checks the parts that need no Mac or Windows. The CI jobs `bundle-smoke-macos`
and `bundle-smoke-windows` run the real thing against the real engine's plugin list.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


# ----------------------------------------------------- the plugins Servo insists on


def read_plugin_list(path: Path) -> list[str]:
    """The names in one of Servo's `*.rs.in` lists (a Rust array of strings with `//`
    comment lines)."""
    names: list[str] = []
    for line in path.read_text().splitlines():
        text = line.strip()
        if text.startswith("//"):
            continue
        names += re.findall(r'"([^"]+)"', text)
    return names


def find_servo_plugin_lists() -> Path:
    """The `gstreamer_plugin_lists` directory of the `servo` crate this workspace builds."""
    override = os.environ.get("FERRITE_SERVO_PLUGIN_LISTS")
    if override:
        return Path(override)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--all-features"],
        # UTF-8, not the machine's default (Windows': cp1252 chokes on package metadata).
        cwd=REPO, check=True, capture_output=True, encoding="utf-8", errors="replace",
    ).stdout
    for package in json.loads(out)["packages"]:
        if package["name"] == "servo":
            lists = Path(package["manifest_path"]).parent / "gstreamer_plugin_lists"
            if lists.is_dir():
                return lists
    sys.exit("could not find the servo crate's gstreamer_plugin_lists (set FERRITE_SERVO_PLUGIN_LISTS)")


def plugin_files(platform: str, lists_dir: Path | None = None) -> list[str]:
    """The plugin files Servo loads on `platform` ("macos" or "windows"), by file name."""
    lists = lists_dir or find_servo_plugin_lists()
    names = read_plugin_list(lists / "common.rs.in") + read_plugin_list(lists / f"{platform}.rs.in")
    if platform == "macos":
        return [f"lib{n}.dylib" for n in names]
    if platform == "windows":
        return [f"{n}.dll" for n in names]
    raise ValueError(platform)


# -------------------------------------------------------------- reading Mach-O files


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
    result = subprocess.run(cmd, check=True, capture_output=True, encoding="utf-8", errors="replace", **kw)
    return result.stdout


def brew_dirs(prefix: Path) -> tuple[list[Path], list[Path]]:
    """(plugin directories, library directories) under a Homebrew prefix."""
    plugin_dirs = sorted(prefix.glob("lib/gstreamer-1.0")) + sorted(prefix.glob("opt/*/lib/gstreamer-1.0"))
    lib_dirs = [prefix / "lib"] + sorted(prefix.glob("opt/*/lib"))
    return plugin_dirs, lib_dirs


def bundle_macos(app: Path, prefix: Path, main_name: str = "ferrite", lists_dir: Path | None = None) -> None:
    frameworks = app / "Contents" / "Frameworks"
    plugins_out = app / "Contents" / "MacOS" / "lib"  # where Servo looks
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
    needed = plugin_files("macos", lists_dir)
    missing = [n for n in needed if n not in found]
    if missing:
        sys.exit(f"Servo loads these GStreamer plugins and they are not under {prefix}: {missing}")

    main = app / "Contents" / "MacOS" / main_name
    todo: list[tuple[Path, Path, str]] = []  # (source, destination, rpath to add)
    for name in needed:
        # Contents/MacOS/lib/<plugin>: the libraries are two folders up, in Frameworks.
        todo.append((found[name], plugins_out / name, "@loader_path/../../Frameworks"))

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
    print(f"bundled {len(needed)} plugins and {len(copied)} libraries into {app}")


# -------------------------------------------------------------------------- Windows


def bundle_windows(stage: Path, root: Path, lists_dir: Path | None = None) -> None:
    plugin_dir = root / "lib" / "gstreamer-1.0"
    needed = plugin_files("windows", lists_dir)
    missing = [n for n in needed if not (plugin_dir / n).exists()]
    if missing:
        sys.exit(f"Servo loads these GStreamer plugins and they are not under {plugin_dir}: {missing}")
    stage.mkdir(parents=True, exist_ok=True)
    # Servo loads the plugins from the directory of ferrite.exe, and Windows finds a
    # plugin's libraries there too.
    for name in needed:
        shutil.copy2(plugin_dir / name, stage)
    libs = 0
    for dll in (root / "bin").glob("*.dll"):
        shutil.copy2(dll, stage)
        libs += 1
    print(f"bundled {len(needed)} plugins and {libs} libraries into {stage}")


# ----------------------------------------------------------------------------- tests

SAMPLE_OTOOL = """/opt/homebrew/opt/gstreamer/lib/gstreamer-1.0/libgstisomp4.dylib:
\t/opt/homebrew/opt/gstreamer/lib/gstreamer-1.0/libgstisomp4.dylib (compatibility version 0.0.0, current version 0.0.0)
\t/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib (compatibility version 8401.0.0, current version 8401.5.0)
\t@rpath/libgstbase-1.0.0.dylib (compatibility version 2401.0.0, current version 2401.1.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)
\t/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 3000.0.0)
"""

SAMPLE_COMMON = """// The list of plugin libraries themselves.
[
// gstreamer
"gstcoreelements","gstnice",
// gst-plugins-base
"gstapp",
"gstplayback",
]
"""
SAMPLE_MACOS = """// The format of this file is intended to be include!()able.
[
// gst-plugins-good
"gstosxaudio",
]
"""
SAMPLE_WINDOWS = """[
// gst-plugins-bad
"gstwasapi"
]
"""


def self_test() -> int:
    import tempfile

    assert parse_otool(SAMPLE_OTOOL, "libgstisomp4.dylib") == [
        "/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib",
        "@rpath/libgstbase-1.0.0.dylib",
    ]
    assert rpath_name("/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib") == "@rpath/libglib-2.0.0.dylib"
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
    assert parse_rpaths(sample) == ["/opt/homebrew/lib", "/opt/homebrew/Cellar/gstreamer/1.26.0/lib"]
    assert parse_rpaths("") == []
    with tempfile.TemporaryDirectory() as t:
        d = Path(t)
        (d / "libfoo.dylib").write_bytes(b"")
        assert resolve_dependency("@rpath/libfoo.dylib", [d], d) == d / "libfoo.dylib"
        assert resolve_dependency("@rpath/libbar.dylib", [d], d) is None
        assert resolve_dependency("@loader_path/libfoo.dylib", [], d) == d / "libfoo.dylib"
        assert resolve_dependency(str(d / "libfoo.dylib"), [], d) == d / "libfoo.dylib"

        # Servo's lists: comments skipped, names read, file names made per platform.
        lists = d / "lists"
        lists.mkdir()
        (lists / "common.rs.in").write_text(SAMPLE_COMMON)
        (lists / "macos.rs.in").write_text(SAMPLE_MACOS)
        (lists / "windows.rs.in").write_text(SAMPLE_WINDOWS)
        assert read_plugin_list(lists / "common.rs.in") == ["gstcoreelements", "gstnice", "gstapp", "gstplayback"]
        assert plugin_files("macos", lists) == [
            "libgstcoreelements.dylib", "libgstnice.dylib", "libgstapp.dylib",
            "libgstplayback.dylib", "libgstosxaudio.dylib",
        ]
        assert plugin_files("windows", lists) == [
            "gstcoreelements.dll", "gstnice.dll", "gstapp.dll", "gstplayback.dll", "gstwasapi.dll",
        ]

        # A fake Windows install: exactly the listed plugins and every DLL land beside the exe.
        root = d / "gst"
        (root / "lib" / "gstreamer-1.0").mkdir(parents=True)
        (root / "bin").mkdir()
        for n in plugin_files("windows", lists) + ["gstunrelated.dll"]:
            (root / "lib" / "gstreamer-1.0" / n).write_bytes(b"x")
        (root / "bin" / "gstreamer-1.0-0.dll").write_bytes(b"x")
        (root / "bin" / "gst-launch-1.0.exe").write_bytes(b"x")
        stage = d / "stage"
        bundle_windows(stage, root, lists)
        assert (stage / "gstwasapi.dll").exists() and (stage / "gstcoreelements.dll").exists()
        assert not (stage / "gstunrelated.dll").exists()
        assert (stage / "gstreamer-1.0-0.dll").exists()
        assert not (stage / "gst-launch-1.0.exe").exists()
        # A plugin Servo needs that is not installed stops the bundle: Servo would exit(1).
        (root / "lib" / "gstreamer-1.0" / "gstnice.dll").unlink()
        try:
            bundle_windows(d / "stage2", root, lists)
        except SystemExit as stop:
            assert "gstnice.dll" in str(stop), stop
        else:
            raise AssertionError("a missing plugin was not refused")

        # A dangling plugin link (Homebrew leaves one for libnice) is not a plugin.
        brew = d / "brew"
        plug = brew / "lib" / "gstreamer-1.0"
        plug.mkdir(parents=True)
        (plug / "libgstcoreelements.dylib").write_bytes(b"x")
        (plug / "libgstnice.dylib").symlink_to(d / "nowhere.dylib")
        assert [f.name for f in plug.glob("*.dylib") if f.exists()] == ["libgstcoreelements.dylib"]
    print("self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["macos", "windows", "list", "self-test"])
    ap.add_argument("target", nargs="?", help="Ferrite.app (macos), the stage directory (windows) or the platform (list)")
    ap.add_argument("--main", default="ferrite", help="name of the executable in Contents/MacOS (the smoke test uses another)")
    ap.add_argument("--prefix", default=os.environ.get("HOMEBREW_PREFIX", "/opt/homebrew"))
    ap.add_argument("--root", default=os.environ.get("GSTREAMER_1_0_ROOT_MSVC_X86_64", r"C:\Program Files\gstreamer\1.0\msvc_x86_64"))
    args = ap.parse_args()
    if args.command == "self-test":
        return self_test()
    if not args.target:
        ap.error("the target is required")
    if args.command == "list":
        if args.target not in ("macos", "windows"):
            ap.error("list takes macos or windows")
        print("\n".join(plugin_files(args.target)))
    elif args.command == "macos":
        bundle_macos(Path(args.target), Path(args.prefix), args.main)
    else:
        bundle_windows(Path(args.target), Path(args.root))
    return 0


if __name__ == "__main__":
    sys.exit(main())
