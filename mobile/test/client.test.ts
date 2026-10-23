import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { generateKeyPair } from "../src/noise/noise";
import {
  BACKOFF_CAP_MS,
  HANDSHAKE_TIMEOUT_MS,
  IDLE_TIMEOUT_MS,
  INPUT_RATE,
  SCREEN_RESUME_MS,
  PING_INTERVAL_MS,
  OUTPUT_TIMEOUT_MS,
  RiftClient,
  backoffDelay,
  type ConnState,
  MAX_RETRIES,
  UPDATE_GRACE_MS,
  UPDATE_RETRY_MS,
  UPLOAD_HIGH_WATER_BYTES,
  UPLOAD_LIVE_MS,
  UPLOAD_POLL_MS,
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
  const screens: { t: string }[] = [];
  const stale: number[] = [];
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
      onScreen: (m) => screens.push(m),
      onScreenStale: (pane) => stale.push(pane),
    },
  });
  const last = () => sockets[sockets.length - 1]!;
  return { client, sockets, states, applied, acks, done, screens, stale, last };
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

  it("pings once at welcome, then every 15 s while connected", () => {
    const { last } = connected();
    expect(last().sent.filter((m) => (m as { t: string }).t === "ping")).toEqual([{ t: "ping", n: 1 }]);
    const before = last().sent.length;
    for (let i = 2; i <= 4; i++) {
      vi.advanceTimersByTime(PING_INTERVAL_MS);
      last().recv({ t: "pong", n: i });
    }
    expect(last().sent.slice(before)).toEqual([
      { t: "ping", n: 2 },
      { t: "ping", n: 3 },
      { t: "ping", n: 4 },
    ]);
  });

  it("measures the path's round trip from the pong of its ping", () => {
    const t = setup();
    t.client.start();
    t.last().connect();
    t.last().recv(welcome);
    expect(t.states.at(-1)!.paths[0]).toMatchObject({ state: "ok", latencyMs: null });
    vi.advanceTimersByTime(84);
    t.last().recv({ t: "pong", n: 1 });
    expect(t.states.at(-1)!.paths[0]).toMatchObject({ state: "ok", latencyMs: 84 });
    // a pong for a ping never sent changes nothing
    vi.advanceTimersByTime(500);
    t.last().recv({ t: "pong", n: 9 });
    expect(t.states.at(-1)!.paths[0]!.latencyMs).toBe(84);
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
    for (let i = 0; i < MAX_RETRIES; i++) {
      last().drop();
      delays.push(states.at(-1)!.retryInMs!);
      vi.advanceTimersByTime(states.at(-1)!.retryInMs!);
    }
    expect(sockets).toHaveLength(MAX_RETRIES + 1);
    expect(delays).toEqual([375, 750, 1500, 3000, 6000]);
  });

  it("stops after MAX_RETRIES failed retries, and starts again at once on a restart", () => {
    const { client, sockets, last, states } = setup();
    client.start();
    for (let i = 0; i < MAX_RETRIES; i++) {
      last().drop();
      vi.advanceTimersByTime(states.at(-1)!.retryInMs!);
    }
    last().drop();
    expect(client.connState).toMatchObject({ status: "failed", failure: "gave_up", retryInMs: null, attempt: MAX_RETRIES });
    const used = sockets.length;
    vi.advanceTimersByTime(120_000);
    expect(sockets).toHaveLength(used);
    // the phone is back online, or the user taps Try again
    client.restart();
    expect(client.connState).toMatchObject({ status: "connecting", failure: null, attempt: 0 });
    expect(sockets).toHaveLength(used + 1);
  });

  it("giveUp ends the retrying at once and is a no-op once open", () => {
    const { client, sockets, last } = setup();
    client.start();
    last().drop();
    expect(client.connState.status).toBe("closed");
    client.giveUp();
    expect(client.connState).toMatchObject({ status: "failed", failure: "gave_up", retryInMs: null, retryAt: null, attempt: 0 });
    const used = sockets.length;
    vi.advanceTimersByTime(120_000);
    expect(sockets).toHaveLength(used);
    const open = connected();
    open.client.giveUp();
    expect(open.client.connState.status).toBe("open");
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

describe("RiftClient Phase 3", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const screenMsg = (full: boolean, pane = 12) => ({
    t: "screen",
    pane,
    cols: 80,
    rows: 2,
    alt: true,
    cursor: null,
    full,
    lines: full ? [[0, [["top", null, null, 0]]], [1, []]] : [[1, [["x", null, null, 0]]]],
  });
  const sentOf = (t: ReturnType<typeof setup>, type: string) => t.last().sent.filter((m) => m.t === type);

  /** `n` upload chunks for request `id` (a few bytes each: the pacing is by count and the socket's backlog). */
  const chunks = (n: number) => (id: string) =>
    Array.from({ length: n }, (_, i) => ({ t: "upload" as const, id, pane: 12, size: 3 * n, offset: 3 * i, data: "AAAA" }));

  it("paces an image's chunks: one per turn, waiting while the socket holds too much", () => {
    const t = connected();
    const id = t.client.upload(chunks(4));
    expect(id).toBe("u1");
    // the first at once, the rest on later turns: the app is never blocked for the whole image
    expect(sentOf(t, "upload")).toHaveLength(1);
    vi.advanceTimersByTime(0);
    expect(sentOf(t, "upload")).toHaveLength(2);
    // a slow link: the socket holds more than the high-water mark, so nothing more goes
    t.last().bufferedAmount = UPLOAD_HIGH_WATER_BYTES + 1;
    vi.advanceTimersByTime(0);
    vi.advanceTimersByTime(UPLOAD_POLL_MS * 10);
    expect(sentOf(t, "upload")).toHaveLength(2);
    vi.advanceTimersByTime(UPLOAD_POLL_MS * 10);
    expect(sentOf(t, "upload")).toHaveLength(2);
    // it drains: the rest follows, in order
    t.last().bufferedAmount = 0;
    vi.advanceTimersByTime(UPLOAD_POLL_MS);
    vi.advanceTimersByTime(0);
    expect(sentOf(t, "upload").map((m) => (m.t === "upload" ? m.offset : -1))).toEqual([0, 3, 6, 9]);
  });

  it("stops sending an image the gateway answered part-way (a refusal)", () => {
    const t = connected();
    t.client.upload(chunks(5));
    vi.advanceTimersByTime(0);
    t.last().recv({ t: "ack", id: "u1", ok: false, error: "busy", message: "pane 12 is at its prompt" });
    vi.advanceTimersByTime(1000);
    expect(sentOf(t, "upload")).toHaveLength(2);
  });

  it("an image going out keeps the connection alive past the silence timeout, until its ack", () => {
    const t = connected();
    t.client.upload(chunks(2));
    vi.advanceTimersByTime(0);
    // the gateway's pong waits behind the image on a slow link: no message for 50 s
    vi.advanceTimersByTime(IDLE_TIMEOUT_MS + 5_000);
    expect(t.sockets).toHaveLength(1);
    expect(t.sockets[0]!.closed).toBe(false);
    expect(t.client.connState.status).toBe("open");
    // its ack: the ordinary silence rule applies again
    t.last().recv({ t: "ack", id: "u1", ok: true });
    vi.advanceTimersByTime(IDLE_TIMEOUT_MS);
    expect(t.sockets[0]!.closed).toBe(true);
  });

  it("an image never keeps a dead connection past the upload's own limit", () => {
    const t = connected();
    t.client.upload(chunks(2));
    vi.advanceTimersByTime(UPLOAD_LIVE_MS + 1);
    expect(t.sockets[0]!.closed).toBe(true);
  });

  it("sends the screen mode for the subscribed pane, once per change", () => {
    const t = connected();
    expect(t.client.setScreen(12, "auto")).toBe(true);
    expect(t.client.setScreen(12, "auto")).toBe(true);
    expect(sentOf(t, "screen")).toEqual([{ t: "screen", pane: 12, mode: "auto" }]);
    expect(t.client.setScreen(13, "on")).toBe(false); // not the subscribed pane
    expect(sentOf(t, "screen")).toHaveLength(1);
  });

  it("asks again after a reconnect and after a gap re-subscribe, which reset it on the gateway", () => {
    const t = connected();
    t.client.setScreen(12, "on");
    t.last().drop();
    vi.advanceTimersByTime(BACKOFF_CAP_MS);
    t.last().connect();
    t.last().recv(welcome);
    expect(t.last().sent.slice(0, 2)).toEqual([
      { t: "subscribe", pane: 12, after: 41 },
      { t: "screen", pane: 12, mode: "on" },
    ]);
    t.last().recv(snapshot(50));
    t.last().recv(update(52)); // gap: re-subscribe
    expect(t.last().sent.slice(-2)).toEqual([
      { t: "subscribe", pane: 12, after: 50 },
      { t: "screen", pane: 12, mode: "on" },
    ]);
  });

  it("a new pane, or unsubscribe, starts with screens off", () => {
    const t = connected();
    t.client.setScreen(12, "auto");
    t.client.subscribe(13);
    expect(t.last().sent.at(-1)).toEqual({ t: "subscribe", pane: 13, after: null });
    t.client.unsubscribe();
    t.client.subscribe(13);
    expect(sentOf(t, "screen")).toHaveLength(1);
  });

  it("reports screens of the subscribed pane only; a re-subscribe keeps the one on display, stale", () => {
    const t = connected();
    t.client.setScreen(12, "auto");
    t.last().recv(screenMsg(true, 13));
    expect(t.screens).toEqual([]);
    t.last().recv(screenMsg(true));
    t.last().recv(screenMsg(false));
    expect(t.screens.map((m) => m.t)).toEqual(["screen", "screen"]);
    t.last().recv({ t: "screen_end", pane: 13 });
    expect(t.screens).toHaveLength(2);
    // a gap re-subscribe stops the screens silently: the one on display is out of date, not gone
    t.last().recv(update(43));
    expect(t.stale).toEqual([12]);
    expect(t.screens).toHaveLength(2);
    // the full screen that follows replaces it; nothing ends
    vi.advanceTimersByTime(SCREEN_RESUME_MS / 2);
    t.last().recv(screenMsg(true));
    vi.advanceTimersByTime(SCREEN_RESUME_MS);
    expect(t.screens.map((m) => m.t)).toEqual(["screen", "screen", "screen"]);
  });

  it("a stale screen ends when no screen comes back (the program left meanwhile), or with a real screen_end", () => {
    const t = connected();
    t.client.setScreen(12, "auto");
    t.last().recv(screenMsg(true));
    t.last().recv(update(43)); // gap re-subscribe
    vi.advanceTimersByTime(SCREEN_RESUME_MS - 1);
    expect(t.screens).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(t.screens.at(-1)).toEqual({ t: "screen_end", pane: 12 });
    // nothing to end the next time
    t.last().recv(update(45));
    vi.advanceTimersByTime(SCREEN_RESUME_MS);
    expect(t.screens).toHaveLength(2);
    // a real screen_end of a stale screen
    t.last().recv(screenMsg(true));
    t.last().recv(update(47));
    t.last().recv({ t: "screen_end", pane: 12 });
    vi.advanceTimersByTime(SCREEN_RESUME_MS);
    expect(t.screens.map((m) => m.t)).toEqual(["screen", "screen_end", "screen", "screen_end"]);
  });

  it("a reconnect keeps the screen on display (stale) until the next one", () => {
    const t = connected();
    t.client.setScreen(12, "auto");
    t.last().recv(screenMsg(true));
    t.last().drop();
    vi.advanceTimersByTime(BACKOFF_CAP_MS);
    t.last().connect();
    t.last().recv(welcome);
    expect(t.stale).toEqual([12]);
    expect(t.screens.map((m) => m.t)).toEqual(["screen"]);
    t.last().recv(screenMsg(true));
    vi.advanceTimersByTime(SCREEN_RESUME_MS * 2);
    expect(t.screens.map((m) => m.t)).toEqual(["screen", "screen"]);
  });

  it("keys and text share the gateway's rate: past 20 a second they wait, in order, never dropped", () => {
    const t = connected();
    const input = () => t.last().sent.filter((m) => m.t === "keys" || m.t === "text");
    const ids: (string | null)[] = [];
    for (let i = 0; i < 30; i++) ids.push(i % 2 ? t.client.keys(12, ["up"]) : t.client.text(12, `t${i}`));
    expect(ids.every((id) => id !== null)).toBe(true);
    expect(input()).toHaveLength(INPUT_RATE);
    expect(t.client.inputDelayMs).toBeGreaterThan(0);
    vi.advanceTimersByTime(999);
    expect(input()).toHaveLength(INPUT_RATE); // still inside the second
    vi.advanceTimersByTime(200);
    expect(input()).toHaveLength(30);
    expect(input().map((m) => ("id" in m ? m.id : ""))).toEqual(ids);
    expect(t.client.inputDelayMs).toBe(0); // 10 in this second: room for more
  });

  it("input waiting for the rate is dropped with its connection", () => {
    const t = connected();
    for (let i = 0; i < 25; i++) t.client.keys(12, ["up"]);
    t.last().drop();
    vi.advanceTimersByTime(BACKOFF_CAP_MS);
    t.last().connect();
    t.last().recv(welcome);
    expect(t.client.inputDelayMs).toBe(0);
    expect(t.last().sent.filter((m) => m.t === "keys")).toEqual([]);
  });

  it("text goes out with its own id and resolves through ack", () => {
    const t = connected();
    expect(t.client.text(12, ":wq")).toBe("x1");
    expect(t.last().sent.at(-1)).toEqual({ t: "text", id: "x1", pane: 12, text: ":wq" });
    t.client.stop();
    expect(t.client.text(12, "a")).toBeNull();
  });

  it("fit goes out with its own id, a size or a release, unpaced", () => {
    const t = connected();
    expect(t.client.fit(12, { cols: 64, rows: 40 })).toBe("f1");
    expect(t.last().sent.at(-1)).toEqual({ t: "fit", id: "f1", pane: 12, cols: 64, rows: 40 });
    expect(t.client.fit(12, null)).toBe("f2");
    expect(t.last().sent.at(-1)).toEqual({ t: "fit", id: "f2", pane: 12, cols: null, rows: null });
    t.client.stop();
    expect(t.client.fit(12, null)).toBeNull();
  });

  it("output resolves with the answer of the same id", async () => {
    const t = connected();
    const p = t.client.output(12, 86, 0, 500);
    expect(t.last().sent.at(-1)).toEqual({ t: "output", id: "o1", pane: 12, block: 86, from: 0, limit: 500 });
    t.last().recv({ t: "output", id: "o9", pane: 12, block: 86, from: 0, text: "other\n", total: 1 });
    t.last().recv({ t: "output", id: "o1", pane: 12, block: 86, from: 0, text: "a\n", total: 40 });
    expect(await p).toEqual({ ok: true, msg: { t: "output", id: "o1", pane: 12, block: 86, from: 0, text: "a\n", total: 40 } });
  });

  it("output answers must match the request's pane and block and start within its range", async () => {
    const t = connected();
    const wrongBlock = t.client.output(12, 86, 0, 500);
    t.last().recv({ t: "output", id: "o1", pane: 12, block: 87, from: 0, text: "a\n", total: 40 });
    expect(await wrongBlock).toMatchObject({ ok: false, code: "bad_data" });
    const wrongPane = t.client.output(12, 86, 0, 500);
    t.last().recv({ t: "output", id: "o2", pane: 13, block: 86, from: 0, text: "a\n", total: 40 });
    expect(await wrongPane).toMatchObject({ ok: false, code: "bad_data" });
    const pastRange = t.client.output(12, 86, 100, 500);
    t.last().recv({ t: "output", id: "o3", pane: 12, block: 86, from: 601, text: "a\n", total: 700 });
    expect(await pastRange).toMatchObject({ ok: false, code: "bad_data" });
    // later (lines gone) but inside the range is an answer
    const later = t.client.output(12, 86, 100, 500);
    t.last().recv({ t: "output", id: "o4", pane: 12, block: 86, from: 600, text: "", total: 700 });
    expect(await later).toMatchObject({ ok: true, msg: { from: 600 } });
  });

  it("output fails with the gateway's ack error, not passed on as an ack", async () => {
    const t = connected();
    const p = t.client.output(12, 99, 0, 10);
    t.last().recv({ t: "ack", id: "o1", ok: false, error: "no_block", message: "block 99 is not in pane 12" });
    expect(await p).toEqual({ ok: false, code: "no_block", message: "block 99 is not in pane 12" });
    expect(t.acks).toEqual([]);
  });

  it("output times out, fails at once on a dropped connection, and when not connected", async () => {
    const t = connected();
    const p1 = t.client.output(12, 86, 0, 10);
    vi.advanceTimersByTime(OUTPUT_TIMEOUT_MS);
    expect(await p1).toMatchObject({ ok: false, code: "timeout" });
    const p2 = t.client.output(12, 86, 0, 10);
    t.last().drop();
    expect(await p2).toMatchObject({ ok: false, code: "disconnected" });
    expect(await t.client.output(12, 86, 0, 10)).toMatchObject({ ok: false, code: "disconnected" });
  });
});

describe("RiftClient path status (the Connection sheet)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const RELAY = "wss://relay.example.com";
  const LAN = "192.168.1.29:7878";

  it("lists every path in trying order, idle before anything is tried", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    expect(t.client.connState.paths.map((p) => [p.path.kind, p.state])).toEqual([
      ["lan", "idle"],
      ["relay", "idle"],
    ]);
  });

  it("a LAN address that does not answer fails with how long it waited, and the relay is tried", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    t.client.start();
    expect(t.states.at(-1)!.paths.map((p) => p.state)).toEqual(["trying", "idle"]);
    t.last().open(); // answers the socket, never the handshake
    vi.advanceTimersByTime(1_500);
    const s = t.states.at(-1)!;
    expect(s.paths[0]).toMatchObject({ state: "failed", error: "no answer from the computer", waitedMs: 1_500, reached: true });
    expect(s.paths[1]).toMatchObject({ state: "trying", reached: false });
    expect(s.path).toMatchObject({ kind: "relay" });
    expect(s.since).not.toBeNull();
  });

  it("keeps the relay's close reason and a socket's error message as the path's error", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    t.client.start();
    t.last().onerror?.({ message: "connect ECONNREFUSED" });
    t.last().drop();
    expect(t.states.at(-1)!.paths[0]).toMatchObject({ state: "failed", error: "connect ECONNREFUSED" });
    // the relay opens, then closes the pair with its reason
    t.last().open();
    t.last().onclose?.({ reason: "the gateway did not answer" });
    expect(t.states.at(-1)!.paths[1]).toMatchObject({ state: "failed", error: "the gateway did not answer", reached: true });
  });

  it("iOS: a failed relay upgrade reports a bare error, then the close reason that names the 404 (captured from the simulator)", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    t.client.start();
    t.last().drop();
    t.last().onerror?.({});
    t.last().onclose?.({ code: 1006, reason: "Received bad response code from server: 404." });
    expect(t.states.at(-1)!.paths[1]).toMatchObject({ state: "failed", error: "Received bad response code from server: 404." });
  });

  it("a round that ends schedules a retry with its time, and the next round starts every path over", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    t.client.start();
    t.last().drop();
    t.last().drop();
    const s = t.states.at(-1)!;
    expect(s).toMatchObject({ status: "closed" });
    expect(s.paths.map((p) => p.state)).toEqual(["failed", "failed"]);
    expect(s.retryAt).toBe(Date.now() + s.retryInMs!);
    vi.advanceTimersByTime(s.retryInMs!);
    const next = t.states.at(-1)!;
    expect(next).toMatchObject({ status: "connecting", retryAt: null });
    expect(next.paths.map((p) => p.state)).toEqual(["trying", "idle"]);
  });

  it("welcome marks the working path ok and clears the others and the wait", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    t.client.start();
    t.last().drop();
    t.last().connect();
    t.last().recv(welcome);
    const s = t.states.at(-1)!;
    expect(s.paths.map((p) => p.state)).toEqual(["idle", "ok"]);
    expect(s).toMatchObject({ status: "open", since: null, retryAt: null });
  });

  it("a connection that breaks marks its path failed with the reason, and starts the wait", () => {
    const t = connected();
    vi.advanceTimersByTime(IDLE_TIMEOUT_MS);
    const s = t.states.find((x) => x.status === "closed")!;
    expect(s.paths[0]).toMatchObject({ state: "failed", error: "gateway stopped responding" });
    expect(s.since).not.toBeNull();
  });

  it("suspending leaves no path trying", () => {
    const t = setup({ addresses: [LAN], relay: RELAY });
    t.client.start();
    t.client.suspend("offline");
    expect(t.states.at(-1)).toMatchObject({ status: "offline", since: null, retryAt: null });
    expect(t.states.at(-1)!.paths.map((p) => p.state)).toEqual(["idle", "idle"]);
  });

  it("a key mismatch fails the relay path with the error", () => {
    const t = setup({ addresses: [LAN], relay: RELAY, gatewayFor: (url) => (url.startsWith("wss://") ? gatewayKey() : GATEWAY) });
    t.client.start();
    t.last().drop();
    t.last().open();
    t.last().deliver();
    const s = t.states.at(-1)!;
    expect(s).toMatchObject({ status: "failed", failure: "key_mismatch" });
    expect(s.paths[1]!.state).toBe("failed");
    expect(s.paths[1]!.error).toBeTruthy();
  });
});

describe("RiftClient closing (the computer said it stops)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("remembers the closing until the next welcome", () => {
    const { client, last } = connected();
    const at = Date.now();
    last().recv({ t: "closing", reason: "update", back_in_s: 60 });
    expect(client.connState).toMatchObject({ status: "open", closing: { reason: "update", backInS: 60, at } });
    last().drop();
    vi.advanceTimersByTime(UPDATE_RETRY_MS);
    last().connect();
    last().recv(welcome);
    expect(client.connState).toMatchObject({ status: "open", attempt: 0, closing: null });
  });

  it("an update reconnects every 3 s for the grace without counting retries, then backs off as usual", () => {
    const { client, last, sockets } = connected();
    last().recv({ t: "closing", reason: "update", back_in_s: 60 });
    const at = Date.now();
    last().drop();
    let rounds = 0;
    while (client.connState.closing) {
      expect(client.connState).toMatchObject({ status: "closed", attempt: 0, retryInMs: UPDATE_RETRY_MS, failure: null });
      expect(client.connState.retryAt).toBe(Date.now() + UPDATE_RETRY_MS);
      vi.advanceTimersByTime(UPDATE_RETRY_MS);
      expect(client.connState.status).toBe("connecting");
      last().drop(); // Rift is not back yet
      rounds++;
    }
    // every round of the grace was tried, far more than MAX_RETRIES, none counted
    expect(Date.now() - at).toBeGreaterThanOrEqual(UPDATE_GRACE_MS);
    expect(rounds).toBe(UPDATE_GRACE_MS / UPDATE_RETRY_MS);
    expect(sockets).toHaveLength(rounds + 1);
    // after it: the usual backoff and the usual MAX_RETRIES from attempt 0
    expect(client.connState).toMatchObject({ status: "closed", attempt: 0, retryInMs: 375 });
    const delays: number[] = [];
    for (let i = 0; i < MAX_RETRIES; i++) {
      delays.push(client.connState.retryInMs!);
      vi.advanceTimersByTime(client.connState.retryInMs!);
      last().drop();
    }
    expect(delays).toEqual([375, 750, 1500, 3000, 6000]);
    expect(client.connState).toMatchObject({ status: "failed", failure: "gave_up", attempt: MAX_RETRIES, closing: null });
  });

  it("Cancel during an update's wait stops it as gave_up; Try again resumes the wait", () => {
    const { client, last, sockets } = connected();
    last().recv({ t: "closing", reason: "update", back_in_s: 60 });
    last().drop();
    client.restart(); // Try again: at once, still the update's cadence
    expect(sockets).toHaveLength(2);
    last().drop();
    expect(client.connState).toMatchObject({ status: "closed", attempt: 0, retryInMs: UPDATE_RETRY_MS });
    client.giveUp();
    expect(client.connState).toMatchObject({ status: "failed", failure: "gave_up", closing: null });
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(2);
  });

  it("a quit ends as host_quit without retrying; Try again starts over", () => {
    const { client, last, sockets } = connected();
    last().recv({ t: "closing", reason: "quit" });
    expect(client.connState).toMatchObject({ status: "open", closing: { reason: "quit", backInS: null } });
    last().drop();
    expect(client.connState).toMatchObject({ status: "failed", failure: "host_quit", retryInMs: null, retryAt: null, closing: null });
    vi.advanceTimersByTime(120_000);
    expect(sockets).toHaveLength(1);
    // the user taps Try again, the phone is back online, or the app opens
    client.restart();
    expect(client.connState).toMatchObject({ status: "connecting", failure: null, attempt: 0 });
    expect(sockets).toHaveLength(2);
    // still not there: the usual backoff
    last().drop();
    expect(client.connState).toMatchObject({ status: "closed", attempt: 0, retryInMs: 375 });
  });

  it("ignores an unknown message type and a malformed closing", () => {
    const { client, last } = connected();
    last().recv({ t: "closing_soon", reason: "quit" });
    last().recv({ t: "closing", reason: "crash" });
    last().recv({ t: "closing", reason: "update", back_in_s: -5 });
    expect(client.connState).toMatchObject({ status: "open", closing: null });
    last().drop();
    expect(client.connState).toMatchObject({ status: "closed", attempt: 0, retryInMs: 375, failure: null });
  });
});
