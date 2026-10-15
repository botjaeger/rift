import { describe, expect, it } from "vitest";
import { CONTROL_KEYS, DEFAULT_CONTROL_KEYS, addChoices, controlKey, moveKey, moveKeyTo, toggleKey, validControlKeys } from "../src/controlKeys";
import { isKeyName } from "../src/protocol/types";

describe("control keys", () => {
  it("every key in the catalogue is a protocol key or a sticky modifier; ids are unique", () => {
    for (const k of CONTROL_KEYS)
      expect(k.sticky ? ["ctrl", "alt", "shift"].includes(k.id) : k.open || k.paste ? !isKeyName(k.id) : isKeyName(k.id)).toBe(true);
    // the new tab / new pane keys open on the computer, they are never sent as keys
    expect(CONTROL_KEYS.filter((k) => k.open).map((k) => [k.id, k.open, k.label])).toEqual([
      ["new_tab", "tab", "+Tab"],
      ["new_pane", "pane", "+Pane"],
    ]);
    expect(addChoices(DEFAULT_CONTROL_KEYS).map((c) => c.label)).toEqual(expect.arrayContaining(["+Tab", "+Pane", "Paste"]));
    expect(validControlKeys(["new_tab", "esc", "new_pane"])).toEqual(["new_tab", "esc", "new_pane"]);
    expect(new Set(CONTROL_KEYS.map((k) => k.id)).size).toBe(CONTROL_KEYS.length);
    for (const id of ["ctrl", "alt", "ctrl+c", "esc", "tab", "shift+tab", "home", "end", "pageup", "pagedown", "delete", "ctrl+d", "ctrl+z", "ctrl+l", "ctrl+r", "f1", "f12"])
      expect(controlKey(id)).toBeDefined();
  });

  it("drag to reorder moves a key anywhere, clamped to the row", () => {
    expect(moveKeyTo(["a", "b", "c", "d"], 0, 2)).toEqual(["b", "c", "a", "d"]);
    expect(moveKeyTo(["a", "b", "c", "d"], 3, 0)).toEqual(["d", "a", "b", "c"]);
    expect(moveKeyTo(["a", "b", "c"], 1, 9)).toEqual(["a", "c", "b"]);
    expect(moveKeyTo(["a", "b", "c"], 1, -4)).toEqual(["b", "a", "c"]);
    expect(moveKeyTo(["a", "b"], 5, 0)).toEqual(["a", "b"]);
  });

  it("the Add chips: unused keys, the F-keys as one chip while none is used", () => {
    const add = addChoices(DEFAULT_CONTROL_KEYS);
    expect(add.map((c) => c.label).slice(0, 4)).toEqual(["⇧Tab", "⌥", "⇧", "←"]);
    const f = add.find((c) => c.label === "F1–F12")!;
    expect(f.ids).toHaveLength(12);
    expect(add.some((c) => c.label === "F1")).toBe(false);
    const withF5 = addChoices([...DEFAULT_CONTROL_KEYS, "f5"]);
    expect(withF5.some((c) => c.label === "F1–F12")).toBe(false);
    expect(withF5.filter((c) => /^F\d+$/.test(c.label))).toHaveLength(11);
    expect(addChoices(CONTROL_KEYS.map((k) => k.id))).toEqual([]);
  });

  it("the default row is the board's", () => {
    expect(DEFAULT_CONTROL_KEYS.map((id) => controlKey(id)!.label)).toEqual(["⌃C", "Esc", "Tab", "⌃", "↑", "↓"]);
  });

  it("validates a stored row", () => {
    expect(validControlKeys(["esc", "esc", "nope", 3, "alt"])).toEqual(["esc", "alt"]);
    expect(validControlKeys(undefined)).toEqual(DEFAULT_CONTROL_KEYS);
    expect(validControlKeys({})).toEqual(DEFAULT_CONTROL_KEYS);
  });

  it("moves a key up or down, not past the ends", () => {
    expect(moveKey(["a", "b", "c"], 1, -1)).toEqual(["b", "a", "c"]);
    expect(moveKey(["a", "b", "c"], 1, 1)).toEqual(["a", "c", "b"]);
    expect(moveKey(["a", "b", "c"], 0, -1)).toEqual(["a", "b", "c"]);
    expect(moveKey(["a", "b", "c"], 2, 1)).toEqual(["a", "b", "c"]);
  });

  it("toggles: on adds at the end (once), off removes; unknown ids are not added", () => {
    expect(toggleKey(["esc"], "f5", true)).toEqual(["esc", "f5"]);
    expect(toggleKey(["esc", "f5"], "esc", true)).toEqual(["esc", "f5"]);
    expect(toggleKey(["esc", "f5"], "esc", false)).toEqual(["f5"]);
    expect(toggleKey(["esc"], "rm", true)).toEqual(["esc"]);
  });
});
