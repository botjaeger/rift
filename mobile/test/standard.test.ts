import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { paneSummary, type RowContext } from "../src/now";
import type { Pane } from "../src/protocol/types";
import { codeSpans } from "../src/ui/format";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, render, textOf, uiDeps, type Node } from "./helpers/render";

// The Standard board's shared parts (Button, FilterChip, Field, PageHeader, SheetHeader, the
// list rows' densities, mono only for code), run as the app's own modules: props in, the
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
    expect(box({}).borderColor).toBe(stubColors.borderStrong);
    expect(label({}).color).toBe(stubColors.text);
    expect(box({ variant: "text" }).borderWidth).toBeUndefined();
    expect(label({ variant: "text" }).color).toBe(stubColors.accent);
    // destructive is the error red, never the needs-you orange
    expect(label({ variant: "destructive" }).color).toBe(stubColors.error);
    expect(box({ variant: "destructive" }).borderColor).toBe(`${stubColors.error}66`);
    for (const v of ["primary", "secondary", "text", "destructive"]) expect(JSON.stringify([box({ variant: v }), label({ variant: v })])).not.toContain(stubColors.warning);
  });
  it("has one radius in both sizes; small is 32 pt with a 44 pt hit area", () => {
    expect(box({}).borderRadius).toBe(10);
    expect(box({ size: "small" }).borderRadius).toBe(10);
    expect(box({}).minHeight).toBe(44);
    expect(box({ size: "small" }).minHeight).toBe(32);
    expect((draw({ size: "small" }) as Node).props.hitSlop).toEqual({ top: 6, bottom: 6 });
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
    expect(off.borderColor).toBe(stubColors.borderStrong);
    expect([on.height, on.borderRadius]).toEqual([32, 16]);
    expect(draw({ count: 2, selected: true }).props.accessibilityState).toEqual({ checked: true, disabled: false });
  });
});

describe("Field: 44 pt, radius 10, an icon, Inter, a clear button while it holds text", () => {
  const { Field } = load("ui/Field.tsx", uiDeps("ios")) as { Field: (p: object) => unknown };
  const draw = (p: object) => render({ type: Field, props: { icon: "search", label: "Search tabs", value: "", onChangeText: () => {}, children: [], ...p } });
  it("names the field and draws words in Inter, placeholder included", () => {
    const tree = draw({ placeholder: "Search by name" });
    const input = findAll(tree, (n) => n.type === "TextInput")[0]!;
    expect(input.props.accessibilityLabel).toBe("Search tabs");
    expect(input.props.placeholder).toBe("Search by name");
    expect(flat(input.props.style).fontFamily).toBe("ui");
    expect(findAll(tree, (n) => n.type === "Icon")[0]!.props.name).toBe("search");
    const box = flat((tree as Node).props.style);
    expect([box.height, box.borderRadius, box.backgroundColor]).toEqual([44, 10, stubColors.surface]);
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
  it("says Close on a camera page's ✕", () => {
    const tree = draw({ back: () => {}, backIcon: "close" });
    expect(byLabel(tree, "Close")).toHaveLength(1);
    expect(byLabel(tree, "Back")).toHaveLength(0);
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
  it("draws its own grabber where the system draws none (iOS draws one)", () => {
    const grabbers = (platform: "ios" | "android" | "web") => findAll(draw(platform, {}), (n) => flat(n.props.style).width === 36 && flat(n.props.style).height === 5);
    expect(grabbers("ios")).toHaveLength(0);
    expect(grabbers("android")).toHaveLength(1);
    expect(grabbers("web")).toHaveLength(1);
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
  it("is regular (56 pt) with a line under the label, compact (44 pt) without", () => {
    expect(rows.ROW_HEIGHT).toEqual({ regular: 56, compact: 44 });
    expect(rows.densityOf("Opens the camera")).toBe("regular");
    expect(rows.densityOf(undefined)).toBe("compact");
    expect(rows.densityOf(undefined, "regular")).toBe("regular");
    const minHeight = (p: object) => flat((render({ type: rows.NavRow, props: { label: "What's New", onPress: () => {}, children: [], ...p } }) as Node).props.style).minHeight;
    expect(minHeight({})).toBe(44);
    expect(minHeight({ detail: "The changes in each version" })).toBe(56);
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

describe("mono only for code: a row's line in runs", () => {
  it("draws only the command and path of a line in mono", () => {
    expect(codeSpans("~/code · idle · echo one ✓", ["~/code", "echo one"])).toEqual([
      { text: "~/code", code: true },
      { text: " · idle · ", code: false },
      { text: "echo one", code: true },
      { text: " ✓", code: false },
    ]);
  });
  it("keeps a line of words whole, and takes the longest code at a place", () => {
    expect(codeSpans("waiting for you · Allow edits?", [])).toEqual([{ text: "waiting for you · Allow edits?", code: false }]);
    expect(codeSpans("npm run dev · running", ["npm", "npm run dev", null, ""])).toEqual([
      { text: "npm run dev", code: true },
      { text: " · running", code: false },
    ]);
    expect(codeSpans("", ["x"])).toEqual([]);
  });
  it("a pane's summary says it in words, its command the only code, never a question", () => {
    const pane = (extra: Partial<Pane>): Pane => ({ id: 1, active: false, cwd: null, job: null, last_command: null, last_exit: null, ...extra });
    const ctx: RowContext = { now: 0, multi: false, waitingSince: () => null, asked: [] };
    const idle = paneSummary("h", pane({ cwd: "/tmp/project1", last_command: "echo one", last_exit: 0 }), ctx, new Map());
    expect(idle.activity).toBe("Idle · last: echo one");
    expect(codeSpans(idle.activity, idle.codes).filter((s) => s.code).map((s) => s.text)).toEqual(["echo one"]);
    // nothing run yet: the folder, in mono
    const fresh = paneSummary("h", pane({ cwd: "/tmp/project1" }), ctx, new Map());
    expect(codeSpans(fresh.activity, fresh.codes)).toEqual([
      { text: "Idle · ", code: false },
      { text: "/tmp/project1", code: true },
    ]);
    const failed = paneSummary("h", pane({ last_command: "cargo test", last_exit: 101 }), ctx, new Map());
    expect(codeSpans(failed.activity, failed.codes)).toEqual([
      { text: "cargo test", code: true },
      { text: " · failed with 101", code: false },
    ]);
    const asking = paneSummary("h", pane({ last_command: "rm -rf build", waiting: { kind: "choice", prompt: "Run rm -rf build?", choices: ["Yes", "No"], context: [], numbered: false } as unknown as Pane["waiting"] }), ctx, new Map());
    expect(asking.codes).toEqual([]);
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
      if (name === "connection") expect(src("ui/ConnectionSheet.tsx")).toContain("<SheetHeader");
      else if (name === "new-tab" || name === "new-pane" || name === "attach") expect(text, r).toContain("<SheetFrame");
      else if (SHEETS.includes(name)) expect(text, r).toContain("<SheetHeader");
      else expect(text, r).toContain(PAGE_VIA[r] ? `<${PAGE_VIA[r]}` : "<PageHeader");
    }
    expect(src("ui/NewSheet.tsx")).toContain("<SheetHeader");
    expect(src("ui/Rows.tsx")).toContain("<PageHeader");
  });
  it("Tabs and Panes both go Back, top left; Diagnostics has the app's header, not the system's", () => {
    expect(src("app/tabs.tsx")).toMatch(/<PageHeader[\s\S]*?back=\{close\}/);
    expect(src("app/panes.tsx")).toMatch(/<PageHeader[\s\S]*?back=\{\(\) => router\.back\(\)\}/);
    expect(src("app/_layout.tsx")).toContain('name="diagnostics" options={{ headerShown: false');
  });
  it("destructive actions are red, never the needs-you orange", () => {
    expect(src("app/host/[key]/pane/[id].tsx")).toContain("menuDestructive: { fontFamily: ui(400), color: colors.error");
    expect(src("app/computer.tsx")).toMatch(/<NavRow[\s\S]*?destructive/);
    expect(src("app/control-keys.tsx")).not.toContain("colors.warning");
    expect(src("ui/BlockCards.tsx")).not.toContain("colors.warning");
    expect(src("app/settings-ntfy.tsx")).toContain('variant="destructive"');
  });
});
