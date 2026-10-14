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
    (cp >= 0x1f900 && cp <= 0x1f9ff) ||
    (cp >= 0x20000 && cp <= 0x3fffd)
  );
}

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
      const w = isWide(ch.codePointAt(0)!) ? 2 : 1;
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
