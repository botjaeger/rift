import { describe, expect, it, vi } from "vitest";
import * as controlKeys from "../src/controlKeys";
import { stubColors, themeStub } from "./helpers/themeStub";
import { findAll, load, render, textOf, uiDeps } from "./helpers/render";

// Settings › Keys (the New Settings board), run as the app's own page: the count in the header, the
// preview being the key block, the list's order with remove in red, and Add a key with its search.
// Dragging and the keyboard stay device checks.

function draw(ids: string[]) {
  const updateSettings = vi.fn();
  const handle = { grip: (n: unknown) => n, accessibilityActions: [{ name: "moveUp", label: "Move up" }], onAccessibilityAction: vi.fn() };
  const deps = uiDeps("ios", {
    "expo-router": { useRouter: () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true }) },
    "@/controlKeys": controlKeys,
    "@/runtime": { fleet: { updateSettings }, useApp: () => ({ settings: { controlKeys: ids } }) },
    "@/ui/Button": { Button: "Button" },
    "@/ui/ControlRow": { ControlRow: "ControlRow", KeyCap: "KeyCap", NO_STICKY: {} },
    "@/ui/Field": { Field: "Field" },
    "@/ui/Icon": { Icon: "Icon" },
    "@/ui/ReorderRow": { ReorderRow: (p: { children: (h: unknown) => unknown }) => p.children(handle), useReorder: () => ({ drag: { active: null, slot: 0 }, setDrag: () => {}, scrollEnabled: true }) },
    "@/ui/SaveError": { SaveError: "SaveError" },
    "@/ui/Screen": { PageHeader: "PageHeader", SectionTitle: "SectionTitle" },
    "@/ui/theme": themeStub,
  });
  const page = load("app/control-keys.tsx", deps) as { default: () => unknown };
  return { tree: render({ type: page.default, props: { children: [] } }), updateSettings, handle };
}
const one = (tree: unknown, type: string) => findAll(tree, (n) => n.type === type);
const pressable = (tree: unknown, label: string) => findAll(tree, (n) => n.type === "Pressable" && n.props.accessibilityLabel === label)[0]!;

describe("Settings › Keys", () => {
  it("counts the block's places with the keyboard key, and previews the key block itself", () => {
    const { tree } = draw([...controlKeys.DEFAULT_CONTROL_KEYS]);
    expect(one(tree, "PageHeader")[0]!.props.subtitle).toBe("5 keys · room for 2 more");
    const preview = one(tree, "ControlRow")[0]!;
    expect(preview.props.ids).toEqual(controlKeys.DEFAULT_CONTROL_KEYS);
    expect(preview.props.tipBelow).toBe(true);
    const titles = one(tree, "SectionTitle").map((n) => textOf(n));
    expect(titles).toEqual(["Preview", "In the key block · drag ≡ to reorder", "Add a key"]);
  });

  it("lists the block's keys in order, each removable in red, and moved by its grip or a screen reader", () => {
    const { tree, updateSettings, handle } = draw(["esc", "tab", "ctrl"]);
    const rows = findAll(tree, (n) => typeof n.props.accessibilityLabel === "string" && /position \d of 3$/.test(n.props.accessibilityLabel as string));
    expect(rows.map((r) => r.props.accessibilityLabel)).toEqual(["Escape, Esc, position 1 of 3", "Tab, Tab, position 2 of 3", "Control, held for the next key, ⌃, position 3 of 3"]);
    expect((rows[0]!.props.accessibilityActions as { name: string }[]).map((a) => a.name)).toEqual(["moveUp", "remove"]);
    const remove = pressable(tree, "Remove Tab");
    expect(one(remove, "Icon")[0]!.props.color).toBe(stubColors.error);
    (remove.props.onPress as () => void)();
    expect(updateSettings).toHaveBeenLastCalledWith({ controlKeys: ["esc", "ctrl"] });
    // a screen reader's Remove, and its Move up through the shared reorder row
    const act = (name: string) => (rows[1]!.props.onAccessibilityAction as (e: unknown) => void)({ nativeEvent: { actionName: name } });
    act("remove");
    expect(updateSettings).toHaveBeenCalledTimes(2);
    expect(updateSettings).toHaveBeenLastCalledWith({ controlKeys: ["esc", "ctrl"] });
    act("moveUp");
    expect(handle.onAccessibilityAction).toHaveBeenCalledTimes(1);
  });

  it("Add a key: a search field and every key not in the block nor on the arrow pad", () => {
    const { tree, updateSettings } = draw(["esc"]);
    const field = one(tree, "Field")[0]!;
    expect(field.props.label).toBe("Search keys");
    expect(field.props.icon).toBe("search");
    const adds = findAll(tree, (n) => n.type === "Pressable" && String(n.props.accessibilityLabel).startsWith("Add "));
    expect(adds.map((a) => a.props.accessibilityLabel)).not.toContain("Add Up arrow");
    expect(adds.map((a) => a.props.accessibilityLabel)).toContain("Add Paste the clipboard");
    (pressable(tree, "Add Tab").props.onPress as () => void)();
    expect(updateSettings).toHaveBeenLastCalledWith({ controlKeys: ["esc", "tab"] });
  });

  it("a full block says so and adds nothing", () => {
    const { tree } = draw(["esc", "tab", "ctrl", "enter", "paste", "alt", "shift"]);
    expect(one(tree, "PageHeader")[0]!.props.subtitle).toBe("7 keys · the block is full");
    expect(one(tree, "SectionTitle").map((n) => textOf(n))).toContain("Add a key · the block is full: remove one first");
    expect(pressable(tree, "Add Shift Tab").props.disabled).toBe(true);
  });
});
