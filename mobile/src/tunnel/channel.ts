/**
 * The tunnel's own connection to the computer (feature `tunnel`): the same path, keys and
 * Noise handshake as the session (src/protocol/client.ts), its hello adding `role: "tunnel"`,
 * then binary frames (./frames) instead of JSON. One per open page; it does not reconnect:
 * the page's next request opens a new one.
 */

import { utf8Decode, utf8Encode } from "../noise/bytes";
import { XXHandshake, type CipherState } from "../noise/noise";
import { PING_INTERVAL_MS, IDLE_TIMEOUT_MS, HANDSHAKE_TIMEOUT_MS, wsUrl, type SocketLike, type TunnelTarget } from "../protocol/client";
import { Reassembler, frameMessage } from "../protocol/framing";
import { relayConnectUrl } from "../protocol/relay";
import { PROLOGUE, PROTOCOL_VERSION, isGatewayHello, type ClientHello } from "../protocol/types";
import { decodeFrame, encodeFrame, MAX_DATA, type Frame } from "./frames";

export interface ChannelEvents {
  onOpen(): void;
  onFrame(f: Frame): void;
  /** The connection ended (`why` for display: the computer's error, a socket error, idle). */
  onClose(why: string): void;
}

function toBytes(data: unknown): Uint8Array | null {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  return null;
}

export class TunnelChannel {
  private ws: SocketLike | null;
  private hs: XXHandshake | null = null;
  private send_: CipherState | null = null;
  private recv_: CipherState | null = null;
  private readonly reassembler = new Reassembler(MAX_DATA + 1024);
  private timers: ReturnType<typeof setTimeout>[] = [];
  private pinger: ReturnType<typeof setInterval> | null = null;
  private heardAt = Date.now();
  private closed = false;

  constructor(
    private readonly target: TunnelTarget,
    private readonly events: ChannelEvents,
  ) {
    const p = target.path;
    const ws = target.socket(p.kind === "lan" ? wsUrl(p.address) : relayConnectUrl(p.url, target.gatewayKey));
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    this.timers.push(setTimeout(() => !this.send_ && this.close("no answer from the computer"), HANDSHAKE_TIMEOUT_MS));
    ws.onopen = () => {
      try {
        this.hs = new XXHandshake({
          initiator: true,
          prologue: utf8Encode(PROLOGUE),
          s: target.identity,
          pinnedRemoteStatic: target.gatewayKey,
          random: target.randomBytes,
        });
        this.raw(this.hs.writeMessage(new Uint8Array(0)));
      } catch (e) {
        this.close(e instanceof Error ? e.message : String(e));
      }
    };
    ws.onmessage = (ev) => {
      const bytes = toBytes(ev.data);
      if (!bytes) return this.close("unexpected text frame");
      this.heardAt = Date.now();
      if (!this.send_) this.handshake(bytes);
      else this.message(bytes);
    };
    ws.onerror = () => this.close("connection error");
    ws.onclose = (ev) => {
      const reason = (ev as { reason?: unknown } | null)?.reason;
      this.close(typeof reason === "string" && reason !== "" ? reason : "connection closed");
    };
  }

  get open(): boolean {
    return this.send_ !== null && !this.closed;
  }

  send(f: Frame): boolean {
    if (!this.send_ || this.closed) return false;
    try {
      for (const chunk of frameMessage(encodeFrame(f))) this.raw(this.send_.encrypt(chunk));
      return true;
    } catch {
      return false;
    }
  }

  close(why: string): void {
    if (this.closed) return;
    this.closed = true;
    for (const t of this.timers) clearTimeout(t);
    if (this.pinger) clearInterval(this.pinger);
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onerror = ws.onclose = null;
      try {
        ws.close();
      } catch {
        // already closing
      }
    }
    this.events.onClose(why);
  }

  private raw(bytes: Uint8Array): void {
    this.ws?.send(bytes.slice().buffer as ArrayBuffer);
  }

  private handshake(bytes: Uint8Array): void {
    const hs = this.hs;
    if (!hs) return this.close("unexpected handshake message");
    try {
      const gw: unknown = JSON.parse(utf8Decode(hs.readMessage(bytes)));
      if (!isGatewayHello(gw) || gw.v !== PROTOCOL_VERSION) return this.close("bad gateway hello");
      const hello: ClientHello = { v: PROTOCOL_VERSION, ...this.target.hello(), role: "tunnel" };
      delete hello.pair;
      this.raw(hs.writeMessage(utf8Encode(JSON.stringify(hello))));
      const t = hs.transport;
      this.send_ = t.send;
      this.recv_ = t.recv;
      this.hs = null;
    } catch (e) {
      return this.close(e instanceof Error ? e.message : "handshake failed");
    }
    this.pinger = setInterval(() => {
      if (Date.now() - this.heardAt > IDLE_TIMEOUT_MS) this.close("the computer stopped responding");
      else this.send({ op: "ping" });
    }, PING_INTERVAL_MS);
    this.events.onOpen();
  }

  private message(bytes: Uint8Array): void {
    let msg: Uint8Array | null;
    try {
      msg = this.reassembler.push(this.recv_!.decrypt(bytes));
    } catch (e) {
      return this.close(e instanceof Error ? e.message : "bad frame");
    }
    if (!msg) return;
    // the computer refusing the tunnel answers in JSON (`error`), then closes
    if (msg[0] === 0x7b) {
      try {
        const v = JSON.parse(utf8Decode(msg)) as { t?: string; code?: string; message?: string };
        if (v.t === "error") return this.close(v.code === "forbidden" ? "forbidden" : (v.message ?? "refused"));
      } catch {
        // not JSON: a malformed frame
      }
    }
    let f: Frame;
    try {
      f = decodeFrame(msg);
    } catch (e) {
      return this.close(e instanceof Error ? e.message : "bad frame");
    }
    this.events.onFrame(f);
  }
}
