import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { questionKey } from "../src/answer";
import { toBase64Url } from "../src/noise/bytes";
import { ACK_TIMEOUT_MS, ASKED_TTL_MS, Fleet, MAX_ACTIVITY, canClose, canOperate, hasFeature, isAsked, ntfyUnsupported, wrapsLines, type Notifier } from "../src/fleet";
import { routingId } from "../src/protocol/relay";
import { LONG_COMMAND_MS } from "../src/notify";
import { parsePairLink, type PairLink } from "../src/pairing";
import { PUBLISH_INTERVAL_MS } from "../src/protocol/batch";
import { MAX_TEXT_BYTES, type Block, type Waiting } from "../src/protocol/types";
import { HostsStore, MemoryStore, loadLastView, loadPaneTitles, restoreTarget, type HostRecord } from "../src/stores";
import { terminalInput, textToActions } from "../src/keyboard";
import { showsTerminal } from "../src/terminal";
import { SCREEN_RESUME_MS } from "../src/protocol/client";
import { FakeGatewaySocket, gatewayKey } from "./helpers/gateway";
import { rng } from "./helpers/gateway";

const GW = gatewayKey();
const GW_KEY = toBase64Url(GW.publicKey);
const welcome = (permission: "operate" | "view" = "operate", features: string[] = []) =>
  ({ t: "welcome", v: 2, host: "studio-mac", os: "macos", version: "0.1.0", device: "d1", permission, features }) as const;
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
  const dismissed: { host: string; pane: number; ask: string }[] = [];
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
    dismissAsk: async (host, pane, ask) => {
      dismissed.push({ host, pane, ask });
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
    dismissed,
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

  it("keys go out and resolve on ack; a view device gets forbidden without sending", async () => {
    const t = await withHost();
    const p = t.fleet.sendKeys(GW_KEY, 12, ["ctrl+c"]);
    expect(t.last().sent.at(-1)).toEqual({ t: "keys", id: "k1", pane: 12, keys: ["ctrl+c"] });
    t.last().recv({ t: "ack", id: "k1", ok: false, error: "forbidden", message: "this device may only view" });
    expect(await p).toMatchObject({ ok: false, code: "forbidden" });
    const v = await withHost("view");
    const before = v.last().sent.length;
    expect(await v.fleet.sendKeys(GW_KEY, 12, ["ctrl+c"])).toMatchObject({ ok: false, code: "forbidden" });
    expect(v.last().sent.length).toBe(before);
  });

  it("open asks for a tab or pane and resolves with the new pane; an older Rift is told to update", async () => {
    const t = await withHost();
    // the welcome of withHost lists no features: nothing goes out
    const before = t.last().sent.length;
    const old = await t.fleet.openShell(GW_KEY, 12, "tab");
    expect(old).toMatchObject({ ok: false, code: "unsupported" });
    expect(old.ok ? "" : old.message).toContain("Update it");
    expect(t.last().sent.length).toBe(before);
    t.last().recv(welcome("operate", ["open"]));
    const p = t.fleet.openShell(GW_KEY, 12, "pane");
    const sent = t.last().sent.at(-1) as { id: string };
    expect(sent).toEqual({ t: "open", id: sent.id, pane: 12, kind: "pane" });
    t.last().recv({ t: "ack", id: sent.id, ok: true, pane: 14 });
    expect(await p).toEqual({ ok: true, pane: 14 });
    // the desktop's own refusal is not "running a command"
    const q = t.fleet.openShell(GW_KEY, 12, "tab");
    const id = (t.last().sent.at(-1) as { id: string }).id;
    t.last().recv({ t: "ack", id, ok: false, error: "busy", message: "cannot split this pane" });
    const r = await q;
    expect(r.ok ? "" : r.message).toBe("studio-mac could not open it. Try again.");
  });

  it("close ends a pane or its tab; offered and sent only by a gateway with `close` that this device may operate", async () => {
    const t = await withHost();
    // no `close` feature: not offered, nothing goes out
    expect(canClose(t.fleet.state.hosts[0])).toBe(false);
    const before = t.last().sent.length;
    const old = await t.fleet.closeShell(GW_KEY, 12, "pane");
    expect(old).toMatchObject({ ok: false, code: "unsupported" });
    expect(old.ok ? "" : old.message).toContain("Update it");
    expect(t.last().sent.length).toBe(before);
    t.last().recv(welcome("operate", ["close"]));
    expect(canClose(t.fleet.state.hosts[0])).toBe(true);
    expect(canClose(undefined)).toBe(false);
    const p = t.fleet.closeShell(GW_KEY, 12, "tab");
    const sent = t.last().sent.at(-1) as { id: string };
    expect(sent).toEqual({ t: "close", id: sent.id, pane: 12, kind: "tab" });
    t.last().recv({ t: "ack", id: sent.id, ok: true });
    expect(await p).toEqual({ ok: true });
    // a view-only device is not offered it even when the gateway serves it
    const v = await withHost("view");
    v.last().recv(welcome("view", ["close"]));
    expect(canClose(v.fleet.state.hosts[0])).toBe(false);
  });

  it("a closed pane's no_pane error is not shown as a notice; other errors are", async () => {
    const t = await withHost();
    t.last().recv({ t: "error", code: "no_pane", message: "no pane 999999" });
    expect(t.fleet.state.hosts[0]!.notice).toBeNull();
    t.last().recv({ t: "error", code: "bad_request", message: "pane 12 is not the subscribed pane" });
    expect(t.fleet.state.hosts[0]!.notice).toBe("pane 12 is not the subscribed pane");
  });

  it("forbidden for remote access turned off says so, not view only", async () => {
    const t = await withHost();
    const p = t.fleet.run(GW_KEY, 12, "ls");
    t.last().recv({ t: "ack", id: "r1", ok: false, error: "forbidden", message: "remote access is off on the desktop" });
    const r = await p;
    expect(r.ok ? "" : r.message).toBe("Remote access is off on studio-mac.");
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
    expect(t.fleet.state.activity[0]).toMatchObject({ block: MAX_ACTIVITY + 4, hostName: "studio-mac", pane: 12, tab: "rift" });
  });

  it("notifies only when enabled, long, and not watched; the command only with the setting", async () => {
    const t = await withHost();
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified).toHaveLength(0); // notifications are off by default

    await t.fleet.updateSettings({ notifyPolicy: "all" });
    expect(t.permissionAsks).toBe(1);
    t.fleet.setViewing({ host: GW_KEY, pane: 12 });
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified).toHaveLength(0); // watching it

    t.fleet.setAppActive(false);
    t.last().recv(done({ exit: 2 }));
    await Promise.resolve();
    expect(t.notified).toEqual([
      { title: "✕ 2 · A command", subtitle: "Failed after 30s · studio-mac · rift", body: "", data: { host: GW_KEY, pane: 12 } },
    ]);

    await t.fleet.updateSettings({ showCommandInNotification: true });
    t.fleet.setAppActive(true);
    t.fleet.setViewing(null);
    t.last().recv(done());
    t.last().recv(done({ duration_ms: 5_000 })); // short: not notified
    await Promise.resolve();
    expect(t.notified.slice(1)).toEqual([
      { title: "✓ 0 · cargo test", subtitle: "Finished in 30s · studio-mac · rift", body: "", data: { host: GW_KEY, pane: 12 } },
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
    expect(pushes()).toEqual([{ t: "push", token: null, long_ms: null, show_command: false, failures_only: false }]);

    t.fleet.setPushToken(TOKEN);
    expect(t.fleet.state.push).toEqual({ state: "on" });
    expect(pushes().at(-1)).toEqual({ t: "push", token: TOKEN, long_ms: null, show_command: false, failures_only: false });
    await t.fleet.updateSettings({ notifyPolicy: "all", showCommandInNotification: true });
    expect(pushes().at(-1)).toEqual({ t: "push", token: TOKEN, long_ms: LONG_COMMAND_MS, show_command: true, failures_only: false });

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
    expect(pushes().at(-1)).toEqual({ t: "push", token: TOKEN, long_ms: LONG_COMMAND_MS, show_command: true, failures_only: false });

    // no token any more: the computers stop, the app notifies again
    t.fleet.setPushToken(null, "no push here");
    expect(t.fleet.state.push).toEqual({ state: "unavailable", reason: "no push here" });
    expect(pushes().at(-1)).toEqual({ t: "push", token: null, long_ms: LONG_COMMAND_MS, show_command: true, failures_only: false });
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
    await t.fleet.updateSettings({ notifyPolicy: "all" });
    expect(t.fleet.state.settings.notifyPolicy).toBe("off");
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
    const base = { conn: t0, gateway: null, windows: [], topology: false, notice: null, lastSeen: null };
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

  it.each([false, true])("an unpair while a record save is in flight is not undone by that save (write fails: %s)", async (fails) => {
    const t = await withHost();
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const set = t.kv.set.bind(t.kv);
    t.kv.set = async (k, v) => {
      if (v.includes("renamed-mac")) {
        await gate;
        if (fails) throw new Error("keychain unavailable");
      }
      return set(k, v);
    };
    reconnect(t, { ...welcome(), host: "renamed-mac" }); // the rename starts a save, held back
    await settle();
    // Unpair waits behind the metadata write: start it now, release the write, then await both.
    const unpairing = t.fleet.unpair(GW_KEY);
    release();
    await unpairing;
    await settle();
    expect(await new HostsStore(t.kv).load()).toEqual([]);
    expect(t.fleet.state.hosts).toEqual([]);
    expect(t.kv.data.has(`rift.host.${GW_KEY}`)).toBe(false);
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
  paths: [] as never[],
  since: null,
  retryAt: null,
} as const;

describe("Fleet Phase 3", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const ALL = ["screen", "text", "output", "push_failures"];
  /** A connected host following pane 12, its gateway serving `features`. */
  async function phase3(features: string[] = ALL, blocks: Block[] = [block(86, { dropped: 1234, output: "tail\n" })]) {
    const t = setup();
    await new HostsStore(t.kv).save(record());
    await t.fleet.init();
    t.last().connect();
    t.last().recv(welcome("operate", features));
    t.last().recv(topology());
    t.fleet.openPane(GW_KEY, 12);
    t.last().recv({ t: "snapshot", pane: 12, seq: 41, blocks, has_older: false });
    t.fleet.store.flush();
    return t;
  }
  const screen = (patch: Record<string, unknown> = {}) => ({
    t: "screen",
    pane: 12,
    cols: 80,
    rows: 2,
    alt: true,
    cursor: [1, 0],
    full: true,
    lines: [
      [0, [["vim", null, null, 0]]],
      [1, []],
    ],
    ...patch,
  });
  const sent = (t: Awaited<ReturnType<typeof phase3>>, type: string) => t.last().sent.filter((m) => m.t === type);

  it("hasFeature reads the welcome; an older gateway (no features) serves none", async () => {
    const t = await phase3();
    expect(hasFeature(t.fleet.state.hosts[0], "screen")).toBe(true);
    const old = await phase3([]);
    expect(hasFeature(old.fleet.state.hosts[0], "screen")).toBe(false);
    expect(hasFeature(undefined, "text")).toBe(false);
  });

  it("asks for the screen only from a gateway that serves it", async () => {
    const old = await phase3([]);
    old.fleet.setScreenMode(GW_KEY, 12, "auto");
    expect(sent(old, "screen")).toEqual([]);
    const t = await phase3();
    t.fleet.setScreenMode(GW_KEY, 13, "auto"); // not the followed pane
    t.fleet.setScreenMode(GW_KEY, 12, "auto");
    expect(sent(t, "screen")).toEqual([{ t: "screen", pane: 12, mode: "auto" }]);
  });

  it("keeps the followed pane's screen next to its blocks: full, diff, end", async () => {
    const t = await phase3();
    t.fleet.setScreenMode(GW_KEY, 12, "auto");
    t.last().recv(screen());
    t.last().recv(screen({ full: false, cursor: null, lines: [[1, [[":q", null, null, 0]]]] }));
    t.fleet.store.flush();
    expect(t.fleet.state.pane?.screen).toMatchObject({ cols: 80, alt: true, cursor: null });
    expect(t.fleet.state.pane?.screen?.lines).toEqual([[["vim", null, null, 0]], [[":q", null, null, 0]]]);
    expect(t.fleet.state.pane?.state?.blocks).toHaveLength(1);
    t.last().recv({ t: "screen_end", pane: 12 });
    t.fleet.store.flush();
    expect(t.fleet.state.pane?.screen).toBeNull();
  });

  it("a screen for another pane is ignored; opening another pane drops the screen", async () => {
    const t = await phase3();
    t.fleet.setScreenMode(GW_KEY, 12, "auto");
    t.last().recv(screen({ pane: 13 }));
    t.fleet.store.flush();
    expect(t.fleet.state.pane?.screen).toBeNull();
    t.last().recv(screen());
    t.fleet.openPane(GW_KEY, 13);
    t.fleet.store.flush();
    expect(t.fleet.state.pane).toMatchObject({ paneId: 13, screen: null, earlier: {} });
  });

  it("text goes out and resolves on ack; at a prompt the gateway answers busy", async () => {
    const t = await phase3();
    const p = t.fleet.sendText(GW_KEY, 12, ":wq");
    expect(t.last().sent.at(-1)).toEqual({ t: "text", id: "x1", pane: 12, text: ":wq" });
    t.last().recv({ t: "ack", id: "x1", ok: false, error: "busy", message: "pane 12 is at its prompt" });
    expect(await p).toMatchObject({ ok: false, code: "busy" });
  });

  it("fit goes out and resolves on ack; a release sends nulls", async () => {
    const t = await phase3();
    const p = t.fleet.sendFit(GW_KEY, 12, { cols: 64, rows: 40 });
    expect(t.last().sent.at(-1)).toEqual({ t: "fit", id: "f1", pane: 12, cols: 64, rows: 40 });
    t.last().recv({ t: "ack", id: "f1", ok: true });
    expect(await p).toMatchObject({ ok: true });
    void t.fleet.sendFit(GW_KEY, 12, null);
    expect(t.last().sent.at(-1)).toEqual({ t: "fit", id: "f2", pane: 12, cols: null, rows: null });
  });

  it("loads earlier output 500 lines at a time, before the tail", async () => {
    const t = await phase3();
    const p = t.fleet.loadEarlier(86);
    t.fleet.store.flush();
    expect(t.fleet.state.pane?.earlier[86]).toMatchObject({ loading: true });
    expect(t.last().sent.at(-1)).toEqual({ t: "output", id: "o1", pane: 12, block: 86, from: 734, limit: 500 });
    const text = Array.from({ length: 500 }, (_, i) => `line ${734 + i}\n`).join("");
    t.last().recv({ t: "output", id: "o1", pane: 12, block: 86, from: 734, text, total: 1235 });
    await p;
    expect(t.fleet.state.pane?.earlier[86]).toMatchObject({ from: 734, end: 1234, gone: false, loading: false, text });
    void t.fleet.loadEarlier(86);
    expect(t.last().sent.at(-1)).toMatchObject({ t: "output", from: 234, limit: 500 });
  });

  it("says when the earlier lines are gone, and when the block is", async () => {
    const t = await phase3();
    const p = t.fleet.loadEarlier(86);
    t.last().recv({ t: "output", id: "o1", pane: 12, block: 86, from: 1000, text: "a\n".repeat(234), total: 1235 });
    await p;
    expect(t.fleet.state.pane?.earlier[86]).toMatchObject({ from: 1000, end: 1234, gone: true });
    const t2 = await phase3();
    const p2 = t2.fleet.loadEarlier(86);
    t2.last().recv({ t: "ack", id: "o1", ok: false, error: "no_block", message: "block 86 is not in pane 12" });
    await p2;
    expect(t2.fleet.state.pane?.earlier[86]?.error).toBe("This block is gone from the computer.");
  });

  it("drops the loaded lines with their block, and an answer for a block that went", async () => {
    const t = await phase3(ALL, [block(86, { dropped: 600 }), block(87, { dropped: 10 })]);
    const p = t.fleet.loadEarlier(86);
    t.last().recv({ t: "output", id: "o1", pane: 12, block: 86, from: 100, text: "x\n".repeat(500), total: 700 });
    await p;
    expect(Object.keys(t.fleet.state.pane!.earlier)).toEqual(["86"]);
    t.last().recv({ t: "update", pane: 12, seq: 42, blocks: [], removed: [86] });
    t.fleet.store.flush();
    expect(t.fleet.state.pane?.earlier).toEqual({});
    const p2 = t.fleet.loadEarlier(87);
    t.last().recv({ t: "update", pane: 12, seq: 43, blocks: [], removed: [87] });
    t.last().recv({ t: "output", id: "o2", pane: 12, block: 87, from: 0, text: "y\n".repeat(10), total: 20 });
    await p2;
    expect(t.fleet.state.pane?.earlier).toEqual({});
  });

  it("an older gateway: no earlier output requests", async () => {
    const t = await phase3([]);
    await t.fleet.loadEarlier(86);
    expect(sent(t, "output")).toEqual([]);
  });

  it("push follows the policy, Notify after and the computer's switch", async () => {
    const TOKEN = "ExponentPushToken[abc-123]";
    const t = await phase3();
    t.fleet.setPushToken(TOKEN);
    const last = () => sent(t, "push").at(-1);
    expect(last()).toMatchObject({ long_ms: null, failures_only: false });
    await t.fleet.updateSettings({ notifyPolicy: "failures", notifyAfterMs: 10_000 });
    expect(last()).toEqual({ t: "push", token: TOKEN, long_ms: 10_000, show_command: false, failures_only: true });
    await t.fleet.setHostNotifications(GW_KEY, false);
    expect(last()).toMatchObject({ long_ms: null, failures_only: true });
    expect((await new HostsStore(t.kv).load())[0]?.notifications).toBe(false);
    await t.fleet.setHostNotifications(GW_KEY, true);
    expect(last()).toMatchObject({ long_ms: 10_000 });
    expect((await new HostsStore(t.kv).load())[0]?.notifications).toBeUndefined();
  });

  it("the app's own notifications follow the same rules", async () => {
    const t = await phase3();
    await t.fleet.updateSettings({ notifyPolicy: "failures", notifyAfterMs: 60_000 });
    const done = (exit: number | null, duration_ms: number, command = "make") => ({
      t: "done",
      pane: 12,
      block: 90,
      command,
      exit,
      duration_ms,
    });
    t.last().recv(done(0, 120_000)); // succeeded
    t.last().recv(done(2, 30_000)); // failed, but shorter than a minute
    t.last().recv(done(2, 60_000)); // failed and long: notifies
    await Promise.resolve();
    expect(t.notified.map((n) => n.title)).toEqual(["✕ 2 · A command"]);
    await t.fleet.setHostNotifications(GW_KEY, false);
    t.last().recv(done(3, 600_000));
    await Promise.resolve();
    expect(t.notified).toHaveLength(1);
    // "Notify me" always notifies, muted or not
    t.fleet.notifyWhenDone(GW_KEY, 12, "short");
    t.last().recv(done(0, 10, "short"));
    await Promise.resolve();
    expect(t.notified).toHaveLength(2);
  });

  it("a view device's refusal: an ack forbidden, or an older gateway's top-level error, fails at once", async () => {
    const t = await phase3(ALL);
    const p1 = t.fleet.sendText(GW_KEY, 12, "x");
    t.last().recv({ t: "ack", id: "x1", ok: false, error: "forbidden", message: "this device may only view" });
    expect(await p1).toMatchObject({ ok: false, code: "forbidden" });
    const p2 = t.fleet.sendKeys(GW_KEY, 12, ["esc"]);
    t.last().recv({ t: "error", code: "forbidden", message: "this device may only view" });
    expect(await p2).toMatchObject({ ok: false, code: "forbidden" });
  });

  it("Run again keeps the draft the user is typing", async () => {
    const t = await phase3();
    t.fleet.setDraft(GW_KEY, 12, "half typed");
    const p = t.fleet.run(GW_KEY, 12, "cargo build", { fromDraft: false });
    t.last().recv({ t: "ack", id: "r1", ok: true });
    expect(await p).toEqual({ ok: true });
    expect(t.fleet.state.drafts[`${GW_KEY}:12`]).toBe("half typed");
  });
});

describe("Fleet: push coverage, pending Notify me, stale screens, input pacing", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const TOKEN = "ExponentPushToken[abc-123]";
  const ALL = ["screen", "text", "output", "push_failures"];
  async function host(features: string[] = ALL) {
    const t = setup();
    await new HostsStore(t.kv).save(record());
    await t.fleet.init();
    t.last().connect();
    t.last().recv(welcome("operate", features));
    t.last().recv(topology());
    t.fleet.openPane(GW_KEY, 12);
    t.last().recv({ t: "snapshot", pane: 12, seq: 41, blocks: [block(5, { command: "vim", state: "running" })], has_older: false });
    t.fleet.store.flush();
    return t;
  }
  const done = (patch: Record<string, unknown> = {}) => ({
    t: "done",
    pane: 12,
    block: 9,
    command: "make",
    exit: 2,
    duration_ms: 600_000,
    ...patch,
  });

  describe("ntfy", () => {
    const NTFY_ALL = [...ALL, "push_ntfy"];
    const pushes = (t: Awaited<ReturnType<typeof host>>) => t.last().sent.filter((m) => m.t === "push");

    it("on: a random topic on ntfy.sh, sent in every push to a push_ntfy host; the done is left to it", async () => {
      const t = await host(NTFY_ALL);
      await t.fleet.updateSettings({ notifyPolicy: "failures", notifyAfterMs: 10_000 });
      await t.fleet.setNtfy(true);
      const topic = t.fleet.state.settings.ntfyTopic!;
      expect(topic).toMatch(/^rift-[A-Za-z0-9_-]{22}$/);
      expect(pushes(t).at(-1)).toEqual({
        t: "push",
        token: null,
        ntfy: `https://ntfy.sh/${topic}`,
        long_ms: 10_000,
        show_command: false,
        failures_only: true,
      });
      // no Expo token: the ntfy registration covers a pane this device does not follow, so the
      // app posts no duplicate
      t.fleet.setAppActive(false);
      t.fleet.setVisible(false);
      const visible = () => t.last().sent.filter((m) => m.t === "visible");
      expect(visible()).toEqual([{ t: "visible", on: false }]);
      t.last().recv(done({ pane: 13 }));
      await Promise.resolve();
      expect(t.notified).toEqual([]);
      // in the background the computer posts about the followed pane too (the app is not visible)
      t.last().recv(done());
      await Promise.resolve();
      expect(t.notified).toEqual([]);
      // on screen, following the pane from another screen: the computer sends no ntfy alert
      // about it, so the app notifies itself
      t.fleet.setAppActive(true);
      t.fleet.setVisible(true);
      expect(visible().at(-1)).toEqual({ t: "visible", on: true });
      t.fleet.setViewing(null);
      t.last().recv(done());
      await Promise.resolve();
      expect(t.notified.map((n) => n.data)).toEqual([{ host: GW_KEY, pane: 12 }]);
      // ... and stays quiet while the pane is on screen
      t.fleet.setViewing({ host: GW_KEY, pane: 12 });
      t.last().recv(done());
      await Promise.resolve();
      expect(t.notified).toHaveLength(1);
      t.fleet.setAppActive(false);
      // a Notify me goes to the computer too
      t.fleet.notifyWhenDone(GW_KEY, 12, "make");
      expect(t.last().sent.filter((m) => m.t === "notify")).toEqual([{ t: "notify", pane: 12 }]);
      // with a token: ntfy replaces Expo push here (one alert per command, not two)
      t.fleet.setPushToken(TOKEN);
      expect(pushes(t).at(-1)).toMatchObject({ token: null, ntfy: `https://ntfy.sh/${topic}` });
    });

    it("inactive (a Face ID prompt) is still on screen: no visible false until background", async () => {
      const t = await host(NTFY_ALL);
      t.fleet.setAppActive(false);
      expect(t.last().sent.filter((m) => m.t === "visible")).toEqual([]);
    });

    it("New topic: a new one goes to every computer at once", async () => {
      const t = await host(NTFY_ALL);
      await t.fleet.setNtfy(true);
      const before = t.fleet.state.settings.ntfyTopic!;
      await t.fleet.rotateNtfyTopic();
      const after = t.fleet.state.settings.ntfyTopic!;
      expect(after).toMatch(/^rift-[A-Za-z0-9_-]{22}$/);
      expect(after).not.toBe(before);
      expect(pushes(t).at(-1)).toMatchObject({ ntfy: `https://ntfy.sh/${after}` });
    });

    it("lists the computers too old for ntfy by their last welcome", async () => {
      const t = await host(ALL);
      expect(ntfyUnsupported(t.fleet.state.hosts).map((h) => h.record.name)).toEqual([t.fleet.state.hosts[0]!.record.name]);
      const n = await host(NTFY_ALL);
      expect(ntfyUnsupported(n.fleet.state.hosts)).toEqual([]);
      // not reached yet: unknown, not listed
      expect(ntfyUnsupported([{ ...t.fleet.state.hosts[0]!, welcome: null }])).toEqual([]);
    });

    it("visible: sent on every change and after a welcome while not on screen; never to an older gateway", async () => {
      const t = await host(NTFY_ALL);
      const visible = () => t.last().sent.filter((m) => m.t === "visible");
      expect(visible()).toEqual([]); // on screen: the default, nothing to say
      t.fleet.setVisible(false);
      t.fleet.setVisible(false);
      expect(visible()).toEqual([{ t: "visible", on: false }]);
      reconnect(t, welcome("operate", NTFY_ALL));
      expect(visible()).toEqual([{ t: "visible", on: false }]);
      t.fleet.setVisible(true);
      expect(visible()).toEqual([
        { t: "visible", on: false },
        { t: "visible", on: true },
      ]);
      reconnect(t, welcome("operate", NTFY_ALL));
      expect(visible()).toEqual([]);
      // a gateway without push_ntfy never gets it
      const old = await host(ALL);
      old.fleet.setVisible(false);
      reconnect(old, welcome("operate", ALL));
      old.fleet.setVisible(true);
      expect(old.last().sent.filter((m) => m.t === "visible")).toEqual([]);
    });

    it("a self-hosted server changes the URL; a bad one is refused and kept", async () => {
      const t = await host(NTFY_ALL);
      await t.fleet.setNtfy(true);
      const topic = t.fleet.state.settings.ntfyTopic!;
      expect(await t.fleet.setNtfyServer("ntfy.example.com:8443/")).toBe(true);
      expect(pushes(t).at(-1)).toMatchObject({ ntfy: `https://ntfy.example.com:8443/${topic}` });
      const n = pushes(t).length;
      expect(await t.fleet.setNtfyServer("http://ntfy.example.com")).toBe(false);
      expect(t.fleet.state.settings.ntfyServer).toBe("https://ntfy.example.com:8443");
      expect(pushes(t)).toHaveLength(n);
    });

    it("off: the computer is told to stop, and the app notifies by itself again; on again reuses the topic", async () => {
      const t = await host(NTFY_ALL);
      await t.fleet.updateSettings({ notifyPolicy: "all", notifyAfterMs: 10_000 });
      await t.fleet.setNtfy(true);
      const topic = t.fleet.state.settings.ntfyTopic!;
      await t.fleet.setNtfy(false);
      expect(t.fleet.state.settings.ntfyEnabled).toBe(false);
      expect(t.fleet.state.settings.ntfyTopic).toBe(topic);
      expect(pushes(t).at(-1)).toMatchObject({ token: null, ntfy: null });
      t.fleet.setAppActive(false);
      t.last().recv(done({ pane: 13 }));
      await Promise.resolve();
      expect(t.notified.map((n) => n.title)).toEqual(["✕ 2 · A command"]);
      // on again: the same topic, so the ntfy app needs no new subscription
      await t.fleet.setNtfy(true);
      expect(t.fleet.state.settings.ntfyTopic).toBe(topic);
      expect(pushes(t).at(-1)).toMatchObject({ ntfy: `https://ntfy.sh/${topic}` });
    });

    it("a computer without push_ntfy keeps the Expo token while ntfy is on", async () => {
      const t = await host(ALL);
      await t.fleet.setNtfy(true);
      t.fleet.setPushToken(TOKEN);
      expect(pushes(t).at(-1)).toMatchObject({ token: TOKEN });
      expect(pushes(t).at(-1)).not.toHaveProperty("ntfy");
    });

    it("a gateway without push_ntfy gets push as before (no ntfy field) and does not cover the done", async () => {
      const t = await host(ALL);
      await t.fleet.updateSettings({ notifyPolicy: "all", notifyAfterMs: 10_000 });
      await t.fleet.setNtfy(true);
      for (const m of pushes(t)) expect(m).not.toHaveProperty("ntfy");
      t.fleet.setAppActive(false);
      t.last().recv(done());
      await Promise.resolve();
      expect(t.notified.map((n) => n.title)).toEqual(["✕ 2 · A command"]);
    });
  });

  it("a push host that will not alert for this done (no long_ms: no push_failures) leaves it to the app", async () => {
    const t = await host(["screen", "text", "output"]);
    t.fleet.setPushToken(TOKEN);
    await t.fleet.updateSettings({ notifyPolicy: "failures", notifyAfterMs: 10_000 });
    t.fleet.setAppActive(false);
    expect(t.last().sent.filter((m) => m.t === "push").at(-1)).toMatchObject({ token: TOKEN, long_ms: null });
    t.last().recv(done());
    await Promise.resolve();
    expect(t.notified.map((n) => n.title)).toEqual(["✕ 2 · A command"]);
  });

  it("a push host whose registration covers the done alerts alone; one it does not cover leaves it to the app", async () => {
    const t = await host();
    t.fleet.setPushToken(TOKEN);
    await t.fleet.updateSettings({ notifyPolicy: "failures", notifyAfterMs: 10_000 });
    t.fleet.setAppActive(false);
    t.last().recv(done()); // failed and long: the computer alerts
    await Promise.resolve();
    expect(t.notified).toEqual([]);
    await t.fleet.updateSettings({ notifyPolicy: "all" });
    t.last().recv(done({ exit: 0, duration_ms: 9_000 })); // short: nobody alerts
    t.last().recv(done({ exit: 0, duration_ms: 20_000 })); // long success, policy all: the computer
    await Promise.resolve();
    expect(t.notified).toEqual([]);
  });

  it("Notify me while disconnected: sent after the next welcome; until then the app notifies itself", async () => {
    const t = await host();
    t.fleet.setPushToken(TOKEN);
    t.fleet.setAppActive(false);
    t.last().drop();
    t.fleet.notifyWhenDone(GW_KEY, 12, "make");
    // not sent: the app keeps it and alerts on the done itself
    t.fleet.notifyWhenDone(GW_KEY, 12, "short");
    await vi.advanceTimersByTimeAsync(1000);
    t.last().connect();
    t.last().recv(welcome("operate", ALL));
    expect(t.last().sent.filter((m) => m.t === "notify")).toEqual([
      { t: "notify", pane: 12 },
      { t: "notify", pane: 12 },
    ]);
    // sent now: the computer alerts for it
    t.last().recv(done({ exit: 0, duration_ms: 1_000 }));
    await Promise.resolve();
    expect(t.notified).toEqual([]);
  });

  it("a Notify me the gateway never received is notified by the app", async () => {
    const t = await host();
    t.fleet.setPushToken(TOKEN);
    t.fleet.setAppActive(false);
    t.last().drop();
    t.fleet.notifyWhenDone(GW_KEY, 12, "make");
    // the done arrives on a connection that has not had its welcome resend yet: not covered
    t.fleet["onDone"](GW_KEY, done({ exit: 0, duration_ms: 1_000 }) as never);
    await Promise.resolve();
    expect(t.notified.map((n) => n.title)).toEqual(["✓ 0 · A command"]);
  });

  it("a reconnect while a full-screen program shows keeps its screen on", async () => {
    const t = await host();
    t.fleet.setScreenMode(GW_KEY, 12, "auto");
    t.last().recv({ t: "screen", pane: 12, cols: 80, rows: 1, alt: true, cursor: null, full: true, lines: [[0, [["vim", null, null, 0]]]] });
    t.fleet.store.flush();
    reconnect(t, welcome("operate", ALL));
    t.fleet.store.flush();
    const screen = t.fleet.state.pane!.screen;
    expect(screen).toMatchObject({ alt: true, stale: true });
    expect(screen!.lines).toEqual([[["vim", null, null, 0]]]);
    expect(showsTerminal(screen, true)).toBe(true); // the screen view stays mounted
    // the next full screen is current again
    t.last().recv({ t: "screen", pane: 12, cols: 80, rows: 1, alt: true, cursor: null, full: true, lines: [[0, [["vim", null, null, 0]]]] });
    t.fleet.store.flush();
    expect(t.fleet.state.pane!.screen).toMatchObject({ stale: false });
  });

  it("a screen that does not come back after a reconnect ends, and so does a real end: blocks again", async () => {
    const t = await host();
    t.fleet.setScreenMode(GW_KEY, 12, "auto");
    t.last().recv({ t: "screen", pane: 12, cols: 80, rows: 1, alt: true, cursor: null, full: true, lines: [] });
    reconnect(t, welcome("operate", ALL));
    vi.advanceTimersByTime(SCREEN_RESUME_MS);
    t.fleet.store.flush();
    const p = t.fleet.state.pane!;
    expect(p.screen).toBeNull();
    expect(p.screenExpired).toBe(true);
    expect(showsTerminal(p.screen, true)).toBe(false); // back to blocks
    // the gateway's own screen_end resets it
    t.last().recv({ t: "screen", pane: 12, cols: 80, rows: 1, alt: true, cursor: null, full: true, lines: [] });
    t.last().recv({ t: "screen_end", pane: 12 });
    t.fleet.store.flush();
    const q = t.fleet.state.pane!;
    expect(q.screenExpired).toBe(false);
    expect(showsTerminal(q.screen, true)).toBe(false);
  });

  it("a 15-line paste in the Terminal view never exceeds 20 sends in any second, loses nothing, keeps the order", async () => {
    const t = await host();
    const b = terminalInput({
      sendKeys: (keys) => t.fleet.sendKeys(GW_KEY, 12, keys),
      sendText: (text) => t.fleet.sendText(GW_KEY, 12, text),
      online: () => true,
      delay: () => t.fleet.inputDelay(GW_KEY),
      authorize: () => null,
      report: () => undefined,
      schedule: (fn, d) => setTimeout(fn, d > 0 ? d : 16),
      now: () => Date.now(),
    });
    // the wire: when each keys/text message reached the (0 ms) gateway
    const at: number[] = [];
    const socket = t.last();
    const send = socket.send.bind(socket);
    socket.send = (data: ArrayBuffer) => {
      const n = socket.sent.length;
      send(data);
      for (let i = n; i < socket.sent.length; i++) if (["keys", "text"].includes(socket.sent[i]!.t)) at.push(Date.now());
    };
    const lines = Array.from({ length: 15 }, (_, i) => `line${i}`);
    b.push(...textToActions(lines.join("\n") + "\n"));
    // a Stop from the block sheet meanwhile shares the same budget
    void t.fleet.sendKeys(GW_KEY, 12, ["ctrl+c"]);
    await vi.advanceTimersByTimeAsync(5_000);
    const input = socket.sent.filter((m) => m.t === "keys" || m.t === "text");
    for (let i = 0; i < at.length; i++) expect(at.filter((x) => x >= at[i]! && x < at[i]! + 1000).length).toBeLessThanOrEqual(20);
    const typed = input.flatMap((m) => (m.t === "text" ? [m.text] : m.t === "keys" ? m.keys.map((k) => `<${k}>`) : []));
    expect(typed.filter((x) => x !== "<ctrl+c>")).toEqual(lines.flatMap((l) => [l, "<enter>"]));
    expect(typed.filter((x) => x === "<ctrl+c>")).toHaveLength(1);
  });
});

describe("Fleet: the redesign's run, bell, wrap and last view", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const doneMsg = (command: string, extra: Record<string, unknown> = {}) => ({
    t: "done",
    pane: 12,
    block: 87,
    command,
    exit: 0,
    duration_ms: 1_000,
    ...extra,
  });

  it("every command run from this phone asks to be told by default, however short", async () => {
    const t = await withHost();
    expect(t.fleet.state.settings).toMatchObject({ notifyMyCommands: true, notifyPolicy: "off" });
    const p = t.fleet.run(GW_KEY, 12, "make");
    expect(isAsked(t.fleet.state, GW_KEY, 12, "make")).toBe(true);
    t.last().recv({ t: "ack", id: "r1", ok: true });
    await p;
    t.last().recv(doneMsg("make"));
    await Promise.resolve();
    expect(t.notified.map((n) => n.title)).toEqual(["✓ 0 · A command"]);
    expect(isAsked(t.fleet.state, GW_KEY, 12, "make")).toBe(false);
  });

  it("with Notify me when my commands finish off, a run asks nothing", async () => {
    const t = await withHost();
    await t.fleet.updateSettings({ notifyMyCommands: false });
    const p = t.fleet.run(GW_KEY, 12, "make");
    expect(t.fleet.state.asked).toEqual([]);
    t.last().recv({ t: "ack", id: "r1", ok: true });
    await p;
    t.last().recv(doneMsg("make"));
    await Promise.resolve();
    expect(t.notified).toEqual([]);
  });

  it("the bell turns one command off and on again", async () => {
    const t = await withHost();
    void t.fleet.run(GW_KEY, 12, "make");
    t.fleet.setNotify(GW_KEY, 12, "make", false);
    expect(isAsked(t.fleet.state, GW_KEY, 12, "make")).toBe(false);
    t.last().recv(doneMsg("make"));
    await Promise.resolve();
    expect(t.notified).toEqual([]);
    // on again for the next one (a block not run from here, too)
    t.fleet.setNotify(GW_KEY, 12, "sleep 9", true);
    t.fleet.setNotify(GW_KEY, 12, "sleep 9", true);
    expect(t.fleet.state.asked).toEqual([{ host: GW_KEY, pane: 12, command: "sleep 9", at: 1_000_000 }]);
    t.last().recv(doneMsg("sleep 9"));
    await Promise.resolve();
    expect(t.notified).toHaveLength(1);
  });

  it("switched off, a computer with notify_off forgets the wait; an older one is not told", async () => {
    for (const features of [["push_ntfy", "notify_off"], ["push_ntfy"]]) {
      const t = setup();
      await new HostsStore(t.kv).save(record());
      await t.fleet.init();
      await t.fleet.setNtfy(true);
      t.last().connect();
      t.last().recv(welcome("operate", features));
      t.last().recv(topology());
      t.fleet.notifyWhenDone(GW_KEY, 12, "make");
      expect(t.last().sent.filter((m) => m.t === "notify")).toEqual([{ t: "notify", pane: 12 }]);
      t.fleet.setNotify(GW_KEY, 12, "make", false);
      expect(t.last().sent.filter((m) => m.t === "notify")).toEqual(
        features.includes("notify_off") ? [{ t: "notify", pane: 12 }, { t: "notify", pane: 12, off: true }] : [{ t: "notify", pane: 12 }],
      );
      t.fleet.shutdown();
    }
  });

  it("a pane's wrap switch overrides Settings › Wrap long lines for that pane only", async () => {
    const t = await withHost();
    expect(wrapsLines(t.fleet.state, GW_KEY, 12)).toBe(true);
    t.fleet.setPaneWrap(GW_KEY, 12, false);
    expect(wrapsLines(t.fleet.state, GW_KEY, 12)).toBe(false);
    expect(wrapsLines(t.fleet.state, GW_KEY, 13)).toBe(true);
    await t.fleet.updateSettings({ wrapLines: false });
    expect(wrapsLines(t.fleet.state, GW_KEY, 13)).toBe(false);
    t.fleet.setPaneWrap(GW_KEY, 13, true);
    expect(wrapsLines(t.fleet.state, GW_KEY, 13)).toBe(true);
  });

  it("keeps the last host and pane for the next cold start, and drops it with the host", async () => {
    const t = await withHost();
    await t.fleet.setLastView({ host: GW_KEY, pane: 12 });
    // a cold start: a new fleet over the same storage
    const again = new Fleet({ kv: t.kv, randomBytes: rng, platform: "ios", systemDeviceName: "x", notifier: { requestPermission: async () => true, notify: async () => undefined }, socket: () => new FakeGatewaySocket("ws://x", { key: GW }) });
    await again.init();
    expect(again.state.lastView).toEqual({ host: GW_KEY, pane: 12 });
    expect(restoreTarget(again.state.lastView, again.state.hosts.map((h) => h.record))).toEqual({ host: GW_KEY, pane: 12 });
    again.shutdown();
    await t.fleet.unpair(GW_KEY);
    expect(t.fleet.state.lastView).toBeNull();
    expect(await loadLastView(t.kv)).toBeNull();
  });

  it("remembers the computer's OS, version and when it was last seen", async () => {
    const t = await withHost();
    await vi.waitFor(async () => expect((await new HostsStore(t.kv).load())[0]).toMatchObject({ os: "macos", version: "0.1.0", seenAt: 1_000_000 }));
    t.fleet.store.flush();
    expect(t.fleet.state.hosts[0]!.lastSeen).toBe(1_000_000);
    t.advanceNow(5_000);
    t.last().drop();
    t.fleet.store.flush();
    expect(t.fleet.state.hosts[0]!.lastSeen).toBe(1_005_000);
  });

  it("keeps each pane's tab title for the next cold start, and drops them with the host", async () => {
    const t = await withHost();
    expect(t.fleet.state.paneTitles).toEqual({ [`${GW_KEY}:12`]: "rift", [`${GW_KEY}:13`]: "rift" });
    await vi.waitFor(async () => expect(await loadPaneTitles(t.kv)).toEqual(t.fleet.state.paneTitles));
    // a cold start: the title is there before any topology
    const again = new Fleet({ kv: t.kv, randomBytes: rng, platform: "ios", systemDeviceName: "x", notifier: { requestPermission: async () => true, notify: async () => undefined }, socket: () => new FakeGatewaySocket("ws://x", { key: GW }) });
    await again.init();
    expect(again.state.hosts[0]!.windows).toEqual([]);
    expect(again.state.paneTitles[`${GW_KEY}:12`]).toBe("rift");
    again.shutdown();
    // a renamed tab replaces it; a pane that closed leaves the cache
    t.last().recv({ ...topology(1, [12]), windows: [{ ...topology(1, [12]).windows[0]!, tabs: [{ ...topology(1, [12]).windows[0]!.tabs[0]!, title: "orbit" }] }] });
    t.fleet.store.flush();
    expect(t.fleet.state.paneTitles).toEqual({ [`${GW_KEY}:12`]: "orbit" });
    await t.fleet.unpair(GW_KEY);
    expect(t.fleet.state.paneTitles).toEqual({});
    expect(await loadPaneTitles(t.kv)).toEqual({});
  });

  it("refreshPane asks the followed pane again, and only that one", async () => {
    const t = await withHost();
    const subs = () => t.last().sent.filter((m) => m.t === "subscribe").length;
    const before = subs();
    t.fleet.refreshPane(GW_KEY, 13);
    expect(subs()).toBe(before);
    t.fleet.refreshPane(GW_KEY, 12);
    expect(subs()).toBe(before + 1);
  });
});

describe("Fleet: prompt chip menus and diffs (feature chips)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("asks for a menu and settles with the matching answer only", async () => {
    const t = await withHost();
    const p = t.fleet.chipMenu(GW_KEY, 12, "dirs");
    const req = t.last().sent.at(-1) as { t: string; id: string; pane: number };
    expect(req).toMatchObject({ t: "dirs", pane: 12 });
    // an answer of another kind, or for another pane, with the same id is bad data
    t.last().recv({ t: "branches", id: req.id, pane: 12, current: "main", branches: ["main"], more: false });
    expect(await p).toEqual({ ok: false, message: "The computer's answer did not fit the request." });
    const q = t.fleet.chipMenu(GW_KEY, 12, "dirs");
    const id = (t.last().sent.at(-1) as { id: string }).id;
    const answer = { t: "dirs", id, pane: 12, cwd: "/r", parent: true, dirs: ["src"], more: false };
    t.last().recv(answer);
    expect(await q).toEqual({ ok: true, msg: answer });
  });

  it("a refused diff, a lost connection and silence each say why", async () => {
    const t = await withHost();
    const p = t.fleet.fileDiff(GW_KEY, 12, "a.rs", false);
    const req = t.last().sent.at(-1) as { t: string; id: string };
    expect(req).toMatchObject({ t: "diff", pane: 12, path: "a.rs", staged: false });
    t.last().recv({ t: "ack", id: req.id, ok: false, error: "no_block", message: "x" });
    expect(await p).toEqual({ ok: false, message: "This file is no longer changed." });
    const q = t.fleet.chipMenu(GW_KEY, 12, "changes");
    t.last().drop();
    expect(await q).toEqual({ ok: false, message: "Not connected to studio-mac." });
    vi.advanceTimersByTime(1_000);
    t.last().connect();
    t.last().recv(welcome());
    const r = t.fleet.chipMenu(GW_KEY, 12, "branches");
    vi.advanceTimersByTime(10_000);
    expect(await r).toEqual({ ok: false, message: "No answer from studio-mac." });
  });

  it("a diff answer for another file does not settle it", async () => {
    const t = await withHost();
    const p = t.fleet.fileDiff(GW_KEY, 12, "a.rs", true);
    const id = (t.last().sent.at(-1) as { id: string }).id;
    t.last().recv({ t: "diff", id, pane: 12, path: "b.rs", staged: true, binary: false, truncated: false, lines: [] });
    expect(await p).toEqual({ ok: false, message: "The computer's answer did not fit the request." });
  });
});

describe("Fleet answering a pane that waits (phone-first)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const menu: Waiting = {
    kind: "choice",
    prompt: "Do you want to make this edit to app.txt?",
    choices: ["Yes", "Yes, and switch to accept edits (shift+tab)", "No"],
    selected: 0,
    numbered: true,
    context: ["Edit file", " 1 -one", " 1 +ONE"],
  };
  /** The topology with pane 13 waiting for `waiting`. */
  const asking = (waiting: unknown) => {
    const t = topology();
    (t.windows[0]!.tabs[0]!.panes[1] as Record<string, unknown>).waiting = waiting;
    (t.windows[0]!.tabs[0]!.panes[1] as Record<string, unknown>).job = "claude";
    return t;
  };
  async function connected(permission: "operate" | "view" = "operate", features = ["text", "waiting", "complete"]) {
    const t = setup();
    await new HostsStore(t.kv).save(record({ permission }));
    await t.fleet.init();
    t.last().connect();
    t.last().recv(welcome(permission, features));
    t.last().recv(topology());
    t.fleet.store.flush();
    return t;
  }

  it("answers a numbered menu with the digit, then a reply after a pause and Enter", async () => {
    const t = await connected();
    t.last().recv(asking(menu));
    const p = t.fleet.answer(GW_KEY, 13, menu, { choice: 2, reply: "use lowercase" });
    expect(t.last().sent.at(-1)).toEqual({ t: "text", id: "x1", pane: 13, text: "3" });
    t.last().recv({ t: "ack", id: "x1", ok: true });
    await vi.advanceTimersByTimeAsync(600);
    expect(t.last().sent.at(-1)).toEqual({ t: "text", id: "x2", pane: 13, text: "use lowercase" });
    t.last().recv({ t: "ack", id: "x2", ok: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last().sent.at(-1)).toEqual({ t: "keys", id: "k3", pane: 13, keys: ["enter"] });
    t.last().recv({ t: "ack", id: "k3", ok: true });
    expect(await p).toEqual({ ok: true });
  });

  it("answers an unnumbered menu with arrows from the selected entry, then Enter", async () => {
    const t = await connected();
    const trust = { ...menu, prompt: "Is this a project you trust?", choices: ["No, exit", "Yes, I trust this folder"], numbered: false, context: [] };
    t.last().recv(asking(trust));
    const p = t.fleet.answer(GW_KEY, 13, trust, { choice: 1 });
    expect(t.last().sent.at(-1)).toEqual({ t: "keys", id: "k1", pane: 13, keys: ["down", "enter"] });
    t.last().recv({ t: "ack", id: "k1", ok: true });
    expect(await p).toEqual({ ok: true });
  });

  it("sends nothing to a question that changed, to a view-only device, or without text", async () => {
    const t = await connected();
    t.last().recv(asking({ ...menu, prompt: "Do you want to run rm -rf?" }));
    const before = t.last().sent.length;
    expect(await t.fleet.answer(GW_KEY, 13, menu, { choice: 0 })).toMatchObject({ ok: false, code: "answered" });
    expect(t.last().sent.length).toBe(before);

    const v = await connected("view");
    v.last().recv(asking(menu));
    expect(await v.fleet.answer(GW_KEY, 13, menu, { choice: 0 })).toMatchObject({ ok: false, code: "forbidden" });

    const old = await connected("operate", ["waiting"]);
    old.last().recv(asking(menu));
    expect(await old.fleet.answer(GW_KEY, 13, menu, { choice: 0 })).toMatchObject({ ok: false, code: "unsupported" });
  });

  it("asks for completions only from a device that may operate; too large reads as too large to show", async () => {
    const v = await connected("view");
    const before = v.last().sent.length;
    expect(await v.fleet.complete(GW_KEY, 13, "ls ")).toEqual({ ok: false, message: "This device may only view this computer." });
    expect(v.last().sent.length).toBe(before);

    const t = await connected();
    const p = t.fleet.complete(GW_KEY, 13, "ls ");
    const sent = t.last().sent.at(-1) as { t: string; id: string };
    expect(sent.t).toBe("complete");
    t.last().recv({ t: "ack", id: sent.id, ok: false, error: "too_large", message: "" });
    expect(await p).toEqual({ ok: false, message: "Too large to show on the phone." });
  });

  it("types a password as text, then Enter, and keeps no copy", async () => {
    const t = await connected();
    const pw: Waiting = { kind: "password", prompt: "[sudo] password for me:", choices: [], selected: 0, numbered: false, context: [] };
    t.last().recv(asking(pw));
    const p = t.fleet.answer(GW_KEY, 13, pw, { text: "hunter2" });
    expect(t.last().sent.at(-1)).toEqual({ t: "text", id: "x1", pane: 13, text: "hunter2" });
    t.last().recv({ t: "ack", id: "x1", ok: true });
    await vi.advanceTimersByTimeAsync(0);
    t.last().recv({ t: "ack", id: "k2", ok: true });
    expect(await p).toEqual({ ok: true });
    t.fleet.store.flush();
    expect(JSON.stringify(t.fleet.state)).not.toContain("hunter2");
    expect(JSON.stringify([...t.kv.data.values()])).not.toContain("hunter2");
  });

  it("alerts once for a new question, with Yes / No / Open, never for one already there when it connected", async () => {
    const t = setup();
    await new HostsStore(t.kv).save(record());
    await t.fleet.init();
    t.last().connect();
    t.last().recv(welcome("operate", ["waiting"]));
    // waiting when it connected: on Now, not an alert
    t.last().recv(asking({ ...menu, prompt: "Old question?" }));
    expect(t.notified).toHaveLength(0);
    t.last().recv(asking(menu));
    t.last().recv(asking(menu));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.notified).toEqual([
      {
        title: "claude is asking",
        subtitle: "studio-mac · rift · pane 2",
        body: menu.prompt,
        data: { host: GW_KEY, pane: 13, ask: questionKey(menu) },
        category: "rift-ask",
      },
    ]);
    // a password has no Yes and No: Open only
    t.last().recv(asking({ kind: "password", prompt: "Password:", choices: [], selected: 0, numbered: false, context: [] }));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.notified.at(-1)).toMatchObject({ title: "claude needs a password", category: "rift-ask-open" });
    // the pane on screen: no alert
    t.fleet.setViewing({ host: GW_KEY, pane: 13 });
    t.last().recv(asking({ ...menu, prompt: "On screen?" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.notified).toHaveLength(2);
  });

  it("an alert's No picks the menu's No once the pane still asks it", async () => {
    const t = await connected();
    t.last().recv(asking(menu));
    const p = t.fleet.answerFromAlert(GW_KEY, 13, questionKey(menu), "no");
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last().sent.at(-1)).toEqual({ t: "text", id: "x1", pane: 13, text: "3" });
    t.last().recv({ t: "ack", id: "x1", ok: true });
    expect(await p).toEqual({ ok: true });
    // moved on meanwhile: nothing sent, after the wait
    const sent = t.last().sent.length;
    const q = t.fleet.answerFromAlert(GW_KEY, 13, "Another question?", "yes", 1000);
    await vi.advanceTimersByTimeAsync(1500);
    expect(await q).toMatchObject({ ok: false, code: "answered" });
    expect(t.last().sent.length).toBe(sent);
  });

  it("an alert's Yes answers only the question it showed: the same wording about other lines is another", async () => {
    const t = await connected();
    t.last().recv(asking({ ...menu, context: ["Edit file", " 1 -two", " 1 +TWO"] }));
    const sent = t.last().sent.length;
    const p = t.fleet.answerFromAlert(GW_KEY, 13, questionKey(menu), "yes", 1000);
    await vi.advanceTimersByTimeAsync(1500);
    expect(await p).toMatchObject({ ok: false, code: "answered" });
    expect(t.last().sent.length).toBe(sent);
  });

  it("removes a delivered alert once its question went, and gives a view-only device Open only", async () => {
    const t = await connected();
    t.last().recv(asking(menu));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.notified).toHaveLength(1);
    t.last().recv(topology());
    await vi.advanceTimersByTimeAsync(0);
    expect(t.dismissed).toEqual([{ host: GW_KEY, pane: 13, ask: questionKey(menu) }]);

    const v = await connected("view");
    v.last().recv(asking(menu));
    await vi.advanceTimersByTimeAsync(0);
    expect(v.notified.at(-1)).toMatchObject({ category: "rift-ask-open" });
  });

  it("plans from the menu as it is now: an unnumbered menu whose selection moved", async () => {
    const t = await connected();
    const trust = { ...menu, prompt: "Is this a project you trust?", choices: ["No, exit", "Yes, I trust this folder"], numbered: false, context: [] };
    // asked with "No, exit" selected; the user moved to "Yes" on the computer since
    t.last().recv(asking({ ...trust, selected: 1 }));
    const p = t.fleet.answer(GW_KEY, 13, trust, { choice: 1 });
    expect(t.last().sent.at(-1)).toEqual({ t: "keys", id: "k1", pane: 13, keys: ["enter"] });
    t.last().recv({ t: "ack", id: "k1", ok: true });
    expect(await p).toEqual({ ok: true });
  });

  it("checks every text before the first step goes out", async () => {
    const t = await connected();
    t.last().recv(asking(menu));
    const before = t.last().sent.length;
    expect(await t.fleet.answer(GW_KEY, 13, menu, { choice: 2, reply: "x".repeat(MAX_TEXT_BYTES + 1) })).toMatchObject({
      ok: false,
      code: "too_large",
    });
    expect(await t.fleet.answer(GW_KEY, 13, menu, { choice: 2, reply: "a\u001bb" })).toMatchObject({ ok: false, code: "bad_request" });
    // the digit "3" would have gone out first
    expect(t.last().sent.length).toBe(before);
  });

  it("sends one answer per pane at a time: a second tap sends nothing", async () => {
    const t = await connected();
    t.last().recv(asking(menu));
    const p = t.fleet.answer(GW_KEY, 13, menu, { choice: 0 });
    const sent = t.last().sent.length;
    expect(await t.fleet.answer(GW_KEY, 13, menu, { choice: 0 })).toMatchObject({ ok: false, code: "busy" });
    expect(t.last().sent.length).toBe(sent);
    t.last().recv({ t: "ack", id: "x1", ok: true });
    expect(await p).toEqual({ ok: true });
    // done: the pane takes an answer again
    const q = t.fleet.answer(GW_KEY, 13, menu, { choice: 0 });
    expect(t.last().sent.at(-1)).toMatchObject({ t: "text", text: "1" });
    t.last().recv({ t: "ack", id: "x2", ok: true });
    expect(await q).toEqual({ ok: true });
  });

  it("a view-only device sends no command, keys or text", async () => {
    const v = await connected("view");
    const before = v.last().sent.length;
    for (const r of [v.fleet.run(GW_KEY, 12, "ls"), v.fleet.sendKeys(GW_KEY, 12, ["ctrl+c"]), v.fleet.sendText(GW_KEY, 12, "y")])
      expect(await r).toMatchObject({ ok: false, code: "forbidden" });
    expect(v.last().sent.length).toBe(before);
  });

  it("a pane screen that goes clears the pane on screen only while it is still the one", async () => {
    const t = await connected();
    t.fleet.setViewing({ host: GW_KEY, pane: 13 });
    // the pager put pane 12 on screen before pane 13's screen went
    t.fleet.setViewing({ host: GW_KEY, pane: 12 });
    t.fleet.leaveViewing(GW_KEY, 13);
    expect(t.fleet.isViewing(GW_KEY, 12)).toBe(true);
    t.fleet.leaveViewing(GW_KEY, 12);
    expect(t.fleet.isViewing(GW_KEY, 12)).toBe(false);
  });

  it("asks the computer for completions with a UTF-8 cursor", async () => {
    const t = await connected();
    const p = t.fleet.complete(GW_KEY, 12, "ls é");
    expect(t.last().sent.at(-1)).toEqual({ t: "complete", id: "c1", pane: 12, line: "ls é", cursor: 5 });
    t.last().recv({ t: "complete", id: "c1", pane: 12, items: [{ text: "ls éa", kind: "path" }], history: [], more: false });
    expect(await p).toMatchObject({ ok: true, msg: { items: [{ text: "ls éa" }] } });
    const old = await connected("operate", ["text"]);
    expect(await old.fleet.complete(GW_KEY, 12, "ls")).toMatchObject({ ok: false });
  });
});
