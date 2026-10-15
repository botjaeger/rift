import { describe, expect, it } from "vitest";
import { AVATAR_COLORS, avatarFor } from "../src/avatar";

describe("avatarFor", () => {
  it("gives a folder's panes one colour, wherever the path starts", () => {
    const a = avatarFor("/Users/me/zzz/rift", "tab", null);
    expect(avatarFor("~/zzz/rift", "other", "cargo")).toEqual(a);
    expect(AVATAR_COLORS).toContain(a.color);
  });

  it("spreads folders over the colours", () => {
    const names = ["rift", "flow", "harbor", "mobile", "quill", "ember", "orbit", "shift"];
    const used = new Set(names.map((n) => avatarFor(`~/${n}`, n, null).color));
    expect(used.size).toBeGreaterThan(3);
  });

  it("falls back to the title without a folder", () => {
    expect(avatarFor(null, "Tab 3", null)).toEqual(avatarFor(undefined, "Tab 3", null));
  });

  it("marks Claude Code with ✳", () => {
    expect(avatarFor("~/flow", "flow", "claude").glyph).toBe("\u2733\uFE0E");
    expect(avatarFor("~/flow", "flow", "codex").glyph).toBeNull();
  });
});
