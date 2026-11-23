// The lines above Rift's view of a pane (feature `history`, docs/remote-protocol.md): the phone
// scrolls them itself, with the platform's own scrolling, instead of asking the computer to move its
// view one step at a time. Every line has a number (the screen's `scrollback`); the view is one
// tall screen: numbered slots from `origin` down to the screen's first row, then the screen. A slot
// whose line has not come yet is blank until a `history` page brings it. The composite only grows at
// its bottom while the user reads above, so what they read never moves.

import type { ScreenState } from "./protocol/store";
import type { HistoryMsg, ScreenRun } from "./protocol/types";

/** Most lines kept above the screen; older ones leave the view. */
export const MAX_HISTORY_ROWS = 5000;
/** Lines one request asks for (the gateway's MAX_HISTORY_LINES). */
export const HISTORY_PAGE = 200;

const BLANK: ScreenRun[] = [];

/** What the phone holds of a pane's numbered lines in one numbering. */
export interface HistoryCache {
  epoch: number;
  cols: number;
  /** The number of the composite's first slot. */
  origin: number;
  /** The oldest line the computer still holds. */
  first: number;
  /** Lines held, by number. */
  lines: Map<number, ScreenRun[]>;
  /** Block rules of lines held, by number. */
  rules: Map<number, string>;
  /**
   * Lines held as the last screen frame showed them, not yet as the computer keeps them: a
   * program that redraws in place (an agent's spinner and input box) may have written over a
   * row between the frame and its leaving the screen. They show until a `history` page brings
   * the final text, and are asked for like missing ones.
   */
  unsure: Set<number>;
}

/**
 * The cache after `screen` replaced `prev`: a new one for another numbering or width (or none
 * without `scrollback`); else the rows that left the top of the screen kept under their numbers,
 * the lines the computer no longer holds dropped once the user follows the end (`following`), and
 * at most MAX_HISTORY_ROWS above the screen.
 */
export function nextCache(cache: HistoryCache | null, prev: ScreenState | null, screen: ScreenState | null, following: boolean): HistoryCache | null {
  const sb = screen?.scrollback;
  if (!screen || !sb || screen.alt) return null;
  let c = cache;
  if (!c || c.epoch !== sb.epoch || c.cols !== screen.cols) {
    c = { epoch: sb.epoch, cols: screen.cols, origin: sb.first, first: sb.first, lines: new Map(), rules: new Map(), unsure: new Set() };
  } else {
    c = { ...c, first: sb.first };
  }
  // rows of the screen before that are above this one now: they keep their numbers
  const was = prev?.scrollback;
  if (prev && was && was.epoch === sb.epoch && prev.cols === screen.cols) {
    for (let i = 0; i < prev.lines.length && was.top + i < sb.top; i++) {
      const n = was.top + i;
      // a page that already brought it is final
      if (c.lines.has(n) && !c.unsure.has(n)) continue;
      c.lines.set(n, prev.lines[i]!);
      c.unsure.add(n);
      const rule = prev.rules.get(i);
      if (rule) c.rules.set(n, rule);
      else c.rules.delete(n);
    }
  }
  // the top only moves down while the user follows the end: above what they read, nothing shifts
  let origin = following ? Math.max(c.origin, sb.first) : Math.min(c.origin, sb.top);
  origin = Math.min(Math.max(origin, sb.top - MAX_HISTORY_ROWS), sb.top);
  if (origin !== c.origin || following) {
    c.origin = origin;
    for (const n of c.lines.keys()) if (n < origin || n >= sb.top) c.lines.delete(n);
    for (const n of c.rules.keys()) if (n < origin || n >= sb.top) c.rules.delete(n);
    for (const n of c.unsure) if (n < origin || n >= sb.top) c.unsure.delete(n);
  }
  return c;
}

/**
 * The cache with a `history` page's lines; null when the page is of another numbering (start
 * again). Lines of `asked` past the page's last one (the computer has no such line) are blank
 * there, never asked for again; lines before its first (a page cut to fit) are asked for again.
 */
export function withPage(cache: HistoryCache, msg: HistoryMsg, asked?: { from: number; to: number }): HistoryCache | null {
  if (msg.epoch !== cache.epoch) return null;
  const c = { ...cache, first: msg.first };
  const rules = new Map(msg.rules ?? []);
  msg.lines.forEach((runs, i) => {
    const n = msg.from + i;
    if (n < c.origin) return;
    c.lines.set(n, runs);
    c.unsure.delete(n);
    const rule = rules.get(i);
    if (rule) c.rules.set(n, rule);
    else c.rules.delete(n);
  });
  if (asked) {
    const past = Math.max(msg.from + msg.lines.length, asked.from, c.first, c.origin);
    for (let n = past; n < asked.to; n++) if (!c.lines.has(n)) c.lines.set(n, BLANK);
  }
  return c;
}

/** The screen with the cache's slots above it, and how many there are. */
export function composite(cache: HistoryCache | null, screen: ScreenState): { screen: ScreenState; above: number } {
  const top = screen.scrollback?.top;
  if (!cache || top === undefined || top <= cache.origin) return { screen, above: 0 };
  const above = top - cache.origin;
  const lines: ScreenRun[][] = new Array(above + screen.lines.length);
  const rules = new Map<number, string>();
  for (let i = 0; i < above; i++) {
    const n = cache.origin + i;
    lines[i] = cache.lines.get(n) ?? BLANK;
    const rule = cache.rules.get(n);
    if (rule) rules.set(i, rule);
  }
  screen.lines.forEach((l, i) => (lines[above + i] = l));
  for (const [row, rule] of screen.rules) rules.set(above + row, rule);
  return {
    screen: {
      ...screen,
      lines,
      rules,
      cursor: screen.cursor ? [screen.cursor[0] + above, screen.cursor[1]] : null,
    },
    above,
  };
}

/**
 * The range to ask for so the composite's rows `from` … `to - 1` are all there: the missing lines
 * nearest the bottom of that range, at most HISTORY_PAGE of them ending at the last missing one;
 * null when none is missing (or the computer no longer holds them).
 */
export function missing(cache: HistoryCache, top: number, from: number, to: number): { from: number; to: number } | null {
  const lo = Math.max(cache.origin + Math.max(0, from), cache.first);
  const hi = Math.min(cache.origin + to, top);
  for (let n = hi - 1; n >= lo; n--) {
    if (!cache.lines.has(n) || cache.unsure.has(n)) {
      const end = n + 1;
      return { from: Math.max(cache.first, end - HISTORY_PAGE), to: end };
    }
  }
  return null;
}
