import { describe, expect, it } from "vitest";
import { TEXT_ON, TINTED_TEXT, colors, contrast, over, tints } from "../src/ui/palette";

describe("text contrast (WCAG AA, 4.5:1)", () => {
  const pairs = Object.entries(TEXT_ON).flatMap(([ground, fgs]) => fgs.map((fg) => [fg, ground as keyof typeof colors] as const));

  it.each(pairs)("%s on %s", (fg, ground) => {
    expect(contrast(colors[fg], colors[ground])).toBeGreaterThanOrEqual(4.5);
  });

  it.each(TINTED_TEXT.map((t) => [t.fg, t.tint, t.ground] as const))("%s on %s over %s", (fg, tint, ground) => {
    expect(contrast(colors[fg], over(tints[tint], colors[ground]))).toBeGreaterThanOrEqual(4.5);
  });

  it("the mockups' own accent tint (22) falls short on a surface, so the app uses 1a", () => {
    expect(contrast(colors.accent, over("#569cd622", colors.surface))).toBeLessThan(4.5);
    expect(over("#ffffff80", "#000000")).toBe("#808080");
  });

  it("computes WCAG ratios (black/white 21, equal colors 1)", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#569cd6", "#569cd6")).toBe(1);
  });

  it("the excluded pairs really fall short (so they stay excluded)", () => {
    expect(contrast(colors.error, colors.surface)).toBeLessThan(4.5);
    expect(contrast(colors.error, colors.bg)).toBeLessThan(4.5);
    expect(contrast(colors.textDim, colors.surface)).toBeLessThan(4.5);
    expect(TEXT_ON.surface).not.toContain("error");
    expect(TEXT_ON.surface).not.toContain("textDim");
    expect(TEXT_ON.bg).not.toContain("error");
  });
});
