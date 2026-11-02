// Pasting an image (a screenshot) into the pane's program (feature `upload`,
// docs/remote-protocol.md): the clipboard's image as PNG (a photo too large for that as JPEG),
// base64, split into `upload` chunks the gateway reassembles; the computer saves it under a name
// of its own and pastes its path, as a file dropped on the window is, so Claude Code or Codex
// attach it. A file (feature `upload_files`) goes the same way with its extension, the only part
// of its name that leaves the phone.

import { MAX_UPLOAD_BYTES, MAX_UPLOAD_CHUNK, UPLOAD_EXT, type UploadMsg } from "./protocol/types";

/** Characters of base64 per chunk: whole quanta, so each chunk decodes alone. */
export const UPLOAD_CHUNK_CHARS = (MAX_UPLOAD_CHUNK / 3) * 4;
/** How long the computer may take to answer an upload's last chunk (a large one over the relay). */
export const UPLOAD_TIMEOUT_MS = 60_000;
/** The JPEG quality of a photo too large to go as PNG. */
export const JPEG_QUALITY = 0.8;

/** What expo-clipboard offers for images (a fake in tests). */
export interface ImageClipboard {
  hasImageAsync(): Promise<boolean>;
  getImageAsync(options: { format: "png" | "jpeg"; jpegQuality?: number }): Promise<{ data: string } | null>;
}

/** The base64 of a `data:image/…;base64,` URI (what getImageAsync gives), or of plain base64; null otherwise. */
export function imageBase64(data: string): string | null {
  const m = /^data:image\/(png|jpeg);base64,(.*)$/s.exec(data);
  const b64 = (m ? m[2]! : data).replace(/\s+/g, "");
  if (b64.length === 0 || b64.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(b64)) return null;
  return b64;
}

/** Bytes that base64 `b64` (valid, padded) decodes to. */
export function base64Bytes(b64: string): number {
  const pad = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  return (b64.length / 4) * 3 - pad;
}

/**
 * The clipboard's image: `null` when it holds none (or will not say: a refused permission is no
 * image, and a text paste goes on); else its base64, as PNG, or as JPEG when the PNG is over
 * MAX_UPLOAD_BYTES (a photo); else why not: `unreadable` (it has an image the phone could not
 * read), `too_large` (over the cap even as JPEG). Never a silent fall back to the clipboard's text.
 */
export type ClipboardImage = { ok: true; b64: string; ext?: string; name?: string } | { ok: false; reason: UploadProblem; file?: boolean } | null;

/** Why an image or a file cannot go; `empty` and `extension` are a file's. */
export type UploadProblem = "unreadable" | "too_large" | "empty" | "extension";

export async function clipboardImage(cb: ImageClipboard): Promise<ClipboardImage> {
  try {
    if (!(await cb.hasImageAsync())) return null;
  } catch {
    return null;
  }
  const read = async (format: "png" | "jpeg"): Promise<string | null> => {
    try {
      const img = await cb.getImageAsync(format === "jpeg" ? { format, jpegQuality: JPEG_QUALITY } : { format });
      return img ? imageBase64(img.data) : null;
    } catch {
      return null;
    }
  };
  const png = await read("png");
  if (png !== null && base64Bytes(png) <= MAX_UPLOAD_BYTES) return { ok: true, b64: png };
  const jpeg = await read("jpeg");
  if (jpeg !== null && base64Bytes(jpeg) <= MAX_UPLOAD_BYTES) return { ok: true, b64: jpeg };
  return { ok: false, reason: png === null && jpeg === null ? "unreadable" : "too_large" };
}

/** What expo-clipboard offers for text (a fake in tests). */
export interface TextClipboard {
  hasStringAsync(): Promise<boolean>;
  getStringAsync(): Promise<string>;
}

/** What a paste says when the phone refused to hand over the clipboard. */
export const CLIPBOARD_REFUSED = "The phone did not let Rift read the clipboard. Tap Paste and allow it, or check Rift's paste setting in the phone's Settings.";

/**
 * The clipboard's text. `refused`: it holds text, but reading gave nothing or failed (iOS answers
 * a denied paste with an empty string); `empty`: there is nothing to paste.
 */
export async function clipboardText(cb: TextClipboard): Promise<{ kind: "text"; text: string } | { kind: "empty" } | { kind: "refused" }> {
  let text: string;
  try {
    text = await cb.getStringAsync();
  } catch {
    return { kind: "refused" };
  }
  if (text !== "") return { kind: "text", text };
  try {
    return (await cb.hasStringAsync()) ? { kind: "refused" } : { kind: "empty" };
  } catch {
    return { kind: "empty" };
  }
}

/** What the paste says when the clipboard's image, or a `file`, cannot go. */
export function clipboardImageProblem(reason: UploadProblem, file = false): string {
  const mb = MAX_UPLOAD_BYTES >> 20;
  if (reason === "extension") return "A file goes with a plain extension, up to 8 letters or digits, such as .txt. This one has none.";
  if (reason === "empty") return "The file is empty.";
  if (file) return reason === "too_large" ? `The file is larger than ${mb} MB.` : "This phone could not read the file. Copy it again, or send it on the computer.";
  return reason === "unreadable"
    ? "The clipboard holds an image this phone could not read. Copy it again, or paste it on the computer."
    : `The clipboard's image is larger than ${mb} MB, even as JPEG.`;
}

/** The extension an `upload` carries for the file `name`, lower case; null when it has none that can go. */
export function fileExt(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  const ext = name.slice(dot + 1).toLowerCase();
  return UPLOAD_EXT.test(ext) ? ext : null;
}

/** What the native side reads of a file on the clipboard (modules/rift-keys). */
export interface NativeFile {
  status: "none" | "ok" | "too_large" | "unreadable";
  /** The file's bytes in base64. */
  data?: string;
  /** Its name, for the extension. */
  name?: string;
}

/** A file from the native side as something to send, or why it cannot go; null when there is none. */
export function fileAttachment(f: NativeFile): ClipboardImage {
  if (f.status === "none") return null;
  if (f.status !== "ok") return { ok: false, reason: f.status, file: true };
  const ext = fileExt(f.name ?? "");
  if (ext === null) return { ok: false, reason: "extension", file: true };
  if (f.data === "") return { ok: false, reason: "empty", file: true };
  const b64 = imageBase64(f.data ?? "");
  if (b64 === null) return { ok: false, reason: "unreadable", file: true };
  if (base64Bytes(b64) > MAX_UPLOAD_BYTES) return { ok: false, reason: "too_large", file: true };
  return { ok: true, b64, ext, name: f.name };
}

/**
 * What a Paste sends: the clipboard's image, else its file (`files`, Android: the clip's content
 * URI; iOS's files arrive through the system paste button); null: its text is pasted as before.
 */
export async function clipboardAttachment(cb: ImageClipboard, files?: { clipFile(): Promise<NativeFile> } | null): Promise<ClipboardImage> {
  const image = await clipboardImage(cb);
  if (image !== null || !files) return image;
  try {
    return fileAttachment(await files.clipFile());
  } catch {
    return { ok: false, reason: "unreadable", file: true };
  }
}

/** The base64 of a `data:…;base64,` URI of any type (what FileReader gives), or "" for none. */
export function dataUrlBase64(url: string): string {
  const m = /^data:[^,]*;base64,(.*)$/s.exec(url);
  return m ? m[1]! : "";
}

/** A picked document (expo-document-picker's fields used here). */
export interface PickedDocument {
  name: string;
  size?: number;
  uri: string;
}

/**
 * The file a picker returned, read only after its name and stated size pass; null when the user
 * cancelled. `pick` and `read` are the phone's (src/pickFile.ts) or a fake in tests.
 */
export async function pickedAttachment(
  pick: () => Promise<{ canceled: boolean; assets?: PickedDocument[] | null }>,
  read: (uri: string) => Promise<string>,
): Promise<ClipboardImage> {
  let picked: PickedDocument | undefined;
  try {
    const r = await pick();
    if (r.canceled) return null;
    picked = r.assets?.[0];
  } catch {
    return { ok: false, reason: "unreadable", file: true };
  }
  if (!picked) return null;
  if (fileExt(picked.name) === null) return { ok: false, reason: "extension", file: true };
  if ((picked.size ?? 0) > MAX_UPLOAD_BYTES) return { ok: false, reason: "too_large", file: true };
  try {
    return fileAttachment({ status: "ok", data: await read(picked.uri), name: picked.name });
  } catch {
    return { ok: false, reason: "unreadable", file: true };
  }
}

/** What a paste says about an image or file at a pane's prompt. */
export const AT_PROMPT = "An image or file goes to a running program, such as Claude Code; this pane is at its prompt.";

/**
 * The `upload` messages that carry the image, or with `ext` the file, `b64` to `pane` under the
 * request id `id`, in order; or why it cannot go (`empty`, `too_large` past MAX_UPLOAD_BYTES).
 */
export function uploadChunks(
  id: string,
  pane: number,
  b64: string,
  ext?: string,
): { ok: true; msgs: UploadMsg[]; size: number } | { ok: false; code: "empty" | "too_large" } {
  const size = base64Bytes(b64);
  if (size <= 0) return { ok: false, code: "empty" };
  if (size > MAX_UPLOAD_BYTES) return { ok: false, code: "too_large" };
  const msgs: UploadMsg[] = [];
  for (let at = 0; at < b64.length; at += UPLOAD_CHUNK_CHARS) {
    msgs.push({ t: "upload", id, pane, size, offset: (at / 4) * 3, data: b64.slice(at, at + UPLOAD_CHUNK_CHARS), ...(ext ? { ext } : {}) });
  }
  return { ok: true, msgs, size };
}

/**
 * What an upload's refusal tells the user, by the gateway's code and message (its reasons:
 * docs/remote-protocol.md › upload). `null`: the code's general message stands.
 */
export function uploadRefusal(code: string, message: string, maxMb = MAX_UPLOAD_BYTES >> 20, what: "image" | "file" = "image"): string | null {
  switch (code) {
    case "busy":
      if (message.includes("at its prompt")) return AT_PROMPT;
      if (message.includes("took longer")) return `The ${what} took too long to reach the computer. Try again on a faster connection.`;
      if (message.includes("are arriving")) return `The computer is taking other ${what}s now. Try again in a moment.`;
      return message ? `The computer could not take the ${what}: ${message}.` : null;
    case "too_large":
      return `The ${what} is larger than ${maxMb} MB.`;
    case "bad_request":
      if (message.startsWith("ext ")) return clipboardImageProblem("extension");
      if (message.includes("PNG or JPEG")) return "The clipboard's image is not a PNG or JPEG.";
      if (message.includes("empty")) return what === "file" ? "The file is empty." : "The clipboard's image is empty.";
      return `The ${what} did not reach the computer whole. Try again.`;
    default:
      return null;
  }
}
