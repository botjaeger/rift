// Persistent app data over a small key-value interface: the platform keychain/keystore
// (expo-secure-store) in the app, a Map in the tests. Free of React Native imports.
//
//   rift.device.key   this device's X25519 private key (base64url)
//   rift.hosts        JSON array of paired gateway keys, in display order
//   rift.host.<key>   one paired desktop (HostRecord)
//   rift.settings     Settings
//   rift.last         LastView: the host (and pane) on screen when the app last showed one
//   rift.theme.<key>  the theme a paired desktop last sent (Theme)
//   rift.theme.last   the computer's theme the app last showed (Theme)
//
// Hosts live one per entry so no single keychain item grows with the number of desktops.

import { fromBase64Url, toBase64Url } from "./noise/bytes";
import { keyPairFromPrivate, type KeyPair, type RandomBytes } from "./noise/noise";
import { DEFAULT_CONTROL_KEYS, validControlKeys } from "./controlKeys";
import { isNtfyTopic, normalizeNtfyServer, NTFY_SERVER } from "./notify";
import { isAddress, normalizeRelayUrl } from "./pairing";
import { isTheme, type Permission, type Theme } from "./protocol/types";
import { BUILTIN_THEMES } from "./themes.data";

export interface KeyValueStore {
  /** null when absent; throws when the store cannot be read (e.g. a locked keychain). */
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

export class MemoryStore implements KeyValueStore {
  readonly data = new Map<string, string>();
  async get(key: string) {
    return this.data.get(key) ?? null;
  }
  async set(key: string, value: string) {
    this.data.set(key, value);
  }
  async delete(key: string) {
    this.data.delete(key);
  }
}

// --- device identity -----------------------------------------------------------------------

const IDENTITY_KEY = "rift.device.key";

export class IdentityError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "IdentityError";
  }
}

/**
 * This device's static key pair, created on first use. A stored key that cannot be read or
 * decoded is an error, never a reason to create a new one: that would unpair every desktop.
 */
export async function loadOrCreateIdentity(kv: KeyValueStore, randomBytes: RandomBytes): Promise<KeyPair> {
  const stored = await kv.get(IDENTITY_KEY);
  if (stored !== null) {
    const key = fromBase64Url(stored);
    if (key?.length !== 32) throw new IdentityError("the stored device key is damaged");
    return keyPairFromPrivate(key);
  }
  const key = randomBytes(32);
  await kv.set(IDENTITY_KEY, toBase64Url(key));
  return keyPairFromPrivate(key);
}

// --- paired desktops -------------------------------------------------------------------------

export interface HostRecord {
  /** The pinned gateway static public key (base64url); identifies the host. */
  key: string;
  name: string;
  addresses: string[];
  permission: Permission;
  /** RFC 3339. */
  pairedAt: string;
  /** The gateway's relay URL from the pairing link (normalized); absent: none. */
  relay?: string;
  /** The path that last reached `welcome`; the next connection starts there. Absent: LAN. */
  via?: "lan" | "relay";
  /** false: no finish notifications from this computer except "Notify me". Absent: on. */
  notifications?: boolean;
  /** The computer's OS and Rift version from its last `welcome` (Portals shows them while offline). */
  os?: string;
  version?: string;
  /** Epoch milliseconds of the last `welcome` ("last seen"). */
  seenAt?: number;
  /**
   * The key (base64url) of another paired computer that answered at this one's address, proving
   * it holds that key: this is an old pairing of a computer paired again (`replacedPairing`).
   */
  replacedBy?: string;
}

/**
 * `record` is an old pairing of a computer paired again: the computer that answered in its place
 * is still paired on this phone, under the same name, paired after it. It connects only when the
 * user asks (Try again): on its own each
 * try is a failed handshake the computer counts against this phone's address, which then refuses
 * the new pairing too.
 */
export function replacedPairing(record: HostRecord, records: readonly HostRecord[]): boolean {
  const by = record.replacedBy;
  if (by === undefined || by === record.key) return false;
  // the same computer paired later (by its name): two Macs that swapped addresses are not this
  const next = records.find((r) => r.key === by);
  return !!next && next.name === record.name && Date.parse(next.pairedAt) > Date.parse(record.pairedAt);
}

const HOSTS_INDEX = "rift.hosts";
const hostEntry = (key: string) => `rift.host.${key}`;

export function isHostRecord(v: unknown): v is HostRecord {
  if (typeof v !== "object" || v === null) return false;
  const h = v as Record<string, unknown>;
  return (
    typeof h.key === "string" &&
    fromBase64Url(h.key)?.length === 32 &&
    typeof h.name === "string" &&
    Array.isArray(h.addresses) &&
    h.addresses.length > 0 &&
    h.addresses.every((a) => typeof a === "string" && isAddress(a)) &&
    (h.permission === "operate" || h.permission === "view") &&
    typeof h.pairedAt === "string" &&
    (h.relay === undefined || (typeof h.relay === "string" && normalizeRelayUrl(h.relay) === h.relay)) &&
    (h.via === undefined || h.via === "lan" || (h.via === "relay" && h.relay !== undefined)) &&
    (h.notifications === undefined || typeof h.notifications === "boolean") &&
    (h.os === undefined || (typeof h.os === "string" && h.os.length <= 64)) &&
    (h.version === undefined || (typeof h.version === "string" && h.version.length <= 64)) &&
    (h.seenAt === undefined || (typeof h.seenAt === "number" && Number.isSafeInteger(h.seenAt) && h.seenAt >= 0)) &&
    (h.replacedBy === undefined || (typeof h.replacedBy === "string" && fromBase64Url(h.replacedBy)?.length === 32))
  );
}

export class HostsStore {
  constructor(private readonly kv: KeyValueStore) {}

  private writing: Promise<void> = Promise.resolve();

  /** The index is shared by every host: each mutation must finish before the next reads it. */
  private write(change: () => Promise<void>): Promise<void> {
    const result = this.writing.then(change);
    this.writing = result.catch(() => undefined);
    return result;
  }

  private async index(): Promise<string[]> {
    const raw = await this.kv.get(HOSTS_INDEX);
    if (raw === null) return [];
    try {
      const v: unknown = JSON.parse(raw);
      return Array.isArray(v) ? v.filter((k): k is string => typeof k === "string" && fromBase64Url(k)?.length === 32) : [];
    } catch {
      return [];
    }
  }

  /** Every readable host, in display order; damaged entries are skipped. */
  async load(): Promise<HostRecord[]> {
    const out: HostRecord[] = [];
    for (const key of await this.index()) {
      const raw = await this.kv.get(hostEntry(key));
      if (raw === null) continue;
      try {
        const v: unknown = JSON.parse(raw);
        if (isHostRecord(v) && v.key === key) out.push(v);
      } catch {
        // skip a damaged entry
      }
    }
    return out;
  }

  /** Adds or replaces (same key) a host; a new host goes last. */
  async save(host: HostRecord): Promise<void> {
    if (!isHostRecord(host)) throw new Error("invalid host record");
    return this.write(async () => {
      await this.kv.set(hostEntry(host.key), JSON.stringify(host));
      const index = await this.index();
      if (!index.includes(host.key)) await this.kv.set(HOSTS_INDEX, JSON.stringify([...index, host.key]));
    });
  }

  /** Refreshes an existing computer's metadata without changing its saved notification preference or re-pairing it. */
  async refresh(host: HostRecord): Promise<void> {
    if (!isHostRecord(host)) throw new Error("invalid host record");
    return this.write(async () => {
      const raw = await this.kv.get(hostEntry(host.key));
      if (raw === null) return;
      const current: unknown = JSON.parse(raw);
      if (!isHostRecord(current) || current.key !== host.key) return;
      const { notifications: _, ...metadata } = host;
      const record = current.notifications === false ? { ...metadata, notifications: false } : metadata;
      await this.kv.set(hostEntry(host.key), JSON.stringify(record));
    });
  }

  async remove(key: string): Promise<void> {
    return this.write(async () => {
      const index = await this.index();
      await this.kv.set(HOSTS_INDEX, JSON.stringify(index.filter((k) => k !== key)));
      await this.kv.delete(hostEntry(key));
      await this.kv.delete(themeEntry(key));
    });
  }
}

// --- settings --------------------------------------------------------------------------------

export interface Settings {
  /** null: the system device name. */
  deviceName: string | null;
  /** "Ask before running": the Confirm sheet before every Run (off: Run sends at once). */
  askBeforeRun: boolean;
  /** "Notify me when my commands finish": every command run from this phone asks to be told. */
  notifyMyCommands: boolean;
  /** "Wrap long lines" in block output (off: each block scrolls sideways). */
  wrapLines: boolean;
  /** "Show keys above the command line" (off: the row shows only under a full-screen program). */
  showKeys: boolean;
  /** Which finished commands notify (besides "Notify me"): none, failed ones, or all. */
  notifyPolicy: NotifyPolicy;
  /** Only commands that ran at least this long notify (one of NOTIFY_AFTER_CHOICES). */
  notifyAfterMs: number;
  showCommandInNotification: boolean;
  /** Face ID / fingerprint (or the device passcode) on launch and after 5 minutes away. */
  biometricLock: boolean;
  /** With the lock and "Ask before running" on: authenticate before Run ("Run with Face ID"). */
  authBeforeRun: boolean;
  /** The control-key row, in order (ids from CONTROL_KEYS). */
  controlKeys: string[];
  /** The ntfy server alerts go through (`https://<host>[:port]`; NTFY_SERVER unless self-hosted). */
  ntfyServer: string;
  /** "Alerts through ntfy". */
  ntfyEnabled: boolean;
  /**
   * The ntfy topic alerts go to (null: none made yet). Kept while ntfy is off, so turning it on
   * again reuses the topic the ntfy app is already subscribed to.
   */
  ntfyTopic: string | null;
  /** "Check for updates automatically": at launch, every 15 minutes and on coming back (ota.ts). */
  autoUpdate: boolean;
  /** Settings › Appearance. */
  appearance: Appearance;
}

export type NotifyPolicy = "off" | "failures" | "all";

/**
 * Settings › Appearance: the paired computer's theme (`computer`, the default), the phone's light
 * or dark (`phone`: Light Default / Dark Default), or one of the builtin themes by name (`theme`).
 * It stays on the phone and never changes a computer.
 */
export type Appearance = { mode: "computer" } | { mode: "phone" } | { mode: "theme"; theme: string };

export const DEFAULT_APPEARANCE: Appearance = { mode: "computer" };

/** A stored appearance as it can be used: `theme` only with a builtin theme's name, else the default. */
export function validAppearance(v: unknown): Appearance {
  if (typeof v !== "object" || v === null) return DEFAULT_APPEARANCE;
  const a = v as Record<string, unknown>;
  if (a.mode === "phone") return { mode: "phone" };
  if (a.mode === "theme" && typeof a.theme === "string" && BUILTIN_THEMES.some((t) => t.name === a.theme))
    return { mode: "theme", theme: a.theme };
  return DEFAULT_APPEARANCE;
}

/** The "Notify after" choices: 10 s, 30 s, 1 min, 5 min. */
export const NOTIFY_AFTER_CHOICES: readonly number[] = [10_000, 30_000, 60_000, 300_000];

export const DEFAULT_SETTINGS: Settings = {
  deviceName: null,
  askBeforeRun: false,
  notifyMyCommands: true,
  wrapLines: true,
  showKeys: false,
  notifyPolicy: "off",
  notifyAfterMs: 30_000,
  showCommandInNotification: false,
  biometricLock: false,
  // Takes effect only with the lock on, so turning the lock on also asks before running.
  authBeforeRun: true,
  controlKeys: [...DEFAULT_CONTROL_KEYS],
  ntfyServer: NTFY_SERVER,
  ntfyEnabled: false,
  ntfyTopic: null,
  autoUpdate: true,
  appearance: DEFAULT_APPEARANCE,
};

const SETTINGS_KEY = "rift.settings";
export const MAX_DEVICE_NAME = 64;

export async function loadSettings(kv: KeyValueStore): Promise<Settings> {
  const raw = await kv.get(SETTINGS_KEY);
  if (raw === null) return DEFAULT_SETTINGS;
  try {
    const v = JSON.parse(raw) as Record<string, unknown>;
    const bool = (k: keyof Settings) => (typeof v[k] === "boolean" ? (v[k] as boolean) : (DEFAULT_SETTINGS[k] as boolean));
    return {
      deviceName: typeof v.deviceName === "string" && v.deviceName.trim() !== "" ? v.deviceName.slice(0, MAX_DEVICE_NAME) : null,
      // Before the redesign "Confirm every command" (`confirmEveryCommand`, on by default). Every
      // save stored the whole object, so a stored value cannot tell a choice from the default:
      // as the user asked, it is not carried over and the new setting starts off.
      askBeforeRun: bool("askBeforeRun"),
      notifyMyCommands: bool("notifyMyCommands"),
      wrapLines: bool("wrapLines"),
      // absent before the switch: off (the default), as the P3 Pane board draws the command line
      // without the key row
      showKeys: bool("showKeys"),
      // Before Phase 3 a boolean `notifyDone` (long commands, failed or not).
      notifyPolicy:
        v.notifyPolicy === "off" || v.notifyPolicy === "failures" || v.notifyPolicy === "all"
          ? v.notifyPolicy
          : typeof v.notifyDone === "boolean"
            ? v.notifyDone
              ? "all"
              : "off"
            : DEFAULT_SETTINGS.notifyPolicy,
      notifyAfterMs:
        typeof v.notifyAfterMs === "number" && NOTIFY_AFTER_CHOICES.includes(v.notifyAfterMs)
          ? v.notifyAfterMs
          : DEFAULT_SETTINGS.notifyAfterMs,
      showCommandInNotification: bool("showCommandInNotification"),
      biometricLock: bool("biometricLock"),
      authBeforeRun: bool("authBeforeRun"),
      controlKeys: validControlKeys(v.controlKeys),
      // Absent before ntfy: off, on ntfy.sh.
      ntfyServer:
        (typeof v.ntfyServer === "string" && normalizeNtfyServer(v.ntfyServer) === v.ntfyServer && v.ntfyServer) ||
        DEFAULT_SETTINGS.ntfyServer,
      // on only with a usable topic (else the switch would show on with nothing under it)
      ntfyEnabled: bool("ntfyEnabled") && typeof v.ntfyTopic === "string" && isNtfyTopic(v.ntfyTopic),
      ntfyTopic: typeof v.ntfyTopic === "string" && isNtfyTopic(v.ntfyTopic) ? v.ntfyTopic : null,
      // absent before the switch: on, as updates always were
      autoUpdate: bool("autoUpdate"),
      // absent before Appearance: the computer's theme
      appearance: validAppearance(v.appearance),
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export async function saveSettings(kv: KeyValueStore, settings: Settings): Promise<void> {
  await kv.set(SETTINGS_KEY, JSON.stringify(settings));
}

// --- themes ----------------------------------------------------------------------------------

const themeEntry = (key: string) => `rift.theme.${key}`;
const LAST_THEME_KEY = "rift.theme.last";

async function loadTheme(kv: KeyValueStore, entry: string): Promise<Theme | null> {
  try {
    const raw = await kv.get(entry);
    if (raw === null) return null;
    const v: unknown = JSON.parse(raw);
    return isTheme(v) ? v : null;
  } catch {
    return null;
  }
}

/** The theme computer `key` last sent; null when none was (an older Rift) or it cannot be read. */
export const loadHostTheme = (kv: KeyValueStore, key: string) => loadTheme(kv, themeEntry(key));

export async function saveHostTheme(kv: KeyValueStore, key: string, theme: Theme): Promise<void> {
  await kv.set(themeEntry(key), JSON.stringify(theme));
}

/** The computer's theme the app last showed: what a computer without one keeps showing. */
export const loadLastTheme = (kv: KeyValueStore) => loadTheme(kv, LAST_THEME_KEY);

export async function saveLastTheme(kv: KeyValueStore, theme: Theme): Promise<void> {
  await kv.set(LAST_THEME_KEY, JSON.stringify(theme));
}

// --- what's new ------------------------------------------------------------------------------

const SEEN_RELEASE_KEY = "rift.seenRelease";

/** The release (changelog.ts releaseKey) whose notes were last shown; null on a first launch. */
export async function loadSeenRelease(kv: KeyValueStore): Promise<string | null> {
  try {
    const raw = await kv.get(SEEN_RELEASE_KEY);
    return raw !== null && /^[0-9A-Za-z.+-]{1,64}$/.test(raw) ? raw : null;
  } catch {
    return null;
  }
}

export async function saveSeenRelease(kv: KeyValueStore, key: string): Promise<void> {
  await kv.set(SEEN_RELEASE_KEY, key);
}

// --- last view -------------------------------------------------------------------------------

/** The host (and pane) on screen when the app last showed one; reopened on a cold start. */
export interface LastView {
  host: string;
  /** null: the host's tabs page. */
  pane: number | null;
}

const LAST_KEY = "rift.last";

export function isLastView(v: unknown): v is LastView {
  if (typeof v !== "object" || v === null) return false;
  const l = v as Record<string, unknown>;
  return (
    typeof l.host === "string" &&
    fromBase64Url(l.host)?.length === 32 &&
    (l.pane === null || (typeof l.pane === "number" && Number.isSafeInteger(l.pane) && l.pane >= 0))
  );
}

/** null when none is stored or it cannot be read (the app then opens Now). */
export async function loadLastView(kv: KeyValueStore): Promise<LastView | null> {
  try {
    const raw = await kv.get(LAST_KEY);
    if (raw === null) return null;
    const v: unknown = JSON.parse(raw);
    return isLastView(v) ? { host: v.host, pane: v.pane } : null;
  } catch {
    return null;
  }
}

export async function saveLastView(kv: KeyValueStore, view: LastView | null): Promise<void> {
  if (view === null) await kv.delete(LAST_KEY);
  else await kv.set(LAST_KEY, JSON.stringify({ host: view.host, pane: view.pane }));
}

// --- pane titles -----------------------------------------------------------------------------

const PANE_TITLES_KEY = "rift.paneTitles";

/** The tab title last seen for each pane, by `host:pane`: the header's title before the topology arrives. */
export type PaneTitles = Record<string, string>;

/** Empty when none is stored or it cannot be read; entries that are not short strings are dropped. */
export async function loadPaneTitles(kv: KeyValueStore): Promise<PaneTitles> {
  try {
    const raw = await kv.get(PANE_TITLES_KEY);
    if (raw === null) return {};
    const v: unknown = JSON.parse(raw);
    if (typeof v !== "object" || v === null || Array.isArray(v)) return {};
    return Object.fromEntries(Object.entries(v).filter((e): e is [string, string] => typeof e[1] === "string" && e[1].length <= 256));
  } catch {
    return {};
  }
}

export async function savePaneTitles(kv: KeyValueStore, titles: PaneTitles): Promise<void> {
  await kv.set(PANE_TITLES_KEY, JSON.stringify(titles));
}

/**
 * Where a cold start opens: the last host (and pane) while that host is still paired; null
 * (Now) when it was unpaired, or nothing was open.
 */
export function restoreTarget(last: LastView | null, hosts: readonly { key: string }[]): LastView | null {
  return last && hosts.some((h) => h.key === last.host) ? last : null;
}
