// Runs the app's real component modules in a bare context: TSX is transpiled, React is a
// stand-in whose elements are plain objects, native views are their names, and function
// components are expanded so a test reads the whole tree (labels, roles, styles, handlers).
// Layout, gestures and native rendering stay device checks.
import { existsSync, readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { themeStub } from "./themeStub";

export type Node = { type: unknown; props: Record<string, unknown> & { children: unknown[] } };

/** The React stand-in: elements as objects; hooks as plain values (one render, no re-render). */
export const react = {
  createElement: (type: unknown, props: object | null, ...children: unknown[]) => ({ type, props: { ...(props ?? {}), children } }),
  Fragment: "Fragment",
  useState: (initial: unknown) => [typeof initial === "function" ? (initial as () => unknown)() : initial, () => {}],
  useCallback: (fn: unknown) => fn,
  useMemo: (fn: () => unknown) => fn(),
  useRef: (initial: unknown) => ({ current: initial }),
  useEffect: () => {},
  useContext: () => undefined,
};

/** What every UI module may import, stubbed; `extra` adds or replaces entries. */
export function uiDeps(platform: "ios" | "android" | "web", extra: Record<string, unknown> = {}): Record<string, unknown> {
  const rn = {
    Platform: { OS: platform },
    StyleSheet: { create: (s: unknown) => s, absoluteFill: {} },
    Pressable: "Pressable",
    Text: "Text",
    View: "View",
    TextInput: "TextInput",
    ScrollView: "ScrollView",
    Switch: "Switch",
    KeyboardAvoidingView: "KeyboardAvoidingView",
    // the boards' phone (402 × 874): the sheets' heights (ui/sheet.ts)
    useWindowDimensions: () => ({ width: 402, height: 874, scale: 3, fontScale: 1 }),
  };
  const deps: Record<string, unknown> = {
    react,
    "react-native": rn,
    // src/links as the app runs it without a handler: the system opens the link (the deps'
    // `Linking`, looked up per call so a test's own stub counts)
    "@/links": {
      openLink: (url: string) => (deps["react-native"] as { Linking?: { openURL(u: string): unknown } }).Linking?.openURL(url),
    },
    "react-native-safe-area-context": { useSafeAreaInsets: () => ({ top: 47, bottom: 34, left: 0, right: 0 }) },
    "./theme": themeStub,
    // a link's colour lifted on the screen's ground: the accent as it is (no contrast to judge in a stub)
    "./palette": { lift: (fg: string) => fg },
    "./Icon": { Icon: "Icon" },
    "./toggle": {
      toggleA11y: (on: boolean, disabled = false) => ({ accessibilityRole: "togglebutton", accessibilityState: { checked: on, disabled } }),
    },
    ...extra,
  };
  return deps;
}

/**
 * Loads `src/<path>` with `deps`; a relative import not in `deps` is loaded the same way from
 * `src/ui/` (Rows uses the real Screen). `edit` changes the source first (negative controls).
 */
export function load(path: string, deps: Record<string, unknown>, edit?: (source: string) => string): Record<string, unknown> {
  let source = readFileSync(new URL(`../../src/${path}`, import.meta.url), "utf8");
  if (edit) source = edit(source);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const module = { exports: {} as Record<string, unknown> };
  runInNewContext(
    compiled,
    {
      module,
      exports: module.exports,
      React: react,
      // the test's timers, looked up per call (fake timers swap them in after the load)
      setTimeout: (...a: Parameters<typeof setTimeout>) => globalThis.setTimeout(...a),
      clearTimeout: (id?: ReturnType<typeof setTimeout>) => globalThis.clearTimeout(id),
      require: (name: string) => {
        if (name in deps) return deps[name];
        if (name.startsWith("./")) {
          const base = `ui/${name.slice(2)}`;
          return load(existsSync(new URL(`../../src/${base}.tsx`, import.meta.url)) ? `${base}.tsx` : `${base}.ts`, deps);
        }
        throw new Error(`Unstubbed import ${name} in ${path}`);
      },
    },
    { filename: path },
  );
  return module.exports;
}

/** Expands function components and evaluates `style` functions (not pressed), depth first. */
export function render(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(render);
  if (!value || typeof value !== "object") return value;
  const node = value as Node;
  if (typeof node.type === "function") {
    const { children, ...rest } = node.props;
    const props = children.length === 0 ? rest : { ...rest, children: children.length === 1 ? children[0] : children };
    return render((node.type as (p: unknown) => unknown)(props));
  }
  const props = { ...node.props };
  if (typeof props.style === "function") props.style = (props.style as (s: { pressed: boolean }) => unknown)({ pressed: false });
  props.children = (node.props.children ?? []).map(render);
  return { type: node.type, props };
}

/** Every node of a rendered tree that `match` accepts, in order. */
export function findAll(tree: unknown, match: (n: Node) => boolean): Node[] {
  const out: Node[] = [];
  const walk = (v: unknown) => {
    if (Array.isArray(v)) return v.forEach(walk);
    if (!v || typeof v !== "object") return;
    const n = v as Node;
    if (match(n)) out.push(n);
    walk(n.props?.children);
  };
  walk(tree);
  return out;
}

/** The text a node holds (its strings, nested Text included). */
export function textOf(v: unknown): string {
  if (typeof v === "string" || typeof v === "number") return String(v);
  if (Array.isArray(v)) return v.map(textOf).join("");
  if (!v || typeof v !== "object") return "";
  return textOf((v as Node).props?.children);
}

/** A style value (array, nested arrays, falsy entries) flattened to one object. */
export function flat(style: unknown): Record<string, unknown> {
  if (Array.isArray(style)) return Object.assign({}, ...style.map(flat));
  return style && typeof style === "object" ? (style as Record<string, unknown>) : {};
}
