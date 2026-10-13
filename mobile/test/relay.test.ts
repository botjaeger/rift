import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fromBase64Url } from "../src/noise/bytes";
import { generateKeyPair } from "../src/noise/noise";
import { parsePairLink } from "../src/pairing";
import {
  HANDSHAKE_TIMEOUT_MS,
  LAN_HANDSHAKE_TIMEOUT_MS,
  RELAY_HANDSHAKE_TIMEOUT_MS,
  RiftClient,
  backoffDelay,
  type ConnState,
} from "../src/protocol/client";
import { addressHost, pathLabel, relayConnectUrl, relayHost, routingId } from "../src/protocol/relay";
import { FakeGatewaySocket, gatewayKey, rng } from "./helpers/gateway";

const relayLink = readFileSync(fileURLToPath(new URL("../../crates/rift-remote/fixtures/pairing-link-relay.txt", import.meta.url)), "utf8");

const welcome = { t: "welcome", v: 2, host: "studio-mac", os: "macos", version: "0.1.0", device: "d1", permission: "operate" } as const;
const GATEWAY = gatewayKey();
const DEVICE = generateKeyPair(rng);
const RELAY = "wss://relay.example.com";

function setup(opts: { addresses?: string[]; relay?: string | null; preferRelay?: boolean; retry?: boolean; gateway?: typeof GATEWAY } = {}) {
  const sockets: FakeGatewaySocket[] = [];
  const states: ConnState[] = [];
  const client = new RiftClient({
    addresses: opts.addresses ?? ["192.168.1.29:7878", "10.0.0.5:7878"],
    relay: opts.relay === undefined ? RELAY : opts.relay,
    preferRelay: opts.preferRelay,
    identity: DEVICE,
    gatewayKey: GATEWAY.publicKey,
    hello: () => ({ name: "Sam's iPhone", platform: "ios" }),
    randomBytes: rng,
    random: () => 0.5,
    retry: opts.retry,
    socket: (url) => {
      const s = new FakeGatewaySocket(url, { key: opts.gateway ?? GATEWAY });
      sockets.push(s);
      return s;
    },
    events: { onState: (s) => states.push(s) },
  });
  const last = () => sockets[sockets.length - 1]!;
  return { client, sockets, states, last };
}

const relayUrl = () => `${RELAY}/connect?id=${routingId(GATEWAY.publicKey)}`;

describe("routing id and relay URL", () => {
  it("is base64url(SHA-256(gateway key)[0..16]) for the fixture key", () => {
    const link = parsePairLink(relayLink)!;
    const key = fromBase64Url(link.key)!;
    const expected = createHash("sha256").update(key).digest().subarray(0, 16).toString("base64url");
    expect(routingId(key)).toBe(expected);
    expect(routingId(key)).toMatch(/^[A-Za-z0-9_-]{22}$/);
    expect(relayConnectUrl(link.relay!, key)).toBe(`wss://relay.example.com/connect?id=${expected}`);
  });

  it("labels the path for status lines", () => {
    expect(pathLabel({ kind: "lan", address: "192.168.1.29:7878" })).toBe("LAN · 192.168.1.29");
    expect(pathLabel({ kind: "lan", address: "[fe80::1]:7878" })).toBe("LAN · fe80::1");
    expect(pathLabel({ kind: "relay", url: "wss://relay.example.com:8443/rift" })).toBe("Relay · relay.example.com");
    expect(relayHost("ws://[2001:db8::1]:80")).toBe("[2001:db8::1]");
    expect(addressHost("studio.local:9000")).toBe("studio.local");
  });
});

describe("RiftClient over LAN and relay", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("tries each LAN address for 1.5 s, then the relay, with the same handshake and pinning", () => {
    const { client, sockets, last, states } = setup();
    client.start();
    expect(last().url).toBe("ws://192.168.1.29:7878");
    expect(states.at(-1)).toMatchObject({ status: "connecting", path: { kind: "lan", address: "192.168.1.29:7878" } });
    last().open(); // a stalled handshake: the socket opens, the gateway never answers
    vi.advanceTimersByTime(LAN_HANDSHAKE_TIMEOUT_MS - 1);
    expect(sockets).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(sockets[0]!.closed).toBe(true);
    expect(last().url).toBe("ws://10.0.0.5:7878");
    vi.advanceTimersByTime(LAN_HANDSHAKE_TIMEOUT_MS); // TCP connect hangs: same deadline
    expect(sockets).toHaveLength(3);
    expect(last().url).toBe(relayUrl());
    last().connect();
    expect(last().clientHello).toEqual({ v: 2, name: "Sam's iPhone", platform: "ios" });
    last().recv(welcome);
    expect(client.connState).toMatchObject({ status: "open", path: { kind: "relay", url: RELAY } });
    expect(client.preferredPath).toEqual({ kind: "relay", url: RELAY });
    expect(client.preferredAddress).toBe("192.168.1.29:7878");
  });

  it("gives the relay its own deadline, then backs off and starts the next round at the first address", () => {
    const { client, sockets, last } = setup({ addresses: ["192.168.1.29:7878"] });
    client.start();
    last().drop();
    expect(last().url).toBe(relayUrl());
    last().open();
    vi.advanceTimersByTime(RELAY_HANDSHAKE_TIMEOUT_MS - 1);
    expect(last().closed).toBe(false);
    vi.advanceTimersByTime(1);
    expect(last().closed).toBe(true);
    expect(client.connState).toMatchObject({ status: "closed", retryInMs: backoffDelay(0, () => 0.5) });
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(sockets).toHaveLength(3);
    expect(last().url).toBe("ws://192.168.1.29:7878");
  });

  it("keeps the 8 s deadline on LAN addresses when there is no relay to fall back on", () => {
    const { client, sockets, last } = setup({ relay: null });
    client.start();
    last().open();
    vi.advanceTimersByTime(LAN_HANDSHAKE_TIMEOUT_MS);
    expect(sockets).toHaveLength(1);
    vi.advanceTimersByTime(HANDSHAKE_TIMEOUT_MS - LAN_HANDSHAKE_TIMEOUT_MS);
    expect(sockets).toHaveLength(2);
  });

  it("starts at the last good path: the relay after it worked, or when the record says so", () => {
    const a = setup();
    a.client.start();
    a.last().drop();
    a.last().drop();
    a.last().connect();
    a.last().recv(welcome);
    a.last().drop();
    vi.advanceTimersByTime(backoffDelay(0, () => 0.5));
    expect(a.last().url).toBe(relayUrl());

    const b = setup({ preferRelay: true });
    b.client.start();
    expect(b.last().url).toBe(relayUrl());
  });

  it("refuses an impostor behind the relay (pinning is end to end)", () => {
    const { client, last } = setup({ addresses: ["192.168.1.29:7878"], gateway: gatewayKey() });
    client.start();
    last().drop();
    expect(last().url).toBe(relayUrl());
    last().open();
    last().deliver();
    expect(last().clientHello).toBeNull();
    expect(client.connState).toMatchObject({ status: "failed", failure: "key_mismatch" });
  });

  it("restart drops an open relay socket and goes LAN first with the backoff reset", () => {
    const { client, sockets, last } = setup({ preferRelay: true });
    client.start();
    last().connect();
    last().recv(welcome);
    expect(client.connState.status).toBe("open");
    client.restart({ lanFirst: true });
    expect(sockets[0]!.closed).toBe(true);
    expect(last().url).toBe("ws://192.168.1.29:7878");
    expect(client.connState).toMatchObject({ status: "connecting", attempt: 0 });
    last().connect();
    last().recv(welcome);
    expect(client.preferredPath).toEqual({ kind: "lan", address: "192.168.1.29:7878" });
  });

  it("restart resets the backoff after failed rounds (the network changed: try at once)", () => {
    const { client, sockets, last } = setup({ relay: null, addresses: ["192.168.1.29:7878"] });
    client.start();
    for (let i = 0; i < 4; i++) {
      last().drop();
      vi.advanceTimersByTime(client.connState.retryInMs!);
    }
    expect(client.connState.attempt).toBe(4);
    client.restart({ lanFirst: true });
    expect(client.connState).toMatchObject({ status: "connecting", attempt: 0 });
    last().drop();
    expect(client.connState.retryInMs).toBe(backoffDelay(0, () => 0.5));
    expect(sockets).toHaveLength(6);
  });

  it("restart without lanFirst starts at the last good path; a client refused by the gateway stays failed", () => {
    const a = setup({ preferRelay: true });
    a.client.start();
    a.client.restart();
    expect(a.last().url).toBe(relayUrl());

    // (A key mismatch is tried again on restart: client.test.ts, key mismatch across paths.)
    const b = setup({ addresses: ["192.168.1.29:7878"], relay: null });
    b.client.start();
    b.last().connect();
    b.last().recv({ t: "error", code: "auth", message: "this device is not paired" });
    expect(b.client.connState.status).toBe("failed");
    b.client.restart({ lanFirst: true });
    expect(b.sockets).toHaveLength(1);
  });

  it("suspend('offline') closes and never retries until restart", () => {
    const { client, sockets, last } = setup();
    client.start();
    last().connect();
    last().recv(welcome);
    client.suspend("offline");
    expect(sockets[0]!.closed).toBe(true);
    expect(client.connState).toMatchObject({ status: "offline", retryInMs: null });
    vi.advanceTimersByTime(10 * 60_000);
    expect(sockets).toHaveLength(1);
    client.restart({ lanFirst: true });
    expect(sockets).toHaveLength(2);
  });

  it("pairs over the relay when no address answers (one pass, then unreachable)", () => {
    const a = setup({ retry: false });
    a.client.start();
    a.last().drop();
    a.last().drop();
    expect(a.last().url).toBe(relayUrl());
    a.last().connect();
    a.last().recv(welcome);
    expect(a.client.connState.status).toBe("open");

    const b = setup({ retry: false });
    b.client.start();
    b.last().drop();
    b.last().drop();
    b.last().drop();
    expect(b.client.connState).toMatchObject({ status: "failed", failure: "unreachable" });
    expect(b.sockets).toHaveLength(3);
  });
});
