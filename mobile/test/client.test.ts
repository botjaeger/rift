import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { generateKeyPair } from "../src/noise/noise";
import {
  BACKOFF_CAP_MS,
  HANDSHAKE_TIMEOUT_MS,
  IDLE_TIMEOUT_MS,
  PING_INTERVAL_MS,
  RiftClient,
  backoffDelay,
  type ConnState,
} from "../src/protocol/client";
import type { Block } from "../src/protocol/types";
import { FakeGatewaySocket, gatewayKey, rng } from "./helpers/gateway";

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
const welcome = {
  t: "welcome",
  v: 2,
  host: "studio-mac",
  os: "macos",
  version: "0.1.0",
  device: "d3f1",
  permission: "operate",
} as const;
const snapshot = (seq: number) => ({ t: "snapshot", pane: 12, seq, blocks: [block(1)], has_older: false }) as const;
const update = (seq: number) => ({ t: "update", pane: 12, seq, blocks: [block(seq)], removed: [] }) as const;

const GATEWAY = gatewayKey();
const DEVICE = generateKeyPair(rng);

function setup(
  opts: { addresses?: string[]; relay?: string; retry?: boolean; pinned?: Uint8Array; gatewayFor?: (url: string) => typeof GATEWAY } = {},
) {
  const sockets: FakeGatewaySocket[] = [];
  const states: ConnState[] = [];
  const applied: string[] = [];
  const acks: unknown[] = [];
  const done: unknown[] = [];
  const client = new RiftClient({
    addresses: opts.addresses ?? ["127.0.0.1:7878"],
    relay: opts.relay ?? null,
    identity: DEVICE,
    gatewayKey: opts.pinned ?? GATEWAY.publicKey,
    hello: () => ({ name: "Sam's iPhone", platform: "ios" }),
    randomBytes: rng,
    random: () => 0.5,
    retry: opts.retry,
    socket: (url) => {
      const s = new FakeGatewaySocket(url, { key: opts.gatewayFor?.(url) ?? GATEWAY });
      sockets.push(s);
      return s;
    },
    events: {
      onState: (s) => states.push(s),
      onPane: (m) => applied.push(m.t === "older" ? "older" : `${m.t}:${m.seq}`),
      onAck: (m) => acks.push(m),
      onDone: (m) => done.push(m),
    },
  });
  const last = () => sockets[sockets.length - 1]!;
  return { client, sockets, states, applied, acks, done, last };
}

/** Connects, authenticates and subscribes pane 12 with a snapshot at `seq`. */
function connected(seq = 41) {
  const t = setup();
  t.client.start();
  t.last().connect();
  t.last().recv(welcome);
  t.client.subscribe(12);
  t.last().recv(snapshot(seq));
  return t;
}

describe("RiftClient handshake", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("runs Noise XX over binary frames, sends the hello in message 3, then reports open on welcome", () => {
    const { client, last, states } = setup();
    client.start();
    expect(last().url).toBe("ws://127.0.0.1:7878");
    expect(last().binaryType).toBe("arraybuffer");
    expect(states.at(-1)!.status).toBe("connecting");
    last().connect();
    expect(last().frames).toHaveLength(2); // message 1 and message 3
    expect(last().frames[0]).toHaveLength(32); // -> e, empty payload
    expect(last().clientHello).toEqual({ v: 2, name: "Sam's iPhone", platform: "ios" });
    expect(states.at(-1)!.status).toBe("connecting");
    last().recv(welcome);
    expect(states.at(-1)).toMatchObject({ status: "open", failure: null });
  });

  it("aborts on a gateway key that is not the pinned one: no message 3, no retry", () => {
    const impostor = gatewayKey();
    const { client, last, sockets, states } = setup({ gatewayFor: () => impostor });
    client.start();
    last().open();
    last().deliver();
    expect(last().frames).toHaveLength(1); // only message 1: the hello was never sent
    expect(last().handshakeDone).toBe(false);
    expect(last().closed).toBe(true);
    expect(states.at(-1)).toMatchObject({ status: "failed", failure: "key_mismatch" });
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(1);
  });

  it("stops on a gateway hello with another protocol version", () => {
    const { client, sockets, states } = setup();
    client.start();
    const s = sockets[0]!;
    // Swap the gateway's hello for a v3 one.
    (s as unknown as { opts: { hello: unknown } }).opts.hello = { v: 3, host: "h", os: "macos", version: "9" };
    s.open();
    s.deliver();
    expect(states.at(-1)).toMatchObject({ status: "failed", failure: "version" });
  });

  it("an auth error stops reconnecting", () => {
    const { client, last, sockets, states } = setup();
    client.start();
    last().connect();
    last().recv({ t: "error", code: "auth", message: "this device is not paired" });
    expect(states.at(-1)).toMatchObject({ status: "failed", failure: "auth", error: "this device is not paired", retryInMs: null });
    expect(last().closed).toBe(true);
    last().drop();
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(1);
  });

  it("closes on a text frame and on a forged transport frame", () => {
    const a = connected();
    a.last().sendRawFrame('{"t":"pong","n":1}');
    expect(a.sockets[0]!.closed).toBe(true);
    expect(a.states.at(-1)!.status).toBe("closed");

    const b = connected();
    const forged = b.last().encryptFrames(new TextEncoder().encode('{"t":"pong","n":1}'))[0]!;
    forged[3] = forged[3]! ^ 1;
    b.last().sendRawFrame(forged.slice().buffer);
    expect(b.sockets[0]!.closed).toBe(true);
  });

  it("gives up on a handshake that does not finish in time and tries again", () => {
    const { client, sockets, last } = setup();
    client.start();
    last().open(); // the gateway never answers message 1
    vi.advanceTimersByTime(HANDSHAKE_TIMEOUT_MS - 1);
    expect(sockets[0]!.closed).toBe(false);
    vi.advanceTimersByTime(1);
    expect(sockets[0]!.closed).toBe(true);
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(sockets).toHaveLength(2);
  });
});

describe("RiftClient addresses", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("tries the addresses in order, then prefers the one that worked", () => {
    const { client, sockets, last } = setup({ addresses: ["192.168.1.29:7878", "10.0.0.5:7878"] });
    client.start();
    expect(last().url).toBe("ws://192.168.1.29:7878");
    last().drop(); // unreachable
    expect(sockets).toHaveLength(2);
    expect(last().url).toBe("ws://10.0.0.5:7878");
    last().connect();
    last().recv(welcome);
    expect(client.preferredAddress).toBe("10.0.0.5:7878");
    last().drop();
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(last().url).toBe("ws://10.0.0.5:7878");
  });

  it("without retry, fails as unreachable after one pass", () => {
    const { client, sockets, last, states } = setup({ addresses: ["192.168.1.29:7878", "10.0.0.5:7878"], retry: false });
    client.start();
    last().drop();
    last().drop();
    expect(states.at(-1)).toMatchObject({ status: "failed", failure: "unreachable" });
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(2);
  });
});

describe("RiftClient session", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("subscribes only after welcome, fresh with after=null", () => {
    const { client, last } = setup();
    client.start();
    client.subscribe(12);
    last().connect();
    expect(last().sent).toEqual([]);
    last().recv(welcome);
    expect(last().sent[0]).toEqual({ t: "subscribe", pane: 12, after: null });
  });

  it("pings every 15 s while connected", () => {
    const { last } = connected();
    const before = last().sent.length;
    for (let i = 1; i <= 3; i++) {
      vi.advanceTimersByTime(PING_INTERVAL_MS);
      last().recv({ t: "pong", n: i });
    }
    expect(last().sent.slice(before)).toEqual([
      { t: "ping", n: 1 },
      { t: "ping", n: 2 },
      { t: "ping", n: 3 },
    ]);
  });

  it("applies in-order updates and drops duplicates", () => {
    const { last, applied, client } = connected(41);
    last().recv(update(42));
    last().recv(update(43));
    last().recv(update(43));
    last().recv(update(40));
    expect(applied).toEqual(["snapshot:41", "update:42", "update:43"]);
    expect(client.appliedSeq).toBe(43);
  });

  it("decodes a message split over several chunks only once the last arrives", () => {
    const { last, applied } = connected(41);
    const big = { ...update(42), blocks: [{ ...block(42), output: "x".repeat(200_000) }] };
    last().recv(big);
    expect(applied).toEqual(["snapshot:41", "update:42"]);
    last().recv(update(43), 7); // 7-byte chunks
    expect(applied.at(-1)).toBe("update:43");
  });

  it("on a seq gap re-subscribes with after = last applied seq, once", () => {
    const { last, applied } = connected(41);
    last().recv(update(42));
    const before = last().sent.length;
    last().recv(update(44)); // 43 missing
    last().recv(update(45));
    expect(applied).toEqual(["snapshot:41", "update:42"]);
    expect(last().sent.slice(before)).toEqual([{ t: "subscribe", pane: 12, after: 42 }]);
    last().recv(update(43));
    last().recv(update(44));
    expect(applied).toEqual(["snapshot:41", "update:42", "update:43", "update:44"]);
  });

  it("ignores updates and snapshots for another pane", () => {
    const { last, applied } = connected(41);
    last().recv({ ...update(42), pane: 13 });
    last().recv({ ...snapshot(5), pane: 13 });
    expect(applied).toEqual(["snapshot:41"]);
  });

  it("reconnects with backoff and resumes: new handshake, then subscribe after = last seq", () => {
    const { last, sockets, states, applied } = connected(41);
    last().recv(update(42));
    last().drop();
    expect(states.at(-1)).toMatchObject({ status: "closed", retryInMs: backoffDelay(0, () => 0.5) });
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(sockets).toHaveLength(2);
    expect(states.at(-1)).toMatchObject({ status: "connecting", attempt: 1 });
    last().connect();
    expect(last().sent).toEqual([]);
    last().recv(welcome);
    expect(last().sent[0]).toEqual({ t: "subscribe", pane: 12, after: 42 });
    expect(states.at(-1)).toMatchObject({ status: "open", attempt: 0 });
    last().recv(update(43));
    expect(applied.at(-1)).toBe("update:43");
  });

  it("reconnectNow skips the wait", () => {
    const { client, last, sockets } = connected();
    last().drop();
    expect(sockets).toHaveLength(1);
    client.reconnectNow();
    expect(sockets).toHaveLength(2);
  });

  it("backs off exponentially from 0.5 s to a 10 s cap with jitter", () => {
    expect(backoffDelay(0, () => 0)).toBe(250);
    expect(backoffDelay(0, () => 1)).toBe(500);
    expect(backoffDelay(1, () => 1)).toBe(1000);
    expect(backoffDelay(4, () => 1)).toBe(8000);
    expect(backoffDelay(5, () => 1)).toBe(BACKOFF_CAP_MS);
    expect(backoffDelay(50, () => 1)).toBe(BACKOFF_CAP_MS);
    expect(backoffDelay(50, () => 0)).toBe(BACKOFF_CAP_MS / 2);

    const { client, sockets, last, states } = setup();
    client.start();
    const delays: number[] = [];
    for (let i = 0; i < 8; i++) {
      last().drop();
      delays.push(states.at(-1)!.retryInMs!);
      vi.advanceTimersByTime(states.at(-1)!.retryInMs!);
    }
    expect(sockets).toHaveLength(9);
    expect(delays).toEqual([375, 750, 1500, 3000, 6000, 7500, 7500, 7500]);
  });

  it("a non-auth error keeps the connection", () => {
    const { last, client } = connected();
    last().recv({ t: "error", code: "no_pane", message: "pane 12 closed" });
    expect(client.connState).toMatchObject({ status: "open", error: "pane 12 closed" });
  });

  it("reconnects when the gateway goes silent for 45 s", () => {
    const { sockets } = connected();
    vi.advanceTimersByTime(IDLE_TIMEOUT_MS - 1);
    expect(sockets).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(sockets[0]!.closed).toBe(true);
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(sockets).toHaveLength(2);
  });

  it("stop closes the socket and never reconnects", () => {
    const { client, sockets, states } = connected();
    client.stop();
    expect(sockets[0]!.closed).toBe(true);
    expect(states.at(-1)!.status).toBe("closed");
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(1);
  });

  it("ignores unknown and malformed messages", () => {
    const { last, applied, client } = connected();
    last().recv({ t: "telemetry", x: 1 });
    last().recv({ t: "update", pane: 12, seq: "42" });
    expect(applied).toEqual(["snapshot:41"]);
    expect(client.connState.status).toBe("open");
  });

  it("switching panes subscribes fresh; older requests carry the pane", () => {
    const { client, last } = connected(41);
    client.requestOlder(1);
    client.subscribe(13);
    expect(last().sent.slice(-2)).toEqual([
      { t: "older", pane: 12, before: 1 },
      { t: "subscribe", pane: 13, after: null },
    ]);
  });

  it("run, keys and forget go out with ids; acks and done come back", () => {
    const { client, last, acks, done } = connected();
    const r = client.run(12, "cargo fmt --all");
    const k = client.keys(12, ["ctrl+c"]);
    expect(r).toBe("r1");
    expect(k).toBe("k2");
    expect(client.forget()).toBe(true);
    expect(last().sent.slice(-3)).toEqual([
      { t: "run", id: "r1", pane: 12, command: "cargo fmt --all" },
      { t: "keys", id: "k2", pane: 12, keys: ["ctrl+c"] },
      { t: "forget" },
    ]);
    last().recv({ t: "ack", id: "r1", ok: false, error: "forbidden", message: "this device may only view" });
    last().recv({ t: "done", pane: 12, block: 87, command: "cargo test", exit: 0, duration_ms: 260500 });
    expect(acks).toEqual([{ t: "ack", id: "r1", ok: false, error: "forbidden", message: "this device may only view" }]);
    expect(done).toHaveLength(1);
  });

  it("refuses run and keys while not connected, and messages over 4 KiB", () => {
    const { client } = setup();
    expect(client.run(12, "ls")).toBeNull();
    const c = connected();
    expect(c.client.keys(12, Array(20).fill("up"))).not.toBeNull();
    // 2048 quotes are a valid command but escape to 4096 bytes of JSON: over the 4 KiB limit.
    expect(c.client.run(12, '"'.repeat(2048))).toBeNull();
    expect(c.last().sent.some((m) => m.t === "run")).toBe(false);
  });
});

describe("RiftClient key mismatch across paths", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const RELAY = "wss://relay.example.com";
  const lanImpostor = gatewayKey();
  /** The LAN addresses reach another machine; the relay reaches the paired one. */
  const impostorOnLan = (url: string) => (url.startsWith("ws://") ? lanImpostor : GATEWAY);
  /** Answers message 1 with the gateway's message 2, where the pinned key is checked. */
  const answer = (s: FakeGatewaySocket) => {
    s.open();
    s.deliver();
  };

  it("a LAN address with another key is skipped, never sent the hello, and the relay connects", () => {
    const { client, sockets, last, states } = setup({ addresses: ["192.168.1.29:7878"], relay: RELAY, gatewayFor: impostorOnLan });
    client.start();
    answer(last());
    expect(sockets[0]!.clientHello).toBeNull();
    expect(sockets[0]!.closed).toBe(true);
    expect(last().url).toMatch(/^wss:\/\/relay\.example\.com\/connect\?id=/);
    expect(states.some((s) => s.status === "failed")).toBe(false);
    last().connect();
    last().recv(welcome);
    expect(states.at(-1)).toMatchObject({ status: "open", failure: null, path: { kind: "relay" } });
  });

  it("the relay answering with another key fails for good", () => {
    const relayImpostor = gatewayKey();
    const { client, sockets, last, states } = setup({
      addresses: ["192.168.1.29:7878"],
      relay: RELAY,
      gatewayFor: (url) => (url.startsWith("wss://") ? relayImpostor : GATEWAY),
    });
    client.start();
    last().drop(); // the LAN address does not answer
    answer(last());
    expect(last().clientHello).toBeNull();
    expect(states.at(-1)).toMatchObject({ status: "failed", failure: "key_mismatch" });
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(2);
  });

  it("without a relay, fails only when every address answered with another key", () => {
    const { client, sockets, last, states } = setup({ addresses: ["192.168.1.29:7878", "10.0.0.5:7878"], gatewayFor: () => lanImpostor });
    client.start();
    answer(last());
    expect(states.at(-1)!.status).toBe("connecting");
    answer(last());
    expect(states.at(-1)).toMatchObject({ status: "failed", failure: "key_mismatch" });
    expect(sockets.every((s) => s.clientHello === null)).toBe(true);

    // One address with another key, one that does not answer: keep trying.
    const mixed = setup({
      addresses: ["192.168.1.29:7878", "10.0.0.5:7878"],
      gatewayFor: (url) => (url.includes("192.168") ? lanImpostor : GATEWAY),
    });
    mixed.client.start();
    answer(mixed.last());
    mixed.last().drop();
    expect(mixed.states.at(-1)).toMatchObject({ status: "closed", failure: null });
    expect(mixed.states.at(-1)!.retryInMs).not.toBeNull();
  });

  it("a network-change restart tries a key mismatch again, but not an auth failure", () => {
    const a = setup({ gatewayFor: () => lanImpostor });
    a.client.start();
    answer(a.last());
    expect(a.states.at(-1)).toMatchObject({ status: "failed", failure: "key_mismatch" });
    a.client.restart({ lanFirst: true });
    expect(a.sockets).toHaveLength(2);
    expect(a.states.at(-1)).toMatchObject({ status: "connecting", failure: null });

    const b = setup();
    b.client.start();
    b.last().connect();
    b.last().recv({ t: "error", code: "auth", message: "this device is not paired" });
    b.client.restart({ lanFirst: true });
    expect(b.sockets).toHaveLength(1);
    expect(b.states.at(-1)).toMatchObject({ status: "failed", failure: "auth" });
  });
});
