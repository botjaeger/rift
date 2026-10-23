import { describe, expect, it } from "vitest";
import {
  CONTROL_KEYS,
  DEFAULT_CONTROL_KEYS,
  MAX_ROW_KEYS,
  OLD_DEFAULT_CONTROL_KEYS,
  PressGate,
  addChoices,
  canAddKey,
  controlKey,
  keyTipTitle,
  moveKey,
  moveKeyTo,
  toggleKey,
  validControlKeys,
} from "../src/controlKeys";
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

  it("the Add chips: every unused key (the row holds 8, so the F-keys are not a group)", () => {
    const add = addChoices(DEFAULT_CONTROL_KEYS);
    expect(add.map((c) => c.label).slice(0, 4)).toEqual(["⇧Tab", "⌥", "⇧", "Home"]);
    expect(add.some((c) => c.label === "F1")).toBe(true);
    expect(add.some((c) => c.label === "F1–F12")).toBe(false);
    expect(addChoices([...DEFAULT_CONTROL_KEYS, "f5"]).filter((c) => /^F\d+$/.test(c.label))).toHaveLength(11);
    expect(addChoices(CONTROL_KEYS.map((k) => k.id))).toEqual([]);
  });

  it("the default row: Esc Tab ⌃C ⌃ ← ↑ ↓ →, at most 8 keys", () => {
    expect(DEFAULT_CONTROL_KEYS).toEqual(["esc", "tab", "ctrl+c", "ctrl", "left", "up", "down", "right"]);
    expect(DEFAULT_CONTROL_KEYS.map((id) => controlKey(id)!.label)).toEqual(["Esc", "Tab", "⌃C", "⌃", "←", "↑", "↓", "→"]);
    expect(DEFAULT_CONTROL_KEYS).toHaveLength(MAX_ROW_KEYS);
  });

  it("a full row takes no more keys", () => {
    expect(canAddKey(DEFAULT_CONTROL_KEYS)).toBe(false);
    expect(toggleKey(DEFAULT_CONTROL_KEYS, "f5", true)).toEqual([...DEFAULT_CONTROL_KEYS]);
    expect(toggleKey(DEFAULT_CONTROL_KEYS, "esc", false)).toHaveLength(7);
    expect(canAddKey(toggleKey(DEFAULT_CONTROL_KEYS, "esc", false))).toBe(true);
  });

  it("migrates a stored row equal to the old default; a customised row is kept", () => {
    expect(OLD_DEFAULT_CONTROL_KEYS).toEqual(["ctrl+c", "esc", "tab", "ctrl", "up", "down"]);
    expect(validControlKeys([...OLD_DEFAULT_CONTROL_KEYS])).toEqual([...DEFAULT_CONTROL_KEYS]);
    // junk around the old default is dropped first, so it is still the old default
    expect(validControlKeys(["ctrl+c", "esc", "nope", "tab", "ctrl", "up", "down"])).toEqual([...DEFAULT_CONTROL_KEYS]);
    // customised: one key more, one fewer, another order
    expect(validControlKeys([...OLD_DEFAULT_CONTROL_KEYS, "left"])).toEqual([...OLD_DEFAULT_CONTROL_KEYS, "left"]);
    expect(validControlKeys(["ctrl+c", "esc", "tab", "ctrl", "up"])).toEqual(["ctrl+c", "esc", "tab", "ctrl", "up"]);
    expect(validControlKeys(["esc", "ctrl+c", "tab", "ctrl", "up", "down"])).toEqual(["esc", "ctrl+c", "tab", "ctrl", "up", "down"]);
    expect(validControlKeys([])).toEqual([]);
    // a longer stored row (before the maximum) is kept as it is
    const long = ["esc", "tab", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8"];
    expect(validControlKeys(long)).toEqual(long);
  });

  it("every catalogue key names what it does (tip, a11y hint)", () => {
    for (const k of CONTROL_KEYS) {
      expect(k.tip.length, k.id).toBeGreaterThan(10);
      expect(keyTipTitle(k), k.id).toContain(k.label);
    }
    expect(keyTipTitle(controlKey("ctrl+c")!)).toBe("⌃C · Interrupt");
    expect(controlKey("ctrl+c")!.tip).toBe("Sends Ctrl+C: stops the running command");
  });

  it("a long press shows the tip and does not send; a tap sends", () => {
    const sent: string[] = [];
    const gate = new PressGate();
    const press = (long: boolean) => {
      gate.begin();
      if (long) gate.longPress();
      // the release fires onPress on a platform that does not cancel it after a long press
      if (gate.release() === "send") sent.push("x");
    };
    press(true);
    expect(sent).toEqual([]);
    // the next, short touch is a tap again (the long one left nothing behind)
    press(false);
    expect(sent).toEqual(["x"]);
    press(true);
    press(true);
    expect(sent).toEqual(["x"]);
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

describe("Paste where images go", () => {
  it("is on the row wherever an image can be pasted, whatever the stored row holds", async () => {
    const { rowWithPaste } = await import("../src/controlKeys");
    // the default row has no Paste: a screenshot needs it, so it is added where uploads work
    expect(DEFAULT_CONTROL_KEYS).not.toContain("paste");
    expect(rowWithPaste(DEFAULT_CONTROL_KEYS, true)).toEqual([...DEFAULT_CONTROL_KEYS, "paste"]);
    // never twice, and where the user put it
    expect(rowWithPaste(["paste", "esc"], true)).toEqual(["paste", "esc"]);
    // without uploads (an old Rift) or on a view-only device the row is as stored
    expect(rowWithPaste(DEFAULT_CONTROL_KEYS, false)).toBe(DEFAULT_CONTROL_KEYS);
    expect(rowWithPaste([], true)).toEqual(["paste"]);
  });
});
