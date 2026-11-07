import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { themeStub } from "./helpers/themeStub";

type Node = { type: string; props: { label?: string; disabled?: boolean; onPress?: () => void; children: unknown[]; footer?: unknown } };
type Auth = { ok: true } | { ok: false; message: string };
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

// Exercise the actual screen callback and single-flight hook without loading native Expo
// modules in Node. Platform leaves and state reads are fixtures; this is not a layout test.
function harness(deferRun = false, parent = false, locked = false) {
  const authResults: ((result: Auth) => void)[] = [];
  const authenticate = vi.fn(() => new Promise<Auth>((resolve) => authResults.push(resolve)));
  const runResults: ((result: { ok: true }) => void)[] = [];
  const run = vi.fn(() => deferRun ? new Promise<{ ok: true }>((resolve) => runResults.push(resolve)) : Promise.resolve({ ok: true }));
  const back = vi.fn();
  const dismissTo = vi.fn();
  const cleanups: (() => void)[] = [];
  const react = {
    createElement: (type: string, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }),
    useEffect: (effect: () => (() => void) | undefined) => { const cleanup = effect(); if (cleanup) cleanups.push(cleanup); },
    useRef: (initial: unknown) => ({ current: initial }),
    useCallback: (fn: unknown) => fn,
    useState: (initial: unknown) => [typeof initial === "function" ? initial() : initial, () => {}],
  };
  function load(path: string, dependencies: Record<string, unknown>) {
    let source = readFileSync(new URL(path, import.meta.url), "utf8");
    // Optional in-memory mutation for checking that the regression catches a missing guard.
    if (process.env.RIFT_CONFIRM_NEGATIVE_CONTROL && path.endsWith("useSingleFlight.ts")) {
      source = source.replace("if (busy) return;", () => "");
    }
    if (process.env.RIFT_CONFIRM_LIFECYCLE_NEGATIVE_CONTROL && path.endsWith("confirm.tsx")) {
      source = source.replaceAll("if (!current()) return;", () => "");
    }
    if (process.env.RIFT_CONFIRM_PARENT_NEGATIVE_CONTROL && path.endsWith("confirm.tsx")) {
      source = source.replace('if (parent === "1")', () => 'if (false)');
    }
    const compiled = ts.transpileModule(source, {
      compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
    }).outputText;
    const module = { exports: {} as Record<string, unknown> };
    runInNewContext(compiled, {
      module, exports: module.exports, React: react,
      require: (name: string) => {
        if (name === "react") return react;
        if (!(name in dependencies)) throw new Error(`Unstubbed import ${name}`);
        return dependencies[name];
      },
    }, { filename: path });
    return module.exports;
  }
  const screen = load("../src/app/confirm.tsx", {
    "expo-router": { useRouter: () => ({ back, dismissTo }), useLocalSearchParams: () => ({ key: "host", pane: "12", parent: parent ? "1" : undefined }) },
    "react-native": { AccessibilityInfo: { announceForAccessibility() {} }, StyleSheet: { create: (s: unknown) => s }, Text: "Text", View: "View" },
    "react-native-safe-area-context": { useSafeAreaInsets: () => ({ bottom: 0 }) },
    "@/chips": { tildePath: (s: string) => s, chipCommand: () => "", isMenuCommand: () => false },
    "@/fleet": { canOperate: () => true, draftKey: () => "host:12" },
    "@/lock": { needsAuthBeforeRun: () => true },
    "@/protocol/types": { commandProblem: () => null, MAX_MENU_NAME: 512 },
    "@/runtime": {
      authenticate, fleet: { run },
      useApp: () => ({ settings: {}, hosts: [{ record: { key: "host", name: "Fixture" }, windows: [], conn: { status: "open" } }], drafts: { "host:12": "echo fixture" } }),
      useLock: () => ({ locked, covered: false }),
    },
    "@/topology": { findPane: () => ({ pane: {} }), paneBranch: () => "", paneTitle: () => "Fixture pane", runningCommand: () => null },
    "@/ui/Button": { Button: "Button" },
    "@/ui/Notice": { ErrorText: "ErrorText", WarningText: "WarningText" },
    "@/ui/SheetFrame": { SheetFrame: "SheetFrame", SheetButtons: "SheetButtons" },
    "@/ui/theme": themeStub,
    "@/ui/useSingleFlight": load("../src/ui/useSingleFlight.ts", {}),
    "@/ui/useActionScope": load("../src/ui/useActionScope.ts", { "expo-router": { useFocusEffect: (effect: () => (() => void)) => cleanups.push(effect()) } }),
  });
  const buttons: Node[] = [];
  function walk(value: unknown) {
    if (!value || typeof value !== "object") return;
    if (Array.isArray(value)) return value.forEach(walk);
    const node = value as Node;
    if (node.type === "Button") buttons.push(node);
    node.props?.children.forEach(walk);
    // a sheet's pinned actions (SheetFrame `footer`)
    walk(node.props?.footer);
  }
  walk((screen.default as () => Node)());
  const button = buttons.find((node) => node.props.label === "Run");
  expect(button?.props.disabled).toBe(false);
  expect(button?.props.onPress).toBeTypeOf("function");
  const cancel = buttons.find((node) => node.props.label === "Cancel")!.props.onPress!;
  return { press: button!.props.onPress!, cancel, unmount: () => cleanups.forEach((cleanup) => cleanup()), authResults, runResults, authenticate, run, back, dismissTo };
}

describe("ConfirmScreen authentication", () => {
  it("returns Cancel to the parent sheet without dismissing it", () => {
    const h = harness(false, true);
    h.cancel();
    expect(h.back).toHaveBeenCalledTimes(1);
    expect(h.dismissTo).not.toHaveBeenCalled();
  });

  it("dismisses both sheets to the pane after a successful run", async () => {
    const h = harness(false, true);
    h.press();
    h.authResults[0]!({ ok: true });
    await flush();
    expect(h.back).not.toHaveBeenCalled();
    expect(h.dismissTo).toHaveBeenCalledExactlyOnceWith({ pathname: "/host/[key]/pane/[id]", params: { key: "host", id: "12" } });
  });

  it("removes the parent sheet as well when locking", () => {
    const h = harness(false, true, true);
    expect(h.back).not.toHaveBeenCalled();
    expect(h.dismissTo).toHaveBeenCalledExactlyOnceWith({ pathname: "/host/[key]/pane/[id]", params: { key: "host", id: "12" } });
  });

  it("leaves the confirmation only once when Cancel is pressed twice", () => {
    const h = harness();
    h.cancel();
    h.cancel();
    expect(h.back).toHaveBeenCalledTimes(1);
  });

  for (const leave of ["cancel", "unmount"] as const) {
    it(`does not send after ${leave} during authentication`, async () => {
      const h = harness();
      h.press();
      h[leave]();
      h.authResults[0]!({ ok: true });
      await flush();
      expect(h.run).not.toHaveBeenCalled();
      expect(h.back).toHaveBeenCalledTimes(leave === "cancel" ? 1 : 0);
    });

    it(`does not navigate again after ${leave} during a command acknowledgement`, async () => {
      const h = harness(true);
      h.press();
      h.authResults[0]!({ ok: true });
      await flush();
      expect(h.run).toHaveBeenCalledTimes(1);
      h[leave]();
      h.runResults[0]!({ ok: true });
      await flush();
      expect(h.back).toHaveBeenCalledTimes(leave === "cancel" ? 1 : 0);
    });
  }

  it("sends once when Run is pressed twice before authentication completes", async () => {
    const h = harness();
    h.press();
    h.press();
    expect(h.authenticate).toHaveBeenCalledTimes(1);
    expect(h.run).not.toHaveBeenCalled();
    h.authResults[0]!({ ok: true });
    await flush();
    expect(h.run).toHaveBeenCalledExactlyOnceWith("host", 12, "echo fixture", { fromDraft: true });
    expect(h.back).toHaveBeenCalledTimes(1);
  });

  it("allows a cancelled authentication to retry without duplicating the retry", async () => {
    const h = harness();
    h.press();
    h.authResults[0]!({ ok: false, message: "Cancelled." });
    await flush();
    expect(h.run).not.toHaveBeenCalled();
    expect(h.back).not.toHaveBeenCalled();
    h.press();
    h.press();
    expect(h.authenticate).toHaveBeenCalledTimes(2);
    h.authResults[1]!({ ok: true });
    await flush();
    expect(h.run).toHaveBeenCalledExactlyOnceWith("host", 12, "echo fixture", { fromDraft: true });
    expect(h.back).toHaveBeenCalledTimes(1);
  });
});
