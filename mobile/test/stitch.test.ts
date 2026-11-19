import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { aheadRows, edgeToAsk, liveMatches, mayAskPage, mergePage, nextSpeed, PAGE_FIRST_MS, PAGE_FLOW, PAGE_JOB_MS, PAGE_READ_AHEAD, PAGE_REPLY_MS, PAGE_RETRY_MS, PAGE_WHEEL_MS, pageAsking, pageComplete, pageExpire, pageFlow, pageScreens, pageToShow, pageUsable, wheelInstead, type PageEffect, type PageEvent, type PageFlow, type StitchPage } from "../src/stitch";
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

  it("keeps the page in view while more rows come in pieces", () => {
    const shown = mergePage(null, msg({ top: 0, bottom: 3, from: 0, lines: [row("a"), row("b"), row("c")] }));
    // the first of two pieces adding rows above: the view keeps the whole page it has
    const half = mergePage(shown, msg({ top: -4, bottom: 3, from: -4, lines: [row("w"), row("x")] }));
    expect(pageToShow(shown, half)).toBe(shown);
    // the last piece: the grown page shows
    const whole = mergePage(half, msg({ top: -4, bottom: 3, from: -2, lines: [row("y"), row("z")] }));
    expect(pageToShow(shown, whole)).toBe(whole);
    // a new epoch, or nothing usable shown yet: what came is shown (it waits for its rows there)
    const fresh = mergePage(shown, msg({ epoch: 2, top: 0, bottom: 2, from: 0, lines: [row("n")] }));
    expect(pageToShow(shown, fresh)).toBe(fresh);
    expect(pageToShow(null, half)).toBe(half);
    expect(pageToShow(shown, null)).toBeNull();
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
  it("asks for one screen first, then three areas' worth, within the bounds", () => {
    expect(aheadRows(null, 40)).toBe(40);
    expect(aheadRows(null, 3)).toBe(20);
    expect(aheadRows(mergePage(null, msg({ area: [1, 21, 0, 10] })) as StitchPage, 40)).toBe(60);
    expect(aheadRows(mergePage(null, msg({ area: [0, 300, 0, 10] })) as StitchPage, 40)).toBe(500);
  });
});

describe("mayAskPage", () => {
  it("asks again after a stopped page once a pause has passed; a flat program never", () => {
    const ok = mergePage(null, msg({ lines: [row("a"), row("b"), row("c")] }));
    expect(mayAskPage(null, 0, 0)).toBe(true);
    expect(mayAskPage(ok, 0, 0)).toBe(true);
    const stopped = { ...ok, state: "stopped" as const };
    expect(mayAskPage(stopped, 1000, 1000)).toBe(false);
    expect(mayAskPage(stopped, 1000 + PAGE_RETRY_MS - 1, 1000)).toBe(false);
    expect(mayAskPage(stopped, 1000 + PAGE_RETRY_MS, 1000)).toBe(true);
    expect(mayAskPage({ ...ok, state: "flat" }, 1e9, 0)).toBe(false);
  });
});

describe("wheelInstead", () => {
  it("leaves the wheel alone while a first page is on its way", () => {
    expect(wheelInstead(10_000, null, -Infinity)).toBe(false);
    expect(wheelInstead(10_000, 10_000 - PAGE_FIRST_MS + 1, -Infinity)).toBe(false);
  });
  it("scrolls the program once the page is overdue, or its request was refused", () => {
    expect(wheelInstead(10_000, 10_000 - PAGE_FIRST_MS, -Infinity)).toBe(true);
    expect(wheelInstead(50_000, null, 50_000 - PAGE_WHEEL_MS + 1)).toBe(true);
    // the refusal is forgotten after a while: a page is asked for again
    expect(wheelInstead(50_000, null, 50_000 - PAGE_WHEEL_MS)).toBe(false);
  });
});

describe("edgeToAsk", () => {
  it("asks within PAGE_READ_AHEAD screens of an edge whose end is not reached, up first", () => {
    const p = mergePage(null, msg({ lines: [row("a"), row("b"), row("c")] })) as StitchPage;
    const open = { ...p, ends: [false, false] as [boolean, boolean] };
    const reach = 100 * PAGE_READ_AHEAD;
    // 100 tall view over 2000 of content
    expect(edgeToAsk(open, 0, 100, 2000)).toBe("up");
    expect(edgeToAsk(open, reach - 1, 100, 2000)).toBe("up");
    expect(edgeToAsk(open, reach, 100, 2000)).toBe(null);
    expect(edgeToAsk(open, 2000 - 100 - reach + 1, 100, 2000)).toBe("down");
    expect(edgeToAsk(open, 2000 - 100 - reach, 100, 2000)).toBe(null);
    expect(edgeToAsk({ ...open, ends: [true, false] }, 0, 100, 150)).toBe("down");
    expect(edgeToAsk({ ...open, ends: [true, true] }, 0, 100, 150)).toBe(null);
    // not laid out yet: nothing
    expect(edgeToAsk(open, 0, 0, 0)).toBe(null);
  });

  it("asks a flick ahead by what it covers while a batch comes in", () => {
    const p = mergePage(null, msg({ lines: [row("a"), row("b"), row("c")] })) as StitchPage;
    const open = { ...p, ends: [false, false] as [boolean, boolean] };
    const reach = 100 * PAGE_READ_AHEAD;
    // 3 px a ms up (30 screens a second of a 100 tall view): 7500 px ahead
    const far = 3 * PAGE_JOB_MS;
    expect(far).toBeGreaterThan(reach);
    expect(edgeToAsk(open, far - 1, 100, 20_000, -3)).toBe("up");
    expect(edgeToAsk(open, far, 100, 20_000, -3)).toBe(null);
    // the same speed the other way leaves the top edge at the screens kept ready
    expect(edgeToAsk(open, reach, 100, 20_000, 3)).toBe(null);
    expect(edgeToAsk(open, 20_000 - 100 - far + 1, 100, 20_000, 3)).toBe("down");
    // a slow scroll keeps PAGE_READ_AHEAD screens
    expect(edgeToAsk(open, reach, 100, 20_000, -0.1)).toBe(null);
    expect(edgeToAsk(open, reach - 1, 100, 20_000, -0.1)).toBe("up");
  });
});

describe("nextSpeed", () => {
  it("averages the last steps and forgets a pause", () => {
    const a = nextSpeed(null, 1000, 0);
    expect(a.v).toBe(0);
    const b = nextSpeed(a, 900, 16);
    expect(b.v).toBeCloseTo(-100 / 16 / 2);
    const c = nextSpeed(b, 800, 32);
    expect(c.v).toBeCloseTo((b.v - 100 / 16) / 2);
    expect(nextSpeed(c, 700, 32 + 201).v).toBe(0);
    expect(nextSpeed(c, 700, 32).v).toBe(0);
  });
});

describe("pageFlow", () => {
  const ok = mergePage(null, msg({ lines: [row("a"), row("b"), row("c")] }));
  const stopped = { ...ok, state: "stopped" as const };
  const run = (events: PageEvent[], from: PageFlow = PAGE_FLOW) => {
    let f = from;
    const fx: PageEffect[] = [];
    for (const ev of events) {
      const [next, e] = pageFlow(f, ev);
      f = next;
      fx.push(e);
    }
    return { f, fx };
  };

  it("sends one page_ahead at a time and seeks once its page is whole", () => {
    const { f, fx } = run([
      { e: "ask", page: ok, now: 0 },
      { e: "ask", page: ok, now: 10 },
      { e: "reply", ok: true, busy: false, now: 20 },
      { e: "page", state: "ok", complete: false, now: 30 },
      { e: "ask", page: ok, now: 40 },
      { e: "page", state: "ok", complete: true, now: 50 },
    ]);
    expect(fx.map((e) => !!e.send)).toEqual([true, false, false, false, false, false]);
    expect(fx[3]?.seek).toBeUndefined();
    expect(fx[5]?.seek).toBe(true);
    expect(f.asked).toBe(null);
    // the page is drawn before the next ask (asked before, it would name the page in view)
    expect(run([{ e: "ask", page: null, now: 55 }], f).fx[0]?.send).toBeUndefined();
    expect(run([{ e: "drawn" }, { e: "ask", page: ok, now: 60 }], f).fx[1]?.send).toBe(true);
  });

  it("asks nothing between a whole page and its draw, and goes on once drawn", () => {
    // the first page came; a scroll before the view drew it would ask a new page (no epoch)
    const { f } = run([{ e: "ask", page: null, now: 0 }, { e: "page", state: "ok", complete: true, now: 400 }]);
    expect(f.undrawn).toBe(true);
    expect(run([{ e: "ask", page: null, now: 410 }], f).fx[0]?.send).toBeUndefined();
    // a chunk of a page still coming leaves the view's page (no draw needed): asks wait for the rest anyway
    const part = run([{ e: "drawn" }, { e: "ask", page: ok, now: 500 }, { e: "page", state: "ok", complete: false, now: 600 }], f).f;
    expect(part.undrawn).toBe(false);
  });

  it("asks again once a reply was lost with the connection", () => {
    const { f } = run([{ e: "ask", page: null, now: 0 }]);
    expect(pageAsking(f, PAGE_REPLY_MS - 1)).toBe(true);
    expect(pageAsking(f, PAGE_REPLY_MS)).toBe(false);
    expect(run([{ e: "ask", page: null, now: PAGE_REPLY_MS - 1 }], f).fx[0]?.send).toBeUndefined();
    expect(run([{ e: "ask", page: null, now: PAGE_REPLY_MS }], f).fx[0]?.send).toBe(true);
    expect(pageExpire(f, PAGE_REPLY_MS - 1).asked).toBe(0);
    expect(pageExpire(f, PAGE_REPLY_MS).asked).toBe(null);
  });

  it("a refusal hands the drag to the wheel, a busy one does not", () => {
    const refused = run([{ e: "ask", page: null, now: 100 }, { e: "reply", ok: false, busy: false, now: 200 }]).f;
    expect(refused).toMatchObject({ asked: null, refused: 200 });
    expect(wheelInstead(300, refused.asked, refused.refused)).toBe(true);
    const busy = run([{ e: "ask", page: null, now: 100 }, { e: "reply", ok: false, busy: true, now: 200 }]).f;
    expect(busy).toMatchObject({ asked: null, refused: -Infinity });
    expect(wheelInstead(300, busy.asked, busy.refused)).toBe(false);
  });

  it("a stopped page waits the pause, retries on its own twice, and again after a drag", () => {
    const { f, fx } = run([{ e: "ask", page: ok, now: 0 }, { e: "page", state: "stopped", complete: true, now: 1000 }, { e: "drawn" }]);
    expect(fx[1]).toEqual({ retryAt: 1000 + PAGE_RETRY_MS + 100, seek: true });
    expect(run([{ e: "ask", page: stopped, now: 1000 + PAGE_RETRY_MS - 1 }], f).fx[0]?.send).toBeUndefined();
    expect(run([{ e: "ask", page: stopped, now: 1000 + PAGE_RETRY_MS }], f).fx[0]?.send).toBe(true);
    const tries = run([{ e: "retry" }, { e: "retry" }, { e: "retry" }], f);
    expect(tries.fx.map((e) => !!e.ask)).toEqual([true, true, false]);
    expect(run([{ e: "drag" }, { e: "retry" }], tries.f).fx[1]?.ask).toBe(true);
    // an ok answer counts the retries again too
    const after = run([{ e: "page", state: "ok", complete: true, now: 5000 }, { e: "retry" }], tries.f);
    expect(after.fx[1]?.ask).toBe(true);
  });

  it("a new screen shape drops the request on its way", () => {
    const { f } = run([{ e: "ask", page: ok, now: 0 }, { e: "shape" }]);
    expect(f.asked).toBe(null);
    expect(run([{ e: "ask", page: null, now: 1 }], f).fx[0]?.send).toBe(true);
    // its late page still merges, but moves nothing: no request is waiting for it
    expect(run([{ e: "page", state: "ok", complete: true, now: 2 }], f).fx[0]?.seek).toBeUndefined();
  });
});
