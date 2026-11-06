import { describe, expect, it, vi } from "vitest";
import * as selection from "../src/selection";
import { findAll, load, react, render, textOf, uiDeps } from "./helpers/render";

// The one selection bar (the "Redesign: selection" board), run as the app's own SelectionBar: the
// same keys in the same places for block output and a program's screen, the kept "✓ Copied", To
// prompt disabled with its reason, and Android's Back ending the selection. Pixels stay device checks.

function draw(props: Record<string, unknown>, edit?: (s: string) => string) {
  const back: (() => boolean)[] = [];
  const deps = uiDeps("android", { "@/selection": selection });
  // effects run at once, so the Back listener is registered
  deps.react = { ...react, useEffect: (fn: () => void) => void fn() };
  deps["react-native"] = {
    ...(deps["react-native"] as object),
    BackHandler: { addEventListener: (_e: string, fn: () => boolean) => (back.push(fn), { remove: () => {} }) },
  };
  const { SelectionBar } = load("ui/SelectionBar.tsx", deps, edit) as { SelectionBar: (p: object) => unknown };
  const onCancel = vi.fn();
  const tree = render({
    type: SelectionBar,
    props: { selected: "brown f\nox", copied: true, onWord: () => {}, onLine: () => {}, onAll: () => {}, onCancel, children: [], ...props },
  });
  return { tree, back, onCancel };
}
const keys = (tree: unknown) => findAll(tree, (n) => n.type === "Pressable");
const labels = (tree: unknown) => keys(tree).map((n) => n.props.accessibilityLabel);

describe("the selection bar", () => {
  it("has Word, Line, All, To prompt and ✕ in the same places on a block and on a program's screen", () => {
    const block = draw({ of: "block", onPrompt: () => {} }).tree;
    const screen = draw({ of: "screen", promptOff: "This device cannot type here." }).tree;
    const want = ["Word", "Line", "All", "To prompt", "End the selection"];
    expect(labels(block)).toEqual(want);
    expect(labels(screen)).toEqual(want);
    expect(textOf(block)).not.toContain("Screen");
    expect(textOf(block)).toContain("All selects the whole block.");
    expect(textOf(screen)).toContain("All selects the whole screen.");
  });

  it("says ✓ Copied on its first line while the selection is copied, else the note; no live region", () => {
    const copied = draw({ of: "block", onPrompt: () => {}, note: "Lift to copy" }).tree;
    expect(textOf(copied)).toContain("2 lines · 9 characters✓ Copied");
    expect(textOf(copied)).not.toContain("Lift to copy");
    const moving = draw({ of: "screen", copied: false, note: "Lift to copy" }).tree;
    expect(textOf(moving)).toContain("Lift to copy");
    expect(textOf(moving)).not.toContain("Copied");
    expect(findAll(copied, (n) => n.props.accessibilityLiveRegion !== undefined)).toHaveLength(0);
  });

  it("disables To prompt with the reason where it cannot go into a command line", () => {
    const { tree } = draw({ of: "screen", promptOff: "This device cannot type here." });
    const prompt = keys(tree).find((n) => n.props.accessibilityLabel === "To prompt")!;
    expect(prompt.props.disabled).toBe(true);
    expect(prompt.props.accessibilityState).toEqual({ disabled: true });
    expect(prompt.props.accessibilityHint).toBe("This device cannot type here.");
    expect(textOf(tree)).toContain("All selects the whole screen. This device cannot type here.");
    const on = draw({ of: "block", onPrompt: () => {}, promptOff: "x" }).tree;
    expect(keys(on).find((n) => n.props.accessibilityLabel === "To prompt")!.props.disabled).toBe(false);
    expect(textOf(on)).not.toContain(" x");
  });

  it("ends the selection on Android's Back, which then does not leave the pane", () => {
    const { back, onCancel } = draw({ of: "block" });
    expect(back).toHaveLength(1);
    expect(back[0]!()).toBe(true);
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

describe("the bar's Word on block output", () => {
  const { selectedText, wordRange } = selection;
  it("widens to the whole words the selection touches", () => {
    const t = "git status\nOn branch main";
    expect(selectedText(t, wordRange(t, { start: 5, end: 7 }))).toBe("status");
    expect(selectedText(t, wordRange(t, { start: 2, end: 16 }))).toBe("git status\nOn branch");
    expect(selectedText(t, wordRange(t, { start: 17, end: 12 }))).toBe("On branch");
    // a caret before a word takes that word
    expect(selectedText(t, wordRange(t, { start: 4, end: 4 }))).toBe("status");
  });
  it("stops at spaces, line breaks and a program's box rules; an edge on a space stays", () => {
    expect(selectedText("a│bc│d", wordRange("a│bc│d", { start: 3, end: 3 }))).toBe("bc");
    expect(selectedText("ab cd", wordRange("ab cd", { start: 2, end: 4 }))).toBe(" cd");
    expect(wordRange("", { start: 0, end: 0 })).toEqual({ start: 0, end: 0 });
    expect(wordRange("ab", { start: -3, end: 99 })).toEqual({ start: 0, end: 2 });
  });
});

describe("the screen's handles", () => {
  it("have 18 pt knobs, taken within 44 pt", async () => {
    const { readFileSync } = await import("node:fs");
    const tv = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
    expect(tv).toContain("const KNOB = 18;");
    expect(tv).toContain("const HANDLE_REACH = 44;");
    expect(tv).toContain('widen("all")');
    expect(tv).not.toContain('widen("screen")');
  });
});
