import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import * as paneView from "../src/paneView";
import * as answer from "../src/answer";
import * as terminal from "../src/terminal";
import * as topology from "../src/topology";
import type { Waiting } from "../src/protocol/types";
import { stubColors, themeStub } from "./helpers/themeStub";
import { findAll, flat, load, react, render, textOf, uiDeps, type Node } from "./helpers/render";

// The New Pairing and New Screen boards (Pair a computer, Paste a pairing link, Pair, Diagnostics,
// the lock, the program screen, the question card folded and open, Tell it why, A change in
// full) and the selection's knobs, loupe and bar, run as the app's own modules: the measures and
// words the planned frames draw. Pixels stay device checks.

const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");
const style = (n: Node | undefined) => flat(n?.props.style);
const byText = (tree: unknown, text: string) => findAll(tree, (n) => n.type === "Text" && textOf(n) === text)[0];
const button = (tree: unknown, label: string) => findAll(tree, (n) => n.type === "Button" && n.props.label === label)[0];
const router = () => ({ push: vi.fn(), back: vi.fn(), replace: vi.fn(), dismissTo: vi.fn(), canGoBack: () => true });
const rn = (extra: object = {}) => ({ ...(uiDeps("ios")["react-native"] as object), useWindowDimensions: () => ({ width: 402, height: 874 }), ...extra });
const pageDeps = (extra: Record<string, unknown>) =>
  uiDeps("ios", {
    "@/ui/Button": { Button: "Button" },
    "@/ui/Icon": { Icon: "Icon" },
    "@/ui/Notice": { ErrorText: "ErrorText", WarningText: "WarningText" },
    "@/ui/Screen": { PageHeader: "PageHeader", SectionTitle: "SectionTitle" },
    "@/ui/theme": themeStub,
    ...extra,
  });

describe("Pair a computer (camera)", () => {
  const scan = (permission: object | null) => {
    const deps = pageDeps({
      "expo-camera": { CameraView: "CameraView", useCameraPermissions: () => [permission, vi.fn()] },
      "expo-router": { useRouter: router },
      "expo-status-bar": { StatusBar: "StatusBar" },
      "react-native": rn({ Linking: { openSettings: vi.fn() } }),
      "@/pairing": { parsePairLink: () => null },
      "@/ui/palette": { DARK_DEFAULT: {} },
    });
    const { default: Scan } = load("app/scan.tsx", deps) as { default: () => unknown };
    return render({ type: Scan, props: { children: [] } });
  };
  const asking = scan({ granted: false, canAskAgain: true });

  it("goes Back with ‹, not ✕", () => {
    const header = findAll(asking, (n) => n.type === "PageHeader")[0]!;
    expect(header.props.title).toBe("Pair a computer");
    expect(header.props.back).toBeTypeOf("function");
  });
  it("draws the 264 pt viewfinder with its four 46 pt accent corners", () => {
    const finder = findAll(asking, (n) => n.props.accessibilityLabel === "Camera view with a frame for the code")[0]!;
    expect(style(finder)).toMatchObject({ width: 264, height: 264, borderRadius: 16, backgroundColor: stubColors.viewfinder });
    const corners = (finder.props.children as unknown[]).flat(3).filter((c) => c && typeof c === "object") as Node[];
    const drawn = findAll(corners, (n) => style(n).borderColor === stubColors.accent);
    expect(drawn).toHaveLength(4);
    for (const c of drawn) expect(style(c)).toMatchObject({ width: 46, height: 46 });
    expect(style(finder).borderWidth).toBeUndefined();
  });
  it("puts the heading on the left and the path under it, without 0.8.0's camera line", () => {
    const heading = byText(asking, "Scan the code on your computer")!;
    expect(style(heading)).toMatchObject({ fontSize: 20, lineHeight: 26, alignSelf: "stretch" });
    expect(style(heading).textAlign).toBeUndefined();
    expect(byText(asking, "Rift › Settings › Remote access › Pair a phone")).toBeDefined();
    expect(textOf(asking)).not.toContain("The camera only reads that code.");
  });
  it("ends with Allow camera full width over the paste link as a text link", () => {
    const allow = button(asking, "Allow camera")!;
    const paste = button(asking, "Paste a pairing link")!;
    expect(allow.props.variant).toBe("primary");
    expect(style(allow)).toMatchObject({ alignSelf: "stretch" });
    expect(paste.props.variant).toBe("text");
    const all = findAll(asking, (n) => n.type === "Button").map((n) => n.props.label);
    expect(all).toEqual(["Allow camera", "Paste a pairing link"]);
  });
  it("with the camera on, keeps only the paste link", () => {
    const on = scan({ granted: true, canAskAgain: true });
    expect(findAll(on, (n) => n.type === "Button").map((n) => n.props.label)).toEqual(["Paste a pairing link"]);
  });
});

describe("Paste a pairing link", () => {
  const deps = pageDeps({
    "expo-router": { useRouter: router },
    "@/pairing": { parsePairLink: () => null },
    "@/ui/Field": { Field: "Field" },
  });
  const { default: Add } = load("app/add.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Add, props: { children: [] } });

  it("offers Continue at full strength while the field is empty", () => {
    expect(button(tree, "Continue")!.props.disabled).toBeFalsy();
  });
  it("sets text, caption and field 14 apart, the caption on the text's edge", () => {
    const scroll = findAll(tree, (n) => n.type === "ScrollView")[0]!;
    expect(flat(scroll.props.contentContainerStyle)).toMatchObject({ gap: 14, paddingTop: 20, paddingHorizontal: 16 });
    const caption = findAll(tree, (n) => n.type === "SectionTitle")[0]!;
    expect(style(caption)).toMatchObject({ marginHorizontal: -4 });
  });
  it("keeps the scan icon on Scan the code instead (pre-0.8.0)", () => {
    expect(button(tree, "Scan the code instead")!.props.icon).toBeTruthy();
  });
});

describe("Pair confirmation", () => {
  const link = { name: "studio-mac", key: "R45tcRB39vYhrDdEmorekey", addresses: [], relay: "wss://relay.example.com", permission: "operate" };
  const deps = pageDeps({
    "expo-router": { useRouter: router, useLocalSearchParams: () => ({ link: "rift-remote://pair?x" }) },
    "react-native": rn({ ActivityIndicator: "ActivityIndicator" }),
    "@/ui/useActionScope": { useActionScope: () => ({ capture: () => () => true, leave: () => true }) },
    "@/pairing": { parsePairLink: () => link },
    "@/protocol/relay": { addressHost: (a: string) => a, relayHost: () => "relay.example.com" },
    "@/runtime": { fleet: { pairedWith: () => false, pair: vi.fn() }, useApp: () => ({ hosts: [], ready: true, loadError: null }) },
    "@/ui/Rows": { ROW_HEIGHT: { regular: 68, compact: 44 } },
  });
  const { default: Pair } = load("app/pair.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Pair, props: { children: [] } });
  const rows = findAll(tree, (n) => typeof n.props.accessibilityLabel === "string" && /^(Computer|Relay|Access|Key): /.test(n.props.accessibilityLabel as string));

  it("draws 68 pt rows, the label 16, the value 15, the key mono 14 at full strength", () => {
    expect(rows.map((r) => r.props.accessibilityLabel)).toEqual(["Computer: studio-mac", "Relay: relay.example.com", "Access: Can run commands", "Key: R45t cRB3 9vYh rDdE"]);
    expect(style(rows[0])).toMatchObject({ minHeight: 68, paddingHorizontal: 16 });
    expect(style(rows[1])).toMatchObject({ minHeight: 69, borderTopWidth: 1 });
    expect(style(byText(rows[0], "Computer"))).toMatchObject({ fontSize: 16, lineHeight: 21, color: stubColors.text });
    expect(style(byText(rows[0], "studio-mac"))).toMatchObject({ fontSize: 15, lineHeight: 20, color: stubColors.textMuted });
    expect(style(byText(rows[3], "R45t cRB3 9vYh rDdE"))).toMatchObject({ fontFamily: "mono", fontSize: 14, color: stubColors.textBright });
  });
  it("says the footnote with a straight apostrophe, 4 in from the card", () => {
    const note = byText(tree, "Check the key matches the one on your computer's screen.")!;
    expect(note).toBeDefined();
    const notes = findAll(tree, (n) => n.type === "View" && (n.props.children as unknown[]).includes(note))[0]!;
    expect(style(notes)).toMatchObject({ paddingTop: 10, paddingHorizontal: 4 });
  });
  it("puts the caption 18 under the intro tile and 8 over the card", () => {
    expect(style(findAll(tree, (n) => n.type === "SectionTitle")[0])).toMatchObject({ paddingTop: 18, paddingBottom: 8 });
    expect(findAll(tree, (n) => n.type === "Icon" && n.props.name === "computer")).toHaveLength(1);
  });
  it("splits Cancel and Pair in equal halves, 8 apart", () => {
    const cancel = button(tree, "Cancel")!;
    const pair = button(tree, "Pair")!;
    expect(style(cancel).flex).toBe(1);
    expect(style(pair).flex).toBe(1);
    const row = findAll(tree, (n) => n.type === "View" && (n.props.children as unknown[]).includes(cancel))[0]!;
    expect(style(row)).toMatchObject({ flexDirection: "row", gap: 8 });
  });
});

describe("Diagnostics", () => {
  const deps = pageDeps({
    "expo-router": { useRouter: router },
    "@/bench": { runCryptoBench: vi.fn() },
    "@/runtime": { randomBytes: vi.fn() },
    "@/ui/Rows": { CardBox: "CardBox", Group: "Group", InfoRow: "InfoRow", Row: "Row" },
  });
  const { default: Diagnostics } = load("app/diagnostics.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Diagnostics, props: { children: [] } });

  it("says what it measures in the board's words, 13/17 under a 16/21 label", () => {
    const lead = byText(tree, "One handshake, then 1 MiB in 65 000-byte chunks")!;
    expect(style(lead)).toMatchObject({ fontSize: 13, lineHeight: 17 });
    expect(style(byText(tree, "Crypto benchmark"))).toMatchObject({ fontSize: 16, lineHeight: 21 });
  });
  it("puts the caption 18 under the header and Run 14 under the card", () => {
    const scroll = findAll(tree, (n) => n.type === "ScrollView")[0]!;
    expect(flat(scroll.props.contentContainerStyle)).toMatchObject({ paddingTop: 18, gap: 14 });
  });
});

describe("App lock", () => {
  const deps = uiDeps("ios", {
    "@/runtime": {
      appLock: { unlock: vi.fn() },
      authenticate: vi.fn(() => Promise.resolve({ ok: false, message: "Cancelled." })),
      biometricSupport: () => Promise.resolve({ label: "Require Face ID" }),
      useLock: () => ({ locked: true, covered: false }),
    },
    "react-native": rn({ Keyboard: { dismiss() {}, addListener: () => ({ remove() {} }) } }),
    "./Splash": { Mark: "Mark" },
    "./Button": { Button: "Button" },
    "./Notice": { ErrorText: "ErrorText" },
  });
  const { LockGate } = load("ui/LockGate.tsx", deps) as { LockGate: (p: object) => unknown };
  const tree = render({ type: LockGate, props: { children: [] } });

  it("keeps the real Rift mark, then the 20/26 title and the 14/20 line, 16 apart", () => {
    expect(findAll(tree, (n) => n.type === "Mark")).toHaveLength(1);
    expect(style(byText(tree, "Rift is locked"))).toMatchObject({ fontSize: 20, lineHeight: 26 });
    expect(style(byText(tree, "Your tabs stay connected."))).toMatchObject({ fontSize: 14, lineHeight: 20 });
    const scroll = findAll(tree, (n) => n.type === "ScrollView")[0]!;
    expect(flat(scroll.props.contentContainerStyle)).toMatchObject({ gap: 16, alignItems: "center" });
    const mark = findAll(tree, (n) => n.type === "View" && (n.props.children as unknown[]).some((c) => (c as Node)?.type === "Mark"))[0]!;
    expect(style(mark).marginBottom).toBeUndefined();
  });
});

describe("Agent question (the New Screen board)", () => {
  const w = {
    kind: "choice",
    prompt: "Make this edit to notes.txt?",
    choices: ["Yes", "Yes, allow all edits", "No"],
    context: ["Update(notes.txt)", " 1  # Notes", " 2 -TODO: ship it", " 2 +Done: shipped 6 Nov"],
    numbered: true,
  } as unknown as Waiting;
  const push = vi.fn();
  const card = (state: "open" | "folded" | "reply" = "open") => {
    const deps = uiDeps("ios", {
      "@/ui/useActionScope": { useActionScope: () => ({ capture: () => () => true }) },
      "@/answer": answer,
      "@/lock": { needsAuthBeforeRun: () => false },
      "@/paneView": paneView,
      "@/runtime": { authenticate: vi.fn(), fleet: { answer: vi.fn(), sendKeys: vi.fn() }, useApp: () => ({ settings: {} }) },
      "./useNow": { useNow: () => 0 },
      "./PromptText": { PromptText: "PromptText" },
      "./Button": { Button: "Button" },
      "expo-router": { useRouter: () => ({ ...router(), push }) },
      "react-native": rn({ AccessibilityInfo: {}, Keyboard: { dismiss() {} } }),
    });
    const edit = (s: string) =>
      state === "folded"
        ? s.replace("const [min, setMin] = useState(false);", "const [min, setMin] = useState(true);")
        : state === "reply"
          ? s.replace("const [replying, setReplying] = useState(false);", "const [replying, setReplying] = useState(true);")
          : s;
    const { AskCard } = load("ui/AskCard.tsx", deps, edit) as { AskCard: (p: object) => unknown };
    return render({ type: AskCard, props: { hostKey: "A", paneId: 31, waiting: w, program: "claude", where: "project4", bottomInset: 20, onSay: vi.fn(), children: [] } });
  };
  const open = card();
  const root = (tree: unknown) => (Array.isArray(tree) ? tree[0] : tree) as Node;

  it("has a neutral top edge, the board's height, the standard header", () => {
    expect(style(root(open))).toMatchObject({ borderTopColor: stubColors.borderStrong, borderTopLeftRadius: 20, minHeight: 598, paddingBottom: 28 });
    expect(textOf(open)).toContain("Claude Code asks");
  });
  it("asks in 19/25, 12 apart from the diff and the answers", () => {
    const prompt = findAll(open, (n) => n.type === "PromptText")[0]!;
    expect(style(prompt)).toMatchObject({ fontSize: 19, lineHeight: 25 });
  });
  it("heads the diff with the file in mono 12, +N green, −N red, no file icon", () => {
    const file = byText(open, "notes.txt")!;
    expect(style(file)).toMatchObject({ fontFamily: "mono", fontSize: 12, color: stubColors.text });
    expect(style(byText(open, "+1"))).toMatchObject({ color: stubColors.success });
    expect(style(byText(open, "−1"))).toMatchObject({ color: stubColors.error });
    expect(findAll(open, (n) => n.type === "Icon" && n.props.name === "file")).toHaveLength(0);
    const box = findAll(open, (n) => style(n).backgroundColor === stubColors.bg && style(n).borderColor === stubColors.hairline)[0]!;
    expect(style(box)).toMatchObject({ borderRadius: 10, borderWidth: 1 });
  });
  it("draws the changed lines as 30 pt rows on their tints, the number in an 18 pt column", () => {
    const removed = byText(open, "− TODO: ship it")!;
    const added = byText(open, "+ Done: shipped 6 Nov")!;
    expect(style(removed)).toMatchObject({ fontFamily: "mono", fontSize: 12, lineHeight: 18 });
    const rowOf = (t: Node) => findAll(open, (n) => n.type === "View" && (n.props.children as unknown[]).includes(t))[0]!;
    expect(style(rowOf(removed))).toMatchObject({ paddingVertical: 6, backgroundColor: stubColors.tints.answerRemoved });
    expect(style(rowOf(added))).toMatchObject({ paddingVertical: 6, backgroundColor: stubColors.tints.answerAdded });
    const num = findAll(rowOf(removed), (n) => n.type === "Text" && textOf(n) === "2")[0]!;
    expect(style(num)).toMatchObject({ width: 18 });
    // the kept line waits for A change in full
    expect(textOf(open)).not.toContain("# Notes");
  });
  it("opens A change in full from Full diff, a page of its own", () => {
    const full = findAll(open, (n) => n.props.accessibilityLabel === "Full diff")[0]!;
    expect(textOf(full)).toBe("Full diff");
    expect(textOf(open)).not.toContain("Less");
    (full.props.onPress as () => void)();
    expect(push).toHaveBeenCalledWith({ pathname: "/change", params: { key: "A", pane: "31", program: "claude" } });
    expect(src("app/_layout.tsx")).toContain('<Stack.Screen name="change" options={{ headerShown: false');
  });
  it("puts Yes full width over the other answers in equal halves", () => {
    expect(button(open, "Yes")!.props.variant).toBe("primary");
    const others = ["Yes, always", "No, explain"].map((l) => button(open, l)!);
    for (const o of others) expect(style(o)).toMatchObject({ flexGrow: 1, flexBasis: "45%" });
  });
  it("keeps the Keys row (pre-0.8.0): 30 pt keys with 8 pt corners in mono 12", () => {
    const esc = findAll(open, (n) => n.props.accessibilityLabel === "Escape")[0]!;
    expect(style(esc)).toMatchObject({ height: 30, borderRadius: 8, backgroundColor: stubColors.surface, borderColor: stubColors.borderStrong });
    expect(style(byText(esc, "Esc"))).toMatchObject({ fontFamily: "monoMedium", fontSize: 12 });
  });
  it("folds to a 48 pt bar, radius 12, in a black strip under a hairline", () => {
    const folded = root(card("folded"));
    expect(style(folded)).toMatchObject({ backgroundColor: stubColors.chrome, borderTopWidth: 1, borderTopColor: stubColors.hairline, paddingTop: 10, paddingBottom: 28 });
    const bar = findAll(folded, (n) => n.props.accessibilityHint === "Opens the question")[0]!;
    expect(style(bar)).toMatchObject({ minHeight: 48, borderRadius: 12 });
    expect(style(byText(bar, "Answer"))).toMatchObject({ color: stubColors.warning });
  });
  it("Tell it why: a 120 pt reply box with the accent rim and a full-width Send", () => {
    const reply = card("reply");
    const box = findAll(reply, (n) => n.type === "TextInput")[0]!;
    expect(box.props.multiline).toBe(true);
    expect(style(box)).toMatchObject({ height: 120, borderColor: stubColors.accent, fontSize: 16, lineHeight: 22 });
    const send = button(reply, "Send")!;
    expect(send.props.variant).toBe("primary");
    expect(send.props.size).toBeUndefined();
    expect(style(root(reply))).toMatchObject({ borderTopColor: stubColors.borderStrong });
  });
});

describe("A change in full", () => {
  const w = { kind: "choice", prompt: "Make this edit?", choices: ["Yes", "No"], numbered: true, context: ["Update(notes.txt)", " 1  # Notes", " 2 -TODO: ship it", " 2 +Done: shipped 6 Nov", " 3 +", " 4  ## Ideas"] };
  const host = { record: { key: "A" }, windows: [{ tabs: [{ panes: [{ id: 31, waiting: w }] }] }] };
  const deps = pageDeps({
    "expo-router": { useRouter: router, useLocalSearchParams: () => ({ key: "A", pane: "31", program: "claude" }) },
    "@/paneView": paneView,
    "@/runtime": { useApp: () => ({ hosts: [host] }) },
    "@/topology": topology,
  });
  const { default: Change } = load("app/change.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Change, props: { children: [] } });

  it("is a page named for the file, the counts and who asks under it", () => {
    const header = findAll(tree, (n) => n.type === "PageHeader")[0]!;
    expect(header.props).toMatchObject({ title: "notes.txt", subtitle: "+2 −1 · from Claude Code" });
  });
  it("draws the hunk header strip, then every line edge to edge, kept ones too", () => {
    const lines = findAll(tree, (n) => n.type === "Text").map(textOf);
    expect(lines).toEqual(["@@ -1,3 +1,4 @@", "# Notes", "− TODO: ship it", "+ Done: shipped 6 Nov", "+ ", "## Ideas"]);
    expect(style(byText(tree, "@@ -1,3 +1,4 @@"))).toMatchObject({ color: stubColors.textMuted, paddingHorizontal: 14, lineHeight: 20 });
    expect(style(byText(tree, "− TODO: ship it"))).toMatchObject({ backgroundColor: `${stubColors.error.slice(0, 7)}26` });
    expect(style(byText(tree, "+ Done: shipped 6 Nov"))).toMatchObject({ backgroundColor: `${stubColors.success.slice(0, 7)}1f` });
    expect(style(byText(tree, "# Notes")).backgroundColor).toBeUndefined();
  });
});

describe("the change's rows and hunk header", () => {
  const ctx = (lines: string[]) => paneView.askContext(lines);
  it("writes a changed line with its sign and a space, a kept one as it is", () => {
    expect(paneView.diffRowText({ n: 2, sign: "-", text: "TODO", kind: "removed" })).toBe("− TODO");
    expect(paneView.diffRowText({ n: 2, sign: "+", text: "Done", kind: "added" })).toBe("+ Done");
    expect(paneView.diffRowText({ n: 1, sign: " ", text: "# Notes", kind: "kept" })).toBe("# Notes");
  });
  it("takes the agent's own hunk header, else makes one from the rows, else none", () => {
    expect(paneView.hunkHeader(ctx(["@@ -10,3 +10,4 @@", "+a"]))).toBe("@@ -10,3 +10,4 @@");
    expect(paneView.hunkHeader(ctx([" 7  a", " 8 -b", " 8 +c", " 9 +d"]))).toBe("@@ -7,2 +7,3 @@");
    expect(paneView.hunkHeader(ctx(["+a", "-b"]))).toBeNull();
  });
});

describe("the program screen and its selection (the New Screen and selection boards)", () => {
  const view = src("ui/TerminalView.tsx");
  it("sets the rows 10 pt in from the left, 15 pt lines at the board's 10.5 and 11 pt", () => {
    expect(view).toContain("const PADDING_X = 10;");
    expect(Math.round(11 * terminal.LINE_RATIO)).toBe(15);
    expect(Math.round(10.5 * terminal.LINE_RATIO)).toBe(15);
  });
  it("puts an 18 pt accent knob with a dark rim on each end's corner, no stems", () => {
    const deps: Record<string, unknown> = {
      react: { ...react, memo: (c: unknown) => c },
      "expo-router": {},
      "react-native": { ...(uiDeps("ios")["react-native"] as object) },
      "@/runtime": {}, "@/scroll": {}, "@/terminal": terminal, "@/paneView": paneView, "@/selection": {}, "@/haptics": {},
      "@/keyboard": {}, "@/lock": {}, "@/pending": {}, "@/fleet": {}, "@/protocol/types": {}, "@/controlKeys": {},
      "./useActionScope": {}, "./useOpenShell": {}, "./Icon": {}, "./SelectionBar": {}, "./ControlRow": {}, "./PaneLoading": {}, "./Notice": {},
      "./theme": themeStub, "../../modules/rift-keys": {}, "expo-clipboard": {},
    };
    const { handleSpots } = load("ui/TerminalView.tsx", deps) as { handleSpots: (a: object, b: object, cell: number, row: number) => { start: object; end: object } };
    // rows 10 to 11, columns 0 to 33, 6.6 pt cells, 15 pt rows: the start's top left, the end's bottom right
    expect(handleSpots({ row: 11, col: 33 }, { row: 10, col: 0 }, 6.6, 15)).toMatchObject({ start: { x: 0, y: 150 }, end: { x: 34 * 6.6, y: 180 } });
    expect(view).toContain("knob: { position: \"absolute\", width: KNOB, height: KNOB, borderRadius: KNOB / 2, backgroundColor: colors.accent, borderWidth: 2, borderColor: colors.bg },");
    expect(view).toContain("const KNOB = 18;");
    expect(view).not.toContain("styles.stem");
  });
  it("draws the loupe on the surface, 150 × 46, a #ffffff33 rim, the row in mono 22, a caret in the text colour", () => {
    expect(view).toContain("const LOUPE_W = 150;");
    expect(view).toContain("const LOUPE_H = 46;");
    expect(view).toContain("const LOUPE_FONT = 22;");
    expect(view).toContain("loupe: { position: \"absolute\", width: LOUPE_W, height: LOUPE_H, overflow: \"hidden\", borderRadius: LOUPE_H / 2, borderWidth: 1, borderColor: tint(colors.textBright, 0x33), backgroundColor: colors.surface },");
    expect(view).toContain("loupeCaret: { position: \"absolute\", width: 2, borderRadius: 1, backgroundColor: colors.textBright },");
    expect(view).not.toContain("shadowOpacity: 0.8");
  });
  it("gives Android's block handles the accent, apart from the selection's ground", () => {
    const field = src("ui/SelectableText.tsx");
    expect(field).toContain("selectionColor={colors.selection}");
    expect(field).toContain("selectionHandleColor={colors.accent}");
  });
});
