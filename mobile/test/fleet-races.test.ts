import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Fleet } from "../src/fleet";
import { toBase64Url } from "../src/noise/bytes";
import { HostsStore, MemoryStore, loadLastView, loadPaneTitles, loadSettings } from "../src/stores";
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

describe("navigation writes keep their arrival order", () => {
  it.each([13, null])("an older pane cannot replace the newest last view (%s) after its slower save finishes", async (pane) => {
    const { fleet, kv, host } = await setup();
    const set = kv.set.bind(kv);
    let release!: () => void;
    let first = true;
    kv.set = async (k, v) => {
      if (k === "rift.last" && first) {
        first = false;
        await new Promise<void>((r) => { release = r; });
      }
      return set(k, v);
    };
    const older = fleet.setLastView({ host, pane: 12 });
    await vi.advanceTimersByTimeAsync(0);
    const next = pane === null ? null : { host, pane };
    const newer = fleet.setLastView(next);
    await vi.advanceTimersByTimeAsync(0);
    release();
    await Promise.all([older, newer]);
    expect(fleet.state.lastView).toEqual(next);
    expect(await loadLastView(kv)).toEqual(next);
  });

  it("a failed last-view save does not block the next navigation", async () => {
    const { fleet, kv, host } = await setup();
    const set = kv.set.bind(kv);
    let first = true;
    kv.set = async (k, v) => {
      if (k === "rift.last" && first) {
        first = false;
        throw new Error("keychain unavailable");
      }
      return set(k, v);
    };
    const older = fleet.setLastView({ host, pane: 12 });
    const newer = fleet.setLastView({ host, pane: 13 });
    await Promise.all([older, newer]);
    expect(await loadLastView(kv)).toEqual({ host, pane: 13 });
  });

  it("an older topology cannot replace the title saved by a newer one", async () => {
    const { fleet, kv, host, ws } = await setup();
    const set = kv.set.bind(kv);
    let release!: () => void;
    let first = true;
    kv.set = async (k, v) => {
      if (k === "rift.paneTitles" && first) {
        first = false;
        await new Promise<void>((r) => { release = r; });
      }
      return set(k, v);
    };
    const topology = (title: string) => ({
      t: "topology" as const,
      windows: [{ pid: 1, tabs: [{ index: 0, title, active: true, panes: [
        { id: 12, active: true, cwd: "/fixture", job: null, last_command: null, last_exit: null },
      ] }] }],
    });
    ws.recv(topology("older title"));
    await vi.advanceTimersByTimeAsync(0);
    ws.recv(topology("newer title"));
    await vi.advanceTimersByTimeAsync(0);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(fleet.state.paneTitles[`${host}:12`]).toBe("newer title");
    expect((await loadPaneTitles(kv))[`${host}:12`]).toBe("newer title");
  });
});

it("an unpair whose local removal fails revokes remotely and can finish on retry", async () => {
  const { fleet, kv, host, ws } = await setup();
  const set = kv.set.bind(kv);
  let first = true;
  kv.set = async (k, v) => {
    if (k === "rift.hosts" && first) {
      first = false;
      throw new Error("keychain unavailable");
    }
    return set(k, v);
  };
  await expect(fleet.unpair(host)).rejects.toThrow("keychain unavailable");
  expect(ws.sent.filter((m) => m.t === "forget")).toHaveLength(1);
  await fleet.unpair(host);
  expect(fleet.state.hosts).toEqual([]);
  expect(await new HostsStore(kv).load()).toEqual([]);
  expect(kv.data.has(`rift.host.${host}`)).toBe(false);
});

async function setup(kv: MemoryStore = new MemoryStore(), requestPermission: () => Promise<boolean> = async () => true) {
  const key = gatewayKey();
  const host = toBase64Url(key.publicKey);
  const sockets: FakeGatewaySocket[] = [];
  const fleet = new Fleet({
    kv,
    randomBytes: rng,
    platform: "ios",
    systemDeviceName: "Test phone",
    notifier: { requestPermission, notify: async () => {} },
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

describe("Run navigation waits for notification permission UI", () => {
  for (const outcome of ["granted", "denied", "error"] as const) {
    it(`keeps the acknowledged draft spent while permission is ${outcome}`, async () => {
      let finish!: (value: boolean) => void;
      let fail!: (error: Error) => void;
      const permission = new Promise<boolean>((resolve, reject) => { finish = resolve; fail = reject; });
      const { fleet, ws, host } = await setup(new MemoryStore(), () => permission);
      if (process.env.RIFT_RUN_PERMISSION_NEGATIVE_CONTROL) {
        const original = fleet.notifyWhenDone.bind(fleet);
        vi.spyOn(fleet, "notifyWhenDone").mockImplementation((...args) => { void original(...args); return Promise.resolve(); });
      }
      fleet.setDraft(host, 12, "echo accepted");
      let settled = false;
      const result = fleet.run(host, 12, "echo accepted", { notify: true }).then((r) => { settled = true; return r; });
      const sent = ws.sent.find((m) => m.t === "run")!;
      expect(sent).toBeDefined(); // The OS prompt does not delay executing the command.
      ws.recv({ t: "ack", id: sent.id, ok: true });
      await vi.advanceTimersByTimeAsync(0);
      expect(fleet.state.drafts[`${host}:12`]).toBe("");
      expect(settled).toBe(false); // A caller cannot dismiss its modal over the OS prompt.
      fleet.setDraft(host, 12, "echo next");
      if (outcome === "error") fail(new Error("permission controller unavailable"));
      else finish(outcome === "granted");
      expect(await result).toEqual({ ok: true });
      expect(fleet.state.drafts[`${host}:12`]).toBe("echo next");
    });
  }
});

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
  it("reports a refused ntfy server save, preserves the saved server, and permits retry", async () => {
    const kv = new FlakyStore();
    const { fleet } = await setup(kv);
    const before = fleet.state.settings.ntfyServer;
    kv.failing = 1;
    expect(await fleet.setNtfyServer("ntfy.example.com")).toBe(false);
    expect(fleet.state.settings.ntfyServer).toBe(before);
    expect((await loadSettings(kv)).ntfyServer).toBe(before);
    expect(fleet.state.settingsError).toMatch(/could not save/);

    expect(await fleet.setNtfyServer("ntfy.example.com")).toBe(true);
    expect(fleet.state.settings.ntfyServer).toBe("https://ntfy.example.com");
    expect((await loadSettings(kv)).ntfyServer).toBe("https://ntfy.example.com");
    expect(fleet.state.settingsError).toBeNull();
  });

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
  it("a failed notification change and concurrent welcome keep the saved preference and fresh metadata", async () => {
    const { fleet, kv, host, ws } = await setup();
    const set = kv.set.bind(kv);
    let fail!: () => void;
    let first = true;
    kv.set = async (k, v) => {
      if (first && k === `rift.host.${host}` && JSON.parse(v).notifications === false) {
        first = false;
        await new Promise<void>((_, reject) => { fail = () => reject(new Error("keychain unavailable")); });
      }
      return set(k, v);
    };
    const changing = fleet.setHostNotifications(host, false);
    const refused = expect(changing).rejects.toThrow("keychain unavailable");
    await vi.advanceTimersByTimeAsync(0);
    ws.recv({ t: "welcome", v: 2, host: "Renamed host", os: "macos", version: "0.13.1", device: "test", permission: "operate", features: ["text"] });
    fail();
    await refused;
    await vi.advanceTimersByTimeAsync(0);
    const saved = (await new HostsStore(kv).load())[0]!;
    expect(saved.notifications).not.toBe(false);
    expect(saved.name).toBe("Renamed host");
    expect(fleet.state.hosts[0]!.record.notifications).not.toBe(false);
  });

  it("an explicit same-value choice survives an earlier failed settings write", async () => {
    const { fleet, kv } = await setup();
    const set = kv.set.bind(kv);
    let fail!: () => void;
    let first = true;
    kv.set = async (k, v) => {
      if (first && k === "rift.settings") {
        first = false;
        await new Promise<void>((_, reject) => { fail = () => reject(new Error("keychain unavailable")); });
      }
      return set(k, v);
    };
    const older = fleet.updateSettings({ biometricLock: true });
    await vi.advanceTimersByTimeAsync(0);
    const newer = fleet.updateSettings({ biometricLock: true });
    fail();
    await older;
    expect(await newer).toBe(true);
    expect(fleet.state.settings.biometricLock).toBe(true);
    expect((await loadSettings(kv)).biometricLock).toBe(true);
  });

  it("keeps a host's notification preference when storage refuses the change", async () => {
    const { fleet, kv, host } = await setup();
    const set = kv.set.bind(kv);
    kv.set = async (k, v) => {
      if (k === `rift.host.${host}` && JSON.parse(v).notifications === false) throw new Error("keychain unavailable");
      return set(k, v);
    };
    await expect(fleet.setHostNotifications(host, false)).rejects.toThrow("keychain unavailable");
    expect(fleet.state.hosts[0]!.record.notifications).not.toBe(false);
    expect((await new HostsStore(kv).load())[0]!.notifications).not.toBe(false);
    kv.set = set;
    await fleet.setHostNotifications(host, false);
    expect(fleet.state.hosts[0]!.record.notifications).toBe(false);
    expect((await new HostsStore(kv).load())[0]!.notifications).toBe(false);
  });

  it("a pending notification permission preserves settings changed while it was open", async () => {
    let grant!: (allowed: boolean) => void;
    const { fleet, kv } = await setup(new MemoryStore(), () => new Promise<boolean>((resolve) => { grant = resolve; }));
    await fleet.updateSettings({ notifyPolicy: "off" });
    const enabling = fleet.updateSettings({ notifyPolicy: "all" });
    expect(await fleet.updateSettings({ biometricLock: true, wrapLines: false })).toBe(true);
    grant(true);
    expect(await enabling).toBe(true);
    expect(fleet.state.settings.biometricLock).toBe(true);
    expect(fleet.state.settings.wrapLines).toBe(false);
    const saved = await loadSettings(kv);
    expect(saved.biometricLock).toBe(true);
    expect(saved.wrapLines).toBe(false);
    expect(saved.notifyPolicy).toBe("all");
  });

  it("a newer notification choice wins over a pending permission response", async () => {
    let grant!: (allowed: boolean) => void;
    const { fleet, kv } = await setup(new MemoryStore(), () => new Promise<boolean>((resolve) => { grant = resolve; }));
    await fleet.updateSettings({ notifyPolicy: "off" });
    const enabling = fleet.updateSettings({ notifyPolicy: "all" });
    expect(await fleet.updateSettings({ notifyPolicy: "off" })).toBe(true);
    grant(true);
    expect(await enabling).toBe(true);
    expect(fleet.state.settings.notifyPolicy).toBe("off");
    expect((await loadSettings(kv)).notifyPolicy).toBe("off");
  });

  it("a late permission response does not claim rollback ownership of a newer setting", async () => {
    let grant!: (allowed: boolean) => void;
    const kv = new FlakyStore();
    const { fleet } = await setup(kv, () => new Promise<boolean>((resolve) => { grant = resolve; }));
    await fleet.updateSettings({ notifyPolicy: "off" });
    const enabling = fleet.updateSettings({ notifyPolicy: "all", biometricLock: true });
    expect(await fleet.updateSettings({ biometricLock: true })).toBe(true);
    kv.failing = 1;
    grant(true);
    expect(await enabling).toBe(false);
    expect(fleet.state.settings.biometricLock).toBe(true);
    expect(fleet.state.settings.notifyPolicy).toBe("off");
    expect((await loadSettings(kv)).biometricLock).toBe(true);
  });

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
