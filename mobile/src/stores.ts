// Persistent app data over a small key-value interface: the platform keychain/keystore
// (expo-secure-store) in the app, a Map in the tests. Free of React Native imports.
//
//   rift.device.key   this device's X25519 private key (base64url)
//   rift.hosts        JSON array of paired gateway keys, in display order
//   rift.host.<key>   one paired desktop (HostRecord)
//   rift.settings     Settings
//
// Hosts live one per entry so no single keychain item grows with the number of desktops.

import { fromBase64Url, toBase64Url } from "./noise/bytes";
import { keyPairFromPrivate, type KeyPair, type RandomBytes } from "./noise/noise";
import { isAddress, normalizeRelayUrl } from "./pairing";
import type { Permission } from "./protocol/types";

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
    (h.via === undefined || h.via === "lan" || (h.via === "relay" && h.relay !== undefined))
  );
}

export class HostsStore {
  constructor(private readonly kv: KeyValueStore) {}

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
    await this.kv.set(hostEntry(host.key), JSON.stringify(host));
    const index = await this.index();
    if (!index.includes(host.key)) await this.kv.set(HOSTS_INDEX, JSON.stringify([...index, host.key]));
  }

  async remove(key: string): Promise<void> {
    const index = await this.index();
    await this.kv.set(HOSTS_INDEX, JSON.stringify(index.filter((k) => k !== key)));
    await this.kv.delete(hostEntry(key));
  }
}

// --- settings --------------------------------------------------------------------------------

export interface Settings {
  /** null: the system device name. */
  deviceName: string | null;
  confirmEveryCommand: boolean;
  /** Notify when a command finishes or fails. */
  notifyDone: boolean;
  showCommandInNotification: boolean;
  /** Face ID / fingerprint (or the device passcode) on launch and after 5 minutes away. */
  biometricLock: boolean;
  /** With the lock and "Confirm every command" on: authenticate before Run. */
  authBeforeRun: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  deviceName: null,
  confirmEveryCommand: true,
  notifyDone: false,
  showCommandInNotification: false,
  biometricLock: false,
  // Takes effect only with the lock on, so turning the lock on also asks before running.
  authBeforeRun: true,
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
      confirmEveryCommand: bool("confirmEveryCommand"),
      notifyDone: bool("notifyDone"),
      showCommandInNotification: bool("showCommandInNotification"),
      biometricLock: bool("biometricLock"),
      authBeforeRun: bool("authBeforeRun"),
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export async function saveSettings(kv: KeyValueStore, settings: Settings): Promise<void> {
  await kv.set(SETTINGS_KEY, JSON.stringify(settings));
}
