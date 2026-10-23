import { describe, expect, it } from "vitest";
import { MAX_SCROLL_LINES } from "../src/protocol/types";
import { dragLines, dragScrollsPane, ScrollCoalescer, SCROLL_INTERVAL_MS } from "../src/scroll";

/** A coalescer on a fake clock: what it sent, and a way to move time. */
function rig() {
  let now = 1_000;
  const sent: number[] = [];
  const timers: { at: number; fn: () => void; id: number }[] = [];
  let nextId = 0;
  const c = new ScrollCoalescer({
    send: (n) => sent.push(n),
    now: () => now,
    schedule: (fn, ms) => {
      const id = ++nextId;
      timers.push({ at: now + ms, fn, id });
      return id;
    },
    unschedule: (h) => {
      const i = timers.findIndex((t) => t.id === h);
      if (i >= 0) timers.splice(i, 1);
    },
  });
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const t = timers[0];
      if (!t || t.at > end) break;
      timers.shift();
      now = t.at;
      t.fn();
    }
    now = end;
  };
  return { c, sent, advance, pending: () => timers.length };
}

describe("drag lines", () => {
  it("counts rows of travel, down is up", () => {
    expect(dragLines(32, 16)).toBe(2);
    expect(dragLines(-8, 16)).toBe(-0.5);
    expect(dragLines(10, 0)).toBe(0);
    expect(dragLines(Number.NaN, 16)).toBe(0);
  });

  it("scrolls the computer unless the view itself has more to show that way", () => {
    const fits = { overflow: false, atTop: true, atBottom: true };
    expect(dragScrollsPane(5, fits)).toBe(true);
    expect(dragScrollsPane(-5, fits)).toBe(true);
    // zoomed in, in the middle: the view scrolls locally
    const middle = { overflow: true, atTop: false, atBottom: false };
    expect(dragScrollsPane(5, middle)).toBe(false);
    expect(dragScrollsPane(-5, middle)).toBe(false);
    // past its edges the computer's pane goes on
    expect(dragScrollsPane(5, { overflow: true, atTop: true, atBottom: false })).toBe(true);
    expect(dragScrollsPane(-5, { overflow: true, atTop: true, atBottom: false })).toBe(false);
    expect(dragScrollsPane(-5, { overflow: true, atTop: false, atBottom: true })).toBe(true);
  });
});

describe("scroll coalescer", () => {
  it("sends the first movement at once and merges what follows within the interval", () => {
    const { c, sent, advance } = rig();
    c.add(1.2);
    expect(sent).toEqual([1]);
    // twenty touch events in one interval: one message
    for (let i = 0; i < 20; i++) c.add(0.5);
    expect(sent).toEqual([1]);
    advance(SCROLL_INTERVAL_MS);
    // 0.2 left over + 10 = 10.2: ten lines, the fraction carried
    expect(sent).toEqual([1, 10]);
    c.add(0.9);
    advance(SCROLL_INTERVAL_MS);
    expect(sent).toEqual([1, 10, 1]);
  });

  it("never sends more than one message per interval, whatever the touch rate", () => {
    const { c, sent, advance } = rig();
    for (let t = 0; t < 1000; t += 4) {
      c.add(0.7);
      advance(4);
    }
    // a second of dragging: at most 1000 / interval + 1 messages, all lines delivered
    expect(sent.length).toBeLessThanOrEqual(Math.floor(1000 / SCROLL_INTERVAL_MS) + 1);
    expect(sent.length).toBeGreaterThan(5);
    advance(SCROLL_INTERVAL_MS);
    // every whole line arrived (less at most the fraction still waiting)
    expect(Math.abs(sent.reduce((a, b) => a + b, 0) - 250 * 0.7)).toBeLessThanOrEqual(1);
  });

  it("caps a message at the gateway's bound and sends the rest next", () => {
    const { c, sent, advance } = rig();
    c.add(-130.5);
    expect(sent).toEqual([-MAX_SCROLL_LINES]);
    advance(SCROLL_INTERVAL_MS);
    expect(sent).toEqual([-MAX_SCROLL_LINES, -MAX_SCROLL_LINES]);
    advance(SCROLL_INTERVAL_MS);
    expect(sent).toEqual([-MAX_SCROLL_LINES, -MAX_SCROLL_LINES, -30]);
    advance(SCROLL_INTERVAL_MS * 3);
    expect(sent).toHaveLength(3);
  });

  it("opposite movements cancel before anything goes out; nothing under a line is sent", () => {
    const { c, sent, advance } = rig();
    c.add(0.6);
    c.add(-0.6);
    c.add(0.4);
    advance(SCROLL_INTERVAL_MS * 4);
    expect(sent).toEqual([]);
  });

  it("flush sends what waits at the interval, dropping the fraction; cancel drops it", () => {
    const { c, sent, pending, advance } = rig();
    c.add(2);
    c.add(3.6);
    expect(sent).toEqual([2]);
    c.flush();
    // still within the first message's interval: it waits for it, never a burst
    expect(sent).toEqual([2]);
    advance(SCROLL_INTERVAL_MS);
    expect(sent).toEqual([2, 3]);
    expect(pending()).toBe(0);
    // a fraction alone is dropped
    advance(SCROLL_INTERVAL_MS);
    c.add(0.4);
    c.flush();
    expect(sent).toEqual([2, 3]);
    expect(pending()).toBe(0);
    // past the interval a line goes at once; what follows within it waits, and cancel drops it
    c.add(4);
    c.add(5);
    expect(sent).toEqual([2, 3, 4]);
    c.cancel();
    expect(pending()).toBe(0);
    c.flush();
    advance(SCROLL_INTERVAL_MS * 3);
    expect(sent).toEqual([2, 3, 4]);
  });

  it("a flush past the bound goes out in bounded messages, one per interval", () => {
    const { c, sent, advance } = rig();
    c.add(1);
    c.add(120.7);
    c.flush();
    expect(sent).toEqual([1]);
    const counts: number[] = [];
    for (let i = 0; i < 6; i++) {
      advance(SCROLL_INTERVAL_MS / 2);
      counts.push(sent.length);
    }
    expect(sent).toEqual([1, MAX_SCROLL_LINES, MAX_SCROLL_LINES, 20]);
    // never two messages within one interval: under the gateway's 20 a second
    expect(counts).toEqual([1, 2, 2, 3, 3, 4]);
  });
});
