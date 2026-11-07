import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { themeStub } from "./helpers/themeStub";

type Input = {
  value: string;
  onChangeText: (text: string) => void;
  onFocus: () => void;
  onBlur: () => Promise<void>;
  onSubmitEditing: () => Promise<void>;
};
type Node = { type: unknown; props: Record<string, unknown> & { children: unknown[] } };

// Execute the real field and state transitions; native views and Fleet persistence are
// fixture leaves. The rendered app's storage-failure walk remains a separate check.
function harness() {
  let saved = "Fixture phone";
  const writes: { deviceName: string; resolve: (ok: boolean) => void }[] = [];
  const updateSettings = vi.fn(({ deviceName }: { deviceName: string }) => new Promise<boolean>((resolve) => {
    writes.push({ deviceName, resolve: (ok) => { if (ok) saved = deviceName; resolve(ok); } });
  }));
  const states: unknown[] = [];
  let cursor = 0;
  const react = {
    createElement: (type: unknown, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }),
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in states)) states[index] = initial;
      return [states[index], (next: unknown) => {
        states[index] = typeof next === "function" ? next(states[index]) : next;
      }];
    },
    useRef: (initial: unknown) => ({ current: initial }),
  };
  let source = readFileSync(new URL("../src/app/settings.tsx", import.meta.url), "utf8");
  if (process.env.RIFT_SETTINGS_NAME_NEGATIVE_CONTROL) {
    source = source.replace(
      /const save = async \(\) => \{[\s\S]*?\n {2}\};/,
      () => "const save = async () => { if (draft !== null) void fleet.updateSettings({ deviceName: draft }); setDraft(null); };",
    ).replace("setDraft((current) => current ?? name)", () => "setDraft(name)");
  }
  if (process.env.RIFT_SETTINGS_NAME_OVERWRITE_NEGATIVE_CONTROL) {
    source = source.replace("setDraft((current) => current === draft ? null : current)", () => "setDraft(null)");
  }
  source += "\nexports.DeviceName = DeviceName;\n";
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const module = { exports: {} as { DeviceName: (props: { name: string }) => Node } };
  const leaves = new Proxy({}, { get: (_target, key) => String(key) });
  runInNewContext(compiled, {
    module, exports: module.exports, React: react,
    require: (name: string) => {
      if (name === "react") return react;
      if (name === "react-native") return { TextInput: "TextInput", Text: "Text", View: "View", Pressable: "Pressable", StyleSheet: { create: (s: unknown) => s } };
      if (name === "@/runtime") return { fleet: { updateSettings } };
      if (name === "@/ui/theme") return themeStub;
      return leaves;
    },
  });
  const input = (): Input => {
    cursor = 0;
    const root = module.exports.DeviceName({ name: saved });
    const walk = (value: unknown): Input | undefined => {
      if (Array.isArray(value)) return value.map(walk).find((item) => item !== undefined);
      if (!value || typeof value !== "object") return undefined;
      const node = value as Node;
      if (node.type === "TextInput") return node.props as unknown as Input;
      return walk(node.props?.children);
    };
    return walk(root)!;
  };
  const edit = (value: string) => input().onChangeText(value);
  return { input, edit, writes, updateSettings, saved: () => saved };
}

describe("the phone name saves without losing edits", () => {
  it("keeps a rejected draft through focus and retry, then shows the saved name", async () => {
    const h = harness();
    h.input().onFocus();
    h.edit("Retry phone");
    const first = h.input().onBlur();
    h.writes[0]!.resolve(false);
    await first;
    expect(h.input().value).toBe("Retry phone");
    expect(h.saved()).toBe("Fixture phone");
    h.input().onFocus();
    expect(h.input().value).toBe("Retry phone");
    const retry = h.input().onSubmitEditing();
    h.writes[1]!.resolve(true);
    await retry;
    expect(h.input().value).toBe("Retry phone");
    expect(h.saved()).toBe("Retry phone");
  });

  it("keeps a newer edit when the earlier save succeeds", async () => {
    const h = harness();
    h.edit("First phone");
    const first = h.input().onBlur();
    h.edit("Second phone");
    h.writes[0]!.resolve(true);
    await first;
    expect(h.input().value).toBe("Second phone");
    expect(h.saved()).toBe("First phone");
  });
});
