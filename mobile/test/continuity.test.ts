// Phone continuity (COMP-006): what a request on its way does when the connection drops, times
// out or comes back. Nothing is sent again by itself, an old answer settles nothing new, and the
// user is told to look before sending it again.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ACK_TIMEOUT_MS, Fleet } from "../src/fleet";
import { toBase64Url } from "../src/noise/bytes";
import { BACKOFF_CAP_MS } from "../src/protocol/client";
import { HostsStore, MemoryStore } from "../src/stores";
import { FakeGatewaySocket, gatewayKey, rng } from "./helpers/gateway";

const fleets: Fleet[] = [];
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  for (const f of fleets.splice(0)) f.shutdown();
  vi.useRealTimers();
});

const welcome = { t: "welcome", v: 2, host: "Host", os: "macos", version: "0.13.0", device: "test", permission: "operate", features: ["text"] } as const;

async function setup() {
  const kv = new MemoryStore();
  const key = gatewayKey();
  const host = toBase64Url(key.publicKey);
  const sockets: FakeGatewaySocket[] = [];
  const fleet = new Fleet({
    kv,
    randomBytes: rng,
    platform: "ios",
    systemDeviceName: "Test phone",
    notifier: { requestPermission: async () => true, notify: async () => {} },
    socket: (url) => {
      const s = new FakeGatewaySocket(url, { key });
      sockets.push(s);
      return s;
    },
  });
  fleets.push(fleet);
  await new HostsStore(kv).save({ key: host, name: "Host", addresses: ["127.0.0.1:1"], permission: "operate", pairedAt: "2026-10-18T00:00:00Z" });
  await fleet.init();
  const last = () => sockets.at(-1)!;
  last().connect();
  last().recv(welcome);
  /** The connection drops and the client reconnects (backoff elapsed). */
  const reconnect = () => {
    last().drop();
    vi.advanceTimersByTime(BACKOFF_CAP_MS);
    last().connect();
    last().recv(welcome);
  };
  return { fleet, sockets, last, host, reconnect };
}

describe("a request on its way when the connection drops", () => {
  it("fails, says it may have gone through, keeps the draft, and is never sent again by itself", async () => {
    const t = await setup();
    t.fleet.setDraft(t.host, 12, "deploy --prod");
    const first = t.last();
    const result = t.fleet.run(t.host, 12, "deploy --prod", { notify: false });
    expect(first.sent.filter((m) => m.t === "run")).toHaveLength(1);
    t.reconnect();
    const r = await result;
    expect(r).toMatchObject({ ok: false, code: "disconnected" });
    expect(r.ok ? "" : r.message).toBe(
      "Lost the connection to Host before it answered. It may have gone through: check the pane before sending it again.",
    );
    expect(t.fleet.state.drafts[`${t.host}:12`]).toBe("deploy --prod");
    // the new connection carries no run, keys or text of the old one
    const later = t.sockets.filter((s) => s !== first).flatMap((s) => s.sent);
    expect(later.length).toBeGreaterThan(0);
    expect(later.filter((m) => ["run", "keys", "text"].includes(m.t))).toEqual([]);
  });

  it("an answer to the old request that arrives after the reconnect settles nothing new", async () => {
    const t = await setup();
    const old = t.fleet.run(t.host, 12, "make", { notify: false });
    const oldId = t.last().sent.find((m) => m.t === "run")!.id;
    t.reconnect();
    expect(await old).toMatchObject({ ok: false, code: "disconnected" });
    const again = t.fleet.run(t.host, 12, "make", { notify: false });
    const newId = t.last().sent.find((m) => m.t === "run")!.id;
    expect(newId).not.toBe(oldId);
    t.last().recv({ t: "ack", id: oldId, ok: true });
    let settled = false;
    void again.then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    t.last().recv({ t: "ack", id: newId, ok: true });
    expect(await again).toEqual({ ok: true });
  });

  it("a wait that runs out says the same", async () => {
    const t = await setup();
    const p = t.fleet.run(t.host, 12, "sleep 1", { notify: false });
    vi.advanceTimersByTime(ACK_TIMEOUT_MS);
    const r = await p;
    expect(r).toMatchObject({ ok: false, code: "timeout" });
    expect(r.ok ? "" : r.message).toBe("No answer from Host. It may have gone through: check the pane before sending it again.");
  });
});
