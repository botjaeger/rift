import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { keyEventToAction } from "../src/keyboard";
import { commandNote, fieldKey, programSend } from "../src/paneView";

// Execute the screen's actual async callback with delayed transport acknowledgement.
// Rendering and native authentication remain separate UI/native checks.
function harness() {
  let source = readFileSync(new URL("../src/ui/Composer.tsx", import.meta.url), "utf8");
  if (process.env.RIFT_COMPOSER_SPENT_NEGATIVE_CONTROL) {
    source = source.replace(
      "if (t.ok) fleet.clearDraft(hostKey, paneId, draft);",
      () => "",
    ).replace("if (!t.ok) return setError(t.message);", () => "if (!t.ok) return setError(t.message); fleet.clearDraft(hostKey, paneId, draft);");
  }
  const start = source.indexOf("const sendToProgram = async (authed = false) => {");
  const end = source.indexOf("\n  const problem =", start);
  expect(start).toBeGreaterThan(0);
  expect(end).toBeGreaterThan(start);
  const compiled = ts.transpileModule(`${source.slice(start, end)}\nexports.send = sendToProgram;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  let active = true;
  let stored = "fixture input";
  let resolve!: (result: { ok: boolean; message?: string }) => void;
  const sendText = vi.fn(() => new Promise((done) => { resolve = done; }));
  const sendKeys = vi.fn(() => Promise.resolve({ ok: true }));
  const clearDraft = vi.fn((_host: string, _pane: number, sent: string) => { if (stored === sent) stored = ""; });
  const module = { exports: {} as { send: () => Promise<void> } };
  runInNewContext(compiled, {
    exports: module.exports, scope: { capture: () => () => active }, programSend,
    draft: stored, canType: true, canPaste: false, sendWhole: vi.fn(), sending: { current: false }, setError: vi.fn(),
    keysNeedAuth: () => false, s: { settings: {} }, hostKey: "host", paneId: 12,
    fleet: { sendText, sendKeys, clearDraft }, AccessibilityInfo: { announceForAccessibility: vi.fn() },
  });
  return { send: module.exports.send, sendText, sendKeys, clearDraft, leave: () => { active = false; },
    replaceDraft: (text: string) => { stored = text; }, stored: () => stored,
    acknowledge: (ok = true) => resolve({ ok, message: ok ? undefined : "Offline" }) };
}

describe("Composer acknowledged program input", () => {
  it("clears accepted text after leaving without sending Return", async () => {
    const h = harness(); const sent = h.send();
    expect(h.sendText).toHaveBeenCalledExactlyOnceWith("host", 12, "fixture input");
    h.leave(); h.acknowledge(); await sent;
    expect(h.stored()).toBe(""); expect(h.sendKeys).not.toHaveBeenCalled();
  });
  it("preserves a newer draft after leaving", async () => {
    const h = harness(); const sent = h.send();
    h.replaceDraft("newer input"); h.leave(); h.acknowledge(); await sent;
    expect(h.clearDraft).toHaveBeenCalledExactlyOnceWith("host", 12, "fixture input");
    expect(h.stored()).toBe("newer input"); expect(h.sendKeys).not.toHaveBeenCalled();
  });
  it("keeps rejected text", async () => {
    const h = harness(); const sent = h.send(); h.leave(); h.acknowledge(false); await sent;
    expect(h.stored()).toBe("fixture input"); expect(h.clearDraft).not.toHaveBeenCalled();
    expect(h.sendKeys).not.toHaveBeenCalled();
  });
  it("clears accepted text and sends Return during the same visit", async () => {
    const h = harness(); const sent = h.send(); h.acknowledge(); await sent;
    expect(h.stored()).toBe(""); expect(h.sendKeys).toHaveBeenCalledExactlyOnceWith("host", 12, ["enter"]);
  });
});

// Return and Enter: the phone keyboard's Return is a new line, only Send or Run sends; a hardware
// Return sends and Shift+Return writes a line. The field's props and the shortcut handler are the
// screen's own source; the native key capture and the keyboards are device checks.
describe("Composer Return and Enter", () => {
  const source = readFileSync(new URL("../src/ui/Composer.tsx", import.meta.url), "utf8");
  const shortcut = (src: string) => {
    const start = src.indexOf("const onShortcut = (e: KeyEvent) => {");
    const end = src.indexOf("\n  // Several lines:", start);
    expect(start).toBeGreaterThan(0);
    const compiled = ts.transpileModule(`${src.slice(start, end)}\nexports.on = onShortcut;`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
    }).outputText;
    const run = vi.fn(), insert = vi.fn(), onRowKey = vi.fn();
    const out = { on: (_e: unknown) => {} };
    runInNewContext(compiled, { exports: out, online: true, run, insert, onRowKey, fieldKey, keyEventToAction });
    return { on: out.on, run, insert, onRowKey };
  };
  const press = (key: string, shift = false) => ({ key, ctrl: false, alt: false, shift, meta: false });

  const rowKey = (src: string, draft: string, running: string | null = "claude") => {
    const start = src.indexOf("const onRowKey = (id: string) => {");
    const end = src.indexOf("\n  // the row's Paste:", start);
    expect(start).toBeGreaterThan(0);
    const compiled = ts.transpileModule(`${src.slice(start, end)}\nexports.on = onRowKey;`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
    }).outputText;
    const run = vi.fn(), insert = vi.fn(), sendKeys = vi.fn();
    const out = { on: (_id: string) => {} };
    runInNewContext(compiled, {
      exports: out, draft, running, run, insert, sendKeys, sticky: {}, setSticky: () => {}, NO_STICKY: {},
      applySticky: (id: string) => id,
    });
    return { on: out.on, run, insert, sendKeys };
  };
  it("the key row's Return is a new line with text in the box and the program's Return without", () => {
    for (const running of ["claude", null]) {
      const typed = rowKey(source, "hi", running);
      typed.on("enter");
      expect(typed.insert).toHaveBeenCalledExactlyOnceWith("\n");
      expect(typed.run).not.toHaveBeenCalled();
      expect(typed.sendKeys).not.toHaveBeenCalled();
      const empty = rowKey(source, "", running);
      empty.on("enter");
      expect(empty.sendKeys).toHaveBeenCalledExactlyOnceWith(["enter"]);
      expect(empty.run).not.toHaveBeenCalled();
      expect(empty.insert).not.toHaveBeenCalled();
    }
  });
  it("the field does not submit on the phone keyboard's Return", () => {
    expect(source).not.toContain("onSubmitEditing");
    expect(source).not.toContain('submitBehavior="submit"');
    expect(source).toContain('submitBehavior="newline"');
    expect(source).not.toContain('returnKeyType="send"');
  });
  it("a hardware Return sends, Shift+Return writes a line, another key stays a key", () => {
    const h = shortcut(source);
    h.on(press("Enter"));
    expect(h.run).toHaveBeenCalledTimes(1);
    expect(h.insert).not.toHaveBeenCalled();
    h.on(press("Enter", true));
    expect(h.insert).toHaveBeenCalledExactlyOnceWith("\n");
    expect(h.run).toHaveBeenCalledTimes(1);
    h.on({ ...press("c"), ctrl: true });
    expect(h.onRowKey).toHaveBeenCalledExactlyOnceWith("ctrl+c");
  });
  it("an empty box sends nothing: Run is off and Return confirms nothing", () => {
    // a pasted item is a token in the draft, so a box holding one is never empty
    expect(source).toContain('const canRun = online && !waiting && draft.trim() !== "" && note === null;');
    expect(source).not.toContain('if (running !== null && draft === "") return void sendKeys(["enter"])');
  });
  it("a draft with line breaks at a shell prompt says it cannot run; to a program it can", () => {
    expect(commandNote("a\nb", false)).toMatch(/one line/);
    expect(commandNote("a\r\nb", false)).not.toBeNull();
    expect(commandNote("ab", false)).toBeNull();
    expect(commandNote("a\nb", true)).toBeNull();
  });
  it("the key names a hardware keyboard sends", () => {
    expect(fieldKey("enter")).toBe("send");
    expect(fieldKey("shift+enter")).toBe("newline");
    expect(fieldKey("ctrl+j")).toBeNull();
    expect(fieldKey("ctrl+enter")).toBeNull();
  });
});
