// Every paired desktop and its live connection: the protocol clients feed one app state, which
// the batched store publishes to React at most once per 100 ms. Free of React Native imports;
// the platform pieces (storage, randomness, notifications) come in as dependencies
// (src/runtime.ts in the app, fakes in the tests).

import { fromBase64Url } from "./noise/bytes";
import type { KeyPair, RandomBytes } from "./noise/noise";
import { emptyEarlier, fetchRange, mergeRange, nextRequest, type EarlierOutput } from "./earlier";
import {
  newNtfyTopic,
  normalizeNtfyServer,
  notificationText,
  ntfyUrl,
  pushCovers,
  pushRegistration,
  shouldNotify,
  type PushRegistration,
} from "./notify";
import type { PairLink } from "./pairing";
import { BatchedStore } from "./protocol/batch";
import { MAX_OUTPUT_LINES } from "./protocol/types";
import { RiftClient, type ConnState, type Failure, type SocketFactory } from "./protocol/client";
import { canLoadOlder, reducePane, reduceScreen, staleScreen, type PaneState, type ScreenState } from "./protocol/store";
import { findPane } from "./topology";
import {
  isPushToken,
  type AckMsg,
  type DoneMsg,
  type Feature,
  type GatewayHello,
  type ScreenMode,
  type WelcomeMsg,
  type Window,
} from "./protocol/types";
import {
  DEFAULT_SETTINGS,
  HostsStore,
  MAX_DEVICE_NAME,
  loadOrCreateIdentity,
  loadSettings,
  saveSettings,
  type HostRecord,
  type KeyValueStore,
  type Settings,
} from "./stores";

export const MAX_ACTIVITY = 100;
/** An `ack` that has not arrived by then is reported as unanswered. */
export const ACK_TIMEOUT_MS = 10_000;
/** "Notify me" requests waiting for their `done`. */
const MAX_ASKED = 50;
/**
 * A "Notify me" whose `done` has not come by then is dropped: `done` is not replayed, so one
 * that finished while this device was disconnected never arrives. 24 h covers long builds.
 */
export const ASKED_TTL_MS = 24 * 60 * 60_000;

interface Asked {
  host: string;
  pane: number;
  command: string;
  /** The process of the pane's window when asked (null: no topology yet). */
  pid: number | null;
  /** Epoch milliseconds. */
  at: number;
  /** The `notify` reached the computer (a push host): it alerts by itself. */
  sent: boolean;
}

export interface Notifier {
  /** Asks the OS for permission (once); true when granted. */
  requestPermission(): Promise<boolean>;
  notify(n: { title: string; body: string; data: { host: string; pane: number } }): Promise<void>;
}

export interface FleetDeps {
  kv: KeyValueStore;
  randomBytes: RandomBytes;
  platform: string;
  /** The system device name, used until the user sets one. */
  systemDeviceName: string;
  notifier: Notifier;
  socket?: SocketFactory;
  random?: () => number;
  now?: () => number;
}

export interface HostView {
  record: HostRecord;
  conn: ConnState;
  gateway: GatewayHello | null;
  welcome: WelcomeMsg | null;
  windows: Window[];
  /** Last gateway error other than auth (busy, no_pane, …). */
  notice: string | null;
}

export interface PaneView {
  host: string;
  paneId: number;
  /** Blocks once the snapshot arrives. */
  state: PaneState | null;
  loadingOlder: boolean;
  /** The pane's terminal screen while the gateway sends one (`screen` feature). */
  screen: ScreenState | null;
  /**
   * The screen went because none came back after a reconnect (the client's guess that the
   * program ended), not with the gateway's `screen_end`: a picked view is kept.
   */
  screenExpired: boolean;
  /** Earlier output lines loaded per block id (`output` feature); dropped with the block. */
  earlier: Record<number, EarlierOutput>;
}

export interface ActivityItem {
  /** Unique within the session. */
  id: string;
  host: string;
  hostName: string;
  pane: number;
  block: number;
  command: string;
  exit: number | null;
  durationMs: number;
  /** Epoch milliseconds. */
  at: number;
}

export interface AppState {
  ready: boolean;
  /** The device key or the hosts could not be read. */
  loadError: string | null;
  settings: Settings;
  /** settings.deviceName or the system name. */
  deviceName: string;
  hosts: HostView[];
  pane: PaneView | null;
  /** Newest first, at most MAX_ACTIVITY. */
  activity: ActivityItem[];
  /** Unsent command text per `host:pane`. */
  drafts: Record<string, string>;
  /** Alerts while the app is closed (push): on, or why not. */
  push: PushStatus;
}

/**
 * Push alerts: `on` (a token, sent to every computer), `off` (notifications not allowed, or
 * nothing to alert about), or `unavailable` with the reason (this build has no push set up, or
 * the OS gave no token). Without push, notifications come only while the app runs.
 */
export type PushStatus = { state: "on" } | { state: "off" } | { state: "unavailable"; reason: string };

export type ActionResult = { ok: true } | { ok: false; code: string; message: string };

/** `storage`: paired, but this device could not save the host (keychain / keystore refused). */
export type PairFailure = Failure | "invalid" | "storage";

export type PairResult = { ok: true; host: HostRecord } | { ok: false; failure: PairFailure; message: string };

export const draftKey = (host: string, pane: number) => `${host}:${pane}`;

const initialConn: ConnState = {
  status: "idle",
  attempt: 0,
  retryInMs: null,
  error: null,
  failure: null,
  address: null,
  path: null,
};

/** Short, human error text for an `ack` or `error` code. */
export function describeCode(code: string, hostName: string, fallback: string): string {
  switch (code) {
    case "busy":
      return "This pane is running a command. Use the keys row to talk to the program.";
    case "forbidden":
      return "This device may only view this computer.";
    case "rate":
      return "Too many at once. Wait a moment and try again.";
    case "no_pane":
      return "This pane has closed.";
    case "too_large":
      return "The command is too long.";
    case "timeout":
      return `No answer from ${hostName}.`;
    case "disconnected":
      return `Not connected to ${hostName}.`;
    default:
      return fallback || `${hostName} refused it (${code}).`;
  }
}

/**
 * Whether this device may run commands and send keys on `host`: the permission of the current
 * welcome when there is one (a desktop that changes it reconnects with the new one), else the
 * stored record's.
 */
export function canOperate(host: HostView): boolean {
  return (host.welcome?.permission ?? host.record.permission) === "operate";
}

/**
 * The paired computers whose last welcome (this app run) lacks `push_ntfy`: their Rift is too old
 * to send ntfy alerts. Computers not reached yet are left out (their version is unknown).
 */
export function ntfyUnsupported(hosts: HostView[]): HostView[] {
  return hosts.filter((h) => h.welcome !== null && !hasFeature(h, "push_ntfy"));
}

/** Whether the gateway's last welcome lists `feature` (an older gateway, or no welcome yet: no). */
export function hasFeature(host: HostView | undefined, feature: Feature): boolean {
  return host?.welcome?.features.includes(feature) ?? false;
}

/** The earlier lines held for each block still in `state`; the others are dropped. */
function keepEarlier(
  earlier: Record<number, EarlierOutput>,
  state: PaneState | null,
): Record<number, EarlierOutput> {
  const ids = Object.keys(earlier);
  if (ids.length === 0) return earlier;
  const live = new Set(state?.blocks.map((b) => b.id));
  return ids.every((id) => live.has(Number(id)))
    ? earlier
    : Object.fromEntries(Object.entries(earlier).filter(([id]) => live.has(Number(id))));
}

export function describePairFailure(failure: PairFailure, name: string, viaRelay = false): string {
  switch (failure) {
    case "auth":
      return "This pairing code has expired or was already used. Create a new one on the computer.";
    case "unreachable":
      return viaRelay
        ? `Could not reach ${name} directly or through its relay. Check that remote access is on.`
        : `Could not reach ${name}. Check that this device is on the same network and that remote access is on.`;
    case "key_mismatch":
      return `The computer that answered is not ${name}: its key differs from the pairing code. Nothing was sent.`;
    case "version":
      return `${name} runs a Rift version this app cannot talk to. Update both.`;
    case "invalid":
      return "This is not a Rift pairing link.";
    case "storage":
      return `${name} accepted this device, but it could not be saved here. Unlock the device, create a new pairing code on the computer and try again.`;
  }
}

interface Pending {
  host: string;
  resolve: (r: ActionResult) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class Fleet {
  readonly store: BatchedStore<AppState>;
  private readonly deps: FleetDeps;
  private readonly hostsStore: HostsStore;
  private readonly now: () => number;
  private identity: KeyPair | null = null;
  private readonly clients = new Map<string, RiftClient>();
  private readonly pending = new Map<string, Pending>();
  private asked: Asked[] = [];
  /** When each host's last welcome came, until the topology that follows it. */
  private readonly welcomedAt = new Map<string, number>();
  private appActive = true;
  /** The app is not in the background (`setVisible`); the computers know it (`visible`). */
  private visible = true;
  private viewing: { host: string; pane: number } | null = null;
  private activityN = 0;
  private initStarted = false;
  /** Connections are held closed: no network (`offline`), or the app is in the background. */
  private paused: "offline" | "closed" | null = null;
  private askedListener: (() => void) | null = null;
  /** Hosts unpaired in this app run (until paired again): a record save in flight must not bring one back. */
  private readonly unpaired = new Set<string>();
  /** This device's Expo push token (null: none). */
  private pushToken: string | null = null;
  /**
   * Hosts that hold this device's push token or ntfy topic, with what they were told to alert about: they alert
   * through push even while the app runs, so for a `done` they cover the app posts no
   * notification of its own (one alert per command).
   */
  private readonly pushHosts = new Map<string, PushRegistration>();
  /** Hosts told "no token" in this app run (once, in case one still holds a token of an earlier run). */
  private readonly pushCleared = new Set<string>();

  constructor(deps: FleetDeps) {
    this.deps = deps;
    this.hostsStore = new HostsStore(deps.kv);
    this.now = deps.now ?? Date.now;
    this.store = new BatchedStore<AppState>({
      ready: false,
      loadError: null,
      settings: DEFAULT_SETTINGS,
      deviceName: deps.systemDeviceName,
      hosts: [],
      pane: null,
      activity: [],
      drafts: {},
      push: { state: "off" },
    });
  }

  /** Loads identity, settings and hosts, then connects every host. Runs once. */
  async init(): Promise<void> {
    if (this.initStarted) return;
    this.initStarted = true;
    try {
      const settings = await loadSettings(this.deps.kv);
      this.identity = await loadOrCreateIdentity(this.deps.kv, this.deps.randomBytes);
      const records = await this.hostsStore.load();
      this.store.update((s) => ({
        ...s,
        settings,
        deviceName: settings.deviceName ?? this.deps.systemDeviceName,
        hosts: records.map((record) => ({
          record,
          conn: initialConn,
          gateway: null,
          welcome: null,
          windows: [],
          notice: null,
        })),
      }));
      for (const r of records) this.connect(r);
      this.store.update((s) => ({ ...s, ready: true }));
    } catch (e) {
      this.store.update((s) => ({
        ...s,
        ready: true,
        loadError: e instanceof Error ? e.message : String(e),
      }));
    }
    this.store.flush();
  }

  /** Loads again after a `loadError` (the keychain was locked, say). */
  retryInit(): Promise<void> {
    if (!this.store.peek().loadError) return Promise.resolve();
    this.initStarted = false;
    this.store.update((s) => ({ ...s, ready: false, loadError: null }));
    return this.init();
  }

  get state(): AppState {
    return this.store.peek();
  }

  host(key: string): HostView | undefined {
    return this.store.peek().hosts.find((h) => h.record.key === key);
  }

  // --- connections -----------------------------------------------------------------------------

  private hello = () => ({
    name: this.store.peek().deviceName,
    platform: this.deps.platform,
  });

  private connect(record: HostRecord): void {
    const gatewayKey = fromBase64Url(record.key);
    if (!this.identity || gatewayKey?.length !== 32) return;
    this.clients.get(record.key)?.stop();
    const key = record.key;
    const client = new RiftClient({
      addresses: record.addresses,
      relay: record.relay ?? null,
      preferRelay: record.via === "relay",
      identity: this.identity,
      gatewayKey,
      hello: this.hello,
      randomBytes: this.deps.randomBytes,
      socket: this.deps.socket,
      random: this.deps.random,
      events: {
        onState: (conn) => {
          this.patchHost(key, { conn });
          if (conn.status !== "open") this.failPending(key, "disconnected");
        },
        onGatewayHello: (gateway) => this.patchHost(key, { gateway }),
        onWelcome: (welcome) => {
          this.patchHost(key, { welcome, notice: null });
          this.welcomedAt.set(key, this.now());
          this.sendVisible(key, client, true);
          this.sendPush(key, client);
          void this.refreshRecord(key, welcome, client);
        },
        onTopology: (msg) => {
          this.patchHost(key, { windows: msg.windows });
          const since = this.welcomedAt.get(key);
          if (since === undefined) return;
          this.welcomedAt.delete(key);
          this.dropUnreachableAsks(key, msg.windows, since);
        },
        onPane: (msg) =>
          this.store.update((s) => {
            if (s.pane?.host !== key || s.pane.paneId !== msg.pane) return s;
            const state = reducePane(s.pane.state, msg);
            return {
              ...s,
              pane: {
                ...s.pane,
                state,
                loadingOlder: msg.t === "older" ? false : s.pane.loadingOlder,
                earlier: keepEarlier(s.pane.earlier, state),
              },
            };
          }),
        onScreen: (msg, synthetic) =>
          this.store.update((s) =>
            s.pane?.host === key && s.pane.paneId === msg.pane
              ? { ...s, pane: { ...s.pane, screen: reduceScreen(s.pane.screen, msg), screenExpired: !!synthetic } }
              : s,
          ),
        onScreenStale: (pane) =>
          this.store.update((s) =>
            s.pane?.host === key && s.pane.paneId === pane
              ? { ...s, pane: { ...s.pane, screen: staleScreen(s.pane.screen) } }
              : s,
          ),
        onAck: (msg) => this.onAck(key, msg),
        onDone: (msg) => this.onDone(key, msg),
        onError: (msg) => {
          if (msg.code === "auth" || msg.code === "version") return;
          // An older gateway refuses a view device's run/keys with a top-level error instead of
          // an ack: fail what waits rather than letting it time out.
          if (msg.code === "forbidden") this.failPending(key, "forbidden");
          this.patchHost(key, { notice: msg.message });
          this.store.update((s) => (s.pane?.host === key ? { ...s, pane: { ...s.pane, loadingOlder: false } } : s));
        },
      },
    });
    this.clients.set(key, client);
    if (this.paused) client.suspend(this.paused);
    else client.start();
  }

  /**
   * Keeps the stored record in step with what the gateway says: name, permission (a desktop
   * that changes it closes the connection, and the next welcome carries the new one), and the
   * path that worked (address order, LAN or relay).
   */
  private async refreshRecord(key: string, welcome: WelcomeMsg, client: RiftClient): Promise<void> {
    const view = this.host(key);
    if (!view) return;
    const r = view.record;
    const address = client.preferredAddress;
    const addresses = [address, ...r.addresses.filter((a) => a !== address)];
    const via = client.preferredPath.kind;
    if (
      r.name === welcome.host &&
      r.permission === welcome.permission &&
      addresses[0] === r.addresses[0] &&
      (r.via ?? "lan") === via
    ) {
      return;
    }
    const record: HostRecord = {
      ...r,
      name: welcome.host || r.name,
      permission: welcome.permission,
      addresses,
      via,
    };
    this.patchHost(key, { record });
    try {
      await this.hostsStore.save(record);
      // Unpaired while the save was in flight: the save may have written the host back.
      if (this.unpaired.has(key)) await this.hostsStore.remove(key);
    } catch {
      // Kept in memory; the next welcome tries again.
    }
  }

  private patchHost(key: string, patch: Partial<HostView>): void {
    this.store.update((s) => {
      const i = s.hosts.findIndex((h) => h.record.key === key);
      if (i === -1) return s;
      const hosts = [...s.hosts];
      hosts[i] = { ...hosts[i]!, ...patch };
      return { ...s, hosts };
    });
  }

  /** The app came to the foreground (true) or left it (false); notifications depend on it. */
  setAppActive(active: boolean): void {
    this.appActive = active;
  }

  /**
   * The app went to the background (false) or came back (true). Only `background` counts: an
   * `inactive` app (Face ID prompt, Control Center) is still on screen. A backgrounded app keeps
   * its sockets for a while but cannot notify (iOS suspends it): the computers then post ntfy
   * alerts about the pane it follows again (`visible`).
   */
  setVisible(visible: boolean): void {
    if (this.visible === visible) return;
    this.visible = visible;
    for (const [key, c] of this.clients) this.sendVisible(key, c, false);
  }

  /**
   * Tells `host` whether the app is on screen, when it understands it (`push_ntfy`). `onWelcome`:
   * a new connection starts visible, so only "not on screen" needs saying then.
   */
  private sendVisible(host: string, client: RiftClient, onWelcome: boolean): void {
    if (!hasFeature(this.host(host), "push_ntfy") || (onWelcome && this.visible)) return;
    client.setVisible(this.visible);
  }

  /**
   * Closes every connection and holds them closed: `offline` (no network; hosts show Offline)
   * or `closed` (the app is in the background). Hosts that failed keep their failure.
   */
  suspendConnections(status: "offline" | "closed"): void {
    this.paused = status;
    for (const c of this.clients.values()) c.suspend(status);
  }

  /**
   * Connects again at once with the backoff reset. `force` also drops connections that are
   * open (the network changed under them); `lanFirst` tries the LAN addresses before the
   * last path that worked. Hosts that failed wait for the user, except a key mismatch on a
   * network change: on the new network the addresses may reach the right computer.
   */
  resumeConnections(opts: { lanFirst: boolean; force: boolean }): void {
    this.paused = null;
    for (const c of this.clients.values()) {
      const st = c.connState.status;
      if (st === "failed" && !(opts.force && c.connState.failure === "key_mismatch")) continue;
      if (!opts.force && (st === "open" || st === "connecting")) continue;
      if (!opts.force && st === "closed" && c.connState.retryInMs !== null) c.reconnectNow();
      else c.restart({ lanFirst: opts.lanFirst });
    }
  }

  /** A "Notify me" is waiting for its command to finish (keeps the socket up in the background). */
  get hasPendingNotify(): boolean {
    return this.asked.some(this.askLive);
  }

  private askLive = (a: Asked) => this.now() - a.at < ASKED_TTL_MS;

  /**
   * The first topology after a welcome: asks made before it whose pane is gone, or now lives in
   * another process (the gateway restarted), can never see their `done`, so they are dropped.
   */
  private dropUnreachableAsks(host: string, windows: Window[], since: number): void {
    const keep = (a: Asked) => {
      if (a.host !== host || a.at > since) return true;
      const loc = findPane(windows, a.pane);
      return loc !== null && (a.pid === null || loc.window.pid === a.pid);
    };
    if (!this.asked.every(keep)) this.setAsked(this.asked.filter(keep));
  }

  /** Called whenever hasPendingNotify may have changed. */
  onPendingNotifyChange(listener: (() => void) | null): void {
    this.askedListener = listener;
  }

  private setAsked(list: Asked[]): void {
    this.asked = list.filter(this.askLive);
    this.askedListener?.();
  }

  /**
   * Retries a host that stopped (auth or key failure shown to the user), or waits less. While
   * connections are paused (offline, background), a stopped host is set up again but held
   * until they resume, and a waiting one is left alone.
   */
  retry(key: string): void {
    const view = this.host(key);
    if (!view) return;
    const c = this.clients.get(key);
    if (c && c.connState.status !== "failed") {
      if (!this.paused) c.restart();
    } else {
      this.connect(view.record);
    }
  }

  // --- pairing ---------------------------------------------------------------------------------

  /** Pairing secrets this app run has spent. Android re-delivers the launching deep link when it
   * recreates the activity (rotation, font size); a spent link then leads to its host, not to a
   * second pairing attempt that could only fail. */
  private readonly spentSecrets = new Set<string>();

  /** Whether `link`'s secret was spent by a pairing in this app run. */
  pairedWith(link: PairLink): boolean {
    return this.spentSecrets.has(link.secret);
  }

  /**
   * Pairs with the desktop in `link`: tries its addresses in order once, with the secret in the
   * client hello. On `welcome` the host is stored and connected like any other.
   */
  pair(link: PairLink): { result: Promise<PairResult>; cancel: () => void } {
    const gatewayKey = fromBase64Url(link.key);
    if (!this.identity || gatewayKey?.length !== 32) {
      const message = this.identity ? describePairFailure("invalid", link.name) : "This device's key is not loaded.";
      return {
        result: Promise.resolve({ ok: false, failure: "invalid", message }),
        cancel: () => undefined,
      };
    }
    const identity = this.identity;
    let cancel = () => undefined as void;
    const result = new Promise<PairResult>((resolve) => {
      let settled = false;
      const finish = (r: PairResult) => {
        if (settled) return;
        settled = true;
        client.stop();
        resolve(r);
      };
      cancel = () => finish({ ok: false, failure: "unreachable", message: "Cancelled." });
      const client: RiftClient = new RiftClient({
        addresses: link.addresses,
        relay: link.relay,
        identity,
        gatewayKey,
        hello: () => ({ ...this.hello(), pair: link.secret }),
        randomBytes: this.deps.randomBytes,
        socket: this.deps.socket,
        random: this.deps.random,
        retry: false,
        events: {
          onState: (conn) => {
            if (conn.status === "failed" && conn.failure) {
              finish({
                ok: false,
                failure: conn.failure,
                message: describePairFailure(conn.failure, link.name, link.relay !== null),
              });
            }
          },
          onWelcome: (welcome) => {
            if (settled) return;
            settled = true;
            client.stop();
            const address = client.preferredAddress;
            const record: HostRecord = {
              key: link.key,
              name: welcome.host || link.name,
              addresses: [address, ...link.addresses.filter((a) => a !== address)],
              permission: welcome.permission,
              pairedAt: new Date(this.now()).toISOString(),
              ...(link.relay ? { relay: link.relay } : {}),
              via: client.preferredPath.kind,
            };
            // Paired on the computer; it counts here only once the keychain holds it.
            this.adopt(record).then(
              () => {
                this.spentSecrets.add(link.secret);
                resolve({ ok: true, host: record });
              },
              () =>
                resolve({
                  ok: false,
                  failure: "storage",
                  message: describePairFailure("storage", record.name),
                }),
            );
          },
        },
      });
      client.start();
    });
    return { result, cancel: () => cancel() };
  }

  /** Stores a newly paired host (replacing one with the same key) and connects it. */
  private async adopt(record: HostRecord): Promise<void> {
    this.unpaired.delete(record.key);
    await this.hostsStore.save(record);
    this.store.update((s) => {
      const view: HostView = {
        record,
        conn: initialConn,
        gateway: null,
        welcome: null,
        windows: [],
        notice: null,
      };
      const i = s.hosts.findIndex((h) => h.record.key === record.key);
      const hosts = i === -1 ? [...s.hosts, view] : s.hosts.map((h, j) => (j === i ? view : h));
      return { ...s, hosts };
    });
    this.connect(record);
    this.store.flush();
  }

  /** Removes a desktop: tells it to forget this device when connected, then deletes it here. */
  async unpair(key: string): Promise<void> {
    this.unpaired.add(key);
    const client = this.clients.get(key);
    if (client) {
      // WebSocket close() sends queued data first, so the forget goes out before the close.
      client.forget();
      client.stop();
      this.clients.delete(key);
    }
    this.failPending(key, "disconnected");
    await this.hostsStore.remove(key);
    this.setAsked(this.asked.filter((a) => a.host !== key));
    this.store.update((s) => ({
      ...s,
      hosts: s.hosts.filter((h) => h.record.key !== key),
      pane: s.pane?.host === key ? null : s.pane,
      activity: s.activity.filter((a) => a.host !== key),
      drafts: Object.fromEntries(Object.entries(s.drafts).filter(([k]) => !k.startsWith(`${key}:`))),
    }));
    if (this.viewing?.host === key) this.viewing = null;
    this.store.flush();
  }

  // --- panes -----------------------------------------------------------------------------------

  /** The screens holding the followed pane open ([openPane]). */
  private readonly paneHolders = new Set<object>();

  /**
   * Follows pane `paneId` on `host` (one pane at a time across hosts) for `holder` (the screen
   * showing it): the pane is left only when every screen holding it has closed it, so the same
   * pane pushed twice and popped once keeps following.
   */
  openPane(host: string, paneId: number, holder: object = this): void {
    const cur = this.store.peek().pane;
    const client = this.clients.get(host);
    const same = cur?.host === host && cur.paneId === paneId;
    if (!same) this.paneHolders.clear();
    this.paneHolders.add(holder);
    if (same && client?.subscribedPane === paneId) return;
    if (cur && cur.host !== host) this.clients.get(cur.host)?.unsubscribe();
    this.store.update((s) => ({
      ...s,
      pane: {
        host,
        paneId,
        state: s.pane?.host === host && s.pane.paneId === paneId ? s.pane.state : null,
        loadingOlder: false,
        screen: s.pane?.host === host && s.pane.paneId === paneId ? s.pane.screen : null,
        screenExpired: s.pane?.host === host && s.pane.paneId === paneId ? s.pane.screenExpired : false,
        earlier: s.pane?.host === host && s.pane.paneId === paneId ? s.pane.earlier : {},
      },
    }));
    this.patchHost(host, { notice: null });
    client?.subscribe(paneId);
  }

  closePane(host: string, paneId: number, holder: object = this): void {
    const cur = this.store.peek().pane;
    if (cur?.host !== host || cur.paneId !== paneId) return;
    this.paneHolders.delete(holder);
    if (this.paneHolders.size > 0) return;
    this.clients.get(host)?.unsubscribe();
    this.store.update((s) => ({ ...s, pane: null }));
  }

  /** The pane on screen, for notifications (null when none is). */
  setViewing(view: { host: string; pane: number } | null): void {
    this.viewing = view;
  }

  loadOlder(): void {
    const p = this.store.peek().pane;
    const first = p?.state?.blocks[0];
    const client = p ? this.clients.get(p.host) : undefined;
    if (!p || !first || !client || p.loadingOlder || !canLoadOlder(p.state)) return;
    this.store.update((s) => (s.pane ? { ...s, pane: { ...s.pane, loadingOlder: true } } : s));
    client.requestOlder(first.id);
  }

  /**
   * What the followed pane's gateway sends of its screen (only a gateway with `screen`); the
   * client sends it again after every re-subscribe and reconnect.
   */
  setScreenMode(host: string, pane: number, mode: ScreenMode): void {
    const p = this.store.peek().pane;
    if (p?.host !== host || p.paneId !== pane || !hasFeature(this.host(host), "screen")) return;
    this.clients.get(host)?.setScreen(pane, mode);
  }

  /**
   * "Load earlier output" for block `blockId` of the followed pane (only a gateway with
   * `output`): fetches the next up to 500 lines before the held ones (earlier.ts) and keeps
   * them with the block. The answer is dropped when the pane or the block went meanwhile.
   */
  async loadEarlier(blockId: number): Promise<void> {
    const p = this.store.peek().pane;
    const block = p?.state?.blocks.find((b) => b.id === blockId);
    const client = p ? this.clients.get(p.host) : undefined;
    if (!p || !block || !client || !hasFeature(this.host(p.host), "output")) return;
    const cur = p.earlier[blockId];
    const req = cur?.loading ? null : nextRequest(block, cur);
    if (!req) return;
    const { host, paneId } = p;
    const patch = (f: (e: EarlierOutput) => EarlierOutput) =>
      this.store.update((s) => {
        const b = s.pane?.state?.blocks.find((x) => x.id === blockId);
        if (s.pane?.host !== host || s.pane.paneId !== paneId || !b) return s;
        return {
          ...s,
          pane: { ...s.pane, earlier: { ...s.pane.earlier, [blockId]: f(s.pane.earlier[blockId] ?? emptyEarlier(b)) } },
        };
      });
    patch((e) => ({ ...e, loading: true, error: null }));
    const name = this.host(host)?.record.name ?? "the computer";
    const r = await fetchRange(
      async (from, limit) => {
        const a = await client.output(paneId, blockId, from, Math.min(limit, MAX_OUTPUT_LINES));
        return a.ok ? { ok: true, from: a.msg.from, text: a.msg.text } : a;
      },
      req.from,
      req.from + req.limit,
    );
    patch((e) =>
      r.ok
        ? mergeRange(block, e, req, r)
        : {
            ...e,
            loading: false,
            error:
              r.code === "no_block"
                ? "This block is gone from the computer."
                : r.code === "incomplete"
                  ? "The computer sent fewer lines than asked. Try again."
                  : describeCode(r.code, name, r.message),
          },
    );
    this.store.flush();
  }

  setDraft(host: string, pane: number, text: string): void {
    const k = draftKey(host, pane);
    this.store.update((s) => (s.drafts[k] === text ? s : { ...s, drafts: { ...s.drafts, [k]: text } }));
    // The command field is controlled by this value: publish at once, or keystrokes typed
    // within one batching interval would be overwritten by the stale value.
    this.store.flush();
  }

  // --- operate ---------------------------------------------------------------------------------

  /**
   * Runs `command` at the pane's prompt; resolves with the gateway's `ack`. On success the
   * pane's draft is cleared, unless `fromDraft` is false (Run again of a block's command).
   */
  run(
    host: string,
    pane: number,
    command: string,
    opts: { notify?: boolean; fromDraft?: boolean } = {},
  ): Promise<ActionResult> {
    const result = this.request(host, (c) => c.run(pane, command));
    if (opts.notify) this.notifyWhenDone(host, pane, command);
    return result.then((r) => {
      if (r.ok && opts.fromDraft !== false) this.setDraft(host, pane, "");
      else if (!r.ok && opts.notify)
        this.setAsked(this.asked.filter((a) => !(a.host === host && a.pane === pane && a.command === command)));
      return r;
    });
  }

  /**
   * "Notify me when it finishes": one notification when `command` in `pane` finishes, whatever
   * the settings say (also from the computer through push when the app is closed by then).
   */
  notifyWhenDone(host: string, pane: number, command: string): void {
    const pid = findPane(this.host(host)?.windows ?? [], pane)?.window.pid ?? null;
    const sent = this.pushHosts.has(host) && (this.clients.get(host)?.notify(pane) ?? false);
    this.setAsked([...this.asked, { host, pane, command, pid, at: this.now(), sent }].slice(-MAX_ASKED));
    void this.deps.notifier.requestPermission();
  }

  /** Sends the "Notify me" requests a push host has not received yet (asked while disconnected). */
  private sendAsked(host: string, client: RiftClient): void {
    if (!this.pushHosts.has(host)) return;
    let changed = false;
    const list = this.asked.map((a) => {
      if (a.host !== host || a.sent || !this.askLive(a) || !client.notify(a.pane)) return a;
      changed = true;
      return { ...a, sent: true };
    });
    if (changed) this.setAsked(list);
  }

  /** Milliseconds until keys or text to `host` would go out (the gateway's input rate; 0: now). */
  inputDelay(host: string): number {
    return this.clients.get(host)?.inputDelayMs ?? 0;
  }

  sendKeys(host: string, pane: number, keys: string[]): Promise<ActionResult> {
    return this.request(host, (c) => c.keys(pane, keys));
  }

  /** Types text into the program running in the pane (only a gateway with `text`). */
  sendText(host: string, pane: number, text: string): Promise<ActionResult> {
    return this.request(host, (c) => c.text(pane, text));
  }

  private request(host: string, send: (c: RiftClient) => string | null): Promise<ActionResult> {
    const name = this.host(host)?.record.name ?? "the computer";
    const client = this.clients.get(host);
    const id = client ? send(client) : null;
    if (id === null) {
      return Promise.resolve({
        ok: false,
        code: "disconnected",
        message: describeCode("disconnected", name, ""),
      });
    }
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(this.pendingKey(host, id));
        resolve({
          ok: false,
          code: "timeout",
          message: describeCode("timeout", name, ""),
        });
      }, ACK_TIMEOUT_MS);
      this.pending.set(this.pendingKey(host, id), { host, resolve, timer });
    });
  }

  private pendingKey = (host: string, id: string) => `${host}\n${id}`;

  private onAck(host: string, msg: AckMsg): void {
    // Ids are unique per client; the host part keeps two hosts' "r1" apart.
    const k = this.pendingKey(host, msg.id);
    const p = this.pending.get(k);
    if (!p) return;
    this.pending.delete(k);
    clearTimeout(p.timer);
    const name = this.host(host)?.record.name ?? "the computer";
    const code = msg.error ?? "error";
    p.resolve(
      msg.ok
        ? { ok: true }
        : {
            ok: false,
            code,
            message: describeCode(code, name, msg.message ?? ""),
          },
    );
  }

  private failPending(host: string, code: string): void {
    const name = this.host(host)?.record.name ?? "the computer";
    for (const [k, p] of this.pending) {
      if (p.host !== host) continue;
      this.pending.delete(k);
      clearTimeout(p.timer);
      p.resolve({ ok: false, code, message: describeCode(code, name, "") });
    }
  }

  // --- done → activity and notifications --------------------------------------------------------

  private onDone(host: string, msg: DoneMsg): void {
    const view = this.host(host);
    const hostName = view?.record.name ?? "your computer";
    const item: ActivityItem = {
      id: `a${++this.activityN}`,
      host,
      hostName,
      pane: msg.pane,
      block: msg.block,
      command: msg.command,
      exit: msg.exit,
      durationMs: msg.duration_ms,
      at: this.now(),
    };
    this.store.update((s) => ({
      ...s,
      activity: [item, ...s.activity].slice(0, MAX_ACTIVITY),
    }));

    const i = this.asked.findIndex(
      (a) => a.host === host && a.pane === msg.pane && a.command === msg.command && this.askLive(a),
    );
    const asked = i !== -1;
    const askSent = asked && this.asked[i]!.sent;
    if (asked) this.setAsked(this.asked.filter((_, j) => j !== i));
    const settings = this.store.peek().settings;
    const notify = shouldNotify({
      appActive: this.appActive,
      viewing: this.viewing,
      host,
      pane: msg.pane,
      durationMs: msg.duration_ms,
      exit: msg.exit,
      asked,
      policy: settings.notifyPolicy,
      afterMs: settings.notifyAfterMs,
      muted: view?.record.notifications === false,
    });
    // A computer with this device's push token alerts by itself (also while the app runs) for
    // what it was told: a "Notify me" it received, or a long command its registration covers.
    const reg = this.pushHosts.get(host);
    // the gateway skips ntfy only for the pane followed by an app on screen (`visible`)
    const followed = this.visible && this.clients.get(host)?.subscribedPane === msg.pane;
    if (!notify || (reg && pushCovers(reg, { askSent, durationMs: msg.duration_ms, exit: msg.exit, followed }))) return;
    const text = notificationText(hostName, msg.exit, msg.command, settings.showCommandInNotification);
    void this.deps.notifier.notify({ ...text, data: { host, pane: msg.pane } }).catch(() => undefined);
  }

  // --- settings --------------------------------------------------------------------------------

  async updateSettings(patch: Partial<Settings>): Promise<void> {
    const cur = this.store.peek().settings;
    const next: Settings = { ...cur, ...patch };
    if (next.deviceName !== null) {
      const name = next.deviceName.trim().slice(0, MAX_DEVICE_NAME);
      next.deviceName = name === "" ? null : name;
    }
    if (patch.notifyPolicy && patch.notifyPolicy !== "off" && cur.notifyPolicy === "off") {
      // Asked lazily, on first enable; the setting stays off when the OS refuses.
      const granted = await this.deps.notifier.requestPermission();
      if (!granted) next.notifyPolicy = "off";
    }
    this.store.update((s) => ({
      ...s,
      settings: next,
      deviceName: next.deviceName ?? this.deps.systemDeviceName,
    }));
    this.store.flush();
    if (
      next.notifyPolicy !== cur.notifyPolicy ||
      next.notifyAfterMs !== cur.notifyAfterMs ||
      next.showCommandInNotification !== cur.showCommandInNotification ||
      next.ntfyServer !== cur.ntfyServer ||
      next.ntfyEnabled !== cur.ntfyEnabled ||
      next.ntfyTopic !== cur.ntfyTopic
    ) {
      for (const [key, c] of this.clients) this.sendPush(key, c);
    }
    await saveSettings(this.deps.kv, next);
  }

  /**
   * A paired computer's Notifications switch: off, it alerts only for "Notify me" (here and
   * through push). Saved with the host.
   */
  async setHostNotifications(key: string, on: boolean): Promise<void> {
    const view = this.host(key);
    if (!view || (view.record.notifications !== false) === on) return;
    const { notifications: _, ...rest } = view.record;
    const record: HostRecord = on ? rest : { ...rest, notifications: false };
    this.patchHost(key, { record });
    this.store.flush();
    const c = this.clients.get(key);
    if (c) this.sendPush(key, c);
    await this.hostsStore.save(record);
    if (this.unpaired.has(key)) await this.hostsStore.remove(key);
  }

  // --- push --------------------------------------------------------------------------------------

  /**
   * Settings › "Alerts through ntfy": on uses the stored topic (the secret that lets the ntfy app
   * read the alerts), making a random one the first time; off tells the computers to stop posting
   * there and keeps the topic, so on again needs no new subscription in the ntfy app.
   */
  async setNtfy(on: boolean): Promise<void> {
    const cur = this.store.peek().settings;
    if (cur.ntfyEnabled === on && (!on || cur.ntfyTopic !== null)) return;
    await this.updateSettings(
      on ? { ntfyEnabled: true, ntfyTopic: cur.ntfyTopic ?? newNtfyTopic(this.deps.randomBytes) } : { ntfyEnabled: false },
    );
  }

  /**
   * "New topic": a new random topic, sent to every computer at once. Computers that get it stop
   * posting to the old one; one that is offline may post there until it reconnects, one unpaired
   * while offline may keep posting there. The ntfy app needs a new subscription.
   */
  async rotateNtfyTopic(): Promise<void> {
    await this.updateSettings({ ntfyTopic: newNtfyTopic(this.deps.randomBytes) });
  }

  /** The ntfy server (self-hosted, or ntfy.sh); false when it is not an https server address. */
  async setNtfyServer(input: string): Promise<boolean> {
    const server = normalizeNtfyServer(input);
    if (server === null) return false;
    await this.updateSettings({ ntfyServer: server });
    return true;
  }

  /** This device's push token (null: none) and, without one, why (shown in Settings). */
  setPushToken(token: string | null, reason: string | null = null): void {
    const valid = token !== null && isPushToken(token) ? token : null;
    this.pushToken = valid;
    const push: PushStatus = valid ? { state: "on" } : reason ? { state: "unavailable", reason } : { state: "off" };
    this.store.update((s) => ({ ...s, push }));
    for (const [key, c] of this.clients) this.sendPush(key, c);
  }

  /** Whether an alert about `host`'s `pane` would show what is already on screen. */
  isViewing(host: string, pane: number): boolean {
    return this.appActive && this.viewing?.host === host && this.viewing.pane === pane;
  }

  /**
   * Tells `host` where to alert while the app is closed (or to stop: no token and no ntfy topic).
   * The ntfy topic goes only to a computer with `push_ntfy`; an older one gets the message as before.
   * With ntfy on, ntfy replaces Expo push there (no token): one alert per command, not two.
   */
  private sendPush(host: string, client: RiftClient): void {
    const settings = this.store.peek().settings;
    const ntfySupported = hasFeature(this.host(host), "push_ntfy");
    const ntfy = ntfySupported ? ntfyUrl(settings) : null;
    const token = ntfy ? null : this.pushToken;
    if (!token && !ntfy && !this.pushHosts.has(host) && this.pushCleared.has(host)) return;
    const muted = this.host(host)?.record.notifications === false;
    const reg = pushRegistration(
      settings.notifyPolicy,
      settings.notifyAfterMs,
      muted,
      hasFeature(this.host(host), "push_failures"),
    );
    const sent = client.setPush({
      token,
      ...(ntfySupported ? { ntfy } : {}),
      ...reg,
      showCommand: settings.showCommandInNotification,
    });
    if (!sent) return;
    if (token || ntfy) {
      this.pushHosts.set(host, token ? reg : { ...reg, ntfyOnly: true });
      this.sendAsked(host, client);
    } else {
      this.pushHosts.delete(host);
      this.pushCleared.add(host);
    }
  }

  /** Stops every connection (tests). */
  shutdown(): void {
    for (const c of this.clients.values()) c.stop();
    this.clients.clear();
  }
}
