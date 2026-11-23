/**
 * Pairs the browser's connections to the phone's loopback listener with tunnel streams
 * (feature `tunnel`). Pure: the listener (`LocalSide`, the rift-tunnel module) and the
 * connection (`send`) are passed in, so tests drive it with fakes.
 *
 * Each side keeps at most WINDOW unacknowledged bytes of a stream in flight: the computer's
 * data is acknowledged once written to the browser's socket; the browser's is held here until
 * the computer acknowledges what it has.
 */

import { MAX_DATA, WINDOW, type Frame } from "./frames";

/** The phone's listener: what the browser's connections are. */
export interface LocalSide {
  /** Write to the browser's connection; resolves once written. */
  write(stream: number, data: Uint8Array): Promise<void>;
  /** Close the browser's connection. */
  close(stream: number): void;
}

/** The browser's bytes held for one stream, past the window: more than this and it is reset. */
const MAX_HELD = 4 * 1024 * 1024;

interface Stream {
  opened: boolean;
  /** The browser's bytes not yet sent: before `opened`, or past the window. */
  held: Uint8Array[];
  heldBytes: number;
  /** Sent to the computer and not yet acknowledged. */
  inFlight: number;
  /** The browser closed its connection: `end` once what is held went. */
  localDone: boolean;
  sentEnd: boolean;
  gotEnd: boolean;
  /** Writes to the browser still going: the computer's `end` closes it after them. */
  writing: Promise<void>;
}

export class Streams {
  private readonly streams = new Map<number, Stream>();

  constructor(
    private readonly port: number,
    private readonly send: (f: Frame) => boolean,
    private readonly local: LocalSide,
  ) {}

  get size(): number {
    return this.streams.size;
  }

  /** The browser connected: ask the computer for the port. */
  localOpen(stream: number): void {
    if (this.streams.has(stream)) return;
    this.streams.set(stream, {
      opened: false,
      held: [],
      heldBytes: 0,
      inFlight: 0,
      localDone: false,
      sentEnd: false,
      gotEnd: false,
      writing: Promise.resolve(),
    });
    if (!this.send({ op: "open", stream, port: this.port })) this.abort(stream);
  }

  /** The browser sent bytes. */
  localData(stream: number, data: Uint8Array): void {
    const s = this.streams.get(stream);
    if (!s || s.localDone) return;
    s.held.push(data);
    s.heldBytes += data.length;
    if (s.heldBytes > MAX_HELD) {
      this.send({ op: "reset", stream, reason: "aborted" });
      return this.abort(stream);
    }
    this.flush(stream, s);
  }

  /**
   * The browser finished sending (half close: it still reads the reply): `end` goes once what it
   * sent went, also when that is before `opened`.
   */
  localEnd(stream: number): void {
    const s = this.streams.get(stream);
    if (!s) return;
    s.localDone = true;
    this.flush(stream, s);
  }

  /** The browser's connection is gone: before `opened` the stream is given up, after it ends. */
  localClose(stream: number): void {
    const s = this.streams.get(stream);
    if (!s) return;
    if (!s.opened) {
      this.send({ op: "reset", stream, reason: "aborted" });
      this.streams.delete(stream);
      return;
    }
    this.localEnd(stream);
  }

  /** A frame from the computer. */
  onFrame(f: Frame): void {
    if (f.op === "ping" || f.op === "pong" || f.op === "open") return;
    const s = this.streams.get(f.stream);
    if (!s) return;
    switch (f.op) {
      case "opened":
        s.opened = true;
        this.flush(f.stream, s);
        break;
      case "data": {
        const n = f.data.length;
        s.writing = s.writing.then(() =>
          this.local.write(f.stream, f.data).then(
            () => {
              if (this.streams.get(f.stream) === s) this.send({ op: "ack", stream: f.stream, bytes: n });
            },
            () => {
              if (this.streams.get(f.stream) !== s) return;
              this.send({ op: "reset", stream: f.stream, reason: "error" });
              this.abort(f.stream);
            },
          ),
        );
        break;
      }
      case "ack":
        s.inFlight = Math.max(0, s.inFlight - f.bytes);
        this.flush(f.stream, s);
        break;
      case "end":
        s.gotEnd = true;
        // the reply is whole once written: the browser reads its end
        void s.writing.then(() => {
          if (this.streams.get(f.stream) !== s) return;
          this.local.close(f.stream);
          this.finish(f.stream, s);
        });
        break;
      case "reset":
        this.abort(f.stream);
        break;
    }
  }

  /** The connection to the computer ended: every browser connection closes. */
  closeAll(): void {
    for (const id of [...this.streams.keys()]) this.abort(id);
  }

  /** Send what the window allows, then `end` once the browser is done and nothing is held. */
  private flush(stream: number, s: Stream): void {
    if (!s.opened) return;
    while (s.held.length > 0 && s.inFlight < WINDOW) {
      let chunk = s.held[0]!;
      const room = Math.min(MAX_DATA, WINDOW - s.inFlight);
      if (chunk.length > room) {
        s.held[0] = chunk.subarray(room);
        chunk = chunk.subarray(0, room);
      } else {
        s.held.shift();
      }
      s.heldBytes -= chunk.length;
      s.inFlight += chunk.length;
      if (!this.send({ op: "data", stream, data: chunk })) return this.abort(stream);
    }
    if (s.localDone && s.held.length === 0 && !s.sentEnd) {
      s.sentEnd = true;
      this.send({ op: "end", stream });
      this.finish(stream, s);
    }
  }

  private finish(stream: number, s: Stream): void {
    if (s.sentEnd && s.gotEnd) this.streams.delete(stream);
  }

  private abort(stream: number): void {
    if (!this.streams.delete(stream)) return;
    this.local.close(stream);
  }
}
