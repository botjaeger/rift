import { describe, expect, it } from "vitest";
import {
  ECHO_WINDOW_MS,
  QUIET_MS,
  SPEED_SAMPLES,
  SpeedMeter,
  bytesText,
  rateText,
  rates,
  summarize,
  verdict,
} from "../src/protocol/speed";

describe("SpeedMeter", () => {
  it("times input and scrolling to the next screen update, the first after a quiet second", () => {
    const m = new SpeedMeter(0);
    m.input(100);
    m.input(150); // same burst: the clock started at 100
    m.frame(180);
    m.frame(200); // no input waiting: only counted as a frame
    m.scrolled(150 + QUIET_MS);
    m.frame(195 + QUIET_MS);
    const s = m.snapshot(2000);
    expect(s.echo).toEqual([80]);
    expect(s.scroll).toEqual([45]);
    expect(s.frames).toBe(3);
  });

  it("times nothing inside a burst, where an update may answer an earlier input", () => {
    const m = new SpeedMeter(0);
    // a swipe: a scroll every 16 ms, the updates answering them 40 ms later
    for (let t = 0; t <= 200; t += 8) {
      if (t % 16 === 0 && t <= 160) m.scrolled(t);
      if (t % 16 === 8 && t >= 40) m.frame(t);
    }
    m.input(300); // typed right after it: still the burst
    m.frame(310);
    const s = m.snapshot(400);
    expect(s.scroll).toEqual([40]);
    expect(s.echo).toEqual([]);
  });

  it("drops a wait that no update answered in time, never counting it late", () => {
    const m = new SpeedMeter(0);
    m.input(0);
    m.frame(ECHO_WINDOW_MS + 1);
    expect(m.snapshot(ECHO_WINDOW_MS + 2).echo).toEqual([]);
    // expired at a snapshot: the next update is not charged to it either
    m.scrolled(10_000);
    m.snapshot(10_000 + ECHO_WINDOW_MS + 1);
    m.frame(10_000 + ECHO_WINDOW_MS + 2);
    expect(m.snapshot(20_000).scroll).toEqual([]);
  });

  it("keeps the last SPEED_SAMPLES round trips and starts over on reset", () => {
    const m = new SpeedMeter(0);
    for (let i = 1; i <= SPEED_SAMPLES + 5; i++) m.pong(i);
    m.sent(10);
    m.received(30);
    const s = m.snapshot(1);
    expect(s.rtt).toHaveLength(SPEED_SAMPLES);
    expect(s.rtt[0]).toBe(6);
    expect([s.bytesIn, s.bytesOut]).toEqual([30, 10]);
    m.reset(50);
    expect(m.snapshot(60)).toMatchObject({ since: 50, bytesIn: 0, bytesOut: 0, frames: 0, rtt: [] });
  });
});

describe("speed summaries", () => {
  it("summarizes samples: median, slowest 5 %, range and jitter", () => {
    expect(summarize([])).toBeNull();
    const s = summarize([40, 60, 50, 200, 50])!;
    expect(s).toMatchObject({ last: 50, median: 50, p95: 200, min: 40, max: 200, count: 5 });
    // |60-40| + |50-60| + |200-50| + |50-200| = 330 over 4 steps
    expect(s.jitter).toBe(82.5);
    expect(summarize([7])!.jitter).toBe(0);
  });

  it("gives rates only between snapshots of one connection", () => {
    const a = { since: 0, at: 1000, bytesIn: 1000, bytesOut: 100, frames: 2, rtt: [], echo: [], scroll: [] };
    const b = { ...a, at: 3000, bytesIn: 5000, bytesOut: 300, frames: 12 };
    expect(rates(a, b)).toEqual({ inPerSec: 2000, outPerSec: 100, framesPerSec: 5 });
    expect(rates(null, b)).toBeNull();
    expect(rates({ ...a, since: -1 }, b)).toBeNull();
    expect(rates(b, b)).toBeNull();
  });

  it("words sizes and speeds", () => {
    expect(rateText(820)).toBe("820 B/s");
    expect(rateText(12_400)).toBe("12.4 KB/s");
    expect(rateText(3_100_000)).toBe("3.1 MB/s");
    expect(bytesText(640)).toBe("640 B");
    expect(bytesText(2_500_000)).toBe("2.5 MB");
    expect(verdict(30).word).toBe("Fast");
    expect(verdict(120).word).toBe("Fair");
    expect(verdict(450)).toEqual({ word: "Slow", tone: "bad" });
  });
});
