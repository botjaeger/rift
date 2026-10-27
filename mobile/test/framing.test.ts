import { describe, expect, it } from "vitest";
import { generateKeyPair, XXHandshake } from "../src/noise/noise";
import { utf8Encode } from "../src/noise/bytes";
import { CHUNK_MAX, FLAG_LAST, FLAG_MORE, FramingError, MAX_GATEWAY_MESSAGE_BYTES, Reassembler, frameMessage } from "../src/protocol/framing";
import { rng, sameBytes } from "./helpers/gateway";

const bytes = (n: number) => {
  const b = new Uint8Array(n);
  for (let i = 0; i < n; i++) b[i] = i % 251;
  return b;
};

describe("frameMessage", () => {
  it("puts a message of exactly 65 000 bytes in one last chunk", () => {
    const chunks = frameMessage(bytes(CHUNK_MAX));
    expect(chunks).toHaveLength(1);
    expect(chunks[0]![0]).toBe(FLAG_LAST);
    expect(chunks[0]!.length).toBe(CHUNK_MAX + 1);
  });

  it("splits 65 001 bytes into 65 000 (more) + 1 (last)", () => {
    const chunks = frameMessage(bytes(CHUNK_MAX + 1));
    expect(chunks.map((c) => [c[0], c.length - 1])).toEqual([
      [FLAG_MORE, CHUNK_MAX],
      [FLAG_LAST, 1],
    ]);
  });

  it("frames an empty message as one empty last chunk", () => {
    expect(frameMessage(new Uint8Array(0))).toEqual([Uint8Array.of(FLAG_LAST)]);
  });

  it("frames 1 MiB in 17 chunks; each fits a Noise message with its tag", () => {
    const chunks = frameMessage(bytes(1024 * 1024));
    expect(chunks).toHaveLength(Math.ceil((1024 * 1024) / CHUNK_MAX));
    expect(chunks.every((c, i) => c[0] === (i === chunks.length - 1 ? FLAG_LAST : FLAG_MORE))).toBe(true);
    expect(Math.max(...chunks.map((c) => c.length)) + 16).toBeLessThanOrEqual(65535);
  });
});

describe("Reassembler", () => {
  it("reassembles a 1 MiB message sent through a Noise channel", () => {
    const gw = generateKeyPair(rng);
    const p = utf8Encode("rift-remote/2");
    const ini = new XXHandshake({ initiator: true, prologue: p, s: generateKeyPair(rng), random: rng, pinnedRemoteStatic: gw.publicKey });
    const res = new XXHandshake({ initiator: false, prologue: p, s: gw, random: rng });
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    ini.readMessage(res.writeMessage(new Uint8Array(0)));
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    const message = bytes(MAX_GATEWAY_MESSAGE_BYTES);
    const r = new Reassembler();
    const outs = frameMessage(message).map((c) => r.push(ini.transport.recv.decrypt(res.transport.send.encrypt(c))));
    expect(outs.slice(0, -1).every((o) => o === null)).toBe(true);
    expect(sameBytes(outs.at(-1), message)).toBe(true);
  });

  it("handles messages back to back and returns each whole", () => {
    const r = new Reassembler();
    const a = bytes(CHUNK_MAX * 2 + 5);
    const b = bytes(3);
    const got = [...frameMessage(a), ...frameMessage(b)].map((c) => r.push(c)).filter((m) => m !== null);
    expect(got).toEqual([a, b]);
  });

  it("rejects a bad flag, an empty chunk, an oversized chunk, and a message over 1 MiB", () => {
    expect(() => new Reassembler().push(Uint8Array.of(2, 0x7b))).toThrow(FramingError);
    expect(() => new Reassembler().push(new Uint8Array(0))).toThrow(FramingError);
    expect(() => new Reassembler().push(new Uint8Array(CHUNK_MAX + 2))).toThrow("chunk too large");
    const r = new Reassembler();
    const big = frameMessage(bytes(MAX_GATEWAY_MESSAGE_BYTES + 1));
    expect(() => big.forEach((c) => r.push(c))).toThrow("message too large");
  });
});
