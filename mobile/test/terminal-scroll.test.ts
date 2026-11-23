import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as scroll from "../src/scroll";
import * as stitchModule from "../src/stitch";
import * as historyModule from "../src/history";
import * as terminal from "../src/terminal";
import * as paneView from "../src/paneView";
import * as controlKeys from "../src/controlKeys";
import * as selection from "../src/selection";
import { themeStub } from "./helpers/themeStub";

type Node = { type: unknown; props: Record<string, unknown> & { children: unknown[] } };
type Touch = (event: { nativeEvent: { touches: { pageX: number; pageY: number }[]; timestamp: number } }) => void;

// Execute the real TerminalView callbacks and scroll engine; native layout, navigation and
// Fleet I/O are leaves. Device walks separately exercise responder ownership and rendering.
function harness(typing = false, props: Record<string, unknown> = {}) {
  const effects: (() => void | (() => void))[] = [];
  const focusEffects: (() => void | (() => void))[] = [];
  const appListeners = new Set<(state: string) => void>();
  const send = vi.fn(() => Promise.resolve({ ok: true } as { ok: boolean; code?: string }));
  const history = vi.fn(() => new Promise(() => {}));
  const pageAhead = vi.fn((..._a: unknown[]) => Promise.resolve({ ok: true } as { ok: boolean; code?: string; message?: string }));
  const react = {
    createElement: (type: unknown, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }),
    memo: (fn: unknown) => fn,
    useRef: (initial: unknown) => ({ current: initial }),
    useState: (initial: unknown) => [typeof initial === "function" ? initial() : initial, () => {}],
    useCallback: (fn: unknown) => fn,
    useMemo: (fn: () => unknown) => fn(),
    useEffect: (effect: () => void | (() => void)) => effects.push(effect),
    useLayoutEffect: (effect: () => void | (() => void)) => effects.push(effect),
  };
  const dependencies: Record<string, unknown> = {
    react,
    "expo-router": { useFocusEffect: (effect: () => void | (() => void)) => focusEffects.push(effect) },
    "react-native": {
      AppState: { addEventListener: (_: string, fn: (state: string) => void) => {
        appListeners.add(fn); return { remove: () => appListeners.delete(fn) };
      } },
      AccessibilityInfo: { isScreenReaderEnabled: () => Promise.resolve(false), addEventListener: () => ({ remove() {} }), announceForAccessibility() {} },
      Keyboard: { addListener: () => ({ remove() {} }) },
      Platform: { OS: "ios" },
      StyleSheet: { create: (s: unknown) => s, absoluteFill: {} },
      View: "View", ScrollView: "ScrollView", Pressable: "Pressable", Text: "Text",
    },
    "@/runtime": { useApp: () => ({ settings: { controlKeys: [] }, hosts: [] }), fleet: { sendScroll: send, history, pageAhead, pageSeek: vi.fn(), onPage: () => () => {} } },
    "@/scroll": scroll,
    "@/links": { openLink: () => {} },
    "@/stitch": stitchModule,
    "@/history": historyModule,
    "@/terminal": terminal,
    "@/paneView": paneView,
    "@/selection": selection,
    "@/haptics": { selectStart() {}, selectStep() {}, copied() {}, dragLift() {} },
    "@/keyboard": { terminalInput: () => ({ flush() {} }) },
    "@/lock": { SubmitAuthHold: class { end() {} } },
    "@/pending": {}, "@/fleet": { draftKey: (h: string, p: number) => `${h}:${p}` },
    "./useActionScope": { useActionScope: () => ({ capture: () => () => true }) },
    "./useOpenShell": { useOpenShell: () => () => {} },
    "./Icon": { Icon: "Icon" },
    "./SelectionBar": { SelectionBar: "SelectionBar" },
    "./ControlRow": { NO_STICKY: {}, ControlRow: "ControlRow" },
    "./theme": themeStub,
    "./palette": { lift: (fg: string) => fg },
    "../../modules/rift-keys": { KeyCaptureView: "KeyCaptureView" },
    "@/protocol/types": {}, "@/controlKeys": controlKeys, "./PaneLoading": {}, "./Notice": {}, "expo-clipboard": {},
  };
  const source = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 } }).outputText;
  const module = { exports: {} as { TerminalView: (props: object) => Node } };
  runInNewContext(compiled, { module, exports: module.exports, React: react, Date, setTimeout, clearTimeout, setInterval, clearInterval, require: (name: string) => {
    if (!(name in dependencies)) throw new Error(`Unstubbed import ${name}`);
    return dependencies[name];
  } });
  const tree = module.exports.TerminalView({ hostKey: "fixture", paneId: 12, screen: null, operate: true, canType: typing, bare: !typing, online: true, running: true, canScroll: true, ...props });
  function find(value: unknown, matches: (node: Node) => boolean): Node | undefined {
    if (Array.isArray(value)) return value.map((v) => find(v, matches)).find(Boolean);
    if (!value || typeof value !== "object") return;
    const node = value as Node;
    return matches(node) ? node : find(node.props?.children, matches);
  }
  const node = find(tree, (n) => !!n.props?.onTouchStart)!;
  const focus = vi.fn();
  const capture = find(tree, (n) => n.type === "KeyCaptureView");
  if (capture) (capture.props.ref as { current: unknown }).current = { focus };
  const cleanups = effects.map((effect) => effect()).filter((f): f is () => void => !!f);
  const focusCleanups = focusEffects.map((effect) => effect()).filter((f): f is () => void => !!f);
  // the event's own time, as React Native stamps every touch event
  const touch = (name: string, y?: number, x = 100) => (node.props[name] as Touch)({ nativeEvent: { touches: y === undefined ? [] : [{ pageX: x, pageY: y }], timestamp: Date.now() } });
  return {
    send, history, pageAhead, touch, tree, focus, find,
    // a press event as React Native gives one: where the finger lifted (no link on this test's screen)
    press: () => (find(tree, (n) => n.type === "Pressable" && n.props.accessible === false)!.props.onPress as (e: unknown) => void)({ nativeEvent: { pageX: 0, pageY: 0 } }),
    background: () => appListeners.forEach((fn) => fn("background")),
    blur: () => focusCleanups.forEach((fn) => fn()),
    unmount: () => { focusCleanups.forEach((fn) => fn()); cleanups.forEach((fn) => fn()); },
  };
}

beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(1000); });
afterEach(() => vi.useRealTimers());

describe("terminal touch scrolling", () => {
  it.each([false, true])("the first vertical movement scrolls immediately (keyboard=%s)", (typing) => {
    const h = harness(typing);
    h.touch("onTouchStart", 100);
    vi.advanceTimersByTime(20);
    h.touch("onTouchMove", 120);
    expect(h.send).toHaveBeenCalledWith("fixture", 12, 1);
    h.unmount();
  });

  it("taps and horizontal drags do not scroll the program", () => {
    const h = harness();
    h.touch("onTouchStart", 100);
    h.touch("onTouchMove", 105);
    h.touch("onTouchEnd");
    h.touch("onTouchStart", 100);
    h.touch("onTouchMove", 105, 140);
    h.touch("onTouchMove", 125, 160);
    h.touch("onTouchEnd");
    vi.advanceTimersByTime(500);
    expect(h.send).not.toHaveBeenCalled();
    h.unmount();
  });

  it.each(["cancel", "background", "blur"])("%s stops queued motion and ignores its late rate refusal", async (stop) => {
    const h = harness();
    let refuse!: (value: { ok: boolean; code: string }) => void;
    h.send.mockImplementationOnce(() => new Promise((resolve) => { refuse = resolve; }));
    h.touch("onTouchStart", 100);
    vi.advanceTimersByTime(20);
    h.touch("onTouchMove", 132);
    vi.advanceTimersByTime(5);
    h.touch("onTouchMove", 164);
    if (stop === "cancel") h.touch("onTouchCancel");
    else if (stop === "background") h.background();
    else h.blur();
    const before = h.send.mock.calls.length;
    refuse({ ok: false, code: "rate" });
    await Promise.resolve();
    vi.advanceTimersByTime(2000);
    expect(h.send.mock.calls.length).toBe(before);
    h.unmount();
  });

  it("a normal flick continues, and touching it stops further motion", () => {
    const h = harness();
    h.touch("onTouchStart", 100);
    for (const y of [132, 164, 196]) { vi.advanceTimersByTime(20); h.touch("onTouchMove", y); }
    h.touch("onTouchEnd");
    const lifted = h.send.mock.calls.length;
    vi.advanceTimersByTime(100);
    expect(h.send.mock.calls.length).toBeGreaterThan(lifted);
    h.touch("onTouchStart", 196);
    const caught = h.send.mock.calls.length;
    vi.advanceTimersByTime(2000);
    expect(h.send.mock.calls.length).toBe(caught);
    h.unmount();
  });

  it("catching a flick ignores its late rate refusal", async () => {
    const h = harness();
    h.touch("onTouchStart", 100);
    for (const y of [132, 164, 196]) { vi.advanceTimersByTime(20); h.touch("onTouchMove", y); }
    let refuse!: (value: { ok: boolean; code: string }) => void;
    h.send.mockImplementationOnce(() => new Promise((resolve) => { refuse = resolve; }));
    h.touch("onTouchEnd");
    vi.advanceTimersByTime(100);
    h.touch("onTouchStart", 196);
    const caught = h.send.mock.calls.length;
    refuse({ ok: false, code: "rate" });
    await Promise.resolve();
    vi.advanceTimersByTime(2000);
    expect(h.send.mock.calls.length).toBe(caught);
    h.unmount();
  });

  it("touching again after a plain drag keeps its distance, a late rate refusal included", async () => {
    const h = harness();
    let refuse!: (value: { ok: boolean; code: string }) => void;
    h.send.mockImplementationOnce(() => new Promise((resolve) => { refuse = resolve; }));
    h.touch("onTouchStart", 100);
    vi.advanceTimersByTime(20);
    h.touch("onTouchMove", 120);
    // held still before lifting: no flick
    vi.advanceTimersByTime(150);
    h.touch("onTouchEnd");
    h.touch("onTouchStart", 120);
    refuse({ ok: false, code: "rate" });
    await Promise.resolve();
    vi.advanceTimersByTime(2000);
    expect(h.send.mock.calls).toEqual([["fixture", 12, 1], ["fixture", 12, 1]]);
    h.unmount();
  });

  it("cancelling a fast drag never turns it into a flick", () => {
    const h = harness();
    h.touch("onTouchStart", 100);
    for (const y of [132, 164, 196, 228]) { vi.advanceTimersByTime(20); h.touch("onTouchMove", y); }
    h.touch("onTouchCancel");
    const cancelled = h.send.mock.calls.length;
    vi.advanceTimersByTime(2000);
    expect(h.send.mock.calls.length).toBe(cancelled);
    h.unmount();
  });

  it("a short drag or catching momentum does not pop open the keyboard; a plain tap still does", () => {
    const h = harness(true);
    h.touch("onTouchStart", 100);
    vi.advanceTimersByTime(20);
    h.touch("onTouchMove", 120);
    vi.advanceTimersByTime(140);
    h.touch("onTouchEnd");
    h.press();
    expect(h.focus).not.toHaveBeenCalled();
    h.touch("onTouchStart", 100);
    h.touch("onTouchEnd");
    h.press();
    expect(h.focus).toHaveBeenCalledTimes(1);
    // A new drag and real release leave momentum to catch.
    h.touch("onTouchStart", 100);
    for (const y of [132, 164, 196]) { vi.advanceTimersByTime(20); h.touch("onTouchMove", y); }
    h.touch("onTouchEnd");
    vi.advanceTimersByTime(40);
    h.touch("onTouchStart", 196);
    h.touch("onTouchEnd");
    h.press();
    expect(h.focus).toHaveBeenCalledTimes(1);
    h.unmount();
  });
});

describe("following the end", () => {
  const screen = {
    cols: 20, rows: 4, alt: false, cursor: null, fg: null, bg: null, rules: new Map(), stale: false, ageMs: null,
    lines: [0, 1, 2, 3].map((n) => [[String(n), null, null, 0]]),
  };
  const ended = (y: number) => ({ nativeEvent: { contentOffset: { x: 0, y }, contentSize: { width: 300, height: 1000 }, layoutMeasurement: { width: 300, height: 300 } } });

  it("the view's own scroll ending above the end (iOS's momentum end, output grew meanwhile) keeps it following; a finger's does not", () => {
    const h = harness(false, { screen, canScroll: false });
    const view = h.find(h.tree, (n) => n.type === "ScrollView")!;
    const scroller = { scrollTo: vi.fn(), scrollToEnd: vi.fn() };
    (view.props.ref as { current: unknown }).current = scroller;
    const moved = () => scroller.scrollTo.mock.calls.length + scroller.scrollToEnd.mock.calls.length;
    const call = (name: string, ...args: unknown[]) => (view.props[name] as (...a: unknown[]) => void)(...args);
    // a new tab: the program's output arrives after the view mounted, and the view scrolls itself to it
    call("onContentSizeChange", 300, 1000);
    expect(moved()).toBeGreaterThan(0);
    call("onMomentumScrollEnd", ended(400));
    const before = moved();
    call("onContentSizeChange", 300, 1100);
    expect(moved()).toBeGreaterThan(before);
    // the user drags up and lets go there: new output leaves the view where they put it
    call("onScrollBeginDrag");
    call("onScrollEndDrag", ended(200));
    call("onMomentumScrollEnd", ended(150));
    vi.advanceTimersByTime(400);
    const read = moved();
    call("onContentSizeChange", 300, 1200);
    vi.advanceTimersByTime(400);
    expect(moved()).toBe(read);
    h.unmount();
  });
});

describe("the lines above a primary screen (feature `history`)", () => {
  // Rift's view of a pane, its first row line 500 of numbering 7
  const styled = (alt = false) => ({
    cols: 20, rows: 4, alt, cursor: [3, 2], fg: null, bg: null, rules: new Map(), stale: false, ageMs: null,
    lines: [500, 501, 502, 503].map((n) => [[String(n), null, null, 0]]),
    scrollback: { epoch: 7, first: 0, top: 500 },
  });
  const drag = (h: ReturnType<typeof harness>) => {
    h.touch("onTouchStart", 100);
    for (const y of [132, 164, 196]) { vi.advanceTimersByTime(20); h.touch("onTouchMove", y); }
    h.touch("onTouchEnd");
    vi.advanceTimersByTime(500);
  };

  it("are one tall screen the phone scrolls itself: a drag sends the computer nothing", () => {
    const h = harness(false, { screen: styled(), canHistory: true });
    const grid = h.find(h.tree, (n) => n.props?.lightAbove !== undefined)!;
    const shown = grid.props.screen as { lines: unknown[]; cursor: number[] };
    // 500 slots above the screen's 4 rows; the cursor moves down with them
    expect(shown.lines).toHaveLength(504);
    expect(shown.cursor).toEqual([503, 2]);
    // the 500 rows above are history (drawn light, in chunks), the rows near the end drawn first
    expect([grid.props.lightAbove, grid.props.historyOrigin, grid.props.mountFrom]).toEqual([500, 0, 492]);
    drag(h);
    expect(h.send).not.toHaveBeenCalled();
    h.unmount();
  });

  it("asks for the page of lines right above the screen", () => {
    const h = harness(false, { screen: styled(), canHistory: true });
    expect(h.history).toHaveBeenCalledWith("fixture", 12, 7, 300, 500);
    h.unmount();
  });

  it("the alternate screen on a computer that stitches: a drag asks for the page, the wheel stays still", () => {
    const h = harness(false, { screen: styled(true), canHistory: true, canStitch: true });
    drag(h);
    // finger down: the rows above, one request (the computer takes one at a time)
    expect(h.pageAhead).toHaveBeenCalledTimes(1);
    expect(h.pageAhead).toHaveBeenCalledWith("fixture", 12, 0, "up", 20);
    expect(h.send).not.toHaveBeenCalled();
    h.unmount();
  });

  it("finger up asks for the rows below; a refused request may be asked again", async () => {
    const h = harness(false, { screen: styled(true), canStitch: true });
    h.pageAhead.mockImplementationOnce(() => Promise.resolve({ ok: false, code: "busy", message: "" }));
    h.touch("onTouchStart", 300);
    for (const y of [260, 220]) { vi.advanceTimersByTime(20); h.touch("onTouchMove", y); }
    expect(h.pageAhead).toHaveBeenLastCalledWith("fixture", 12, 0, "down", 20);
    await Promise.resolve();
    await Promise.resolve();
    vi.advanceTimersByTime(20);
    h.touch("onTouchMove", 180);
    expect(h.pageAhead).toHaveBeenCalledTimes(2);
    h.unmount();
  });

  it("a first page that does not come leaves the drag to the wheel: a drag always moves something", () => {
    const h = harness(false, { screen: styled(true), canStitch: true });
    h.pageAhead.mockImplementation(() => new Promise(() => {}));
    drag(h);
    // on its way: the wheel is left alone (its steps would break the stitching)
    vi.advanceTimersByTime(2000);
    drag(h);
    vi.advanceTimersByTime(200);
    expect(h.send).not.toHaveBeenCalled();
    // overdue: the program is scrolled as one that cannot be stitched
    vi.advanceTimersByTime(1000);
    drag(h);
    vi.advanceTimersByTime(200);
    expect(h.send).toHaveBeenCalled();
    expect(h.pageAhead).toHaveBeenCalledTimes(1);
    h.unmount();
  });

  it("one request at a time; one never answered (the connection dropped) counts as lost after 8 s", () => {
    const h = harness(false, { screen: styled(true), canStitch: true });
    h.pageAhead.mockImplementation(() => new Promise(() => {}));
    drag(h);
    drag(h);
    expect(h.pageAhead).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(8000);
    drag(h);
    expect(h.pageAhead).toHaveBeenCalledTimes(2);
    expect(h.send).not.toHaveBeenCalled();
    h.unmount();
  });

  it("a primary screen, or a view-only device, never asks for a page", () => {
    for (const props of [{ screen: styled(false), canHistory: false, canStitch: true }, { screen: styled(true), canStitch: false }]) {
      const h = harness(false, props);
      drag(h);
      expect(h.pageAhead).not.toHaveBeenCalled();
      expect(h.send).toHaveBeenCalled();
      h.unmount();
    }
  });

  it.each([
    ["an older computer", { screen: styled(), canHistory: false }],
    ["the alternate screen", { screen: styled(true), canHistory: true }],
  ])("%s: a drag scrolls the computer's pane as before", (_name, props) => {
    const h = harness(false, props);
    drag(h);
    expect(h.send).toHaveBeenCalled();
    expect(h.history).not.toHaveBeenCalled();
    h.unmount();
  });
});
