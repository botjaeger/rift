import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { bytesToHex, fromBase64Url, hexToBytes, toBase64Url, utf8Decode, utf8DecodeFallback, utf8Encode, utf8EncodeFallback } from "../src/noise/bytes";
import {
  CipherState,
  NoiseError,
  NoiseKeyMismatchError,
  PROTOCOL_NAME,
  XXHandshake,
  chachaNonce,
  generateKeyPair,
  keyPairFromPrivate,
  type TransportCiphers,
} from "../src/noise/noise";
import { frameMessage, Reassembler } from "../src/protocol/framing";
import { rng, sameBytes } from "./helpers/gateway";

interface Vector {
  protocol_name: string;
  init_prologue: string;
  init_static: string;
  init_ephemeral: string;
  resp_prologue: string;
  resp_static: string;
  resp_ephemeral: string;
  handshake_hash: string;
  messages: { payload: string; ciphertext: string }[];
}

const vectors = (
  JSON.parse(readFileSync(fileURLToPath(new URL("./vectors/cacophony-xx.json", import.meta.url)), "utf8")) as { vectors: Vector[] }
).vectors;

const noRandom = () => {
  throw new Error("a fixed ephemeral key must be used");
};

interface Transcript {
  prologue: Uint8Array;
  initStatic: Uint8Array;
  initEphemeral: Uint8Array;
  respStatic: Uint8Array;
  respEphemeral: Uint8Array;
  handshakeHash: Uint8Array;
  messages: { payload: Uint8Array; ciphertext: Uint8Array }[];
}

/**
 * Replays a transcript with both roles: each handshake message must come out byte for byte, and
 * the other side must read the payload back. Transport messages may go either way; the sender is
 * the side whose cipher reproduces the ciphertext. Returns the number of messages checked.
 */
function replay(t: Transcript, pin = true): number {
  const ini = new XXHandshake({
    initiator: true,
    prologue: t.prologue,
    s: keyPairFromPrivate(t.initStatic),
    e: keyPairFromPrivate(t.initEphemeral),
    pinnedRemoteStatic: pin ? keyPairFromPrivate(t.respStatic).publicKey : undefined,
    random: noRandom,
  });
  const res = new XXHandshake({
    initiator: false,
    prologue: t.prologue,
    s: keyPairFromPrivate(t.respStatic),
    e: keyPairFromPrivate(t.respEphemeral),
    random: noRandom,
  });
  let i = 0;
  for (; i < 3; i++) {
    const m = t.messages[i]!;
    const [w, r] = i % 2 === 0 ? [ini, res] : [res, ini];
    expect(bytesToHex(w.writeMessage(m.payload)), `handshake message ${i}`).toBe(bytesToHex(m.ciphertext));
    expect(bytesToHex(r.readMessage(m.ciphertext))).toBe(bytesToHex(m.payload));
  }
  expect(ini.isComplete && res.isComplete).toBe(true);
  expect(bytesToHex(ini.transport.handshakeHash)).toBe(bytesToHex(t.handshakeHash));
  expect(bytesToHex(res.transport.handshakeHash)).toBe(bytesToHex(t.handshakeHash));
  expect(bytesToHex(ini.transport.remoteStatic)).toBe(bytesToHex(keyPairFromPrivate(t.respStatic).publicKey));
  expect(bytesToHex(res.transport.remoteStatic)).toBe(bytesToHex(keyPairFromPrivate(t.initStatic).publicKey));
  const sides: [TransportCiphers, TransportCiphers][] = [
    [ini.transport, res.transport],
    [res.transport, ini.transport],
  ];
  for (; i < t.messages.length; i++) {
    const m = t.messages[i]!;
    const hit = sides.find(([from]) => bytesToHex(from.send.clone().encrypt(m.payload)) === bytesToHex(m.ciphertext));
    expect(hit, `transport message ${i} matches neither direction`).toBeDefined();
    const [from, to] = hit!;
    expect(bytesToHex(from.send.encrypt(m.payload))).toBe(bytesToHex(m.ciphertext));
    expect(bytesToHex(to.recv.decrypt(m.ciphertext))).toBe(bytesToHex(m.payload));
  }
  return t.messages.length;
}

describe("Noise_XX_25519_ChaChaPoly_BLAKE2s: cacophony vectors", () => {
  it("has the XX ChaChaPoly BLAKE2s vectors", () => {
    expect(vectors.length).toBeGreaterThan(0);
    for (const v of vectors) expect(v.protocol_name).toBe(PROTOCOL_NAME);
  });

  it.each(vectors.map((v, i) => [i, v] as const))("vector %i replays byte for byte", (_i, v) => {
    expect(v.init_prologue).toBe(v.resp_prologue);
    const n = replay({
      prologue: hexToBytes(v.init_prologue),
      initStatic: hexToBytes(v.init_static),
      initEphemeral: hexToBytes(v.init_ephemeral),
      respStatic: hexToBytes(v.resp_static),
      respEphemeral: hexToBytes(v.resp_ephemeral),
      handshakeHash: hexToBytes(v.handshake_hash),
      messages: v.messages.map((m) => ({ payload: hexToBytes(m.payload), ciphertext: hexToBytes(m.ciphertext) })),
    });
    expect(n).toBe(v.messages.length);
  });
});

const interopPath = fileURLToPath(new URL("../../crates/rift-remote/fixtures/noise/xx.json", import.meta.url));
const haveInterop = existsSync(interopPath);
if (!haveInterop) {
  console.warn(`noise interop: ${interopPath} is absent (written by the Rust side with snow); skipping the replay`);
}

describe.skipIf(!haveInterop)("snow interop transcript (fixtures/noise/xx.json)", () => {
  it("replays the Rust (snow) transcript byte for byte", () => {
    const x = JSON.parse(readFileSync(interopPath, "utf8")) as {
      protocol: string;
      prologue_hex: string;
      initiator_static_hex: string;
      initiator_ephemeral_hex: string;
      responder_static_hex: string;
      responder_ephemeral_hex: string;
      messages: { payload_hex: string; ciphertext_hex: string }[];
      handshake_hash_hex: string;
    };
    expect(x.protocol).toBe(PROTOCOL_NAME);
    expect(utf8Decode(hexToBytes(x.prologue_hex))).toBe("rift-remote/2");
    expect(x.messages.length).toBe(7); // 3 handshake + 2 transport each way
    const n = replay({
      prologue: hexToBytes(x.prologue_hex),
      initStatic: hexToBytes(x.initiator_static_hex),
      initEphemeral: hexToBytes(x.initiator_ephemeral_hex),
      respStatic: hexToBytes(x.responder_static_hex),
      respEphemeral: hexToBytes(x.responder_ephemeral_hex),
      handshakeHash: hexToBytes(x.handshake_hash_hex),
      messages: x.messages.map((m) => ({ payload: hexToBytes(m.payload_hex), ciphertext: hexToBytes(m.ciphertext_hex) })),
    });
    expect(n).toBe(7);
  });
});

function pair(pinned?: Uint8Array) {
  const gw = generateKeyPair(rng);
  const dev = generateKeyPair(rng);
  const prologue = utf8Encode("rift-remote/2");
  const ini = new XXHandshake({ initiator: true, prologue, s: dev, random: rng, pinnedRemoteStatic: pinned ?? gw.publicKey });
  const res = new XXHandshake({ initiator: false, prologue, s: gw, random: rng });
  return { gw, dev, ini, res };
}

describe("XXHandshake", () => {
  it("completes with random keys and carries payloads", () => {
    const { ini, res, dev } = pair();
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    expect(utf8Decode(ini.readMessage(res.writeMessage(utf8Encode("gateway hello"))))).toBe("gateway hello");
    expect(utf8Decode(res.readMessage(ini.writeMessage(utf8Encode("client hello"))))).toBe("client hello");
    expect(bytesToHex(res.transport.remoteStatic)).toBe(bytesToHex(dev.publicKey));
    expect(utf8Decode(res.transport.recv.decrypt(ini.transport.send.encrypt(utf8Encode("ping"))))).toBe("ping");
    expect(utf8Decode(ini.transport.recv.decrypt(res.transport.send.encrypt(utf8Encode("pong"))))).toBe("pong");
  });

  it("aborts when the responder's static key is not the pinned one", () => {
    const other = generateKeyPair(rng).publicKey;
    const { ini, res } = pair(other);
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    const msg2 = res.writeMessage(utf8Encode("gateway hello"));
    expect(() => ini.readMessage(msg2)).toThrow(NoiseKeyMismatchError);
    // The handshake is dead: no message 3 can be produced.
    expect(() => ini.writeMessage(new Uint8Array(0))).toThrow(NoiseError);
    expect(ini.isComplete).toBe(false);
  });

  it("says whose key answered only when the responder proved it holds that key", () => {
    // a computer paired again (another key) answers: its key, proven
    const other = generateKeyPair(rng).publicKey;
    const { gw, ini, res } = pair(other);
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    let err: unknown = null;
    try {
      ini.readMessage(res.writeMessage(utf8Encode("gateway hello")));
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(NoiseKeyMismatchError);
    expect(bytesToHex((err as NoiseKeyMismatchError).proven!)).toBe(bytesToHex(gw.publicKey));

    // one that only claims a key (sends another computer's public key with its own private one)
    const claimed = generateKeyPair(rng).publicKey;
    const own = generateKeyPair(rng);
    const prologue = utf8Encode("rift-remote/2");
    const dev = generateKeyPair(rng);
    const ini2 = new XXHandshake({ initiator: true, prologue, s: dev, random: rng, pinnedRemoteStatic: other });
    const liar = new XXHandshake({ initiator: false, prologue, s: { publicKey: claimed, privateKey: own.privateKey }, random: rng });
    liar.readMessage(ini2.writeMessage(new Uint8Array(0)));
    let err2: unknown = null;
    try {
      ini2.readMessage(liar.writeMessage(utf8Encode("gateway hello")));
    } catch (e) {
      err2 = e;
    }
    expect(err2).toBeInstanceOf(NoiseKeyMismatchError);
    expect((err2 as NoiseKeyMismatchError).proven).toBeNull();
    expect(() => ini2.writeMessage(new Uint8Array(0))).toThrow(NoiseError);
  });

  it("rejects a tampered handshake message and a tampered transport message", () => {
    const { ini, res } = pair();
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    const msg2 = res.writeMessage(new Uint8Array(0));
    msg2[40] = msg2[40]! ^ 1;
    expect(() => ini.readMessage(msg2)).toThrow(NoiseError);

    const ok = pair();
    ok.res.readMessage(ok.ini.writeMessage(new Uint8Array(0)));
    ok.ini.readMessage(ok.res.writeMessage(new Uint8Array(0)));
    ok.res.readMessage(ok.ini.writeMessage(new Uint8Array(0)));
    const ct = ok.ini.transport.send.encrypt(utf8Encode("hello"));
    const bad = ct.slice();
    bad[0] = bad[0]! ^ 1;
    expect(() => ok.res.transport.recv.decrypt(bad)).toThrow("decryption failed");
    // A failed decrypt does not advance the nonce: the genuine message still opens.
    expect(utf8Decode(ok.res.transport.recv.decrypt(ct))).toBe("hello");
  });

  it("rejects messages out of turn and a different prologue", () => {
    const { ini, res } = pair();
    expect(() => ini.readMessage(new Uint8Array(32))).toThrow("not our turn");
    expect(() => res.writeMessage(new Uint8Array(0))).toThrow("not our turn");

    const gw = generateKeyPair(rng);
    const a = new XXHandshake({ initiator: true, prologue: utf8Encode("rift-remote/2"), s: generateKeyPair(rng), random: rng, pinnedRemoteStatic: gw.publicKey });
    const b = new XXHandshake({ initiator: false, prologue: utf8Encode("rift-remote/1"), s: gw, random: rng });
    b.readMessage(a.writeMessage(new Uint8Array(0)));
    expect(() => a.readMessage(b.writeMessage(new Uint8Array(0)))).toThrow(NoiseError);
  });

  it("rejects a short message 2", () => {
    const { ini, res } = pair();
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    expect(() => ini.readMessage(res.writeMessage(new Uint8Array(0)).subarray(0, 40))).toThrow(NoiseError);
  });
});

describe("CipherState", () => {
  it("encodes the nonce as 4 zero bytes + 64-bit little-endian counter", () => {
    expect(bytesToHex(chachaNonce(0))).toBe("000000000000000000000000");
    expect(bytesToHex(chachaNonce(1))).toBe("000000000100000000000000");
    expect(bytesToHex(chachaNonce(0x0102_0304))).toBe("000000000403020100000000");
    expect(bytesToHex(chachaNonce(2 ** 32))).toBe("000000000000000001000000");
    expect(bytesToHex(chachaNonce(Number.MAX_SAFE_INTEGER))).toBe("00000000ffffffffffff1f00");
  });

  it("passes plaintext through without a key and counts nonces with one", () => {
    const c = new CipherState();
    expect(c.encryptWithAd(new Uint8Array(0), utf8Encode("x"))).toEqual(utf8Encode("x"));
    c.initializeKey(new Uint8Array(32));
    c.encrypt(new Uint8Array(1));
    c.encrypt(new Uint8Array(1));
    expect(c.nonce).toBe(2);
  });
});

describe("bytes", () => {
  it("base64url round-trips and is strict", () => {
    for (let n = 0; n < 40; n++) {
      const b = rng(n);
      expect(fromBase64Url(toBase64Url(b))).toEqual(b);
    }
    expect(fromBase64Url("3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")?.length).toBe(32);
    expect(fromBase64Url("ab+c")).toBeNull();
    expect(fromBase64Url("ab=")).toBeNull();
    expect(fromBase64Url("a")).toBeNull();
    expect(fromBase64Url("AB")).toBeNull(); // non-zero padding bits
  });

  it("UTF-8 fallbacks match TextEncoder/TextDecoder", () => {
    const text = "héllo ✓ 𝄞 \u0000 end";
    expect(utf8EncodeFallback(text)).toEqual(new TextEncoder().encode(text));
    expect(utf8DecodeFallback(new TextEncoder().encode(text))).toBe(text);
    const bad = Uint8Array.of(0x61, 0xc3, 0x28, 0xe2, 0x82, 0x62, 0xff);
    expect(utf8DecodeFallback(bad)).toBe(new TextDecoder().decode(bad));
  });
});

describe("throughput", () => {
  it("encrypts and decrypts a 1 MiB message in 65 000-byte chunks", () => {
    const { ini, res } = pair();
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    ini.readMessage(res.writeMessage(new Uint8Array(0)));
    res.readMessage(ini.writeMessage(new Uint8Array(0)));
    const message = rng(1024 * 1024);
    // Warm up the JIT once, then measure.
    const run = () => {
      const reasm = new Reassembler();
      let out: Uint8Array | null = null;
      for (const chunk of frameMessage(message)) out = reasm.push(res.transport.recv.decrypt(ini.transport.send.encrypt(chunk)));
      return out;
    };
    run();
    const rounds = 5;
    const t0 = performance.now();
    let out: Uint8Array | null = null;
    for (let r = 0; r < rounds; r++) out = run();
    const ms = (performance.now() - t0) / rounds;
    expect(sameBytes(out, message)).toBe(true);
    const mbps = 1 / (ms / 1000);
    console.info(`noise throughput (node ${process.version}): 1 MiB encrypt+decrypt+reassemble in ${ms.toFixed(1)} ms = ${mbps.toFixed(1)} MiB/s`);
    expect(ms).toBeLessThan(2000);
  });
});
