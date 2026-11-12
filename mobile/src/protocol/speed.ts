// What the connection to one computer measures, for the Connection speed page: ping round
// trips, bytes each way, screen updates, and how long input and scrolling take to show. Pure
// (no React Native): the client feeds it, the page reads snapshots once a second. Nothing is
// estimated: a number exists only once the client timed it.

/** Samples kept per series (a minute at the page's 1 s probe). */
export const SPEED_SAMPLES = 60;
/** How often the client pings while the page is open (else every PING_INTERVAL_MS). */
export const PROBE_INTERVAL_MS = 1_000;
/**
 * An input or a scroll that no screen update follows within this long caused none (a program
 * that does not redraw): it is dropped, never counted late.
 */
export const ECHO_WINDOW_MS = 5_000;
/**
 * Only an input or a scroll this long after the one before starts a clock. Nothing ties a screen
 * update to the input that caused it, so during a burst (a swipe sends a scroll a frame) the next
 * update may answer an earlier one, timing a few ms below the round trip; after a quiet second,
 * the next update can only be the answer to this one.
 */
export const QUIET_MS = 1_000;
/** How long after an input the pane's updates show at once: the gateway's KICK_WINDOW. */
export const ANSWER_WINDOW_MS = 600;

/** Cumulative counters and recent samples, since the connection's `welcome`. */
export interface SpeedSnapshot {
  /** Epoch ms the counting began (the welcome). */
  since: number;
  /** Epoch ms of this snapshot. */
  at: number;
  /** Bytes received and sent on the socket (encrypted, as on the wire). */
  bytesIn: number;
  bytesOut: number;
  /** Screen updates of the followed pane: snapshots, updates and full-screen frames. */
  frames: number;
  /** Ping round trips, oldest first, in ms. */
  rtt: number[];
  /** From an input (typed text, keys, paste, a tap) to the next screen update, in ms. */
  echo: number[];
  /** From a scroll request to the next screen update, in ms. */
  scroll: number[];
}

export interface Summary {
  last: number;
  median: number;
  p95: number;
  min: number;
  max: number;
  /** Mean change between consecutive samples (how steady it is). */
  jitter: number;
  count: number;
}

/** Per-second rates between two snapshots of the same connection. */
export interface Rates {
  inPerSec: number;
  outPerSec: number;
  framesPerSec: number;
}

const push = (list: number[], v: number) => {
  list.push(v);
  if (list.length > SPEED_SAMPLES) list.splice(0, list.length - SPEED_SAMPLES);
};

export class SpeedMeter {
  private since = 0;
  private bytesIn = 0;
  private bytesOut = 0;
  private frames = 0;
  private rtt: number[] = [];
  private echo: number[] = [];
  private scroll: number[] = [];
  private echoAt: number | null = null;
  private scrollAt: number | null = null;
  /** The last input or scroll (epoch ms), for [`SpeedMeter.answering`]. */
  private lastInputAt = -Infinity;

  constructor(now: number = Date.now()) {
    this.since = now;
  }

  /** A new connection: its numbers start over. */
  reset(now: number): void {
    this.since = now;
    this.bytesIn = this.bytesOut = this.frames = 0;
    this.rtt = [];
    this.echo = [];
    this.scroll = [];
    this.echoAt = this.scrollAt = null;
  }

  received(bytes: number): void {
    this.bytesIn += bytes;
  }

  sent(bytes: number): void {
    this.bytesOut += bytes;
  }

  pong(ms: number): void {
    push(this.rtt, ms);
  }

  /** Input went to the pane: the first after a quiet QUIET_MS starts the clock. */
  input(now: number): void {
    if (now - this.lastInputAt >= QUIET_MS) this.echoAt ??= now;
    this.lastInputAt = now;
  }

  /** A scroll went to the pane: timed as `input` is. */
  scrolled(now: number): void {
    if (now - this.lastInputAt >= QUIET_MS) this.scrollAt ??= now;
    this.lastInputAt = now;
  }

  /**
   * This device typed or scrolled within ANSWER_WINDOW_MS: the program is answering it, and its
   * updates should show within a frame (the gateway reads the pane every 16 ms meanwhile).
   */
  answering(now: number): boolean {
    return now - this.lastInputAt < ANSWER_WINDOW_MS;
  }

  /** A screen update of the followed pane arrived. */
  frame(now: number): void {
    this.frames++;
    if (this.echoAt !== null) {
      const ms = now - this.echoAt;
      if (ms <= ECHO_WINDOW_MS) push(this.echo, Math.max(0, ms));
      this.echoAt = null;
    }
    if (this.scrollAt !== null) {
      const ms = now - this.scrollAt;
      if (ms <= ECHO_WINDOW_MS) push(this.scroll, Math.max(0, ms));
      this.scrollAt = null;
    }
  }

  snapshot(now: number): SpeedSnapshot {
    // a wait that has run out caused nothing: forget it rather than count it later
    if (this.echoAt !== null && now - this.echoAt > ECHO_WINDOW_MS) this.echoAt = null;
    if (this.scrollAt !== null && now - this.scrollAt > ECHO_WINDOW_MS) this.scrollAt = null;
    return {
      since: this.since,
      at: now,
      bytesIn: this.bytesIn,
      bytesOut: this.bytesOut,
      frames: this.frames,
      rtt: [...this.rtt],
      echo: [...this.echo],
      scroll: [...this.scroll],
    };
  }
}

/** The samples' summary, or null when there are none. */
export function summarize(samples: readonly number[]): Summary | null {
  if (samples.length === 0) return null;
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!;
  let jitter = 0;
  for (let i = 1; i < samples.length; i++) jitter += Math.abs(samples[i]! - samples[i - 1]!);
  return {
    last: samples[samples.length - 1]!,
    median: at(0.5),
    p95: at(0.95),
    min: sorted[0]!,
    max: sorted[sorted.length - 1]!,
    jitter: samples.length > 1 ? jitter / (samples.length - 1) : 0,
    count: samples.length,
  };
}

/** Rates from `prev` to `cur`; null when they are not of one connection or no time passed. */
export function rates(prev: SpeedSnapshot | null, cur: SpeedSnapshot): Rates | null {
  if (!prev || prev.since !== cur.since) return null;
  const secs = (cur.at - prev.at) / 1000;
  if (secs <= 0) return null;
  return {
    inPerSec: (cur.bytesIn - prev.bytesIn) / secs,
    outPerSec: (cur.bytesOut - prev.bytesOut) / secs,
    framesPerSec: (cur.frames - prev.frames) / secs,
  };
}

/** `820 B/s`, `12.4 KB/s`, `3.1 MB/s` (1000-based, as network speeds are given). */
export function rateText(bytesPerSec: number): string {
  if (bytesPerSec < 1000) return `${Math.round(bytesPerSec)} B/s`;
  if (bytesPerSec < 1_000_000) return `${(bytesPerSec / 1000).toFixed(1)} KB/s`;
  return `${(bytesPerSec / 1_000_000).toFixed(1)} MB/s`;
}

/** `640 B`, `12.4 KB`, `3.1 MB`. */
export function bytesText(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} KB`;
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/** How a round trip feels, from its median: the page's one-word verdict. */
export function verdict(medianMs: number): { word: string; tone: "ok" | "warn" | "bad" } {
  if (medianMs < 80) return { word: "Fast", tone: "ok" };
  if (medianMs < 200) return { word: "Fair", tone: "warn" };
  return { word: "Slow", tone: "bad" };
}
