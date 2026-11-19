// Noise_XX_25519_ChaChaPoly_BLAKE2s, following the Noise Protocol Framework revision 34
// (https://noiseprotocol.org/noise.html) section by section: CipherState (§5.1),
// SymmetricState (§5.2), HandshakeState (§5.3) and the XX pattern (§7.5). Primitives are the
// audited pure-JS @noble packages: X25519 (§12.1), ChaChaPoly (§12.3) and BLAKE2s with HMAC/HKDF
// (§12.8, §4.3). The vectors in test/vectors/ and the snow transcript in
// crates/rift-remote/fixtures/noise/ check it byte for byte (test/noise.test.ts).

import { chacha20poly1305 } from "@noble/ciphers/chacha.js";
import { x25519 } from "@noble/curves/ed25519.js";
import { blake2s } from "@noble/hashes/blake2.js";
import { hmac } from "@noble/hashes/hmac.js";
import { concatBytes, equalBytes, utf8Encode } from "./bytes";

export const PROTOCOL_NAME = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
export const DHLEN = 32;
export const HASHLEN = 32;
const TAGLEN = 16;
/** Noise messages are at most 65535 bytes (§3). */
export const MAX_MESSAGE = 65535;
/**
 * The spec reserves nonce 2^64-1 and lets n run to it. JavaScript numbers are exact to 2^53, so
 * the counter stops there; 2^53 messages on one connection cannot happen in practice.
 */
const MAX_NONCE = Number.MAX_SAFE_INTEGER;

export type RandomBytes = (n: number) => Uint8Array;

export interface KeyPair {
  privateKey: Uint8Array;
  publicKey: Uint8Array;
}

export class NoiseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "NoiseError";
  }
}

/**
 * The responder's static key differs from the one pinned at pairing. `proven`: the rest of its
 * message decrypted with that key, so the responder holds it (`received` is the key of whoever
 * answered, not only a claim); null when the message failed to decrypt.
 */
export class NoiseKeyMismatchError extends NoiseError {
  constructor(readonly proven: Uint8Array | null = null) {
    super("the computer's key does not match the paired key");
    this.name = "NoiseKeyMismatchError";
  }
}

// --- Primitives (§4) ---------------------------------------------------------------------

/** GENERATE_KEYPAIR from 32 random bytes; X25519 clamps the scalar itself (RFC 7748). */
export function keyPairFromPrivate(privateKey: Uint8Array): KeyPair {
  if (privateKey.length !== DHLEN) throw new NoiseError("private key must be 32 bytes");
  return { privateKey, publicKey: x25519.getPublicKey(privateKey) };
}

export function generateKeyPair(random: RandomBytes): KeyPair {
  return keyPairFromPrivate(random(DHLEN));
}

function dh(kp: KeyPair, publicKey: Uint8Array): Uint8Array {
  try {
    return x25519.getSharedSecret(kp.privateKey, publicKey);
  } catch {
    // A low-order public key yields the all-zero output; noble refuses it (§12.1 allows this).
    throw new NoiseError("invalid public key");
  }
}

function hash(data: Uint8Array): Uint8Array {
  return blake2s(data);
}

function hmacHash(key: Uint8Array, data: Uint8Array): Uint8Array {
  return hmac(blake2s, key, data);
}

/** HKDF(chaining_key, input_key_material, num_outputs) of §4.3, for 2 or 3 outputs. */
export function hkdf(chainingKey: Uint8Array, ikm: Uint8Array, outputs: 2 | 3): Uint8Array[] {
  const tempKey = hmacHash(chainingKey, ikm);
  const out1 = hmacHash(tempKey, Uint8Array.of(0x01));
  const out2 = hmacHash(tempKey, concatBytes(out1, Uint8Array.of(0x02)));
  if (outputs === 2) return [out1, out2];
  return [out1, out2, hmacHash(tempKey, concatBytes(out2, Uint8Array.of(0x03)))];
}

/** ChaChaPoly nonce (§12.3): 32 bits of zeros, then the 64-bit counter little-endian. */
export function chachaNonce(n: number): Uint8Array {
  const nonce = new Uint8Array(12);
  let lo = n % 0x1_0000_0000;
  let hi = Math.floor(n / 0x1_0000_0000);
  for (let i = 0; i < 4; i++) {
    nonce[4 + i] = lo & 0xff;
    nonce[8 + i] = hi & 0xff;
    lo = Math.floor(lo / 256);
    hi = Math.floor(hi / 256);
  }
  return nonce;
}

// --- CipherState (§5.1) ------------------------------------------------------------------

export class CipherState {
  private k: Uint8Array | null = null;
  private n = 0;

  initializeKey(key: Uint8Array | null): void {
    this.k = key;
    this.n = 0;
  }

  hasKey(): boolean {
    return this.k !== null;
  }

  /** The next nonce (for tests). */
  get nonce(): number {
    return this.n;
  }

  encryptWithAd(ad: Uint8Array, plaintext: Uint8Array): Uint8Array {
    if (!this.k) return plaintext;
    if (this.n >= MAX_NONCE) throw new NoiseError("nonce exhausted");
    const ct = chacha20poly1305(this.k, chachaNonce(this.n), ad).encrypt(plaintext);
    this.n++;
    return ct;
  }

  decryptWithAd(ad: Uint8Array, ciphertext: Uint8Array): Uint8Array {
    if (!this.k) return ciphertext;
    if (this.n >= MAX_NONCE) throw new NoiseError("nonce exhausted");
    let pt: Uint8Array;
    try {
      pt = chacha20poly1305(this.k, chachaNonce(this.n), ad).decrypt(ciphertext);
    } catch {
      // Authentication failed: n is not incremented (§5.1).
      throw new NoiseError("decryption failed");
    }
    this.n++;
    return pt;
  }

  /** Transport-phase helpers: no associated data (§5.3 Split). */
  encrypt(plaintext: Uint8Array): Uint8Array {
    if (plaintext.length + TAGLEN > MAX_MESSAGE) throw new NoiseError("message too long");
    return this.encryptWithAd(new Uint8Array(0), plaintext);
  }

  decrypt(ciphertext: Uint8Array): Uint8Array {
    if (ciphertext.length > MAX_MESSAGE) throw new NoiseError("message too long");
    return this.decryptWithAd(new Uint8Array(0), ciphertext);
  }

  /** A copy with the same key and nonce (tests replaying a transcript). */
  clone(): CipherState {
    const c = new CipherState();
    c.k = this.k;
    c.n = this.n;
    return c;
  }
}

// --- SymmetricState (§5.2) ---------------------------------------------------------------

export class SymmetricState {
  private readonly cipher = new CipherState();
  private ck: Uint8Array;
  private h: Uint8Array;

  constructor(protocolName: string) {
    const name = utf8Encode(protocolName);
    if (name.length <= HASHLEN) {
      this.h = new Uint8Array(HASHLEN);
      this.h.set(name);
    } else {
      this.h = hash(name);
    }
    this.ck = this.h;
  }

  mixKey(ikm: Uint8Array): void {
    const [ck, tempK] = hkdf(this.ck, ikm, 2) as [Uint8Array, Uint8Array];
    this.ck = ck;
    // HASHLEN is 32, so temp_k needs no truncation.
    this.cipher.initializeKey(tempK);
  }

  mixHash(data: Uint8Array): void {
    this.h = hash(concatBytes(this.h, data));
  }

  get handshakeHash(): Uint8Array {
    return this.h;
  }

  hasKey(): boolean {
    return this.cipher.hasKey();
  }

  encryptAndHash(plaintext: Uint8Array): Uint8Array {
    const ct = this.cipher.encryptWithAd(this.h, plaintext);
    this.mixHash(ct);
    return ct;
  }

  decryptAndHash(ciphertext: Uint8Array): Uint8Array {
    const pt = this.cipher.decryptWithAd(this.h, ciphertext);
    this.mixHash(ciphertext);
    return pt;
  }

  split(): [CipherState, CipherState] {
    const [k1, k2] = hkdf(this.ck, new Uint8Array(0), 2) as [Uint8Array, Uint8Array];
    const c1 = new CipherState();
    const c2 = new CipherState();
    c1.initializeKey(k1);
    c2.initializeKey(k2);
    return [c1, c2];
  }
}

// --- HandshakeState for XX (§5.3, §7.5) ----------------------------------------------------
//
//   XX:
//     -> e
//     <- e, ee, s, es
//     -> s, se

export interface HandshakeOptions {
  initiator: boolean;
  prologue: Uint8Array;
  /** The local static key pair. */
  s: KeyPair;
  /** A fixed ephemeral key pair (test vectors only); otherwise generated from `random`. */
  e?: KeyPair;
  /**
   * Initiator only: the responder static key pinned at pairing. The handshake aborts with
   * NoiseKeyMismatchError when message 2 carries another key.
   */
  pinnedRemoteStatic?: Uint8Array;
  random: RandomBytes;
}

export interface TransportCiphers {
  send: CipherState;
  recv: CipherState;
  handshakeHash: Uint8Array;
  remoteStatic: Uint8Array;
}

export class XXHandshake {
  private readonly ss: SymmetricState;
  private readonly initiator: boolean;
  private readonly s: KeyPair;
  private e: KeyPair | null;
  private re: Uint8Array | null = null;
  private rs: Uint8Array | null = null;
  private readonly pinned: Uint8Array | null;
  private readonly random: RandomBytes;
  /** Index of the next pattern message (0, 1, 2); 3 once complete. */
  private step = 0;
  private failed = false;
  private result: TransportCiphers | null = null;

  constructor(opts: HandshakeOptions) {
    this.initiator = opts.initiator;
    this.s = opts.s;
    this.e = opts.e ?? null;
    this.pinned = opts.pinnedRemoteStatic ?? null;
    if (this.pinned && this.pinned.length !== DHLEN) throw new NoiseError("pinned key must be 32 bytes");
    this.random = opts.random;
    this.ss = new SymmetricState(PROTOCOL_NAME);
    this.ss.mixHash(opts.prologue);
    // XX has no pre-messages.
  }

  get isComplete(): boolean {
    return this.result !== null;
  }

  /** Whether the next call is writeMessage (true) or readMessage (false). */
  get isMyTurn(): boolean {
    return this.step < 3 && (this.step % 2 === 0) === this.initiator;
  }

  /** The transport ciphers once all three messages are through. */
  get transport(): TransportCiphers {
    if (!this.result) throw new NoiseError("handshake not complete");
    return this.result;
  }

  writeMessage(payload: Uint8Array): Uint8Array {
    this.guard(true);
    try {
      const out: Uint8Array[] = [];
      switch (this.step) {
        case 0: // -> e
          out.push(this.writeE());
          break;
        case 1: // <- e, ee, s, es
          out.push(this.writeE());
          this.ss.mixKey(dh(this.e!, this.re!));
          out.push(this.ss.encryptAndHash(this.s.publicKey));
          this.ss.mixKey(dh(this.s, this.re!)); // es, responder side: DH(s, re)
          break;
        case 2: // -> s, se
          out.push(this.ss.encryptAndHash(this.s.publicKey));
          this.ss.mixKey(dh(this.s, this.re!)); // se, initiator side: DH(s, re)
          break;
      }
      out.push(this.ss.encryptAndHash(payload));
      const msg = concatBytes(...out);
      if (msg.length > MAX_MESSAGE) throw new NoiseError("message too long");
      this.advance();
      return msg;
    } catch (e) {
      this.failed = true;
      throw e;
    }
  }

  readMessage(message: Uint8Array): Uint8Array {
    this.guard(false);
    try {
      if (message.length > MAX_MESSAGE) throw new NoiseError("message too long");
      let at = 0;
      const take = (n: number): Uint8Array => {
        if (message.length - at < n) throw new NoiseError("handshake message too short");
        const part = message.subarray(at, at + n);
        at += n;
        return part;
      };
      const keyLen = () => DHLEN + (this.ss.hasKey() ? TAGLEN : 0);
      switch (this.step) {
        case 0: // -> e
          this.readE(take(DHLEN));
          break;
        case 1: {
          // <- e, ee, s, es
          this.readE(take(DHLEN));
          this.ss.mixKey(dh(this.e!, this.re!));
          const rs = this.ss.decryptAndHash(take(keyLen()));
          // Pinning: this handshake ends here, whatever follows. Only to say whose key answered
          // (an old pairing of a computer paired again), the rest of the message is read with
          // it: it decrypts only for a responder that holds that key, so a key merely claimed
          // is never reported.
          if (this.pinned && !equalBytes(rs, this.pinned)) {
            const claimed = Uint8Array.from(rs);
            let proven: Uint8Array | null = null;
            try {
              this.ss.mixKey(dh(this.e!, claimed));
              this.ss.decryptAndHash(message.subarray(at));
              proven = claimed;
            } catch {
              // not its key: nothing is said about who answered
            }
            throw new NoiseKeyMismatchError(proven);
          }
          this.rs = Uint8Array.from(rs);
          this.ss.mixKey(dh(this.e!, this.rs)); // es, initiator side: DH(e, rs)
          break;
        }
        case 2: {
          // -> s, se
          this.rs = Uint8Array.from(this.ss.decryptAndHash(take(keyLen())));
          this.ss.mixKey(dh(this.e!, this.rs)); // se, responder side: DH(e, rs)
          break;
        }
      }
      const payload = this.ss.decryptAndHash(message.subarray(at));
      this.advance();
      return Uint8Array.from(payload);
    } catch (e) {
      this.failed = true;
      throw e;
    }
  }

  private guard(writing: boolean): void {
    if (this.failed) throw new NoiseError("handshake already failed");
    if (this.result) throw new NoiseError("handshake already complete");
    if (this.isMyTurn !== writing) throw new NoiseError(writing ? "not our turn to write" : "not our turn to read");
  }

  private writeE(): Uint8Array {
    if (!this.e) this.e = generateKeyPair(this.random);
    this.ss.mixHash(this.e.publicKey);
    // No PSK, so `e` does not also call MixKey.
    return this.e.publicKey;
  }

  private readE(re: Uint8Array): void {
    this.re = Uint8Array.from(re);
    this.ss.mixHash(this.re);
  }

  private advance(): void {
    this.step++;
    if (this.step < 3) return;
    const [c1, c2] = this.ss.split();
    this.result = {
      send: this.initiator ? c1 : c2,
      recv: this.initiator ? c2 : c1,
      handshakeHash: this.ss.handshakeHash,
      remoteStatic: this.rs!,
    };
    // Ephemeral private key is no longer needed.
    this.e = null;
  }
}
