#!/usr/bin/env python3
"""The Visual C++ runtime a Windows release needs, and a check that nothing is missing.

    scripts/windows-runtime.py bundle STAGE_DIR   copy the runtime DLLs next to ferrite.exe
    scripts/windows-runtime.py check  STAGE_DIR   every DLL each program and DLL in STAGE_DIR
                                                  loads is in STAGE_DIR or part of Windows
    scripts/windows-runtime.py self-test

Rust programs built for Windows (the MSVC target), the JavaScript engine and the
GStreamer DLLs all load the Visual C++ runtime: VCRUNTIME140.dll, VCRUNTIME140_1.dll and
MSVCP140.dll. Those are not part of Windows. Every CI machine has them installed, so the
first releases started there and nowhere else: on a clean Windows the app stopped before
it began with "The code execution cannot proceed because VCRUNTIME140.dll was not found".

Microsoft allows the runtime to be shipped next to the program ("app-local"), from the
Visual Studio install that built it: `VC\\Redist\\MSVC\\<version>\\x64\\Microsoft.VC14*.CRT`.
Windows looks in the program's own folder first, so those copies are used even where an
older runtime is installed.

`check` reads each file's import tables (the normal and the delay-load one) and needs
every DLL named there to be either in STAGE_DIR (where Windows looks first, for the
program and for every DLL it loads) or a part of Windows: an API set (api-ms-win-*,
ext-ms-win-*) or a file in System32 that is not a redistributable. The redistributables
are named in REDISTRIBUTABLE, because CI's System32 has them and a clean Windows does not.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path
from typing import Callable

# DLLs found in System32 on a machine with Visual Studio or a redistributable installed,
# and not on a clean Windows. Matched against the start of the lower-case name.
REDISTRIBUTABLE = (
    "vcruntime", "msvcp", "concrt", "vccorlib", "vcomp", "vcamp", "mfc", "msvcr1",
    "ucrtbased", "vulkan-1",
)


# ------------------------------------------------------------------- reading PE files


class NotPE(ValueError):
    pass


def imports(data: bytes) -> list[str]:
    """The DLL names a PE file (exe or dll) imports, normal and delay-loaded, in order."""
    if data[:2] != b"MZ" or len(data) < 0x40:
        raise NotPE("no MZ header")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise NotPE("no PE signature")
    sections_count = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    if magic == 0x20B:  # PE32+
        count_at, dirs_at = optional + 108, optional + 112
    elif magic == 0x10B:  # PE32
        count_at, dirs_at = optional + 92, optional + 96
    else:
        raise NotPE(f"unknown optional header {magic:#x}")
    dir_count = struct.unpack_from("<I", data, count_at)[0]
    sections = []
    for i in range(sections_count):
        at = optional + optional_size + 40 * i
        vsize, vaddr, rsize, raddr = struct.unpack_from("<IIII", data, at + 8)
        sections.append((vaddr, max(vsize, rsize), raddr))

    def offset(rva: int) -> int:
        for vaddr, size, raddr in sections:
            if vaddr <= rva < vaddr + size:
                return rva - vaddr + raddr
        raise NotPE(f"address {rva:#x} is in no section")

    def name_at(rva: int) -> str:
        start = offset(rva)
        return data[start:data.index(b"\0", start)].decode("ascii", "replace")

    def directory(index: int) -> int:
        if index >= dir_count:
            return 0
        return struct.unpack_from("<I", data, dirs_at + 8 * index)[0]

    names: list[str] = []
    # Import directory (1): 20-byte descriptors, the name's address at +12.
    rva = directory(1)
    if rva:
        at = offset(rva)
        while True:
            name_rva = struct.unpack_from("<I", data, at + 12)[0]
            if name_rva == 0:
                break
            names.append(name_at(name_rva))
            at += 20
    # Delay-load import directory (13): 32-byte descriptors, the name's address at +4.
    rva = directory(13)
    if rva:
        at = offset(rva)
        while True:
            name_rva = struct.unpack_from("<I", data, at + 4)[0]
            if name_rva == 0:
                break
            names.append(name_at(name_rva))
            at += 32
    return names


# ------------------------------------------------------------------------- checking


def is_windows_part(name: str, in_system32: Callable[[str], bool]) -> bool:
    lower = name.lower()
    if lower.startswith(("api-ms-win-", "ext-ms-win-")):
        return True
    if lower.startswith(REDISTRIBUTABLE):
        return False
    return in_system32(lower)


def system32_has(name: str) -> bool:
    root = os.environ.get("SystemRoot", r"C:\Windows")
    return (Path(root) / "System32" / name).is_file()


def check(stage: Path, in_system32: Callable[[str], bool] = system32_has) -> int:
    present = {f.name.lower() for f in stage.iterdir() if f.is_file()}
    files = [f for f in sorted(stage.rglob("*")) if f.is_file() and f.suffix.lower() in (".exe", ".dll")]
    problems: list[str] = []
    from_windows: set[str] = set()
    for f in files:
        try:
            names = imports(f.read_bytes())
        except (NotPE, struct.error, ValueError) as e:
            problems.append(f"{f.relative_to(stage)}: unreadable ({e})")
            continue
        for name in names:
            if name.lower() in present:
                continue
            if is_windows_part(name, in_system32):
                from_windows.add(name.lower())
                continue
            problems.append(f"{f.relative_to(stage)} needs {name}, which is not in the package or part of Windows")
    print(f"from Windows: {', '.join(sorted(from_windows)) or 'nothing'}")
    for line in problems:
        print(line)
    print(f"checked {len(files)} files: {'ok' if not problems else f'{len(problems)} problems'}")
    return 1 if problems else 0


# ------------------------------------------------------------------------- bundling


def version_key(text: str) -> tuple[int, ...]:
    return tuple(int(p) for p in re.findall(r"\d+", text))


def find_crt_dir() -> Path:
    """The newest Microsoft.VC14*.CRT folder (x64) of the Visual Studio on this machine."""
    override = os.environ.get("FERRITE_VC_CRT_DIR")
    if override:
        return Path(override)
    roots: list[Path] = []
    redist = os.environ.get("VCToolsRedistDir")
    if redist:
        roots.append(Path(redist))
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
    if vswhere.is_file():
        out = subprocess.run(
            [str(vswhere), "-latest", "-products", "*", "-property", "installationPath"],
            check=True, capture_output=True, encoding="utf-8", errors="replace",
        ).stdout
        for line in out.splitlines():
            if line.strip():
                roots.extend(sorted((Path(line.strip()) / "VC" / "Redist" / "MSVC").glob("*"), key=lambda p: version_key(p.name), reverse=True))
    for root in roots:
        found = sorted(root.glob("x64/Microsoft.VC*.CRT"), key=lambda p: version_key(p.name), reverse=True)
        if found:
            return found[0]
    sys.exit("no Visual C++ redistributable folder (x64/Microsoft.VC*.CRT) found; set FERRITE_VC_CRT_DIR")


def bundle(stage: Path, crt: Path | None = None) -> None:
    crt = crt or find_crt_dir()
    dlls = sorted(crt.glob("*.dll"))
    if not any(d.name.lower() == "vcruntime140.dll" for d in dlls):
        sys.exit(f"{crt} has no vcruntime140.dll")
    for dll in dlls:
        shutil.copy2(dll, stage / dll.name)
    print(f"copied the Visual C++ runtime from {crt}: {', '.join(d.name for d in dlls)}")


# ---------------------------------------------------------------------------- tests


def fake_pe(dlls: list[str], delayed: list[str], pe32: bool = False) -> bytes:
    """A minimal PE file whose import tables name `dlls` and `delayed`."""
    base = 0x1000
    imports_size = 20 * (len(dlls) + 1)
    delay_size = 32 * (len(delayed) + 1)
    strings = b""
    name_rvas = []
    for name in dlls + delayed:
        name_rvas.append(base + imports_size + delay_size + len(strings))
        strings += name.encode() + b"\0"
    section = bytearray(imports_size + delay_size) + strings
    for i, rva in enumerate(name_rvas[:len(dlls)]):
        struct.pack_into("<I", section, 20 * i + 12, rva)
    for i, rva in enumerate(name_rvas[len(dlls):]):
        struct.pack_into("<I", section, imports_size + 32 * i + 4, rva)
    optional_size = (96 if pe32 else 112) + 16 * 8
    optional = bytearray(optional_size)
    struct.pack_into("<H", optional, 0, 0x10B if pe32 else 0x20B)
    count_at, dirs_at = (92, 96) if pe32 else (108, 112)
    struct.pack_into("<I", optional, count_at, 16)
    if dlls:
        struct.pack_into("<II", optional, dirs_at + 8 * 1, base, imports_size)
    if delayed:
        struct.pack_into("<II", optional, dirs_at + 8 * 13, base + imports_size, delay_size)
    header = bytearray(0x40)
    header[:2] = b"MZ"
    struct.pack_into("<I", header, 0x3C, 0x40)
    coff = struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, optional_size, 0x22)
    sect = bytearray(40)
    sect[:6] = b".idata"
    struct.pack_into("<IIII", sect, 8, len(section), base, len(section), 0x200)
    head = bytes(header) + b"PE\0\0" + coff + bytes(optional) + bytes(sect)
    return head + bytes(0x200 - len(head)) + bytes(section)


def self_test() -> int:
    import tempfile

    sample = fake_pe(["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-runtime-l1-1-0.dll"], ["d3d11.dll"])
    assert imports(sample) == ["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-runtime-l1-1-0.dll", "d3d11.dll"], imports(sample)
    assert imports(fake_pe(["USER32.dll"], [], pe32=True)) == ["USER32.dll"]
    assert imports(fake_pe([], [])) == []
    for bad in (b"", b"MZ" + bytes(100), b"\x7fELF" + bytes(100)):
        try:
            imports(bad)
        except (NotPE, struct.error):
            pass
        else:
            raise AssertionError("not a PE file, yet read")

    system = {"kernel32.dll", "user32.dll", "d3d11.dll", "vcruntime140.dll", "msvcp140.dll", "ws2_32.dll"}
    in_system32 = system.__contains__
    # CI's System32 has the runtime; a clean Windows does not, so it never counts.
    assert not is_windows_part("VCRUNTIME140.dll", in_system32)
    assert not is_windows_part("MSVCP140.dll", in_system32)
    assert not is_windows_part("vcruntime140_1.dll", lambda _: True)
    assert is_windows_part("KERNEL32.dll", in_system32)
    assert is_windows_part("api-ms-win-crt-heap-l1-1-0.dll", in_system32)
    assert not is_windows_part("libglib-2.0-0.dll", in_system32)

    with tempfile.TemporaryDirectory() as t:
        stage = Path(t) / "stage"
        (stage / "gst-extra").mkdir(parents=True)
        (stage / "ferrite.exe").write_bytes(fake_pe(["KERNEL32.dll", "VCRUNTIME140.dll", "MSVCP140.dll"], ["d3d11.dll"]))
        (stage / "gst-extra" / "gstopusparse.dll").write_bytes(fake_pe(["gstreamer-1.0-0.dll", "VCRUNTIME140.dll"], []))
        # Without the runtime and GStreamer beside it, the release this replaces: refused.
        assert check(stage, in_system32) == 1
        crt = Path(t) / "Microsoft.VC143.CRT"
        crt.mkdir()
        for name in ("msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"):
            (crt / name).write_bytes(fake_pe(["KERNEL32.dll"], []))
        bundle(stage, crt)
        assert (stage / "vcruntime140_1.dll").exists()
        # A plugin in a folder of its own finds its libraries in the program's folder.
        assert check(stage, in_system32) == 1, "gstreamer-1.0-0.dll is still missing"
        (stage / "gstreamer-1.0-0.dll").write_bytes(fake_pe(["KERNEL32.dll", "VCRUNTIME140.dll"], []))
        assert check(stage, in_system32) == 0
        # A file that is not a PE file is reported, not skipped.
        (stage / "broken.dll").write_bytes(b"not a dll")
        assert check(stage, in_system32) == 1
        # A runtime folder without the runtime is refused.
        try:
            bundle(stage, Path(t))
        except SystemExit:
            pass
        else:
            raise AssertionError("a folder without vcruntime140.dll was accepted")
    print("self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["bundle", "check", "self-test"])
    ap.add_argument("stage", nargs="?", help="the folder of ferrite.exe")
    args = ap.parse_args()
    if args.command == "self-test":
        return self_test()
    if not args.stage:
        ap.error("the stage folder is required")
    if args.command == "bundle":
        bundle(Path(args.stage))
        return 0
    return check(Path(args.stage))


if __name__ == "__main__":
    sys.exit(main())
