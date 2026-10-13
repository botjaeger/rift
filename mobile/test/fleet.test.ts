import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toBase64Url } from "../src/noise/bytes";
import { ACK_TIMEOUT_MS, ASKED_TTL_MS, Fleet, MAX_ACTIVITY, canOperate, type Notifier } from "../src/fleet";
import { routingId } from "../src/protocol/relay";
import { LONG_COMMAND_MS } from "../src/notify";
import { parsePairLink, type PairLink } from "../src/pairing";
import { PUBLISH_INTERVAL_MS } from "../src/protocol/batch";
import type { Block } from "../src/protocol/types";
import { HostsStore, MemoryStore, type HostRecord } from "../src/stores";
import { FakeGatewaySocket, gatewayKey } from "./helpers/gateway";
import { rng } from "./helpers/gateway";

const GW = gatewayKey();
const GW_KEY = toBase64Url(GW.publicKey);
const welcome = (permission: "operate" | "view" = "operate") =>
  ({ t: "welcome", v: 2, host: "studio-mac", os: "macos", version: "0.1.0", device: "d1", permission }) as const;
const block = (id: number, extra: Partial<Block> = {}): Block => ({
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
  ...extra,
});

const link = (extra: Partial<PairLink> = {}): PairLink => ({
  addresses: ["192.168.1.29:7878", "10.0.0.5:7878"],
  key: GW_KEY,
  secret: "q83vEjRWeJCrze8SNFZ4kA",
  name: "studio-mac",
  permission: "operate",
  relay: null,
  ...extra,
});

const record = (extra: Partial<HostRecord> = {}): HostRecord => ({
  key: GW_KEY,
  name: "studio-mac",
  addresses: ["10.0.0.5:7878"],
  permission: "operate",
  pairedAt: "2026-10-12T10:00:00.000Z",
  ...extra,
});

function setup(opts: { gateway?: typeof GW; gatewayFor?: (url: string) => typeof GW } = {}) {
  const kv = new MemoryStore();
  let now = 1_000_000;
  const sockets: FakeGatewaySocket[] = [];
  const notified: { title: string; body: string; data: unknown }[] = [];
  let permissionAsks = 0;
  let grant = true;
  const notifier: Notifier = {
    requestPermission: async () => {
      permissionAsks++;
      return grant;
    },
    notify: async (n) => {
      notified.push(n);
    },
  };
  const fleet = new Fleet({
    kv,
    randomBytes: rng,
    platform: "ios",
    systemDeviceName: "Sam's iPhone",
    notifier,
    random: () => 0.5,
    now: () => now,
    socket: (url) => {
      const s = new FakeGatewaySocket(url, { key: opts.gatewayFor?.(url) ?? opts.gateway ?? GW });
      sockets.push(s);
      return s;
    },
  });
  const last = () => sockets[sockets.length - 1]!;
  return {
    kv,
    fleet,
    sockets,
    last,
    notified,
    get permissionAsks() {
      return permissionAsks;
    },
    setGrant: (g: boolean) => (grant = g),
    /** Moves the fleet's wall clock (not the timers). */
    advanceNow: (ms: number) => (now += ms),
  };
}

/** One window of Rift process `pid` with panes 12 and 13. */
const topology = (pid = 1, panes = [12, 13]) => ({
  t: "topology",
  windows: [
    {
      pid,
      tabs: [
        {
          index: 0,
          title: "rift",
          active: true,
          panes: panes.map((id) => ({
            id,
            active: id === 12,
            cwd: "/r",
            job: null,
            last_command: null,
            last_exit: null,
          })),
        },
      ],
    },
  ],
});

/** A fleet with one stored, connected host following pane 12. */
async function withHost(permission: "operate" | "view" = "operate") {
  const t = setup();
  await new HostsStore(t.kv).save(record({ permission }));
  await t.fleet.init();
  t.last().connect();
  t.last().recv(welcome(permission));
  t.last().recv(topology());
  t.fleet.openPane(GW_KEY, 12);
  t.last().recv({ t: "snapshot", pane: 12, seq: 41, blocks: [block(1)], has_older: false });
  t.fleet.store.flush();
  return t;
}

describe("Fleet pairing", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("pairs: sends the secret in the hello, stores the host, then connects without the secret", async () => {
    const t = setup();
    await t.fleet.init();
    const { result } = t.fleet.pair(link());
    t.last().drop(); // first address unreachable
    expect(t.last().url).toBe("ws://10.0.0.5:7878");
    t.last().connect();
    expect(t.last().clientHello).toEqual({
      v: 2,
      name: "Sam's iPhone",
      platform: "ios",
      pair: "q83vEjRWeJCrze8SNFZ4kA",
    });
    t.last().recv(welcome("view"));
    const r = await result;
    expect(r).toMatchObject({
      ok: true,
      host: { key: GW_KEY, name: "studio-mac", permission: "view", addresses: ["10.0.0.5:7878", "192.168.1.29:7878"] },
    });
    await vi.waitFor(() => expect(t.fleet.state.hosts).toHaveLength(1));
    expect(await new HostsStore(t.kv).load()).toHaveLength(1);
    // The pairing socket is closed; the host's own connection starts at the address that worked.
    expect(t.sockets.at(-2)!.closed).toBe(true);
    t.last().connect();
    expect(t.last().url).toBe("ws://10.0.0.5:7878");
    expect(t.last().clientHello).toEqual({ v: 2, name: "Sam's iPhone", platform: "ios" });
  });

  it("reports a spent or expired secret (auth) and stores nothing", async () => {
    const t = setup();
    await t.fleet.init();
    const { result } = t.fleet.pair(link({ addresses: ["10.0.0.5:7878"] }));
    t.last().connect();
    t.last().recv({ t: "error", code: "auth", message: "pairing secret expired" });
    expect(await result).toMatchObject({
      ok: false,
      failure: "auth",
      message: expect.stringContaining("expired or was already used"),
    });
    expect(await new HostsStore(t.kv).load()).toEqual([]);
    expect(t.fleet.pairedWith(link())).toBe(false);
  });

  it("remembers the secrets it spent, so a re-delivered launch link opens the host", async () => {
    const t = setup();
    await t.fleet.init();
    expect(t.fleet.pairedWith(link())).toBe(false);
    const { result } = t.fleet.pair(link({ addresses: ["10.0.0.5:7878"] }));
    t.last().connect();
    t.last().recv(welcome("operate"));
    expect(await result).toMatchObject({ ok: true });
    expect(t.fleet.pairedWith(link())).toBe(true);
    expect(t.fleet.pairedWith(link({ secret: "AAAAAAAAAAAAAAAAAAAAAA" }))).toBe(false);
  });

  it("reports unreachable after trying every address once", async () => {
    const t = setup();
    await t.fleet.init();
    const { result } = t.fleet.pair(link());
    t.last().drop();
    t.last().drop();
    expect(await result).toMatchObject({
      ok: false,
      failure: "unreachable",
      message: expect.stringContaining("Could not reach studio-mac"),
    });
    vi.advanceTimersByTime(60_000);
    expect(t.sockets).toHaveLength(2);
  });

  it("reports a key mismatch and never sends the hello", async () => {
    const t = setup({ gateway: gatewayKey() });
    await t.fleet.init();
    const { result } = t.fleet.pair(link({ addresses: ["10.0.0.5:7878"] }));
    t.last().open();
    t.last().deliver();
    expect(await result).toMatchObject({
      ok: false,
      failure: "key_mismatch",
      message: expect.stringContaining("Nothing was sent"),
    });
    expect(t.last().clientHello).toBeNull();
    expect(t.fleet.state.hosts).toEqual([]);
  });

  it("cancel stops trying", async () => {
    const t = setup();
    await t.fleet.init();
    const { result, cancel } = t.fleet.pair(link());
    cancel();
    expect(await result).toMatchObject({ ok: false });
    expect(t.sockets[0]!.closed).toBe(true);
    expect(t.sockets).toHaveLength(1);
  });
});

describe("Fleet operate", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("run sends the command; ok clears the draft", async () => {
    const t = await withHost();
    t.fleet.setDraft(GW_KEY, 12, "cargo test");
    const p = t.fleet.run(GW_KEY, 12, "cargo test");
    expect(t.last().sent.at(-1)).toEqual({ t: "run", id: "r1", pane: 12, command: "cargo test" });
    t.last().recv({ t: "ack", id: "r1", ok: true });
    expect(await p).toEqual({ ok: true });
    expect(t.fleet.state.drafts[`${GW_KEY}:12`]).toBe("");
  });

  it("a draft is published at once (the command field is controlled by it)", async () => {
    const t = await withHost();
    for (const text of ["s", "sl", "sle"]) {
      t.fleet.setDraft(GW_KEY, 12, text);
      expect(t.fleet.store.getSnapshot().drafts[`${GW_KEY}:12`]).toBe(text);
    }
  });

  it.each([
    ["busy", "This pane is running a command"],
    ["forbidden", "may only view"],
    ["rate", "Too many"],
    ["no_pane", "has closed"],
  ])("an ack error %s comes back as a message and keeps the draft", async (code, text) => {
    const t = await withHost();
    t.fleet.setDraft(GW_KEY, 12, "ls");
    const p = t.fleet.run(GW_KEY, 12, "ls");
    t.last().recv({ t: "ack", id: "r1", ok: false, error: code, message: "gateway text" });
    const r = await p;
    expect(r).toMatchObject({ ok: false, code });
    expect(r.ok ? "" : r.message).toContain(text);
    expect(t.fleet.state.drafts[`${GW_KEY}:12`]).toBe("ls");
  });

  it("keys go out and resolve on ack; a view device gets forbidden", async () => {
    const t = await withHost("view");
    const p = t.fleet.sendKeys(GW_KEY, 12, ["ctrl+c"]);
    expect(t.last().sent.at(-1)).toEqual({ t: "keys", id: "k1", pane: 12, keys: ["ctrl+c"] });
    t.last().recv({ t: "ack", id: "k1", ok: false, error: "forbidden", message: "this device may only view" });
    expect(await p).toMatchObject({ ok: false, code: "forbidden" });
  });

  it("an unanswered request times out; a dropped connection fails it at once", async () => {
    const t = await withHost();
    const p1 = t.fleet.run(GW_KEY, 12, "ls");
    vi.advanceTimersByTime(ACK_TIMEOUT_MS);
    expect(await p1).toMatchObject({ ok: false, code: "timeout" });
    const p2 = t.fleet.run(GW_KEY, 12, "ls");
    t.last().drop();
    expect(await p2).toMatchObject({ ok: false, code: "disconnected" });
    expect(await t.fleet.run(GW_KEY, 12, "ls")).toMatchObject({ ok: false, code: "disconnected" });
  });

  it("an ack for an id this host never issued is ignored", async () => {
    const t = await withHost();
    const p = t.fleet.run(GW_KEY, 12, "ls");
    // A forged ack for an id this host never issued is ignored.
    t.last().recv({ t: "ack", id: "r99", ok: true });
    vi.advanceTimersByTime(ACK_TIMEOUT_MS);
    expect(await p).toMatchObject({ ok: false, code: "timeout" });
  });

  it("resumes the pane after a reconnect", async () => {
    const t = await withHost();
    t.last().recv({ t: "update", pane: 12, seq: 42, blocks: [block(2)], removed: [] });
    t.last().drop();
    vi.advanceTimersByTime(1_000);
    t.last().connect();
    t.last().recv(welcome());
    expect(t.last().sent[0]).toEqual({ t: "subscribe", pane: 12, after: 42 });
    t.fleet.store.flush();
    expect(t.fleet.state.pane?.state?.blocks.map((b) => b.id)).toEqual([1, 2]);
  });

  it("publishes at most 10 times a second under an update flood", async () => {
    const t = await withHost();
    let publishes = 0;
    t.fleet.store.subscribe(() => publishes++);
    for (let ms = 0; ms < 1000; ms++) {
      t.last().recv({
        t: "update",
        pane: 12,
        seq: 42 + ms,
        blocks: [block(2, { output: `line ${ms}\n` })],
        removed: [],
      });
      vi.advanceTimersByTime(1);
    }
    vi.advanceTimersByTime(PUBLISH_INTERVAL_MS);
    expect(publishes).toBeGreaterThan(0);
    expect(publishes).toBeLessThanOrEqual(11);
    expect(t.fleet.state.pane?.state?.seq).toBe(1041);
  });

  it("unpair sends forget, closes, and deletes the host", async () => {
    const t = await withHost();
    await t.fleet.unpair(GW_KEY);
    expect(t.sockets[0]!.sent.at(-1)).toEqual({ t: "forget" });
    expect(t.sockets[0]!.closed).toBe(true);
    expect(t.fleet.state.hosts).toEqual([]);
    expect(t.fleet.state.pane).toBeNull();
    expect(await new HostsStore(t.kv).load()).toEqual([]);
  });
});

describe("Fleet done → activity and notifications", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const done = (extra: Record<string, unknown> = {}) => ({
    t: "done",
    pane: 12,
    block: 87,
    command: "cargo test",
    exit: 0,
    duration_ms: LONG_COMMAND_MS,
    ...extra,
  });

  it("records every done, newest first, at most 100", async () => {
    const t = await withHost();
    for (let i = 0; i < MAX_ACTIVITY + 5; i++) t.last().recv(done({ block: i }));
    t.fleet.store.flush();
    expect(t.fleet.state.activity).toHaveLength(MAX_ACTIVITY);
    expect(t.fleet.state.activity[0]).toMatchObject({ block: MAX_ACTIVITY + 4, hostName: "studio-mac", pane: 12 });
  });

  it("notifies only when enabled, long, and not watched; the command only with the setting", async () => {
    const t = await withHost();
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified).toHaveLength(0); // notifications are off by default

    await t.fleet.updateSettings({ notifyDone: true });
    expect(t.permissionAsks).toBe(1);
    t.fleet.setViewing({ host: GW_KEY, pane: 12 });
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified).toHaveLength(0); // watching it

    t.fleet.setAppActive(false);
    t.last().recv(done({ exit: 2 }));
    await Promise.resolve();
    expect(t.notified).toEqual([
      { title: "A command failed on studio-mac (exit 2)", body: "", data: { host: GW_KEY, pane: 12 } },
    ]);

    await t.fleet.updateSettings({ showCommandInNotification: true });
    t.fleet.setAppActive(true);
    t.fleet.setViewing(null);
    t.last().recv(done());
    t.last().recv(done({ duration_ms: 5_000 })); // short: not notified
    await Promise.resolve();
    expect(t.notified.slice(1)).toEqual([
      { title: "A command finished on studio-mac", body: "cargo test", data: { host: GW_KEY, pane: 12 } },
    ]);
  });

  it("'notify me' notifies a short command once, even with notifications off", async () => {
    const t = await withHost();
    const p = t.fleet.run(GW_KEY, 12, "make", { notify: true });
    t.last().recv({ t: "ack", id: "r1", ok: true });
    await p;
    t.last().recv(done({ command: "make", duration_ms: 1_000 }));
    t.last().recv(done({ command: "make", duration_ms: 1_000 }));
    await Promise.resolve();
    expect(t.notified).toHaveLength(1);
  });

  it("a push token goes to every computer, which then alerts instead of the app", async () => {
    const TOKEN = "ExponentPushToken[abc-123]";
    const t = await withHost();
    const pushes = () => t.last().sent.filter((m) => (m as { t: string }).t === "push");
    expect(pushes()).toEqual([{ t: "push", token: null, long_ms: null, show_command: false }]);

    t.fleet.setPushToken(TOKEN);
    expect(t.fleet.state.push).toEqual({ state: "on" });
    expect(pushes().at(-1)).toEqual({ t: "push", token: TOKEN, long_ms: null, show_command: false });
    await t.fleet.updateSettings({ notifyDone: true, showCommandInNotification: true });
    expect(pushes().at(-1)).toEqual({ t: "push", token: TOKEN, long_ms: LONG_COMMAND_MS, show_command: true });

    // the computer alerts: the app posts nothing, but the activity still lists it
    t.fleet.setAppActive(false);
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified).toHaveLength(0);
    t.fleet.store.flush();
    expect(t.fleet.state.activity).toHaveLength(1);
    // "notify me" asks the computer too
    const p = t.fleet.run(GW_KEY, 12, "make", { notify: true });
    expect(t.last().sent.at(-1)).toEqual({ t: "notify", pane: 12 });
    t.last().recv({ t: "ack", id: "r1", ok: true });
    await p;

    // a reconnect sends the token again
    const before = t.sockets.length;
    t.last().drop();
    vi.advanceTimersByTime(1_000);
    expect(t.sockets.length).toBe(before + 1);
    t.last().connect();
    t.last().recv(welcome());
    expect(pushes().at(-1)).toEqual({ t: "push", token: TOKEN, long_ms: LONG_COMMAND_MS, show_command: true });

    // no token any more: the computers stop, the app notifies again
    t.fleet.setPushToken(null, "no push here");
    expect(t.fleet.state.push).toEqual({ state: "unavailable", reason: "no push here" });
    expect(pushes().at(-1)).toEqual({ t: "push", token: null, long_ms: LONG_COMMAND_MS, show_command: true });
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified).toHaveLength(1);
  });

  it("ignores a push token the gateway would refuse", async () => {
    const t = await withHost();
    t.fleet.setPushToken("fcm:not-an-expo-token");
    expect(t.fleet.state.push).toEqual({ state: "off" });
    expect(
      t
        .last()
        .sent.filter((m) => (m as { t: string }).t === "push")
        .at(-1),
    ).toMatchObject({ token: null });
  });

  it("an alert about the pane on screen is not shown again", async () => {
    const t = await withHost();
    t.fleet.setViewing({ host: GW_KEY, pane: 12 });
    expect(t.fleet.isViewing(GW_KEY, 12)).toBe(true);
    expect(t.fleet.isViewing(GW_KEY, 13)).toBe(false);
    t.fleet.setAppActive(false);
    expect(t.fleet.isViewing(GW_KEY, 12)).toBe(false);
  });

  it("enabling notifications stays off when the OS refuses", async () => {
    const t = await withHost();
    t.setGrant(false);
    await t.fleet.updateSettings({ notifyDone: true });
    expect(t.fleet.state.settings.notifyDone).toBe(false);
  });
});

describe("Fleet init", () => {
  it("reports an unreadable keychain instead of making a new identity", async () => {
    const t = setup();
    t.kv.get = async () => {
      throw new Error("keychain locked");
    };
    await t.fleet.init();
    expect(t.fleet.state).toMatchObject({ ready: true, loadError: "keychain locked" });
    expect(t.sockets).toHaveLength(0);
  });

  it("the fixture pairing link parses into a pairable link", () => {
    expect(
      parsePairLink(`rift-remote://pair?a=10.0.0.5:7878&k=${GW_KEY}&s=q83vEjRWeJCrze8SNFZ4kA&n=studio-mac&p=operate`),
    ).toEqual(link({ addresses: ["10.0.0.5:7878"] }));
  });
});

describe("Fleet relay (Phase 2)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const relayUrl = () => `wss://relay.example.com/connect?id=${routingId(GW.publicKey)}`;

  it("pairs through the relay when the addresses do not answer, and keeps the relay as last good path", async () => {
    const t = setup();
    await t.fleet.init();
    const { result } = t.fleet.pair(link({ relay: "wss://relay.example.com" }));
    t.last().drop();
    t.last().drop();
    expect(t.last().url).toBe(relayUrl());
    t.last().connect();
    expect(t.last().clientHello?.pair).toBe("q83vEjRWeJCrze8SNFZ4kA");
    t.last().recv(welcome());
    const r = await result;
    expect(r).toMatchObject({ ok: true, host: { relay: "wss://relay.example.com", via: "relay" } });
    await vi.waitFor(() => expect(t.fleet.state.hosts).toHaveLength(1));
    expect((await new HostsStore(t.kv).load())[0]).toMatchObject({ relay: "wss://relay.example.com", via: "relay" });
    // The host's own connection starts at the relay, without the secret.
    expect(t.last().url).toBe(relayUrl());
    t.last().connect();
    expect(t.last().clientHello?.pair).toBeUndefined();
  });

  it("an unreachable relay pairing says so", async () => {
    const t = setup();
    await t.fleet.init();
    const { result } = t.fleet.pair(link({ addresses: ["10.0.0.5:7878"], relay: "wss://relay.example.com" }));
    t.last().drop();
    t.last().drop();
    expect(await result).toMatchObject({
      ok: false,
      failure: "unreachable",
      message: expect.stringContaining("through its relay"),
    });
  });

  it("records the path that worked: back on LAN, via becomes lan", async () => {
    const t = setup();
    await new HostsStore(t.kv).save(record({ relay: "wss://relay.example.com", via: "relay" }));
    await t.fleet.init();
    expect(t.last().url).toBe(relayUrl());
    t.fleet.resumeConnections({ lanFirst: true, force: true });
    expect(t.last().url).toBe("ws://10.0.0.5:7878");
    t.last().connect();
    t.last().recv(welcome());
    await vi.waitFor(async () => expect((await new HostsStore(t.kv).load())[0]?.via).toBe("lan"));
    t.fleet.store.flush();
    expect(t.fleet.state.hosts[0]!.conn.path).toEqual({ kind: "lan", address: "10.0.0.5:7878" });
  });
});

describe("Fleet connectivity hooks", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("offline closes every socket, shows Offline and never retries; resume reconnects LAN first", async () => {
    const t = await withHost();
    t.fleet.suspendConnections("offline");
    t.fleet.store.flush();
    expect(t.sockets[0]!.closed).toBe(true);
    expect(t.fleet.state.hosts[0]!.conn.status).toBe("offline");
    vi.advanceTimersByTime(10 * 60_000);
    expect(t.sockets).toHaveLength(1);
    t.fleet.resumeConnections({ lanFirst: true, force: true });
    expect(t.sockets).toHaveLength(2);
    t.last().connect();
    t.last().recv(welcome());
    expect(t.last().sent[0]).toEqual({ t: "subscribe", pane: 12, after: 41 });
  });

  it("hosts loaded while offline wait for the network", async () => {
    const t = setup();
    await new HostsStore(t.kv).save(record());
    t.fleet.suspendConnections("offline");
    await t.fleet.init();
    t.fleet.store.flush();
    expect(t.sockets).toHaveLength(0);
    expect(t.fleet.state.hosts[0]!.conn.status).toBe("offline");
    t.fleet.resumeConnections({ lanFirst: false, force: false });
    expect(t.sockets).toHaveLength(1);
  });

  it("resume without force keeps open connections and retries waiting ones now", async () => {
    const t = await withHost();
    t.fleet.resumeConnections({ lanFirst: false, force: false });
    expect(t.sockets).toHaveLength(1);
    t.last().drop(); // now waiting out the backoff
    t.fleet.resumeConnections({ lanFirst: false, force: false });
    expect(t.sockets).toHaveLength(2);
  });

  it("hasPendingNotify follows 'Notify me' and tells the listener", async () => {
    const t = await withHost();
    let calls = 0;
    t.fleet.onPendingNotifyChange(() => calls++);
    const p = t.fleet.run(GW_KEY, 12, "make", { notify: true });
    expect(t.fleet.hasPendingNotify).toBe(true);
    t.last().recv({ t: "ack", id: "r1", ok: true });
    await p;
    t.last().recv({ t: "done", pane: 12, block: 9, command: "make", exit: 0, duration_ms: 1 });
    expect(t.fleet.hasPendingNotify).toBe(false);
    expect(calls).toBe(2);
  });
});

describe("Fleet permission changes", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("the next welcome's permission updates the host and the UI without re-pairing", async () => {
    const t = await withHost("operate");
    expect(canOperate(t.fleet.state.hosts[0]!)).toBe(true);
    // The desktop changes the permission and closes the connection.
    t.last().drop();
    vi.advanceTimersByTime(1_000);
    t.last().connect();
    expect(t.last().clientHello?.pair).toBeUndefined();
    t.last().recv(welcome("view"));
    await vi.waitFor(async () => expect((await new HostsStore(t.kv).load())[0]?.permission).toBe("view"));
    t.fleet.store.flush();
    const host = t.fleet.state.hosts[0]!;
    expect(host.record.permission).toBe("view");
    expect(canOperate(host)).toBe(false);

    // And back.
    t.last().drop();
    vi.advanceTimersByTime(1_000);
    t.last().connect();
    t.last().recv(welcome("operate"));
    await vi.waitFor(async () => expect((await new HostsStore(t.kv).load())[0]?.permission).toBe("operate"));
    t.fleet.store.flush();
    expect(canOperate(t.fleet.state.hosts[0]!)).toBe(true);
    expect(t.sockets.every((s) => s.clientHello?.pair === undefined)).toBe(true);
  });

  it("canOperate prefers the live welcome over the stored record", () => {
    const base = { conn: t0, gateway: null, windows: [], notice: null };
    expect(canOperate({ ...base, record: record({ permission: "operate" }), welcome: null })).toBe(true);
    expect(canOperate({ ...base, record: record({ permission: "operate" }), welcome: welcome("view") })).toBe(false);
    expect(canOperate({ ...base, record: record({ permission: "view" }), welcome: welcome("operate") })).toBe(true);
  });
});

/** Lets every pending promise chain of the in-memory store run to its end. */
async function settle() {
  for (let i = 0; i < 100; i++) await Promise.resolve();
}

/** Drops the host's socket and reconnects it with a welcome. */
function reconnect(t: ReturnType<typeof setup>, w: ReturnType<typeof welcome> | Record<string, unknown> = welcome()) {
  t.last().drop();
  vi.advanceTimersByTime(1_000);
  t.last().connect();
  t.last().recv(w);
}

describe("Fleet audit fixes", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const relayUrl = () => `wss://relay.example.com/connect?id=${routingId(GW.publicKey)}`;
  const lanImpostor = gatewayKey();

  it("pairing goes on to the relay when a LAN address answers with another key", async () => {
    const t = setup({ gatewayFor: (url) => (url.startsWith("ws://") ? lanImpostor : GW) });
    await t.fleet.init();
    const { result } = t.fleet.pair(link({ addresses: ["10.0.0.5:7878"], relay: "wss://relay.example.com" }));
    t.last().open();
    t.last().deliver();
    expect(t.sockets[0]!.clientHello).toBeNull();
    expect(t.last().url).toBe(relayUrl());
    t.last().connect();
    t.last().recv(welcome());
    expect(await result).toMatchObject({ ok: true, host: { via: "relay" } });
  });

  it("a network change tries a key-mismatch host again; a foreground resume does not", async () => {
    const t = setup({ gateway: lanImpostor });
    await new HostsStore(t.kv).save(record());
    await t.fleet.init();
    t.last().open();
    t.last().deliver();
    expect(t.fleet.host(GW_KEY)!.conn).toMatchObject({ status: "failed", failure: "key_mismatch" });
    t.fleet.resumeConnections({ lanFirst: false, force: false });
    expect(t.sockets).toHaveLength(1);
    t.fleet.resumeConnections({ lanFirst: true, force: true });
    expect(t.sockets).toHaveLength(2);
  });

  it("an unpair while a record save is in flight is not undone by that save", async () => {
    const t = await withHost();
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const set = t.kv.set.bind(t.kv);
    t.kv.set = async (k, v) => {
      if (v.includes("renamed-mac")) await gate;
      return set(k, v);
    };
    reconnect(t, { ...welcome(), host: "renamed-mac" }); // the rename starts a save, held back
    await t.fleet.unpair(GW_KEY);
    release();
    await settle();
    expect(await new HostsStore(t.kv).load()).toEqual([]);
    expect(t.fleet.state.hosts).toEqual([]);
  });

  it("a keychain that refuses the new host fails the pairing instead of reporting it paired", async () => {
    const t = setup();
    await t.fleet.init();
    t.kv.set = async () => {
      throw new Error("keychain locked");
    };
    const { result } = t.fleet.pair(link({ addresses: ["10.0.0.5:7878"] }));
    t.last().connect();
    t.last().recv(welcome());
    expect(await result).toMatchObject({
      ok: false,
      failure: "storage",
      message: expect.stringContaining("could not be saved"),
    });
    expect(t.fleet.state.hosts).toEqual([]);
    expect(t.fleet.pairedWith(link())).toBe(false);
    expect(t.sockets).toHaveLength(1);
    expect(t.sockets[0]!.closed).toBe(true);
  });

  it("a 'Notify me' expires after 24 h", async () => {
    const t = await withHost();
    const p = t.fleet.run(GW_KEY, 12, "make", { notify: true });
    t.last().recv({ t: "ack", id: "r1", ok: true });
    await p;
    t.advanceNow(ASKED_TTL_MS - 1);
    expect(t.fleet.hasPendingNotify).toBe(true);
    t.advanceNow(1);
    expect(t.fleet.hasPendingNotify).toBe(false);
    t.last().recv({ t: "done", pane: 12, block: 9, command: "make", exit: 0, duration_ms: 1 });
    await Promise.resolve();
    expect(t.notified).toHaveLength(0);
  });

  it("a 'Notify me' survives a reconnect, but not a restarted computer or a closed pane", async () => {
    const t = await withHost();
    const ask = async (pane: number) => {
      const p = t.fleet.run(GW_KEY, pane, "make", { notify: true });
      t.last().recv({ t: "ack", id: (t.last().sent.at(-1) as { id: string }).id, ok: true });
      expect(await p).toEqual({ ok: true });
    };
    await ask(12);
    reconnect(t);
    t.last().recv(topology(1));
    expect(t.fleet.hasPendingNotify).toBe(true);
    reconnect(t);
    t.last().recv(topology(2)); // another Rift process: the pane ids start over
    expect(t.fleet.hasPendingNotify).toBe(false);

    await ask(13);
    reconnect(t);
    t.last().recv(topology(2, [12]));
    expect(t.fleet.hasPendingNotify).toBe(false);
  });

  it("Retry after an unreadable keychain loads and connects", async () => {
    const t = setup();
    await new HostsStore(t.kv).save(record());
    const get = t.kv.get.bind(t.kv);
    t.kv.get = async () => {
      throw new Error("keychain locked");
    };
    await t.fleet.init();
    expect(t.fleet.state.loadError).toBe("keychain locked");
    t.kv.get = get;
    await t.fleet.retryInit();
    expect(t.fleet.state).toMatchObject({ ready: true, loadError: null });
    expect(t.fleet.state.hosts).toHaveLength(1);
    expect(t.sockets).toHaveLength(1);
  });

  it("Try again while offline waits for the network", async () => {
    const t = await withHost();
    t.fleet.suspendConnections("offline");
    t.fleet.retry(GW_KEY);
    t.fleet.store.flush();
    expect(t.sockets).toHaveLength(1);
    expect(t.fleet.state.hosts[0]!.conn.status).toBe("offline");
    t.fleet.resumeConnections({ lanFirst: true, force: true });
    expect(t.sockets).toHaveLength(2);
  });

  it("closing a pane pushed over another on the same host, then refocusing the one below, follows it again", async () => {
    const t = await withHost();
    t.fleet.openPane(GW_KEY, 13); // a notification tap pushes pane 13
    t.fleet.closePane(GW_KEY, 13); // back
    t.fleet.openPane(GW_KEY, 12); // pane 12 regains focus
    expect(t.last().sent.at(-1)).toEqual({ t: "subscribe", pane: 12, after: null });
    t.last().recv({ t: "snapshot", pane: 12, seq: 50, blocks: [block(1)], has_older: false });
    t.fleet.store.flush();
    expect(t.fleet.state.pane).toMatchObject({ host: GW_KEY, paneId: 12, state: { blocks: [{ id: 1 }] } });
  });

  it("the same pane pushed twice and popped once keeps following, whichever screen event comes first", async () => {
    const t = await withHost();
    const [below, above] = [{}, {}];
    t.fleet.openPane(GW_KEY, 12, below);
    t.fleet.openPane(GW_KEY, 12, above); // a deep link pushes pane 12 again
    const sent = t.last().sent.length;
    t.fleet.openPane(GW_KEY, 12, below); // back: the screen below regains focus first ...
    t.fleet.closePane(GW_KEY, 12, above); // ... then the one above unmounts
    expect(t.last().sent.slice(sent)).not.toContainEqual({ t: "unsubscribe" });
    expect(t.fleet.state.pane).toMatchObject({ host: GW_KEY, paneId: 12 });
    t.fleet.closePane(GW_KEY, 12); // the harness's own hold
    t.fleet.closePane(GW_KEY, 12, below); // the last screen leaves
    expect(t.last().sent.at(-1)).toEqual({ t: "unsubscribe" });
    expect(t.fleet.state.pane).toBeNull();
  });
});

const t0 = {
  status: "idle",
  attempt: 0,
  retryInMs: null,
  error: null,
  failure: null,
  address: null,
  path: null,
} as const;
