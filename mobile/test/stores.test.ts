import { describe, expect, it } from "vitest";
import { toBase64Url } from "../src/noise/bytes";
import {
  DEFAULT_SETTINGS,
  HostsStore,
  IdentityError,
  MemoryStore,
  NOTIFY_AFTER_CHOICES,
  isHostRecord,
  loadOrCreateIdentity,
  loadSettings,
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
    expect(DEFAULT_SETTINGS.confirmEveryCommand).toBe(true);
    expect(DEFAULT_SETTINGS.showCommandInNotification).toBe(false);
    await saveSettings(kv, { ...DEFAULT_SETTINGS, deviceName: "Pixel", notifyPolicy: "failures" });
    expect(await loadSettings(kv)).toEqual({ ...DEFAULT_SETTINGS, deviceName: "Pixel", notifyPolicy: "failures" });
    kv.data.set("rift.settings", JSON.stringify({ confirmEveryCommand: "no", deviceName: "  " }));
    expect(await loadSettings(kv)).toEqual(DEFAULT_SETTINGS);
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

  it("control keys: the old row by default; unknown and repeated ids dropped, order kept", async () => {
    const kv = new MemoryStore();
    expect(DEFAULT_SETTINGS.controlKeys).toEqual(["ctrl+c", "esc", "tab", "ctrl", "up", "down", "left", "right"]);
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
