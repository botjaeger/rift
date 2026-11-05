import { describe, expect, it } from "vitest";
import { BatchedStore } from "../src/protocol/batch";
import { reduceScreen, screenShapeSame, type ScreenState } from "../src/protocol/store";
import type { ScreenMsg, ScreenRun } from "../src/protocol/types";
import { rowKeys } from "../src/terminal";

const row = (t: string): ScreenRun[] => [[t, null, null, 0]];
const msg = (lines: [number, ScreenRun[]][], extra: Partial<ScreenMsg> = {}): ScreenMsg => ({
  t: "screen",
  pane: 1,
  cols: 10,
  rows: 4,
  alt: true,
  cursor: null,
  full: false,
  lines,
  ...extra,
});

describe("a scrolled screen redraws only the rows it brings in", () => {
  const first = reduceScreen(null, msg([[0, row("a")], [1, row("b")], [2, row("c")], [3, row("d")]], { full: true }))!;
  // a scroll by one: every row comes again, three of them with what a row held before
  const next = reduceScreen(first, msg([[0, row("b")], [1, row("c")], [2, row("d")], [3, row("e")]]))!;

  it("keeps the arrays of rows whose content comes back, wherever they move", () => {
    expect(next.lines[0]).toBe(first.lines[1]);
    expect(next.lines[1]).toBe(first.lines[2]);
    expect(next.lines[2]).toBe(first.lines[3]);
    expect(next.lines[3]).not.toBe(first.lines[0]);
    expect(next.lines[3]).toEqual(row("e"));
  });

  it("keys moved rows as they were, so their views move rather than draw again", () => {
    const before = rowKeys(first.lines);
    const after = rowKeys(next.lines);
    expect(after.slice(0, 3)).toEqual(before.slice(1, 4));
    expect(before).not.toContain(after[3]);
  });

  it("keys equal rows of one screen apart (blank ones)", () => {
    const blank = reduceScreen(null, msg([[0, row("")], [1, row("")], [2, row("x")], [3, row("")]], { full: true }))!;
    const keys = rowKeys(blank.lines);
    expect(new Set(keys).size).toBe(4);
  });

  it("tells a frame that changed only rows from one the rest of the app reads", () => {
    expect(screenShapeSame(first, next)).toBe(true);
    const wider = reduceScreen(first, msg([], { cols: 12 }));
    expect(screenShapeSame(first, wider)).toBe(false);
    const primary = reduceScreen(first, msg([], { alt: false }));
    expect(screenShapeSame(first, primary)).toBe(false);
    expect(screenShapeSame(null, first)).toBe(false);
    expect(screenShapeSame(null, null)).toBe(true);
    const held = { ...first, ageMs: 4000 } as ScreenState;
    expect(screenShapeSame(first, held)).toBe(false);
  });
});

describe("a quiet write", () => {
  it("changes what peek reads, tells no listener, and goes out with the next publish", () => {
    let t = 0;
    const s = new BatchedStore({ n: 0, m: 0 }, 100, () => t);
    let heard = 0;
    s.subscribe(() => heard++);
    s.quiet((v) => ({ ...v, n: 1 }));
    expect(s.peek().n).toBe(1);
    expect(s.getSnapshot().n).toBe(0);
    expect(heard).toBe(0);
    t = 1000;
    s.update((v) => ({ ...v, m: 1 }));
    s.flush();
    expect(heard).toBe(1);
    expect(s.getSnapshot()).toEqual({ n: 1, m: 1 });
  });
});
