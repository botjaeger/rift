import { describe, expect, it } from "vitest";
import { createSingleFlight } from "../src/ui/useSingleFlight";

/** A task that runs until `release` is called, counting its runs. */
function gate() {
  let runs = 0;
  let release: () => void = () => undefined;
  let fail: (e: Error) => void = () => undefined;
  const task = () =>
    new Promise<void>((resolve, reject) => {
      runs++;
      release = resolve;
      fail = reject;
    });
  return { task, runs: () => runs, release: () => release(), fail: (e: Error) => fail(e) };
}

describe("createSingleFlight", () => {
  it("runs a second call made while the first is pending only once", async () => {
    const once = createSingleFlight();
    const g = gate();
    const first = once(g.task);
    const second = once(g.task); // the double tap
    await second; // dropped: resolves without waiting for the first
    expect(g.runs()).toBe(1);
    g.release();
    await first;
    expect(g.runs()).toBe(1);
  });

  it("runs again once the last task settled", async () => {
    const once = createSingleFlight();
    const g = gate();
    const first = once(g.task);
    g.release();
    await first;
    const again = once(g.task);
    expect(g.runs()).toBe(2);
    g.release();
    await again;
  });

  it("frees the slot when a task rejects, and passes the rejection on", async () => {
    const once = createSingleFlight();
    const g = gate();
    const first = once(g.task);
    g.fail(new Error("offline"));
    await expect(first).rejects.toThrow("offline");
    const again = once(g.task);
    expect(g.runs()).toBe(2);
    g.release();
    await again;
  });

  it("keeps separate flights apart", async () => {
    const a = createSingleFlight();
    const b = createSingleFlight();
    const ga = gate();
    const gb = gate();
    const pa = a(ga.task);
    const pb = b(gb.task);
    expect(ga.runs() + gb.runs()).toBe(2);
    ga.release();
    gb.release();
    await Promise.all([pa, pb]);
  });
});
