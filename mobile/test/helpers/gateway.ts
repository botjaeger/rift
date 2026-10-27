// A fake Rift gateway behind a fake WebSocket: the Noise responder side of the handshake (with
// this app's own Noise code, checked independently against the vectors in noise.test.ts), the
// chunk framing, and JSON. Its replies wait in a queue until the test calls deliver().

import { randomBytes as nodeRandomBytes } from "node:crypto";
import { utf8Decode, utf8Encode } from "../../src/noise/bytes";
import { XXHandshake, generateKeyPair, type CipherState, type KeyPair } from "../../src/noise/noise";
import type { SocketLike } from "../../src/protocol/client";
import { Reassembler, frameMessage } from "../../src/protocol/framing";
import {
  MAX_CLIENT_MESSAGE_BYTES,
  MAX_UPLOAD_MESSAGE_BYTES,
  PROLOGUE,
  isClientHello,
  isClientMsg,
  type ClientHello,
  type ClientMsg,
} from "../../src/protocol/types";

export const rng = (n: number) => new Uint8Array(nodeRandomBytes(n));

/** Byte equality for large arrays: `toEqual` walks a 1 MiB array element by
 *  element for seconds, past the test timeout on a loaded machine. */
export const sameBytes = (a: Uint8Array | null | undefined, b: Uint8Array) =>
  a != null && Buffer.from(a.buffer, a.byteOffset, a.byteLength).equals(b);

export interface GatewayOptions {
  key: KeyPair;
  hello?: Record<string, unknown>;
}

export class FakeGatewaySocket implements SocketLike {
  binaryType?: string;
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  /** Bytes the fake holds unsent (a test sets it to model a slow link; 0: all sent). */
  bufferedAmount = 0;

  /** Decrypted, decoded client messages. */
  sent: ClientMsg[] = [];
  /** Raw frames the client sent (handshake and transport). */
  frames: Uint8Array[] = [];
  clientHello: ClientHello | null = null;
  closed = false;
  private readonly hs: XXHandshake;
  private send_: CipherState | null = null;
  private recv_: CipherState | null = null;
  // as the gateway: an `upload` chunk may be larger than every other message
  private readonly reasm = new Reassembler(MAX_UPLOAD_MESSAGE_BYTES);
  private readonly queue: Uint8Array[] = [];

  constructor(
    readonly url: string,
    private readonly opts: GatewayOptions,
  ) {
    this.hs = new XXHandshake({ initiator: false, prologue: utf8Encode(PROLOGUE), s: opts.key, random: rng });
  }

  send(data: ArrayBuffer): void {
    if (!(data instanceof ArrayBuffer)) throw new Error("client sent a non-binary frame");
    const bytes = new Uint8Array(data);
    this.frames.push(bytes);
    if (!this.send_) {
      const payload = this.hs.readMessage(bytes);
      if (this.hs.isComplete) {
        const hello: unknown = JSON.parse(utf8Decode(payload));
        if (!isClientHello(hello)) throw new Error(`bad client hello ${utf8Decode(payload)}`);
        this.clientHello = hello;
        this.send_ = this.hs.transport.send;
        this.recv_ = this.hs.transport.recv;
      } else {
        const hello = this.opts.hello ?? { v: 2, host: "studio-mac", os: "macos", version: "0.1.0" };
        this.queue.push(this.hs.writeMessage(utf8Encode(JSON.stringify(hello))));
      }
      return;
    }
    const msg = this.reasm.push(this.recv_!.decrypt(bytes));
    if (!msg) return;
    const json: unknown = JSON.parse(utf8Decode(msg));
    if (!isClientMsg(json)) throw new Error(`client sent a malformed message: ${utf8Decode(msg)}`);
    if (json.t !== "upload" && msg.length > MAX_CLIENT_MESSAGE_BYTES) throw new Error(`client sent a ${msg.length} byte ${json.t}`);
    this.sent.push(json);
  }

  close() {
    this.closed = true;
  }

  // --- test side ---

  get handshakeDone(): boolean {
    return this.send_ !== null;
  }

  open() {
    this.onopen?.({});
  }

  /** Delivers queued handshake replies. */
  deliver() {
    while (this.queue.length) this.onmessage?.({ data: this.queue.shift()!.slice().buffer });
  }

  /** open + handshake. */
  connect() {
    this.open();
    this.deliver();
    if (!this.handshakeDone) throw new Error("handshake did not complete");
  }

  /** Sends one JSON message, chunked and encrypted. */
  recv(msg: unknown, chunkMax?: number) {
    for (const f of this.encryptFrames(utf8Encode(JSON.stringify(msg)), chunkMax)) this.onmessage?.({ data: f.slice().buffer });
  }

  encryptFrames(message: Uint8Array, chunkMax?: number): Uint8Array[] {
    return frameMessage(message, chunkMax).map((p) => this.send_!.encrypt(p));
  }

  sendRawFrame(data: unknown) {
    this.onmessage?.({ data });
  }

  drop() {
    this.onclose?.({});
  }
}

export function gatewayKey(): KeyPair {
  return generateKeyPair(rng);
}
