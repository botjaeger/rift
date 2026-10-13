import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BatchedStore, PUBLISH_INTERVAL_MS } from "../src/protocol/batch";

describe("BatchedStore", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("coalesces a flood into one publish per interval", () => {
    const store = new BatchedStore(0, PUBLISH_INTERVAL_MS, () => Date.now());
    const seen: number[] = [];
    store.subscribe(() => seen.push(store.getSnapshot()));
    for (let i = 1; i <= 1000; i++) store.update((n) => n + 1);
    expect(store.peek()).toBe(1000);
    expect(store.getSnapshot()).toBe(0);
    vi.advanceTimersByTime(0);
    expect(seen).toEqual([1000]);

    // Over one second of a steady stream (one write per ms) listeners run at most 10 times.
    for (let ms = 0; ms < 1000; ms++) {
      store.update((n) => n + 1);
      vi.advanceTimersByTime(1);
    }
    vi.advanceTimersByTime(PUBLISH_INTERVAL_MS);
    expect(seen.length - 1).toBeLessThanOrEqual(11);
    expect(seen.at(-1)).toBe(2000);
  });

  it("does not publish an unchanged state", () => {
    const store = new BatchedStore({ a: 1 });
    const listener = vi.fn();
    store.subscribe(listener);
    store.update((s) => s);
    vi.runAllTimers();
    expect(listener).not.toHaveBeenCalled();
  });

  it("flush publishes at once and unsubscribe stops notifications", () => {
    const store = new BatchedStore(0);
    const listener = vi.fn();
    const off = store.subscribe(listener);
    store.update(() => 1);
    store.flush();
    expect(listener).toHaveBeenCalledTimes(1);
    off();
    store.update(() => 2);
    vi.runAllTimers();
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot()).toBe(2);
  });
});
