// "Load earlier output": lines before a block's tail (`Block.output` starts at line `dropped`),
// fetched 500 at a time with `output` (docs/remote-protocol.md, Output paging) and kept per
// block above the tail, at most MAX_EARLIER_LINES of them. Pure; the fleet holds the state.

import { MAX_OUTPUT_LINES, type Block } from "./protocol/types";

export const MAX_EARLIER_LINES = 5_000;
/** Requests for one press at most (an answer stops at 64 KiB, so a page can take several). */
export const MAX_REQUESTS_PER_LOAD = 10;

/** The earlier lines held for one block: lines `from` … `end - 1` as `text`. */
export interface EarlierOutput {
  from: number;
  end: number;
  text: string;
  /** Lines before `from` are gone from the computer's scrollback. */
  gone: boolean;
  loading: boolean;
  error: string | null;
}

export function emptyEarlier(block: Block): EarlierOutput {
  return { from: block.dropped, end: block.dropped, text: "", gone: false, loading: false, error: null };
}

/** Lines in `text` (a trailing line break ends the last line; an unfinished last line counts). */
export function countLines(text: string): number {
  let n = 0;
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) n++;
  return text !== "" && !text.endsWith("\n") ? n + 1 : n;
}

/** The first `n` lines of `text`. */
function firstLines(text: string, n: number): string {
  let idx = -1;
  for (let k = 0; k < n; k++) {
    idx = text.indexOf("\n", idx + 1);
    if (idx === -1) return text;
  }
  return text.slice(0, idx + 1);
}

/**
 * Held lines that no longer touch the tail are of no use: the block's `dropped` moved past
 * `end` (a running block's tail slides on), so lines `end` … `dropped - 1` are between.
 */
export function gapLines(block: Block, e: EarlierOutput | undefined): number {
  return e ? Math.max(0, block.dropped - e.end) : 0;
}

/** Earlier lines not held here (still on the computer or gone): for the button's count. */
export function unloadedLines(block: Block, e: EarlierOutput | undefined): number {
  if (!e) return block.dropped;
  return (e.gone ? 0 : e.from) + gapLines(block, e);
}

export function heldLines(e: EarlierOutput | undefined): number {
  return e ? e.end - e.from : 0;
}

/**
 * The next request of "Load earlier output": first the lines between the held ones and a tail
 * that moved on, else the 500 before the first held line. Null when there is nothing to load
 * or no room left (MAX_EARLIER_LINES).
 */
export function nextRequest(
  block: Block,
  e: EarlierOutput | undefined,
): { from: number; limit: number; fill: boolean } | null {
  const room = MAX_EARLIER_LINES - heldLines(e);
  if (room <= 0) return null;
  const gap = gapLines(block, e);
  if (e && gap > 0) return { from: e.end, limit: Math.min(MAX_OUTPUT_LINES, gap, room), fill: true };
  if (e?.gone) return null;
  const top = e ? e.from : block.dropped;
  if (top <= 0) return null;
  const limit = Math.min(MAX_OUTPUT_LINES, top, room);
  return { from: top - limit, limit, fill: false };
}

export type PageResult =
  | { ok: true; from: number; text: string }
  | { ok: false; code: string; message: string };

export type RangeResult =
  | { ok: true; from: number; text: string; gone: boolean }
  | { ok: false; code: string; message: string };

/**
 * Fetches lines `from` … `end - 1` page by page (an answer stops at 64 KiB, so it may hold fewer
 * lines than asked). When the computer no longer holds the first ones, the result starts later
 * (`gone`); the lines it returns always reach `end`, or the result is an error.
 */
export async function fetchRange(
  fetchPage: (from: number, limit: number) => Promise<PageResult>,
  from: number,
  end: number,
): Promise<RangeResult> {
  let pos = from;
  let start: number | null = null;
  let text = "";
  let gone = false;
  for (let i = 0; i < MAX_REQUESTS_PER_LOAD && pos < end; i++) {
    const r = await fetchPage(pos, end - pos);
    if (!r.ok) return r;
    if (r.from > pos) {
      // lines vanished from the computer's scrollback: what was fetched before them is cut off
      gone = true;
      start = null;
      text = "";
    }
    if (r.from >= end) {
      pos = end;
      break;
    }
    let got = firstLines(r.text, end - r.from);
    const n = countLines(got);
    if (n === 0) break;
    if (start === null) start = r.from;
    // An answer cut at 64 KiB inside a line ends without a line break; that line counts as one
    // (the next page starts after it), so it is ended here or it would glue to the next line.
    // Lines always follow a fetched range (more pages, the held lines or the block's tail).
    if (!got.endsWith("\n")) got += "\n";
    text += got;
    pos = r.from + n;
  }
  if (pos < end) return { ok: false, code: "incomplete", message: "The computer sent fewer lines than asked." };
  return { ok: true, from: start ?? end, text: start === null ? "" : text, gone };
}

/** `e` with a fetched range merged in: before the held lines, or after them (`fill`). */
export function mergeRange(
  block: Block,
  e: EarlierOutput | undefined,
  req: { from: number; limit: number; fill: boolean },
  r: { from: number; text: string; gone: boolean },
): EarlierOutput {
  const cur = e ?? emptyEarlier(block);
  const end = req.from + req.limit;
  if (req.fill) {
    // the lines right after the held ones are gone: keep only what reaches the tail
    if (r.gone) return { ...cur, from: r.from, end, text: r.text, gone: true, loading: false, error: null };
    return { ...cur, end, text: cur.text + r.text, loading: false, error: null };
  }
  return { ...cur, from: r.from, text: r.text + cur.text, gone: r.gone, loading: false, error: null };
}
