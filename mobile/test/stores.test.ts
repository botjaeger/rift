import { describe, expect, it } from "vitest";
import { toBase64Url } from "../src/noise/bytes";
import {
  DEFAULT_SETTINGS,
  HostsStore,
  IdentityError,
  MemoryStore,
  NOTIFY_AFTER_CHOICES,
  isHostRecord,
  loadLastView,
  loadOrCreateIdentity,
  loadSeenRelease,
  loadSettings,
  saveSeenRelease,
  replacedPairing,
  restoreTarget,
  saveLastView,
  saveSettings,
  type HostRecord,
} from "../src/stores";
import { rng } from "./helpers/gateway";

const host = (n: number, extra: Partial<HostRecord> = {}): HostRecord => ({
  key: toBase64Url(new Uint8Array(32).fill(n)),
  name: `mac-${n}`,
  addresses: [`10.0.0.${n}:7878`],
  permission: "operate",
  pairedAt: "2026-10-12T10:00:00.000Z",
  ...extra,
});

describe("HostsStore", () => {
  it("keeps every host when saves overlap", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    await Promise.all([store.save(host(1)), store.save(host(2)), store.save(host(3))]);
    expect((await store.load()).map((h) => h.key)).toEqual([host(1).key, host(2).key, host(3).key]);
  });

  it("does not resurrect a removed host when another host is saved concurrently", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    await store.save(host(1));
    await Promise.all([store.save(host(2)), store.remove(host(1).key)]);
    expect(await store.load()).toEqual([host(2)]);
    expect(JSON.parse(kv.data.get("rift.hosts")!)).toEqual([host(2).key]);
  });

  it("continues queued host mutations after a write fails", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    const set = kv.set.bind(kv);
    let fail = true;
    kv.set = async (k, v) => {
      if (k === "rift.hosts" && fail) {
        fail = false;
        throw new Error("keychain unavailable");
      }
      return set(k, v);
    };
    const failed = expect(store.save(host(1))).rejects.toThrow("keychain unavailable");
    const saved = store.save(host(2));
    await failed;
    await saved;
    expect(await store.load()).toEqual([host(2)]);
    await store.save(host(1));
    expect(await store.load()).toEqual([host(2), host(1)]);
  });

  it("refreshes metadata without overwriting the saved notification preference", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    await store.save(host(1, { notifications: false }));
    await store.refresh(host(1, { name: "Renamed computer" }));
    expect(await store.load()).toEqual([host(1, { name: "Renamed computer", notifications: false })]);
  });

  it("a late metadata refresh never recreates an unpaired computer", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    await store.save(host(1));
    await Promise.all([store.remove(host(1).key), store.refresh(host(1, { name: "Late welcome" }))]);
    expect(await store.load()).toEqual([]);
    expect(kv.data.has(`rift.host.${host(1).key}`)).toBe(false);
    expect(JSON.parse(kv.data.get("rift.hosts")!)).toEqual([]);
  });

  it("adds hosts in order, replaces by key, removes", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    expect(await store.load()).toEqual([]);
    await store.save(host(1));
    await store.save(host(2));
    await store.save(host(1, { name: "renamed", permission: "view" }));
    expect((await store.load()).map((h) => [h.name, h.permission])).toEqual([
      ["renamed", "view"],
      ["mac-2", "operate"],
    ]);
    // Persisted: a new store over the same keychain sees the same hosts.
    expect(await new HostsStore(kv).load()).toHaveLength(2);
    await store.remove(host(1).key);
    expect((await store.load()).map((h) => h.name)).toEqual(["mac-2"]);
    expect(kv.data.has(`rift.host.${host(1).key}`)).toBe(false);
  });

  it("keeps one keychain entry per host", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    for (let i = 1; i <= 5; i++) await store.save(host(i));
    expect([...kv.data.keys()].filter((k) => k.startsWith("rift.host.")).length).toBe(5);
  });

  it("skips damaged entries and survives a damaged index", async () => {
    const kv = new MemoryStore();
    const store = new HostsStore(kv);
    await store.save(host(1));
    await store.save(host(2));
    kv.data.set(`rift.host.${host(1).key}`, "{not json");
    expect((await store.load()).map((h) => h.name)).toEqual(["mac-2"]);
    kv.data.set(`rift.host.${host(2).key}`, JSON.stringify({ ...host(2), addresses: ["nope"] }));
    expect(await store.load()).toEqual([]);
    kv.data.set("rift.hosts", "garbage");
    expect(await store.load()).toEqual([]);
  });

  it("refuses to save an invalid record", async () => {
    await expect(new HostsStore(new MemoryStore()).save(host(1, { key: "short" }))).rejects.toThrow("invalid host record");
  });
});

describe("device identity", () => {
  it("is created once and then loaded", async () => {
    const kv = new MemoryStore();
    const a = await loadOrCreateIdentity(kv, rng);
    const b = await loadOrCreateIdentity(kv, () => {
      throw new Error("must not generate again");
    });
    expect(b.publicKey).toEqual(a.publicKey);
    expect(a.publicKey).toHaveLength(32);
  });

  it("never replaces a damaged or unreadable key", async () => {
    const kv = new MemoryStore();
    kv.data.set("rift.device.key", "!!");
    await expect(loadOrCreateIdentity(kv, rng)).rejects.toThrow(IdentityError);
    expect(kv.data.get("rift.device.key")).toBe("!!");

    const locked = new MemoryStore();
    locked.get = async () => {
      throw new Error("keychain locked");
    };
    let wrote = false;
    locked.set = async () => {
      wrote = true;
    };
    await expect(loadOrCreateIdentity(locked, rng)).rejects.toThrow("keychain locked");
    expect(wrote).toBe(false);
  });
});

describe("settings", () => {
  it("defaults, saves and loads; ignores bad fields", async () => {
    const kv = new MemoryStore();
    expect(await loadSettings(kv)).toEqual(DEFAULT_SETTINGS);
    expect(DEFAULT_SETTINGS).toMatchObject({ askBeforeRun: false, notifyMyCommands: true, wrapLines: true, showKeys: false });
    expect(DEFAULT_SETTINGS.showCommandInNotification).toBe(false);
    await saveSettings(kv, { ...DEFAULT_SETTINGS, deviceName: "Pixel", notifyPolicy: "failures" });
    expect(await loadSettings(kv)).toEqual({ ...DEFAULT_SETTINGS, deviceName: "Pixel", notifyPolicy: "failures" });
    kv.data.set("rift.settings", JSON.stringify({ askBeforeRun: "no", wrapLines: 0, deviceName: "  " }));
    expect(await loadSettings(kv)).toEqual(DEFAULT_SETTINGS);
  });

  it("checks for updates by default, and keeps the switch turned off", async () => {
    const kv = new MemoryStore();
    expect(DEFAULT_SETTINGS.autoUpdate).toBe(true);
    // stored before the switch existed
    const { autoUpdate: _, ...old } = DEFAULT_SETTINGS;
    kv.data.set("rift.settings", JSON.stringify(old));
    expect((await loadSettings(kv)).autoUpdate).toBe(true);
    await saveSettings(kv, { ...DEFAULT_SETTINGS, autoUpdate: false });
    expect((await loadSettings(kv)).autoUpdate).toBe(false);
  });

  it("remembers the release whose notes were shown", async () => {
    const kv = new MemoryStore();
    expect(await loadSeenRelease(kv)).toBeNull();
    await saveSeenRelease(kv, "0.1.0-u2");
    expect(await loadSeenRelease(kv)).toBe("0.1.0-u2");
    kv.data.set("rift.seenRelease", "x".repeat(200));
    expect(await loadSeenRelease(kv)).toBeNull();
  });

  it("migrates Confirm every command to Ask before running, off: a stored value cannot tell a choice from the default", async () => {
    const kv = new MemoryStore();
    // the old default (every save stored the whole object), and an explicit choice either way
    for (const old of [true, false]) {
      kv.data.set("rift.settings", JSON.stringify({ ...DEFAULT_SETTINGS, askBeforeRun: undefined, confirmEveryCommand: old }));
      const s = await loadSettings(kv);
      expect(s.askBeforeRun).toBe(false);
      expect(s).not.toHaveProperty("confirmEveryCommand");
      // the switches that did not exist before take their defaults
      expect(s).toMatchObject({ notifyMyCommands: true, wrapLines: true, showKeys: false });
    }
    // once saved under the new name, the choice is kept
    await saveSettings(kv, { ...DEFAULT_SETTINGS, askBeforeRun: true, notifyMyCommands: false, wrapLines: false, showKeys: true });
    expect(await loadSettings(kv)).toMatchObject({ askBeforeRun: true, notifyMyCommands: false, wrapLines: false, showKeys: true });
    // a stored value of the wrong type is the default
    kv.data.set("rift.settings", JSON.stringify({ showKeys: "yes" }));
    expect((await loadSettings(kv)).showKeys).toBe(false);
  });

  it("migrates the Phase 2 notifyDone switch: on is every long command, off is none", async () => {
    const kv = new MemoryStore();
    kv.data.set("rift.settings", JSON.stringify({ notifyDone: true }));
    expect((await loadSettings(kv)).notifyPolicy).toBe("all");
    kv.data.set("rift.settings", JSON.stringify({ notifyDone: false }));
    expect((await loadSettings(kv)).notifyPolicy).toBe("off");
    // a saved policy wins over a leftover boolean
    kv.data.set("rift.settings", JSON.stringify({ notifyDone: true, notifyPolicy: "failures" }));
    expect((await loadSettings(kv)).notifyPolicy).toBe("failures");
    kv.data.set("rift.settings", JSON.stringify({ notifyPolicy: "loud" }));
    expect((await loadSettings(kv)).notifyPolicy).toBe("off");
  });

  it("keeps a Notify after choice, anything else is 30 s", async () => {
    const kv = new MemoryStore();
    expect(DEFAULT_SETTINGS.notifyAfterMs).toBe(30_000);
    for (const ms of NOTIFY_AFTER_CHOICES) {
      kv.data.set("rift.settings", JSON.stringify({ notifyAfterMs: ms }));
      expect((await loadSettings(kv)).notifyAfterMs).toBe(ms);
    }
    for (const bad of [0, 12_345, "30000", -1]) {
      kv.data.set("rift.settings", JSON.stringify({ notifyAfterMs: bad }));
      expect((await loadSettings(kv)).notifyAfterMs).toBe(30_000);
    }
  });

  it("control keys: the board's row by default; unknown and repeated ids dropped, order kept", async () => {
    const kv = new MemoryStore();
    expect(DEFAULT_SETTINGS.controlKeys).toEqual(["esc", "tab", "ctrl", "enter", "paste"]);
    // a row stored as an old default moves to the key block's
    kv.data.set("rift.settings", JSON.stringify({ controlKeys: ["ctrl+c", "esc", "tab", "ctrl", "up", "down"] }));
    expect((await loadSettings(kv)).controlKeys).toEqual(DEFAULT_SETTINGS.controlKeys);
    kv.data.set("rift.settings", JSON.stringify({ controlKeys: ["esc", "tab", "ctrl+c", "ctrl", "up", "down", "enter"] }));
    expect((await loadSettings(kv)).controlKeys).toEqual(DEFAULT_SETTINGS.controlKeys);
    // a chosen row keeps its keys; its arrows are on the pad
    kv.data.set("rift.settings", JSON.stringify({ controlKeys: ["f5", "left", "ctrl+r", "up"] }));
    expect((await loadSettings(kv)).controlKeys).toEqual(["f5", "ctrl+r"]);
    kv.data.set("rift.settings", JSON.stringify({ controlKeys: ["f5", "alt", "rm -rf", "f5", 7, "ctrl+z"] }));
    expect((await loadSettings(kv)).controlKeys).toEqual(["f5", "alt", "ctrl+z"]);
    kv.data.set("rift.settings", JSON.stringify({ controlKeys: [] }));
    expect((await loadSettings(kv)).controlKeys).toEqual([]);
    kv.data.set("rift.settings", JSON.stringify({ controlKeys: "esc" }));
    expect((await loadSettings(kv)).controlKeys).toEqual(DEFAULT_SETTINGS.controlKeys);
  });

  it("ntfy: off on ntfy.sh before it existed; a saved topic and server kept, bad ones dropped", async () => {
    const kv = new MemoryStore();
    kv.data.set("rift.settings", JSON.stringify({ notifyPolicy: "all" }));
    const old = await loadSettings(kv);
    expect([old.ntfyServer, old.ntfyEnabled, old.ntfyTopic]).toEqual(["https://ntfy.sh", false, null]);
    const on = {
      ...DEFAULT_SETTINGS,
      ntfyServer: "https://ntfy.example.com:8443",
      ntfyEnabled: true,
      ntfyTopic: "rift-x7Kq2mP9vT4wL8nR3sY6hZ",
    };
    await saveSettings(kv, on);
    expect(await loadSettings(kv)).toEqual(on);
    // on with a damaged topic loads off (the switch never shows on with nothing under it)
    kv.data.set("rift.settings", JSON.stringify({ ...on, ntfyTopic: "a b" }));
    const damaged = await loadSettings(kv);
    expect([damaged.ntfyEnabled, damaged.ntfyTopic]).toEqual([false, null]);
    // off keeps the topic for the next time it is turned on
    await saveSettings(kv, { ...on, ntfyEnabled: false });
    expect(await loadSettings(kv)).toEqual({ ...on, ntfyEnabled: false });
    for (const [ntfyServer, ntfyTopic] of [
      ["http://ntfy.example.com", "a b"],
      ["https://ntfy.example.com/path", "x".repeat(65)],
      ["ntfy.example.com", 7],
      ["https://u@ntfy.example.com", ""],
    ]) {
      kv.data.set("rift.settings", JSON.stringify({ ntfyServer, ntfyTopic }));
      const s = await loadSettings(kv);
      expect([s.ntfyServer, s.ntfyTopic], String(ntfyServer)).toEqual(["https://ntfy.sh", null]);
    }
  });

  it("a host's Notifications switch is stored with it; another value is a damaged record", () => {
    const h = host(1);
    expect(isHostRecord(h)).toBe(true);
    expect(isHostRecord({ ...h, notifications: false })).toBe(true);
    expect(isHostRecord({ ...h, notifications: "off" })).toBe(false);
  });
});

describe("last view", () => {
  const key = toBase64Url(new Uint8Array(32).fill(7));
  it("saves, loads and clears the host (and pane) to reopen; damaged entries read as none", async () => {
    const kv = new MemoryStore();
    expect(await loadLastView(kv)).toBeNull();
    await saveLastView(kv, { host: key, pane: 12 });
    expect(await loadLastView(kv)).toEqual({ host: key, pane: 12 });
    await saveLastView(kv, { host: key, pane: null });
    expect(await loadLastView(kv)).toEqual({ host: key, pane: null });
    for (const bad of ["{", JSON.stringify({ host: "x", pane: 1 }), JSON.stringify({ host: key, pane: -1 }), JSON.stringify({ host: key })]) {
      kv.data.set("rift.last", bad);
      expect(await loadLastView(kv)).toBeNull();
    }
    await saveLastView(kv, null);
    expect(kv.data.has("rift.last")).toBe(false);
  });

  it("an old pairing is one whose computer answered as a later pairing of the same name", () => {
    const next = host(2, { name: "mac-1", pairedAt: "2026-11-18T10:00:00.000Z" });
    const old = host(1, { replacedBy: next.key });
    expect(replacedPairing(old, [old, next])).toBe(true);
    // the new pairing unpaired: the old one is itself again
    expect(replacedPairing(old, [old])).toBe(false);
    // another computer (another name), or one paired before it: not this one paired again
    expect(replacedPairing(old, [old, { ...next, name: "build-box" }])).toBe(false);
    expect(replacedPairing(old, [old, { ...next, pairedAt: "2026-10-01T10:00:00.000Z" }])).toBe(false);
    expect(replacedPairing({ ...old, replacedBy: old.key }, [old])).toBe(false);
    expect(replacedPairing(host(1), [host(1), next])).toBe(false);
    expect(isHostRecord(old)).toBe(true);
    expect(isHostRecord({ ...old, replacedBy: "nope" })).toBe(false);
  });

  it("reopens only a host that is still paired", () => {
    const last = { host: key, pane: 3 };
    expect(restoreTarget(last, [{ key }])).toEqual(last);
    expect(restoreTarget(last, [])).toBeNull();
    expect(restoreTarget(null, [{ key }])).toBeNull();
  });
});
