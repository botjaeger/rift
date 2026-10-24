import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as scroll from "../src/scroll";
import * as terminal from "../src/terminal";
import * as paneView from "../src/paneView";
import * as controlKeys from "../src/controlKeys";

type Node = { type: unknown; props: Record<string, unknown> & { children: unknown[] } };
type Touch = (event: { nativeEvent: { touches: { pageX: number; pageY: number }[] } }) => void;

// Execute the real TerminalView callbacks and scroll engine; native layout, navigation and
// Fleet I/O are leaves. Device walks separately exercise responder ownership and rendering.
function harness(typing = false) {
  const effects: (() => void | (() => void))[] = [];
  const focusEffects: (() => void | (() => void))[] = [];
  const appListeners = new Set<(state: string) => void>();
  const send = vi.fn(() => Promise.resolve({ ok: true } as { ok: boolean; code?: string }));
  const react = {
    createElement: (type: unknown, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }),
    memo: (fn: unknown) => fn,
    useRef: (initial: unknown) => ({ current: initial }),
    useState: (initial: unknown) => [typeof initial === "function" ? initial() : initial, () => {}],
    useCallback: (fn: unknown) => fn,
    useEffect: (effect: () => void | (() => void)) => effects.push(effect),
  };
  const dependencies: Record<string, unknown> = {
    react,
    "expo-router": { useFocusEffect: (effect: () => void | (() => void)) => focusEffects.push(effect) },
    "react-native": {
      AppState: { addEventListener: (_: string, fn: (state: string) => void) => {
        appListeners.add(fn); return { remove: () => appListeners.delete(fn) };
      } },
      StyleSheet: { create: (s: unknown) => s },
      View: "View", ScrollView: "ScrollView", Pressable: "Pressable", Text: "Text",
    },
    "@/runtime": { useApp: () => ({ settings: {} }), fleet: { sendScroll: send } },
    "@/scroll": scroll,
    "@/terminal": terminal,
    "@/paneView": paneView,
    "@/keyboard": { terminalInput: () => ({ flush() {} }) },
    "@/lock": { SubmitAuthHold: class { end() {} } },
    "@/upload": { ImagePaste: class {} },
    "./useActionScope": { useActionScope: () => ({ capture: () => () => true }) },
    "./useOpenShell": { useOpenShell: () => () => {} },
    "./ControlRow": { NO_STICKY: {}, ControlRow: "ControlRow" },
    "./theme": { colors: {}, ui: () => "ui" },
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
  const tree = module.exports.TerminalView({ hostKey: "fixture", paneId: 12, screen: null, operate: true, canType: typing, bare: !typing, online: true, running: true, canScroll: true });
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
  const touch = (name: string, y?: number, x = 100) => (node.props[name] as Touch)({ nativeEvent: { touches: y === undefined ? [] : [{ pageX: x, pageY: y }] } });
  return {
    send, touch, tree, focus,
    press: () => (find(tree, (n) => n.type === "Pressable" && n.props.accessible === false)!.props.onPress as () => void)(),
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
