import { afterEach, describe, expect, it, vi } from "vitest";
import * as controlKeys from "../src/controlKeys";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, render, uiDeps, type Node } from "./helpers/render";

// The key block and arrow pad (the Keys redesign board), run as the app's own ControlRow: the keys
// it draws for a stored list, their order, the lit modifier and its announcement, and a held
// arrow's repeat. Pixels and touch stay device checks.

const announce = vi.fn();
function draw(props: Record<string, unknown>, edit?: (s: string) => string) {
  const deps = uiDeps("ios", {
    "@/controlKeys": controlKeys,
    "../../modules/rift-keys": { hasSystemPasteButton: false, SystemPasteButton: "SystemPasteButton" },
  });
  deps["react-native"] = {
    ...(deps["react-native"] as object),
    AccessibilityInfo: { announceForAccessibility: announce },
    AppState: { addEventListener: () => ({ remove: () => {} }) },
  };
  const { ControlRow, NO_STICKY } = load("ui/ControlRow.tsx", deps, edit) as { ControlRow: (p: object) => unknown; NO_STICKY: object };
  const onKey = vi.fn();
  const onSticky = vi.fn();
  const tree = render({
    type: ControlRow,
    props: { ids: controlKeys.DEFAULT_CONTROL_KEYS, sticky: NO_STICKY, disabled: false, onSticky, onKey, onPaste: () => {}, onShowKeyboard: () => {}, children: [], ...props },
  });
  return { tree, onKey, onSticky };
}
const toolbar = (tree: unknown, label: string) => findAll(tree, (n) => n.props.accessibilityRole === "toolbar" && n.props.accessibilityLabel === label)[0]!;
const keysOf = (tree: unknown) => findAll(tree, (n) => n.type === "Pressable").map((n) => n.props.accessibilityLabel as string);
const rowsOf = (tree: unknown) => (toolbar(tree, "Keys").props.children as Node[][]).flat().map((row) => keysOf(row));
const byLabel = (tree: unknown, label: string) => findAll(tree, (n) => n.type === "Pressable" && n.props.accessibilityLabel === label)[0]!;

describe("the key block", () => {
  afterEach(() => vi.useRealTimers());

  it("is the six default keys in two rows of three, the keyboard key last, beside the arrow pad", () => {
    const { tree } = draw({});
    expect(rowsOf(tree)).toEqual([
      ["Escape", "Tab", "Control, held for the next key"],
      ["Return", "Paste the clipboard", "Show keyboard"],
    ]);
    // ↑ above ← ↓ →
    expect(keysOf(toolbar(tree, "Arrow keys"))).toEqual(["Up arrow", "Left arrow", "Down arrow", "Right arrow"]);
  });

  it("is 88 pt high: two 41 pt rows 6 pt apart, the pad 132 x 88", () => {
    const { tree } = draw({});
    const block = findAll(tree, (n) => n.type === "View" && flat(n.props.style).height === 88);
    expect(block.length).toBeGreaterThanOrEqual(2);
    const pad = toolbar(tree, "Arrow keys");
    expect(flat(pad.props.style)).toMatchObject({ width: 132, height: 88, gap: 4 });
    expect(flat(toolbar(tree, "Keys").props.style).gap).toBe(6);
    const cells = findAll(toolbar(tree, "Keys"), (n) => n.type === "Pressable").map((n) => flat(n.props.style).height);
    expect(new Set(cells)).toEqual(new Set([41]));
  });

  it("never puts an arrow in the block; a longer list widens the block and keeps two rows", () => {
    const { tree } = draw({ ids: ["esc", "up", "tab", "ctrl", "alt", "enter", "paste", "ctrl+c"] });
    const rows = rowsOf(tree);
    expect(rows).toHaveLength(2);
    expect(rows.flat()).not.toContain("Up arrow");
    // 7 keys and the keyboard key: 4 columns
    expect(rows.map((r) => r.length)).toEqual([4, 4]);
    expect(rows[1]!.at(-1)).toBe("Show keyboard");
  });

  it("the keyboard key hides the keyboard while it is up", () => {
    const hide = vi.fn();
    const { tree } = draw({ onHideKeyboard: hide });
    expect(rowsOf(tree).flat().at(-1)).toBe("Hide keyboard");
    (byLabel(tree, "Hide keyboard").props.onPress as () => void)();
    expect(hide).toHaveBeenCalledTimes(1);
  });

  it("a held modifier is lit and announced as selected; pressing it says so", () => {
    const off = draw({});
    const ctrl = byLabel(off.tree, "Control, held for the next key");
    expect(ctrl.props.accessibilityState).toMatchObject({ checked: false, selected: false });
    (ctrl.props.onPressIn as () => void)();
    (ctrl.props.onPress as () => void)();
    expect(off.onSticky).toHaveBeenCalledWith({ ctrl: true, alt: false, shift: false });
    expect(announce).toHaveBeenLastCalledWith("Control held for the next key");

    const on = draw({ sticky: { ctrl: true, alt: false, shift: false } });
    const lit = byLabel(on.tree, "Control, held for the next key");
    expect(lit.props.accessibilityState).toMatchObject({ checked: true, selected: true });
    const face = findAll(lit, (n) => n.type === "View")[0]!;
    expect(flat(face.props.style).backgroundColor).toBe(stubColors.accent);
    expect(findAll(lit, (n) => n.type === "Icon")[0]!.props.color).toBe(stubColors.onAccent);
    // the others stay unlit
    const esc = findAll(byLabel(on.tree, "Escape"), (n) => n.type === "View")[0]!;
    expect(flat(esc.props.style).backgroundColor).toBe(stubColors.surface);
  });

  it("a held arrow repeats until it is let go; a screen reader's activation sends it once", () => {
    vi.useFakeTimers();
    const { tree, onKey } = draw({});
    const up = byLabel(tree, "Up arrow");
    expect(up.props.accessibilityHint).toContain("Hold to repeat");
    (up.props.onPressIn as () => void)();
    expect(onKey).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(controlKeys.REPEAT_DELAY_MS + controlKeys.REPEAT_EVERY_MS * 2);
    expect(onKey).toHaveBeenCalledTimes(3);
    (up.props.onPressOut as () => void)();
    (up.props.onPress as () => void)();
    vi.advanceTimersByTime(1000);
    expect(onKey).toHaveBeenCalledTimes(3);
    expect(onKey.mock.calls.every((c) => c[0] === "up")).toBe(true);
    const down = byLabel(tree, "Down arrow");
    (down.props.onPress as () => void)();
    expect(onKey).toHaveBeenLastCalledWith("down");
  });

  it("a disabled block sends nothing", () => {
    const { tree } = draw({ disabled: true });
    for (const label of ["Escape", "Up arrow", "Control, held for the next key"]) expect(byLabel(tree, label).props.disabled).toBe(true);
  });
});
