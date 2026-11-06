import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ARROW_KEYS,
  BLOCK_SLOTS,
  CONTROL_KEYS,
  DEFAULT_CONTROL_KEYS,
  KeyRepeat,
  MAX_ROW_KEYS,
  REPEAT_DELAY_MS,
  REPEAT_EVERY_MS,
  ROW_DEFAULT_CONTROL_KEYS,
  blockColumns,
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
    expect(addChoices(DEFAULT_CONTROL_KEYS).map((c) => c.label)).toEqual(expect.arrayContaining(["+Tab", "+Pane"]));
    expect(addChoices(["esc"]).map((c) => c.label)).toContain("Paste");
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

  it("Add a key: every key not in the block and not on the arrow pad, found by a search", () => {
    const add = addChoices(DEFAULT_CONTROL_KEYS);
    expect(add.map((c) => c.label).slice(0, 4)).toEqual(["⌃C", "⇧Tab", "New line", "⌥"]);
    // the arrows are the pad's: never offered, never added
    for (const id of ARROW_KEYS) expect(add.some((c) => c.ids[0] === id)).toBe(false);
    expect(toggleKey(["esc"], "up", true)).toEqual(["esc"]);
    expect(add.some((c) => c.label === "F1")).toBe(true);
    expect(addChoices([...DEFAULT_CONTROL_KEYS, "f5"]).filter((c) => /^F\d+$/.test(c.label))).toHaveLength(11);
    expect(addChoices(CONTROL_KEYS.map((k) => k.id))).toEqual([]);
    // the search: by name, short word or id, any case; nothing when nothing matches
    expect(addChoices(DEFAULT_CONTROL_KEYS, "back tab").map((c) => c.ids[0])).toEqual(["shift+tab"]);
    expect(addChoices(DEFAULT_CONTROL_KEYS, "PgUp").map((c) => c.ids[0])).toEqual(["pageup"]);
    expect(addChoices(DEFAULT_CONTROL_KEYS, "suspend").map((c) => c.ids[0])).toEqual(["ctrl+z"]);
    expect(addChoices(DEFAULT_CONTROL_KEYS, "  ")).toEqual(add);
    expect(addChoices(DEFAULT_CONTROL_KEYS, "zzz")).toEqual([]);
  });

  it("the default block: Esc Tab ⌃ Return Paste, then the keyboard key; 8 places", () => {
    expect(DEFAULT_CONTROL_KEYS).toEqual(["esc", "tab", "ctrl", "enter", "paste"]);
    expect(DEFAULT_CONTROL_KEYS.map((id) => controlKey(id)!.label)).toEqual(["Esc", "Tab", "⌃", "↵", "Paste"]);
    expect(BLOCK_SLOTS).toBe(8);
    expect(MAX_ROW_KEYS).toBe(BLOCK_SLOTS - 1);
    expect(DEFAULT_CONTROL_KEYS.length).toBeLessThanOrEqual(MAX_ROW_KEYS);
    // New line stays in Add a key
    expect(addChoices(DEFAULT_CONTROL_KEYS).some((c) => c.label === "New line")).toBe(true);
    expect(ARROW_KEYS).toEqual(["up", "left", "down", "right"]);
  });

  it("the block keeps two rows: three columns for six keys, more for more", () => {
    expect(blockColumns(6)).toBe(3);
    expect(blockColumns(1)).toBe(3);
    expect(blockColumns(7)).toBe(4);
    expect(blockColumns(8)).toBe(4);
    expect(blockColumns(9)).toBe(5);
  });

  it("a full block takes no more keys", () => {
    const full = ["esc", "tab", "ctrl", "enter", "paste", "ctrl+c", "alt"];
    expect(canAddKey(full)).toBe(false);
    expect(toggleKey(full, "f5", true)).toEqual(full);
    expect(toggleKey(full, "esc", false)).toHaveLength(6);
    expect(canAddKey(toggleKey(full, "esc", false))).toBe(true);
  });

  it("migrates every earlier default to the key block's; a customised list keeps its keys, the arrows go to the pad", () => {
    expect(ROW_DEFAULT_CONTROL_KEYS).toEqual(["esc", "tab", "ctrl+c", "ctrl", "up", "down", "enter"]);
    expect(validControlKeys([...ROW_DEFAULT_CONTROL_KEYS])).toEqual([...DEFAULT_CONTROL_KEYS]);
    expect(OLD_DEFAULT_CONTROL_KEYS).toEqual(["ctrl+c", "esc", "tab", "ctrl", "up", "down"]);
    expect(validControlKeys([...OLD_DEFAULT_CONTROL_KEYS])).toEqual([...DEFAULT_CONTROL_KEYS]);
    // junk around the old default is dropped first, so it is still the old default
    expect(validControlKeys(["ctrl+c", "esc", "nope", "tab", "ctrl", "up", "down"])).toEqual([...DEFAULT_CONTROL_KEYS]);
    // the default with the arrows moves too; the same keys in another order keep theirs, the arrows on the pad
    expect(validControlKeys(["esc", "tab", "ctrl+c", "ctrl", "left", "up", "down", "right"])).toEqual([...DEFAULT_CONTROL_KEYS]);
    expect(validControlKeys(["tab", "esc", "ctrl+c", "ctrl", "left", "up", "down", "right"])).toEqual(["tab", "esc", "ctrl+c", "ctrl"]);
    // the default that held New line moves too; Return and New line kept by choice stay
    expect(validControlKeys(["esc", "tab", "ctrl+c", "ctrl", "up", "down", "enter", "ctrl+j"])).toEqual([...DEFAULT_CONTROL_KEYS]);
    expect(validControlKeys(["esc", "tab", "ctrl+c", "ctrl", "up", "down", "ctrl+j", "enter"])).toEqual(["esc", "tab", "ctrl+c", "ctrl", "ctrl+j", "enter"]);
    // customised: one key more, one fewer, another order
    expect(validControlKeys([...OLD_DEFAULT_CONTROL_KEYS, "left"])).toEqual(["ctrl+c", "esc", "tab", "ctrl"]);
    expect(validControlKeys(["ctrl+c", "esc", "tab", "ctrl", "up"])).toEqual(["ctrl+c", "esc", "tab", "ctrl"]);
    expect(validControlKeys(["esc", "ctrl+c", "tab", "ctrl", "up", "down"])).toEqual(["esc", "ctrl+c", "tab", "ctrl"]);
    expect(validControlKeys([])).toEqual([]);
    // the new default stays itself
    expect(validControlKeys([...DEFAULT_CONTROL_KEYS])).toEqual([...DEFAULT_CONTROL_KEYS]);
    // a longer stored list (before the maximum) keeps every key
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
  it("is in the block wherever an image can be pasted, whatever the stored list holds", async () => {
    const { rowWithPaste } = await import("../src/controlKeys");
    // the default block has Paste: nothing to add
    expect(rowWithPaste(DEFAULT_CONTROL_KEYS, true)).toBe(DEFAULT_CONTROL_KEYS);
    // a list without it gets it at its end where uploads work
    expect(rowWithPaste(["esc", "tab"], true)).toEqual(["esc", "tab", "paste"]);
    // never twice, and where the user put it
    expect(rowWithPaste(["paste", "esc"], true)).toEqual(["paste", "esc"]);
    // without uploads (an old Rift) or on a view-only device the list is as stored
    expect(rowWithPaste(["esc"], false)).toEqual(["esc"]);
    expect(rowWithPaste([], true)).toEqual(["paste"]);
  });
});

describe("a held arrow", () => {
  afterEach(() => vi.useRealTimers());
  it("goes at once, repeats after the delay, and stops when let go", () => {
    vi.useFakeTimers();
    const r = new KeyRepeat();
    const fire = vi.fn();
    r.start(fire);
    expect(fire).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(REPEAT_DELAY_MS - 1);
    expect(fire).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1 + REPEAT_EVERY_MS * 3);
    expect(fire).toHaveBeenCalledTimes(4);
    expect(r.held).toBe(true);
    r.stop();
    // the press after the touch (same release, as Pressability sends it) sends nothing more
    r.tap(fire);
    vi.advanceTimersByTime(REPEAT_EVERY_MS * 10);
    expect(fire).toHaveBeenCalledTimes(4);
    expect(r.held).toBe(false);
  });
  it("a cancelled touch (finger slid off, no press) does not swallow the next screen reader activation", () => {
    vi.useFakeTimers();
    const r = new KeyRepeat();
    const fire = vi.fn();
    r.start(fire);
    r.stop();
    vi.advanceTimersByTime(1);
    r.tap(fire);
    expect(fire).toHaveBeenCalledTimes(2);
  });
  it("a quick tap is one key; a let-go before the delay never repeats", () => {
    vi.useFakeTimers();
    const r = new KeyRepeat();
    const fire = vi.fn();
    r.start(fire);
    r.stop();
    r.tap(fire);
    vi.advanceTimersByTime(REPEAT_DELAY_MS * 5);
    expect(fire).toHaveBeenCalledTimes(1);
  });
  it("a screen reader's activation (no touch) sends once; a new touch restarts cleanly", () => {
    vi.useFakeTimers();
    const r = new KeyRepeat();
    const fire = vi.fn();
    r.tap(fire);
    expect(fire).toHaveBeenCalledTimes(1);
    r.start(fire);
    r.start(fire); // a second touch before the first ended: one repeat, not two
    vi.advanceTimersByTime(REPEAT_DELAY_MS + REPEAT_EVERY_MS);
    expect(fire).toHaveBeenCalledTimes(4);
    r.stop();
  });
});
