import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { failureMessage, lastLines, parseLogLines } from "../src/paneView";
import type { Block } from "../src/protocol/types";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, react, render, textOf, uiDeps, type Node } from "./helpers/render";

// The pane against the planned New Blocks board (batches 3 and 4 of the pixel plan): the header
// block, the tab row, the toast, the ⋯ menu, a closed pane, the block cards, the opening card,
// the command line's keys. Layout and pixels stay device checks (the sim captures).

const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");
const pane = src("app/host/[key]/pane/[id].tsx");

describe("the header block", () => {
  it("holds the computer's row, the tab row and the search field, with one hairline under all of it", () => {
    const header = pane.slice(pane.indexOf("<View style={[styles.header, { paddingTop: headerPad }]}>"), pane.indexOf("{loc && tab && !closed && !compact && tab.panes.length > 1"));
    expect(header).toContain("{tabBar}");
    expect(header).toContain('label="Search output"');
    expect(header).toContain("{`${shownBlocks} of ${blocks.length} ${blocks.length === 1 ? \"block\" : \"blocks\"}`}");
    expect(pane).toContain(
      'header: { backgroundColor: colors.chrome, paddingHorizontal: 14, paddingBottom: 8, gap: 8, borderBottomWidth: 1, borderBottomColor: colors.hairline },',
    );
    expect(pane).toContain("const headerPad = compact ? Math.max(insets.top, 6) : headerTop(insets.top);");
    expect(pane).toContain('title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 16, lineHeight: 20 },');
    // the TabBar draws no ground or hairline of its own
    expect(src("ui/TabBar.tsx")).toContain('bar: { flexDirection: "row", alignItems: "center", gap: 8 },');
  });
  it("⌕ and ⋯ are two round glass buttons, over a program too, ⌕ untinted while searching", () => {
    expect(pane).toContain('<IconButton glass icon="search" label="Search output" pressed={searching !== null} onPress={() => setSearch(searching === null ? "" : null)} />');
    expect(pane).not.toContain("GlassGroup");
    expect(src("ui/Screen.tsx")).not.toContain("GlassGroup");
    // searching shows the blocks, over a full-screen program as well, until the search closes
    expect(pane).toContain("const fullScreen = view.tui && search === null;");
    expect(pane).toMatch(/<IconButton\s+glass\s+icon="more"\s+label="More"\s+expanded=\{menu\}/);
  });
  it("the count sits centred under the field, 12/16", () => {
    expect(pane).toContain('searchCount: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16, textAlign: "center" },');
    expect(pane).toContain("search: { paddingTop: 4 },");
  });
});

describe("the tab row", () => {
  const Gesture = { Race: () => ({}), Fling: () => { const f: Record<string, unknown> = {}; for (const k of ["runOnJS", "direction", "enabled", "onEnd"]) f[k] = () => f; return f; } };
  const deps = uiDeps("ios", {
    "react-native-gesture-handler": { Directions: { LEFT: 1, RIGHT: 2 }, Gesture, GestureDetector: "GestureDetector" },
    "./StatusMark": { KIND_COLOR: { needs: "warning", working: "accent", failed: "error", idle: "success" } },
  });
  const { TabBar } = load("ui/TabBar.tsx", deps) as { TabBar: (p: object) => unknown };
  const draw = (p: object = {}) =>
    render({ type: TabBar, props: { title: "project1", kind: "idle", number: 2, count: 30, prev: "notes", next: null, onPrev: vi.fn(), onNext: vi.fn(), onOpen: vi.fn(), children: [], ...p } }) as Node;
  it("‹ › are 32 pt glass circles; the dot comes before the name, the place under it", () => {
    const tree = draw();
    const prev = findAll(tree, (n) => n.props.accessibilityLabel === "Previous tab: notes")[0]!;
    expect(flat(prev.props.style)).toMatchObject({ width: 32, height: 32, borderRadius: 16, backgroundColor: stubColors.glass, borderColor: stubColors.glassRim });
    const middle = findAll(tree, (n) => n.props.accessibilityLabel === "project1, tab 2 of 30. Open the tabs")[0]!;
    const line = (middle.props.children as Node[])[0]!;
    const [dot, name] = line.props.children as Node[];
    expect(flat(dot!.props.style)).toMatchObject({ width: 7, height: 7, backgroundColor: stubColors.success });
    expect(textOf(name)).toBe("project1");
    expect(flat(name!.props.style)).toMatchObject({ fontSize: 14, lineHeight: 18 });
    expect(textOf(middle)).toBe("project1tab 2 of 30");
  });
  it("a dot, never a spinner: working is the accent; the dot can be grey while the pane opens", () => {
    const working = findAll(draw({ kind: "working" }), (n) => n.type === "View" && flat(n.props.style).width === 7)[0]!;
    expect(flat(working.props.style).backgroundColor).toBe(stubColors.accent);
    const opening = findAll(draw({ kind: "failed", dot: "textMuted" }), (n) => n.type === "View" && flat(n.props.style).width === 7)[0]!;
    expect(flat(opening.props.style).backgroundColor).toBe(stubColors.textMuted);
    expect(pane).toContain('dot={closed ? "textDim" : waitingForPane ? "textMuted" : undefined}');
  });
});

describe("the toast", () => {
  const { Toast } = load("ui/useToast.tsx", uiDeps("ios")) as { Toast: (p: object) => unknown };
  it("is a dark pill with a rim and a shadow, white 500 13 (not a white pill)", () => {
    const tree = render({ type: Toast, props: { text: "Notifications off for this computer", children: [] } }) as Node;
    const text = findAll(tree, (n) => n.type === "Text")[0]!;
    expect(flat(text.props.style)).toMatchObject({
      color: stubColors.textBright,
      fontSize: 13,
      lineHeight: 18,
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderRadius: 18,
      borderWidth: 1,
      borderColor: stubColors.borderStrong,
      backgroundColor: stubColors.menu,
    });
    expect(flat(tree.props.style)).toMatchObject({ shadowOpacity: 0.6, shadowOffset: { width: 0, height: 8 } });
  });
  it("hangs over the tab row, as the ⋯ menu does", () => {
    expect(pane).toContain("const MENU_DROP = 58;");
    expect(pane).toContain("style={[styles.toastWrap, { top: headerPad + MENU_DROP }]}");
    expect(pane).toContain("style={[styles.menu, { top: headerPad + MENU_DROP }]}");
    expect(pane).toContain("<Toast text={toast} />");
  });
});

describe("a pane that closed", () => {
  it("keeps the tab row of its tab and offers the tab's other pane, or Portals", () => {
    expect(pane).toContain("const rowLoc = loc ?? closedLoc;");
    expect(pane).toContain("This pane has closed\n");
    expect(pane).toContain("Its tab has ${closedLoc.tab.panes.length === 1 ? \"one other pane\"");
    expect(pane).toContain('<Button label="Other pane" variant="primary" numberOfLines={1} style={styles.closedAction} onPress={() => goPane(closedLoc.pane.id)} />');
    expect(pane).toContain('closedAction: { width: 106, paddingHorizontal: 8 },');
    expect(pane).toContain('<Button label="Portals" variant={closedLoc ? "secondary" : "primary"}');
    expect(pane).toContain('closed: { flex: 1, backgroundColor: colors.bg, alignItems: "center", justifyContent: "center", gap: 14, padding: 24 },');
    expect(pane).toContain('closedTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 18, lineHeight: 24, textAlign: "center" },');
  });
});

const block = (b: Partial<Block>): Block => ({
  id: 1,
  command: "npm test",
  output: "Tests: 2 failed, 41 passed\n",
  state: "done",
  exit: 1,
  duration_ms: 1200,
  cwd: "~/code/project1",
  dropped: 0,
  ...b,
}) as Block;

describe("the block cards", () => {
  const deps = uiDeps("ios", {
    react: { ...react, memo: (c: unknown) => c },
    "expo-clipboard": { setStringAsync: async () => {} },
    "@/paneView": { failureMessage, lastLines, parseLogLines },
    "./Button": { Button: "Button" },
    "./format": { formatClock: (ms: number) => `0:${String(Math.round(ms / 1000)).padStart(2, "0")}` },
    "./Marked": { Marked: "Marked" },
    "./SelectableText": { SelectableText: "SelectableText" },
    "./StatusMark": { Spinner: "Spinner" },
  });
  const { FailedCard, RunningCard } = load("ui/BlockCards.tsx", deps) as Record<string, { type: (p: object) => unknown }>;
  // memo() in the stand-in React is the component itself
  const draw = (C: unknown, p: object) => render({ type: C, props: { onActions: () => {}, children: [], ...p } }) as Node;
  it("a failed card: radius 12, the stronger red rim, ✕ 1 as plain mono text, output in mono 12/17", () => {
    const tree = draw(FailedCard, { block: block({}), onRunAgain: () => {} });
    expect(flat(tree.props.style)).toMatchObject({ borderRadius: 12, paddingVertical: 10, paddingHorizontal: 12, gap: 5, borderColor: `${stubColors.error}88` });
    const status = findAll(tree, (n) => n.type === "Text" && textOf(n) === "✕ 1")[0]!;
    expect(flat(status.props.style)).toMatchObject({ fontFamily: "monoMedium", fontSize: 11, color: stubColors.error });
    expect(flat(status.props.style).backgroundColor).toBeUndefined();
    const out = findAll(tree, (n) => n.type === "SelectableText")[0]!;
    expect(out.props.text).toBe("Tests: 2 failed, 41 passed");
    expect(flat(out.props.style)).toMatchObject({ fontFamily: "mono", fontSize: 12, color: stubColors.text });
    expect(out.props.lineHeight).toBe(17);
    expect(findAll(tree, (n) => n.type === "Button").map((b) => b.props.label)).toEqual(["Run again", "Copy error"]);
  });
  it("search results show the matching lines only: no Run again, Copy error or Stop", () => {
    expect(findAll(draw(FailedCard, { block: block({}), onRunAgain: () => {}, mark: "error" }), (n) => n.type === "Button")).toHaveLength(0);
    const running = { block: block({ state: "running", exit: null, command: "deploy", output: "a\nDone. notes.txt is the only file.\n" }), notify: false, onBell: () => {}, onStop: () => {} };
    expect(findAll(draw(RunningCard, running), (n) => n.type === "Button")).toHaveLength(2);
    expect(findAll(draw(RunningCard, { ...running, mark: "notes" }), (n) => n.type === "Button")).toHaveLength(0);
  });
  it("a search result shows the line that holds the words when the message does not", () => {
    const { searchedLine } = load("ui/BlockCards.tsx", deps) as { searchedLine: (o: string, q?: string) => unknown };
    const out = "> project1@1.0.0 test\nerror in App.test.tsx\nTests: 2 failed, 41 passed\n";
    expect(searchedLine(out, "ERROR")).toEqual({ text: "error in App.test.tsx", path: null });
    // the message holds it, no search, or no line holds it: the usual message
    expect(searchedLine(out, "passed")).toBeNull();
    expect(searchedLine(out, undefined)).toBeNull();
    expect(searchedLine(out, "nowhere")).toBeNull();
    const tree = draw(FailedCard, { block: block({ output: out }), mark: "error" });
    expect(findAll(tree, (n) => n.type === "SelectableText").map((n) => n.props.text)).toEqual(["error in App.test.tsx"]);
  });
  it("while searching the command line has no context row", () => {
    expect(pane).toMatch(/chips=\{\s*searching !== null \|\| waitingForPane\s*\? \[\]\s*: chipItems\(/);
    // and the opening card stands alone, no skeleton blocks under it
    expect(pane).toContain("{!state && host && !load?.card ? <BlockSkeleton /> : null}");
  });
  it("the running card: one latest line, the spinner 6 pt before its command", () => {
    const tree = draw(RunningCard, { block: block({ state: "running", exit: null, command: "deploy", output: "a\nDone. notes.txt is the only file.\n" }), notify: false, onBell: () => {} });
    expect(findAll(tree, (n) => n.type === "SelectableText").map((n) => n.props.text)).toEqual(["Done. notes.txt is the only file."]);
    expect(flat(tree.props.style).borderColor).toBe(`${stubColors.accent}66`);
    const spinner = findAll(tree, (n) => n.type === "Spinner")[0]!;
    expect(spinner.props.size).toBe(15);
  });
});

describe("the opening card", () => {
  const deps = uiDeps("ios", { "./Button": { Button: "Button" } });
  const { PaneCard } = load("ui/PaneLoading.tsx", { ...deps, "react-native": { ...(deps["react-native"] as object), ActivityIndicator: "ActivityIndicator", Animated: { View: "Animated.View" }, AccessibilityInfo: {}, Easing: {} } }) as { PaneCard: (p: object) => unknown };
  const card = {
    title: "Opening project1",
    hint: null,
    actions: false,
    steps: [
      { tone: "done", mark: "✓", label: "Reached studio-mac on this network", note: "" },
      { tone: "now", mark: "◌", label: "Loading the pane’s blocks…", note: "" },
      { tone: "todo", mark: "○", label: "The screen", note: "" },
    ],
  };
  const draw = (c: object) => render({ type: PaneCard, props: { card: c, onRetry: () => {}, onDetails: vi.fn(), onLeave: () => {}, children: [] } }) as Node;
  it("on the surface with a hairline rim and no shadow; the steps as glyphs; Details while it waits", () => {
    const tree = draw(card);
    const box = findAll(tree, (n) => n.props.accessibilityLabel === "Opening project1")[0]!;
    expect(flat(box.props.style)).toMatchObject({ backgroundColor: stubColors.surface, borderColor: stubColors.hairline, padding: 16, gap: 12 });
    expect(flat(box.props.style).shadowOpacity).toBeUndefined();
    expect(textOf(box)).toBe("Opening project1✓Reached studio-mac on this network◌Loading the pane’s blocks…○The screen");
    expect(findAll(tree, (n) => n.type === "Button").map((b) => [b.props.label, b.props.size])).toEqual([["Details", "small"]]);
    // a card with its own actions keeps them, without a second Details
    expect(findAll(draw({ ...card, actions: true, leave: "Back to Portals" }), (n) => n.type === "Button" && n.props.label === "Details")).toHaveLength(1);
  });
});

describe("the keys", () => {
  const row = src("ui/ControlRow.tsx");
  it("labels in JetBrains Mono Bold 14 on the surface with the #ffffff24 rim; Paste as the word", () => {
    expect(row).toMatch(/key: \{[\s\S]*?borderWidth: 1,\s*borderColor: colors\.borderStrong,/);
    expect(row).toContain("label: { fontFamily: monoBold, fontSize: 14 },");
    expect(src("../modules/rift-keys/ios/RiftPasteButton.swift")).toContain(".labelOnly");
  });
  it("black under the key block of a full-screen program (not the page ground)", () => {
    expect(pane).toContain("!bareScreen && styles.chromeGround");
    expect(pane).toContain("chromeGround: { backgroundColor: colors.chrome },");
  });
  it("sit 10 pt under the field, the completions above it", () => {
    const composer = src("ui/Composer.tsx");
    expect(composer).toContain("keys: { marginTop: 2, zIndex: 2 },");
    expect(composer.indexOf("{completions}")).toBeLessThan(composer.indexOf("<View style={[styles.inputRow, waiting && styles.waiting]}>"));
  });
});
