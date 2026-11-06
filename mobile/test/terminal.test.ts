import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { reduceScreen, type ScreenState } from "../src/protocol/store";
import { decodeServer, type ScreenMsg, type ScreenRun } from "../src/protocol/types";
import { textRoom, charWidth, atEnd, cursorScrollY, fitArea, fitFont, lastTextRow, isWide, mix, rowPieces, rowSegments, screenDefaults, segmentColors } from "../src/terminal";
import { DARK_DEFAULT, contrast, paletteFrom } from "../src/ui/palette";

const colors = paletteFrom(DARK_DEFAULT);

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

  it("a capture behind the program keeps its rows and says how far", () => {
    const first = reduceScreen(null, screen({})) as ScreenState;
    expect(first.ageMs).toBeNull();
    const msg = decodeServer(fixture("screen-behind.json")) as ScreenMsg;
    const behind = reduceScreen(first, { ...msg, cols: first.cols, rows: first.rows }) as ScreenState;
    expect(behind.ageMs).toBe(4000);
    expect(behind.lines).toEqual(first.lines);
    expect(decodeServer({ ...msg, age_ms: -1 })).toBeNull();
  });

  it("applies the fixture", () => {
    const msg = decodeServer(fixture("screen.json")) as ScreenMsg;
    const s = reduceScreen(null, msg) as ScreenState;
    expect(s.lines).toHaveLength(24);
    expect(s.lines[6]).toEqual([run("  PID USER"), run("      CPU%", "1e1e1e", "e5c07b")]);
    expect(s.cursor).toEqual([23, 1]);
  });
});

describe("lastTextRow", () => {
  it("skips rows of nothing and of spaces", () => {
    expect(lastTextRow([[["a"]], [], [["   "]]])).toBe(0);
    expect(lastTextRow([[], []])).toBe(-1);
  });
});

describe("fitArea", () => {
  const tall = { width: 390, height: 700 };
  const short = { width: 390, height: 200 };
  it("keeps the program's height under a question card or the keyboard", () => {
    expect(fitArea(tall, short, true)).toBe(tall);
  });
  it("follows the height otherwise, and a new width (rotation) always", () => {
    expect(fitArea(tall, short, false)).toBe(short);
    const wide = { width: 800, height: 200 };
    expect(fitArea(tall, wide, true)).toBe(wide);
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

describe("the cell grid: wide characters take exactly two cells", () => {
  const pieces = (runs: ScreenRun[], cursor: number | null = null) =>
    rowPieces(rowSegments(runs, cursor)).map((p) => [p.text, p.col, p.cells, p.wide] as const);

  it("emoji and CJK are 2 cells, combining marks and joiners 0", () => {
    for (const ch of ["🚀", "✅", "❌", "⭐", "漢", "한", "Ａ"]) expect(charWidth(ch.codePointAt(0)!), ch).toBe(2);
    for (const ch of ["a", "é", "✓", "─", "█", "·"]) expect(charWidth(ch.codePointAt(0)!), ch).toBe(1);
    for (const ch of ["\u0301", "\u200d", "\ufe0f", "\u{1f3fd}"]) expect(charWidth(ch.codePointAt(0)!), ch).toBe(0);
    expect(isWide("✅".codePointAt(0)!)).toBe(true);
  });

  it("a wide character is a piece of its own at its column; the text after it keeps its column", () => {
    // "✅ deploy done 🚀": ✅ at 0-1, " deploy done " at 2-14, 🚀 at 15-16
    expect(pieces([["✅ deploy done 🚀", null, null, 0]])).toEqual([
      ["✅", 0, 2, true],
      [" deploy done ", 2, 13, false],
      ["🚀", 15, 2, true],
    ]);
    // CJK: two cells each
    expect(pieces([["日本語 ok", null, null, 0]])).toEqual([
      ["日", 0, 2, true],
      ["本", 2, 2, true],
      ["語", 4, 2, true],
      [" ok", 6, 3, false],
    ]);
  });

  it("columns continue across runs and the cursor splits a piece without moving anything", () => {
    const runs: ScreenRun[] = [["a🚀", "ff0000", null, 0], ["bc", null, null, 0]];
    expect(pieces(runs)).toEqual([
      ["a", 0, 1, false],
      ["🚀", 1, 2, true],
      ["bc", 3, 2, false],
    ]);
    // the cursor on the second cell of the emoji is the emoji; "b" stays at column 3
    expect(pieces(runs, 2)).toEqual([
      ["a", 0, 1, false],
      ["🚀", 1, 2, true],
      ["bc", 3, 2, false],
    ]);
    expect(rowSegments(runs, 2).find((x) => x.cursor)!.text).toBe("🚀");
  });

  it("a combining mark or a variation selector joins the character before it and takes no cell", () => {
    expect(pieces([["e\u0301x ❤\ufe0f!", null, null, 0]])).toEqual([["e\u0301x ❤\ufe0f!", 0, 5, false]]);
    expect(pieces([["👍\u{1f3fd}z", null, null, 0]])).toEqual([
      ["👍\u{1f3fd}", 0, 2, true],
      ["z", 2, 1, false],
    ]);
  });

  it("every piece ends where the next begins or later; the row never exceeds the cells of its text", () => {
    const text = "ab✅日c🚀";
    const ps = rowPieces(rowSegments([[text, null, null, 0]], null));
    let end = 0;
    for (const p of ps) {
      expect(p.col).toBeGreaterThanOrEqual(end);
      end = p.col + p.cells;
    }
    expect(end).toBe(1 + 1 + 2 + 2 + 1 + 2);
  });
});

describe("the screen's own colours and block rules (optional: older desktops omit them)", () => {
  it("decodes the styled fixture: fg, bg, rules, a tinted badge background", () => {
    const msg = decodeServer(fixture("screen-styled.json")) as ScreenMsg;
    expect(msg.fg).toBe("d4d4d4");
    expect(msg.bg).toBe("1e1e1e");
    expect(msg.rules).toEqual([[0, "3fb950"], [1, "3fb950"], [2, "f85149"]]);
    const s = reduceScreen(null, msg) as ScreenState;
    expect(s).toMatchObject({ fg: "d4d4d4", bg: "1e1e1e" });
    expect([...s.rules]).toEqual([[0, "3fb950"], [1, "3fb950"], [2, "f85149"]]);
    expect(s.lines[0]![1]).toEqual(["  ✓ 0", "3fb950", "1a3a22", 1]);
  });

  it("without them: null colours, no rules, and the phone's own defaults are used", () => {
    const msg = decodeServer(fixture("screen.json")) as ScreenMsg;
    expect(msg.fg).toBeUndefined();
    const s = reduceScreen(null, msg) as ScreenState;
    expect(s).toMatchObject({ fg: null, bg: null });
    expect(s.rules.size).toBe(0);
    expect(screenDefaults(s, { fg: "#d4d4d4", bg: "#1e1e1e" })).toEqual({ fg: "#d4d4d4", bg: "#1e1e1e" });
    expect(screenDefaults(null, { fg: "#111111", bg: "#222222" })).toEqual({ fg: "#111111", bg: "#222222" });
  });

  it("uses the desktop's colours when sent", () => {
    const s = reduceScreen(null, screen({ fg: "c0caf5", bg: "1a1b26" }))!;
    expect(screenDefaults(s, { fg: "#d4d4d4", bg: "#1e1e1e" })).toEqual({ fg: "#c0caf5", bg: "#1a1b26" });
  });

  it("rules are the whole list on every frame: a diff replaces them, one without clears them; a rules-only frame has no lines", () => {
    const a = reduceScreen(null, screen({ fg: "c0caf5", bg: "1a1b26", rules: [[0, "3fb950"]] }))!;
    const b = reduceScreen(a, screen({ full: false, fg: "c0caf5", bg: "1a1b26", rules: [[0, "3fb950"], [1, "3fb950"]], lines: [[1, [run("x")]]] }))!;
    expect(b).toMatchObject({ fg: "c0caf5", bg: "1a1b26" });
    expect([...b.rules]).toEqual([[0, "3fb950"], [1, "3fb950"]]);
    // a rules-only change: lines [], the text stays
    const c = reduceScreen(b, screen({ full: false, rules: [[2, "f85149"]], lines: [] }))!;
    expect([...c.rules]).toEqual([[2, "f85149"]]);
    expect(c.lines).toEqual(b.lines);
    // omitted: none
    expect(reduceScreen(c, screen({ full: false, lines: [] }))!.rules.size).toBe(0);
    expect(reduceScreen(c, screen({}))!.fg).toBeNull();
  });

  it("rejects malformed colours and rules; a rule row must be on the screen", () => {
    const base = fixture("screen-styled.json") as Record<string, unknown>;
    expect(decodeServer({ ...base, fg: "red" })).toBeNull();
    expect(decodeServer({ ...base, bg: 3 })).toBeNull();
    expect(decodeServer({ ...base, rules: [[99, "3fb950"]] })).toBeNull();
    expect(decodeServer({ ...base, rules: [[0, "zzzzzz"]] })).toBeNull();
    expect(decodeServer({ ...base, rules: "x" })).toBeNull();
    expect(decodeServer({ ...base, rules: [[0, "3fb950"]] })).not.toBeNull();
  });
});

describe("selection", () => {
  const run = (text: string): ScreenRun => [text, null, null, 0];
  const lines = [[run("hello "), run("world")], [run("/usr/local/bin  x")], [], [run("日本 ok")]];

  it("takes the cells between both ends, in reading order", async () => {
    const { selectionText } = await import("../src/terminal");
    expect(selectionText(lines, { row: 0, col: 6 }, { row: 0, col: 10 })).toBe("world");
    // either end may be first
    expect(selectionText(lines, { row: 0, col: 10 }, { row: 0, col: 6 })).toBe("world");
    expect(selectionText(lines, { row: 0, col: 6 }, { row: 1, col: 3 })).toBe("world\n/usr");
  });
  it("keeps a row's inner blanks, leaves its trailing ones", async () => {
    const { selectionText } = await import("../src/terminal");
    expect(selectionText(lines, { row: 1, col: 0 }, { row: 1, col: 40 })).toBe("/usr/local/bin  x");
    expect(selectionText(lines, { row: 0, col: 5 }, { row: 0, col: 5 })).toBe("");
    expect(selectionText(lines, { row: 1, col: 14 }, { row: 2, col: 3 })).toBe("  x\n");
  });
  it("a wide character touched by either of its cells comes whole", async () => {
    const { selectionText } = await import("../src/terminal");
    expect(selectionText(lines, { row: 3, col: 1 }, { row: 3, col: 2 })).toBe("日本");
    expect(selectionText(lines, { row: 3, col: 5 }, { row: 3, col: 6 })).toBe("ok");
  });
  it("a hold takes a word, a path as one, a blank alone", async () => {
    const { selectionText, wordAt } = await import("../src/terminal");
    const word = (row: number, col: number) => {
      const [a, b] = wordAt(lines, { row, col });
      return selectionText(lines, a, b);
    };
    expect(word(0, 7)).toBe("world");
    expect(word(1, 5)).toBe("/usr/local/bin");
    expect(word(1, 14)).toBe("");
    expect(word(2, 3)).toBe("");
  });
  it("Line takes the row's text, Screen every cell", async () => {
    const { lineAt, screenSpan, selectionText } = await import("../src/terminal");
    const line = (row: number, col: number) => {
      const [a, b] = lineAt(lines, { row, col });
      return selectionText(lines, a, b);
    };
    expect(line(0, 2)).toBe("hello world");
    expect(line(1, 30)).toBe("/usr/local/bin  x");
    // a wide character's second cell ends the row
    expect(lineAt(lines, { row: 3, col: 0 })).toEqual([{ row: 3, col: 0 }, { row: 3, col: 6 }]);
    expect(lineAt(lines, { row: 2, col: 4 })).toEqual([{ row: 2, col: 4 }, { row: 2, col: 4 }]);
    expect(screenSpan(4, 80)).toEqual([{ row: 0, col: 0 }, { row: 3, col: 79 }]);
    expect(screenSpan(0, 0)).toEqual([{ row: 0, col: 0 }, { row: 0, col: 0 }]);
  });
  it("a handle steps (and the phone ticks) only when it leaves a word or a run of blanks", async () => {
    const { wordStep } = await import("../src/terminal");
    const at = (row: number, col: number) => ({ row, col });
    // within "world", within "/usr/local/bin": no step
    expect(wordStep(lines, at(0, 6), at(0, 10))).toBe(false);
    expect(wordStep(lines, at(1, 0), at(1, 13))).toBe(false);
    // onto the blank between, then from the blanks into "x"
    expect(wordStep(lines, at(0, 4), at(0, 5))).toBe(true);
    expect(wordStep(lines, at(1, 14), at(1, 15))).toBe(false);
    expect(wordStep(lines, at(1, 15), at(1, 16))).toBe(true);
    // past a row's text is one run of blanks
    expect(wordStep(lines, at(1, 20), at(1, 70))).toBe(false);
    expect(wordStep(lines, at(2, 0), at(2, 50))).toBe(false);
    // another row, or the same cell
    expect(wordStep(lines, at(0, 6), at(1, 6))).toBe(true);
    expect(wordStep(lines, at(0, 6), at(0, 6))).toBe(false);
    // a wide word: both cells of 日本 are one
    expect(wordStep(lines, at(3, 0), at(3, 3))).toBe(false);
    expect(wordStep(lines, at(3, 3), at(3, 5))).toBe(true);
  });
  it("a touch maps to a cell on the screen, never off it", async () => {
    const { cellAt } = await import("../src/terminal");
    expect(cellAt(13, 31, 6, 15, 80, 24)).toEqual({ row: 2, col: 2 });
    expect(cellAt(-5, -5, 6, 15, 80, 24)).toEqual({ row: 0, col: 0 });
    expect(cellAt(9999, 9999, 6, 15, 80, 24)).toEqual({ row: 23, col: 79 });
  });
  it("draws the triangles the fonts lack, without changing the text copied", async () => {
    const { drawnText, selectionText } = await import("../src/terminal");
    expect(drawnText("\u23F5\u23F5 auto")).toBe("\u25B8\u25B8 auto");
    const r = [[run("\u23F5 x")]];
    expect(selectionText(r, { row: 0, col: 0 }, { row: 0, col: 2 })).toBe("\u23F5 x");
  });
});

describe("cursorScrollY", () => {
  it("keeps the cursor and the row under it in sight", () => {
    // 16 pt rows, 8 pt padding, a 160 pt view: 9 rows and a bit
    expect(cursorScrollY(6, 16, 8, 160)).toBe(0);
    expect(cursorScrollY(39, 16, 8, 160)).toBe(41 * 16 + 8 - 160);
    expect(cursorScrollY(undefined, 16, 8, 160)).toBeNull();
  });
});

describe("a run's text room", () => {
  it("leaves room for a font 8% wider than its cells, on any length", () => {
    for (const w of [10, 100, 400]) expect(textRoom(w)).toBeGreaterThanOrEqual(w * 1.08);
  });
});

describe("atEnd", () => {
  it("is true at the end and within the slop", () => {
    expect(atEnd(500, 1000, 500, 48)).toBe(true);
    expect(atEnd(460, 1000, 500, 48)).toBe(true);
  });
  it("is false once scrolled up", () => {
    expect(atEnd(300, 1000, 500, 48)).toBe(false);
  });
  it("reads a view shrunk by the keyboard as scrolled up: judge it on a gesture's end, not on the shrink's scroll event", () => {
    expect(atEnd(500, 1000, 500, 48)).toBe(true);
    expect(atEnd(500, 1000, 220, 48)).toBe(false);
  });
});
