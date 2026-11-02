import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { outHead, outParts, PendingStore, sendParts, plainDraft } from "../src/pending";
import { partSend, programSend } from "../src/paneView";

// Executes the screen's own Send-with-tokens callback against the real store: the draft goes in order,
// then Return as its own request; a failure keeps the failed part and what follows. Rendering is a device check.
function harness(opts: { failAt?: string; auth?: boolean; authFail?: boolean; canPaste?: boolean; leaveAt?: string; onAct?: (what: string, type: (t: string) => void) => void } = {}) {
  const source = readFileSync(new URL("../src/ui/Composer.tsx", import.meta.url), "utf8");
  const start = source.indexOf("const sendWithItems = async () => {");
  const end = source.indexOf("\n  const openMenu", start);
  expect(start).toBeGreaterThan(0);
  expect(end).toBeGreaterThan(start);
  const compiled = ts.transpileModule(`${source.slice(start, end)}\nexports.send = sendWithItems;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const pending = new PendingStore();
  const pk = "host:12";
  let draft = "";
  const add = (item: Parameters<PendingStore["add"]>[1], before: string) => {
    pending.add(pk, item);
    const i = pending.view(pk).items.at(-1)!;
    draft += before;
    draft = pending.place(pk, draft, { start: draft.length, end: draft.length }, i.id, i.label).text;
  };
  const log: string[] = [];
  let live = true;
  const act = (what: string) => {
    log.push(what);
    if (what === opts.leaveAt) live = false;
    opts.onAct?.(what, (more) => { draft += more; });
    return what === opts.failAt ? { ok: false as const, message: "Not connected." } : { ok: true as const };
  };
  const fleet = {
    store: { peek: () => ({ pane: null }) },
    uploadImage: vi.fn(async (_h: string, _p: number, b: string, ext?: string) => act(`upload ${b}${ext ? `.${ext}` : ""}`)),
    sendText: vi.fn(async (_h: string, _p: number, text: string) => act(`text ${JSON.stringify(text)}`)),
    sendKeys: vi.fn(async (_h: string, _p: number, keys: string[]) => act(`keys ${keys}`)),
    setDraft: (_h: string, _p: number, text: string) => { draft = text; },
  };
  const sendWhole = vi.fn(async (p: { paste?: string; lines?: string[] }) => (p.paste !== undefined ? act(`paste ${JSON.stringify(p.paste)}`) : act(`lines ${JSON.stringify(p.lines)}`)));
  const setError = vi.fn();
  const authenticate = vi.fn(async () => (opts.authFail ? { ok: false as const, message: "Cancelled." } : { ok: true as const }));
  const out = { exports: {} as { send: () => Promise<void> } };
  const sending = { current: false };
  const run = () => {
    runInNewContext(compiled, {
      exports: out.exports, program: "claude", isImageItem: () => false, UPLOAD_GAP_MS: 0, waitForImage: async () => {}, boxImages: () => 0, setTimeout, Promise, scope: { capture: () => () => live }, programSend, partSend, draft, canType: true, canPaste: opts.canPaste ?? true,
      sending, setError, liveDraft: () => draft, keysNeedAuth: () => opts.auth ?? false, s: { settings: {} }, authenticate,
      sendParts: (parts: never, send: never, sent: never) => sendParts(parts, send, sent, async () => {}), outParts, outHead, plainDraft, queued: pending.view(pk), pk, hostKey: "host", paneId: 12, fleet, pending, sendWhole, Map,
      AccessibilityInfo: { announceForAccessibility: (t: string) => log.push(`announce ${t}`) },
    });
    return out.exports.send();
  };
  return { run, add, log, draft: () => draft, pending, pk, setError, authenticate, sending, type: (t: string) => { draft += t; pending.edit(pk, draft); } };
}

const image = { kind: "image" as const, b64: "IMG" };
const file = { kind: "file" as const, name: "note.pdf", ext: "pdf", b64: "FILE" };
const text = { kind: "text" as const, text: "1\n2\n3\n4" };

describe("Composer Send with tokens in the draft", () => {
  it("sends the draft in place order, then Return", async () => {
    const h = harness();
    h.add(image, "see ");
    h.add(file, " and ");
    h.add(text, " ");
    h.type(" please");
    expect(h.draft()).toBe("see [Image 1] and [File note.pdf] [Text 4 lines] please");
    await h.run();
    expect(h.log).toEqual(['text "see "', "upload IMG", 'text " and "', "upload FILE.pdf", 'lines [" "]', 'paste "1\\n2\\n3\\n4"', 'text " please"', "keys enter", "announce Sent"]);
    expect(h.draft()).toBe("");
    expect(h.pending.view(h.pk).items).toEqual([]);
    expect(h.sending.current).toBe(false);
    expect(h.pending.view(h.pk).walking).toBe(false);
  });
  it("with only a token, Send still submits", async () => {
    const h = harness();
    h.add(image, "");
    await h.run();
    expect(h.log).toEqual(["upload IMG", "keys enter", "announce Sent"]);
  });
  it("sends nothing until Send: placing tokens and typing make no request", () => {
    const h = harness();
    h.add(image, "see ");
    h.add(text, " ");
    h.type("words");
    expect(h.log).toEqual([]);
  });
  it("a failure keeps the failed token and all after it with their items, what went is gone, no Return", async () => {
    const h = harness({ failAt: "upload FILE.pdf" });
    h.add(image, "see ");
    h.add(file, " and ");
    h.type(" end");
    await h.run();
    expect(h.draft()).toBe("[File note.pdf] end");
    expect(h.pending.view(h.pk).items.map((i) => [i.label, i.at])).toEqual([["[File note.pdf]", 0]]);
    expect(h.pending.view(h.pk).seen).toBe(h.draft());
    expect(h.log).not.toContain("keys enter");
    expect(h.pending.view(h.pk).failure).toBe("[File note.pdf] was not sent. Not connected.");
    expect(h.sending.current).toBe(false);
    expect(h.pending.view(h.pk).walking).toBe(false);
  });
  it("a failed text keeps it, and sending again goes on from there", async () => {
    const h = harness({ failAt: 'text " end"' });
    h.add(image, "");
    h.type(" end");
    await h.run();
    expect(h.draft()).toBe(" end");
    expect(h.pending.view(h.pk).failure).toBe("The text was not sent. Not connected.");
  });
  it("a failed Return leaves the box empty and says so", async () => {
    const h = harness({ failAt: "keys enter" });
    h.add(image, "");
    await h.run();
    expect(h.draft()).toBe("");
    expect(h.pending.view(h.pk).failure).toBe("Not connected.");
  });
  it("asks once, before anything goes, also with only tokens", async () => {
    const h = harness({ auth: true });
    h.add(image, "");
    h.add(image, "");
    await h.run();
    expect(h.authenticate).toHaveBeenCalledTimes(1);
    expect(h.log[0]).toBe("upload IMG");
    const refused = harness({ auth: true, authFail: true });
    refused.add(image, "");
    await refused.run();
    expect(refused.log).toEqual([]);
    expect(refused.draft()).toBe("[Image 1]");
  });
  it("a text token too long for one paste stops the send before anything goes", async () => {
    const h = harness();
    h.add(image, "");
    h.add({ kind: "text", text: "a\n".repeat(30_000) }, "");
    await h.run();
    expect(h.log).toEqual([]);
    expect(h.setError).toHaveBeenCalledWith("That is too long to send at once.");
    expect(h.draft()).toContain("[Image 1]");
  });
  it("leaving mid-way still sends the rest and the Return: nothing is stranded in the program's input", async () => {
    const h = harness({ leaveAt: "upload IMG" });
    h.add(image, "");
    h.type(" words");
    await h.run();
    expect(h.log).toEqual(["upload IMG", 'text " words"', "keys enter", "announce Sent"]);
    expect(h.draft()).toBe("");
  });
  it("a failure with the screen gone waits for the next Composer", async () => {
    const h = harness({ leaveAt: "upload IMG", failAt: 'text " words"' });
    h.add(image, "");
    h.type(" words");
    await h.run();
    expect(h.pending.takeFailure(h.pk)).toBe("The text was not sent. Not connected.");
    expect(h.pending.takeFailure(h.pk)).toBe("");
    expect(h.draft()).toBe(" words");
  });
  it("a second Send, from a screen opened since, sends nothing while the first walks", async () => {
    const h = harness({});
    h.add(image, "");
    const first = h.run();
    const second = h.run();
    await Promise.all([first, second]);
    expect(h.log.filter((l) => l === "upload IMG")).toHaveLength(1);
    expect(h.log.filter((l) => l === "keys enter")).toHaveLength(1);
  });
  it("words typed after the draft during the walk stay for the next message", async () => {
    const h = harness({ onAct: (what, type) => { if (what === "upload IMG") type(" later"); } });
    h.add(image, "see ");
    await h.run();
    expect(h.log).toEqual(['text "see "', "upload IMG", "keys enter", "announce Sent"]);
    expect(h.draft()).toBe(" later");
  });
});

describe("the full-screen view's Paste", () => {
  const source = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
  it("sends nothing itself: no direct upload or paste is left", () => {
    expect(source).not.toContain("fleet.uploadImage");
    expect(source).not.toContain("fleet.sendPaste");
    expect(source).not.toContain("ImagePaste");
    const start = source.indexOf("const deliver = (a: Attached) => {");
    const end = source.indexOf("\n  const onPaste", start);
    const compiled = ts.transpileModule(`${source.slice(start, end)}\nexports.deliver = deliver;`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
    const pending = new PendingStore();
    const onPasted = vi.fn();
    const setError = vi.fn();
    const out = { deliver: (_: unknown) => {} };
    runInNewContext(compiled, { exports: out, pending, onPasted, setError, box: "b", running: true, canType: true, canPaste: true, programSend, PENDING_FULL: "full" });
    out.deliver({ kind: "text", text: "a\nb\nc\nd" });
    expect(pending.view("b").text).toBe("a\nb\nc\nd");
    expect(onPasted).toHaveBeenCalledTimes(1);
    out.deliver({ kind: "item", item: image });
    expect(pending.view("b").items).toHaveLength(1);
    expect(onPasted).toHaveBeenCalledTimes(2);
    out.deliver({ kind: "text", text: "x\n".repeat(30_000) });
    expect(setError).toHaveBeenCalledWith("That is too long to send at once.");
    expect(onPasted).toHaveBeenCalledTimes(2);
  });
});
