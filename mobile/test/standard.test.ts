import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { paneSummary, type RowContext } from "../src/now";
import type { Pane } from "../src/protocol/types";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, render, textOf, uiDeps, type Node } from "./helpers/render";

// The Standard board's shared parts (Button, FilterChip, Field, PageHeader, SheetHeader, the
// list rows' densities, a row's line in words), run as the app's own modules: props in, the
// rendered labels, roles, variants and handlers out. Pixels stay device checks.

const router = () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true });
const byLabel = (tree: unknown, label: string) => findAll(tree, (n) => n.props.accessibilityLabel === label);
const one = (tree: unknown, label: string): Node => {
  const found = byLabel(tree, label);
  expect(found, label).toHaveLength(1);
  return found[0]!;
};

describe("Button: Primary, Secondary, Text, Destructive; regular and small; one radius", () => {
  const { Button } = load("ui/Button.tsx", uiDeps("ios")) as { Button: (p: object) => unknown };
  const draw = (p: object) => render({ type: Button, props: { label: "Run again", onPress: () => {}, children: [], ...p } });
  const box = (p: object) => flat((draw(p) as Node).props.style);
  const label = (p: object) => flat(findAll(draw(p), (n) => n.type === "Text")[0]!.props.style);

  it("is one button named by its label, or by what it says more", () => {
    const b = draw({}) as Node;
    expect(b.type).toBe("Pressable");
    expect(b.props.accessibilityRole).toBe("button");
    expect(b.props.accessibilityLabel).toBe("Run again");
    expect(textOf(b)).toBe("Run again");
    expect((draw({ accessibilityLabel: "Stop, sends control C" }) as Node).props.accessibilityLabel).toBe("Stop, sends control C");
  });
  it("draws each variant in its own colours", () => {
    expect(box({ variant: "primary" }).backgroundColor).toBe(stubColors.accent);
    expect(label({ variant: "primary" }).color).toBe(stubColors.onAccent);
    // the boards' outline: #ffffff3d, not the field's #ffffff24
    expect(box({}).borderColor).toBe(stubColors.rim);
    expect(stubColors.rim).toBe("#ffffff3d");
    expect(label({}).color).toBe(stubColors.text);
    expect(box({ variant: "text" }).borderWidth).toBeUndefined();
    expect(label({ variant: "text" }).color).toBe(stubColors.accent);
    // destructive is the error red, never the needs-you orange
    expect(label({ variant: "destructive" }).color).toBe(stubColors.error);
    expect(box({ variant: "destructive" }).borderColor).toBe(`${stubColors.error}66`);
    for (const v of ["primary", "secondary", "text", "destructive"]) expect(JSON.stringify([box({ variant: v }), label({ variant: v })])).not.toContain(stubColors.warning);
  });
  it("has one radius in both sizes: 48 pt in Inter 600 16, small 34 pt in 600 13 with a 44 pt hit area", () => {
    expect(box({}).borderRadius).toBe(10);
    expect(box({ size: "small" }).borderRadius).toBe(10);
    expect([box({}).minHeight, box({}).paddingHorizontal]).toEqual([48, 18]);
    expect([box({ size: "small" }).minHeight, box({ size: "small" }).paddingHorizontal]).toEqual([34, 12]);
    expect([label({}).fontFamily, label({}).fontSize]).toEqual(["ui", 16]);
    expect([label({ size: "small" }).fontFamily, label({ size: "small" }).fontSize]).toEqual(["ui", 13]);
    expect((draw({ size: "small" }) as Node).props.hitSlop).toEqual({ top: 5, bottom: 5 });
    expect((draw({}) as Node).props.hitSlop).toBeUndefined();
  });
  it("reports disabled, and a toggle's state", () => {
    expect((draw({ disabled: true }) as Node).props.accessibilityState).toEqual({ disabled: true });
    const on = draw({ pressed: true }) as Node;
    expect(on.props.accessibilityRole).toBe("togglebutton");
    expect(on.props.accessibilityState).toEqual({ checked: true, disabled: false });
    expect(flat(on.props.style).backgroundColor).toBe(stubColors.tints.accentBadge);
  });
});

describe("FilterChip: one filter, the chosen one filled", () => {
  const { FilterChip } = load("ui/FilterChip.tsx", uiDeps("ios")) as { FilterChip: (p: object) => unknown };
  const draw = (p: object) => render({ type: FilterChip, props: { label: "Working", onPress: () => {}, children: [], ...p } }) as Node;
  it("says its word and count, as a toggle", () => {
    const chip = draw({ count: 2, selected: false });
    expect(chip.props.accessibilityLabel).toBe("Working 2");
    expect(chip.props.accessibilityRole).toBe("togglebutton");
    expect(chip.props.accessibilityState).toEqual({ checked: false, disabled: false });
    expect(textOf(chip)).toBe("Working 2");
    expect(draw({ selected: false }).props.accessibilityLabel).toBe("Working");
  });
  it("fills the chosen one and outlines the rest, 32 pt round", () => {
    const on = flat(draw({ count: 2, selected: true }).props.style);
    const off = flat(draw({ count: 2, selected: false }).props.style);
    expect(on.backgroundColor).toBe(stubColors.textBright);
    expect(off.backgroundColor).toBeUndefined();
    expect(off.borderColor).toBe(stubColors.rim);
    expect([on.height, on.borderRadius, on.paddingHorizontal]).toEqual([32, 16, 11]);
    expect(draw({ count: 2, selected: true }).props.accessibilityState).toEqual({ checked: true, disabled: false });
  });
});

describe("Field: 46 pt, radius 10, an icon, Inter, the accent rim while focused, a clear button while it holds text", () => {
  const { Field } = load("ui/Field.tsx", uiDeps("ios")) as { Field: (p: object) => unknown };
  const draw = (p: object) => render({ type: Field, props: { icon: "search", label: "Search tabs", value: "", onChangeText: () => {}, children: [], ...p } });
  it("names the field and draws words in Inter, placeholder included", () => {
    const tree = draw({ placeholder: "Search by name" });
    const input = findAll(tree, (n) => n.type === "TextInput")[0]!;
    expect(input.props.accessibilityLabel).toBe("Search tabs");
    expect(input.props.placeholder).toBe("Search by name");
    expect(flat(input.props.style).fontFamily).toBe("ui");
    expect(input.props.placeholderTextColor).toBe(stubColors.placeholder);
    expect(findAll(tree, (n) => n.type === "Icon")[0]!.props.name).toBe("search");
    const box = flat((tree as Node).props.style);
    expect([box.height, box.borderRadius, box.backgroundColor, box.borderColor]).toEqual([46, 10, stubColors.surface, stubColors.borderStrong]);
    expect([box.paddingLeft, box.paddingRight]).toEqual([12, 8]);
  });
  it("draws its rim in the accent while it has focus, and still calls the caller's focus handlers", () => {
    // one render: the focus flag comes from useState, so a field drawn focused shows the accent rim
    const { Field: Focused } = load("ui/Field.tsx", uiDeps("ios"), (s) => s.replace("useState(false)", "useState(true)")) as { Field: (p: object) => unknown };
    const focused = render({ type: Focused, props: { icon: "search", label: "Search tabs", value: "", onChangeText: () => {}, children: [] } }) as Node;
    expect(flat(focused.props.style).borderColor).toBe(stubColors.accent);
    const onFocus = vi.fn();
    const onBlur = vi.fn();
    const input = findAll(draw({ onFocus, onBlur }), (n) => n.type === "TextInput")[0]!;
    (input.props.onFocus as (e: unknown) => void)("f");
    (input.props.onBlur as (e: unknown) => void)("b");
    expect([onFocus.mock.calls, onBlur.mock.calls]).toEqual([[["f"]], [["b"]]]);
  });
  it("draws Clear as a 16 pt round mark in a 28 pt button, 44 to the touch", () => {
    const clear = one(draw({ value: "proj" }), "Clear");
    expect([flat(clear.props.style).width, flat(clear.props.style).height, clear.props.hitSlop]).toEqual([28, 28, 8]);
  });
  it("offers Clear only with text, and Clear empties it", () => {
    expect(byLabel(draw({}), "Clear")).toHaveLength(0);
    const change = vi.fn();
    (one(draw({ value: "proj", onChangeText: change }), "Clear").props.onPress as () => void)();
    expect(change).toHaveBeenCalledExactlyOnceWith("");
  });
  it("passes other TextInput props through", () => {
    const input = findAll(draw({ returnKeyType: "search", autoCapitalize: "none" }), (n) => n.type === "TextInput")[0]!;
    expect([input.props.returnKeyType, input.props.autoCapitalize]).toEqual(["search", "none"]);
  });
});

describe("PageHeader: ‹ Back (✕ only for a camera or a lock), a title, a subtitle, actions", () => {
  const { PageHeader } = load("ui/Screen.tsx", uiDeps("ios")) as { PageHeader: (p: object) => unknown };
  const draw = (p: object) => render({ type: PageHeader, props: { title: "Tabs", children: [], ...p } });
  it("has a Back that goes back, and the title as a header", () => {
    const back = vi.fn();
    const tree = draw({ subtitle: "studio-mac · 30 tabs", back });
    (one(tree, "Back").props.onPress as () => void)();
    expect(back).toHaveBeenCalledOnce();
    expect(findAll(tree, (n) => n.type === "Icon").map((n) => n.props.name)).toContain("back");
    const header = findAll(tree, (n) => n.props.accessibilityRole === "header");
    expect(header.map(textOf)).toEqual(["Tabs"]);
    expect(textOf(tree)).toContain("studio-mac · 30 tabs");
  });
  it("goes Back with ‹ on every page, the camera page too (the New Pairing board)", () => {
    const tree = draw({ back: () => {} });
    expect(byLabel(tree, "Back")).toHaveLength(1);
    expect(byLabel(tree, "Close")).toHaveLength(0);
    expect(readFileSync(new URL("../src/ui/Screen.tsx", import.meta.url), "utf8")).not.toContain("backIcon");
  });
  it("cuts a long title once, and draws its actions", () => {
    const tree = draw({ title: "customer-portal-migration-2026-q4", back: () => {}, right: { type: "Action", props: { children: [] } } });
    expect(findAll(tree, (n) => n.props.accessibilityRole === "header")[0]!.props.numberOfLines).toBe(1);
    expect(findAll(tree, (n) => n.type === "Action")).toHaveLength(1);
  });
});

describe("SheetHeader: grabber, title, subtitle, round ✕, always", () => {
  const draw = (platform: "ios" | "android" | "web", p: object, r = router()) => {
    const { SheetHeader } = load("ui/SheetHeader.tsx", uiDeps(platform, { "expo-router": { useRouter: () => r } })) as { SheetHeader: (p: object) => unknown };
    return render({ type: SheetHeader, props: { title: "New tab", children: [], ...p } });
  };
  it("has the title as a header, the subtitle, and a ✕ named Close on every platform", () => {
    for (const platform of ["ios", "android", "web"] as const) {
      const tree = draw(platform, { subtitle: "On studio-mac" });
      expect(findAll(tree, (n) => n.props.accessibilityRole === "header").map(textOf)).toEqual(["New tab"]);
      expect(textOf(tree)).toContain("On studio-mac");
      expect(one(tree, "Close").props.accessibilityRole).toBe("button");
    }
  });
  it("draws the boards' 36 × 5 grabber on every platform (the system's is off: iOS 27 draws a wider one)", () => {
    const grabbers = (platform: "ios" | "android" | "web") => findAll(draw(platform, {}), (n) => flat(n.props.style).width === 36 && flat(n.props.style).height === 5);
    for (const platform of ["ios", "android", "web"] as const) expect(grabbers(platform), platform).toHaveLength(1);
    const layout = readFileSync(new URL("../src/app/_layout.tsx", import.meta.url), "utf8");
    expect(layout).not.toContain("sheetGrabberVisible: true");
    expect(layout.match(/sheetGrabberVisible: false/g)?.length).toBe(layout.match(/presentation: "formSheet"/g)?.length);
  });
  it("closes through onClose, else goes back", () => {
    const onClose = vi.fn();
    (one(draw("ios", { onClose }), "Close").props.onPress as () => void)();
    expect(onClose).toHaveBeenCalledOnce();
    const r = router();
    (one(draw("ios", {}, r), "Close").props.onPress as () => void)();
    expect(r.back).toHaveBeenCalledOnce();
  });
});

describe("list rows: one row in two densities", () => {
  const rows = load("ui/Rows.tsx", uiDeps("ios", { "expo-router": { useRouter: router } })) as {
    densityOf: (detail: unknown, d?: string) => string;
    ROW_HEIGHT: Record<string, number>;
    NavRow: (p: object) => unknown;
  };
  it("is 68 pt with or without a line under the label (the boards' 52 and 8 over and under); compact (44 pt) only when asked", () => {
    expect(rows.ROW_HEIGHT).toEqual({ regular: 68, compact: 44 });
    expect(rows.densityOf("Opens the camera")).toBe("regular");
    expect(rows.densityOf(undefined)).toBe("regular");
    expect(rows.densityOf(undefined, "compact")).toBe("compact");
    const style = (p: object) => flat((render({ type: rows.NavRow, props: { label: "What's New", onPress: () => {}, children: [], ...p } }) as Node).props.style);
    expect(style({ first: true }).minHeight).toBe(68);
    expect(style({ first: true, detail: "The changes in each version" }).minHeight).toBe(68);
    // a row under a hairline keeps its 68 pt under it, as the boards draw the rule outside the row
    expect([style({}).minHeight, style({}).borderTopWidth]).toEqual([69, 1]);
    expect([style({}).paddingVertical, style({}).paddingHorizontal]).toEqual([8, 16]);
  });
  it("draws the label in the text colour, a value in 15/20 and the chevron in the muted grey", () => {
    const tree = render({ type: rows.NavRow, props: { label: "Appearance", value: "Same as the computer", onPress: () => {}, children: [] } });
    const text = (s: string) => flat(findAll(tree, (n) => n.type === "Text" && textOf(n) === s)[0]!.props.style);
    expect(text("Appearance")).toMatchObject({ color: stubColors.text, fontSize: 16, lineHeight: 21 });
    expect(stubColors.text).toBe("#e6e6e6");
    expect(text("Same as the computer")).toMatchObject({ color: stubColors.textMuted, fontSize: 15, lineHeight: 20 });
    expect(findAll(tree, (n) => n.type === "Icon")[0]!.props).toMatchObject({ name: "forward", size: 8, color: stubColors.textMuted });
  });
  it("draws a destructive row in the error red, without a chevron", () => {
    const tree = render({ type: rows.NavRow, props: { label: "Unpair this computer", destructive: true, onPress: () => {}, children: [] } });
    const label = findAll(tree, (n) => n.type === "Text" && textOf(n) === "Unpair this computer")[0]!;
    expect(flat(label.props.style).color).toBe(stubColors.error);
    expect(findAll(tree, (n) => n.type === "Icon")).toHaveLength(0);
  });
  const long = "Aurora Borealis Midnight Variant with Extra Contrast for Late Ni…";
  const valueOf = (source: typeof rows) => {
    const tree = render({ type: source.NavRow, props: { label: "Theme", value: long, onPress: () => {}, children: [] } });
    return findAll(tree, (n) => n.type === "Text" && textOf(n) === long)[0]!;
  };
  it("keeps the label and chevron beside a long value: one line that shrinks, at most 60 %", () => {
    const value = valueOf(rows);
    expect(value.props.numberOfLines).toBe(1);
    expect(flat(value.props.style)).toMatchObject({ flexShrink: 1, maxWidth: "60%" });
  });
  it("a path value still takes the room its label leaves, cut at its head", () => {
    const { ValueRow } = load("ui/Rows.tsx", uiDeps("ios", { "expo-router": { useRouter: router } })) as { ValueRow: (p: object) => unknown };
    const tree = render({ type: ValueRow, props: { path: true, label: "Folder", value: "~", children: [] } });
    const value = findAll(tree, (n) => n.type === "Text" && textOf(n) === "~")[0]!;
    expect(flat(value.props.style)).toMatchObject({ flex: 1, maxWidth: "100%", textAlign: "right" });
    expect(value.props.ellipsizeMode).toBe("head");
  });
  it("negative control: a value that cannot shrink is caught", () => {
    const loose = load("ui/Rows.tsx", uiDeps("ios", { "expo-router": { useRouter: router } }), (s) =>
      s.replace('value: { flexShrink: 1, maxWidth: "60%", ', "value: { "),
    ) as typeof rows;
    expect(flat(valueOf(loose).props.style).flexShrink).toBeUndefined();
  });
});

describe("the boards' switch, glass buttons, sheet header and page ground", () => {
  const deps = () => uiDeps("ios", { "expo-router": { useRouter: router } });
  const rows = load("ui/Rows.tsx", deps()) as { Toggle: (p: object) => unknown; SwitchRow: (p: object) => unknown; SettingsPage: (p: object) => unknown };
  it("draws a 51 × 31 track with a round 27 pt knob 2 pt in: accent when on, the off veil when off", () => {
    const track = (on: boolean) => render({ type: rows.Toggle, props: { on, children: [] } }) as Node;
    for (const on of [true, false]) {
      const t = track(on);
      expect(flat(t.props.style)).toMatchObject({ width: 51, height: 31, borderRadius: 16, backgroundColor: on ? stubColors.accent : stubColors.switchOff });
      const knob = flat((t.props.children[0] as Node).props.style);
      expect(knob).toMatchObject({ width: 27, height: 27, borderRadius: 14, top: 2, ...(on ? { right: 2 } : { left: 2 }) });
    }
  });
  it("a switch row is one switch to a screen reader and draws the drawn switch, not the system's", () => {
    const row = render({ type: rows.SwitchRow, props: { label: "App lock", detail: "Face ID", value: true, onChange: () => {}, children: [] } }) as Node;
    expect([row.props.accessibilityRole, row.props.accessibilityState]).toEqual(["switch", { checked: true, disabled: false }]);
    expect(findAll(row, (n) => n.type === "Switch")).toHaveLength(0);
    expect(findAll(row, (n) => flat(n.props.style).width === 51 && flat(n.props.style).height === 31)).toHaveLength(1);
    // the pane menu's toggle is the same switch
    const menu = readFileSync(new URL("../src/app/host/[key]/pane/[id].tsx", import.meta.url), "utf8");
    expect(menu).toContain("<Toggle on={on} />");
    expect(menu).not.toMatch(/track: \{ width: 40/);
  });
  it("puts a settings page on the bg under the header's chrome, groups 18 pt apart, captions 4 pt in", () => {
    const tree = render({ type: rows.SettingsPage, props: { title: "Settings", children: [] } }) as Node;
    expect(flat(tree.props.style).backgroundColor).toBe(stubColors.bg);
    const scroll = findAll(tree, (n) => n.type === "ScrollView")[0]!;
    expect(flat(scroll.props.style).backgroundColor).toBe(stubColors.bg);
    const body = flat((scroll.props.children[0] as Node).props.style);
    expect([body.paddingTop, body.paddingHorizontal, body.gap]).toEqual([18, 16, 18]);
    const header = findAll(tree, (n) => flat(n.props.style).borderBottomWidth === 1)[0]!;
    expect(flat(header.props.style).backgroundColor).toBe(stubColors.chrome);
    const { Group } = load("ui/Rows.tsx", deps()) as { Group: (p: object) => unknown };
    const group = render({ type: Group, props: { title: "Computers", children: [] } }) as Node;
    const caption = findAll(group, (n) => n.props.accessibilityRole === "header")[0]!;
    expect(flat(caption.props.style).paddingHorizontal).toBe(4);
    // the caption's own 4 pt only: no second indent from the group
    expect(flat((group.props.children[0] as Node).props.style).paddingHorizontal).toBeUndefined();
  });
  it("draws the header's glass buttons 38 pt round, 44 to the touch", () => {
    const { IconButton } = load("ui/Screen.tsx", uiDeps("ios")) as { IconButton: (p: object) => unknown };
    const b = render({ type: IconButton, props: { glass: true, icon: "back", label: "Back", onPress: () => {}, children: [] } }) as Node;
    expect(flat(b.props.style)).toMatchObject({ width: 38, height: 38, borderRadius: 19, backgroundColor: stubColors.glass, borderColor: stubColors.glassRim });
    expect(b.props.hitSlop).toBe(3);
    // the boards' glyphs: ‹ at the default 18, + and the gear drawn smaller
    const size = (icon: string) => findAll(render({ type: IconButton, props: { glass: true, icon, label: icon, onPress: () => {}, children: [] } }), (n) => n.type === "Icon")[0]!.props.size;
    expect([size("back"), size("add"), size("settings")]).toEqual([undefined, 12, 16]);
  });
  it("lays the sheet header out as the boards: 10 over, 14 under, 18 / 14 in, a 36 pt ✕ 44 to the touch, a mono subtitle 13/17", () => {
    const { SheetHeader } = load("ui/SheetHeader.tsx", uiDeps("android", { "expo-router": { useRouter: router } })) as { SheetHeader: (p: object) => unknown };
    const tree = render({ type: SheetHeader, props: { title: "Change folder", subtitle: "~/code/project1", subtitleMono: true, children: [] } }) as Node;
    const row = flat((tree.props.children[1] as Node).props.style);
    expect([row.paddingTop, row.paddingBottom, row.paddingLeft, row.paddingRight, row.gap]).toEqual([10, 14, 18, 14, 12]);
    const close = one(tree, "Close");
    expect([flat(close.props.style).width, flat(close.props.style).height, close.props.hitSlop]).toEqual([36, 36, 4]);
    expect(findAll(close, (n) => n.type === "Icon")[0]!.props).toMatchObject({ name: "close", size: 10 });
    const sub = findAll(tree, (n) => n.type === "Text" && textOf(n) === "~/code/project1")[0]!;
    expect(flat(sub.props.style)).toMatchObject({ fontFamily: "mono", fontSize: 13, lineHeight: 17 });
    const grabber = findAll(tree, (n) => flat(n.props.style).width === 36 && flat(n.props.style).height === 5)[0]!;
    expect(flat(grabber.props.style).backgroundColor).toBe("#ffffff33");
  });
});

describe("a row's line says what a pane does, in words (the New Lists board: all Inter)", () => {
  it("names the command it ran, failed at or runs, and the question it asks", () => {
    const pane = (extra: Partial<Pane>): Pane => ({ id: 1, active: false, cwd: null, job: null, last_command: null, last_exit: null, ...extra });
    const ctx: RowContext = { now: 0, multi: false, waitingSince: () => null, asked: [] };
    expect(paneSummary("h", pane({ cwd: "/tmp/project1", last_command: "echo one", last_exit: 0 }), ctx, new Map()).activity).toBe("Idle · last: echo one");
    expect(paneSummary("h", pane({ cwd: "/tmp/project1" }), ctx, new Map()).activity).toBe("Idle · /tmp/project1");
    expect(paneSummary("h", pane({ last_command: "cargo test", last_exit: 101 }), ctx, new Map()).activity).toBe("cargo test · failed with 101");
    const asking = paneSummary("h", pane({ last_command: "rm -rf build", waiting: { kind: "choice", prompt: "Run rm -rf build?", choices: ["Yes", "No"], context: [], numbered: false } as unknown as Pane["waiting"] }), ctx, new Map());
    expect(asking.activity).toBe("Waiting for you · Run rm -rf build?");
  });
});

// Every route and sheet uses the one page header or the one sheet header (rendering is a device check).
describe("the standard is wired into every page and sheet", () => {
  const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");
  const routes = [...readdirSync(new URL("../src/app", import.meta.url)).filter((f) => f.endsWith(".tsx") && !f.startsWith("_") && !f.startsWith("+")).map((f) => `app/${f}`), "app/host/[key]/index.tsx"];
  const SHEETS = ["attach", "block", "chips", "confirm", "connection", "diff", "new-pane", "new-tab", "pane-chips", "whats-new"];
  // the root page keeps its large title; Settings' pages draw PageHeader through SettingsPage
  const PAGE_VIA = { "app/index.tsx": "FirstRun", "app/settings.tsx": "SettingsPage", "app/settings-ntfy.tsx": "SettingsPage", "app/computer.tsx": "SettingsPage", "app/appearance.tsx": "SettingsPage" } as Record<string, string>;
  it("pages draw PageHeader, sheets SheetHeader", () => {
    for (const r of routes) {
      const name = r.slice(4, -4);
      const text = src(r);
      if (name === "connection") expect(src("ui/ConnectionSheet.tsx")).toContain("<SheetFrame");
      else if (["new-tab", "new-pane", "attach", "block", "confirm", "whats-new"].includes(name)) expect(text, r).toContain("<SheetFrame");
      // Change folder is a SheetFrame, the chips' other menus the header over one scroll view
      else if (name === "chips") expect(text, r).toMatch(/<SheetFrame\s+board="folder"[\s\S]*<SheetHeader/);
      else if (SHEETS.includes(name)) expect(text, r).toContain("<SheetHeader");
      else expect(text, r).toContain(PAGE_VIA[r] ? `<${PAGE_VIA[r]}` : "<PageHeader");
    }
    expect(src("ui/SheetFrame.tsx")).toContain("<SheetHeader");
    expect(src("ui/Rows.tsx")).toContain("<PageHeader");
  });
  it("Tabs and Panes both go Back, top left; Diagnostics has the app's header, not the system's", () => {
    expect(src("app/tabs.tsx")).toMatch(/<PageHeader[\s\S]*?back=\{close\}/);
    expect(src("app/panes.tsx")).toMatch(/<PageHeader[\s\S]*?back=\{\(\) => router\.back\(\)\}/);
    expect(src("app/_layout.tsx")).toContain('name="diagnostics" options={{ headerShown: false');
  });
  it("destructive actions are red, never the needs-you orange", () => {
    expect(src("app/host/[key]/pane/[id].tsx")).toContain("menuDestructive: { fontFamily: ui(400), color: colors.error");
    expect(src("app/computer.tsx")).toMatch(/<Button\s+variant="destructive"/);
    expect(src("app/control-keys.tsx")).not.toContain("colors.warning");
    expect(src("ui/BlockCards.tsx")).not.toContain("colors.warning");
    expect(src("app/settings-ntfy.tsx")).toContain('variant="destructive"');
  });
  it("Paste link, Pair, Diagnostics and Keys sit on the bg under the header's chrome, as Settings does", () => {
    for (const page of ["app/add.tsx", "app/pair.tsx", "app/diagnostics.tsx", "app/control-keys.tsx"]) {
      expect(src(page), page).toContain("fill: { flex: 1, backgroundColor: colors.bg },");
      expect(src(page), page).toContain("<PageHeader");
    }
  });
  it("a selection draws in the theme's selection colour, in a block and on a program's screen", () => {
    // the selection colour, except on iOS in a field holding a link: iOS draws detected links in the
    // field's tint, which the translucent selection colour left unreadable (about 2:1)
    expect(src("ui/SelectableText.tsx")).toContain('selectionColor={Platform.OS === "ios" && links.length > 0 ? colors.accent : colors.selection}');
    expect(src("ui/TerminalView.tsx")).toContain('band: { position: "absolute", backgroundColor: colors.selection },');
  });
});
