import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { programSend } from "../src/paneView";

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
  const start = source.indexOf("const sendToProgram = async () => {");
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
    draft: stored, canType: true, sending: { current: false }, setError: vi.fn(),
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
