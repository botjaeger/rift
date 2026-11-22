import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Every ⌄ that opens something points the other way while that is open (user, 2026-11-07).
const read = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");
const pane = read("app/host/[key]/pane/[id].tsx");

describe("a chevron flips while what it opens is open", () => {
  it("has an up chevron to flip to", () => {
    expect(read("ui/Icon.tsx")).toContain('chevronUp: { ios: "chevron.up", android: "expand_less", web: "expand_less" }');
  });
  it("the computer name's ⌄ points up while the computer menu is open", () => {
    expect(pane).toContain('<Icon name={computers ? "chevronUp" : "chevronDown"} size={glyph(16, 11)}');
  });
  it("the pane picker's ⌄ points up while its Panes sheet is up, and down again once the pane has focus", () => {
    expect(pane).toContain('<Icon name={picking && !focused ? "chevronUp" : "chevronDown"} size={14}');
    expect(pane).toMatch(/setPicking\(true\);\s*router\.push\(\{ pathname: "\/panes"/);
    expect(pane).toContain("useFocusEffect(useCallback(() => setPicking(false), []));");
  });
  it("the keys button is a keyboard, in the accent while the keys show (a chevron read as scroll up)", () => {
    expect(read("ui/Composer.tsx")).toContain('<Icon name="keyboard" size={glyph(20, 17)} weight="medium" color={keys ? colors.accent : colors.text} />');
  });
});
