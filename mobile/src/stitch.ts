// A full-screen program's rows stitched into one page the phone scrolls natively (feature
// `stitch`, docs/remote-protocol.md). The computer scrolls the program, joins its screens, and
// sends the page in `page` messages: this holds what came, and builds the screens the view draws
// (the rows above the scrolling area, the page, the rows below).

import type { ScreenState } from "./protocol/store";
import type { PageMsg, ScreenRun } from "./protocol/types";

export interface StitchPage {
  epoch: number;
  /** Screen rows `[0]..[1]`, columns `[2]..[3]` that scroll. */
  area: [number, number, number, number];
  top: number;
  bottom: number;
  /** The row the program shows first in its area. */
  at: number;
  /** Rows by page index (top…bottom − 1); a hole while a chunk is on its way. */
  rows: Map<number, ScreenRun[]>;
  /** The program's start and end were reached. */
  ends: [boolean, boolean];
  state: PageMsg["state"];
}

/** A page message merged in: another epoch starts the page over. */
export function mergePage(page: StitchPage | null, msg: PageMsg): StitchPage {
  const fresh = !page || page.epoch !== msg.epoch;
  const rows = fresh ? new Map<number, ScreenRun[]>() : new Map(page.rows);
  msg.lines.forEach((l, i) => rows.set(msg.from + i, l));
  // rows outside the page's span now (a page never shrinks, but keep it honest)
  for (const k of rows.keys()) if (k < msg.top || k >= msg.bottom) rows.delete(k);
  return { epoch: msg.epoch, area: msg.area, top: msg.top, bottom: msg.bottom, at: msg.at, rows, ends: msg.ends, state: msg.state };
}

/** Every row of the page has come (no chunk on its way). */
export function pageComplete(page: StitchPage): boolean {
  return page.rows.size === page.bottom - page.top;
}

/** A page the view can scroll: an area, and every row there. */
export function pageUsable(page: StitchPage | null): page is StitchPage {
  return !!page && page.state !== "flat" && page.area[1] - page.area[0] > 0 && page.bottom > page.top && pageComplete(page);
}

const BLANK: ScreenRun[] = [];

function rowText(runs: readonly ScreenRun[]): string {
  return runs.map((r) => r[0]).join("");
}

/** A page row as a screen row: its columns from the area's first on, the panel beside it blank. */
function padded(row: ScreenRun[], firstCol: number): ScreenRun[] {
  return firstCol > 0 ? [[" ".repeat(firstCol), null, null, 0], ...row] : row;
}

/**
 * The three screens the view draws for `page` over `live` (the program's screen now): its rows
 * above the area, the page (one tall screen), its rows below. Each has the live screen's width,
 * so the three draw at one font size.
 */
export function pageScreens(page: StitchPage, live: ScreenState): { above: ScreenState; body: ScreenState; below: ScreenState } {
  return { ...pageFixed(page, live), body: pageBody(page, live) };
}

function part(live: ScreenState, lines: ScreenRun[][]): ScreenState {
  return { ...live, rows: lines.length, lines, cursor: null, rules: new Map(), scrollback: null };
}

/** The program's rows above the area and below it, as they are on `live` now. */
export function pageFixed(page: StitchPage, live: ScreenState): { above: ScreenState; below: ScreenState } {
  const [r0, r1] = page.area;
  return { above: part(live, live.lines.slice(0, r0)), below: part(live, live.lines.slice(r1)) };
}

/**
 * The page as one tall screen: of `live` it takes only the width and colours, so the view builds
 * it once per page (its rows keep their identity, and drawn rows stay drawn), not per live frame.
 */
export function pageBody(page: StitchPage, live: ScreenState): ScreenState {
  const c0 = page.area[2];
  const body: ScreenRun[][] = [];
  for (let i = page.top; i < page.bottom; i++) body.push(padded(page.rows.get(i) ?? BLANK, c0));
  return part(live, body);
}

/**
 * The live screen still shows the page where the program is: its area's rows, cut to the area's
 * columns, are the page's rows from `at`. False once the program drew something else there (new
 * output): the page no longer says where the program is.
 */
export function liveMatches(page: StitchPage, live: ScreenState): boolean {
  const [r0, r1, c0, c1] = page.area;
  if (live.rows < r1) return false;
  for (let r = r0; r < r1; r++) {
    const want = rowText(page.rows.get(page.at + (r - r0)) ?? BLANK).trimEnd();
    const got = [...rowText(live.lines[r] ?? BLANK)].slice(c0, c1).join("").trimEnd();
    if (want !== got) return false;
  }
  return true;
}

/**
 * After a `stopped` answer (a screen the computer could not join, or a page the program no longer
 * shows as it was: it laid a message out anew), how long before another `page_ahead` may go out.
 * The computer then goes on from the page, or starts a new one where the program is.
 */
export const PAGE_RETRY_MS = 1500;

/** A `page_ahead` may go out for `page` at `now` (`stoppedAt`: when its last `stopped` came). */
export function mayAskPage(page: StitchPage | null, now: number, stoppedAt: number): boolean {
  if (!page || page.state === "ok") return true;
  return page.state === "stopped" && now - stoppedAt >= PAGE_RETRY_MS;
}

/**
 * `stopped` answers in a row that are asked again without the finger (the next scroll still asks):
 * a page that keeps stopping does not keep moving the program on the computer.
 */
export const PAGE_AUTO_RETRIES = 2;

/**
 * Screens of rows kept ready past the view toward an edge whose end the program has not reached.
 * After the first page the computer holds the pane tall and brings several hundred rows in one
 * job of a few seconds, so rows are asked for well before the finger gets there (a flick covers
 * a few screens a second), and again as each batch comes in.
 */
export const PAGE_READ_AHEAD = 8;

/**
 * A first page that has not come this long after it was asked for (the computer answers within
 * two seconds), or whose request was refused: for this long the wheel scrolls the program, as
 * for one that cannot be stitched. A drag always moves something.
 */
export const PAGE_FIRST_MS = 3000;
export const PAGE_WHEEL_MS = 10_000;

/**
 * The wheel scrolls the program although it could be stitched: no page yet, and the request for
 * one was refused (`refusedAt`) within PAGE_WHEEL_MS, or went out (`askedAt`) PAGE_FIRST_MS ago.
 */
export function wheelInstead(now: number, askedAt: number | null, refusedAt: number): boolean {
  if (now - refusedAt < PAGE_WHEEL_MS) return true;
  return askedAt !== null && now - askedAt >= PAGE_FIRST_MS;
}

/**
 * Which way a `page_ahead` goes for a scroller at `y` (`viewH` tall, `contentH` of content):
 * within PAGE_READ_AHEAD screens of an edge whose end the program has not reached.
 */
export function edgeToAsk(page: StitchPage, y: number, viewH: number, contentH: number): "up" | "down" | null {
  const reach = viewH * PAGE_READ_AHEAD;
  if (y < reach && !page.ends[0]) return "up";
  if (contentH - y - viewH < reach && !page.ends[1]) return "down";
  return null;
}

/**
 * How many rows a `page_ahead` asks for: the first, one screen (the page shows sooner, the rest
 * is read ahead); then a few screens of the area, within the bound.
 */
export function aheadRows(page: StitchPage | null, liveRows: number): number {
  if (!page) return Math.min(500, Math.max(20, liveRows));
  return Math.min(500, Math.max(20, (page.area[1] - page.area[0]) * 3));
}
