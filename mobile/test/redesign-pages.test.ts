import { describe, expect, it, vi } from "vitest";
import * as connStatus from "../src/ui/connStatus";
import * as hostDot from "../src/ui/hostDot";
import * as now from "../src/now";
import * as paneView from "../src/paneView";
import * as answer from "../src/answer";
import type { ConnState } from "../src/protocol/client";
import type { Waiting } from "../src/protocol/types";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, render, textOf, uiDeps } from "./helpers/render";

// The redesign's page contents (the New Portals, New Screen and New Sheets boards), run as the
// app's own modules: props in, the rendered words, roles and handlers out. Pixels stay device checks.

const NOW = Date.UTC(2026, 10, 6, 12);

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

const tab = (index: number, title: string) => ({ index, title, active: index === 0, panes: [{ id: index + 10, active: true, cwd: null, job: null, last_command: null, last_exit: null }] });

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

describe("Computer menu: each computer with where it is, the current one ticked", () => {
  const deps = uiDeps("ios", { "@/now": now, "./connStatus": connStatus, "./hostDot": hostDot });
  const { ComputerMenu } = load("ui/ComputerMenu.tsx", deps) as { ComputerMenu: (p: object) => unknown };
  const hosts = [
    host("A", "studio-mac", conn({})),
    host("B", "build-box", conn({ status: "failed", failure: "gave_up", attempt: 5, path: null }), { record: { key: "B", name: "build-box", addresses: [], permission: "operate", pairedAt: "2026-11-01T09:00:00Z", seenAt: NOW - 2 * 3_600_000 } }),
    host("C", "pi-lab", conn({ path: { kind: "relay", url: "wss://relay.example.com" } })),
  ];
  const tree = render({ type: ComputerMenu, props: { hosts, current: "A", now: NOW, top: 0, onOpen: vi.fn(), onConnection: vi.fn(), onPortals: vi.fn(), onDismiss: vi.fn(), children: [] } });
  const items = findAll(tree, (n) => n.props.accessibilityRole === "menuitem" && n.props.accessibilityState !== undefined);

  it("says where each one is", () => {
    expect(items.map((n) => n.props.accessibilityLabel)).toEqual([
      "studio-mac, This network · 2 tabs",
      "build-box, Can’t reach it · last seen 2 h ago",
      "pi-lab, Over the relay · 2 tabs",
    ]);
  });
  it("ticks only the one on screen", () => {
    expect(items.map((n) => (n.props.accessibilityState as { checked: boolean }).checked)).toEqual([true, false, false]);
    expect(findAll(items[0], (n) => n.type === "Icon" && n.props.name === "check")).toHaveLength(1);
    expect(findAll(items[1], (n) => n.type === "Icon" && n.props.name === "check")).toHaveLength(0);
  });
});

describe("Connection sheet: the standard header and Test again", () => {
  const retry = vi.fn();
  const sheet = (c: ConnState) => {
    const deps = uiDeps("ios", {
      "expo-router": { useRouter: () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true }) },
      "@/runtime": { fleet: { retry }, useApp: () => ({ hosts: [host("A", "studio-mac", c)] }), useLock: () => ({ locked: false, covered: false }) },
      "./connStatus": connStatus,
      "./useNow": { useNow: () => NOW },
    });
    const { ConnectionSheet } = load("ui/ConnectionSheet.tsx", deps) as { ConnectionSheet: (p: object) => unknown };
    return render({ type: ConnectionSheet, props: { hostKey: "A", children: [] } });
  };
  const buttons = (tree: unknown) => findAll(tree, (n) => n.props.accessibilityRole === "button").map((n) => n.props.accessibilityLabel);

  it("has the sheet header (title, where, ✕)", () => {
    const tree = sheet(conn({}));
    expect(findAll(tree, (n) => n.props.accessibilityRole === "header").map(textOf)).toEqual(["Connection"]);
    expect(buttons(tree)).toContain("Close");
  });
  it("offers Test again while connected, which tries this network first", () => {
    const tree = sheet(conn({}));
    expect(buttons(tree)).toContain("Test again");
    expect(buttons(tree)).not.toContain("Try again now");
    const test = findAll(tree, (n) => n.props.accessibilityLabel === "Test again")[0]!;
    (test.props.onPress as () => void)();
    expect(retry).toHaveBeenCalledWith("A", { lanFirst: true });
  });
  it("offers Try again now when it cannot reach it", () => {
    const tree = sheet(conn({ status: "failed", failure: "gave_up", attempt: 5, path: null }));
    expect(buttons(tree)).toContain("Try again now");
    expect(buttons(tree)).not.toContain("Test again");
  });
});

describe("Agent question: the program named, buttons that say what happens, Tell it why", () => {
  const w: Waiting = { kind: "choice", prompt: "Make this edit to notes.txt?", choices: ["Yes", "Yes, and don't ask again this session", "No, and tell Claude what to do differently"], context: [], numbered: true } as unknown as Waiting;
  const card = (replying: boolean) => {
    const deps = uiDeps("ios", {
      "@/ui/useActionScope": { useActionScope: () => ({ capture: () => () => true }) },
      "@/answer": answer,
      "@/lock": { needsAuthBeforeRun: () => false },
      "@/paneView": paneView,
      "@/runtime": { authenticate: vi.fn(), fleet: { answer: vi.fn(), sendKeys: vi.fn() }, useApp: () => ({ settings: {} }) },
      "./useNow": { useNow: () => NOW },
      "./PromptText": { PromptText: "PromptText" },
      "expo-router": { useRouter: () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true }) },
      "react-native": { ...(uiDeps("ios")["react-native"] as object), AccessibilityInfo: {}, Keyboard: { dismiss() {} }, useWindowDimensions: () => ({ width: 390, height: 844 }) },
    });
    // the reply state is the card's own; a fixture opens it by its first value
    const { AskCard } = load("ui/AskCard.tsx", deps, (s) => (replying ? s.replace("const [replying, setReplying] = useState(false);", "const [replying, setReplying] = useState(true);") : s)) as {
      AskCard: (p: object) => unknown;
    };
    return render({ type: AskCard, props: { hostKey: "A", paneId: 31, waiting: w, program: "claude", where: "project4", bottomInset: 0, onSay: vi.fn(), children: [] } });
  };

  it("names the program and the tab it asks in", () => {
    const text = textOf(card(false));
    expect(text).toContain("Claude Code asks");
    expect(text).toContain("project4 · ");
  });
  it("says on each button what it does; No opens the field for why", () => {
    const labels = findAll(card(false), (n) => n.props.accessibilityRole === "button").map(textOf);
    expect(labels).toContain("No, explain");
    expect(labels).not.toContain("No…");
  });
  it("Tell it why is a sheet of its own: header, the field, Send; the question gives it its place", () => {
    const tree = card(true);
    expect(findAll(tree, (n) => n.props.accessibilityRole === "header").map(textOf)).toEqual(["Tell it why"]);
    expect(textOf(tree)).toContain("Claude Code · project4");
    expect(textOf(tree)).not.toContain("Make this edit to notes.txt?");
    const send = findAll(tree, (n) => n.props.accessibilityRole === "button" && textOf(n) === "Send");
    expect(send).toHaveLength(1);
    expect(flat(send[0]!.props.style).backgroundColor).toBe(stubColors.accent);
    expect(findAll(tree, (n) => n.props.accessibilityLabel === "Close")).toHaveLength(1);
  });
});

describe("Portals page", () => {
  const unpair = vi.fn(() => Promise.resolve());
  const retry = vi.fn();
  const push = vi.fn();
  const many = Array.from({ length: 30 }, (_, i) => tab(i, `project${i + 1}`));
  // project21 asks; the card shows it first
  many[20]!.panes[0] = { ...many[20]!.panes[0]!, waiting: { kind: "choice", prompt: "Make this edit to notes.txt?", choices: ["Yes", "No"], context: [], numbered: false } } as never;
  const hosts = [
    host("A", "studio-mac", conn({}), { windows: [{ pid: 1, background: false, tabs: many }] }),
    host("B", "studio-mac", conn({ status: "failed", failure: "gave_up", attempt: 5, path: null }), {
      record: { key: "B", name: "studio-mac", addresses: [], permission: "operate", pairedAt: "2026-10-12T09:00:00Z", seenAt: NOW - 2 * 3_600_000 },
    }),
  ];
  const confirm = vi.fn(() => Promise.resolve(true));
  const deps = uiDeps("ios", {
    react: { ...(uiDeps("ios").react as object), memo: (c: unknown) => c },
    "react-native": { ...(uiDeps("ios")["react-native"] as object), AccessibilityInfo: {} },
    "@/ui/useActionScope": { useActionScope: () => ({ capture: () => () => true }) },
    "expo-router": { useRouter: () => ({ push, back: vi.fn(), replace: vi.fn(), canGoBack: () => true }), useIsFocused: () => true, usePathname: () => "/x" },
    "@/answer": answer,
    "@/now": now,
    "@/fleet": { canOperate: () => true },
    "@/lock": { needsAuthBeforeRun: () => false },
    "@/runtime": { authenticate: vi.fn(), fleet: { retry, unpair, waitingSince: () => null, answer: vi.fn() }, useApp: () => ({ hosts, ready: true, loadError: null, asked: [], lastView: null, settings: {} }) },
    "@/stores": { restoreTarget: () => null },
    "@/ui/Avatar": { Tile: "Tile" },
    "@/ui/Button": { Button: "Button" },
    "@/ui/ConnectionSheet": { lineColor: () => ({ muted: "m", busy: "b", bad: "r", dim: "d" }) },
    "@/ui/connStatus": connStatus,
    "@/ui/hostDot": hostDot,
    "@/ui/FirstRun": { FirstRun: "FirstRun" },
    "@/ui/Icon": { Icon: "Icon" },
    "@/ui/confirm": { confirmDestructive: confirm },
    "@/ui/ListRow": { RowMark: "RowMark" },
    "@/ui/newTab": { canNewTab: () => true },
    "@/ui/Notice": { ErrorText: "ErrorText" },
    "@/ui/ProgressBar": { ProgressBar: "ProgressBar" },
    "@/ui/Rows": { NavRow: "NavRow" },
    "@/ui/SwipeAction": { SwipeAction: "SwipeAction" },
    "@/ui/Screen": { IconButton: "IconButton", glyph: (m: number) => m, headerTop: () => 0, tabBottom: () => 0 },
    "@/ui/theme": (uiDeps("ios")["./theme"] as object),
    "@/ui/useNow": { useNow: () => NOW },
  });
  const { default: Portals } = load("app/index.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Portals, props: { children: [] } });

  it("counts the tabs in one line of words, never chips", () => {
    expect(findAll(tree, (n) => n.type === "FilterChip")).toHaveLength(0);
    const summary = findAll(tree, (n) => n.type === "Text" && textOf(n).includes("needs you"))[0]!;
    expect(textOf(summary)).toBe("● 1 needs you · 29 idle");
  });
  it("a card of 30 tabs shows the five that matter in the computer's tab order, and a way to the rest", () => {
    const rows = findAll(tree, (n) => n.props.accessibilityHint === "Opens the tab");
    expect(rows).toHaveLength(5);
    const labels = rows.map((r) => String(r.props.accessibilityLabel));
    expect(labels.some((l) => /^project21, Needs you/.test(l))).toBe(true);
    const numbers = labels.map((l) => Number(/^project(\d+)/.exec(l)?.[1]));
    expect(numbers).toEqual([...numbers].sort((a, b) => a - b));
    const all = findAll(tree, (n) => n.props.accessibilityLabel === "Show all 30 tabs on studio-mac")[0]!;
    expect(textOf(all)).toBe("Show all 30 tabs");
    (all.props.onPress as () => void)();
    expect(push).toHaveBeenCalledWith({ pathname: "/tabs", params: { key: "A", from: "home" } });
  });
  it("twin pairings say where each is and when each was paired", () => {
    const lines = findAll(tree, (n) => String(n.props.accessibilityLabel ?? "").endsWith(". Connection details")).map((n) => n.props.accessibilityLabel);
    expect(lines).toEqual(["This network · 30 tabs · paired 3 Nov. Connection details", "Can’t reach it · last seen 2 h ago · paired 12 Oct. Connection details"]);
  });
  it("the card that cannot be reached offers Try again and Unpair, which asks first", async () => {
    const fixes = findAll(tree, (n) => n.type === "NavRow");
    expect(fixes.map((n) => [n.props.label, !!n.props.accent, !!n.props.destructive])).toEqual([
      ["Try again", true, false],
      ["Unpair", false, true],
    ]);
    (fixes[0]!.props.onPress as () => void)();
    expect(retry).toHaveBeenCalledWith("B");
    (fixes[1]!.props.onPress as () => void)();
    await Promise.resolve();
    expect(confirm).toHaveBeenCalledTimes(1);
    await new Promise((r) => setTimeout(r, 0));
    expect(unpair).toHaveBeenCalledWith("B");
  });
});

describe("Tabs page: sections, and an empty filter that says so and offers the way back", () => {
  const page = (filter: string, tabs = [tab(0, "project1"), { ...tab(1, "notes"), pinned: true }]) => {
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
      "@/ui/ListRow": { LIST_ROW_H: 60, ListRow: "ListRow" },
      "@/ui/newTab": { canNewTab: () => true },
      "@/ui/Button": { Button: "Button" },
      "@/ui/Notice": { ErrorText: "ErrorText" },
      "@/ui/Screen": { IconButton: "IconButton", PageHeader: "PageHeader" },
      "@/ui/theme": uiDeps("ios")["./theme"],
      "@/ui/ReorderRow": { ReorderRow: "ReorderRow", useReorder: () => ({ drag: { active: null }, setDrag: vi.fn(), scrollEnabled: true }) },
      "@/ui/useClose": { useClose: () => vi.fn() },
      "@/ui/useMove": { useMove: () => vi.fn() },
      "@/ui/useNow": { useNow: () => NOW },
      "@/ui/useToast": { useToast: () => ({ say: vi.fn(), toast: null }) },
    });
    const { default: Tabs } = load("app/tabs.tsx", deps, (s) => s.replace('useState<SheetFilter>("all")', `useState<SheetFilter>("${filter}")`)) as { default: () => unknown };
    const list = findAll(render({ type: Tabs, props: { children: [] } }), (n) => n.type === "FlatList")[0]!;
    return list.props as unknown as { data: { kind: string; text?: string }[]; ListEmptyComponent: unknown };
  };

  it("the whole list: Pinned, then Tabs · Drag ≡ to reorder", () => {
    expect(page("all").data.filter((i) => i.kind === "cap").map((i) => i.text)).toEqual(["Pinned", "Tabs · Drag ≡ to reorder"]);
  });
  it("Needs you with nothing to show says so, and Show all tabs goes back", () => {
    const p = page("needs");
    expect(p.data).toEqual([]);
    const empty = render(p.ListEmptyComponent);
    expect(findAll(empty, (n) => n.props.accessibilityRole === "header").map(textOf)).toEqual(["Nothing needs you"]);
    expect(textOf(empty)).toContain("Agents that ask a question show here.");
    expect(findAll(empty, (n) => n.type === "Button").map((n) => n.props.label)).toEqual(["Show all tabs"]);
  });
});

describe("Portals: a summary line in words, never chips; the unreachable card's fixes", () => {
  it("reads the counts as one line, what needs you first", () => {
    const rows = [{ kind: "idle" }, { kind: "working" }, { kind: "needs" }, { kind: "failed" }, { kind: "idle" }] as const;
    expect(now.homeSummary(rows).map((p) => p.text).join(" · ")).toBe("1 needs you · 1 working · 1 failed · 2 idle");
  });
  it("an old pairing that cannot be reached says when it was last seen and when it was paired", () => {
    const line = connStatus.hostLine(conn({ status: "failed", failure: "gave_up", attempt: 5, path: null }), NOW, null, NOW - 2 * 3_600_000);
    expect(`${line.text} · ${connStatus.pairedText("2026-11-03T09:00:00Z", NOW)}`).toBe("Can’t reach it · last seen 2 h ago · paired 3 Nov");
    expect(line.retry).toBe(true);
  });
});

describe("Sheets say it in words", () => {
  it("New tab and New pane rows say what each start does", async () => {
    const { programLine, PROGRAMS } = await import("../src/launch");
    expect(PROGRAMS.map((p) => programLine(p, "tab"))).toEqual(["A plain shell", "Opens a shell, then runs claude", "Opens a shell, then runs codex", "Opens a shell, then runs opencode"]);
  });
  it("a block's status is in words", () => {
    expect(paneView.sheetChip({ state: "done", exit: 1 } as never).text).toBe("✕ Failed with 1");
  });
});

describe("Panes page: each row leads with the program's tile, as on Portals", () => {
  const leads = (edit?: (s: string) => string) => {
    const panes = [
      { id: 10, active: true, cwd: "/Users/me/code/project4", job: "claude", last_command: "claude", last_exit: null },
      { id: 11, active: false, cwd: "/Users/me/code/project4", job: "codex", last_command: "codex", last_exit: null },
      { id: 12, active: false, cwd: "/Users/me/code/project4", job: null, last_command: "ls", last_exit: 0 },
    ];
    const deps = uiDeps("ios", {
      "expo-router": { useRouter: () => ({ push: vi.fn(), back: vi.fn(), replace: vi.fn(), dismiss: vi.fn(), canGoBack: () => true }), useLocalSearchParams: () => ({ key: "A", pane: "10" }) },
      "@/now": now,
      "@/fleet": { canClose: () => true, canMove: () => true, canOperate: () => true },
      "@/paneView": paneView,
      "@/runtime": { fleet: { waitingSince: () => null, selectPane: vi.fn() }, useApp: () => ({ hosts: [host("A", "studio-mac", conn({}), { windows: [{ pid: 1, background: false, tabs: [{ index: 0, title: "project4", active: true, panes }] }] })], asked: [] }), useLock: () => ({ locked: false, covered: false }) },
      "@/topology": { findPane: () => ({ tab: { index: 0, title: "project4", active: true, panes }, pane: panes[0] }), tabTitle: (t: { title: string }) => t.title },
      "@/ui/Avatar": { Tile: "Tile" },
      "@/ui/ListRow": { LIST_ROW_H: 60, ListRow: "ListRow" },
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
    const { default: Panes } = load("app/panes.tsx", deps, edit) as { default: () => unknown };
    return findAll(render({ type: Panes, props: { children: [] } }), (n) => n.type === "ListRow").map((n) => n.props.lead as { type: unknown; props: Record<string, unknown> });
  };

  it("✳ for Claude Code, the program's own mark otherwise, from the one Tile", () => {
    const l = leads();
    expect(l.map((n) => n.type)).toEqual(["Tile", "Tile", "Tile"]);
    expect(l.map((n) => [n.props.agent, n.props.program])).toEqual([[true, "claude"], [false, "codex"], [false, "ls"]]);
  });
  it("negative control: a hand-made tile in its place is caught", () => {
    expect(leads((s) => s.replace("lead={<Tile ", "lead={<View ")).map((n) => n.type)).toEqual(["View", "View", "View"]);
  });
});
