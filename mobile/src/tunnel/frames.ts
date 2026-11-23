/**
 * The tunnel's binary frames (feature `tunnel`): the same bytes as
 * `crates/rift-remote/src/tunnel.rs`, tested against the shared fixtures
 * (`crates/rift-remote/fixtures/tunnel/frames.json`). Spec:
 * docs/remote-protocol.md, "Local web servers".
 */

/** Unacknowledged bytes of one stream either side may have in flight. */
export const WINDOW = 256 * 1024;
/** The most data one `data` frame carries. */
export const MAX_DATA = 32 * 1024;
/** Streams open at once on one tunnel (the computer resets the next `limit`). */
export const MAX_STREAMS = 64;

export type ResetReason = "refused" | "limit" | "error" | "protocol" | "aborted";
const REASONS: readonly ResetReason[] = ["refused", "limit", "error", "protocol", "aborted"];

export type Frame =
  | { op: "open"; stream: number; port: number }
  | { op: "opened"; stream: number }
  | { op: "data"; stream: number; data: Uint8Array }
  | { op: "ack"; stream: number; bytes: number }
  | { op: "end"; stream: number }
  | { op: "reset"; stream: number; reason: ResetReason }
  | { op: "ping" }
  | { op: "pong" };

const OPS = { open: 1, opened: 2, data: 3, ack: 4, end: 5, reset: 6, ping: 7, pong: 8 } as const;

export class FrameError extends Error {}

export function encodeFrame(f: Frame): Uint8Array {
  if (f.op === "ping" || f.op === "pong") return Uint8Array.of(OPS[f.op]);
  const extra = f.op === "open" ? 2 : f.op === "data" ? f.data.length : f.op === "ack" ? 4 : f.op === "reset" ? 1 : 0;
  const out = new Uint8Array(5 + extra);
  const view = new DataView(out.buffer);
  out[0] = OPS[f.op];
  view.setUint32(1, f.stream);
  switch (f.op) {
    case "open":
      view.setUint16(5, f.port);
      break;
    case "data":
      out.set(f.data, 5);
      break;
    case "ack":
      view.setUint32(5, f.bytes);
      break;
    case "reset":
      out[5] = REASONS.indexOf(f.reason) + 1;
      break;
  }
  return out;
}

/** One frame from the computer; throws FrameError on anything malformed. */
export function decodeFrame(b: Uint8Array): Frame {
  if (b.length === 0) throw new FrameError("empty frame");
  const op = b[0]!;
  if (op === 7 || op === 8) {
    if (b.length !== 1) throw new FrameError("bad length");
    return { op: op === 7 ? "ping" : "pong" };
  }
  if (b.length < 5) throw new FrameError("bad length");
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const stream = view.getUint32(1);
  const body = b.length - 5;
  if (stream === 0) throw new FrameError("stream 0");
  const exact = (n: number) => {
    if (body !== n) throw new FrameError("bad length");
  };
  switch (op) {
    case 1: {
      exact(2);
      const port = view.getUint16(5);
      if (port === 0) throw new FrameError("port 0");
      return { op: "open", stream, port };
    }
    case 2:
      exact(0);
      return { op: "opened", stream };
    case 3:
      if (body === 0 || body > MAX_DATA) throw new FrameError("bad length");
      return { op: "data", stream, data: b.slice(5) };
    case 4: {
      exact(4);
      const bytes = view.getUint32(5);
      if (bytes === 0 || bytes > WINDOW) throw new FrameError("bad ack");
      return { op: "ack", stream, bytes };
    }
    case 5:
      exact(0);
      return { op: "end", stream };
    case 6: {
      exact(1);
      const reason = REASONS[b[5]! - 1];
      if (!reason) throw new FrameError("bad reason");
      return { op: "reset", stream, reason };
    }
    default:
      throw new FrameError("unknown op");
  }
}
