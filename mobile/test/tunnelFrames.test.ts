import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { decodeFrame, encodeFrame, MAX_DATA, type Frame } from "../src/tunnel/frames";

interface Case {
  name: string;
  hex: string;
  op?: string;
  error?: string;
  stream?: number;
  port?: number;
  data?: string;
  bytes?: number;
  reason?: number;
}

const cases = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../crates/rift-remote/fixtures/tunnel/frames.json", import.meta.url)), "utf8"),
) as Case[];

const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (h) => parseInt(h, 16));
const reasons = ["refused", "limit", "error", "protocol", "aborted"] as const;

function want(c: Case): Frame {
  const stream = c.stream!;
  switch (c.op) {
    case "open":
      return { op: "open", stream, port: c.port! };
    case "opened":
      return { op: "opened", stream };
    case "data":
      return { op: "data", stream, data: new TextEncoder().encode(c.data!) };
    case "ack":
      return { op: "ack", stream, bytes: c.bytes! };
    case "end":
      return { op: "end", stream };
    case "reset":
      return { op: "reset", stream, reason: reasons[c.reason! - 1]! };
    case "ping":
      return { op: "ping" };
    case "pong":
      return { op: "pong" };
  }
  throw new Error(`${c.name}: op ${c.op}`);
}

describe("tunnel frames (shared fixtures with the computer)", () => {
  it("has the fixtures", () => {
    expect(cases.length).toBeGreaterThanOrEqual(12);
  });

  it.each(cases.filter((c) => !c.error).map((c) => [c.name, c] as const))("%s decodes and encodes back", (_, c) => {
    const bytes = hex(c.hex);
    const f = decodeFrame(bytes);
    expect(f).toEqual(want(c));
    expect(encodeFrame(f)).toEqual(bytes);
  });

  it.each(cases.filter((c) => c.error).map((c) => [c.name, c] as const))("%s is refused", (_, c) => {
    expect(() => decodeFrame(hex(c.hex))).toThrow(c.error);
  });

  it("bounds data", () => {
    const ok = new Uint8Array(5 + MAX_DATA);
    ok.set([3, 0, 0, 0, 1]);
    expect(() => decodeFrame(ok)).not.toThrow();
    const big = new Uint8Array(6 + MAX_DATA);
    big.set([3, 0, 0, 0, 1]);
    expect(() => decodeFrame(big)).toThrow("bad length");
  });
});
