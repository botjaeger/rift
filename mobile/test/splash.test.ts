import { describe, expect, it } from "vitest";
import { SPLASH, splashDuration, splashFadeAt, splashFrame, splashSteps } from "../src/splash";

describe("splash timeline", () => {
  it("blinks once at the empty prompt, then types rift with the cursor lit", () => {
    expect(splashFrame(0)).toEqual({ typed: 0, cursor: true });
    expect(splashFrame(199)).toEqual({ typed: 0, cursor: true });
    expect(splashFrame(200)).toEqual({ typed: 0, cursor: false });
    expect(splashFrame(399)).toEqual({ typed: 0, cursor: false });
    expect(splashFrame(400)).toEqual({ typed: 1, cursor: true });
    expect(splashFrame(469)).toEqual({ typed: 1, cursor: true });
    expect(splashFrame(470)).toEqual({ typed: 2, cursor: true });
    expect(splashFrame(540)).toEqual({ typed: 3, cursor: true });
    expect(splashFrame(610)).toEqual({ typed: 4, cursor: true });
    expect(SPLASH.word.slice(0, splashFrame(10_000).typed)).toBe("rift");
  });

  it("changes only at its steps, which run in order", () => {
    const steps = splashSteps();
    expect(steps).toEqual([...steps].sort((a, b) => a - b));
    for (let t = 1; t < splashFadeAt(); t++) {
      if (!steps.includes(t)) expect(splashFrame(t)).toEqual(splashFrame(t - 1));
    }
  });

  it("is gone in under a second, after the word is complete", () => {
    expect(splashFadeAt()).toBeGreaterThan(Math.max(...SPLASH.typeAt));
    expect(splashDuration()).toBeLessThan(1000);
  });

  it("shows the finished prompt at once with reduced motion", () => {
    expect(splashFrame(0, true)).toEqual({ typed: SPLASH.word.length, cursor: true });
    expect(splashSteps(true)).toEqual([]);
    expect(splashDuration(true)).toBeLessThan(splashDuration());
  });
});
