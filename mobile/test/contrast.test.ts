import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { TEXT_ON, TINTED_TEXT, colors, contrast, over, tints } from "../src/ui/palette";

describe("text contrast (WCAG AA, 4.5:1)", () => {
  const pairs = Object.entries(TEXT_ON).flatMap(([ground, fgs]) => fgs.map((fg) => [fg, ground as keyof typeof colors] as const));

  it.each(pairs)("%s on %s", (fg, ground) => {
    expect(contrast(colors[fg], colors[ground])).toBeGreaterThanOrEqual(4.5);
  });

  it.each(TINTED_TEXT.map((t) => [t.fg, t.tint, t.ground] as const))("%s on %s over %s", (fg, tint, ground) => {
    expect(contrast(colors[fg], over(tints[tint], colors[ground]))).toBeGreaterThanOrEqual(4.5);
  });

  it("lays a tint over its ground", () => {
    expect(over("#ffffff80", "#000000")).toBe("#808080");
  });

  it("the Updated pill's text reads on its ground", () => {
    const view = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
    const fg = /ageText: \{[^}]*color: colors\.(\w+)/.exec(view)![1] as keyof typeof colors;
    expect(/\bage: \{[^}]*backgroundColor: colors\.chrome/.test(view)).toBe(true);
    // a strip above the screen: floating, it covered the start of the program's first row
    expect(/\bage: \{[^}]*position: "absolute"/.test(view)).toBe(false);
    expect(contrast(colors[fg], colors.chrome)).toBeGreaterThanOrEqual(4.5);
  });

  it("computes WCAG ratios (black/white 21, equal colors 1)", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#569cd6", "#569cd6")).toBe(1);
  });

  it("the excluded pair really falls short (so it stays excluded)", () => {
    expect(contrast(colors.textDim, colors.surface)).toBeLessThan(4.5);
    expect(TEXT_ON.surface).not.toContain("textDim");
  });
});
