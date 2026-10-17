import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Fleet } from "../src/fleet";
import { toBase64Url } from "../src/noise/bytes";
import { HostsStore, MemoryStore, loadSettings } from "../src/stores";
import { FakeGatewaySocket, gatewayKey, rng } from "./helpers/gateway";

/** Settings writes fail while `failing` is above zero (one failure each). */
class FlakyStore extends MemoryStore {
  failing = 0;
  override async set(k: string, v: string): Promise<void> {
    if (k === "rift.settings" && this.failing > 0) {
      this.failing--;
      throw new Error("storage unavailable");
    }
    return super.set(k, v);
  }
}

const fleets: Fleet[] = [];
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  for (const f of fleets.splice(0)) f.shutdown();
  vi.useRealTimers();
});

async function setup(kv: MemoryStore = new MemoryStore()) {
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
  await new HostsStore(kv).save({ key: host, name: "Host", addresses: ["127.0.0.1:1"], permission: "operate", pairedAt: "2026-10-17T00:00:00Z" });
  await fleet.init();
  const ws = sockets.at(-1)!;
  ws.connect();
  ws.recv({ t: "welcome", v: 2, host: "Host", os: "macos", version: "0.13.0", device: "test", permission: "operate", features: ["text"] });
  return { fleet, ws, host, kv };
}

describe("a Run's acknowledgement clears only the draft it sent", () => {
  it("keeps text typed while the run was on its way", async () => {
    const { fleet, ws, host } = await setup();
    fleet.setDraft(host, 12, "echo first");
    const result = fleet.run(host, 12, "echo first", { notify: false });
    const run = ws.sent.find((m) => m.t === "run")!;
    fleet.setDraft(host, 12, "echo second");
    ws.recv({ t: "ack", id: run.id, ok: true });
    expect(await result).toEqual({ ok: true });
    expect(fleet.state.drafts[`${host}:12`]).toBe("echo second");
  });

  it("clears the unchanged draft, and keeps it when the run fails", async () => {
    const { fleet, ws, host } = await setup();
    fleet.setDraft(host, 12, "echo ok");
    const ok = fleet.run(host, 12, "echo ok", { notify: false });
    ws.recv({ t: "ack", id: ws.sent.filter((m) => m.t === "run").at(-1)!.id, ok: true });
    await ok;
    expect(fleet.state.drafts[`${host}:12`]).toBe("");

    fleet.setDraft(host, 12, "echo refused");
    const refused = fleet.run(host, 12, "echo refused", { notify: false });
    ws.recv({ t: "ack", id: ws.sent.filter((m) => m.t === "run").at(-1)!.id, ok: false, error: "no_pane", message: "gone" });
    expect((await refused).ok).toBe(false);
    expect(fleet.state.drafts[`${host}:12`]).toBe("echo refused");
  });

  it("Run again leaves the draft alone", async () => {
    const { fleet, ws, host } = await setup();
    fleet.setDraft(host, 12, "ls");
    const again = fleet.run(host, 12, "ls", { notify: false, fromDraft: false });
    ws.recv({ t: "ack", id: ws.sent.find((m) => m.t === "run")!.id, ok: true });
    await again;
    expect(fleet.state.drafts[`${host}:12`]).toBe("ls");
  });
});

describe("a settings change that cannot be saved is put back and said", () => {
  it("never shows the app lock on when storage refused it; a retry saves", async () => {
    const kv = new FlakyStore();
    const { fleet } = await setup(kv);
    kv.failing = 1;
    expect(await fleet.updateSettings({ biometricLock: true })).toBe(false);
    expect(fleet.state.settings.biometricLock).toBe(false);
    expect(fleet.state.settingsError).toMatch(/could not save/);
    expect((await loadSettings(kv)).biometricLock).toBe(false);

    expect(await fleet.updateSettings({ biometricLock: true })).toBe(true);
    expect(fleet.state.settings.biometricLock).toBe(true);
    expect(fleet.state.settingsError).toBeNull();
    expect((await loadSettings(kv)).biometricLock).toBe(true);
  });

  it("a failed write does not undo a newer change, and the next write keeps the settings that hold", async () => {
    const kv = new FlakyStore();
    const { fleet } = await setup(kv);
    kv.failing = 1;
    const a = fleet.updateSettings({ biometricLock: true, wrapLines: false });
    const b = fleet.updateSettings({ biometricLock: false, askBeforeRun: true });
    expect(await a).toBe(false);
    expect(await b).toBe(true);
    const shown = fleet.state.settings;
    // the lock was changed again after the failed write: that newer value stays
    expect(shown.biometricLock).toBe(false);
    // the failed write's own change is put back
    expect(shown.wrapLines).toBe(true);
    expect(shown.askBeforeRun).toBe(true);
    const saved = await loadSettings(kv);
    expect({ lock: saved.biometricLock, wrap: saved.wrapLines, ask: saved.askBeforeRun }).toEqual({ lock: false, wrap: true, ask: true });
  });
});

describe("the topology a connection received", () => {
  it("is known once it came, even with no windows, and unknown again on the next connection", async () => {
    const { fleet, ws } = await setup();
    expect(fleet.state.hosts[0]!.topology).toBe(false);
    ws.recv({ t: "topology", windows: [] });
    fleet.store.flush();
    expect(fleet.state.hosts[0]!.topology).toBe(true);
    expect(fleet.state.hosts[0]!.windows).toEqual([]);
    ws.recv({ t: "welcome", v: 2, host: "Host", os: "macos", version: "0.13.0", device: "test", permission: "operate", features: ["text"] });
    fleet.store.flush();
    expect(fleet.state.hosts[0]!.topology).toBe(false);
  });
});

describe("draft ownership: the other paths", () => {
  it("a run that fails by disconnect keeps the draft", async () => {
    const { fleet, ws, host } = await setup();
    fleet.setDraft(host, 12, "echo lost");
    const result = fleet.run(host, 12, "echo lost", { notify: false });
    ws.drop();
    expect((await result).ok).toBe(false);
    expect(fleet.state.drafts[`${host}:12`]).toBe("echo lost");
  });

  it("an ack clears only its own pane's draft", async () => {
    const { fleet, ws, host } = await setup();
    fleet.setDraft(host, 12, "echo a");
    fleet.setDraft(host, 13, "echo a");
    const result = fleet.run(host, 12, "echo a", { notify: false });
    ws.recv({ t: "ack", id: ws.sent.find((m) => m.t === "run")!.id, ok: true });
    await result;
    expect(fleet.state.drafts[`${host}:12`]).toBe("");
    expect(fleet.state.drafts[`${host}:13`]).toBe("echo a");
  });

  it("program text: the line typed while the text was on its way is kept", async () => {
    const { fleet, ws, host } = await setup();
    fleet.setDraft(host, 12, "yes");
    const sent = fleet.sendText(host, 12, "yes");
    fleet.setDraft(host, 12, "no");
    ws.recv({ t: "ack", id: ws.sent.find((m) => m.t === "text")!.id, ok: true });
    expect((await sent).ok).toBe(true);
    // the Composer's step after sendText
    fleet.clearDraft(host, 12, "yes");
    expect(fleet.state.drafts[`${host}:12`]).toBe("no");
    fleet.clearDraft(host, 12, "no");
    expect(fleet.state.drafts[`${host}:12`]).toBe("");
  });
});

describe("settings writes: shown agrees with saved", () => {
  it("a write that saved a later change wins over that change's own failed write", async () => {
    const kv = new FlakyStore();
    const { fleet } = await setup(kv);
    const a = fleet.updateSettings({ wrapLines: false });
    const b = fleet.updateSettings({ biometricLock: true });
    // a's write (with b's change in it) saves; b's own write fails
    const set = kv.set.bind(kv);
    let n = 0;
    kv.set = async (k, v) => {
      if (k === "rift.settings" && n++ === 1) throw new Error("storage unavailable");
      return set(k, v);
    };
    await a;
    await b;
    const saved = await loadSettings(kv);
    expect(fleet.state.settings.biometricLock).toBe(saved.biometricLock);
    expect(fleet.state.settings.wrapLines).toBe(saved.wrapLines);
  });

  it("on, off, on with the first write failing keeps the last choice", async () => {
    const kv = new FlakyStore();
    const { fleet } = await setup(kv);
    kv.failing = 1;
    const w1 = fleet.updateSettings({ biometricLock: true });
    const w2 = fleet.updateSettings({ biometricLock: false });
    const w3 = fleet.updateSettings({ biometricLock: true });
    await w1;
    await w2;
    expect(await w3).toBe(true);
    expect(fleet.state.settings.biometricLock).toBe(true);
    expect((await loadSettings(kv)).biometricLock).toBe(true);
  });

  it("a failed write and its retry keep identity, paired hosts and other preferences", async () => {
    const kv = new FlakyStore();
    const { fleet } = await setup(kv);
    expect(await fleet.updateSettings({ askBeforeRun: true, notifyAfterMs: 60_000 })).toBe(true);
    const identity = kv.data.get("rift.device.key");
    const hosts = kv.data.get("rift.hosts");
    expect(identity).toBeTruthy();
    expect(hosts).toBeTruthy();
    kv.failing = 1;
    expect(await fleet.updateSettings({ biometricLock: true })).toBe(false);
    expect(await fleet.updateSettings({ biometricLock: true })).toBe(true);
    expect(kv.data.get("rift.device.key")).toBe(identity);
    expect(kv.data.get("rift.hosts")).toBe(hosts);
    const saved = await loadSettings(kv);
    expect({ lock: saved.biometricLock, ask: saved.askBeforeRun, after: saved.notifyAfterMs }).toEqual({ lock: true, ask: true, after: 60_000 });
    expect(fleet.state.hosts).toHaveLength(1);
  });

  it("a failed ntfy topic rotation tells the computers the old topic again", async () => {
    const kv = new FlakyStore();
    const { fleet, ws } = await setup(kv);
    ws.recv({ t: "welcome", v: 2, host: "Host", os: "macos", version: "0.13.0", device: "test", permission: "operate", features: ["text", "push_ntfy"] });
    fleet.store.flush();
    expect(await fleet.setNtfy(true)).toBeUndefined();
    const rotated = ws.sent.filter((m) => m.t === "push").length;
    const topic = fleet.state.settings.ntfyTopic;
    kv.failing = 1;
    expect(await fleet.rotateNtfyTopic()).toBe(false);
    expect(fleet.state.settings.ntfyTopic).toBe(topic);
    const pushes = ws.sent.filter((m) => m.t === "push");
    // the new topic went out, then the old one again
    expect(pushes.length).toBe(rotated + 2);
    expect(JSON.stringify(pushes.at(-2))).not.toContain(topic!);
    expect(JSON.stringify(pushes.at(-1))).toContain(topic!);
  });
});
