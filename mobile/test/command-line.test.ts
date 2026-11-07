import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { returnTakes } from "../src/chips";
import * as promptChips from "../src/promptChips";
import type { ChipItem } from "../src/promptChips";
import { findAll, flat, load, render, textOf, uiDeps } from "./helpers/render";

// The command line redesign board: one line with one job each side, context you can tap, the
// completions closest to the keyboard with the first one taken by Return, a running program's Stop.
// The field, the keyboard and the layout's pixels stay device checks.

const source = readFileSync(new URL("../src/ui/Composer.tsx", import.meta.url), "utf8");

describe("Return takes the first completion", () => {
  it("at the end, or wherever the cursor put the new line", () => {
    expect(returnTakes("git pu", "git pu\n", "git pull")).toBe("git pull");
    expect(returnTakes("git pu", "git\n pu", "git pull")).toBe("git pull");
  });
  it("nothing to take, or not one new line: the field keeps what was typed", () => {
    expect(returnTakes("git pu", "git pu\n", undefined)).toBeNull();
    expect(returnTakes("git pu", "git pul", "git pull")).toBeNull();
    expect(returnTakes("git pu", "git pu\n\n", "git pull")).toBeNull();
    // a pasted line with a break is a paste, not Return
    expect(returnTakes("", "ls\n", "ls -la")).toBeNull();
  });

  // the field's own handler, from the screen's source: Return at a prompt puts the completion in
  const onChange = (running: string | null, sticky = { ctrl: false, alt: false, shift: false }) => {
    const start = source.indexOf("const onChange = (typed: string) => {");
    const end = source.indexOf("\n  const onOpen", start);
    expect(start).toBeGreaterThan(0);
    const compiled = ts.transpileModule(`${source.slice(start, end)}\nexports.on = onChange;`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
    }).outputText;
    const setDraft = vi.fn();
    const moveCursor = vi.fn();
    const sendKeys = vi.fn();
    const out = { on: (_t: string) => {} };
    runInNewContext(compiled, {
      exports: out,
      running,
      sticky,
      draft: "git pu",
      rows: [{ text: "git pull" }, { text: "git push" }],
      returnTakes,
      plainPunctuation: (t: string) => t,
      setError: () => {},
      setSticky: () => {},
      NO_STICKY: {},
      insertedChar: (a: string, b: string) => b.slice(a.length) || null,
      stickyKey: (ch: string) => `ctrl+${ch === "\n" ? "j" : ch}`,
      sendKeys,
      moveCursor,
      hostKey: "host",
      paneId: 3,
      pk: "host:3",
      fleet: { setDraft },
      // the rest of the handler: the field's text goes in as typed
      pending: { takeFailure: () => {}, view: () => ({ walking: true }) },
      liveDraft: () => "git pu",
    });
    return { on: out.on, setDraft, moveCursor, sendKeys };
  };
  it("at a prompt Return takes the first; with a program running it is a new line", () => {
    const prompt = onChange(null);
    prompt.on("git pu\n");
    expect(prompt.setDraft).toHaveBeenCalledExactlyOnceWith("host", 3, "git pull");
    expect(prompt.moveCursor).toHaveBeenCalledWith("git pull".length);
    const program = onChange("claude");
    program.on("git pu\n");
    expect(program.setDraft).toHaveBeenCalledExactlyOnceWith("host", 3, "git pu\n");
  });
  it("a held ⌃ makes Return a key, not a completion", () => {
    const held = onChange(null, { ctrl: true, alt: false, shift: false });
    held.on("git pu\n");
    expect(held.sendKeys).toHaveBeenCalledWith(["ctrl+j"]);
    expect(held.setDraft).not.toHaveBeenCalled();
  });
});

describe("the command line's order", () => {
  const at = (s: string) => {
    const i = source.indexOf(s);
    expect(i, s).toBeGreaterThan(0);
    return i;
  };
  const view = source.slice(source.indexOf("  return (\n    <KeyCaptureView"));
  const pos = (s: string) => {
    const i = view.indexOf(s);
    expect(i, s).toBeGreaterThan(0);
    return i;
  };
  it("context, then +, ⌃ and the field on one line, then the keys, then the completions", () => {
    expect(pos("<ChipRow")).toBeLessThan(pos("styles.inputRow"));
    expect(pos('accessibilityLabel="Attach"')).toBeLessThan(pos('accessibilityLabel="Control keys"'));
    expect(pos('accessibilityLabel="Control keys"')).toBeLessThan(pos("<TextInput"));
    expect(pos("<TextInput")).toBeLessThan(pos("<ControlRow"));
    expect(pos("<ControlRow")).toBeLessThan(pos("{completions}"));
  });
  it("⌃ shows the keys and says whether they show; no ⌨ beside the field", () => {
    const keys = source.slice(at('accessibilityLabel="Control keys"') - 100, at('<Icon name="control"'));
    expect(keys).toContain("accessibilityState={{ expanded: keys, disabled: waiting }}");
    expect(keys).toContain("onPress={() => setKeys((k) => !k)}");
    expect(view).not.toContain('accessibilityLabel="Keys"');
    expect(view).not.toContain('<Icon name="keyboard"');
  });
  it("no Stop button on the command line, running or not", () => {
    expect(source).not.toContain("Stop ⌃C");
    expect(view.slice(0, pos('accessibilityLabel="Attach"'))).not.toContain('sendKeys(["ctrl+c"])');
  });
  it("an empty field says Command, in the command's mono", () => {
    expect(source).toContain(': "Command"');
    expect(source).toContain("inputEmpty: { height: 36 },");
  });
});

describe("context you can tap", () => {
  const chip = (id: string, text: string, opens: ChipItem["opens"]): ChipItem => ({
    id,
    name: id,
    icon: "folder",
    iconColor: "accent",
    full: [{ text, color: "text" }],
    short: [{ text, color: "text" }],
    attention: false,
    opens,
    spoken: `${id}: ${text}`,
  });
  const draw = (items: ChipItem[], onOpen = vi.fn()) => {
    const { ChipRow } = load("ui/ChipRow.tsx", uiDeps("ios", { "@/promptChips": promptChips })) as { ChipRow: (p: object) => unknown };
    return { tree: render({ type: ChipRow, props: { items, onOpen, onMore: () => {}, children: [] } }), onOpen };
  };
  it("the timer is plain text after the chips, still opening its card", () => {
    const { tree, onOpen } = draw([chip("cwd", "project1", "dirs"), chip("elapsed", "2m", "elapsed"), chip("git_branch", "main", "branches")]);
    const timer = findAll(tree, (n) => n.type === "Pressable" && n.props.accessibilityLabel === "elapsed: 2m")[0]!;
    expect(textOf(timer)).toBe("· 2m");
    // no chip's border or fill
    expect(flat(timer.props.style).borderWidth).toBeUndefined();
    (timer.props.onPress as () => void)();
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "elapsed" }));
    // the measuring copy holds the chips only
    const probe = findAll(tree, (n) => n.type === "View" && flat(n.props.style).opacity === 0)[0]!;
    expect(textOf(probe)).not.toContain("2m");
    expect(textOf(probe)).toContain("project1");
  });
  it("chips are 28 pt pills", () => {
    const { tree } = draw([chip("cwd", "project1", "dirs")]);
    const probe = findAll(tree, (n) => n.type === "View" && flat(n.props.style).opacity === 0)[0]!;
    const c = findAll(probe, (n) => n.type === "View" && flat(n.props.style).height === 28)[0]!;
    expect(flat(c.props.style).borderRadius).toBe(14);
  });
});
