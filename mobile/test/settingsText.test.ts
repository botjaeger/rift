import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { colors, contrast, over } from "../src/ui/palette";
import { connWord, hostMeta, notifyExample, ntfySummary, shortAfter, unpairFooter, updateLine } from "../src/ui/settingsText";

const lan = { status: "open", path: { kind: "lan", address: "10.0.0.2:7000" } } as const;
const relay = { status: "open", path: { kind: "relay", url: "wss://r.example" } } as const;
const away = { status: "closed", path: null } as const;

describe("Settings words (the Settings board)", () => {
  it("shortAfter: 30 s, 1 min, 5 min", () => {
    expect(shortAfter(10_000)).toBe("10 s");
    expect(shortAfter(30_000)).toBe("30 s");
    expect(shortAfter(60_000)).toBe("1 min");
    expect(shortAfter(300_000)).toBe("5 min");
  });

  it("connWord and hostMeta name the path and the permission", () => {
    expect(connWord(lan)).toBe("LAN");
    expect(connWord(relay)).toBe("Relay");
    expect(connWord(away)).toBe("Offline");
    expect(connWord({ status: "connecting", path: null })).toBe("Connecting");
    expect(hostMeta({ os: "macos", conn: lan, viewOnly: false })).toBe("macOS · LAN · can run commands");
    expect(hostMeta({ os: "linux", conn: relay, viewOnly: true })).toBe("Linux · Relay · view only");
    expect(hostMeta({ os: null, conn: away, viewOnly: false })).toBe("Offline · can run commands");
  });

  it("notifyExample shows the alert for the current value, with the computer's name", () => {
    expect(notifyExample("build-box", true)).toBe("“✕ 1 · npm test” · Failed after 8.4s · build-box");
    expect(notifyExample("build-box", false)).toBe("“✕ 1 · A command” · Failed after 8.4s · build-box");
  });

  it("updateLine: checking, just checked (green), older, failed, pending", () => {
    const now = 1_000_000;
    expect(updateLine({ state: "checking", at: null }, false, now)).toEqual({ text: "Checking…", fresh: false });
    expect(updateLine({ state: "none", at: now - 5_000 }, false, now)).toEqual({ text: "Up to date · checked just now", fresh: true });
    const older = updateLine({ state: "none", at: now - 120_000 }, false, now);
    expect(older.fresh).toBe(false);
    expect(older.text).toMatch(/^Up to date/);
    expect(older.text).not.toMatch(/just now/);
    expect(updateLine({ state: "failed", at: now }, false, now).text).toMatch(/^Could not check/);
    expect(updateLine({ state: "none", at: now }, true, now)).toEqual({
      text: "An update is ready and applies when the app restarts.",
      fresh: false,
    });
  });

  it("unpairFooter: before the second tap, after it, and offline", () => {
    expect(unpairFooter(false, "Mac", true)).toBe("To pair again you need a new code from Rift on the computer.");
    expect(unpairFooter(true, "Mac", true)).toBe("Mac forgets this phone. Pairing again needs a new code.");
    expect(unpairFooter(true, "Mac", false)).toMatch(/not connected now, so remove this phone on the computer too/);
  });

  it("ntfySummary", () => {
    expect(ntfySummary(true)).toBe("On");
    expect(ntfySummary(false)).toBe("Off");
  });
});

describe("Settings board colours", () => {
  const rows = readFileSync(fileURLToPath(new URL("../src/ui/Rows.tsx", import.meta.url)), "utf8");
  const segmentOn = /SEGMENT_ON = "(#[0-9a-f]{6})"/.exec(rows)?.[1];

  it("the chosen segment's ground keeps textBright above 4.5:1", () => {
    expect(segmentOn).toBe("#373737");
    expect(contrast(colors.textBright, segmentOn!)).toBeGreaterThanOrEqual(4.5);
  });

  it("the off track keeps the switch visible on a surface, the on track is the accent (Warp)", () => {
    const off = /SWITCH_OFF = "(#[0-9a-f]{8})"/.exec(rows)?.[1];
    expect(off).toBe("#ffffff29");
    expect(contrast(over(off!, colors.surface), colors.surface)).toBeGreaterThan(1.2);
    expect(rows).toContain("true: colors.accent");
  });
});
