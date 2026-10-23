import { describe, expect, it } from "vitest";
import { isClientMsg, MAX_UPLOAD_BYTES, MAX_UPLOAD_CHUNK, MAX_UPLOAD_MESSAGE_BYTES, utf8Length } from "../src/protocol/types";
import {
  base64Bytes,
  clipboardImage,
  clipboardImageProblem,
  ImagePaste,
  imageBase64,
  UPLOAD_CHUNK_CHARS,
  uploadChunks,
  uploadRefusal,
  type ImageClipboard,
} from "../src/upload";

const b64 = (bytes: Uint8Array) => Buffer.from(bytes).toString("base64");
/** A PNG-signed blob of `n` bytes. */
function png(n: number): Uint8Array {
  const b = new Uint8Array(n);
  b.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  for (let i = 8; i < n; i++) b[i] = i % 251;
  return b;
}

describe("upload encoding", () => {
  it("takes the base64 of a data URI or of plain base64, nothing else", () => {
    expect(imageBase64("data:image/png;base64,iVBORw0KGgo=")).toBe("iVBORw0KGgo=");
    expect(imageBase64("data:image/jpeg;base64,/9j/4AAQ")).toBe("/9j/4AAQ");
    expect(imageBase64("iVBO\nRw0K")).toBe("iVBORw0K");
    for (const bad of ["", "data:image/png;base64,", "abc", "data:text/plain;base64,aGk=", "not base64!"]) {
      expect(imageBase64(bad), bad).toBeNull();
    }
  });

  it("counts decoded bytes from the padding", () => {
    for (const n of [1, 2, 3, 4, 68, 45_000, 45_001]) expect(base64Bytes(b64(png(Math.max(8, n)).subarray(0, n)))).toBe(n);
  });

  it("splits an image into in-order chunks that decode alone and fit the upload limit", () => {
    const image = png(100_000);
    const r = uploadChunks("u7", 12, b64(image));
    if (!r.ok) throw new Error(r.code);
    expect(r.size).toBe(100_000);
    expect(r.msgs.map((m) => m.offset)).toEqual([0, MAX_UPLOAD_CHUNK, 2 * MAX_UPLOAD_CHUNK]);
    const joined = Buffer.concat(r.msgs.map((m) => Buffer.from(m.data, "base64")));
    expect(Buffer.compare(joined, Buffer.from(image))).toBe(0);
    for (const m of r.msgs) {
      expect(m).toMatchObject({ t: "upload", id: "u7", pane: 12, size: 100_000 });
      // every chunk but the last is whole: MAX_UPLOAD_CHUNK bytes, no padding inside
      expect(Buffer.from(m.data, "base64").length).toBeLessThanOrEqual(MAX_UPLOAD_CHUNK);
      expect(m.data.length).toBeLessThanOrEqual(UPLOAD_CHUNK_CHARS);
      expect(utf8Length(JSON.stringify(m))).toBeLessThanOrEqual(MAX_UPLOAD_MESSAGE_BYTES);
      expect(isClientMsg(m)).toBe(true);
    }
    expect(r.msgs[0]!.data).not.toContain("=");
  });

  it("refuses an empty image and one over the cap before anything is sent", () => {
    expect(uploadChunks("u", 1, "")).toEqual({ ok: false, code: "empty" });
    // the size comes from the base64's length: no need to build 10 MB of it to say no
    const over = "A".repeat(4 * Math.ceil((MAX_UPLOAD_BYTES + 1) / 3));
    expect(uploadChunks("u", 1, over)).toEqual({ ok: false, code: "too_large" });
    const at = "A".repeat((Math.floor(MAX_UPLOAD_BYTES / 3) * 4));
    expect(uploadChunks("u", 1, at).ok).toBe(true);
  });

  it("isClientMsg holds an upload to the protocol's bounds", () => {
    const ok = { t: "upload", id: "u1", pane: 1, size: 3, offset: 0, data: "AAAA" };
    expect(isClientMsg(ok)).toBe(true);
    for (const bad of [
      { ...ok, size: 0 },
      { ...ok, size: MAX_UPLOAD_BYTES + 1 },
      { ...ok, offset: 3 },
      { ...ok, data: "" },
      { ...ok, data: "A".repeat(UPLOAD_CHUNK_CHARS + 4) },
      { ...ok, data: "AA$A" },
      { ...ok, id: "" },
    ]) {
      expect(isClientMsg(bad), JSON.stringify(bad).slice(0, 80)).toBe(false);
    }
  });
});

describe("clipboard image", () => {
  /** A clipboard whose image reads as `png` / `jpeg` (null: cannot); `fail` refuses to say. */
  const board = (has: boolean, png: string | null, fail = false, jpeg: string | null = null): ImageClipboard & { asked: string[] } => {
    const asked: string[] = [];
    return {
      asked,
      hasImageAsync: async () => {
        if (fail) throw new Error("denied");
        return has;
      },
      getImageAsync: async (o) => {
        asked.push(o.format === "jpeg" ? `jpeg@${o.jpegQuality}` : o.format);
        const data = o.format === "png" ? png : jpeg;
        if (data === "throw") throw new Error("decode failed");
        return data === null ? null : { data };
      },
    };
  };
  const big = (bytes: number) => "A".repeat(4 * Math.ceil(bytes / 3));

  it("is the PNG's base64 when the clipboard holds an image, null when it holds none", async () => {
    const b = board(true, "data:image/png;base64,iVBORw0KGgo=");
    expect(await clipboardImage(b)).toEqual({ ok: true, b64: "iVBORw0KGgo=" });
    expect(b.asked).toEqual(["png"]);
    expect(await clipboardImage(board(false, "data:image/png;base64,iVBORw0KGgo="))).toBeNull();
    // a clipboard that refuses (permission) is no image: the text paste goes on
    expect(await clipboardImage(board(true, "x", true))).toBeNull();
  });

  it("an image the phone cannot read says so, never falling back to the text", async () => {
    expect(await clipboardImage(board(true, null))).toEqual({ ok: false, reason: "unreadable" });
    expect(await clipboardImage(board(true, "data:image/png;base64,"))).toEqual({ ok: false, reason: "unreadable" });
    expect(await clipboardImage(board(true, "throw", false, "throw"))).toEqual({ ok: false, reason: "unreadable" });
  });

  it("a photo over the cap as PNG goes as JPEG, and only if that fits", async () => {
    const b = board(true, big(MAX_UPLOAD_BYTES + 1), false, "data:image/jpeg;base64,/9j/4AAQ");
    expect(await clipboardImage(b)).toEqual({ ok: true, b64: "/9j/4AAQ" });
    expect(b.asked).toEqual(["png", "jpeg@0.8"]);
    expect(await clipboardImage(board(true, big(MAX_UPLOAD_BYTES + 1), false, big(MAX_UPLOAD_BYTES + 1)))).toEqual({
      ok: false,
      reason: "too_large",
    });
    // a PNG under the cap never asks for a JPEG
    const small = board(true, big(MAX_UPLOAD_BYTES - 2), false, "data:image/jpeg;base64,/9j/4AAQ");
    expect((await clipboardImage(small))?.ok).toBe(true);
    expect(small.asked).toEqual(["png"]);
  });
});

describe("image paste", () => {
  const sent = (b64: string) => ({ ok: true as const, b64 });

  it("ignores a second paste while an image goes out, and says it is sending", async () => {
    const states: boolean[] = [];
    const paste = new ImagePaste((on) => states.push(on));
    let finish!: (r: { ok: true }) => void;
    const sends: string[] = [];
    const send = (b64: string) => {
      sends.push(b64);
      return new Promise<{ ok: true }>((r) => (finish = r));
    };
    const first = paste.paste(async () => sent("AAAA"), send);
    await Promise.resolve();
    await Promise.resolve();
    expect(paste.sending).toBe(true);
    expect(states).toEqual([true]);
    expect(await paste.paste(async () => sent("BBBB"), send)).toEqual({ kind: "sending" });
    finish({ ok: true });
    expect(await first).toEqual({ kind: "sent" });
    expect(sends).toEqual(["AAAA"]);
    expect(states).toEqual([true, false]);
    // free again afterwards
    const again = paste.paste(async () => sent("CCCC"), async () => ({ ok: true as const }));
    expect(await again).toEqual({ kind: "sent" });
  });

  it("text when there is no image; the reason when it cannot go; nothing sent then", async () => {
    const states: boolean[] = [];
    const paste = new ImagePaste((on) => states.push(on));
    const send = async () => {
      throw new Error("must not send");
    };
    expect(await paste.paste(async () => null, send)).toEqual({ kind: "text" });
    expect(await paste.paste(async () => ({ ok: false, reason: "unreadable" }), send)).toEqual({
      kind: "error",
      message: clipboardImageProblem("unreadable"),
    });
    expect(await paste.paste(async () => sent("AAAA"), send, () => "at its prompt")).toEqual({ kind: "error", message: "at its prompt" });
    // no "sending" shown for any of them
    expect(states).toEqual([]);
    expect(await paste.paste(async () => sent("AAAA"), async () => ({ ok: false as const, message: "no" }))).toEqual({ kind: "error", message: "no" });
  });
});

describe("upload refusals", () => {
  it("are worded by the gateway's reason, not only its code", () => {
    expect(uploadRefusal("busy", "pane 12 is at its prompt; an image goes to a running program")).toContain("at its prompt");
    expect(uploadRefusal("busy", "the image took longer than 60 s to arrive; try again")).toContain("too long");
    expect(uploadRefusal("busy", "other images are arriving; try again in a moment")).toContain("other images");
    expect(uploadRefusal("busy", "cannot save the image: disk full")).toContain("disk full");
    expect(uploadRefusal("bad_request", "not a PNG or JPEG image")).toContain("not a PNG or JPEG");
    expect(uploadRefusal("bad_request", "the image is empty")).toContain("empty");
    // a chunk lost or out of order is not "not a PNG"
    for (const m of ["an upload chunk out of order", "data is not base64", "more bytes than the upload's size"]) {
      expect(uploadRefusal("bad_request", m)).toBe("The image did not reach the computer whole. Try again.");
    }
    expect(uploadRefusal("too_large", "the image is larger than 10 MiB")).toBe("The image is larger than 10 MB.");
    expect(uploadRefusal("forbidden", "this device may only view")).toBeNull();
  });
});
