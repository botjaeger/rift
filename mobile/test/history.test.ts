import { describe, expect, it } from "vitest";
import { composite, HISTORY_PAGE, MAX_HISTORY_ROWS, missing, nextCache, withPage } from "../src/history";
import type { ScreenState } from "../src/protocol/store";
import type { HistoryMsg, ScreenRun } from "../src/protocol/types";

const row = (t: string): ScreenRun[] => [[t, null, null, 0]];
const text = (lines: ScreenRun[][]) => lines.map((l) => l[0]?.[0] ?? "");

/** A styled screen of `rows` rows whose row i shows line `top + i` (its text is its number). */
function screen(top: number, opts: { epoch?: number; first?: number; rows?: number; cols?: number; rule?: number } = {}): ScreenState {
  const rows = opts.rows ?? 4;
  return {
    cols: opts.cols ?? 80,
    rows,
    alt: false,
    cursor: [rows - 1, 2],
    lines: Array.from({ length: rows }, (_, i) => row(String(top + i))),
    fg: null,
    bg: null,
    rules: opts.rule === undefined ? new Map() : new Map([[opts.rule, "3fb950"]]),
    stale: false,
    ageMs: null,
    scrollback: { epoch: opts.epoch ?? 1, first: opts.first ?? 0, top },
  };
}

const page = (from: number, to: number, extra: Partial<HistoryMsg> = {}): HistoryMsg => ({
  t: "history",
  id: "h",
  pane: 1,
  epoch: 1,
  first: 0,
  from,
  lines: Array.from({ length: to - from }, (_, i) => row(String(from + i))),
  ...extra,
});

describe("the lines above the screen", () => {
  it("has none without scrollback or on the alternate screen", () => {
    expect(nextCache(null, null, { ...screen(10), scrollback: null }, true)).toBeNull();
    expect(nextCache(null, null, { ...screen(10), alt: true }, true)).toBeNull();
  });

  it("starts at the oldest line held, every slot blank until a page brings it", () => {
    const c = nextCache(null, null, screen(10, { first: 4 }), true)!;
    const { screen: s, above } = composite(c, screen(10, { first: 4 }));
    expect(above).toBe(6);
    expect(text(s.lines)).toEqual(["", "", "", "", "", "", "10", "11", "12", "13"]);
    // the cursor and the rules move down with the screen's rows
    expect(s.cursor).toEqual([9, 2]);
    expect(composite(c, screen(10, { first: 4, rule: 1 })).screen.rules.get(7)).toBe("3fb950");
    // the missing lines nearest the screen: one page ending at the last missing one
    expect(missing(c, 10, 0, 10)).toEqual({ from: 4, to: 10 });
    const full = withPage(c, page(4, 10, { first: 4, rules: [[1, "f85149"]] }))!;
    const after = composite(full, screen(10, { first: 4 })).screen;
    expect(text(after.lines).slice(0, 6)).toEqual(["4", "5", "6", "7", "8", "9"]);
    expect(after.rules.get(1)).toBe("f85149");
    expect(missing(full, 10, 0, 10)).toBeNull();
  });

  it("keeps the rows that leave the top of the screen under their numbers", () => {
    const a = screen(10);
    const c1 = nextCache(null, null, a, true)!;
    // two lines of output: rows 10 and 11 are above the screen now
    const b = screen(12);
    const c2 = nextCache(c1, a, b, true)!;
    expect(c2.lines.get(10)).toBe(a.lines[0]);
    expect(c2.lines.get(11)).toBe(a.lines[1]);
    // the rows still on the screen are not held twice
    expect(c2.lines.has(12)).toBe(false);
    // a jump of more than a screen leaves a gap: asked for
    const far = screen(30);
    const c3 = nextCache(c2, b, far, true)!;
    expect(missing(c3, 30, 0, 30)).toEqual({ from: 0, to: 30 });
    const big = screen(1000);
    expect(missing(nextCache(null, null, big, true)!, 1000, 0, 1000)).toEqual({ from: 1000 - HISTORY_PAGE, to: 1000 });
  });

  it("moves nothing the user reads: trimmed lines leave the top only while following the end", () => {
    const a = screen(10, { first: 0 });
    const c1 = withPage(nextCache(null, null, a, true)!, page(0, 10))!;
    // scrolled up: three lines trimmed and three added, the top stays where it was
    const b = screen(13, { first: 3 });
    const reading = nextCache(c1, a, b, false)!;
    expect(reading.origin).toBe(0);
    expect(composite(reading, b).above).toBe(13);
    // the computer no longer holds them: never asked for
    expect(missing({ ...reading, lines: new Map() }, 13, 0, 13)).toEqual({ from: 3, to: 13 });
    // back at the end: the gone lines leave
    const following = nextCache(reading, b, screen(13, { first: 3 }), true)!;
    expect(following.origin).toBe(3);
    expect(following.lines.has(2)).toBe(false);
    expect(composite(following, b).above).toBe(10);
  });

  it("starts again on another numbering or width, and drops a page of an old one", () => {
    const a = screen(10, { epoch: 1 });
    const c1 = withPage(nextCache(null, null, a, true)!, page(0, 10))!;
    const renumbered = nextCache(c1, a, screen(5, { epoch: 2 }), true)!;
    expect(renumbered.lines.size).toBe(0);
    expect(renumbered.epoch).toBe(2);
    const narrower = nextCache(c1, a, screen(10, { cols: 40 }), true)!;
    expect(narrower.lines.size).toBe(0);
    expect(withPage(c1, page(0, 3, { epoch: 9 }))).toBeNull();
  });

  it("holds at most MAX_HISTORY_ROWS above the screen", () => {
    const top = MAX_HISTORY_ROWS + 500;
    const c = nextCache(null, null, screen(top), true)!;
    expect(composite(c, screen(top)).above).toBe(MAX_HISTORY_ROWS);
    // a page reaching above the composite keeps only what it shows
    const p = withPage(c, page(400, 600))!;
    expect(p.lines.has(499)).toBe(false);
    expect(p.lines.has(500)).toBe(true);
  });

  it("asks only for the range in view", () => {
    const s = screen(500);
    const c = withPage(nextCache(null, null, s, true)!, page(300, 500))!;
    // rows 100 … 149 of the composite (lines 100 … 149) are missing
    expect(missing(c, 500, 100, 150)).toEqual({ from: 0, to: 150 });
    // rows already held: nothing
    expect(missing(c, 500, 350, 450)).toBeNull();
  });
});

