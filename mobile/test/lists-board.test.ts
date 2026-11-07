import { describe, expect, it, vi } from "vitest";
import * as connStatus from "../src/ui/connStatus";
import * as hostDot from "../src/ui/hostDot";
import * as now from "../src/now";
import * as paneView from "../src/paneView";
import * as answer from "../src/answer";
import * as settingsText from "../src/ui/settingsText";
import type { ConnState, PathStatus } from "../src/protocol/client";
import { DARK_DEFAULT, paletteFrom } from "../src/ui/palette";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, render, textOf, uiDeps, type Node } from "./helpers/render";

// The New Lists and New Portals boards (Tabs, Panes, Portals, First run, the computer menu,
// Connection, a computer's settings), run as the app's own modules: the measures and words the
// planned frames draw. Pixels stay device checks.

const NOW = Date.UTC(2026, 10, 7, 12);

const conn = (over: Partial<ConnState>): ConnState =>
  ({
    status: "open",
    path: { kind: "lan", address: "192.168.1.15:7878" },
    paths: [],
    attempt: 0,
    error: null,
    failure: null,
    retryAt: null,
    retryInMs: null,
    since: null,
    closing: null,
    ...over,
  }) as ConnState;

const pane = (id: number, extra: object = {}) => ({ id, active: true, cwd: null, job: null, last_command: null, last_exit: null, ...extra });
const tab = (index: number, title: string, extra: object = {}) => ({ index, title, active: index === 0, panes: [pane(index + 10)], ...extra });

const host = (key: string, name: string, c: ConnState, extra: object = {}) => ({
  record: { key, name, addresses: [], permission: "operate", pairedAt: "2026-11-03T09:00:00Z" },
  conn: c,
  gateway: null,
  welcome: null,
  windows: c.status === "open" ? [{ pid: 1, background: false, tabs: [tab(0, "project1"), tab(1, "project2")] }] : [],
  topology: c.status === "open",
  notice: null,
  lastSeen: null,
  ...extra,
});

const UNREACHABLE = conn({ status: "failed", failure: "gave_up", attempt: 5, path: null });
const style = (n: Node | undefined) => flat(n?.props.style);
const byText = (tree: unknown, text: string) => findAll(tree, (n) => n.type === "Text" && textOf(n) === text)[0];

describe("ListRow: the New Lists board's row", () => {
  const { ListRow, LIST_ROW_H } = load("ui/ListRow.tsx", uiDeps("ios")) as { ListRow: (p: object) => unknown; LIST_ROW_H: number };
  const handle = { grip: (n: unknown) => ({ type: "Grip", props: { children: [n] } }), accessibilityActions: [], onAccessibilityAction: vi.fn() };
  const row = (over: object = {}) =>
    render({
      type: ListRow,
      props: { handle, movable: true, title: "project2", line: "npm run dev · running", kind: "working", when: "4:32", label: "project2", selected: false, onPress: vi.fn(), onClose: vi.fn(), closeLabel: "Close tab project2", children: [], ...over },
    }) as Node;

  it("is 62 pt and its hairline, 16 pt in, 12 at the end and between its parts", () => {
    expect(LIST_ROW_H).toBe(63);
    expect(style(row())).toMatchObject({ height: 63, paddingLeft: 16, paddingRight: 12, gap: 12, borderBottomWidth: 1, borderBottomColor: stubColors.hairline });
  });
  it("draws the title 600 15/20 and the line in Inter 13/17, never mono", () => {
    const tree = row({ line: "Idle · last: echo one" });
    expect(style(byText(tree, "project2"))).toMatchObject({ fontSize: 15, lineHeight: 20, color: stubColors.textBright });
    const line = byText(tree, "Idle · last: echo one")!;
    expect(style(line)).toMatchObject({ fontFamily: "ui", fontSize: 13, lineHeight: 17, color: stubColors.textMuted });
    expect(findAll(line, (n) => n.type === "Text")).toHaveLength(1);
  });
  it("marks a working row with the accent dot (not a spinner) and its time in JetBrains Mono 500 11/14", () => {
    const tree = row();
    const dot = findAll(tree, (n) => style(n).width === 8 && style(n).height === 8)[0]!;
    expect(style(dot)).toMatchObject({ borderRadius: 4, backgroundColor: stubColors.accent });
    expect(findAll(tree, (n) => n.type === "Spinner")).toHaveLength(0);
    expect(style(byText(tree, "4:32"))).toMatchObject({ fontFamily: "monoMedium", fontSize: 11, lineHeight: 14, color: stubColors.accent });
  });
  it("closes with a 32 pt ✕ in the muted grey, 44 to the touch", () => {
    const close = findAll(row(), (n) => n.props.accessibilityLabel === "Close tab project2")[0]!;
    expect([style(close).width, style(close).height, close.props.hitSlop]).toEqual([32, 32, 6]);
    expect(findAll(close, (n) => n.type === "Icon")[0]!.props.color).toBe(stubColors.textMuted);
  });
  it("the grip only on the whole list; without it the row starts with its title", () => {
    expect(findAll(row(), (n) => n.type === "Grip")).toHaveLength(1);
    const still = row({ movable: false });
    expect(findAll(still, (n) => n.type === "Grip")).toHaveLength(0);
    expect((still.props.children as Node[]).filter(Boolean)[0]!.type).toBe("Pressable");
  });
  it("the current row on the accent tint #19aad80f", () => {
    expect(style(row({ selected: true })).backgroundColor).toBe("#19aad80f");
    // the stub mirrors the real Dark theme's tint
    expect(paletteFrom(DARK_DEFAULT).tints.selected).toBe(stubColors.tints.selected);
  });
});

describe("Tabs page: the New Lists board", () => {
  const page = (filter: string, extra: Record<string, unknown> = {}) => {
    const tabs = [tab(0, "project1"), { ...tab(1, "notes"), pinned: true }, { ...tab(2, "project2"), panes: [pane(12), pane(13, { active: false })] }];
    const deps = uiDeps("ios", {
      react: { ...(uiDeps("ios").react as object), memo: (c: unknown) => c },
      "react-native": { ...(uiDeps("ios")["react-native"] as object), FlatList: "FlatList" },
      "@/ui/useActionScope": { useActionScope: () => ({ capture: () => () => true, leave: () => true }) },
      "expo-router": { useRouter: () => ({ push: vi.fn(), back: vi.fn(), replace: vi.fn(), dismiss: vi.fn(), canGoBack: () => true }), useLocalSearchParams: () => ({ key: "A", pane: "10" }) },
      "@/now": now,
      "@/fleet": { canClose: () => true, canMove: () => true, canOperate: () => true },
      "@/paneView": paneView,
      "@/runtime": { fleet: { waitingSince: () => null }, useApp: () => ({ hosts: [host("A", "studio-mac", conn({}), { windows: [{ pid: 1, background: false, tabs }] })], asked: [] }), useLock: () => ({ locked: false, covered: false }) },
      "@/ui/Field": { Field: "Field" },
      "@/ui/FilterChip": { FilterChip: "FilterChip" },
      "@/ui/Icon": { Icon: "Icon" },
      "@/ui/ListRow": { LIST_ROW_H: 63, ListRow: "ListRow" },
      "@/ui/newTab": { canNewTab: () => true },
      "@/ui/Button": { Button: "Button" },
      "@/ui/Notice": { ErrorText: "ErrorText" },
      "@/ui/Screen": { IconButton: "IconButton", PageHeader: "PageHeader" },
      "@/ui/theme": uiDeps("ios")["./theme"],
      "@/ui/ReorderRow": {
        ReorderRow: ({ children }: { children: unknown }) => (Array.isArray(children) ? children[0] : children)({ grip: (n: unknown) => n, accessibilityActions: [] }),
        useReorder: () => ({ drag: { active: null }, setDrag: vi.fn(), scrollEnabled: true }),
      },
      "@/ui/useClose": { useClose: () => vi.fn() },
      "@/ui/useMove": { useMove: () => vi.fn() },
      "@/ui/useNow": { useNow: () => NOW },
      "@/ui/useToast": { useToast: () => ({ say: vi.fn(), toast: null }) },
      ...extra,
    });
    const { default: Tabs } = load("app/tabs.tsx", deps, (s) => s.replace('useState<SheetFilter>("all")', `useState<SheetFilter>("${filter}")`)) as { default: () => unknown };
    return render({ type: Tabs, props: { children: [] } });
  };
  const list = (tree: unknown) =>
    findAll(tree, (n) => n.type === "FlatList")[0]!.props as unknown as {
      data: { kind: string; key: string }[];
      renderItem: (p: { item: unknown }) => unknown;
      ListEmptyComponent: unknown;
      ListFooterComponent: unknown;
      contentContainerStyle: unknown;
    };

  it("“Search tabs” right under the header, no dot before the subtitle", () => {
    const tree = page("all");
    expect(findAll(tree, (n) => n.type === "Field")[0]!.props.placeholder).toBe("Search tabs");
    const tools = findAll(tree, (n) => style(n).gap === 10 && style(n).paddingHorizontal === 14)[0]!;
    expect(style(tools).paddingTop ?? 0).toBe(0);
    expect(findAll(tree, (n) => n.type === "PageHeader")[0]!.props.dot).toBeUndefined();
  });
  it("captions with no hairline under them", () => {
    const l = list(page("all"));
    const cap = render(l.renderItem({ item: l.data[0] })) as Node;
    expect(textOf(cap)).toBe("Pinned");
    expect(style(cap)).toMatchObject({ fontSize: 12, lineHeight: 16, paddingTop: 18, paddingBottom: 8, paddingHorizontal: 20 });
    expect(style(cap).borderBottomWidth).toBeUndefined();
  });
  it("“2 panes” in a hairline badge of radius 8", () => {
    const l = list(page("all"));
    const item = l.data.find((i) => i.kind === "row" && i.key.endsWith(":12"))!;
    const row = render(l.renderItem({ item })) as Node;
    const marks = render(row.props.marks) as unknown;
    const badge = findAll(marks, (n) => n.props.accessibilityLabel === "2 panes in project2")[0]!;
    expect(style(badge)).toMatchObject({ borderRadius: 8, borderWidth: 1, borderColor: stubColors.hairline, paddingHorizontal: 6 });
  });
  it("an empty filter: the message centred, title 17, Show all tabs a 48 pt accent text button", () => {
    const l = list(page("needs"));
    expect(flat(l.contentContainerStyle).flexGrow).toBe(1);
    const empty = render(l.ListEmptyComponent) as Node;
    expect(style(empty)).toMatchObject({ flex: 1, justifyContent: "center", alignItems: "center", gap: 8, padding: 24 });
    expect(style(byText(empty, "Nothing needs you"))).toMatchObject({ fontSize: 17, lineHeight: 22 });
    const show = findAll(empty, (n) => n.type === "Button")[0]!;
    expect([show.props.label, show.props.variant, show.props.size]).toEqual(["Show all tabs", "text", undefined]);
  });
});

describe("Panes page: no hairline above the first row", () => {
  it("lists its rows straight under the caption", () => {
    const panes = [pane(10, { job: "node", last_command: "npm run dev" }), pane(11, { active: false, last_command: "git --version", last_exit: 0 })];
    const deps = uiDeps("ios", {
      "expo-router": { useRouter: () => ({ push: vi.fn(), back: vi.fn(), replace: vi.fn(), dismiss: vi.fn(), canGoBack: () => true }), useLocalSearchParams: () => ({ key: "A", pane: "10" }) },
      "@/now": now,
      "@/fleet": { canClose: () => true, canMove: () => true, canOperate: () => true },
      "@/paneView": paneView,
      "@/runtime": { fleet: { waitingSince: () => null }, useApp: () => ({ hosts: [host("A", "studio-mac", conn({}))], asked: [] }), useLock: () => ({ locked: false, covered: false }) },
      "@/topology": { findPane: () => ({ tab: { index: 0, title: "project2", active: true, panes }, pane: panes[0] }), tabTitle: (t: { title: string }) => t.title },
      "@/ui/Avatar": { Tile: "Tile" },
      "@/ui/ListRow": { LIST_ROW_H: 63, ListRow: "ListRow" },
      "@/ui/newTab": { canNewPane: () => true },
      "@/ui/Notice": { ErrorText: "ErrorText" },
      "@/ui/Screen": { IconButton: "IconButton", PageHeader: "PageHeader", SectionTitle: "SectionTitle" },
      "@/ui/theme": uiDeps("ios")["./theme"],
      "@/ui/ReorderRow": { ReorderRow: ({ children }: { children: unknown }) => (Array.isArray(children) ? children[0] : children)(undefined), useReorder: () => ({ drag: { active: null }, setDrag: vi.fn(), scrollEnabled: true }) },
      "@/ui/useClose": { useClose: () => vi.fn() },
      "@/ui/useMove": { useMove: () => vi.fn() },
      "@/ui/useNow": { useNow: () => NOW },
      "@/ui/useToast": { useToast: () => ({ say: vi.fn(), toast: null }) },
    });
    const { default: Panes } = load("app/panes.tsx", deps) as { default: () => unknown };
    const tree = render({ type: Panes, props: { children: [] } });
    const listBox = findAll(tree, (n) => n.props.accessibilityRole === "list")[0]!;
    expect(style(listBox).borderTopWidth).toBeUndefined();
    expect(findAll(listBox, (n) => n.type === "ListRow")).toHaveLength(2);
  });
});

describe("Portals: the New Portals board", () => {
  const asking = { ...tab(0, "project4"), panes: [pane(10, { job: "claude", last_command: "claude", waiting: { kind: "choice", prompt: "Make this edit to notes.txt?", choices: ["Yes", "No"], context: [], numbered: false } })] };
  const hosts = [
    host("A", "studio-mac", conn({}), { windows: [{ pid: 1, background: false, tabs: [asking, tab(1, "notes"), { ...tab(2, "project2"), panes: [pane(12), pane(13, { active: false })] }] }] }),
    host("B", "build-box", UNREACHABLE, { record: { key: "B", name: "build-box", addresses: [], permission: "operate", pairedAt: "2026-11-01T09:00:00Z", seenAt: NOW - 2 * 3_600_000 } }),
  ];
  const deps = uiDeps("ios", {
    react: { ...(uiDeps("ios").react as object), memo: (c: unknown) => c },
    "react-native": { ...(uiDeps("ios")["react-native"] as object), AccessibilityInfo: {} },
    "@/ui/useActionScope": { useActionScope: () => ({ capture: () => () => true }) },
    "expo-router": { useRouter: () => ({ push: vi.fn(), back: vi.fn(), replace: vi.fn(), canGoBack: () => true }), useIsFocused: () => true, usePathname: () => "/x" },
    "@/answer": answer,
    "@/now": now,
    "@/fleet": { canOperate: () => true },
    "@/lock": { needsAuthBeforeRun: () => false },
    "@/runtime": { authenticate: vi.fn(), fleet: { retry: vi.fn(), unpair: vi.fn(), waitingSince: () => null, answer: vi.fn() }, useApp: () => ({ hosts, ready: true, loadError: null, asked: [], lastView: null, settings: {} }) },
    "@/stores": { restoreTarget: () => null },
    "@/ui/Avatar": { Tile: "Tile" },
    "@/ui/Button": { Button: "Button" },
    "@/ui/ConnectionSheet": { lineColor: () => ({ muted: "m", busy: "b", bad: "r", dim: "d" }) },
    "@/ui/connStatus": connStatus,
    "@/ui/hostDot": hostDot,
    "@/ui/FirstRun": { FirstRun: "FirstRun" },
    "@/ui/Icon": { Icon: "Icon" },
    "@/ui/confirm": { confirmDestructive: vi.fn() },
    "@/ui/ListRow": { RowMark: "RowMark" },
    "@/ui/newTab": { canNewTab: (h: { conn: ConnState }) => h.conn.status === "open" },
    "@/ui/Notice": { ErrorText: "ErrorText" },
    "@/ui/ProgressBar": { ProgressBar: "ProgressBar" },
    "@/ui/Rows": { NavRow: "NavRow" },
    "@/ui/SwipeAction": { SwipeAction: "SwipeAction" },
    "@/ui/Screen": { IconButton: "IconButton", glyph: (m: number) => m, headerTop: () => 69, tabBottom: () => 0 },
    "@/ui/theme": uiDeps("ios")["./theme"],
    "@/ui/useNow": { useNow: () => NOW },
  });
  const { default: Portals } = load("app/index.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Portals, props: { children: [] } });
  const cards = findAll(tree, (n) => n.type === "SwipeAction").map((n) => n.props.children as unknown);

  it("the header 16 pt in, the cards 14 apart", () => {
    const header = findAll(tree, (n) => style(n).paddingBottom === 12 && style(n).gap === 12)[0]!;
    expect(style(header).paddingHorizontal).toBe(16);
    expect(findAll(tree, (n) => n.type === "ScrollView")[0]!.props.contentContainerStyle).toEqual(expect.arrayContaining([expect.objectContaining({ paddingTop: 4, gap: 14, paddingHorizontal: 16 })]));
  });
  it("New tab is a 34 pt round glass button on each card, the unreachable one too", () => {
    const plus = findAll(tree, (n) => String(n.props.accessibilityLabel ?? "").startsWith("New tab on "));
    expect(plus.map((n) => n.props.accessibilityLabel)).toEqual(["New tab on studio-mac", "New tab on build-box"]);
    expect(style(plus[1])).toMatchObject({ width: 34, height: 34, borderRadius: 17, backgroundColor: stubColors.glass, borderColor: stubColors.glassRim });
  });
  it("the card's line has no chevron; the unreachable one is grey, with no hint paragraph", () => {
    expect(findAll(tree, (n) => n.type === "Icon" && n.props.name === "forward")).toHaveLength(0);
    const line = byText(cards[1], "Can’t reach it · last seen 2 h ago")!;
    expect(flat(line.props.style).color).toBe(stubColors.textMuted);
    expect(textOf(cards[1])).not.toContain("Check that your computer is on");
  });
  it("rows 62 pt; the needs-you row on the accent tint; the line without the program; “2 panes” in the title's 600 white", () => {
    const rows = findAll(cards[0], (n) => n.props.accessibilityHint === "Opens the tab");
    expect(rows).toHaveLength(3);
    expect(style(rows[0]).minHeight).toBe(62);
    const needs = findAll(cards[0], (n) => style(n).backgroundColor === stubColors.tints.selected);
    expect(needs).toHaveLength(1);
    expect(textOf(needs[0])).toContain("Waiting for you · Make this edit to notes.txt?");
    expect(textOf(needs[0])).not.toContain("claude ·");
    expect(style(byText(cards[0], " · 2 panes"))).toMatchObject({ color: stubColors.textBright, fontSize: 15 });
  });
});

describe("First run: a plain list of steps (the New Portals board)", () => {
  const deps = uiDeps("ios", { "expo-router": { useRouter: () => ({ push: vi.fn() }) }, "./Button": { Button: "Button" } });
  const { FirstRun } = load("ui/FirstRun.tsx", deps) as { FirstRun: () => unknown };
  const tree = render({ type: FirstRun, props: { children: [] } });

  it("a hairline under the header, the #050505 page", () => {
    expect(style(tree as Node).backgroundColor).toBe(stubColors.bg);
    const header = findAll(tree, (n) => style(n).borderBottomWidth === 1)[0]!;
    expect(style(header)).toMatchObject({ backgroundColor: stubColors.chrome, borderBottomColor: stubColors.hairline, paddingHorizontal: 14 });
  });
  it("three steps, no card: 28 pt circles 600 14, words 16/23 with no bold runs, 22 apart", () => {
    const list = findAll(tree, (n) => n.props.accessibilityRole === "list")[0]!;
    expect(style(list)).toEqual({ gap: 22 });
    const circles = findAll(list, (n) => style(n).width === 28);
    expect(circles.map((c) => [style(c).height, style(c).borderRadius, textOf(c)])).toEqual([[28, 14, "1"], [28, 14, "2"], [28, 14, "3"]]);
    const words = byText(list, "Go to Settings › Remote access and turn it on.")!;
    expect(style(words)).toMatchObject({ fontSize: 16, lineHeight: 23, color: stubColors.text });
    expect(findAll(words, (n) => n.type === "Text")).toHaveLength(1);
  });
  it("Scan the code, then Paste a pairing link", () => {
    expect(findAll(tree, (n) => n.type === "Button").map((n) => [n.props.label, n.props.variant])).toEqual([["Scan the code", "primary"], ["Paste a pairing link", "text"]]);
  });
});

describe("Computer menu: one hairline per row, 68 pt rows (the New Portals board)", () => {
  const deps = uiDeps("ios", { "@/now": now, "./connStatus": connStatus, "./hostDot": hostDot });
  const { ComputerMenu } = load("ui/ComputerMenu.tsx", deps) as { ComputerMenu: (p: object) => unknown };
  const hosts = [host("A", "studio-mac", conn({})), host("B", "build-box", UNREACHABLE, { record: { key: "B", name: "build-box", addresses: [], permission: "operate", pairedAt: "2026-11-01T09:00:00Z", seenAt: NOW - 2 * 3_600_000 } })];
  const tree = render({ type: ComputerMenu, props: { hosts, current: "A", now: NOW, top: 117, onOpen: vi.fn(), onConnection: vi.fn(), onPortals: vi.fn(), onDismiss: vi.fn(), children: [] } }) as Node;
  const items = findAll(tree, (n) => n.props.accessibilityRole === "menuitem");

  it("border #ffffff24, shadow 0 12 40", () => {
    expect(style(tree)).toMatchObject({ top: 117, borderColor: stubColors.borderStrong, shadowOffset: { width: 0, height: 12 }, shadowRadius: 20 });
  });
  it("every row 68 pt, a hairline under each but the last, no band", () => {
    expect(items).toHaveLength(4);
    expect(items.map((n) => [style(n).minHeight, style(n).paddingHorizontal, style(n).borderBottomWidth ?? 0, style(n).borderTopWidth ?? 0])).toEqual([
      [68, 16, 1, 0],
      [68, 16, 1, 0],
      [68, 16, 1, 0],
      [68, 16, 0, 0],
    ]);
  });
  it("names 16 in #e6e6e6, lines 13; an unreachable computer is not dimmed", () => {
    expect(style(byText(items[1], "build-box"))).toMatchObject({ fontSize: 16, lineHeight: 21, color: stubColors.text });
    expect(style(byText(items[1], "Can’t reach it · last seen 2 h ago"))).toMatchObject({ fontSize: 13, lineHeight: 17 });
  });
});

describe("Connection sheet: one line per path, the one in use ticked, Test again at the foot", () => {
  const status = (p: PathStatus["path"], extra: Partial<PathStatus>): PathStatus => ({ path: p, state: "idle", error: null, latencyMs: null, reached: false, waitedMs: null, ...extra }) as PathStatus;
  const lan = { kind: "lan" as const, address: "192.168.1.15:7878" };
  const relay = { kind: "relay" as const, url: "wss://relay.example.com" };
  const c = conn({ path: lan, paths: [status(lan, { state: "ok", latencyMs: 2 }), status(relay, {})] });
  const deps = uiDeps("ios", {
    "expo-router": { useRouter: () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true }) },
    "@/runtime": { fleet: { retry: vi.fn() }, useApp: () => ({ hosts: [host("A", "studio-mac", c)] }), useLock: () => ({ locked: false, covered: false }) },
    "./connStatus": connStatus,
    "./useNow": { useNow: () => NOW },
  });
  const { ConnectionSheet } = load("ui/ConnectionSheet.tsx", deps) as { ConnectionSheet: (p: object) => unknown };
  const tree = render({ type: ConnectionSheet, props: { hostKey: "A", children: [] } });

  it("says it is connected under the title", () => {
    expect(textOf(tree)).toContain("studio-mac · connected");
  });
  it("each path: title 16, one Inter line of where and why, ✓ on the one in use", () => {
    const rows = findAll(tree, (n) => style(n).minHeight === 68);
    expect(rows.map((r) => textOf(r))).toEqual([
      "This network192.168.1.15 · in use · 2 ms",
      "Relayrelay.example.com · not needed on the same network",
      "End-to-end encryptedThe relay passes bytes it cannot read.",
    ]);
    expect(rows.map((r) => findAll(r, (n) => n.type === "Icon" && n.props.name === "check").length)).toEqual([1, 0, 0]);
    expect(style(byText(rows[0], "This network"))).toMatchObject({ fontSize: 16, lineHeight: 21, color: stubColors.text });
    expect(style(byText(rows[0], "192.168.1.15 · in use · 2 ms"))).toMatchObject({ fontFamily: "ui", fontSize: 13, lineHeight: 17 });
    const lock = findAll(rows[2], (n) => style(n).width === 28)[0]!;
    expect(style(lock)).toMatchObject({ height: 28, borderRadius: 8, backgroundColor: stubColors.faint });
  });
  it("Test again pinned to the foot of a sheet the board's 541 pt tall", () => {
    const root = tree as Node;
    // the board's height, never a growing root: that collapses a fitToContents sheet to nothing (seen on iOS 27)
    // iOS adds the home indicator's inset (the helper's 34) under the content: the sheet is 541
    expect(style(root)).toMatchObject({ height: 541 - 34 });
    expect(style(root).flexGrow).toBeUndefined();
    expect(style(root).flex).toBeUndefined();
    expect(root.props.collapsable).toBe(false);
    const parts = (root.props.children as unknown[]).flat().filter(Boolean) as Node[];
    const foot = parts[parts.length - 1]!;
    expect(findAll(foot, (n) => n.props.accessibilityLabel === "Test again")).toHaveLength(1);
    expect(style(foot)).toMatchObject({ paddingBottom: 0, paddingHorizontal: 16 });
    // the rows scroll between the header and the foot
    expect(parts.map((p) => p.type)).toEqual(["View", "ScrollView", "View"]);
  });
});

describe("Computer settings: Unpair as an outlined red button (the New Portals board)", () => {
  const deps = uiDeps("ios", {
    "expo-router": { useRouter: () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true, push: vi.fn() }), useLocalSearchParams: () => ({ key: "A" }) },
    "@/fleet": { canOperate: () => true },
    "@/runtime": { fleet: { unpair: vi.fn() }, useApp: () => ({ hosts: [host("A", "studio-mac", conn({}))], ready: true }) },
    "@/ui/connStatus": connStatus,
    "@/ui/format": { osName: (o: string) => o },
    "@/ui/Notice": { ErrorText: "ErrorText" },
    "@/ui/Button": { Button: "Button" },
    "@/ui/Rows": { CardBox: "CardBox", Group: "Group", NavRow: "NavRow", SettingsPage: "SettingsPage", SwitchRow: "SwitchRow", ValueRow: "ValueRow" },
    "@/ui/settingsText": settingsText,
    "@/ui/theme": uiDeps("ios")["./theme"],
    "@/ui/useHostNotifications": { useHostNotifications: () => ({ saving: false, change: vi.fn() }) },
    "@/ui/useNow": { useNow: () => NOW },
  });
  const { default: Computer } = load("app/computer.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Computer, props: { children: [] } });

  it("a destructive Button, the note under it 13/18, no card row", () => {
    const unpair = findAll(tree, (n) => n.type === "Button")[0]!;
    expect([unpair.props.variant, unpair.props.label]).toEqual(["destructive", "Unpair this computer"]);
    expect(findAll(tree, (n) => n.type === "NavRow" && !!n.props.destructive)).toHaveLength(0);
    const note = byText(tree, "To pair again you need a new code from Rift on the computer.")!;
    expect(style(note)).toMatchObject({ fontSize: 13, lineHeight: 18, color: stubColors.textMuted });
  });
});
