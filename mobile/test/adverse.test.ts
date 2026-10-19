// Adverse networks at the WebSocket layer: frames delayed, dropped, duplicated and reordered
// between the gateway and the client. Noise transport messages carry an implicit counter
// nonce, so any frame out of sequence fails to decrypt: the client must drop the socket
// cleanly, reconnect, and resume the pane with `after`, never apply out-of-order data, never
// buffer without bound, and never back off beyond the cap.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { utf8Encode } from "../src/noise/bytes";
import { generateKeyPair } from "../src/noise/noise";
import {
  BACKOFF_CAP_MS,
  MAX_RETRIES,
  IDLE_TIMEOUT_MS,
  LAN_HANDSHAKE_TIMEOUT_MS,
  RELAY_HANDSHAKE_TIMEOUT_MS,
  RiftClient,
  backoffDelay,
  type ConnState,
} from "../src/protocol/client";
import { CHUNK_MAX, FLAG_MORE, MAX_GATEWAY_MESSAGE_BYTES } from "../src/protocol/framing";
import { routingId } from "../src/protocol/relay";
import type { Block } from "../src/protocol/types";
import { FakeGatewaySocket, gatewayKey, rng } from "./helpers/gateway";

const GATEWAY = gatewayKey();
const DEVICE = generateKeyPair(rng);
const welcome = { t: "welcome", v: 2, host: "studio-mac", os: "macos", version: "0.1.0", device: "d1", permission: "operate" } as const;
const block = (id: number): Block => ({
  id,
  command: "true",
  state: "done",
  exit: 0,
  duration_ms: 1,
  cwd: null,
  branch: null,
  output: "",
  dropped: 0,
  restored: false,
});
const snapshot = (seq: number) => ({ t: "snapshot", pane: 12, seq, blocks: [block(1)], has_older: false });
const update = (seq: number) => ({ t: "update", pane: 12, seq, blocks: [block(seq)], removed: [] });

function setup(opts: { addresses?: string[]; relay?: string | null; random?: () => number } = {}) {
  const sockets: FakeGatewaySocket[] = [];
  const states: ConnState[] = [];
  const applied: string[] = [];
  const client = new RiftClient({
    addresses: opts.addresses ?? ["192.168.1.29:7878"],
    relay: opts.relay ?? null,
    identity: DEVICE,
    gatewayKey: GATEWAY.publicKey,
    hello: () => ({ name: "Sam's iPhone", platform: "ios" }),
    randomBytes: rng,
    random: opts.random ?? (() => 0.5),
    socket: (url) => {
      const s = new FakeGatewaySocket(url, { key: GATEWAY });
      sockets.push(s);
      return s;
    },
    events: {
      onState: (s) => states.push(s),
      onPane: (m) => applied.push(m.t === "older" ? "older" : `${m.t}:${m.seq}`),
    },
  });
  const last = () => sockets[sockets.length - 1]!;
  /** The encrypted frames of one gateway message, in the gateway's send order (nonces advance). */
  const framesOf = (msg: unknown) => last().encryptFrames(utf8Encode(JSON.stringify(msg)));
  const deliver = (frame: Uint8Array) => last().sendRawFrame(frame.slice().buffer);
  return { client, sockets, states, applied, last, framesOf, deliver };
}

/** Open, welcomed, following pane 12 at seq 41. */
function connected(opts: Parameters<typeof setup>[0] = {}) {
  const t = setup(opts);
  t.client.start();
  t.last().connect();
  t.last().recv(welcome);
  t.client.subscribe(12);
  t.last().recv(snapshot(41));
  return t;
}

/** Reconnects after the backoff and checks the pane resumes from `after`. */
function expectResume(t: ReturnType<typeof setup>, after: number) {
  const before = t.sockets.length;
  vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
  expect(t.sockets).toHaveLength(before + 1);
  t.last().connect();
  t.last().recv(welcome);
  expect(t.last().sent[0]).toEqual({ t: "subscribe", pane: 12, after });
}

describe("adverse network: Noise rejects out-of-sequence frames", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("a duplicated frame closes the socket; nothing is applied twice; resumes with after", () => {
    const t = connected();
    const [f] = t.framesOf(update(42));
    t.deliver(f!);
    t.deliver(f!); // replayed by the network
    expect(t.applied).toEqual(["snapshot:41", "update:42"]);
    expect(t.sockets[0]!.closed).toBe(true);
    expect(t.client.connState).toMatchObject({ status: "closed", error: "decryption failed" });
    expectResume(t, 42);
  });

  it("reordered frames close the socket before anything out of order is applied", () => {
    const t = connected();
    const [f42] = t.framesOf(update(42));
    const [f43] = t.framesOf(update(43));
    t.deliver(f43!);
    t.deliver(f42!); // the old socket's handlers are gone: ignored
    expect(t.applied).toEqual(["snapshot:41"]);
    expect(t.sockets[0]!.closed).toBe(true);
    expectResume(t, 41);
  });

  it("a dropped frame makes the next one fail; resumes from the last applied seq", () => {
    const t = connected();
    const [f42] = t.framesOf(update(42));
    t.deliver(f42!);
    t.framesOf(update(43)); // lost on the way
    const [f44] = t.framesOf(update(44));
    t.deliver(f44!);
    expect(t.applied).toEqual(["snapshot:41", "update:42"]);
    expect(t.sockets[0]!.closed).toBe(true);
    expectResume(t, 42);
  });

  it("a reordered chunk inside one message is rejected the same way", () => {
    const t = connected();
    const big = { ...update(42), blocks: [{ ...block(42), output: "x".repeat(CHUNK_MAX * 2) }] };
    const frames = t.last().encryptFrames(utf8Encode(JSON.stringify(big)));
    expect(frames.length).toBeGreaterThanOrEqual(3);
    t.deliver(frames[1]!);
    expect(t.applied).toEqual(["snapshot:41"]);
    expect(t.sockets[0]!.closed).toBe(true);
  });

  it("delayed frames that keep their order are applied in order", () => {
    const t = connected();
    const frames = [42, 43, 44].map((s) => t.framesOf(update(s))[0]!);
    frames.forEach((f, i) => setTimeout(() => t.deliver(f), 1_000 * (i + 1)));
    vi.advanceTimersByTime(3_000);
    expect(t.applied).toEqual(["snapshot:41", "update:42", "update:43", "update:44"]);
    expect(t.client.connState.status).toBe("open");
  });

  it("frames delayed past the idle limit: the client gives up and resumes on a new socket", () => {
    const t = connected();
    const f = t.framesOf(update(42))[0]!;
    const s0 = t.last();
    setTimeout(() => s0.sendRawFrame(f.slice().buffer), IDLE_TIMEOUT_MS + 5_000);
    vi.advanceTimersByTime(IDLE_TIMEOUT_MS);
    expect(t.sockets[0]!.closed).toBe(true);
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(t.sockets).toHaveLength(2);
    t.last().connect();
    vi.advanceTimersByTime(5_000); // the late frame lands on the dead socket: ignored
    expect(t.applied).toEqual(["snapshot:41"]);
    t.last().recv(welcome);
    expect(t.last().sent[0]).toEqual({ t: "subscribe", pane: 12, after: 41 });
  });

  it("a frame from the previous session replayed into the new one is rejected", () => {
    const t = connected();
    const old = t.framesOf(update(42))[0]!;
    t.last().drop();
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    t.last().connect();
    t.last().recv(welcome);
    t.deliver(old); // other session keys: fails to authenticate
    expect(t.applied).toEqual(["snapshot:41"]);
    expect(t.sockets[1]!.closed).toBe(true);
  });

  it("a duplicated handshake message 2 is rejected and the attempt dropped", () => {
    const t = setup();
    t.client.start();
    const s = t.last();
    s.open();
    const queue = (s as unknown as { queue: Uint8Array[] }).queue;
    const msg2 = queue[0]!.slice();
    s.deliver();
    expect(s.handshakeDone).toBe(true);
    s.sendRawFrame(msg2.buffer); // replayed message 2 arrives as a transport frame: fails to decrypt
    expect(s.closed).toBe(true);
    expect(t.client.connState.status).toBe("closed");
  });
});

describe("adverse network: bounds", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("an endless chunk stream is cut at 1 MiB, not buffered", () => {
    const t = connected();
    const chunk = new Uint8Array(1 + CHUNK_MAX);
    chunk[0] = FLAG_MORE;
    // The gateway's own cipher, so every chunk authenticates: only the size limit can stop it.
    const cipher = (t.last() as unknown as { send_: { encrypt(p: Uint8Array): Uint8Array } }).send_;
    const limit = Math.ceil(MAX_GATEWAY_MESSAGE_BYTES / CHUNK_MAX);
    let sent = 0;
    while (!t.sockets[0]!.closed && sent < limit + 5) {
      t.deliver(cipher.encrypt(chunk));
      sent++;
    }
    expect(t.sockets[0]!.closed).toBe(true);
    expect(sent).toBe(limit);
    expect(t.client.connState.error).toBe("message too large");
  });

  it("backoff stays at or under the cap, and the client gives up after MAX_RETRIES failed rounds, LAN and relay", () => {
    const t = setup({ addresses: ["192.168.1.29:7878", "10.0.0.5:7878"], relay: "wss://relay.example.com", random: () => 1 });
    t.client.start();
    const delays: number[] = [];
    for (let round = 0; round < MAX_RETRIES; round++) {
      for (let i = 0; i < 3; i++) t.last().drop();
      const st = t.client.connState;
      expect(st.status).toBe("closed");
      expect(st.attempt).toBe(round);
      delays.push(st.retryInMs!);
      vi.advanceTimersByTime(st.retryInMs!);
    }
    expect(delays.every((d) => d <= BACKOFF_CAP_MS)).toBe(true);
    expect(delays).toEqual([0, 1, 2, 3, 4].map((n) => backoffDelay(n, () => 1)));
    // the round after the last retry ends the client: nothing is scheduled
    for (let i = 0; i < 3; i++) t.last().drop();
    expect(t.client.connState).toMatchObject({ status: "failed", failure: "gave_up", retryInMs: null, retryAt: null });
    // Three paths per round, one socket each, the first round and each retry.
    expect(t.sockets).toHaveLength((MAX_RETRIES + 1) * 3);
    vi.advanceTimersByTime(60_000);
    expect(t.sockets).toHaveLength((MAX_RETRIES + 1) * 3);
  });

  it("a stalled handshake on every LAN address hands over to the relay at the deadline", () => {
    const t = setup({ addresses: ["192.168.1.29:7878", "10.0.0.5:7878"], relay: "wss://relay.example.com" });
    t.client.start();
    t.last().open();
    vi.advanceTimersByTime(LAN_HANDSHAKE_TIMEOUT_MS);
    t.last().open();
    vi.advanceTimersByTime(LAN_HANDSHAKE_TIMEOUT_MS);
    expect(t.last().url).toBe(`wss://relay.example.com/connect?id=${routingId(GATEWAY.publicKey)}`);
    expect(t.client.connState).toMatchObject({ status: "connecting", path: { kind: "relay" } });
    // The relay stalls too: its own deadline, then a backoff round.
    t.last().open();
    vi.advanceTimersByTime(RELAY_HANDSHAKE_TIMEOUT_MS);
    expect(t.client.connState).toMatchObject({ status: "closed", error: "no answer from the computer" });
  });
});
