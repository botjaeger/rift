// Connection to one Rift remote gateway (docs/remote-protocol.md): binary WebSocket, Noise XX
// handshake with the pinned gateway key, chunked JSON messages, keep-alive pings, address
// failover, reconnect with backoff, resume by `seq`, gap detection.

import { utf8Decode, utf8Encode } from "../noise/bytes";
import {
  NoiseError,
  NoiseKeyMismatchError,
  XXHandshake,
  type CipherState,
  type KeyPair,
  type RandomBytes,
} from "../noise/noise";
import { FramingError, Reassembler, frameMessage } from "./framing";
import { endpointsFor, relayConnectUrl, type Endpoint } from "./relay";
import {
  MAX_CLIENT_MESSAGE_BYTES,
  MAX_UPLOAD_MESSAGE_BYTES,
  PROLOGUE,
  PROTOCOL_VERSION,
  isGatewayHello,
  parseServerFrame,
  utf8Length,
  type AckMsg,
  type ClientHello,
  type ClientMsg,
  type DoneMsg,
  type ErrorMsg,
  type GatewayHello,
  type OlderMsg,
  type CloseKind,
  type MoveKind,
  type OpenKind,
  type OutputMsg,
  type ChipAnswerMsg,
  type ChipDetailKind,
  type ChipMsg,
  type ChipRequestMsg,
  type ChipDetailRequestMsg,
  type CompleteMsg,
  type CompleteRequestMsg,
  type DiffMsg,
  type DiffRequestMsg,
  type ScreenEndMsg,
  type Theme,
  type ScreenMode,
  type ScreenMsg,
  type ServerMsg,
  type SnapshotMsg,
  type TopologyMsg,
  type UpdateMsg,
  type WelcomeMsg,
} from "./types";

export const PING_INTERVAL_MS = 15_000;
/** The gateway drops a client silent for 45 s; the client gives up on a silent gateway alike. */
export const IDLE_TIMEOUT_MS = 45_000;
/**
 * From opening the socket to a finished handshake. The gateway allows 5 s after the socket
 * opens; the extra covers the TCP connect, which on a dead address can otherwise hang a minute.
 */
export const HANDSHAKE_TIMEOUT_MS = 8_000;
/**
 * A LAN address when a relay is there to fall back on (docs/remote-protocol.md, Relay): off the
 * home network the addresses never answer, and the relay should not wait behind them.
 */
export const LAN_HANDSHAKE_TIMEOUT_MS = 1_500;
/** Through the relay: it gives the gateway 10 s to accept, then the gateway's own handshake. */
export const RELAY_HANDSHAKE_TIMEOUT_MS = 12_000;
export const BACKOFF_BASE_MS = 500;
export const BACKOFF_CAP_MS = 10_000;
/** Reconnect rounds after a failed one, before the client stops and waits for the user. */
export const MAX_RETRIES = 5;
/**
 * After a `closing` for an update (Rift restarts on the computer), how long the client keeps
 * reconnecting at UPDATE_RETRY_MS without counting the rounds against MAX_RETRIES.
 */
export const UPDATE_GRACE_MS = 180_000;
/** The fixed wait between reconnect rounds during UPDATE_GRACE_MS. */
export const UPDATE_RETRY_MS = 3_000;
/**
 * `keys` and `text` messages per second per connection: the gateway's KEYS_RATE (20 within any
 * second; `text` counts against it). Sends past it wait in order, never dropped.
 */
export const INPUT_RATE = 20;
/** The window the client counts INPUT_RATE in: a second plus a margin for network jitter. */
export const INPUT_WINDOW_MS = 1_100;
/**
 * After a re-subscribe a screen that was showing is kept (marked stale) until the gateway sends
 * the next one; when none comes by then, the program has left the screen meanwhile.
 */
export const SCREEN_RESUME_MS = 5_000;
/**
 * An image's `upload` chunks go out one per turn of the event loop (the app stays responsive),
 * and wait while the socket holds more than this unsent.
 */
export const UPLOAD_HIGH_WATER_BYTES = 256 * 1024;
/** How often a waiting upload looks at the socket's unsent bytes again. */
export const UPLOAD_POLL_MS = 50;
/**
 * While an image goes out (until its ack, at most the gateway's 60 s for one upload) the
 * connection counts as alive: the gateway's pong to a ping waits behind the chunks on a slow
 * link, and the silence timeout would otherwise drop the connection mid-upload.
 */
export const UPLOAD_LIVE_MS = 60_000;

/** The subset of the WebSocket API the client uses (the React Native global, or a fake). */
export interface SocketLike {
  binaryType?: string;
  /** Bytes queued but not yet sent (where the platform reports it). */
  readonly bufferedAmount?: number;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  send(data: ArrayBuffer): void;
  close(code?: number, reason?: string): void;
}

export type SocketFactory = (url: string) => SocketLike;

/** `offline`: the device has no network; nothing is tried until it returns. */
export type ConnStatus = "idle" | "connecting" | "open" | "closed" | "failed" | "offline";

/** Why a client stopped for good (status `failed`). */
export type Failure =
  /** The gateway refused this device (revoked, or a spent/expired pairing secret). */
  | "auth"
  /** The relay, or every path of a round, answered with another key than the pinned one. */
  | "key_mismatch"
  /** The gateway speaks another protocol version. */
  | "version"
  /** No address answered (only when retrying is off, i.e. pairing). */
  | "unreachable"
  /** [MAX_RETRIES] reconnect rounds in a row found no answer, or the user cancelled the retrying. */
  | "gave_up"
  /** Rift quit on the computer (its gateway said `closing` with `quit`, then went away). */
  | "host_quit";

/** `trying`: connecting now; `failed`: this path ended the last round without a welcome. */
export type PathState = "idle" | "trying" | "ok" | "failed";

/** What is known of one path (a LAN address, the relay), for the Connection sheet. */
export interface PathStatus {
  path: Endpoint;
  state: PathState;
  /** `failed`: the error that ended the attempt (a socket error, the relay's close reason). */
  error: string | null;
  /** `failed`: how long the attempt waited, in ms. */
  waitedMs: number | null;
  /** `ok`: the round trip of the last ping, in ms (null until one has been answered). */
  latencyMs: number | null;
  /** A socket to it opened in the last attempt (the relay answered, whatever its gateway did). */
  reached: boolean;
}

/** The `error` of an attempt whose handshake timer ran out (the paths' "No answer" notes match on it). */
export const NO_ANSWER = "no answer from the computer";

export interface ConnState {
  status: ConnStatus;
  /** Reconnect rounds since the last successful welcome. */
  attempt: number;
  /** Milliseconds until the next attempt while `closed` and retrying, else null. */
  retryInMs: number | null;
  /** Last error the gateway or the socket reported, for display. */
  error: string | null;
  failure: Failure | null;
  /** The address (or relay URL) of the current or last connection attempt. */
  address: string | null;
  /** The path of the current or last connection attempt: a LAN address or the relay. */
  path: Endpoint | null;
  /** Every path in trying order, with how its last attempt went. */
  paths: PathStatus[];
  /** Epoch ms the current stretch of connecting (or of being cut off) began; null while open. */
  since: number | null;
  /** Epoch ms of the next attempt while `closed` and retrying, else null (`retryInMs` is the delay when it was scheduled). */
  retryAt: number | null;
  /**
   * The gateway said it is about to stop (`closing`): why, about when it is back (seconds, null
   * when it did not say), and when it said so (epoch ms). Cleared by the next `welcome`, by a
   * failure, and when an update's UPDATE_GRACE_MS is over.
   */
  closing: { reason: "quit" | "update"; backInS: number | null; at: number } | null;
}

export interface ClientEvents {
  onState?(state: ConnState): void;
  onGatewayHello?(hello: GatewayHello): void;
  onWelcome?(msg: WelcomeMsg): void;
  onTopology?(msg: TopologyMsg): void;
  /** A snapshot or an in-order update for the subscribed pane, ready to apply. */
  onPane?(msg: SnapshotMsg | UpdateMsg | OlderMsg): void;
  onAck?(msg: AckMsg): void;
  onDone?(msg: DoneMsg): void;
  /** A program on the computer copied this (feature `clipboard`). */
  onClipboard?(text: string): void;
  /** The desktop's theme changed (feature `theme`); the welcome carries the first one. */
  onTheme?(theme: Theme): void;
  onError?(msg: ErrorMsg): void;
  /**
   * The subscribed pane's screen, or its end. A `screen_end` is also reported (with nothing
   * received) when a re-subscribe stopped the screens it was showing and none came back within
   * SCREEN_RESUME_MS (`synthetic` true).
   */
  onScreen?(msg: ScreenMsg | ScreenEndMsg, synthetic?: boolean): void;
  /** A re-subscribe or reconnect: the screen on display is out of date until the next one. */
  onScreenStale?(pane: number): void;
}

/** An `output` request that has not been answered by then fails with `timeout`. */
export const OUTPUT_TIMEOUT_MS = 10_000;

export type OutputResult = { ok: true; msg: OutputMsg } | { ok: false; code: string; message: string };

/** The answer to a chip request (`dirs` / `branches` / `changes` / `diff`), or why there is none. */
export type ChipResult<M> = { ok: true; msg: M } | { ok: false; code: string; message: string };

export interface ClientOptions {
  /** `host:port` addresses, tried in order. */
  addresses: string[];
  /** The gateway's relay (normalized URL), tried after the addresses; null or absent: none. */
  relay?: string | null;
  /** Start at the relay: it was the last path that worked. */
  preferRelay?: boolean;
  /** This device's static key pair. */
  identity: KeyPair;
  /** The gateway static key pinned at pairing. */
  gatewayKey: Uint8Array;
  /** Handshake message 3 payload, without `v`; read at every handshake (the name may change). */
  hello: () => Omit<ClientHello, "v">;
  events: ClientEvents;
  randomBytes: RandomBytes;
  socket?: SocketFactory;
  /** Jitter source for the backoff. */
  random?: () => number;
  /** false: one pass over the paths, then fail with `unreachable` (pairing). Default true. */
  retry?: boolean;
}

/** Backoff before reconnect round `attempt` (0-based): 0.5 s doubling to 10 s, equal jitter. */
export function backoffDelay(attempt: number, random: () => number = Math.random): number {
  const ceiling = Math.min(BACKOFF_CAP_MS, BACKOFF_BASE_MS * 2 ** Math.min(attempt, 30));
  return Math.round(ceiling / 2 + (ceiling / 2) * random());
}

/** `host:port` → WebSocket URL (IPv6 hosts keep their brackets). */
export function wsUrl(address: string): string {
  return `ws://${address}`;
}

const defaultSocket: SocketFactory = (url) => new WebSocket(url) as unknown as SocketLike;

/** The largest input message of its kind: a `paste` may pass the usual limit, as an `upload` chunk does. */
const inputLimit = (msg: ClientMsg) => (msg.t === "paste" ? MAX_UPLOAD_MESSAGE_BYTES : MAX_CLIENT_MESSAGE_BYTES);

function toBytes(data: unknown): Uint8Array | null {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  return null;
}

/** An exact ArrayBuffer for `bytes` (React Native's send takes ArrayBuffer views unreliably). */
function exactBuffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.slice().buffer as ArrayBuffer;
}

export class RiftClient {
  private readonly endpoints: Endpoint[];
  private readonly hasRelay: boolean;
  private readonly identity: KeyPair;
  private readonly gatewayKey: Uint8Array;
  private readonly hello: () => Omit<ClientHello, "v">;
  private readonly events: ClientEvents;
  private readonly randomBytes: RandomBytes;
  private readonly makeSocket: SocketFactory;
  private readonly random: () => number;
  private readonly retry: boolean;

  private ws: SocketLike | null = null;
  private hs: XXHandshake | null = null;
  private sendCipher: CipherState | null = null;
  private recvCipher: CipherState | null = null;
  private reassembler = new Reassembler();
  private stopped = true;
  private authed = false;
  private state: ConnState = {
    status: "idle",
    attempt: 0,
    retryInMs: null,
    error: null,
    failure: null,
    address: null,
    path: null,
    paths: [],
    since: null,
    retryAt: null,
    closing: null,
  };
  /** When the current attempt began (epoch ms), and its error so far (a socket error, the relay's close reason). */
  private attemptAt = 0;
  private attemptError: string | null = null;
  /** The last ping sent: its number and when (epoch ms), for the round trip. */
  private pingSent: { n: number; at: number } | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private idleTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private handshakeTimer: ReturnType<typeof setTimeout> | null = null;
  private pingN = 0;
  private idN = 0;

  /** Path index the next round starts at: the last one that reached `welcome`. */
  private preferred = 0;
  /** The LAN address that last reached `welcome` (or the first one). */
  private lastLan = 0;
  /** Index of the path being tried, and how many this round has tried. */
  private current = 0;
  private triedThisRound = 0;
  /** Paths this round that answered with another key than the pinned one. */
  private mismatchedThisRound = 0;

  private pane: number | null = null;
  /** Last `seq` applied for `pane`; null until a snapshot arrives. */
  private lastSeq: number | null = null;
  /** A re-subscribe for a gap is in flight; further gaps wait for it. */
  private resyncing = false;
  /** The screen mode wanted for the current subscription; sent again after every (re)subscribe. */
  private screenMode: ScreenMode = "off";
  /** A screen of the current subscription is showing (no `screen_end` since). */
  private screenLive = false;
  /** Waits for a screen after a re-subscribe ([SCREEN_RESUME_MS]). */
  private screenResumeTimer: ReturnType<typeof setTimeout> | null = null;
  /** When the last INPUT_RATE `keys` / `text` messages went out (epoch ms). */
  private inputTimes: number[] = [];
  /** `keys` / `text` messages waiting for the rate, in order. */
  private inputQueue: ClientMsg[] = [];
  private inputTimer: ReturnType<typeof setTimeout> | null = null;
  /** Chip requests waiting for their answer, by id: what fits the request, and who waits. */
  private readonly chipAsks = new Map<
    string,
    { fits: (m: ChipAnswerMsg) => boolean; resolve: (r: ChipResult<ChipAnswerMsg>) => void; timer: ReturnType<typeof setTimeout> }
  >();
  /** `output` requests waiting for their answer, by id, with what was asked. */
  private readonly outputs = new Map<
    string,
    {
      resolve: (r: OutputResult) => void;
      timer: ReturnType<typeof setTimeout>;
      pane: number;
      block: number;
      from: number;
      limit: number;
    }
  >();

  constructor(opts: ClientOptions) {
    if (opts.addresses.length === 0) throw new Error("no gateway address");
    this.endpoints = endpointsFor(opts.addresses, opts.relay ?? null);
    this.hasRelay = !!opts.relay;
    if (opts.preferRelay && this.hasRelay) this.preferred = this.endpoints.length - 1;
    this.identity = opts.identity;
    this.gatewayKey = opts.gatewayKey;
    this.hello = opts.hello;
    this.events = opts.events;
    this.randomBytes = opts.randomBytes;
    this.makeSocket = opts.socket ?? defaultSocket;
    this.random = opts.random ?? Math.random;
    this.retry = opts.retry ?? true;
    this.state = { ...this.state, paths: this.freshPaths() };
  }

  private freshPaths(): PathStatus[] {
    return this.endpoints.map((path) => ({ path, state: "idle", error: null, waitedMs: null, latencyMs: null, reached: false }));
  }

  /** `paths` with the current path's status patched (a new array, for [setState]). */
  private patchPath(patch: Partial<PathStatus>): PathStatus[] {
    return this.state.paths.map((p, i) => (i === this.current ? { ...p, ...patch } : p));
  }

  /** The current attempt ended without a welcome: its path failed, with `error` and how long it waited. */
  private failPath(error: string | null): void {
    if (this.state.paths[this.current]?.state !== "trying") return;
    this.setState({
      paths: this.patchPath({
        state: "failed",
        error: error ?? this.attemptError ?? "connection closed",
        waitedMs: Math.max(0, Date.now() - this.attemptAt),
      }),
    });
  }

  /** No attempt is in flight any more: `trying` paths go back to `idle`. */
  private idleTrying(): void {
    if (this.state.paths.some((p) => p.state === "trying"))
      this.setState({ paths: this.state.paths.map((p) => (p.state === "trying" ? { ...p, state: "idle" } : p)) });
  }

  get connState(): ConnState {
    return this.state;
  }

  get subscribedPane(): number | null {
    return this.pane;
  }

  get appliedSeq(): number | null {
    return this.lastSeq;
  }

  /** The LAN address that last reached `welcome` (the first address until one has). */
  get preferredAddress(): string {
    const e = this.endpoints[this.lastLan]!;
    return e.kind === "lan" ? e.address : "";
  }

  /** The path that last reached `welcome`, where the next round starts. */
  get preferredPath(): Endpoint {
    return this.endpoints[this.preferred]!;
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.setState({ attempt: 0, error: null, failure: null });
    this.startRound();
  }

  /** Closes the socket and stops reconnecting. */
  stop(): void {
    this.suspend("closed");
  }

  /**
   * Closes the socket and stops reconnecting until `restart`: `offline` while the device has no
   * network, `closed` otherwise (the app is in the background). A `failed` state is kept.
   */
  suspend(status: "closed" | "offline"): void {
    this.stopped = true;
    this.teardown();
    this.idleTrying();
    if (this.state.status !== "failed") this.setState({ status, retryInMs: null, retryAt: null, since: null });
  }

  /**
   * Stops retrying now (the user cancelled): the client ends as `failed` with `gave_up`, which the
   * user, or the phone coming back online, starts again. Does nothing once open or failed.
   */
  giveUp(): void {
    if (this.state.status === "open" || this.state.status === "failed") return;
    this.fail("gave_up", this.state.error ?? NO_ANSWER);
  }

  /** While waiting to retry, tries again now with the backoff reset (the app came to the foreground). */
  reconnectNow(): void {
    if (this.stopped || this.ws || !this.retryTimer) return;
    clearTimeout(this.retryTimer);
    this.retryTimer = null;
    this.setState({ attempt: 0 });
    this.startRound();
  }

  /**
   * Drops the current socket, open or not, and connects again at once with the backoff reset:
   * the network changed, so the old socket is bound to a path that may be gone. `lanFirst`
   * starts at the first LAN address instead of the last path that worked, so a phone that
   * moves from cellular (relay) to the home Wi-Fi goes back to the direct path. Does nothing
   * once the client has `failed` with auth or version, which need the user; a key mismatch is
   * tried again, since on another network the addresses may reach the right computer, and so is
   * a computer where Rift quit (it may be open again). An update's wait goes on.
   */
  restart(opts: { lanFirst?: boolean } = {}): void {
    const f = this.state.failure;
    if (this.state.status === "failed" && f !== "key_mismatch" && f !== "gave_up" && f !== "host_quit") return;
    this.teardown();
    this.stopped = false;
    this.probing = false;
    this.setState({ attempt: 0, error: null, failure: null, retryInMs: null, retryAt: null, since: null });
    this.startRound(opts.lanFirst ? 0 : this.preferred);
  }

  /**
   * One more round for a client that stopped because nothing answered: the computer may be back while
   * the app stays open. No answer ends it as `gave_up` again, with no run of retries.
   */
  probe(): void {
    const f = this.state.failure;
    if (this.state.status !== "failed" || (f !== "gave_up" && f !== "host_quit")) return;
    this.probing = true;
    this.teardown();
    this.stopped = false;
    this.setState({ attempt: MAX_RETRIES, failure: null, retryInMs: null, retryAt: null, since: null });
    this.startRound();
  }

  /** Follows `pane` (replacing the previous subscription). Resumes when it is the same pane. */
  subscribe(pane: number): void {
    if (this.pane !== pane) {
      this.pane = pane;
      this.lastSeq = null;
      this.screenMode = "off";
      this.screenLive = false;
    }
    this.resyncing = false;
    this.sendSubscribe();
  }

  unsubscribe(): void {
    this.pane = null;
    this.lastSeq = null;
    this.resyncing = false;
    this.screenMode = "off";
    this.screenLive = false;
    this.stopScreenResume();
    if (this.authed) this.send({ t: "unsubscribe" });
  }

  /**
   * Sets what the gateway sends of the subscribed pane's screen; remembered for this
   * subscription and sent again after every re-subscribe and reconnect (which reset it to
   * `off` on the gateway). False when `pane` is not the subscribed pane.
   */
  setScreen(pane: number, mode: ScreenMode): boolean {
    if (pane !== this.pane) return false;
    if (mode === this.screenMode) return true;
    this.screenMode = mode;
    if (this.authed) this.send({ t: "screen", pane, mode });
    return true;
  }

  /**
   * Types printable text into the pane's program; returns the `ack` id, or null when not sent.
   * Paced with `keys` (INPUT_RATE): it may go out a moment later.
   */
  text(pane: number, text: string): string | null {
    if (!this.authed) return null;
    const id = `x${++this.idN}`;
    return this.sendInput({ t: "text", id, pane, text }) ? id : null;
  }

  /** A click at a cell of the pane's screen; returns the `ack` id, or null when not sent. Paced with `keys`. */
  mouse(pane: number, col: number, row: number): string | null {
    if (!this.authed) return null;
    const id = `x${++this.idN}`;
    return this.sendInput({ t: "mouse", id, pane, col, row }) ? id : null;
  }

  /**
   * Pastes text into the pane's program as one paste (feature `paste`); returns the `ack` id, or
   * null when not sent. Paced with `keys`, in order with them.
   */
  paste(pane: number, text: string): string | null {
    if (!this.authed) return null;
    const id = `x${++this.idN}`;
    return this.sendInput({ t: "paste", id, pane, text }) ? id : null;
  }

  /**
   * Sizes the pane's program to `cols` x `rows` (null: release); returns the `ack` id, or null
   * when not connected. Not paced: it is a state, re-sent while held (feature `fit`).
   */
  fit(pane: number, size: { cols: number; rows: number } | null): string | null {
    if (!this.authed) return null;
    const id = `f${++this.idN}`;
    return this.send({ t: "fit", id, pane, cols: size?.cols ?? null, rows: size?.rows ?? null }) ? id : null;
  }

  /**
   * Scrolls the pane as the desktop's wheel would, `lines` up (negative: down; feature `scroll`);
   * returns the `ack` id, or null when not connected. Not paced here: the caller coalesces
   * (scroll.ts), within the gateway's own rate for scrolls.
   */
  scroll(pane: number, lines: number): string | null {
    if (!this.authed) return null;
    const id = `s${++this.idN}`;
    return this.send({ t: "scroll", id, pane, lines }) ? id : null;
  }

  /**
   * Sends an image's `upload` chunks, which `chunks` builds for the request id it is given
   * (upload.ts `uploadChunks`), in order; returns that id, which the `ack` carries, or null when
   * not connected or the first chunk could not be sent. The first goes at once, the rest paced:
   * one per turn of the event loop, waiting while the socket holds more than
   * UPLOAD_HIGH_WATER_BYTES unsent. Until its ack the connection counts as alive
   * (UPLOAD_LIVE_MS). One image at a time: a new one replaces what is left of the last.
   */
  upload(chunks: (id: string) => ClientMsg[]): string | null {
    if (!this.authed || !this.ws) return null;
    const id = `u${++this.idN}`;
    const msgs = chunks(id);
    if (msgs.length === 0) return null;
    this.stopUpload();
    if (!this.send(msgs[0]!, MAX_UPLOAD_MESSAGE_BYTES)) return null;
    this.upload_ = { id, until: Date.now() + UPLOAD_LIVE_MS, buffered: this.ws.bufferedAmount ?? 0 };
    this.uploadTimer = setTimeout(() => this.pumpUpload(msgs, 1), 0);
    return id;
  }

  /** The image going out: its id, until when it keeps the connection alive, the unsent bytes last seen. */
  private upload_: { id: string; until: number; buffered: number } | null = null;
  private uploadTimer: ReturnType<typeof setTimeout> | null = null;

  /** Sends chunk `next` of `msgs` when the socket has room, then the rest on later turns. */
  private pumpUpload(msgs: ClientMsg[], next: number): void {
    this.uploadTimer = null;
    const ws = this.ws;
    const up = this.upload_;
    if (!ws || !this.authed || !up) return;
    const buffered = ws.bufferedAmount ?? 0;
    // bytes left the phone since the last look: the connection carries the image
    if (buffered < up.buffered) this.touch();
    up.buffered = buffered;
    const again = (ms: number) => {
      this.uploadTimer = setTimeout(() => this.pumpUpload(msgs, next), ms);
    };
    if (next >= msgs.length) {
      // all sent: watch it drain (each drop is a sign of life)
      if (buffered > 0) again(UPLOAD_POLL_MS);
      return;
    }
    if (buffered > UPLOAD_HIGH_WATER_BYTES) return again(UPLOAD_POLL_MS);
    // a failed send is a closing socket: onclose follows
    if (!this.send(msgs[next]!, MAX_UPLOAD_MESSAGE_BYTES)) return;
    up.buffered = ws.bufferedAmount ?? 0;
    next += 1;
    again(0);
  }

  private stopUpload(): void {
    if (this.uploadTimer) clearTimeout(this.uploadTimer);
    this.uploadTimer = null;
    this.upload_ = null;
  }

  /**
   * Milliseconds until a `keys` / `text` message would go out (0: at once). Input that can wait
   * (the terminal view's) waits for this, merging what is typed meanwhile.
   */
  get inputDelayMs(): number {
    const wait = this.inputWait();
    return this.inputQueue.length > 0 ? Math.max(1, wait) : wait;
  }

  private inputWait(): number {
    const now = Date.now();
    while (this.inputTimes.length > 0 && now - this.inputTimes[0]! >= INPUT_WINDOW_MS) this.inputTimes.shift();
    return this.inputTimes.length < INPUT_RATE ? 0 : this.inputTimes[0]! + INPUT_WINDOW_MS - now;
  }

  /** Queues a `keys` / `text` message and sends what the rate allows; false when it cannot be sent. */
  private sendInput(msg: ClientMsg): boolean {
    if (!this.sendCipher || utf8Encode(JSON.stringify(msg)).length > inputLimit(msg)) return false;
    this.inputQueue.push(msg);
    this.drainInput();
    return true;
  }

  private drainInput(): void {
    while (this.inputQueue.length > 0) {
      const wait = this.inputWait();
      if (wait > 0) {
        this.inputTimer ??= setTimeout(() => {
          this.inputTimer = null;
          this.drainInput();
        }, wait);
        return;
      }
      this.inputTimes.push(Date.now());
      const msg = this.inputQueue.shift()!;
      this.send(msg, inputLimit(msg));
    }
  }

  /**
   * Asks for `limit` lines of a block's output from line `from` on. Resolves with the answer,
   * or with `no_block` / `no_pane` / `rate` (the gateway's `ack`), `timeout`, or
   * `disconnected` (not connected, or the connection dropped before the answer).
   */
  output(pane: number, block: number, from: number, limit: number): Promise<OutputResult> {
    const id = this.authed ? `o${++this.idN}` : null;
    if (id === null || !this.send({ t: "output", id, pane, block, from, limit })) {
      return Promise.resolve({ ok: false, code: "disconnected", message: "not connected" });
    }
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.outputs.delete(id);
        resolve({ ok: false, code: "timeout", message: "no answer" });
      }, OUTPUT_TIMEOUT_MS);
      this.outputs.set(id, { resolve, timer, pane, block, from, limit });
    });
  }

  /**
   * A prompt chip's menu of `pane` (feature `chips`): resolves with the answer, or with the
   * gateway's refusal (`no_pane`, `rate`), `timeout`, `disconnected` or `bad_data` (an answer
   * of another kind or pane).
   */
  chipMenu<T extends "dirs" | "branches" | "changes">(
    pane: number,
    t: T,
  ): Promise<ChipResult<Extract<ChipAnswerMsg, { t: T }>>> {
    return this.askChip({ t, id: "", pane }, (m) => m.t === t && m.pane === pane) as Promise<
      ChipResult<Extract<ChipAnswerMsg, { t: T }>>
    >;
  }

  /**
   * What a pull request, interpreter, Docker or Time on tab chip opens (feature `prompt_chips`):
   * resolves like `chipMenu`.
   */
  chipDetail<K extends ChipDetailKind>(pane: number, kind: K): Promise<ChipResult<Extract<ChipMsg, { kind: K }>>> {
    return this.askChip(
      { t: "chip", id: "", pane, kind },
      (m) => m.t === "chip" && m.pane === pane && m.kind === kind,
    ) as Promise<ChipResult<Extract<ChipMsg, { kind: K }>>>;
  }

  /** Restarts the pane's tab clock (feature `prompt_chips`); returns the `ack` id, or null when not connected. */
  resetElapsed(pane: number): string | null {
    if (!this.authed) return null;
    const id = `e${++this.idN}`;
    return this.send({ t: "reset_elapsed", id, pane }) ? id : null;
  }

  /** One changed file's diff (`no_block` when it is no longer a change). */
  diff(pane: number, path: string, staged: boolean): Promise<ChipResult<DiffMsg>> {
    return this.askChip(
      { t: "diff", id: "", pane, path, staged },
      (m) => m.t === "diff" && m.pane === pane && m.path === path && m.staged === staged,
    ) as Promise<ChipResult<DiffMsg>>;
  }

  /**
   * Completions and history for `line` with the cursor at JS index `cursor` (feature
   * `complete`); the wire cursor is the UTF-8 byte offset of that index.
   */
  complete(pane: number, line: string, cursor: number = line.length): Promise<ChipResult<CompleteMsg>> {
    const at = utf8Length(line.slice(0, Math.max(0, Math.min(cursor, line.length))));
    return this.askChip(
      { t: "complete", id: "", pane, line, cursor: at },
      (m) => m.t === "complete" && m.pane === pane,
    ) as Promise<ChipResult<CompleteMsg>>;
  }

  private askChip(
    req: ChipRequestMsg | ChipDetailRequestMsg | DiffRequestMsg | CompleteRequestMsg,
    fits: (m: ChipAnswerMsg) => boolean,
  ): Promise<ChipResult<ChipAnswerMsg>> {
    const id = this.authed ? `c${++this.idN}` : null;
    if (id === null || !this.send({ ...req, id })) {
      return Promise.resolve({ ok: false, code: "disconnected", message: "not connected" });
    }
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.chipAsks.delete(id);
        resolve({ ok: false, code: "timeout", message: "no answer" });
      }, OUTPUT_TIMEOUT_MS);
      this.chipAsks.set(id, { fits, resolve, timer });
    });
  }

  private settleChip(id: string, r: ChipResult<ChipAnswerMsg> | ChipAnswerMsg): boolean {
    const p = this.chipAsks.get(id);
    if (!p) return false;
    this.chipAsks.delete(id);
    clearTimeout(p.timer);
    if ("t" in r)
      p.resolve(p.fits(r) ? { ok: true, msg: r } : { ok: false, code: "bad_data", message: "The computer's answer did not fit the request." });
    else p.resolve(r);
    return true;
  }

  private settleOutput(id: string, r: OutputResult): boolean {
    const p = this.outputs.get(id);
    if (!p) return false;
    this.outputs.delete(id);
    clearTimeout(p.timer);
    p.resolve(r);
    return true;
  }

  /** Asks for up to 30 blocks older than block id `before` in the subscribed pane. */
  requestOlder(before: number): void {
    if (this.pane !== null && this.authed) this.send({ t: "older", pane: this.pane, before });
  }

  /** Submits a command line; returns the `ack` id, or null when not connected or not sendable. */
  run(pane: number, command: string): string | null {
    if (!this.authed) return null;
    const id = `r${++this.idN}`;
    return this.send({ t: "run", id, pane, command }) ? id : null;
  }

  /**
   * Sends keys to a pane; returns the `ack` id, or null when not connected or not sendable.
   * Paced (INPUT_RATE): it may go out a moment later.
   */
  keys(pane: number, keys: string[]): string | null {
    if (!this.authed) return null;
    const id = `k${++this.idN}`;
    return this.sendInput({ t: "keys", id, pane, keys }) ? id : null;
  }

  /** Starts a shell while no Rift window is open (feature `start`); the `ack` id, or null when not connected. */
  startShell(): string | null {
    if (!this.authed) return null;
    const id = `n${++this.idN}`;
    return this.send({ t: "start", id }) ? id : null;
  }

  /** Opens a new tab or split next to a pane (feature `open`); returns the `ack` id, or null when not connected. */
  openShell(pane: number, kind: OpenKind): string | null {
    if (!this.authed) return null;
    const id = `n${++this.idN}`;
    return this.send({ t: "open", id, pane, kind }) ? id : null;
  }

  /** Closes a pane or its whole tab (feature `close`); returns the `ack` id, or null when not connected. */
  closeShell(pane: number, kind: CloseKind): string | null {
    if (!this.authed) return null;
    const id = `n${++this.idN}`;
    return this.send({ t: "close", id, pane, kind }) ? id : null;
  }

  /** Moves a pane's tab, or the pane, to position `to` (feature `move`); returns the `ack` id, or null when not connected. */
  moveShell(pane: number, kind: MoveKind, to: number): string | null {
    if (!this.authed) return null;
    const id = `n${++this.idN}`;
    return this.send({ t: "move", id, pane, kind, to }) ? id : null;
  }

  /**
   * Registers (or with neither a token nor an ntfy URL stops) alerts while the app is closed; false
   * when not connected. `ntfy` undefined leaves the field out (a gateway without `push_ntfy`).
   */
  setPush(reg: {
    token: string | null;
    ntfy?: string | null;
    longMs: number | null;
    showCommand: boolean;
    failuresOnly: boolean;
  }): boolean {
    return (
      this.authed &&
      this.send({
        t: "push",
        token: reg.token,
        ...(reg.ntfy !== undefined ? { ntfy: reg.ntfy } : {}),
        long_ms: reg.longMs,
        show_command: reg.showCommand,
        failures_only: reg.failuresOnly,
      })
    );
  }

  /** Tells the gateway whether the app is on screen (`push_ntfy` gateways only); false when not connected. */
  setVisible(on: boolean): boolean {
    return this.authed && this.send({ t: "visible", on });
  }

  /** Asks for an alert when the next command in `pane` finishes; false when not connected. */
  notify(pane: number, off = false): boolean {
    return this.authed && this.send(off ? { t: "notify", pane, off: true } : { t: "notify", pane });
  }

  /** Asks the gateway to remove this device; it then closes the connection. */
  forget(): boolean {
    return this.authed && this.send({ t: "forget" });
  }

  get isOpen(): boolean {
    return this.authed;
  }

  // --- connection lifecycle --------------------------------------------------------------

  private startRound(from = this.preferred): void {
    this.triedThisRound = 0;
    this.mismatchedThisRound = 0;
    this.current = from;
    this.state = { ...this.state, paths: this.freshPaths() };
    this.open();
  }

  private open(): void {
    const path = this.endpoints[this.current]!;
    const address = path.kind === "lan" ? path.address : path.url;
    this.triedThisRound++;
    this.attemptAt = Date.now();
    this.attemptError = null;
    this.setState({
      status: "connecting",
      retryInMs: null,
      retryAt: null,
      address,
      path,
      since: this.state.since ?? this.attemptAt,
      paths: this.patchPath({ state: "trying", error: null, waitedMs: null, latencyMs: null, reached: false }),
    });
    const deadline =
      path.kind === "relay"
        ? RELAY_HANDSHAKE_TIMEOUT_MS
        : this.hasRelay
          ? LAN_HANDSHAKE_TIMEOUT_MS
          : HANDSHAKE_TIMEOUT_MS;
    let ws: SocketLike;
    try {
      ws = this.makeSocket(path.kind === "lan" ? wsUrl(address) : relayConnectUrl(path.url, this.gatewayKey));
      ws.binaryType = "arraybuffer";
    } catch (e) {
      this.setState({ error: e instanceof Error ? e.message : String(e) });
      this.attemptFailed();
      return;
    }
    this.ws = ws;
    this.reassembler = new Reassembler();
    this.handshakeTimer = setTimeout(() => {
      if (this.ws === ws && !this.sendCipher) this.dropAttempt(NO_ANSWER);
    }, deadline);
    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.setState({ paths: this.patchPath({ reached: true }) });
      try {
        const hs = new XXHandshake({
          initiator: true,
          prologue: utf8Encode(PROLOGUE),
          s: this.identity,
          pinnedRemoteStatic: this.gatewayKey,
          random: this.randomBytes,
        });
        this.hs = hs;
        this.sendRaw(hs.writeMessage(new Uint8Array(0)));
      } catch (e) {
        this.dropAttempt(e instanceof Error ? e.message : String(e));
      }
    };
    ws.onmessage = (ev) => {
      if (this.ws !== ws) return;
      const bytes = toBytes(ev.data);
      if (!bytes) {
        // Text frames are a protocol violation (binary only).
        this.dropAttempt("unexpected text frame");
        return;
      }
      if (this.sendCipher) this.touch();
      if (!this.sendCipher) this.onHandshakeMessage(bytes);
      else this.onTransportMessage(bytes);
    };
    ws.onerror = (ev) => {
      if (this.ws !== ws) return;
      const message = (ev as { message?: unknown } | null)?.message;
      this.attemptError ??= typeof message === "string" && message !== "" ? message : "connection error";
      this.setState({ error: this.state.error ?? "connection error" });
    };
    ws.onclose = (ev) => {
      if (this.ws !== ws) return;
      // the relay says why it closed a phone whose gateway did not answer
      const reason = (ev as { reason?: unknown } | null)?.reason;
      // a close reason says more than the error event's bare "connection error"
      if (typeof reason === "string" && reason !== "" && (this.attemptError === null || this.attemptError === "connection error")) this.attemptError = reason;
      this.dropAttempt(null);
    };
  }

  /** Message 2 from the gateway: check the pinned key and its hello, answer with message 3. */
  private onHandshakeMessage(bytes: Uint8Array): void {
    const hs = this.hs;
    if (!hs || hs.isMyTurn) {
      this.dropAttempt("unexpected handshake message");
      return;
    }
    let gatewayHello: unknown;
    try {
      const payload = hs.readMessage(bytes);
      gatewayHello = JSON.parse(utf8Decode(payload));
    } catch (e) {
      if (e instanceof NoiseKeyMismatchError) {
        this.keyMismatch(e.message);
        return;
      }
      this.dropAttempt(e instanceof NoiseError ? `handshake failed: ${e.message}` : "bad gateway hello");
      return;
    }
    if (!isGatewayHello(gatewayHello)) {
      this.dropAttempt("bad gateway hello");
      return;
    }
    if (gatewayHello.v !== PROTOCOL_VERSION) {
      this.fail("version", `the computer speaks protocol version ${gatewayHello.v}, this app ${PROTOCOL_VERSION}`);
      return;
    }
    this.events.onGatewayHello?.(gatewayHello);
    try {
      const hello: ClientHello = { v: PROTOCOL_VERSION, ...this.hello() };
      this.sendRaw(hs.writeMessage(utf8Encode(JSON.stringify(hello))));
      const t = hs.transport;
      this.sendCipher = t.send;
      this.recvCipher = t.recv;
    } catch (e) {
      this.dropAttempt(e instanceof Error ? e.message : String(e));
      return;
    }
    this.hs = null;
    if (this.handshakeTimer) clearTimeout(this.handshakeTimer);
    this.handshakeTimer = null;
    this.pingTimer = setInterval(() => this.ping(), PING_INTERVAL_MS);
    this.touch();
  }

  /** A keep-alive ping, timed: its pong gives the path's round trip. */
  private ping(): void {
    const n = ++this.pingN;
    this.pingSent = { n, at: Date.now() };
    this.send({ t: "ping", n });
  }

  private onTransportMessage(bytes: Uint8Array): void {
    let message: Uint8Array | null;
    try {
      message = this.reassembler.push(this.recvCipher!.decrypt(bytes));
    } catch (e) {
      // A forged or corrupted frame, or a malformed chunk stream: this channel is unusable.
      this.dropAttempt(e instanceof NoiseError || e instanceof FramingError ? e.message : "bad frame");
      return;
    }
    if (!message) return;
    const msg = parseServerFrame(utf8Decode(message));
    if (msg) this.handle(msg);
  }

  private handle(msg: ServerMsg): void {
    switch (msg.t) {
      case "welcome":
        this.authed = true;
        this.preferred = this.current;
        if (this.endpoints[this.current]!.kind === "lan") this.lastLan = this.current;
        this.setState({
          status: "open",
          attempt: 0,
          error: null,
          failure: null,
          since: null,
          retryAt: null,
          closing: null,
          paths: this.state.paths.map((p, i) =>
            i === this.current
              ? { ...p, state: "ok", error: null, waitedMs: null, latencyMs: null, reached: true }
              : { ...p, state: "idle", error: null, waitedMs: null, latencyMs: null, reached: false },
          ),
        });
        // the pane first: what is on screen comes back before anything else is sent
        if (this.pane !== null) this.sendSubscribe();
        this.events.onWelcome?.(msg);
        // the first round trip, so the Connection sheet has a latency before the 15 s keep-alive
        this.ping();
        return;
      case "topology":
        this.events.onTopology?.(msg);
        return;
      case "snapshot":
        if (msg.pane !== this.pane) return;
        this.lastSeq = msg.seq;
        this.resyncing = false;
        this.events.onPane?.(msg);
        return;
      case "update":
        this.onUpdate(msg);
        return;
      case "older":
        if (msg.pane === this.pane) this.events.onPane?.(msg);
        return;
      case "ack":
        // an image's answer (its last chunk, or a refusal part-way): nothing more of it goes out
        if (this.upload_?.id === msg.id) this.stopUpload();
        // an `output` request is refused with an `ack` carrying its id
        if (!msg.ok && this.settleOutput(msg.id, { ok: false, code: msg.error ?? "error", message: msg.message ?? "" }))
          return;
        // and so is a chip request
        if (!msg.ok && this.settleChip(msg.id, { ok: false, code: msg.error ?? "error", message: msg.message ?? "" })) return;
        this.events.onAck?.(msg);
        return;
      case "output": {
        // an answer for another pane or block, or starting past the range asked for, is bad data
        const p = this.outputs.get(msg.id);
        if (!p) return;
        const fits = msg.pane === p.pane && msg.block === p.block && msg.from <= p.from + p.limit;
        this.settleOutput(
          msg.id,
          fits ? { ok: true, msg } : { ok: false, code: "bad_data", message: "The computer's answer did not fit the request." },
        );
        return;
      }
      case "dirs":
      case "branches":
      case "changes":
      case "chip":
      case "diff":
      case "complete":
        this.settleChip(msg.id, msg);
        return;
      case "screen":
        if (msg.pane !== this.pane) return;
        this.screenLive = true;
        this.stopScreenResume();
        this.events.onScreen?.(msg);
        return;
      case "screen_end":
        if (msg.pane !== this.pane) return;
        this.screenLive = false;
        this.stopScreenResume();
        this.events.onScreen?.(msg);
        return;
      case "done":
        this.events.onDone?.(msg);
        return;
      case "clipboard":
        this.events.onClipboard?.(msg.text);
        return;
      case "theme":
        this.events.onTheme?.(msg.theme);
        return;
      case "closing":
        this.setState({ closing: { reason: msg.reason, backInS: msg.back_in_s ?? null, at: Date.now() } });
        return;
      case "pong": {
        const sent = this.pingSent;
        if (sent?.n !== msg.n) return;
        this.pingSent = null;
        const ms = Math.max(1, Math.round(Date.now() - sent.at));
        if (this.state.paths[this.current]?.latencyMs !== ms) this.setState({ paths: this.patchPath({ latencyMs: ms }) });
        return;
      }
      case "error":
        this.events.onError?.(msg);
        if (msg.code === "auth" || msg.code === "version") {
          this.fail(msg.code === "auth" ? "auth" : "version", msg.message);
        } else {
          this.setState({ error: msg.message });
        }
        return;
    }
  }

  private onUpdate(msg: UpdateMsg): void {
    if (msg.pane !== this.pane || this.lastSeq === null) return;
    if (msg.seq <= this.lastSeq) return; // replayed or duplicate
    if (msg.seq !== this.lastSeq + 1) {
      if (!this.resyncing) {
        this.resyncing = true;
        this.sendSubscribe();
      }
      return;
    }
    this.lastSeq = msg.seq;
    this.resyncing = false;
    this.events.onPane?.(msg);
  }

  private sendSubscribe(): void {
    if (this.pane === null || !this.authed) return;
    const pane = this.pane;
    this.send({ t: "subscribe", pane, after: this.lastSeq });
    // A subscribe stops the screens silently and resets the mode to `off`: ask again. The
    // screen on display stays, out of date, until the next one (a full one follows while the
    // program is still up); when none comes, it has ended meanwhile.
    if (this.screenLive) {
      this.events.onScreenStale?.(pane);
      this.stopScreenResume();
      if (this.screenMode === "off") this.endScreen(pane);
      else this.screenResumeTimer = setTimeout(() => this.endScreen(pane), SCREEN_RESUME_MS);
    }
    if (this.screenMode !== "off") this.send({ t: "screen", pane, mode: this.screenMode });
  }

  private endScreen(pane: number): void {
    this.screenResumeTimer = null;
    if (!this.screenLive || pane !== this.pane) return;
    this.screenLive = false;
    this.events.onScreen?.({ t: "screen_end", pane }, true);
  }

  private stopScreenResume(): void {
    if (this.screenResumeTimer) clearTimeout(this.screenResumeTimer);
    this.screenResumeTimer = null;
  }

  /**
   * Encrypts and sends one protocol message. False when there is no channel or it is larger than
   * `maxBytes` (MAX_CLIENT_MESSAGE_BYTES; an `upload` chunk may be MAX_UPLOAD_MESSAGE_BYTES).
   */
  private send(msg: ClientMsg, maxBytes = MAX_CLIENT_MESSAGE_BYTES): boolean {
    if (!this.sendCipher) return false;
    const bytes = utf8Encode(JSON.stringify(msg));
    if (bytes.length > maxBytes) return false;
    try {
      for (const chunk of frameMessage(bytes)) this.sendRaw(this.sendCipher.encrypt(chunk));
      return true;
    } catch {
      // A send on a closing socket fails; onclose reconnects.
      return false;
    }
  }

  private sendRaw(bytes: Uint8Array): void {
    this.ws?.send(exactBuffer(bytes));
  }

  private touch(ms: number = IDLE_TIMEOUT_MS): void {
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.idleTimer = setTimeout(() => {
      // an image still going out: the gateway's answers wait behind it
      const left = (this.upload_?.until ?? 0) - Date.now();
      if (left > 0) return this.touch(left);
      // A silent gateway: drop this socket and reconnect.
      this.dropAttempt("gateway stopped responding");
    }, ms);
  }

  /** The current socket is done (closed, broken or timed out): try the next address or back off. */
  private dropAttempt(error: string | null): void {
    const wasOpen = this.authed;
    this.teardown();
    if (error) this.setState({ error });
    if (wasOpen) {
      this.setState({
        since: Date.now(),
        paths: this.patchPath({ state: "failed", error: error ?? this.attemptError ?? "connection closed", waitedMs: null, latencyMs: null }),
      });
    } else if (this.stopped) this.idleTrying();
    else this.failPath(error);
    if (this.stopped) {
      if (this.state.status !== "failed") this.setState({ status: "closed", retryInMs: null, retryAt: null });
      return;
    }
    if (wasOpen && this.state.closing?.reason === "quit") {
      // Rift quit on the computer: nothing answers until it is opened again
      this.fail("host_quit", "Rift quit on the computer");
      return;
    }
    if (wasOpen) {
      // A working connection broke: a fresh round, from the address that worked.
      this.scheduleReconnect();
      return;
    }
    this.attemptFailed();
  }

  /**
   * Another key answered (message 3, with the hello, was never sent). A LAN address can belong
   * to some other machine (another network reusing the address, or an impostor on it): drop
   * that attempt and go on to the next path. The relay routes by the pinned key, so a
   * mismatch there, or on every path of the round, fails for good.
   */
  private keyMismatch(error: string): void {
    this.mismatchedThisRound++;
    if (this.endpoints[this.current]!.kind === "relay" || this.mismatchedThisRound >= this.endpoints.length) {
      this.fail("key_mismatch", error);
      return;
    }
    this.dropAttempt(error);
  }

  private attemptFailed(): void {
    if (this.stopped) return;
    if (this.triedThisRound < this.endpoints.length) {
      this.current = (this.current + 1) % this.endpoints.length;
      this.open();
      return;
    }
    if (!this.retry) {
      this.fail("unreachable", this.state.error ?? "no address answered");
      return;
    }
    this.scheduleReconnect();
  }

  private fail(failure: Failure, error: string): void {
    this.stopped = true;
    this.teardown();
    this.failPath(error);
    this.setState({ status: "failed", failure, error, retryInMs: null, retryAt: null, closing: null });
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    const closing = this.state.closing;
    if (closing?.reason === "update") {
      if (Date.now() - closing.at < UPDATE_GRACE_MS) {
        // Rift is restarting on the computer: a steady cadence, not counted as a retry
        this.setState({ status: "closed", retryInMs: UPDATE_RETRY_MS, retryAt: Date.now() + UPDATE_RETRY_MS });
        this.retryTimer = setTimeout(() => {
          this.retryTimer = null;
          this.startRound();
        }, UPDATE_RETRY_MS);
        return;
      }
      // it has not come back: the usual backoff and retry limit from here
      this.setState({ closing: null, attempt: 0 });
    }
    if (this.state.attempt >= MAX_RETRIES) {
      this.fail("gave_up", this.state.error ?? NO_ANSWER);
      return;
    }
    const delay = backoffDelay(this.state.attempt, this.random);
    this.setState({ status: "closed", retryInMs: delay, retryAt: Date.now() + delay });
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      this.setState({ attempt: this.state.attempt + 1 });
      this.startRound();
    }, delay);
  }

  /** Drops the current socket and its timers; handlers of the old socket become no-ops. */
  private teardown(): void {
    const ws = this.ws;
    this.ws = null;
    this.hs = null;
    this.sendCipher = this.recvCipher = null;
    this.authed = false;
    this.resyncing = false;
    if (this.pingTimer) clearInterval(this.pingTimer);
    if (this.idleTimer) clearTimeout(this.idleTimer);
    if (this.retryTimer) clearTimeout(this.retryTimer);
    if (this.handshakeTimer) clearTimeout(this.handshakeTimer);
    if (this.inputTimer) clearTimeout(this.inputTimer);
    this.pingTimer = this.idleTimer = this.retryTimer = this.handshakeTimer = this.inputTimer = null;
    // input waiting for the rate belonged to this connection: its requests fail as disconnected
    this.inputQueue = [];
    this.inputTimes = [];
    // what is left of an image goes nowhere now
    this.stopUpload();
    this.stopScreenResume();
    // answers to requests on this socket never come
    for (const id of [...this.outputs.keys()])
      this.settleOutput(id, { ok: false, code: "disconnected", message: "the connection dropped" });
    for (const id of [...this.chipAsks.keys()])
      this.settleChip(id, { ok: false, code: "disconnected", message: "the connection dropped" });
    if (ws) {
      ws.onopen = ws.onmessage = ws.onerror = ws.onclose = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
  }

  /** A `probe` is on its way: the views hear only how it ends. */
  private probing = false;

  private setState(patch: Partial<ConnState>): void {
    this.state = { ...this.state, ...patch };
    if (this.probing) {
      // the card keeps saying it stopped: its rows must not jump under a finger every 30 s
      if (patch.status !== "open" && patch.status !== "failed") return;
      this.probing = false;
    }
    this.events.onState?.(this.state);
  }
}
