// Layout of the terminal screen (docs/remote-protocol.md, Terminal screen) for the pane's
// Terminal view: the font size that fits the width, and each row's runs split at the cursor.
// Pure; src/ui/TerminalView.tsx draws them.

import { RUN_DIM, type ScreenRun } from "./protocol/types";

export const MIN_FONT = 7;
export const MAX_FONT = 14;
/** A monospace cell is about this wide per point of font size (Menlo, Roboto Mono, Courier). */
export const CELL_RATIO = 0.6;
export const LINE_RATIO = 1.25;

/**
 * The font size (7 to 14 pt, in half points) at which `cols` cells fit `width`; `scroll` when
 * even 7 pt is too wide and the screen scrolls sideways.
 */
export function fitFont(width: number, cols: number): { fontSize: number; scroll: boolean } {
  if (width <= 0 || cols <= 0) return { fontSize: MAX_FONT, scroll: false };
  const fit = Math.floor((width / (cols * CELL_RATIO)) * 2) / 2;
  if (fit < MIN_FONT) return { fontSize: MIN_FONT, scroll: true };
  return { fontSize: Math.min(MAX_FONT, fit), scroll: false };
}

/**
 * Whether code point `cp` takes two cells (East Asian wide and fullwidth, emoji). The gateway
 * sends a wide character as one character of a run; this only places the cursor after it.
 */
export function isWide(cp: number): boolean {
  return (
    (cp >= 0x1100 && cp <= 0x115f) ||
    (cp >= 0x2e80 && cp <= 0x303e) ||
    (cp >= 0x3041 && cp <= 0x33ff) ||
    (cp >= 0x3400 && cp <= 0x4dbf) ||
    (cp >= 0x4e00 && cp <= 0x9fff) ||
    (cp >= 0xa000 && cp <= 0xa4cf) ||
    (cp >= 0xac00 && cp <= 0xd7a3) ||
    (cp >= 0xf900 && cp <= 0xfaff) ||
    (cp >= 0xfe30 && cp <= 0xfe4f) ||
    (cp >= 0xff00 && cp <= 0xff60) ||
    (cp >= 0xffe0 && cp <= 0xffe6) ||
    (cp >= 0x1f300 && cp <= 0x1f64f) ||
    (cp >= 0x1f680 && cp <= 0x1f6ff) ||
    (cp >= 0x1f7e0 && cp <= 0x1f7eb) ||
    (cp >= 0x1f900 && cp <= 0x1f9ff) ||
    (cp >= 0x1fa70 && cp <= 0x1faff) ||
    (cp >= 0x20000 && cp <= 0x3fffd) ||
    EMOJI_WIDE.has(cp)
  );
}

/** Emoji below U+1F000 that the East Asian Width table makes two cells wide (✅ ❌ ⭐ ⚡ ☕ …). */
const EMOJI_WIDE = new Set([
  0x231a, 0x231b, 0x23e9, 0x23ea, 0x23eb, 0x23ec, 0x23f0, 0x23f3, 0x25fd, 0x25fe, 0x2614, 0x2615,
  0x2648, 0x2649, 0x264a, 0x264b, 0x264c, 0x264d, 0x264e, 0x264f, 0x2650, 0x2651, 0x2652, 0x2653,
  0x267f, 0x2693, 0x26a1, 0x26aa, 0x26ab, 0x26bd, 0x26be, 0x26c4, 0x26c5, 0x26ce, 0x26d4, 0x26ea,
  0x26f2, 0x26f3, 0x26f5, 0x26fa, 0x26fd, 0x2705, 0x270a, 0x270b, 0x2728, 0x274c, 0x274e, 0x2753,
  0x2754, 0x2755, 0x2757, 0x2795, 0x2796, 0x2797, 0x27b0, 0x27bf, 0x2b1b, 0x2b1c, 0x2b50, 0x2b55,
  0x1f004, 0x1f0cf, 0x1f18e, 0x1f191, 0x1f192, 0x1f193, 0x1f194, 0x1f195, 0x1f196, 0x1f197,
  0x1f198, 0x1f199, 0x1f19a, 0x1f201, 0x1f21a, 0x1f22f,
]);

/** Whether `cp` takes no cell of its own: combining marks, variation selectors, zero-width joiners, skin tones. */
export function isZeroWidth(cp: number): boolean {
  return (
    (cp >= 0x0300 && cp <= 0x036f) ||
    (cp >= 0x200b && cp <= 0x200f) ||
    (cp >= 0x20d0 && cp <= 0x20ff) ||
    (cp >= 0xfe00 && cp <= 0xfe0f) ||
    (cp >= 0xfe20 && cp <= 0xfe2f) ||
    (cp >= 0x1f3fb && cp <= 0x1f3ff) ||
    (cp >= 0xe0100 && cp <= 0xe01ef)
  );
}

/** The cells a character takes: 0 (combining), 1, or 2 (wide). */
export const charWidth = (cp: number): 0 | 1 | 2 => (isZeroWidth(cp) ? 0 : isWide(cp) ? 2 : 1);

export interface Segment {
  text: string;
  fg: string | null;
  bg: string | null;
  flags: number;
  /** The cell under the cursor, drawn inverse. */
  cursor: boolean;
}

/**
 * A row's runs as segments, the cell at `cursorCol` (null: none on this row) split out as
 * its own segment. A cursor past the row's text (blank cells at the end are not sent) gets
 * blanks up to it.
 */
export function rowSegments(runs: readonly ScreenRun[], cursorCol: number | null): Segment[] {
  if (cursorCol === null) return runs.map(([text, fg, bg, flags]) => ({ text, fg, bg, flags, cursor: false }));
  const out: Segment[] = [];
  let col = 0;
  let placed = false;
  for (const [text, fg, bg, flags] of runs) {
    if (placed) {
      out.push({ text, fg, bg, flags, cursor: false });
      continue;
    }
    let before = "";
    let at: string | null = null;
    let after = "";
    for (const ch of text) {
      if (at !== null) {
        after += ch;
        continue;
      }
      const w = charWidth(ch.codePointAt(0)!);
      if (cursorCol >= col && cursorCol < col + w) at = ch;
      else before += ch;
      col += w;
    }
    if (before) out.push({ text: before, fg, bg, flags, cursor: false });
    if (at !== null) {
      out.push({ text: at, fg, bg, flags, cursor: true });
      placed = true;
    }
    if (after) out.push({ text: after, fg, bg, flags, cursor: false });
  }
  if (!placed) {
    if (cursorCol > col) out.push({ text: " ".repeat(cursorCol - col), fg: null, bg: null, flags: 0, cursor: false });
    out.push({ text: " ", fg: null, bg: null, flags: 0, cursor: true });
  }
  return out;
}

/** A stretch of a row placed on the cell grid: `col` and `cells` count cells, so every piece sits where the terminal puts it. */
export interface Piece extends Segment {
  col: number;
  cells: number;
  /** One wide character (drawn centred in its two cells), not a run of narrow ones. */
  wide: boolean;
}

/**
 * A row's segments laid on the cell grid. The font cannot be trusted to give an emoji or a CJK
 * character exactly two cells, so a wide character is a piece of its own (2 cells, drawn centred
 * in a box 2 cells wide) and runs of narrow characters are pieces at their column; a combining
 * mark joins the character before it. A piece's pixels are `col * cell` and `cells * cell`.
 */
export function rowPieces(segments: readonly Segment[]): Piece[] {
  const out: Piece[] = [];
  let col = 0;
  for (const seg of segments) {
    let narrow: Piece | null = null;
    for (const ch of seg.text) {
      const w = charWidth(ch.codePointAt(0)!);
      if (w === 0) {
        // joins the previous character of this segment, else the previous piece
        const prev = narrow ?? out.at(-1);
        if (prev) prev.text += ch;
        continue;
      }
      if (w === 2) {
        narrow = null;
        out.push({ ...seg, text: ch, col, cells: 2, wide: true });
      } else if (narrow) {
        narrow.text += ch;
        narrow.cells += 1;
      } else {
        narrow = { ...seg, text: ch, col, cells: 1, wide: false };
        out.push(narrow);
      }
      col += w;
    }
  }
  return out;
}

/** `#rrggbb` halfway between `fg` and `bg` (dim text). */
export function mix(fg: string, bg: string, t = 0.5): string {
  const ch = (hex: string, i: number) => parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16);
  const parts = [0, 1, 2].map((i) => Math.round(ch(fg, i) * (1 - t) + ch(bg, i) * t));
  return `#${parts.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

/**
 * The colours a segment is drawn in: the run's (`rrggbb`) or the terminal defaults, dim
 * halfway to the background, and swapped under the cursor.
 */
export function segmentColors(
  s: Segment,
  defaults: { fg: string; bg: string },
): { color: string; backgroundColor: string | undefined } {
  let fg = s.fg ? `#${s.fg}` : defaults.fg;
  const bg = s.bg ? `#${s.bg}` : defaults.bg;
  if (s.flags & RUN_DIM) fg = mix(fg, bg);
  if (s.cursor) return { color: bg, backgroundColor: fg };
  return { color: fg, backgroundColor: s.bg ? bg : undefined };
}

/**
 * Whether the pane shows its terminal screen instead of blocks: only while a full-screen
 * program (vim, htop, less: the alternate screen) runs, on a gateway with `screen`. A screen
 * kept, stale, across a reconnect still shows; when the program quits the blocks come back.
 */
export function showsTerminal(screen: { alt: boolean } | null, screenFeature: boolean): boolean {
  return screenFeature && !!screen?.alt;
}

/**
 * Whether the control-key row shows: always under a full-screen program (it needs the keys),
 * else as Settings › Output › Show keys above the command line says.
 */
export function showsKeyRow(showKeys: boolean, fullScreen: boolean): boolean {
  return fullScreen || showKeys;
}

/** The full-screen program's name for the banner: the running command's first word, or the pane's job. */
export function screenProgram(running: string | null, job: string | null): string {
  const word = running?.trim().split(/\s+/)[0];
  const name = word ? (word.split("/").at(-1) ?? word) : null;
  return name || job || "A program";
}

/**
 * The screen's default text and ground colours as `#rrggbb`: the desktop theme's when the gateway
 * sent them (`fg` / `bg`), else the phone's `fallback`.
 */
export function screenDefaults(
  screen: { fg: string | null; bg: string | null } | null,
  fallback: { fg: string; bg: string },
): { fg: string; bg: string } {
  return { fg: screen?.fg ? `#${screen.fg}` : fallback.fg, bg: screen?.bg ? `#${screen.bg}` : fallback.bg };
}
