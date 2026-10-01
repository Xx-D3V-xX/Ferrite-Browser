#!/usr/bin/env python3
"""Render assets/icon/ferrite.svg into every raster the app and its installers use.

    pip install cairosvg pillow
    python3 scripts/make_icons.py            # rewrite the generated files
    python3 scripts/make_icons.py --check    # exit 1 if they are out of date

Outputs (all under assets/icon/, committed so a build never needs Python):
    ferrite-256.png    window icon embedded in the binary (crates/ferrite-ui)
    ferrite-512.png    Linux desktop / hicolor icon
    ferrite.ico        Windows: 16-256 px, embedded into ferrite.exe by build.rs
    ferrite.icns       macOS: the .app bundle icon
"""
import io
import sys
from pathlib import Path

import cairosvg
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ICON_DIR = ROOT / "assets" / "icon"
SVG = ICON_DIR / "ferrite.svg"


def render(size: int) -> Image.Image:
    png = cairosvg.svg2png(url=str(SVG), output_width=size, output_height=size)
    return Image.open(io.BytesIO(png)).convert("RGBA")


def encode(img: Image.Image, fmt: str, **kw) -> bytes:
    buf = io.BytesIO()
    img.save(buf, format=fmt, **kw)
    return buf.getvalue()


def build() -> dict[str, bytes]:
    out = {}
    out["ferrite-256.png"] = encode(render(256), "PNG", optimize=True)
    out["ferrite-512.png"] = encode(render(512), "PNG", optimize=True)
    # Pillow downsamples the largest frame for the smaller ICO sizes, so draw
    # each size from the vector instead: small sizes stay crisp.
    sizes = [16, 24, 32, 48, 64, 128, 256]
    big = render(256)
    out["ferrite.ico"] = encode(
        big, "ICO", sizes=[(s, s) for s in sizes],
        append_images=[render(s) for s in sizes if s != 256],
    )
    out["ferrite.icns"] = encode(render(1024), "ICNS")
    return out


def main() -> int:
    built = build()
    if "--check" in sys.argv:
        stale = [n for n, data in built.items()
                 if not (ICON_DIR / n).exists() or not _same(ICON_DIR / n, data)]
        if stale:
            print("stale icon files (run scripts/make_icons.py):", ", ".join(stale))
            return 1
        print("icon files are up to date")
        return 0
    for name, data in built.items():
        (ICON_DIR / name).write_bytes(data)
        print(f"wrote assets/icon/{name} ({len(data)} bytes)")
    return 0


def _same(path: Path, data: bytes) -> bool:
    # Rasterizer output can differ across cairo versions; compare decoded pixels
    # for the PNGs and fall back to byte equality for the container formats.
    if path.suffix == ".png":
        a = Image.open(path).convert("RGBA")
        b = Image.open(io.BytesIO(data)).convert("RGBA")
        return a.size == b.size and a.tobytes() == b.tobytes()
    return path.read_bytes() == data


if __name__ == "__main__":
    sys.exit(main())
