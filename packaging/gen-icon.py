#!/usr/bin/env python3
"""Generate the Shift app icon for macOS, Linux and Windows.

The design ("Prompt"): a dark rounded tile holding the prompt chevron in the
accent color and a block cursor. Colors are read from the default theme
(crates/shift-core/src/theme/dark-default.toml): `surface` for the tile,
`accent` for the chevron, `cursor` for the cursor.

Usage:  python3 packaging/gen-icon.py      (needs Pillow)
Outputs (all under assets/icon/):
  shift-1024.png                   master
  shift.iconset/icon_{16,32,128,256,512}x{..}.png (+ @2x)
  shift.icns                       macOS bundle icon (macOS only, via iconutil)
  hicolor/<size>x<size>/apps/shift.png   Linux icon theme (.deb, AppImage, tarball)
  shift.ico                        Windows (16–256 px)
  shift-window.png                 256 px window icon embedded in the binary
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
THEME = ROOT / "crates" / "shift-core" / "src" / "theme" / "dark-default.toml"
OUT = ROOT / "assets" / "icon"

SIZE = 1024
SS = 4  # supersampling factor for antialiased edges

ICONSET_SIZES = [16, 32, 128, 256, 512]
HICOLOR_SIZES = [16, 32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]

# Geometry on the 1024 design grid (macOS icon grid: 824 px tile, 100 px margin).
TILE = (100, 100, 924, 924)
TILE_RADIUS = 185
CHEVRON = [(336, 360), (506, 512), (336, 664)]
CHEVRON_WIDTH = 96
CURSOR = (596, 372, 736, 652)
CURSOR_RADIUS = 16
HAIRLINE = (255, 255, 255, 26)  # white at 10 %

# Small icons (tabs, taskbars, title bars) waste the macOS margin: at or below
# this size the tile is drawn closer to the canvas edge.
SMALL = 48
SMALL_MARGIN = 40


def token(name: str, default: str) -> tuple[int, int, int]:
    """`name = "#rrggbb"` from the default theme, else `default`."""
    try:
        text = THEME.read_text(encoding="utf-8")
    except OSError:
        text = ""
    m = re.search(rf'^\s*{name}\s*=\s*"#([0-9a-fA-F]{{6}})', text, re.MULTILINE)
    h = m.group(1) if m else default.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16))


def render(size: int, colors: dict[str, tuple[int, int, int]]) -> Image.Image:
    """Draw the icon at `size` px (supersampled, then downsampled)."""
    s = size * SS
    if size <= SMALL:
        # map [margin, 1024 - margin] of the design grid onto the canvas
        lo = SMALL_MARGIN
        scale = s / (SIZE - 2 * (TILE[0] - lo))
        off = -(TILE[0] - lo) * scale
    else:
        scale = s / SIZE
        off = 0.0

    def p(v: float) -> float:
        return off + v * scale

    def box(b: tuple[int, int, int, int]) -> list[float]:
        return [p(b[0]), p(b[1]), p(b[2]), p(b[3])]

    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle(box(TILE), radius=TILE_RADIUS * scale, fill=(*colors["surface"], 255))
    if size > SMALL:
        d.rounded_rectangle(
            box(TILE),
            radius=TILE_RADIUS * scale,
            outline=HAIRLINE,
            width=max(1, round(4 * scale)),
        )

    # chevron: a round-capped, round-joined polyline
    width = CHEVRON_WIDTH * scale
    pts = [(p(x), p(y)) for x, y in CHEVRON]
    d.line(pts, fill=(*colors["accent"], 255), width=round(width), joint="curve")
    r = width / 2
    for x, y in pts:
        d.ellipse([x - r, y - r, x + r, y + r], fill=(*colors["accent"], 255))

    d.rounded_rectangle(box(CURSOR), radius=CURSOR_RADIUS * scale, fill=(*colors["cursor"], 255))
    return img.resize((size, size), Image.Resampling.LANCZOS)


def main() -> int:
    colors = {
        "surface": token("surface", "#232326"),
        "accent": token("accent", "#569cd6"),
        "cursor": token("cursor", "#d4d4d4"),
    }
    print("colors", {k: "#%02x%02x%02x" % v for k, v in colors.items()})

    OUT.mkdir(parents=True, exist_ok=True)
    cache: dict[int, Image.Image] = {}

    def at(px: int) -> Image.Image:
        if px not in cache:
            cache[px] = render(px, colors)
        return cache[px]

    at(SIZE).save(OUT / "shift-1024.png")
    at(256).save(OUT / "shift-window.png")
    print("wrote shift-1024.png, shift-window.png")

    iconset = OUT / "shift.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)
    for px in ICONSET_SIZES:
        at(px).save(iconset / f"icon_{px}x{px}.png")
        at(px * 2).save(iconset / f"icon_{px}x{px}@2x.png")
    print(f"wrote shift.iconset ({len(ICONSET_SIZES) * 2} files)")

    for px in HICOLOR_SIZES:
        d = OUT / "hicolor" / f"{px}x{px}" / "apps"
        d.mkdir(parents=True, exist_ok=True)
        at(px).save(d / "shift.png")
    print(f"wrote hicolor ({len(HICOLOR_SIZES)} sizes)")

    # every ICO frame is its own render (append_images), not a downscale
    largest = at(ICO_SIZES[-1])
    largest.save(
        OUT / "shift.ico",
        format="ICO",
        sizes=[(px, px) for px in ICO_SIZES],
        append_images=[at(px) for px in ICO_SIZES[:-1]],
    )
    print(f"wrote shift.ico ({len(ICO_SIZES)} sizes)")

    icns = OUT / "shift.icns"
    if sys.platform == "darwin" and shutil.which("iconutil"):
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(icns)], check=True)
        print("wrote shift.icns")
    else:
        print("skipped shift.icns (iconutil is macOS-only)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
