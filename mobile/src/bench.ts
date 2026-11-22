// The transport's cost on this device's JavaScript engine: one Noise XX handshake and 1 MiB of
// application data framed and sealed the way the gateway sends a snapshot (65 000-byte chunks,
// ChaCha20-Poly1305), then opened again. Run from the hidden diagnostics screen
// (rift://diagnostics) on a real phone to decide whether a native cipher is needed.

import { generateKeyPair, XXHandshake, type RandomBytes } from "./noise/noise";
import { frameMessage, Reassembler } from "./protocol/framing";
import { PROLOGUE } from "./protocol/types";

export interface BenchResult {
  /** "Hermes" when the app runs on Hermes, else "JavaScript". */
  engine: string;
  handshakeMs: number;
  bytes: number;
  sealMs: number;
  openMs: number;
  /** MiB per second for sealing plus opening (the slower of the two dominates). */
  mibPerSecond: number;
}

// the handshake's own prologue, as the client binds it (one home: protocol/types)
const PROLOGUE_BYTES = new TextEncoder().encode(PROLOGUE);

export function runCryptoBench(random: RandomBytes, now: () => number, bytes = 1024 * 1024): BenchResult {
  const t0 = now();
  const i = new XXHandshake({ initiator: true, prologue: PROLOGUE_BYTES, s: generateKeyPair(random), random });
  const r = new XXHandshake({ initiator: false, prologue: PROLOGUE_BYTES, s: generateKeyPair(random), random });
  r.readMessage(i.writeMessage(new Uint8Array(0)));
  i.readMessage(r.writeMessage(new Uint8Array(0)));
  r.readMessage(i.writeMessage(new Uint8Array(0)));
  const handshakeMs = now() - t0;

  // Payload bytes need no secrecy (expo-crypto also caps one draw at 1024 bytes): xorshift.
  const payload = new Uint8Array(bytes);
  let x = 0x2545f491;
  for (let k = 0; k < bytes; k++) {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    payload[k] = x & 0xff;
  }
  const t1 = now();
  const sealed = frameMessage(payload).map((chunk) => r.transport.send.encrypt(chunk));
  const sealMs = now() - t1;

  const t2 = now();
  const reassembler = new Reassembler();
  let out: Uint8Array | null = null;
  for (const c of sealed) out = reassembler.push(i.transport.recv.decrypt(c)) ?? out;
  const openMs = now() - t2;

  if (!out || out.length !== payload.length || out.some((b, k) => b !== payload[k])) {
    throw new Error("the benchmark's round trip did not return the same bytes");
  }
  const engine = (globalThis as { HermesInternal?: unknown }).HermesInternal ? "Hermes" : "JavaScript";
  const total = sealMs + openMs;
  return { engine, handshakeMs, bytes, sealMs, openMs, mibPerSecond: total > 0 ? (2 * bytes) / (1024 * 1024) / (total / 1000) : Infinity };
}
