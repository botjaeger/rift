import { describe, expect, it, vi } from "vitest";
import * as paneView from "../src/paneView";
import { MAX_MARKS, markParts } from "../src/paneView";
import type { Block } from "../src/protocol/types";
import * as selection from "../src/selection";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, react, render, textOf, uiDeps } from "./helpers/render";

// Search output marks every match in the blocks it shows (the New Blocks board's "Search output"
// mock): the pure split, then the app's own cards run with a query. Pixels stay device checks.

describe("markParts", () => {
  it("marks every occurrence in any case and keeps the text exactly", () => {
    const parts = markParts("Error: an error in ERRORS", "error")!;
    expect(parts.filter((p) => p.mark).map((p) => p.text)).toEqual(["Error", "error", "ERROR"]);
    expect(parts.map((p) => p.text).join("")).toBe("Error: an error in ERRORS");
    expect(parts.at(-1)).toEqual({ text: "S", mark: false });
  });

  it("is null without a query or a match, so the plain text draws as before", () => {
    expect(markParts("abc", "")).toBeNull();
    expect(markParts("abc", "   ")).toBeNull();
    expect(markParts("abc", "x")).toBeNull();
    expect(markParts("", "a")).toBeNull();
  });

  it("trims the query and takes regex characters literally", () => {
    expect(markParts("a.b axb", " a.b ")!.filter((p) => p.mark).map((p) => p.text)).toEqual(["a.b"]);
    expect(markParts("f(x) [1] $5 *", "(x)")!.find((p) => p.mark)!.text).toBe("(x)");
    expect(markParts("a\\b", "\\")!.find((p) => p.mark)!.text).toBe("\\");
  });

  it("marks at most MAX_MARKS in one text, the rest plain", () => {
    const text = "ab ".repeat(MAX_MARKS + 50);
    const parts = markParts(text, "ab")!;
    expect(parts.filter((p) => p.mark)).toHaveLength(MAX_MARKS);
    expect(parts.map((p) => p.text).join("")).toBe(text);
    expect(markParts("aaaa", "a", 2)!.map((p) => [p.text, p.mark])).toEqual([["a", true], ["a", true], ["aa", false]]);
  });
});

const block = (b: Partial<Block>): Block => ({
  id: 1,
  command: "npm test",
  state: "done",
  exit: 0,
  duration_ms: 1200,
  cwd: null,
  branch: null,
  output: "Tests: 2 failed · error in App.test.tsx\nmore",
  dropped: 0,
  restored: false,
  ...b,
});

function deps(platform: "ios" | "android" = "ios") {
  const d = uiDeps(platform, {
    "@/paneView": paneView,
    "@/selection": selection,
    "@/haptics": { copied: () => {} },
    "expo-clipboard": { setStringAsync: vi.fn(async () => {}) },
    "./SelectionScope": { useSelectionScope: () => null, useSelectionOwner: () => null },
    "./StatusMark": { Spinner: "Spinner" },
  });
  d.react = { ...react, memo: (f: unknown) => f, useId: () => "id" };
  d["react-native"] = { ...(d["react-native"] as object), AccessibilityInfo: { announceForAccessibility: () => {} } };
  return d;
}

const marks = (tree: unknown) => findAll(tree, (n) => n.type === "Text" && flat(n.props.style).backgroundColor === stubColors.tints.match);
const fields = (tree: unknown) => findAll(tree, (n) => n.type === "TextInput");

describe("the blocks Search output shows", () => {
  const { BlockView, Output } = load("ui/BlockView.tsx", deps()) as { BlockView: (p: object) => unknown; Output: (p: object) => unknown };
  const draw = (b: Partial<Block>, mark?: string, wrap = true) =>
    render({ type: BlockView, props: { block: block(b), onActions: () => {}, wrap, mark, children: [] } });

  it("marks the query in the command and the output, bright on the match tint", () => {
    const tree = draw({ command: "grep error log", output: "an Error here\n" }, "error");
    expect(marks(tree).map(textOf)).toEqual(["error", "Error"]);
    for (const m of marks(tree)) expect(flat(m.props.style).color).toBe(stubColors.textBright);
  });

  it("keeps the output a selectable field holding the same text (styled pieces in place of value)", () => {
    const tree = draw({ output: "an Error here\n" }, "error");
    const [field] = fields(tree);
    expect(field!.props.value).toBeUndefined();
    expect(textOf(field)).toBe("an Error here");
    expect(field!.props.onSelectionChange).toBeTypeOf("function");
  });

  it("draws as before without a query or a match: value, no marks", () => {
    for (const q of [undefined, "", "zzz"]) {
      const tree = draw({ output: "an Error here\n" }, q);
      expect(marks(tree)).toHaveLength(0);
      expect(fields(tree)[0]!.props.value).toBe("an Error here");
      expect(fields(tree)[0]!.props.children).toEqual([null]);
    }
  });

  it("marks inside wide output that scrolls sideways (wrap off)", () => {
    const text = `${"x".repeat(200)} needle ${"y".repeat(200)}`;
    const tree = render({ type: Output, props: { text, wrap: false, ground: "#000000", mark: "NEEDLE", children: [] } });
    const scroll = findAll(tree, (n) => n.type === "ScrollView")[0]!;
    expect(scroll.props.horizontal).toBe(true);
    expect(marks(scroll).map(textOf)).toEqual(["needle"]);
    expect(textOf(fields(scroll)[0])).toBe(text);
  });

  it("marks a table's rows (as a list)", () => {
    const output = "IMAGE    NAMES      STATUS\nnginx    web-error  Up 2 hours\nnode     api        Exited (1) error\n";
    const tree = draw({ command: "docker ps", output }, "error");
    expect(findAll(tree, (n) => n.props.accessibilityRole === "list")).toHaveLength(1);
    expect(marks(tree).map(textOf)).toEqual(["error", "error"]);
  });
});

describe("failed and running cards mark too", () => {
  const { FailedCard, RunningCard } = load("ui/BlockCards.tsx", deps("android")) as {
    FailedCard: (p: object) => unknown;
    RunningCard: (p: object) => unknown;
  };

  it("a failed card marks the command and its message", () => {
    const tree = render({ type: FailedCard, props: { block: block({ command: "cargo build", exit: 101, output: "error[E0425]: cannot find value\n" }), onActions: () => {}, mark: "error", children: [] } });
    expect(marks(tree).map(textOf)).toContain("error");
    // Android: a field never gets both `value` and children
    for (const f of fields(tree)) expect(f.props.value === undefined || (f.props.children as unknown[]).every((c) => c === null)).toBe(true);
  });

  it("a running card marks the command and its latest lines", () => {
    const tree = render({
      type: RunningCard,
      props: { block: block({ command: "make all", state: "running", exit: null, output: "building\nlinking all\n" }), onActions: () => {}, notify: false, onBell: () => {}, mark: "ALL", children: [] },
    });
    expect(marks(tree).map(textOf)).toEqual(["all", "all"]);
  });
});

describe("the pane passes the query", () => {
  it("to every card it shows, and re-renders the list when it changes", async () => {
    const { readFileSync } = await import("node:fs");
    const route = readFileSync(new URL("../src/app/host/[key]/pane/[id].tsx", import.meta.url), "utf8");
    expect(route).toMatch(/const found = searching\?\.trim\(\) \|\| undefined;/);
    expect(route).toMatch(/extraData = useMemo\(\(\) => \(\{[^}]*found[^}]*\}\)/);
    expect(route.match(/mark=\{found\}/g)).toHaveLength(3);
  });
});

