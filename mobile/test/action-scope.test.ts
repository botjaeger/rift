import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { themeStub } from "./helpers/themeStub";

type Element = { type: string; props: Record<string, unknown> & { children: unknown[] } };
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
}

// Actual screen/hook callbacks; only native leaves, router, auth and Fleet I/O are fixtures.
// This proves lifecycle behavior, not layout, native biometrics or gateway authorization.
function harness() {
  const focuses: (() => void | (() => void))[] = [];
  let cleanups: (() => void)[] = [];
  const router = { back: vi.fn(), replace: vi.fn(), push: vi.fn(), dismiss: vi.fn(), dismissTo: vi.fn(), canGoBack: () => true };
  const auth = deferred<{ ok: true }>();
  const ack = deferred<{ ok: true; pane: number; host?: { key: string } }>();
  const fleet = { state: { hosts: [{ record: { key: "host" }, windows: [] }] }, updateSettings: vi.fn(() => Promise.resolve(true)), answer: vi.fn(() => ack.promise), openShell: vi.fn(() => ack.promise), closeShell: vi.fn(() => ack.promise), pair: vi.fn(() => ({ result: ack.promise, cancel: vi.fn() })), pairedWith: () => false };
  const waiting = { kind: "choice", choices: ["Yes", "No"], prompt: "Continue?", context: [], numbered: false };
  const host = { record: { key: "host", name: "Fixture" }, conn: { status: "open" }, windows: [] };
  const react = {
    createElement: (type: string, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }),
    useEffect: (effect: () => (() => void) | undefined) => { const cleanup = effect(); if (cleanup) cleanups.push(cleanup); },
    useCallback: (fn: unknown) => fn,
    useRef: (initial: unknown) => ({ current: initial }),
    useState: (initial: unknown) => [typeof initial === "function" ? initial() : initial, () => {}],
  };
  const dependencies: Record<string, unknown> = {
    "expo-router": { Stack: { Screen: "StackScreen" }, useRouter: () => router, useLocalSearchParams: () => ({ key: "host", pane: "12", link: "fixture" }), useFocusEffect: (effect: () => void | (() => void)) => { focuses.push(effect); const cleanup = effect(); if (cleanup) cleanups.push(cleanup); } },
    "react-native": { Platform: { OS: "web" }, AccessibilityInfo: { announceForAccessibility() {} }, Keyboard: { dismiss() {} }, StyleSheet: { create: (s: unknown) => s }, Text: "Text", View: "View", Pressable: "Pressable", ScrollView: "ScrollView", TextInput: "TextInput", KeyboardAvoidingView: "KeyboardAvoidingView" },
    "react-native-safe-area-context": { useSafeAreaInsets: () => ({ bottom: 0 }) },
    "@/runtime": { fleet, authenticate: () => auth.promise, biometricSupport: () => Promise.resolve({ label: "Face ID" }), useApp: () => ({ settings: { biometricLock: true }, hosts: [host], ready: true, loadError: null }) },
    "@/answer": { keepAsked: (x: unknown) => x, sameQuestion: () => true, primaryChoice: () => 0, replyChoice: () => null, askingTitle: () => "Continue?" },
    "@/now": { programName: () => "Fixture" },
    "@/lock": { needsAuthBeforeRun: () => true },
    "@/fleet": { canOperate: () => true, hasFeature: () => true },
    "@/topology": { findPane: () => ({ pane: { waiting }, tab: { panes: [{ id: 12 }] }, paneIndex: 0 }), tabTitle: () => "Fixture" },
    "@/pairing": { parsePairLink: () => ({ key: "host", name: "Fixture", addresses: [], relay: null, permission: "operate" }) },
    "@/protocol/relay": { addressHost: (s: string) => s },
    "@/ui/Icon": { Icon: "Icon" }, "@/ui/Notice": { ErrorText: "ErrorText", WarningText: "WarningText" },
    "@/ui/PromptText": { PromptText: "PromptText" }, "@/ui/Screen": { PageHeader: "PageHeader" }, "@/ui/Button": { Button: "Button" },
    "@/ui/theme": themeStub,
    "@/closing": { afterClose: () => 13, closedText: () => "Closed", closeConfirm: () => null, closesCurrent: () => true, runningCount: () => 0 },
    "./confirm": { confirmDestructive: () => Promise.resolve(true) },
    "@/ui/Rows": { Group: "Group", CardBox: "CardBox", SwitchRow: "SwitchRow" },
  };
  function load(path: string): Record<string, unknown> {
    let source = readFileSync(new URL(`../src/${path}`, import.meta.url), "utf8");
    if (path === "app/settings.tsx") source += "\nexport { AppLock };\n";
    if (process.env.RIFT_ACTION_SCOPE_NEGATIVE_CONTROL && path === "ui/useActionScope.ts") source = source.replace("active && at === generation", () => "true");
    const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 } }).outputText;
    const module = { exports: {} as Record<string, unknown> };
    runInNewContext(compiled, { module, exports: module.exports, React: react, require: (name: string) => {
      if (name === "react") return react;
      if (name === "@/ui/useActionScope") return load("ui/useActionScope.ts");
      if (name in dependencies) return dependencies[name];
      if (path === "app/settings.tsx") return {};
      throw new Error(`Unstubbed import ${name}`);
    } }, { filename: path });
    return module.exports;
  }
  function find(tree: unknown, label: string): Element {
    if (Array.isArray(tree)) { for (const item of tree) { const node = find(item, label); if (node) return node; } }
    else if (tree && typeof tree === "object") {
      const node = tree as Element;
      if (node.props?.accessibilityLabel === label || node.props?.label === label || node.type === label) return node;
      return find(node.props?.children, label);
    }
    return undefined as unknown as Element;
  }
  const blur = () => { cleanups.forEach((cleanup) => cleanup()); cleanups = []; };
  const refocus = () => focuses.forEach((effect) => { const cleanup = effect(); if (cleanup) cleanups.push(cleanup); });
  return { load, find, router, auth, ack, fleet, blur, refocus };
}

describe("mobile action visit lifecycle", () => {
  it("Settings cannot enable the app lock after leaving during authentication", async () => {
    const h = harness(); const tree = (h.load("app/settings.tsx").AppLock as () => Element)();
    (h.find(tree, "SwitchRow").props.onChange as (on: boolean) => void)(true);
    h.blur(); h.refocus(); h.auth.resolve({ ok: true }); await flush();
    expect(h.fleet.updateSettings).not.toHaveBeenCalled();
  });
  it("current answer, open and close acknowledgements retain their normal navigation", async () => {
    const answer = harness(); const tree = (answer.load("app/answer.tsx").default as () => Element)();
    (answer.find(tree, "Yes").props.onPress as () => void)(); answer.auth.resolve({ ok: true }); await flush();
    answer.ack.resolve({ ok: true, pane: 13 }); await flush(); expect(answer.router.back).toHaveBeenCalledTimes(1);
    const opened = harness(); (opened.load("ui/useOpenShell.ts").useOpenShell as (...args: unknown[]) => (kind: string) => void)("host", 12, vi.fn())("tab");
    opened.ack.resolve({ ok: true, pane: 13 }); await flush(); expect(opened.router.replace).toHaveBeenCalledTimes(1);
    const closed = harness(); (closed.load("ui/useClose.ts").useClose as (...args: unknown[]) => (pane: number, kind: string) => void)("host", 12, vi.fn(), undefined, true)(12, "pane");
    closed.ack.resolve({ ok: true, pane: 13 }); await flush(); expect(closed.router.dismiss).toHaveBeenCalledTimes(1); expect(closed.router.replace).toHaveBeenCalledTimes(1);
  });
  it("an old capture stays cancelled after refocus", () => {
    const h = harness();
    const scope = (h.load("ui/useActionScope.ts").createActionScope as () => { capture: () => () => boolean; enter: () => void; leave: () => boolean })();
    const old = scope.capture(); scope.leave(); scope.enter();
    expect(old()).toBe(false); expect(scope.capture()()).toBe(true);
  });
  it("Answer does not send after leaving and returning during authentication", async () => {
    const h = harness(); const tree = (h.load("app/answer.tsx").default as () => Element)();
    (h.find(tree, "Yes").props.onPress as () => void)();
    h.blur(); h.refocus(); h.auth.resolve({ ok: true }); await flush();
    expect(h.fleet.answer).not.toHaveBeenCalled(); expect(h.router.back).not.toHaveBeenCalled();
  });
  it("a late answer acknowledgement does not navigate a later visit", async () => {
    const h = harness(); const tree = (h.load("app/answer.tsx").default as () => Element)();
    (h.find(tree, "Yes").props.onPress as () => void)(); h.auth.resolve({ ok: true }); await flush();
    expect(h.fleet.answer).toHaveBeenCalledTimes(1);
    h.blur(); h.refocus(); h.ack.resolve({ ok: true, pane: 13 }); await flush(); expect(h.router.back).not.toHaveBeenCalled();
  });
  it("Pair ignores completion after Cancel while storage is still pending", async () => {
    const h = harness(); const tree = (h.load("app/pair.tsx").default as () => Element)();
    (h.find(tree, "Pair").props.onPress as () => void)(); (h.find(tree, "Cancel").props.onPress as () => void)();
    h.ack.resolve({ ok: true, pane: 13, host: { key: "host" } }); await flush();
    expect(h.router.back).toHaveBeenCalledTimes(1); expect(h.router.push).not.toHaveBeenCalled(); expect(h.router.dismissTo).not.toHaveBeenCalled();
  });
  it("a late open acknowledgement cannot redirect after leaving and returning", async () => {
    const h = harness(); const open = (h.load("ui/useOpenShell.ts").useOpenShell as (...args: unknown[]) => (kind: string) => void)("host", 12, vi.fn());
    open("tab"); h.blur(); h.refocus(); h.ack.resolve({ ok: true, pane: 13 }); await flush(); expect(h.router.replace).not.toHaveBeenCalled();
  });
  it("a late close acknowledgement cannot dismiss a different sheet", async () => {
    const h = harness(); const close = (h.load("ui/useClose.ts").useClose as (...args: unknown[]) => (pane: number, kind: string) => void)("host", 12, vi.fn(), undefined, true);
    close(12, "pane"); h.blur(); h.refocus(); h.ack.resolve({ ok: true, pane: 13 }); await flush();
    expect(h.router.dismiss).not.toHaveBeenCalled(); expect(h.router.replace).not.toHaveBeenCalled();
  });
});
