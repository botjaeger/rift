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
  PROLOGUE,
  PROTOCOL_VERSION,
  isGatewayHello,
  parseServerFrame,
  type AckMsg,
  type ClientHello,
  type ClientMsg,
  type DoneMsg,
  type ErrorMsg,
  type GatewayHello,
  type OlderMsg,
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

/** The subset of the WebSocket API the client uses (the React Native global, or a fake). */
export interface SocketLike {
  binaryType?: string;
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
  | "unreachable";

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
  onError?(msg: ErrorMsg): void;
}

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
  };
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
    if (this.state.status !== "failed") this.setState({ status, retryInMs: null });
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
   * tried again, since on another network the addresses may reach the right computer.
   */
  restart(opts: { lanFirst?: boolean } = {}): void {
    if (this.state.status === "failed" && this.state.failure !== "key_mismatch") return;
    this.teardown();
    this.stopped = false;
    this.setState({ attempt: 0, error: null, failure: null, retryInMs: null });
    this.startRound(opts.lanFirst ? 0 : this.preferred);
  }

  /** Follows `pane` (replacing the previous subscription). Resumes when it is the same pane. */
  subscribe(pane: number): void {
    if (this.pane !== pane) {
      this.pane = pane;
      this.lastSeq = null;
    }
    this.resyncing = false;
    this.sendSubscribe();
  }

  unsubscribe(): void {
    this.pane = null;
    this.lastSeq = null;
    this.resyncing = false;
    if (this.authed) this.send({ t: "unsubscribe" });
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

  /** Sends keys to a pane; returns the `ack` id, or null when not connected or not sendable. */
  keys(pane: number, keys: string[]): string | null {
    if (!this.authed) return null;
    const id = `k${++this.idN}`;
    return this.send({ t: "keys", id, pane, keys }) ? id : null;
  }

  /** Registers (or with a null token stops) alerts while the app is closed; false when not connected. */
  setPush(reg: { token: string | null; longMs: number | null; showCommand: boolean }): boolean {
    return (
      this.authed &&
      this.send({
        t: "push",
        token: reg.token,
        long_ms: reg.longMs,
        show_command: reg.showCommand,
      })
    );
  }

  /** Asks for an alert when the next command in `pane` finishes; false when not connected. */
  notify(pane: number): boolean {
    return this.authed && this.send({ t: "notify", pane });
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
    this.open();
  }

  private open(): void {
    const path = this.endpoints[this.current]!;
    const address = path.kind === "lan" ? path.address : path.url;
    this.triedThisRound++;
    this.setState({ status: "connecting", retryInMs: null, address, path });
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
      if (this.ws === ws && !this.sendCipher) this.dropAttempt("no answer from the computer");
    }, deadline);
    ws.onopen = () => {
      if (this.ws !== ws) return;
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
    ws.onerror = () => {
      if (this.ws !== ws) return;
      this.setState({ error: this.state.error ?? "connection error" });
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
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
    this.pingTimer = setInterval(() => this.send({ t: "ping", n: ++this.pingN }), PING_INTERVAL_MS);
    this.touch();
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
        });
        // the pane first: what is on screen comes back before anything else is sent
        if (this.pane !== null) this.sendSubscribe();
        this.events.onWelcome?.(msg);
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
        this.events.onAck?.(msg);
        return;
      case "done":
        this.events.onDone?.(msg);
        return;
      case "pong":
        return;
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
    this.send({ t: "subscribe", pane: this.pane, after: this.lastSeq });
  }

  /** Encrypts and sends one protocol message. False when there is no channel or it is too large. */
  private send(msg: ClientMsg): boolean {
    if (!this.sendCipher) return false;
    const bytes = utf8Encode(JSON.stringify(msg));
    if (bytes.length > MAX_CLIENT_MESSAGE_BYTES) return false;
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

  private touch(): void {
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.idleTimer = setTimeout(() => {
      // A silent gateway: drop this socket and reconnect.
      this.dropAttempt("gateway stopped responding");
    }, IDLE_TIMEOUT_MS);
  }

  /** The current socket is done (closed, broken or timed out): try the next address or back off. */
  private dropAttempt(error: string | null): void {
    const wasOpen = this.authed;
    this.teardown();
    if (error) this.setState({ error });
    if (this.stopped) {
      if (this.state.status !== "failed") this.setState({ status: "closed", retryInMs: null });
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
    this.setState({ status: "failed", failure, error, retryInMs: null });
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    const delay = backoffDelay(this.state.attempt, this.random);
    this.setState({ status: "closed", retryInMs: delay });
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
    this.pingTimer = this.idleTimer = this.retryTimer = this.handshakeTimer = null;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onerror = ws.onclose = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
  }

  private setState(patch: Partial<ConnState>): void {
    this.state = { ...this.state, ...patch };
    this.events.onState?.(this.state);
  }
}
