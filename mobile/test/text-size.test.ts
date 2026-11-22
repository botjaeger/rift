import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import * as paneView from "../src/paneView";
import { keptZoom, onKeptZoom, setKeptZoom, ZOOM_MAX, ZOOM_MIN, zoomStep } from "../src/paneView";
import { findAll, load, react, render, textOf, uiDeps, type Node } from "./helpers/render";

// Text size in the pane's ⋯ menu over a full-screen program (the New Screen board: "Text size ·
// Pinch the screen, or here"): the same kept zoom the pinch changes, within its bounds.

afterEach(() => {
  setKeptZoom(1);
});

describe("the kept zoom", () => {
  it("is set clamped, as a pinch keeps it, and every follower hears the change", () => {
    const heard = vi.fn();
    const off = onKeptZoom(heard);
    expect(setKeptZoom(1.3)).toBe(1.3);
    expect(keptZoom.value).toBe(1.3);
    expect(setKeptZoom(9)).toBe(ZOOM_MAX);
    expect(setKeptZoom(0.1)).toBe(ZOOM_MIN);
    expect(setKeptZoom(NaN)).toBe(1);
    expect(heard.mock.calls.map((c) => c[0])).toEqual([1.3, ZOOM_MAX, ZOOM_MIN, 1]);
    off();
    setKeptZoom(1.5);
    expect(heard).toHaveBeenCalledTimes(4);
  });

  it("the terminal view keeps its zoom through it and follows it (the menu's change shows at once)", () => {
    const view = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
    expect(view).toContain("useEffect(() => onKeptZoom(setZoomState), [])");
    expect(view).toMatch(/const c = setKeptZoom\(z\);/);
    expect(view).not.toMatch(/keptZoom\.value = /);
  });

  it("the terminal view's − and + stop at the kept zoom's bounds, as the menu's do", () => {
    const view = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
    expect(view).toContain("disabled={zoom <= ZOOM_MIN}");
    expect(view).toContain("disabled={zoom >= ZOOM_MAX}");
    expect(view).toContain("zoom <= ZOOM_MIN && styles.disabled");
    expect(view).toContain("zoom >= ZOOM_MAX && styles.disabled");
    // no copy of the bounds
    expect(view).not.toMatch(/zoom [<>]= (0\.6|2)\b/);
  });
});

/** The menu row, opened (`open` is the component's one boolean state) or not. */
function draw(open: boolean) {
  const deps = uiDeps("ios", { "@/paneView": paneView });
  deps.react = { ...react, useState: (initial: unknown) => [initial === false ? open : typeof initial === "function" ? (initial as () => unknown)() : initial, () => {}] };
  const { TextSizeMenu } = load("ui/TextSizeMenu.tsx", deps) as { TextSizeMenu: (p: object) => unknown };
  return render({ type: TextSizeMenu, props: { children: [] } });
}
const byLabel = (tree: unknown, label: string): Node => {
  // the first: "Text size" is the row, then (opened) the adjustable control
  const found = findAll(tree, (n) => n.props.accessibilityLabel === label);
  expect(found.length, label).toBeGreaterThan(0);
  return found[0]!;
};

describe("Text size in the menu", () => {
  it("is a menu row with the board's words, no size on it, closed until tapped", () => {
    const tree = draw(false);
    const row = byLabel(tree, "Text size");
    expect(row.props.accessibilityRole).toBe("menuitem");
    expect(row.props.accessibilityHint).toBe("Pinch the screen, or here");
    expect(row.props.accessibilityState).toEqual({ expanded: false });
    expect(textOf(row)).toBe("Text sizePinch the screen, or here");
    // the size is still spoken (and shown once the row opens)
    expect(row.props.accessibilityValue).toEqual({ text: "100%" });
    expect(findAll(tree, (n) => n.props.accessibilityRole === "adjustable")).toHaveLength(0);
  });

  it("opened: − size + as one adjustable control, and Reset", () => {
    setKeptZoom(1.3);
    const tree = draw(true);
    const adj = findAll(tree, (n) => n.props.accessibilityRole === "adjustable");
    expect(adj).toHaveLength(1);
    const a = adj[0]!;
    expect(a.props.accessible).toBe(true);
    expect(a.props.accessibilityLabel).toBe("Text size");
    expect(a.props.accessibilityValue).toEqual({ min: 60, max: 200, now: 130, text: "130%" });
    expect((a.props.accessibilityActions as { name: string }[]).map((x) => x.name)).toEqual(["increment", "decrement"]);
    expect(textOf(a)).toBe("−130%+");
    expect(byLabel(tree, "Reset text size").props.accessibilityRole).toBe("button");
  });

  it("− and + step the kept zoom as the terminal view's buttons, within the bounds", () => {
    const tree = draw(true);
    (byLabel(tree, "Larger text").props.onPress as () => void)();
    expect(keptZoom.value).toBeCloseTo(zoomStep(1, 1));
    (byLabel(tree, "Smaller text").props.onPress as () => void)();
    expect(keptZoom.value).toBeCloseTo(zoomStep(1, -1));
    setKeptZoom(ZOOM_MAX);
    const top = draw(true);
    expect(byLabel(top, "Larger text").props.disabled).toBe(true);
    expect(byLabel(top, "Smaller text").props.disabled).toBe(false);
    (byLabel(top, "Reset text size").props.onPress as () => void)();
    expect(keptZoom.value).toBe(1);
    expect(byLabel(draw(true), "Reset text size").props.disabled).toBe(true);
  });

  it("a screen reader's increment and decrement change it too", () => {
    const heard = vi.fn();
    const off = onKeptZoom(heard);
    const act = (name: string) =>
      (findAll(draw(true), (n) => n.props.accessibilityRole === "adjustable")[0]!.props.onAccessibilityAction as (e: object) => void)({ nativeEvent: { actionName: name } });
    act("increment");
    expect(keptZoom.value).toBeCloseTo(1.15);
    act("decrement");
    expect(keptZoom.value).toBeCloseTo(1);
    act("activate");
    expect(heard).toHaveBeenCalledTimes(2);
    off();
  });
});

describe("the pane's ⋯ menu", () => {
  const route = readFileSync(new URL("../src/app/host/[key]/pane/[id].tsx", import.meta.url), "utf8");
  it("shows Text size only over a full-screen program, where it has no Wrap long lines", () => {
    expect(route).toContain("{fullScreen ? <TextSizeMenu last={!elapsedChip?.opens && !canSplit && !canOpenTab && !closeable} /> : null}");
    expect(route).toContain("last={!fullScreen && !elapsedChip?.opens && !canSplit && !canOpenTab && !closeable}");
  });
  it("draws one hairline between every row: no dark band between its groups", () => {
    expect(route).not.toContain("menuGap");
    expect(route).toContain('menuRule: { minHeight: 61, borderBottomWidth: 1, borderBottomColor: colors.hairline },');
    // Wrap long lines is the blocks' (not over a program, not on a closed pane), and no note row over a program
    expect(route).toMatch(/\{fullScreen \|\| closed \? null : \(\s*<MenuSwitch\s+icon="wrapOn"/);
    expect(route).not.toContain("programs wrap their own lines.</Text>");
  });
  it("is the board's menu: scrim, 270 wide 16 from the right on #28282a, rows of 48 + 6 / 6", () => {
    expect(route).toContain('scrim: { backgroundColor: "#00000066" },');
    expect(route).toContain("style={[StyleSheet.absoluteFill, styles.scrim]}");
    expect(route).toMatch(/menu: \{\s*position: "absolute",\s*right: 16,\s*width: 270,\s*borderRadius: 14,\s*backgroundColor: colors\.popover,\s*borderWidth: 1,\s*borderColor: colors\.borderStrong,/);
    expect(route).toContain('menuRow: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 14, paddingVertical: 6 },');
    // Close pane / Close tab line up with the titles; a closed pane has nothing to close
    expect(route).toContain("menuAction: { paddingLeft: 44 },");
    expect(route).toContain("const closeable = canClose(host) && !closed;");
  });
});
