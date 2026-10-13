// Message framing inside the Noise channel (docs/remote-protocol.md, Transport): each protocol
// message is UTF-8 JSON split into chunks; every Noise plaintext is one flag byte (0x00 last
// chunk, 0x01 more follow) and at most 65 000 bytes of the JSON text.

export const CHUNK_MAX = 65_000;
export const FLAG_LAST = 0x00;
export const FLAG_MORE = 0x01;
/** Gateway messages are at most 1 MiB of JSON. */
export const MAX_GATEWAY_MESSAGE_BYTES = 1024 * 1024;

export class FramingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "FramingError";
  }
}

/** The Noise plaintexts that carry `message`, in order. */
export function frameMessage(message: Uint8Array, chunkMax = CHUNK_MAX): Uint8Array[] {
  const out: Uint8Array[] = [];
  let at = 0;
  do {
    const end = Math.min(at + chunkMax, message.length);
    const chunk = new Uint8Array(1 + end - at);
    chunk[0] = end === message.length ? FLAG_LAST : FLAG_MORE;
    chunk.set(message.subarray(at, end), 1);
    out.push(chunk);
    at = end;
  } while (at < message.length);
  return out;
}

/** Joins chunks back into messages; throws FramingError on a malformed or oversized stream. */
export class Reassembler {
  private parts: Uint8Array[] = [];
  private size = 0;

  constructor(private readonly maxBytes = MAX_GATEWAY_MESSAGE_BYTES) {}

  /** Feeds one decrypted plaintext; returns the whole message once its last chunk arrives. */
  push(plaintext: Uint8Array): Uint8Array | null {
    if (plaintext.length === 0) throw new FramingError("empty chunk");
    const flag = plaintext[0];
    if (flag !== FLAG_LAST && flag !== FLAG_MORE) throw new FramingError(`bad chunk flag ${flag}`);
    const body = plaintext.subarray(1);
    if (body.length > CHUNK_MAX) throw new FramingError("chunk too large");
    this.size += body.length;
    if (this.size > this.maxBytes) throw new FramingError("message too large");
    this.parts.push(body);
    if (flag === FLAG_MORE) return null;
    const parts = this.parts;
    const size = this.size;
    this.parts = [];
    this.size = 0;
    if (parts.length === 1) return Uint8Array.from(parts[0]!);
    const out = new Uint8Array(size);
    let at = 0;
    for (const p of parts) {
      out.set(p, at);
      at += p.length;
    }
    return out;
  }
}
