import { afterEach, describe, expect, it, vi } from "vitest";
import { OtaChecks, otaChecker, shouldOfferRestart, type OtaUpdates } from "../src/ota";

const updates = (available: boolean | (() => Promise<{ isAvailable: boolean }>)): OtaUpdates & { fetched: number } => {
  const u = {
    fetched: 0,
    checkForUpdateAsync: typeof available === "function" ? available : async () => ({ isAvailable: available }),
    fetchUpdateAsync: async () => {
      u.fetched += 1;
    },
  };
  return u;
};

describe("otaChecker", () => {
  afterEach(() => vi.useRealTimers());

  it("downloads an available update", async () => {
    const u = updates(true);
    expect(await otaChecker(u)()).toBe("downloaded");
    expect(u.fetched).toBe(1);
  });

  it("downloads nothing when the build is current", async () => {
    const u = updates(false);
    expect(await otaChecker(u)()).toBe("none");
    expect(u.fetched).toBe(0);
  });

  it("gives up on a check that does not answer, and downloads nothing after", async () => {
    vi.useFakeTimers();
    let answer: (v: { isAvailable: boolean }) => void = () => undefined;
    const u = updates(() => new Promise((r) => (answer = r)));
    const run = otaChecker(u, 2_500)();
    await vi.advanceTimersByTimeAsync(2_500);
    expect(await run).toBe("failed");
    answer({ isAvailable: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(u.fetched).toBe(0);
  });

  it("reports a failed check or download without throwing", async () => {
    const u = updates(async () => {
      throw new Error("offline");
    });
    expect(await otaChecker(u)()).toBe("failed");
    const v = updates(true);
    v.fetchUpdateAsync = async () => {
      throw new Error("disk full");
    };
    expect(await otaChecker(v)()).toBe("failed");
  });

  it("runs one check at a time, and again once it finished", async () => {
    let answer: (v: { isAvailable: boolean }) => void = () => undefined;
    const u = updates(() => new Promise((r) => (answer = r)));
    const check = otaChecker(u);
    const first = check();
    expect(await check()).toBe("busy");
    answer({ isAvailable: true });
    expect(await first).toBe("downloaded");
    u.checkForUpdateAsync = async () => ({ isAvailable: false });
    expect(await check()).toBe("none");
    expect(u.fetched).toBe(1);
  });
});

describe("OtaChecks", () => {
  it("reports checking, then the outcome with its time", async () => {
    let answer: (v: { isAvailable: boolean }) => void = () => undefined;
    const u = updates(() => new Promise((r) => (answer = r)));
    const checks = new OtaChecks(otaChecker(u), () => 42);
    const seen: string[] = [];
    checks.subscribe(() => seen.push(checks.getSnapshot().state));
    const run = checks.run();
    expect(checks.getSnapshot()).toEqual({ state: "checking", at: null });
    // a second run (Check now during an automatic check) leaves the report to the first
    const again = checks.run();
    answer({ isAvailable: false });
    expect(await run).toBe("none");
    expect(await again).toBe("busy");
    expect(checks.getSnapshot()).toEqual({ state: "none", at: 42 });
    expect(seen.at(-1)).toBe("none");
  });

  it("does nothing in a build that cannot update", async () => {
    const checks = new OtaChecks(null);
    expect(checks.enabled).toBe(false);
    expect(await checks.run()).toBeNull();
    expect(checks.getSnapshot().state).toBe("idle");
  });
});

describe("shouldOfferRestart", () => {
  it("offers each downloaded update once", () => {
    const offered = new Set<string>();
    expect(shouldOfferRestart(true, "u1", false, offered)).toBe(true);
    expect(shouldOfferRestart(true, "u1", false, offered)).toBe(false);
    expect(shouldOfferRestart(true, "u2", false, offered)).toBe(true);
  });

  it("waits for the unlock, and for a downloaded update", () => {
    const offered = new Set<string>();
    expect(shouldOfferRestart(true, "u1", true, offered)).toBe(false);
    expect(shouldOfferRestart(false, "u1", false, offered)).toBe(false);
    expect(shouldOfferRestart(true, undefined, false, offered)).toBe(false);
    expect(shouldOfferRestart(true, "u1", false, offered)).toBe(true);
  });
});
