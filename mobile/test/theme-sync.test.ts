import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { appearanceLabel, computerDetail, resolveTheme } from "../src/appearance";
import { Fleet } from "../src/fleet";
import { toBase64Url } from "../src/noise/bytes";
import type { Theme } from "../src/protocol/types";
import {
  DEFAULT_SETTINGS,
  HostsStore,
  MemoryStore,
  loadHostTheme,
  loadLastTheme,
  loadSettings,
  saveSettings,
  validAppearance,
  type HostRecord,
} from "../src/stores";
import { BUILTIN_THEMES } from "../src/themes.data";
import { DARK_DEFAULT, LIGHT_DEFAULT } from "../src/ui/palette";
import { FakeGatewaySocket, gatewayKey, rng } from "./helpers/gateway";

const fixture = (name: string) =>
  JSON.parse(readFileSync(new URL(`../../crates/rift-remote/fixtures/server/${name}`, import.meta.url), "utf8")) as Record<string, unknown>;
const WELCOME = fixture("welcome-theme.json");
const DRACULA = (fixture("theme.json") as { theme: Theme }).theme;
const OLD_WELCOME = fixture("welcome.json");

const GW = gatewayKey();
const GW_KEY = toBase64Url(GW.publicKey);
const record: HostRecord = { key: GW_KEY, name: "studio-mac", addresses: ["10.0.0.5:7878"], permission: "operate", pairedAt: "2026-11-06T10:00:00.000Z" };

async function connected(kv = new MemoryStore()) {
  const sockets: FakeGatewaySocket[] = [];
  const fleet = new Fleet({
    kv,
    randomBytes: rng,
    platform: "ios",
    systemDeviceName: "Phone",
    notifier: { requestPermission: async () => true, notify: async () => undefined },
    random: () => 0.5,
    socket: (url) => {
      const s = new FakeGatewaySocket(url, { key: GW });
      sockets.push(s);
      return s;
    },
  });
  if ((await new HostsStore(kv).load()).length === 0) await new HostsStore(kv).save(record);
  await fleet.init();
  const last = () => sockets[sockets.length - 1]!;
  last().connect();
  const settle = async () => {
    fleet.store.flush();
    // the saves run on the navigation queue
    await new Promise((r) => setTimeout(r, 0));
  };
  return { kv, fleet, last, settle, host: () => fleet.store.peek().hosts[0]! };
}

describe("the computer's theme on the phone", () => {
  it("the welcome's theme is the computer's, kept for the next start", async () => {
    const t = await connected();
    t.last().recv(WELCOME);
    await t.settle();
    expect(t.host().theme?.name).toBe("Dark Default");
    expect((await loadHostTheme(t.kv, GW_KEY))?.name).toBe("Dark Default");
    // nothing in view yet: no last theme
    expect(t.fleet.store.peek().lastTheme).toBeNull();
  });

  it("a theme message follows a change on the computer", async () => {
    const t = await connected();
    t.last().recv(WELCOME);
    t.last().recv({ t: "theme", theme: DRACULA });
    await t.settle();
    expect(t.host().theme).toEqual(DRACULA);
    expect(await loadHostTheme(t.kv, GW_KEY)).toEqual(DRACULA);
  });

  it("the theme of the computer in view becomes the app's last one, stored", async () => {
    const t = await connected();
    t.last().recv({ ...WELCOME, theme: DRACULA });
    await t.fleet.setLastView({ host: GW_KEY, pane: null });
    await t.settle();
    expect(t.fleet.store.peek().lastTheme).toEqual(DRACULA);
    expect(await loadLastTheme(t.kv)).toEqual(DRACULA);
    // a change while it is in view moves the last one too
    t.last().recv({ t: "theme", theme: LIGHT_DEFAULT });
    await t.settle();
    expect(t.fleet.store.peek().lastTheme?.name).toBe("Light Default");
  });

  it("a cold start has each computer's theme and the last one before any connects", async () => {
    const kv = new MemoryStore();
    const a = await connected(kv);
    a.last().recv({ ...WELCOME, theme: DRACULA });
    await a.fleet.setLastView({ host: GW_KEY, pane: null });
    await a.settle();
    const b = await connected(kv);
    expect(b.host().theme).toEqual(DRACULA);
    expect(b.fleet.store.peek().lastTheme).toEqual(DRACULA);
  });

  it("an older computer (no theme) keeps the theme it last sent, or none", async () => {
    const t = await connected();
    t.last().recv(OLD_WELCOME);
    await t.settle();
    expect(t.host().theme).toBeUndefined();
    t.last().recv({ ...WELCOME, theme: DRACULA });
    t.last().recv(OLD_WELCOME);
    await t.settle();
    expect(t.host().theme).toEqual(DRACULA);
  });

  it("unpairing forgets the computer's theme", async () => {
    const t = await connected();
    t.last().recv(WELCOME);
    await t.settle();
    await t.fleet.unpair(GW_KEY);
    expect(await loadHostTheme(t.kv, GW_KEY)).toBeNull();
  });
});

describe("which theme the app shows", () => {
  const sources = { computer: DRACULA, last: LIGHT_DEFAULT, scheme: "light" };

  it("Same as the computer: the one in view, else the last shown, else Dark Default", () => {
    expect(resolveTheme({ mode: "computer" }, sources)).toBe(DRACULA);
    expect(resolveTheme({ mode: "computer" }, { ...sources, computer: undefined })).toBe(LIGHT_DEFAULT);
    expect(resolveTheme({ mode: "computer" }, { computer: null, last: null, scheme: "light" })).toBe(DARK_DEFAULT);
  });

  it("Match this phone: Light Default in light mode, Dark Default otherwise", () => {
    expect(resolveTheme({ mode: "phone" }, sources).name).toBe("Light Default");
    expect(resolveTheme({ mode: "phone" }, { ...sources, scheme: "dark" }).name).toBe("Dark Default");
    expect(resolveTheme({ mode: "phone" }, { ...sources, scheme: null }).name).toBe("Dark Default");
  });

  it("Choose a theme: that builtin one, whatever the computer uses", () => {
    expect(resolveTheme({ mode: "theme", theme: "Nord" }, sources).name).toBe("Nord");
    expect(resolveTheme({ mode: "theme", theme: "Gone" }, sources).name).toBe("Dark Default");
  });

  it("says what each choice shows", () => {
    expect(appearanceLabel({ mode: "computer" })).toBe("Same as the computer");
    expect(appearanceLabel({ mode: "phone" })).toBe("Match this phone");
    expect(appearanceLabel({ mode: "theme", theme: "Nord" })).toBe("Nord");
    expect(computerDetail({ name: "studio-mac", theme: DRACULA }, null)).toBe("studio-mac uses Dracula now");
    expect(computerDetail({ name: "old-box" }, DRACULA)).toBe("old-box sends no theme (update Rift there): Dracula");
    expect(computerDetail(null, null)).toBe("Dark Default until a computer sends its theme");
  });

  it("the builtin themes include both defaults, each whole", () => {
    expect(BUILTIN_THEMES.map((t) => t.name)).toEqual(expect.arrayContaining(["Dark Default", "Light Default"]));
    expect(new Set(BUILTIN_THEMES.map((t) => t.name)).size).toBe(BUILTIN_THEMES.length);
    for (const t of BUILTIN_THEMES) expect(t.ansi).toHaveLength(16);
  });
});

describe("the Appearance setting", () => {
  it("defaults to the computer's theme and survives a save", async () => {
    const kv = new MemoryStore();
    expect((await loadSettings(kv)).appearance).toEqual({ mode: "computer" });
    await saveSettings(kv, { ...DEFAULT_SETTINGS, appearance: { mode: "theme", theme: "Dracula" } });
    expect((await loadSettings(kv)).appearance).toEqual({ mode: "theme", theme: "Dracula" });
    await saveSettings(kv, { ...DEFAULT_SETTINGS, appearance: { mode: "phone" } });
    expect((await loadSettings(kv)).appearance).toEqual({ mode: "phone" });
  });

  it("a stored value it cannot use is the default", () => {
    for (const bad of [undefined, null, "dark", { mode: "theme" }, { mode: "theme", theme: "Not A Theme" }, { mode: "system" }])
      expect(validAppearance(bad)).toEqual({ mode: "computer" });
  });
});
