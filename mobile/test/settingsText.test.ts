import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { BUILTIN_THEMES } from "../src/themes.data";
import { DARK_DEFAULT, contrast, over, paletteFrom } from "../src/ui/palette";

const colors = paletteFrom(DARK_DEFAULT);
import { hostMeta, placeWord, notifyExample, ntfySummary, shortAfter, unpairFooter, updateLine } from "../src/ui/settingsText";

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

  it("placeWord and hostMeta say where a computer is and what this phone may do there", () => {
    expect(placeWord(lan)).toBe("This network");
    expect(placeWord(relay)).toBe("Over the relay");
    expect(placeWord(away)).toBe("Can’t reach it");
    expect(placeWord({ status: "offline", path: null })).toBe("Offline");
    expect(placeWord({ status: "connecting", path: null })).toBe("Connecting…");
    expect(placeWord({ status: "failed", path: null })).toBe("Can’t reach it");
    expect(hostMeta({ conn: lan, viewOnly: false })).toBe("This network · can run commands");
    expect(hostMeta({ conn: relay, viewOnly: true })).toBe("Over the relay · view only");
    // one it cannot reach says only that
    expect(hostMeta({ conn: { status: "closed", path: null }, viewOnly: false })).toBe("Can’t reach it");
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

  it("the chosen segment's ground keeps textBright above 4.5:1, in every builtin theme", () => {
    expect(colors.segmentOn).toBe("#373737");
    expect(rows).toContain("segmentOn: { backgroundColor: colors.segmentOn }");
    for (const t of BUILTIN_THEMES) {
      const c = paletteFrom(t);
      expect(contrast(c.textBright, c.segmentOn), t.name).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("the off track keeps the switch visible on a card and on a menu, the on track is the accent (Warp)", () => {
    // the boards' #3a3a3c on a card, made from the theme
    expect(over(colors.switchOff, colors.surface)).toBe("#3a3a3a");
    expect(rows).toContain("backgroundColor: on ? colors.accent : colors.switchOff");
    for (const t of BUILTIN_THEMES) {
      const c = paletteFrom(t);
      for (const ground of [c.surface, c.menu]) expect(contrast(over(c.switchOff, ground), ground), t.name).toBeGreaterThan(1.2);
    }
  });
});
