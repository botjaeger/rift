import { describe, expect, it } from "vitest";
import { CONTROL_KEYS, DEFAULT_CONTROL_KEYS, controlKey, moveKey, toggleKey, validControlKeys } from "../src/controlKeys";
import { isKeyName } from "../src/protocol/types";

describe("control keys", () => {
  it("every key in the catalogue is a protocol key or a sticky modifier; ids are unique", () => {
    for (const k of CONTROL_KEYS) expect(k.sticky ? ["ctrl", "alt"].includes(k.id) : isKeyName(k.id)).toBe(true);
    expect(new Set(CONTROL_KEYS.map((k) => k.id)).size).toBe(CONTROL_KEYS.length);
    for (const id of ["ctrl", "alt", "ctrl+c", "esc", "tab", "shift+tab", "home", "end", "pageup", "pagedown", "delete", "ctrl+d", "ctrl+z", "ctrl+l", "ctrl+r", "f1", "f12"])
      expect(controlKey(id)).toBeDefined();
  });

  it("the default row is the Phase 2 one", () => {
    expect(DEFAULT_CONTROL_KEYS.map((id) => controlKey(id)!.label)).toEqual(["^C", "esc", "tab", "ctrl", "↑", "↓", "←", "→"]);
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
