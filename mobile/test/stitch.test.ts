import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { aheadRows, liveMatches, mergePage, pageComplete, pageScreens, pageUsable, type StitchPage } from "../src/stitch";
import type { ScreenState } from "../src/protocol/store";
import type { PageMsg, ScreenRun } from "../src/protocol/types";

const fixture = JSON.parse(readFileSync(new URL("../../crates/rift-remote/fixtures/server/page.json", import.meta.url), "utf8")) as PageMsg;

const row = (t: string): ScreenRun[] => [[t, null, null, 0]];

function msg(over: Partial<PageMsg>): PageMsg {
  return { t: "page", pane: 1, epoch: 1, area: [1, 4, 0, 10], top: 0, bottom: 3, at: 0, from: 0, lines: [], ends: [false, false], state: "ok", ...over };
}

function live(texts: string[]): ScreenState {
  return { cols: 20, rows: texts.length, alt: true, cursor: [4, 0], lines: texts.map(row), fg: null, bg: null, rules: new Map(), stale: false, ageMs: null, scrollback: null };
}

describe("mergePage", () => {
  it("takes the fixture whole", () => {
    const p = mergePage(null, fixture);
    expect(p.rows.get(-40)?.[0]?.[0]).toContain("question 40");
    expect([p.top, p.bottom, p.at]).toEqual([-40, 36, -40]);
  });

  it("joins chunks until every row is there, and a new epoch starts over", () => {
    let p = mergePage(null, msg({ top: -2, bottom: 3, from: -2, lines: [row("a"), row("b")] }));
    expect(pageComplete(p)).toBe(false);
    expect(pageUsable(p)).toBe(false);
    p = mergePage(p, msg({ top: -2, bottom: 3, from: 0, lines: [row("c"), row("d"), row("e")] }));
    expect(pageComplete(p)).toBe(true);
    expect(pageUsable(p)).toBe(true);
    // rows added at the top later: the page grows, nothing lost
    p = mergePage(p, msg({ top: -4, bottom: 3, from: -4, lines: [row("y"), row("z")], at: -4 }));
    expect([...p.rows.keys()].sort((a, b) => a - b)).toEqual([-4, -3, -2, -1, 0, 1, 2]);
    expect(p.at).toBe(-4);
    // another epoch: the old rows are gone
    p = mergePage(p, msg({ epoch: 2, top: 0, bottom: 1, from: 0, lines: [row("n")] }));
    expect([...p.rows.keys()]).toEqual([0]);
  });

  it("a flat answer is no page to scroll", () => {
    expect(pageUsable(mergePage(null, msg({ state: "flat", area: [0, 0, 0, 0], top: 0, bottom: 0 })))).toBe(false);
  });
});

describe("pageScreens", () => {
  it("draws the rows above and below the area as they are now, the page between, at the live width", () => {
    const p = mergePage(null, msg({ area: [1, 3, 2, 10], top: -1, bottom: 3, at: 0, from: -1, lines: [row("p0"), row("p1"), row("p2"), row("p3")] }));
    const { above, body, below } = pageScreens(p, live(["header", "  p1", "  p2", "input", "status"]));
    expect(above.lines.map((l) => l[0]?.[0])).toEqual(["header"]);
    expect(below.lines.map((l) => l[0]?.[0])).toEqual(["input", "status"]);
    // the page's rows start at the area's column
    expect(body.lines.map((l) => l.map((r) => r[0]).join(""))).toEqual(["  p0", "  p1", "  p2", "  p3"]);
    expect([above.cols, body.cols, below.cols]).toEqual([20, 20, 20]);
    expect(body.cursor).toBeNull();
  });
});

describe("liveMatches", () => {
  const p = mergePage(null, msg({ area: [1, 3, 0, 10], top: 0, bottom: 4, at: 1, from: 0, lines: [row("a"), row("b"), row("c"), row("d")] }));

  it("holds while the program shows the page's rows where it is (a panel beside them changing)", () => {
    expect(liveMatches(p, live(["head", "b         │panel 1", "c         │panel 2", "foot"]))).toBe(true);
  });

  it("fails once the program drew something else there", () => {
    expect(liveMatches(p, live(["head", "b", "new output", "foot"]))).toBe(false);
    expect(liveMatches(p, live(["head", "b"]))).toBe(false);
  });
});

describe("aheadRows", () => {
  it("asks for three areas' worth, within the bounds", () => {
    expect(aheadRows(null, 40)).toBe(120);
    expect(aheadRows(null, 3)).toBe(20);
    expect(aheadRows(mergePage(null, msg({ area: [0, 300, 0, 10] })) as StitchPage, 40)).toBe(500);
  });
});
