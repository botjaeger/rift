import { describe, expect, it } from "vitest";
import { toBase64Url } from "../src/noise/bytes";
import {
  DEFAULT_SETTINGS,
  HostsStore,
  IdentityError,
  MemoryStore,
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
    await saveSettings(kv, { ...DEFAULT_SETTINGS, deviceName: "Pixel", notifyDone: true });
    expect(await loadSettings(kv)).toEqual({ ...DEFAULT_SETTINGS, deviceName: "Pixel", notifyDone: true });
    kv.data.set("rift.settings", JSON.stringify({ confirmEveryCommand: "no", deviceName: "  " }));
    expect(await loadSettings(kv)).toEqual(DEFAULT_SETTINGS);
  });
});
