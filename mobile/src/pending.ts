// What waits in a pane's message box until Send: images, files and long texts, each a token in the
// draft at its place (draftTokens.ts). The Attach sheet and the paste buttons put them here; Composer
// places them and sends the draft in order. Pure but for the one store the views share: `pending`.

import { draftParts, fileLabel, imageLabel, insertText, reconcile, textLabel, type DraftPart, type Reconciled, type Selection, placeToken, type Token } from "./draftTokens";
import { base64Bytes, AT_PROMPT, clipboardImageProblem, CLIPBOARD_REFUSED, clipboardAttachment, clipboardText, type ClipboardImage, type ImageClipboard, type NativeFile, type TextClipboard, fileAttachment, imageBase64 } from "./upload";
import { MAX_UPLOAD_BYTES } from "./protocol/types";

/** How many items wait in one message box: the computer takes them one at a time. */
export const MAX_PENDING = 4;

export const PENDING_FULL = `Send the ${MAX_PENDING} waiting items first, or remove one.`;

/** What a pasted thing is, before it has a place. An image goes without an extension, a file with its own. */
export type NewPending =
  | { kind: "image"; b64: string }
  | { kind: "file"; name: string; b64: string; ext: string }
  | { kind: "text"; text: string };

export type PendingItem = NewPending & {
  id: number;
  /** Its token's text in the draft. */
  label: string;
  /** Where its token is in the draft; -1 until Composer places it. */
  at: number;
};

/** The sendable part of a `ClipboardImage`. */
type Ok = Extract<NonNullable<ClipboardImage>, { ok: true }>;

/** An accepted image or file as an item to wait; `name` is the file's own. */
export function pendingOf(att: Ok, name?: string): NewPending {
  return att.ext !== undefined ? { kind: "file", name: name || `File.${att.ext}`, b64: att.b64, ext: att.ext } : { kind: "image", b64: att.b64 };
}

/** A photo's base64 (what the image picker gives), checked against the upload limit. */
export function photoAttachment(data: string): ClipboardImage {
  const b64 = imageBase64(data);
  if (b64 === null) return { ok: false, reason: "unreadable" };
  if (base64Bytes(b64) > MAX_UPLOAD_BYTES) return { ok: false, reason: "too_large" };
  return { ok: true, b64 };
}

/** What the computer can take, and what this pane is doing: the rules for what may wait. */
export interface AttachRules {
  /** The pane is at its prompt: there is no program to take an image or file. */
  atPrompt: boolean;
  /** Feature `upload`. */
  image: boolean;
  /** Feature `upload_files`. */
  file: boolean;
}

/** Why an image (or with `file`, a file) may not wait for this pane, or null: checked before a picker opens. */
export function routeRefusal(file: boolean, rules: AttachRules): string | null {
  if (rules.atPrompt) return AT_PROMPT;
  if (file) return rules.file ? null : "Rift on this computer cannot take files from the phone. Update it.";
  return rules.image ? null : "Rift on this computer cannot take images from the phone. Update it.";
}

/** Why `att` may not wait for this pane, or null. */
export const itemRefusal = (att: Ok, rules: AttachRules): string | null => routeRefusal(att.ext !== undefined, rules);

/** What an attach route produced: text for the field, an item to wait, a message, or nothing (cancelled). */
export type Attached =
  | { kind: "text"; text: string }
  | { kind: "item"; item: NewPending }
  | { kind: "error"; message: string }
  | { kind: "none" };

/** An image or file result (`null`: nothing chosen) as something that waits, or why not. */
export function attachedItem(att: ClipboardImage, rules: AttachRules, name?: string): Attached {
  if (att === null) return { kind: "none" };
  if (!att.ok) return { kind: "error", message: clipboardImageProblem(att.reason, att.file) };
  const why = itemRefusal(att, rules);
  return why ? { kind: "error", message: why } : { kind: "item", item: pendingOf(att, name ?? att.name) };
}

/**
 * The clipboard read without iOS's paste button: its image, else its file, else its text. An
 * image or file is only looked for where one can go (`rules.image`); the text is the field's.
 */
export async function readClipboard(
  cb: ImageClipboard & TextClipboard,
  files: { clipFile(): Promise<NativeFile> } | null,
  rules: AttachRules,
): Promise<Attached> {
  if (rules.image) {
    const att = await clipboardAttachment(cb, files);
    if (att !== null) return attachedItem(att, rules);
  }
  const text = await clipboardText(cb);
  if (text.kind === "refused") return { kind: "error", message: CLIPBOARD_REFUSED };
  if (text.kind === "empty") return { kind: "error", message: "The clipboard is empty." };
  return { kind: "text", text: text.text };
}

/** What iOS's paste button handed over, as something that waits. */
export function nativeAttached(
  paste: { text: string } | { image: string } | { file: NativeFile },
  rules: AttachRules,
): Attached {
  if ("text" in paste) return { kind: "text", text: paste.text };
  if ("image" in paste) return attachedItem(photoAttachment(paste.image), rules);
  return attachedItem(fileAttachment(paste.file), rules, paste.file.name);
}

/** The wait between two items going out: over the gateway's `UPLOAD_RATE` of one a second. */
export const UPLOAD_GAP_MS = 1100;

/** How long Claude Code may take to show a pasted image's `[Image #N]`, on a computer under load: past this, the send goes on. */
export const IMAGE_LAND_MAX_MS = 15_000;
/** The same with the pane's screen out of sight (the user left): 10 MiB took 0.9 s idle, 3 s under load. */
export const IMAGE_LAND_BLIND_MS = 4_000;
const IMAGE_LAND_POLL_MS = 100;

const wait = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));

/** The files Claude Code reads as images when their path is pasted. */
const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp"]);

/** Whether the item reaches the program as an image: a photo or screenshot, or a file that is one. */
export const isImageItem = (item: PendingItem) => item.kind === "image" || (item.kind === "file" && IMAGE_EXT.has(item.ext.toLowerCase()));

/**
 * The upload's gap, then until the input box shows more images than `before` (Claude Code's image
 * landed). `count` is null while the pane's screen is not on hand; each limit lets the send go on anyway.
 */
export async function waitForImage(
  before: number | null,
  count: () => number | null,
  pause: (ms: number) => Promise<void> = wait,
  clock: () => number = Date.now,
): Promise<void> {
  const start = clock();
  await pause(UPLOAD_GAP_MS);
  for (;;) {
    const now = count();
    if (now !== null && before !== null && now > before) return;
    if (clock() - start >= (now === null ? IMAGE_LAND_BLIND_MS : IMAGE_LAND_MAX_MS)) return;
    await pause(IMAGE_LAND_POLL_MS);
  }
}

/** A draft's part as it goes out: its text, or the item behind its token. */
export type OutPart = { text: string } | { item: PendingItem };

const tokensOf = (items: readonly PendingItem[]): Token[] => items.filter((i) => i.at >= 0).map((i) => ({ id: i.id, at: i.at, label: i.label }));

/** The draft in the order it goes out. */
export function outParts(draft: string, items: readonly PendingItem[]): OutPart[] {
  return draftParts(draft, tokensOf(items)).map((p: DraftPart) => ("text" in p ? p : { item: items.find((i) => i.id === p.token.id)! }));
}

/** The part's text in the draft. */
export const outHead = (p: OutPart) => ("text" in p ? p.text : p.item.label);

/** The draft with its text tokens written out: what a shell prompt takes in place of them. */
export function plainDraft(draft: string, items: readonly PendingItem[]): string {
  return outParts(draft, items).map((p) => ("text" in p ? p.text : p.item.kind === "text" ? p.item.text : p.item.label)).join("");
}

/**
 * Sends the draft's parts in order through `send`; `sent` is told of each that went (the view takes
 * it out of the box). The first failure stops the rest and is worded with the part's name; what has
 * not gone stays. After an upload nothing goes until `settle` is done, the Return that follows included:
 * the gateway starts one upload a second per connection and refuses the next as `rate`, and Claude Code
 * reads the pasted path's image in the background and puts it where the cursor is by then, so words sent
 * before it landed go ahead of it and a Return leaves it behind (`waitForImage`).
 */
export async function sendParts(
  parts: readonly OutPart[],
  send: (part: OutPart) => Promise<{ ok: true } | { ok: false; message: string }>,
  sent: (part: OutPart) => void,
  settle: (item: PendingItem) => Promise<void> = () => wait(UPLOAD_GAP_MS),
): Promise<{ ok: true } | { ok: false; message: string }> {
  for (const part of parts) {
    const r = await send(part);
    if (!r.ok) return { ok: false, message: `${"item" in part ? part.item.label : "The text"} was not sent. ${r.message}` };
    sent(part);
    if ("item" in part && part.item.kind !== "text") await settle(part.item);
  }
  return { ok: true };
}

/** What one pane's box holds, as the views read it. */
export interface PendingView {
  items: readonly PendingItem[];
  /** Text the Attach sheet or the screen's Paste pasted, for the field to take. */
  text: string;
  /** The draft the tokens' places belong to. */
  seen: string;
  /** The last image number given: none comes back while the box holds items. */
  images: number;
  /** A Send is walking this draft: it outlives the screen, and a new Composer on this pane must see it. */
  walking: boolean;
  /** Why the last walk stopped, until a Composer shows it: the screen that sent may be gone. */
  failure: string;
}

const EMPTY: PendingView = { items: [], text: "", seen: "", images: 0, walking: false, failure: "" };

/** The waiting items and pasted text of every pane's message box, by `draftKey`. */
export class PendingStore {
  private views = new Map<string, PendingView>();
  private listeners = new Set<() => void>();
  private next = 1;

  view(key: string): PendingView {
    return this.views.get(key) ?? EMPTY;
  }

  private set(key: string, view: PendingView) {
    if (view.items.length === 0 && view.text === "" && !view.walking && view.failure === "") this.views.delete(key);
    else this.views.set(key, view.items.length === 0 ? { ...view, images: 0, seen: "" } : view);
    for (const l of [...this.listeners]) l();
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Adds an item for Composer to place; false when MAX_PENDING already wait. */
  add(key: string, item: NewPending): boolean {
    const v = this.view(key);
    if (v.items.length >= MAX_PENDING) return false;
    const images = item.kind === "image" ? v.images + 1 : v.images;
    const label = item.kind === "image" ? imageLabel(images) : item.kind === "file" ? fileLabel(item.name) : textLabel(item.text);
    this.set(key, { ...v, images, items: [...v.items, { ...item, id: this.next++, label, at: -1 }] });
    return true;
  }

  private apply(key: string, r: Reconciled): Reconciled {
    const v = this.view(key);
    const places = new Map(r.tokens.map((t) => [t.id, t]));
    const items = v.items.filter((i) => i.at < 0 || places.has(i.id)).map((i) => {
      const t = places.get(i.id);
      return t ? { ...i, at: t.at, label: t.label } : i;
    });
    this.set(key, { ...v, items, seen: r.text });
    return r;
  }

  /** What the box holds after the draft became `next`: tokens it touched are gone, with their items. */
  edit(key: string, next: string): Reconciled {
    const v = this.view(key);
    const tokens = tokensOf(v.items);
    if (tokens.length === 0) return { text: next, tokens: [], gone: [] };
    return this.apply(key, reconcile(v.seen, next, tokens));
  }

  /** Whether the draft moved without the box knowing (a completion taken, a block's Edit). */
  stale(key: string, draft: string): boolean {
    const v = this.view(key);
    return tokensOf(v.items).length > 0 && v.seen !== draft;
  }

  /** Gives the unplaced item `id` its token at the selection of `draft`. */
  place(key: string, draft: string, sel: Selection, id: number, label: string): Reconciled {
    const tokens = tokensOf(this.view(key).items);
    const synced = tokens.length > 0 ? this.edit(key, draft).text : draft;
    return this.apply(key, placeToken(synced, tokensOf(this.view(key).items), sel, id, label));
  }

  /** Puts `text` at the selection of `draft`. */
  insert(key: string, draft: string, sel: Selection, text: string): Reconciled {
    const synced = this.edit(key, draft).text;
    const r = insertText(synced, tokensOf(this.view(key).items), sel, text);
    return tokensOf(this.view(key).items).length > 0 ? this.apply(key, r) : r;
  }

  /**
   * `draft` without `head`, its start, which went (the token's item with it): what is left, words
   * typed after it since included. A draft something else replaced meanwhile stays as it is.
   */
  spend(key: string, draft: string, head: string): string {
    const v = this.view(key);
    const text = draft.startsWith(head) ? draft.slice(head.length) : draft;
    this.apply(key, { text, tokens: tokensOf(v.items).filter((t) => t.at >= head.length).map((t) => ({ ...t, at: t.at - head.length })), gone: [] });
    return text;
  }

  /** Starts a walk of the box's draft; false when one is already on its way. */
  beginWalk(key: string): boolean {
    const v = this.view(key);
    if (v.walking) return false;
    this.set(key, { ...v, walking: true, failure: "" });
    return true;
  }

  endWalk(key: string, failure = "") {
    this.set(key, { ...this.view(key), walking: false, failure });
  }

  takeFailure(key: string): string {
    const v = this.view(key);
    if (v.failure === "") return "";
    this.set(key, { ...v, failure: "" });
    return v.failure;
  }

  /** Forgets every token; their text stays in the draft as `text`. */
  clearTokens(key: string, text: string) {
    this.apply(key, { text, tokens: [], gone: [] });
  }

  /** Text for the field to take (`takeText`). */
  giveText(key: string, text: string) {
    const v = this.view(key);
    this.set(key, { ...v, text: v.text + text });
  }

  /** Drops what waits for a box `alive` no longer knows: its bytes would stay in memory until the app quits. */
  keep(alive: (key: string) => boolean) {
    for (const key of [...this.views.keys()]) if (!alive(key) && !this.view(key).walking) this.set(key, EMPTY);
  }

  takeText(key: string): string {
    const v = this.view(key);
    if (v.text === "") return "";
    this.set(key, { ...v, text: "" });
    return v.text;
  }
}

/** Whether a box's pane may still come back: its computer is paired, and is either out of touch or still shows it. */
export function boxAlive(hosts: readonly { record: { key: string }; conn: { status: string }; topology: boolean; windows: readonly { tabs: readonly { panes: readonly { id: number }[] }[] }[] }[], box: string): boolean {
  const at = box.lastIndexOf(":");
  const host = hosts.find((h) => h.record.key === box.slice(0, at));
  if (!host) return false;
  if (host.conn.status !== "open" || !host.topology) return true;
  const pane = Number(box.slice(at + 1));
  return host.windows.some((w) => w.tabs.some((t) => t.panes.some((p) => p.id === pane)));
}

export const pending = new PendingStore();
