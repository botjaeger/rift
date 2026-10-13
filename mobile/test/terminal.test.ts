import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { reduceScreen, type ScreenState } from "../src/protocol/store";
import { decodeServer, type ScreenMsg, type ScreenRun } from "../src/protocol/types";
import { fitFont, isWide, mix, rowSegments, segmentColors } from "../src/terminal";
import { colors, contrast } from "../src/ui/palette";

const fixture = (name: string) =>
  JSON.parse(
    readFileSync(fileURLToPath(new URL(`../../crates/rift-remote/fixtures/server/${name}`, import.meta.url)), "utf8"),
  ) as unknown;

const run = (text: string, fg: string | null = null, bg: string | null = null, flags = 0): ScreenRun => [text, fg, bg, flags];
const screen = (patch: Partial<ScreenMsg>): ScreenMsg => ({
  t: "screen",
  pane: 12,
  cols: 80,
  rows: 3,
  alt: true,
  cursor: null,
  full: true,
  lines: [
    [0, [run("top")]],
    [1, []],
    [2, [run(":")]],
  ],
  ...patch,
});

describe("reduceScreen", () => {
  it("a full screen replaces; every row is present", () => {
    const s = reduceScreen(null, screen({}))!;
    expect(s).toMatchObject({ cols: 80, rows: 3, alt: true, cursor: null });
    expect(s.lines).toEqual([[run("top")], [], [run(":")]]);
  });

  it("a diff patches only its rows and keeps the others by reference (memoized rows)", () => {
    const a = reduceScreen(null, screen({}))!;
    const b = reduceScreen(a, screen({ full: false, cursor: [2, 1], lines: [[1, [run("x")]]] }))!;
    expect(b.lines).toEqual([[run("top")], [run("x")], [run(":")]]);
    expect(b.lines[0]).toBe(a.lines[0]);
    expect(b.lines[2]).toBe(a.lines[2]);
    expect(b.lines[1]).not.toBe(a.lines[1]);
    expect(b.cursor).toEqual([2, 1]);
  });

  it("an empty row in a diff clears it", () => {
    const a = reduceScreen(null, screen({}))!;
    expect(reduceScreen(a, screen({ full: false, lines: [[0, []]] }))!.lines[0]).toEqual([]);
  });

  it("a size change (sent as full) starts over; a diff without one before starts from blank rows", () => {
    const a = reduceScreen(null, screen({}))!;
    const b = reduceScreen(a, screen({ rows: 2, cols: 40, lines: [[0, [run("a")]], [1, [run("b")]]] }))!;
    expect(b).toMatchObject({ rows: 2, cols: 40, lines: [[run("a")], [run("b")]] });
    const c = reduceScreen(null, screen({ full: false, lines: [[1, [run("only")]]] }))!;
    expect(c.lines).toEqual([[], [run("only")], []]);
  });

  it("screen_end clears", () => {
    expect(reduceScreen(reduceScreen(null, screen({})), { t: "screen_end", pane: 12 })).toBeNull();
  });

  it("applies the fixture", () => {
    const msg = decodeServer(fixture("screen.json")) as ScreenMsg;
    const s = reduceScreen(null, msg) as ScreenState;
    expect(s.lines).toHaveLength(24);
    expect(s.lines[6]).toEqual([run("  PID USER"), run("      CPU%", "1e1e1e", "e5c07b")]);
    expect(s.cursor).toEqual([23, 1]);
  });
});

describe("fitFont", () => {
  it("fits 80 columns on a phone between 7 and 14 pt", () => {
    // 390 pt wide: 390 / (80 × 0.6) = 8.1 pt
    expect(fitFont(374, 80)).toEqual({ fontSize: 7.5, scroll: false });
    expect(fitFont(374, 40)).toEqual({ fontSize: 14, scroll: false });
    expect(fitFont(1000, 80)).toEqual({ fontSize: 14, scroll: false });
  });

  it("scrolls sideways when even 7 pt is too wide", () => {
    expect(fitFont(374, 120)).toEqual({ fontSize: 7, scroll: true });
    expect(fitFont(374, 300)).toEqual({ fontSize: 7, scroll: true });
  });

  it("before layout: the largest size, no scroll", () => {
    expect(fitFont(0, 80)).toEqual({ fontSize: 14, scroll: false });
  });
});

describe("rowSegments", () => {
  it("without the cursor: the runs as they are", () => {
    expect(rowSegments([run("ab", "ff0000")], null)).toEqual([{ text: "ab", fg: "ff0000", bg: null, flags: 0, cursor: false }]);
  });

  it("splits out the cell under the cursor", () => {
    const segs = rowSegments([run("top", null, null, 1), run(" -", "aaaaaa")], 1);
    expect(segs.map((s) => [s.text, s.cursor])).toEqual([
      ["t", false],
      ["o", true],
      ["p", false],
      [" -", false],
    ]);
    expect(segs[1]).toMatchObject({ flags: 1 });
  });

  it("the cursor at the first column of a later run", () => {
    expect(rowSegments([run("ab"), run("cd")], 2).map((s) => [s.text, s.cursor])).toEqual([
      ["ab", false],
      ["c", true],
      ["d", false],
    ]);
  });

  it("a cursor past the row's text gets blanks up to it", () => {
    expect(rowSegments([run(":")], 4).map((s) => [s.text, s.cursor])).toEqual([
      [":", false],
      ["   ", false],
      [" ", true],
    ]);
    expect(rowSegments([], 0).map((s) => [s.text, s.cursor])).toEqual([[" ", true]]);
  });

  it("a wide character takes two columns", () => {
    expect(isWide("漢".codePointAt(0)!)).toBe(true);
    expect(isWide("a".codePointAt(0)!)).toBe(false);
    // 漢 is columns 0–1, so column 2 is "x"
    expect(rowSegments([run("漢xy")], 2).map((s) => [s.text, s.cursor])).toEqual([
      ["漢", false],
      ["x", true],
      ["y", false],
    ]);
    expect(rowSegments([run("漢x")], 1).map((s) => [s.text, s.cursor])).toEqual([
      ["漢", true],
      ["x", false],
    ]);
  });
});

describe("colours", () => {
  const defaults = { fg: colors.text, bg: colors.bg };

  it("null is the terminal default; a set background is drawn", () => {
    expect(segmentColors({ text: "a", fg: null, bg: null, flags: 0, cursor: false }, defaults)).toEqual({
      color: colors.text,
      backgroundColor: undefined,
    });
    expect(segmentColors({ text: "a", fg: "1e1e1e", bg: "e5c07b", flags: 0, cursor: false }, defaults)).toEqual({
      color: "#1e1e1e",
      backgroundColor: "#e5c07b",
    });
  });

  it("the cursor is inverse", () => {
    expect(segmentColors({ text: "a", fg: null, bg: null, flags: 0, cursor: true }, defaults)).toEqual({
      color: colors.bg,
      backgroundColor: colors.text,
    });
  });

  it("dim is halfway to the background", () => {
    expect(mix("#ffffff", "#000000")).toBe("#808080");
    expect(segmentColors({ text: "a", fg: "ffffff", bg: "000000", flags: 8, cursor: false }, defaults).color).toBe("#808080");
  });

  it("the default text on the default background reads (4.5:1)", () => {
    expect(contrast(defaults.fg, defaults.bg)).toBeGreaterThanOrEqual(4.5);
  });
});
