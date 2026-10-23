// Pasting an image (a screenshot) into the pane's program (feature `upload`,
// docs/remote-protocol.md): the clipboard's image as PNG (a photo too large for that as JPEG),
// base64, split into `upload` chunks the gateway reassembles; the computer saves it under a name
// of its own and pastes its path, as a file dropped on the window is, so Claude Code or Codex
// attach it.

import { MAX_UPLOAD_BYTES, MAX_UPLOAD_CHUNK, type UploadMsg } from "./protocol/types";

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
export type ClipboardImage = { ok: true; b64: string } | { ok: false; reason: "unreadable" | "too_large" } | null;

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

/** What the paste says when the clipboard's image cannot go (`clipboardImage`'s reasons). */
export function clipboardImageProblem(reason: "unreadable" | "too_large"): string {
  return reason === "unreadable"
    ? "The clipboard holds an image this phone could not read. Copy it again, or paste it on the computer."
    : `The clipboard's image is larger than ${MAX_UPLOAD_BYTES >> 20} MB, even as JPEG.`;
}

/**
 * The `upload` messages that carry the image `b64` to `pane` under the request id `id`, in order;
 * or why it cannot go (`empty`, `too_large` past MAX_UPLOAD_BYTES).
 */
export function uploadChunks(
  id: string,
  pane: number,
  b64: string,
): { ok: true; msgs: UploadMsg[]; size: number } | { ok: false; code: "empty" | "too_large" } {
  const size = base64Bytes(b64);
  if (size <= 0) return { ok: false, code: "empty" };
  if (size > MAX_UPLOAD_BYTES) return { ok: false, code: "too_large" };
  const msgs: UploadMsg[] = [];
  for (let at = 0; at < b64.length; at += UPLOAD_CHUNK_CHARS) {
    msgs.push({ t: "upload", id, pane, size, offset: (at / 4) * 3, data: b64.slice(at, at + UPLOAD_CHUNK_CHARS) });
  }
  return { ok: true, msgs, size };
}

/** What a Paste did with the clipboard (`ImagePaste.paste`). */
export type PasteOutcome =
  /** An image is still going out: this paste is ignored (the view shows it is sending). */
  | { kind: "sending" }
  /** The clipboard holds no image: its text is pasted as before. */
  | { kind: "text" }
  | { kind: "sent" }
  | { kind: "error"; message: string };

/**
 * A view's image pastes, one at a time: a second Paste while an image goes out (a large one over
 * the relay takes a while) is ignored and says so, instead of starting another upload that
 * would replace the first. `onSending` tells the view when one starts and ends.
 */
export class ImagePaste {
  private busy = false;

  constructor(private readonly onSending: (on: boolean) => void = () => {}) {}

  get sending(): boolean {
    return this.busy;
  }

  /**
   * Reads the clipboard (`read`), then sends its image (`send`) unless `refuse` names why it cannot
   * go here (a pane at its prompt). The clipboard's text is never pasted in place of an image the
   * phone could not read.
   */
  async paste(
    read: () => Promise<ClipboardImage>,
    send: (b64: string) => Promise<{ ok: true } | { ok: false; message: string }>,
    refuse: () => string | null = () => null,
  ): Promise<PasteOutcome> {
    if (this.busy) return { kind: "sending" };
    this.busy = true;
    let shown = false;
    try {
      const image = await read();
      if (image === null) return { kind: "text" };
      if (!image.ok) return { kind: "error", message: clipboardImageProblem(image.reason) };
      const why = refuse();
      if (why) return { kind: "error", message: why };
      shown = true;
      this.onSending(true);
      const r = await send(image.b64);
      return r.ok ? { kind: "sent" } : { kind: "error", message: r.message };
    } finally {
      this.busy = false;
      if (shown) this.onSending(false);
    }
  }
}

/**
 * What an upload's refusal tells the user, by the gateway's code and message (its reasons:
 * docs/remote-protocol.md › upload). `null`: the code's general message stands.
 */
export function uploadRefusal(code: string, message: string, maxMb = MAX_UPLOAD_BYTES >> 20): string | null {
  switch (code) {
    case "busy":
      if (message.includes("at its prompt")) return "An image goes to a running program, such as Claude Code; this pane is at its prompt.";
      if (message.includes("took longer")) return "The image took too long to reach the computer. Try again on a faster connection.";
      if (message.includes("other images")) return "The computer is taking other images now. Try again in a moment.";
      return message ? `The computer could not take the image: ${message}.` : null;
    case "too_large":
      return `The image is larger than ${maxMb} MB.`;
    case "bad_request":
      if (message.includes("PNG or JPEG")) return "The clipboard's image is not a PNG or JPEG.";
      if (message.includes("empty")) return "The clipboard's image is empty.";
      return "The image did not reach the computer whole. Try again.";
    default:
      return null;
  }
}
