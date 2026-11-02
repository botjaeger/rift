import { describe, expect, it } from "vitest";
import { isClientMsg, MAX_UPLOAD_BYTES, MAX_UPLOAD_CHUNK, MAX_UPLOAD_MESSAGE_BYTES, utf8Length } from "../src/protocol/types";
import {
  base64Bytes,
  clipboardImage,
  clipboardAttachment,
  clipboardImageProblem,
  clipboardText,
  dataUrlBase64,
  fileAttachment,
  fileExt,
  imageBase64,
  pickedAttachment,
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

describe("clipboardText", () => {
  const cb = (text: string, has: boolean) => ({ getStringAsync: async () => text, hasStringAsync: async () => has });

  it("reads text", async () => {
    expect(await clipboardText(cb("a\nb", true))).toEqual({ kind: "text", text: "a\nb" });
  });
  it("says refused when the phone holds text and gave none", async () => {
    expect(await clipboardText(cb("", true))).toEqual({ kind: "refused" });
    expect(await clipboardText({ getStringAsync: () => Promise.reject(new Error("no")), hasStringAsync: async () => true })).toEqual({ kind: "refused" });
  });
  it("stays silent on a truly empty clipboard", async () => {
    expect(await clipboardText(cb("", false))).toEqual({ kind: "empty" });
  });
});

describe("files", () => {
  const over = "A".repeat(4 * Math.ceil((MAX_UPLOAD_BYTES + 1) / 3));

  it("takes only a plain extension from a name, lower case; the rest of the name stays on the phone", () => {
    expect(fileExt("notes.txt")).toBe("txt");
    expect(fileExt("Archive.TAR.GZ")).toBe("gz");
    expect(fileExt("a.abcdefgh")).toBe("abcdefgh");
    for (const bad of ["", "Makefile", ".bashrc", "a.", "a.abcdefghi", "a.t t", "a.t/x", "../x", "a.é"]) {
      expect(fileExt(bad), bad).toBeNull();
    }
  });

  it("chunks carry the extension, every one, and hold to the protocol's bounds", () => {
    const r = uploadChunks("f1", 3, b64(png(100_000)), "txt");
    if (!r.ok) throw new Error(r.code);
    expect(r.msgs.length).toBe(3);
    for (const m of r.msgs) {
      expect(m.ext).toBe("txt");
      expect(isClientMsg(m)).toBe(true);
    }
    // an image's have none
    const image = uploadChunks("i", 3, b64(png(10)));
    expect(image.ok && "ext" in image.msgs[0]!).toBe(false);
    for (const ext of ["PNG", "../x", "abcdefghi", "", 7]) {
      expect(isClientMsg({ t: "upload", id: "u", pane: 1, size: 3, offset: 0, data: "AAAA", ext }), String(ext)).toBe(false);
    }
  });

  it("turns what the native side read into something to send, or why not", () => {
    expect(fileAttachment({ status: "none" })).toBeNull();
    expect(fileAttachment({ status: "ok", name: "a.TXT", data: "aGk=" })).toEqual({ ok: true, b64: "aGk=", ext: "txt", name: "a.TXT" });
    expect(fileAttachment({ status: "too_large" })).toEqual({ ok: false, reason: "too_large", file: true });
    expect(fileAttachment({ status: "unreadable" })).toEqual({ ok: false, reason: "unreadable", file: true });
    expect(fileAttachment({ status: "ok", name: "Makefile", data: "aGk=" })).toEqual({ ok: false, reason: "extension", file: true });
    expect(fileAttachment({ status: "ok", name: "a.txt", data: "" })).toEqual({ ok: false, reason: "empty", file: true });
    expect(fileAttachment({ status: "ok", name: "a.txt", data: "not base64!" })).toEqual({ ok: false, reason: "unreadable", file: true });
    expect(fileAttachment({ status: "ok", name: "a.txt", data: over })).toEqual({ ok: false, reason: "too_large", file: true });
    for (const reason of ["too_large", "unreadable"] as const) expect(clipboardImageProblem(reason, true)).toMatch(/file/i);
    expect(clipboardImageProblem("extension")).toMatch(/extension/);
  });

  const noImage: ImageClipboard = { hasImageAsync: async () => false, getImageAsync: async () => null };

  it("a Paste takes the image first, then the clip's file, else leaves it to the text", async () => {
    const file = { clipFile: async () => ({ status: "ok" as const, name: "a.pdf", data: "aGk=" }) };
    expect(await clipboardAttachment(noImage, file)).toEqual({ ok: true, b64: "aGk=", ext: "pdf", name: "a.pdf" });
    expect(await clipboardAttachment(noImage, { clipFile: async () => ({ status: "none" as const }) })).toBeNull();
    expect(await clipboardAttachment(noImage, null)).toBeNull();
    expect(await clipboardAttachment(noImage, { clipFile: () => Promise.reject(new Error("no")) })).toEqual({ ok: false, reason: "unreadable", file: true });
    const image: ImageClipboard = { hasImageAsync: async () => true, getImageAsync: async () => ({ data: "data:image/png;base64,iVBORw0KGgo=" }) };
    expect(await clipboardAttachment(image, { clipFile: () => Promise.reject(new Error("must not ask")) })).toEqual({ ok: true, b64: "iVBORw0KGgo=" });
  });

  it("a picked file is read only after its name and size pass", async () => {
    const reads: string[] = [];
    const read = async (uri: string) => {
      reads.push(uri);
      return "aGk=";
    };
    const one = (name: string, size?: number) => async () => ({ canceled: false, assets: [{ name, uri: `file:///${name}`, ...(size === undefined ? {} : { size }) }] });
    expect(await pickedAttachment(async () => ({ canceled: true, assets: null }), read)).toBeNull();
    expect(await pickedAttachment(one("Makefile"), read)).toEqual({ ok: false, reason: "extension", file: true });
    expect(await pickedAttachment(one("a.bin", MAX_UPLOAD_BYTES + 1), read)).toEqual({ ok: false, reason: "too_large", file: true });
    expect(reads).toEqual([]);
    expect(await pickedAttachment(one("a.txt", 2), read)).toEqual({ ok: true, b64: "aGk=", ext: "txt", name: "a.txt" });
    expect(reads).toEqual(["file:///a.txt"]);
    expect(await pickedAttachment(one("a.txt"), () => Promise.reject(new Error("no")))).toEqual({ ok: false, reason: "unreadable", file: true });
    expect(await pickedAttachment(() => Promise.reject(new Error("no")), read)).toEqual({ ok: false, reason: "unreadable", file: true });
    expect(dataUrlBase64("data:application/pdf;base64,aGk=")).toBe("aGk=");
    expect(dataUrlBase64("nonsense")).toBe("");
  });

  it("refusals name a file, and a bad extension says so", () => {
    expect(uploadRefusal("too_large", "the file is larger than 10 MiB", undefined, "file")).toBe("The file is larger than 10 MB.");
    expect(uploadRefusal("busy", "pane 1 is at its prompt; a file goes to a running program", undefined, "file")).toContain("at its prompt");
    expect(uploadRefusal("busy", "other files are arriving; try again in a moment", undefined, "file")).toContain("other files");
    expect(uploadRefusal("bad_request", "ext holds 1 to 8 of a-z and 0-9", undefined, "file")).toContain("extension");
    expect(uploadRefusal("bad_request", "the file is empty", undefined, "file")).toBe("The file is empty.");
  });
});
