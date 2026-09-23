#!/usr/bin/env python3
"""Generate the Shift app icon (macOS .icns / .iconset, Linux hicolor PNGs).

Palette is read from crates/shift-core/src/theme/dark-default.toml (read-only)
so the icon accent always matches the shipped default theme.

Usage:  python3 packaging/gen-icon.py
Outputs (all under assets/icon/):
  shift-1024.png
  shift.iconset/icon_{16,32,128,256,512}x{..}.png (+ @2x)
  shift.icns                       (macOS only, via iconutil)
  hicolor/<size>x<size>/apps/shift.png
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
THEME = ROOT / "crates" / "shift-core" / "src" / "theme" / "dark-default.toml"
OUT = ROOT / "assets" / "icon"

# Canvas is rendered at SS x the final size and downsampled — cheap antialiasing
# for the rounded-rect edge and the chevron.
SIZE = 1024
SS = 4

# Background gradient (the app chrome reads darker than the terminal bg).
BG_TOP = (0x1B, 0x1F, 0x2A)
BG_BOTTOM = (0x0B, 0x0D, 0x13)

ICONSET_SIZES = [16, 32, 128, 256, 512]
HICOLOR_SIZES = [16, 32, 48, 64, 128, 256, 512]

CHEVRON = "❯"  # U+276F heavy right-pointing angle quotation mark ornament

# The repo's own JetBrains Mono comes first: it is the typeface the terminal
# renders with, and using it keeps the generator reproducible off macOS.
# System fonts are fallbacks; if none carries the glyph the chevron is drawn
# as a polygon instead (see draw_chevron_polygon).
FONT_CANDIDATES = [
    str(ROOT / "assets" / "fonts" / "JetBrainsMono-Bold.ttf"),
    str(ROOT / "assets" / "fonts" / "JetBrainsMono-SemiBold.ttf"),
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/System/Library/Fonts/Apple Symbols.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
]


def read_accent() -> tuple[int, int, int]:
    """Pull `accent = "#rrggbb"` out of the default theme; fall back to spec value."""
    default = (0x56, 0x9C, 0xD6)
    try:
        text = THEME.read_text(encoding="utf-8")
    except OSError:
        return default
    m = re.search(r'^\s*accent\s*=\s*"#([0-9a-fA-F]{6})"', text, re.MULTILINE)
    if not m:
        return default
    h = m.group(1)
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16))


def gradient(size: int) -> Image.Image:
    """Vertical dark gradient with a soft accent bloom in the upper third."""
    img = Image.new("RGB", (1, size))
    px = img.load()
    for y in range(size):
        t = y / max(size - 1, 1)
        px[0, y] = tuple(
            round(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3)
        )
    return img.resize((size, size), Image.Resampling.BILINEAR)


# A codepoint no font maps: whatever it renders IS that font's .notdef glyph.
# Comparing against it is the only reliable "does this font have the glyph?"
# oracle without fontTools -- a bbox/non-empty check happily accepts tofu.
NOTDEF_PROBE = "\ue83f"


def probe_font(px: int) -> ImageFont.FreeTypeFont | None:
    """First candidate font that really carries the chevron (not .notdef, not blank)."""
    for path in FONT_CANDIDATES:
        if not os.path.exists(path):
            continue
        try:
            font = ImageFont.truetype(path, px)
        except OSError:
            continue
        try:
            glyph = font.getmask(CHEVRON, mode="L")
            notdef = font.getmask(NOTDEF_PROBE, mode="L")
        except Exception:
            continue
        if glyph.getbbox() is None:
            continue  # blank
        if glyph.size == notdef.size and bytes(glyph) == bytes(notdef):
            continue  # rendered the font's .notdef box
        return font
    return None


def draw_chevron_polygon(draw: ImageDraw.ImageDraw, cx: int, cy: int, h: int, color):
    """Fallback ❯ drawn as a stroked polyline — used when no font carries the glyph."""
    arm = h / 2.0
    width = max(2, int(h * 0.17))
    dx = arm * 0.62
    pts = [(cx - dx * 0.55, cy - arm), (cx + dx, cy), (cx - dx * 0.55, cy + arm)]
    draw.line(pts, fill=color, width=width, joint="curve")
    r = width / 2.0
    for p in pts:
        draw.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=color)


def render(size: int, accent: tuple[int, int, int]) -> Image.Image:
    """Draw the icon at `size` px, supersampled by SS and downsampled at the end."""
    s = size * SS

    inset = round(s * 0.09)
    # macOS squircle-ish: ~22.4% of the CARD's width, not of the full canvas.
    radius = round((s - 2 * inset) * 0.2237)
    card_box = [inset, inset, s - inset - 1, s - inset - 1]

    # The card is painted fully opaque across the whole canvas and only masked
    # to the rounded square at the very end. Downsampling colour and alpha
    # separately keeps the transparent surround from bleeding a black fringe
    # into the corners.
    card = gradient(s).convert("RGBA")

    # accent bloom behind the glyph
    bloom = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    bd = ImageDraw.Draw(bloom)
    br = round(s * 0.30)
    bcx, bcy = s // 2, round(s * 0.42)
    bd.ellipse([bcx - br, bcy - br, bcx + br, bcy + br], fill=(*accent, 64))
    card = Image.alpha_composite(card, bloom.filter(ImageFilter.GaussianBlur(s * 0.09)))

    d = ImageDraw.Draw(card)

    # --- chevron -------------------------------------------------------------
    # Centred on the glyph's RENDERED ink box, never on font.getbbox(): side
    # bearings differ per font and would push it visibly off centre.
    glyph_h = round(s * 0.34)
    cx, cy = s // 2, round(s * 0.42)
    font = probe_font(round(glyph_h * 1.55))
    if font is not None:
        pad = round(glyph_h * 1.2)
        layer = Image.new("RGBA", (s, s), (0, 0, 0, 0))
        ImageDraw.Draw(layer).text((pad, pad), CHEVRON, font=font, fill=(*accent, 255))
        box = layer.getbbox()
        ink = layer.crop(box)
        card.alpha_composite(ink, (cx - ink.width // 2, cy - ink.height // 2))
    else:
        draw_chevron_polygon(d, cx, cy, glyph_h, (*accent, 255))

    # --- prompt markers: three faint accent dashes ----------------------------
    dash_y = round(s * 0.735)
    dash_h = max(1, round(s * 0.030))
    gap = round(s * 0.045)
    widths = [round(s * 0.19), round(s * 0.125), round(s * 0.070)]
    alphas = [150, 105, 65]
    total = sum(widths) + gap * (len(widths) - 1)
    x = (s - total) / 2.0
    for w, a in zip(widths, alphas):
        d.rounded_rectangle(
            [x, dash_y, x + w, dash_y + dash_h],
            radius=dash_h / 2.0,
            fill=(*accent, a),
        )
        x += w + gap

    # --- hairline top edge highlight ----------------------------------------
    edge = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(
        card_box,
        radius=radius,
        outline=(255, 255, 255, 26),
        width=max(1, round(s * 0.0035)),
    )
    card = Image.alpha_composite(card, edge)

    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle(card_box, radius=radius, fill=255)

    rgb = card.convert("RGB").resize((size, size), Image.Resampling.LANCZOS)
    alpha = mask.resize((size, size), Image.Resampling.LANCZOS)
    out = rgb.convert("RGBA")
    out.putalpha(alpha)
    return out


def main() -> int:
    accent = read_accent()
    print(f"accent #{accent[0]:02x}{accent[1]:02x}{accent[2]:02x} (from {THEME.relative_to(ROOT)})")
    print("chevron source:", "font" if probe_font(200) else "polygon fallback")

    OUT.mkdir(parents=True, exist_ok=True)
    master = render(SIZE, accent)
    master_path = OUT / "shift-1024.png"
    master.save(master_path)
    print("wrote", master_path.relative_to(ROOT))

    def scaled(px: int) -> Image.Image:
        if px == SIZE:
            return master
        # Re-render (not downscale) above 256 so small sizes stay crisp.
        return render(px, accent) if px <= 256 else master.resize(
            (px, px), Image.Resampling.LANCZOS
        )

    iconset = OUT / "shift.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)
    for px in ICONSET_SIZES:
        scaled(px).save(iconset / f"icon_{px}x{px}.png")
        scaled(px * 2).save(iconset / f"icon_{px}x{px}@2x.png")
    print("wrote", iconset.relative_to(ROOT), f"({len(ICONSET_SIZES) * 2} files)")

    for px in HICOLOR_SIZES:
        p = OUT / "hicolor" / f"{px}x{px}" / "apps"
        p.mkdir(parents=True, exist_ok=True)
        scaled(px).save(p / "shift.png")
    print("wrote", (OUT / "hicolor").relative_to(ROOT), f"({len(HICOLOR_SIZES)} sizes)")

    icns = OUT / "shift.icns"
    if sys.platform == "darwin" and shutil.which("iconutil"):
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(icns)], check=True
        )
        print("wrote", icns.relative_to(ROOT))
    else:
        print("skipped shift.icns (iconutil is macOS-only)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
