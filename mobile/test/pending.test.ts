import { describe, expect, it, vi } from "vitest";
import {
  attachedItem,
  boxAlive,
  PendingStore,
  MAX_PENDING,
  nativeAttached,
  pendingOf,
  photoAttachment,
  readClipboard,
  routeRefusal,
  outParts,
  plainDraft,
  sendParts,
  type AttachRules,
  type OutPart,
  type PendingItem,
  IMAGE_LAND_BLIND_MS,
  IMAGE_LAND_MAX_MS,
  UPLOAD_GAP_MS,
  waitForImage,
} from "../src/pending";
import { AT_PROMPT, CLIPBOARD_REFUSED } from "../src/upload";
import { MAX_UPLOAD_BYTES } from "../src/protocol/types";

const PNG = "iVBORw0KGgo="; // 8 bytes
const open: AttachRules = { atPrompt: false, image: true, file: true };
const prompt: AttachRules = { ...open, atPrompt: true };

describe("a waiting item", () => {
  it("an image is what it is, a file has its own name", () => {
    expect(pendingOf({ ok: true, b64: PNG })).toEqual({ kind: "image", b64: PNG });
    expect(pendingOf({ ok: true, b64: "aGk=", ext: "pdf" }, "report.pdf")).toEqual({ kind: "file", name: "report.pdf", ext: "pdf", b64: "aGk=" });
    expect(pendingOf({ ok: true, b64: "aGk=", ext: "pdf" })).toMatchObject({ name: "File.pdf" });
  });
  it("checks a photo against the upload limit", () => {
    expect(photoAttachment(PNG)).toEqual({ ok: true, b64: PNG });
    expect(photoAttachment("not base64!")).toEqual({ ok: false, reason: "unreadable" });
    expect(photoAttachment("A".repeat((Math.floor(MAX_UPLOAD_BYTES / 3) + 2) * 4))).toEqual({ ok: false, reason: "too_large" });
  });
});

describe("what may wait", () => {
  it("refuses an image or file at a shell prompt, whatever the computer takes", () => {
    expect(routeRefusal(false, prompt)).toBe(AT_PROMPT);
    expect(routeRefusal(true, prompt)).toBe(AT_PROMPT);
  });
  it("refuses what the computer's Rift cannot take, by kind", () => {
    expect(routeRefusal(false, { ...open, image: false })).toContain("images");
    expect(routeRefusal(true, { ...open, file: false })).toContain("files");
    expect(routeRefusal(false, { ...open, file: false })).toBeNull();
    expect(routeRefusal(true, { ...open, image: false })).toBeNull();
  });
  it("turns a picked file into an item, a problem into a message, a cancel into nothing", () => {
    expect(attachedItem({ ok: true, b64: "aGk=", ext: "txt" }, open, "a.txt")).toMatchObject({ kind: "item", item: { name: "a.txt", kind: "file" } });
    expect(attachedItem({ ok: false, reason: "too_large", file: true }, open)).toMatchObject({ kind: "error" });
    expect(attachedItem(null, open)).toEqual({ kind: "none" });
    expect(attachedItem({ ok: true, b64: PNG }, prompt)).toEqual({ kind: "error", message: AT_PROMPT });
  });
});

describe("the clipboard without iOS's paste button", () => {
  const cb = (over: Partial<{ image: string | null; text: string; hasText: boolean }>) => ({
    hasImageAsync: async () => over.image != null,
    getImageAsync: async () => (over.image == null ? null : { data: `data:image/png;base64,${over.image}` }),
    hasStringAsync: async () => over.hasText ?? (over.text ?? "") !== "",
    getStringAsync: async () => over.text ?? "",
  });
  it("an image waits, it is not sent", async () => {
    expect(await readClipboard(cb({ image: PNG }), null, open)).toMatchObject({ kind: "item", item: { kind: "image" } });
  });
  it("an image at a prompt is refused, never replaced by the clipboard's text", async () => {
    expect(await readClipboard(cb({ image: PNG, text: "hello" }), null, prompt)).toEqual({ kind: "error", message: AT_PROMPT });
  });
  it("text goes to the field; with images off the clipboard's image is not looked at", async () => {
    expect(await readClipboard(cb({ text: "hello" }), null, open)).toEqual({ kind: "text", text: "hello" });
    expect(await readClipboard(cb({ image: PNG, text: "hello" }), null, { ...open, image: false })).toEqual({ kind: "text", text: "hello" });
  });
  it("says when the phone refused the clipboard or it is empty", async () => {
    expect(await readClipboard(cb({ text: "", hasText: true }), null, open)).toEqual({ kind: "error", message: CLIPBOARD_REFUSED });
    expect(await readClipboard(cb({}), null, open)).toMatchObject({ kind: "error", message: "The clipboard is empty." });
  });
  it("a file copied on Android waits under its extension", async () => {
    const files = { clipFile: async () => ({ status: "ok" as const, name: "a.pdf", data: "aGk=" }) };
    expect(await readClipboard(cb({}), files, open)).toMatchObject({ kind: "item", item: { kind: "file", ext: "pdf", name: "a.pdf" } });
  });
});

describe("what iOS's paste button handed over", () => {
  it("routes text, an image and a file", () => {
    expect(nativeAttached({ text: "hi" }, prompt)).toEqual({ kind: "text", text: "hi" });
    expect(nativeAttached({ image: PNG }, open)).toMatchObject({ kind: "item", item: { kind: "image" } });
    expect(nativeAttached({ file: { status: "ok", name: "r.pdf", data: "aGk=" } }, open)).toMatchObject({ kind: "item", item: { name: "r.pdf" } });
    expect(nativeAttached({ file: { status: "too_large" } }, open)).toMatchObject({ kind: "error" });
    expect(nativeAttached({ image: PNG }, prompt)).toEqual({ kind: "error", message: AT_PROMPT });
  });
});

describe("the waiting items of a message box", () => {
  const image = () => pendingOf({ ok: true, b64: PNG });
  const labels = (s: PendingStore, key = "a") => s.view(key).items.map((i) => i.label);
  const place = (s: PendingStore, key = "a", start = 0) => {
    let draft = s.view(key).seen;
    for (const i of s.view(key).items) if (i.at < 0) draft = s.place(key, draft, { start, end: start }, i.id, i.label).text;
    return draft;
  };
  it("labels an item by kind, per box", () => {
    const s = new PendingStore();
    s.add("a", image());
    s.add("a", { kind: "file", name: "note.pdf", ext: "pdf", b64: "aGk=" });
    s.add("a", { kind: "text", text: "1\n2\n3\n4" });
    s.add("b", image());
    expect(labels(s)).toEqual(["[Image 1]", "[File note.pdf]", "[Text 4 lines]"]);
    expect(labels(s, "b")).toEqual(["[Image 1]"]);
  });
  it("does not reuse an image number while the box holds items, and starts over when it is empty", () => {
    const s = new PendingStore();
    s.add("a", image());
    s.add("a", image());
    const draft = place(s);
    expect(draft).toBe("[Image 2][Image 1]");
    // placed at 0, Image 2 went before Image 1: delete it
    const r = s.edit("a", draft.slice(1));
    expect(r.text).toBe("[Image 1]");
    s.add("a", image());
    expect(labels(s)).toEqual(["[Image 1]", "[Image 3]"]);
    place(s, "a", 9);
    expect(s.edit("a", "").text).toBe("");
    expect(s.view("a").items).toEqual([]);
    s.add("a", image());
    expect(labels(s)).toEqual(["[Image 1]"]);
  });
  it("holds at most MAX_PENDING, text tokens counted", () => {
    const s = new PendingStore();
    for (let i = 0; i < MAX_PENDING - 1; i++) expect(s.add("a", image())).toBe(true);
    expect(s.add("a", { kind: "text", text: "a\nb\nc\nd" })).toBe(true);
    expect(s.add("a", image())).toBe(false);
    expect(s.add("a", { kind: "text", text: "a\nb\nc\nd" })).toBe(false);
    expect(s.view("a").items).toHaveLength(MAX_PENDING);
  });
  it("deleting a character of a token takes its item; the words stay", () => {
    const s = new PendingStore();
    s.add("a", image());
    const i = s.view("a").items[0]!;
    s.place("a", "see  now", { start: 4, end: 4 }, i.id, i.label);
    expect(s.view("a").seen).toBe("see [Image 1] now");
    const r = s.edit("a", "see [Image 1 now");
    expect(r).toMatchObject({ text: "see  now", gone: [i.id], cursor: 4 });
    expect(s.view("a").items).toEqual([]);
  });
  it("a draft moved by something else takes the tokens it touched", () => {
    const s = new PendingStore();
    s.add("a", image());
    place(s);
    expect(s.stale("a", "[Image 1]")).toBe(false);
    expect(s.stale("a", "ls -l")).toBe(true);
    expect(s.edit("a", "ls -l").text).toBe("ls -l");
    expect(s.view("a").items).toEqual([]);
    expect(s.stale("a", "anything")).toBe(false);
  });
  it("spending the front of the draft keeps the rest in place", () => {
    const s = new PendingStore();
    s.add("a", image());
    s.add("a", image());
    const [one, two] = s.view("a").items;
    s.place("a", "hi  x ", { start: 3, end: 3 }, one!.id, one!.label);
    s.place("a", s.view("a").seen, { start: 16, end: 16 }, two!.id, two!.label);
    expect(s.view("a").seen).toBe("hi [Image 1] x [Image 2]");
    expect(s.spend("a", s.view("a").seen, "hi ")).toBe("[Image 1] x [Image 2]");
    expect(s.spend("a", s.view("a").seen + "!", "[Image 1]")).toBe(" x [Image 2]!");
    expect(labels(s)).toEqual(["[Image 2]"]);
    expect(s.view("a").items[0]!.at).toBe(3);
  });
  it("a draft replaced during a walk is not cut", () => {
    const s = new PendingStore();
    s.add("a", image());
    const [one] = s.view("a").items;
    s.place("a", "", { start: 0, end: 0 }, one!.id, one!.label);
    expect(s.spend("a", "ls -la", "[Image 1]")).toBe("ls -la");
  });
  it("a walk holds its box open until it ends, and keep() leaves it", () => {
    const s = new PendingStore();
    expect(s.beginWalk("a")).toBe(true);
    expect(s.beginWalk("a")).toBe(false);
    s.keep(() => false);
    expect(s.view("a").walking).toBe(true);
    s.endWalk("a", "why");
    expect(s.view("a").walking).toBe(false);
    expect(s.takeFailure("a")).toBe("why");
    expect(s.view("a")).toEqual(new PendingStore().view("a"));
  });
  it("hands pasted text over once", () => {
    const s = new PendingStore();
    s.giveText("a", "one ");
    s.giveText("a", "two");
    expect(s.takeText("a")).toBe("one two");
    expect(s.takeText("a")).toBe("");
  });
  it("tells its listeners, and a view stays the same object until something changes", () => {
    const s = new PendingStore();
    const seen = vi.fn();
    const off = s.subscribe(seen);
    const before = s.view("a");
    expect(s.view("a")).toBe(before);
    s.add("a", image());
    expect(seen).toHaveBeenCalledTimes(1);
    expect(s.view("a")).not.toBe(before);
    off();
    s.clearTokens("a", "");
    expect(seen).toHaveBeenCalledTimes(1);
  });
});

describe("sending the draft", () => {
  const item = (id: number, label: string, at: number): PendingItem => ({ id, kind: "file", name: "x", b64: "aGk=", ext: "txt", label, at });
  const parts = (): OutPart[] => outParts("hi [A] then [B][T]", [item(1, "[A]", 3), item(2, "[B]", 12), { id: 3, kind: "text", text: "a\nb", label: "[T]", at: 15 }]);
  const name = (p: OutPart) => ("text" in p ? p.text : p.item.label);
  it("cuts the draft at its tokens, in place order, whatever order they were added", () => {
    expect(parts().map(name)).toEqual(["hi ", "[A]", " then ", "[B]", "[T]"]);
    expect(outParts("[B]x[A]", [item(1, "[A]", 4), item(2, "[B]", 0)]).map(name)).toEqual(["[B]", "x", "[A]"]);
  });
  it("waits after each upload, before whatever goes next", async () => {
    const order: string[] = [];
    const r = await sendParts(parts(), async (p) => { order.push(name(p)); return { ok: true }; }, () => {}, async () => void order.push("wait 1100"));
    expect(r).toEqual({ ok: true });
    expect(order).toEqual(["hi ", "[A]", "wait 1100", " then ", "[B]", "wait 1100", "[T]"]);
  });
  it("stops at the first failure, names it, and reports only what went", async () => {
    const sent: string[] = [];
    const send = vi.fn(async (p: OutPart) => (name(p) === "[B]" ? { ok: false as const, message: "The computer is taking other files now." } : { ok: true as const }));
    const r = await sendParts(parts(), send, (p) => sent.push(name(p)), async () => {});
    expect(r).toEqual({ ok: false, message: "[B] was not sent. The computer is taking other files now." });
    expect(sent).toEqual(["hi ", "[A]", " then "]);
    expect(send).toHaveBeenCalledTimes(4);
  });
  it("a failed text between tokens says it was the text", async () => {
    const r = await sendParts([{ text: "hi" }], async () => ({ ok: false, message: "Not connected." }), () => {}, async () => {});
    expect(r).toEqual({ ok: false, message: "The text was not sent. Not connected." });
  });
  it("at a prompt a text token becomes the text it stands for", () => {
    expect(plainDraft("x [T] y", [{ id: 1, kind: "text", text: "a\nb", label: "[T]", at: 2 }])).toBe("x a\nb y");
  });
});

describe("what waits goes with its pane or computer", () => {
  const host = (status: string, ids: number[], topology = true) => ({ record: { key: "h:1" }, conn: { status }, topology, windows: [{ tabs: [{ panes: ids.map((id) => ({ id })) }] }] });
  it("keeps a box while its pane exists or the computer is out of touch, not after", () => {
    expect(boxAlive([host("open", [3])], "h:1:3")).toBe(true);
    expect(boxAlive([host("open", [4])], "h:1:3")).toBe(false);
    expect(boxAlive([host("connecting", [])], "h:1:3")).toBe(true);
    expect(boxAlive([host("open", [], false)], "h:1:3")).toBe(true);
    expect(boxAlive([], "h:1:3")).toBe(false);
  });
  it("keep() drops only the boxes not alive", () => {
    const p = new PendingStore();
    p.add("a:1", { kind: "image", b64: "AA==" });
    p.add("b:1", { kind: "image", b64: "AA==" });
    p.keep((k) => k === "b:1");
    expect(p.view("a:1").items).toHaveLength(0);
    expect(p.view("b:1").items).toHaveLength(1);
  });
});

describe("waiting for a pasted image to land", () => {
  const run = async (counts: (number | null)[], before: number | null) => {
    let now = 0;
    let i = 0;
    await waitForImage(before, () => counts[Math.min(i++, counts.length - 1)]!, async (ms) => { now += ms; }, () => now);
    return now;
  };
  it("waits past the gap until the box shows one more", async () => {
    expect(await run([1, 1, 1, 2], 1)).toBe(UPLOAD_GAP_MS + 300);
  });
  it("never goes on before the gap, the image already there", async () => {
    expect(await run([2], 1)).toBe(UPLOAD_GAP_MS);
  });
  it("gives up at the limit when the box never shows it", async () => {
    expect(await run([1], 1)).toBeGreaterThanOrEqual(IMAGE_LAND_MAX_MS);
    expect(await run([1], 1)).toBeLessThan(IMAGE_LAND_MAX_MS + 200);
  });
  it("with the screen out of sight, the shorter limit", async () => {
    expect(await run([null], null)).toBeGreaterThanOrEqual(IMAGE_LAND_BLIND_MS);
    expect(await run([null], null)).toBeLessThan(IMAGE_LAND_BLIND_MS + 200);
  });
});
