#!/usr/bin/env python3
"""Generate the Rift app icon for macOS, Linux and Windows.

The design ("Prompt"): a dark rounded tile holding the prompt chevron in the
accent color and a block cursor, in the icon's own fixed colors (`COLORS`:
#232326 tile, #569cd6 chevron, #d4d4d4 cursor; DESIGN.md "App icon").

Usage:  python3 packaging/gen-icon.py      (needs Pillow)
Outputs (all under assets/icon/):
  rift-1024.png                   master
  rift.iconset/icon_{16,32,128,256,512}x{..}.png (+ @2x)
  rift.icns                       macOS bundle icon (macOS only, via iconutil)
  hicolor/<size>x<size>/apps/rift.png   Linux icon theme (.deb, AppImage, tarball)
  rift.ico                        Windows (16–256 px)
  rift-window.png                 256 px window icon embedded in the binary
  rift-menubar.png                36 px menu bar template icon embedded in the binary (macOS)
  variants/<name>.png             512 px app icon choices (`app_icon`), embedded in the
                                  binary and the macOS Dock tile plugin
and mobile/assets/icon.png (iOS, opaque) and adaptive-icon.png (Android foreground).
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
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

# The icon's own colours. They are not the theme's: the default theme follows
# Warp's colours (2026-10-16), and the icon stays the same in every theme
# (`welcome::logo` draws it with these too).
COLORS = {
    "surface": (0x23, 0x23, 0x26),
    "accent": (0x56, 0x9C, 0xD6),
    "cursor": (0xD4, 0xD4, 0xD4),
}

# The app icon choices (`app_icon`, Settings › Appearance › App icon): the
# same "Prompt" design in other colourways. Order and names match
# `rift_core::AppIcon`; "default" is COLORS.
VARIANTS = {
    "default": COLORS,
    "light": {"surface": (0xF5, 0xF5, 0xF7), "accent": (0x1F, 0x6F, 0xCC), "cursor": (0x2B, 0x2B, 0x2E),
              "hairline": (0, 0, 0, 26)},
    "mono": {"surface": (0x16, 0x16, 0x18), "accent": (0xF0, 0xF0, 0xF0), "cursor": (0xF0, 0xF0, 0xF0)},
    "ocean": {"surface": (0x0F, 0x2A, 0x44), "accent": (0x5E, 0xC4, 0xFF), "cursor": (0xE6, 0xF4, 0xFF)},
    "ember": {"surface": (0x2B, 0x16, 0x10), "accent": (0xFF, 0x8A, 0x3D), "cursor": (0xFF, 0xE2, 0xCC)},
    "forest": {"surface": (0x12, 0x26, 0x1B), "accent": (0x4F, 0xD1, 0x8B), "cursor": (0xDF, 0xF7, 0xE9)},
    "neon": {"surface": (0x1A, 0x0F, 0x2E), "accent": (0xFF, 0x4F, 0xD8), "cursor": (0x4F, 0xF0, 0xFF)},
}
# 512 px: the running app's Dock icon (embedded in the binary) and the Dock
# tile plugin's not-running icon (`packaging/docktile/`)
VARIANT_SIZE = 512

# Small icons (tabs, taskbars, title bars) waste the macOS margin: at or below
# this size the tile is drawn closer to the canvas edge.
SMALL = 48
SMALL_MARGIN = 40


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
            outline=colors.get("hairline", HAIRLINE),
            width=max(1, round(4 * scale)),
        )
    marks(d, p, scale, colors)
    return img.resize((size, size), Image.Resampling.LANCZOS)


def marks(d: ImageDraw.ImageDraw, p, scale: float, colors: dict[str, tuple[int, int, int]]) -> None:
    """The chevron and the cursor, through the design-grid mapping `p`."""

    def box(b: tuple[int, int, int, int]) -> list[float]:
        return [p(b[0]), p(b[1]), p(b[2]), p(b[3])]

    # chevron: a round-capped, round-joined polyline
    width = CHEVRON_WIDTH * scale
    pts = [(p(x), p(y)) for x, y in CHEVRON]
    d.line(pts, fill=(*colors["accent"], 255), width=round(width), joint="curve")
    r = width / 2
    for x, y in pts:
        d.ellipse([x - r, y - r, x + r, y + r], fill=(*colors["accent"], 255))

    d.rounded_rectangle(box(CURSOR), radius=CURSOR_RADIUS * scale, fill=(*colors["cursor"], 255))


def render_mobile(colors: dict[str, tuple[int, int, int]], tile_fraction: float, opaque: bool) -> Image.Image:
    """A 1024 px phone icon: the tile's area of the design grid mapped onto the central
    `tile_fraction` of the canvas, with no tile shape (the OS masks the corners). `opaque`
    fills the canvas with `surface` (iOS needs an icon without alpha); otherwise the
    background stays transparent (an Android adaptive-icon foreground)."""
    s = SIZE * SS
    tile = TILE[2] - TILE[0]
    scale = s * tile_fraction / tile
    off = (s - tile * scale) / 2 - TILE[0] * scale

    def p(v: float) -> float:
        return off + v * scale

    bg = (*colors["surface"], 255) if opaque else (0, 0, 0, 0)
    img = Image.new("RGBA", (s, s), bg)
    marks(ImageDraw.Draw(img), p, scale, colors)
    img = img.resize((SIZE, SIZE), Image.Resampling.LANCZOS)
    return img.convert("RGB") if opaque else img


# The menu bar icon ("Tile outline", chosen 2026-10-17 from the design
# canvas): the app icon as a line drawing on an 18 pt grid.
MENUBAR_TILE = (1.5, 2.0, 16.5, 16.0)
MENUBAR_TILE_RADIUS = 3.6
MENUBAR_TILE_STROKE = 1.5
MENUBAR_CHEVRON = [(5.0, 6.4), (7.8, 9.0), (5.0, 11.6)]
MENUBAR_CHEVRON_WIDTH = 1.7
MENUBAR_CURSOR = (9.6, 6.4, 12.4, 11.6)
MENUBAR_CURSOR_RADIUS = 0.5


def render_menubar(px: int) -> Image.Image:
    """The menu bar icon in black on transparent (macOS tints a template
    image): the tile's outline holding the chevron and the cursor, from the
    18 pt grid above onto a `px` square."""
    s = px * SS
    scale = s / 18

    def box(b: tuple[float, float, float, float]) -> list[float]:
        return [v * scale for v in b]

    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    black = (0, 0, 0, 255)
    # the outline straddles the tile's edge, as an SVG stroke does
    half = MENUBAR_TILE_STROKE / 2
    l, t, r, b = MENUBAR_TILE
    d.rounded_rectangle(
        box((l - half, t - half, r + half, b + half)),
        radius=(MENUBAR_TILE_RADIUS + half) * scale,
        outline=black,
        width=round(MENUBAR_TILE_STROKE * scale),
    )
    width = MENUBAR_CHEVRON_WIDTH * scale
    pts = [(x * scale, y * scale) for x, y in MENUBAR_CHEVRON]
    d.line(pts, fill=black, width=round(width), joint="curve")
    for x, y in pts:
        d.ellipse([x - width / 2, y - width / 2, x + width / 2, y + width / 2], fill=black)
    d.rounded_rectangle(box(MENUBAR_CURSOR), radius=MENUBAR_CURSOR_RADIUS * scale, fill=black)
    return img.resize((px, px), Image.Resampling.LANCZOS)


def main() -> int:
    colors = dict(COLORS)
    print("colors", {k: "#%02x%02x%02x" % v for k, v in colors.items()})

    OUT.mkdir(parents=True, exist_ok=True)
    cache: dict[int, Image.Image] = {}

    def at(px: int) -> Image.Image:
        if px not in cache:
            cache[px] = render(px, colors)
        return cache[px]

    at(SIZE).save(OUT / "rift-1024.png")
    at(256).save(OUT / "rift-window.png")
    print("wrote rift-1024.png, rift-window.png")
    # 18 pt at 2x; NSImage is given the point size (status_item.rs)
    render_menubar(36).save(OUT / "rift-menubar.png")
    print("wrote rift-menubar.png")

    iconset = OUT / "rift.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)
    for px in ICONSET_SIZES:
        at(px).save(iconset / f"icon_{px}x{px}.png")
        at(px * 2).save(iconset / f"icon_{px}x{px}@2x.png")
    print(f"wrote rift.iconset ({len(ICONSET_SIZES) * 2} files)")

    for px in HICOLOR_SIZES:
        d = OUT / "hicolor" / f"{px}x{px}" / "apps"
        d.mkdir(parents=True, exist_ok=True)
        at(px).save(d / "rift.png")
    print(f"wrote hicolor ({len(HICOLOR_SIZES)} sizes)")

    # every ICO frame is its own render (append_images), not a downscale
    largest = at(ICO_SIZES[-1])
    largest.save(
        OUT / "rift.ico",
        format="ICO",
        sizes=[(px, px) for px in ICO_SIZES],
        append_images=[at(px) for px in ICO_SIZES[:-1]],
    )
    print(f"wrote rift.ico ({len(ICO_SIZES)} sizes)")

    # the phone companion: iOS draws the whole square under its own mask; Android keeps
    # adaptive-icon foregrounds inside the central 2/3 (the safe zone)
    mobile = ROOT / "mobile" / "assets"
    mobile.mkdir(parents=True, exist_ok=True)
    render_mobile(colors, 1.0, opaque=True).save(mobile / "icon.png")
    render_mobile(colors, 0.62, opaque=False).save(mobile / "adaptive-icon.png")
    print("wrote mobile/assets/icon.png, adaptive-icon.png")

    variants = OUT / "variants"
    variants.mkdir(parents=True, exist_ok=True)
    for name, vcolors in VARIANTS.items():
        render(VARIANT_SIZE, vcolors).save(variants / f"{name}.png", optimize=True)
    print(f"wrote variants ({len(VARIANTS)} app icons)")

    icns = OUT / "rift.icns"
    if sys.platform == "darwin" and shutil.which("iconutil"):
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(icns)], check=True)
        print("wrote rift.icns")
    else:
        print("skipped rift.icns (iconutil is macOS-only)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
