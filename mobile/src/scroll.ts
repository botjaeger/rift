// A finger dragging the Terminal view scrolls the pane on the computer (feature `scroll`,
// docs/remote-protocol.md): its movement becomes lines, coalesced and sent at most every
// SCROLL_INTERVAL_MS, so a drag is small steps (the program moves smoothly), never one message
// per touch event, under the gateway's rate (40 a second; 20 on a desktop before 0.22.5, which
// SLOW_SCROLL_INTERVAL_MS keeps to). A flick goes on after the finger lifts (Fling), slowing as a
// native list does.

import { MAX_SCROLL_LINES } from "./protocol/types";

/** At most one `scroll` this often (33 a second, under the gateway's 40). */
export const SCROLL_INTERVAL_MS = 30;
/** The pace after a desktop refused one for its rate (an older one allows 20 a second). */
export const SLOW_SCROLL_INTERVAL_MS = 80;
/** Finger travel before a touch counts as a drag (as a tap's `moved`). */
export const DRAG_SLOP = 10;

/**
 * Lines a finger movement of `dy` points scrolls, rows `rowHeight` points tall: dragging down
 * (positive `dy`) shows what is above, as a wheel turned up (positive lines).
 */
export function dragLines(dy: number, rowHeight: number): number {
  if (!(rowHeight > 0) || !Number.isFinite(dy)) return 0;
  return dy / rowHeight;
}

/**
 * Whether a vertical drag of `dy` scrolls the computer's pane rather than the view's own scroller:
 * always when the view has nothing to scroll locally; when it does (zoomed in), only past its
 * edge (pulling down at its top, up at its bottom).
 */
export function dragScrollsPane(dy: number, local: { overflow: boolean; atTop: boolean; atBottom: boolean }): boolean {
  if (!local.overflow) return true;
  return (dy > 0 && local.atTop) || (dy < 0 && local.atBottom);
}

/**
 * Collects drag movement as fractional lines and sends whole lines, at most every
 * `intervalMs`: the first movement goes out at once, what comes within the interval waits and
 * goes out together. Each message is at most MAX_SCROLL_LINES either way; the rest waits for the
 * next one, and the fraction left over carries on. `flush` (the finger lifted) drops the
 * fraction and sends the whole lines that wait as soon as the interval allows, never faster;
 * `cancel` drops them.
 */
export class ScrollCoalescer {
  private pending = 0;
  private lastSent = Number.NEGATIVE_INFINITY;
  private timer: unknown = null;
  private slow = false;

  constructor(
    private readonly deps: {
      send: (lines: number) => void;
      now: () => number;
      schedule: (fn: () => void, ms: number) => unknown;
      unschedule: (handle: unknown) => void;
      intervalMs?: number;
    },
  ) {}

  private get interval(): number {
    return this.slow ? Math.max(SLOW_SCROLL_INTERVAL_MS, this.deps.intervalMs ?? 0) : (this.deps.intervalMs ?? SCROLL_INTERVAL_MS);
  }

  /**
   * The desktop refused `lines` for its rate: keep to the slower pace from now on and send them
   * again, so no distance is lost.
   */
  refused(lines: number): void {
    this.slow = true;
    this.add(lines);
  }

  /** Adds `lines` (fractional; positive: up). */
  add(lines: number): void {
    if (!Number.isFinite(lines) || lines === 0) return;
    this.pending += lines;
    this.pump();
  }

  /**
   * The drag ended: drops the fraction; the whole lines that wait go out at the interval (at once
   * when it has passed), in as many messages as the bound needs. Never a burst past the
   * gateway's rate, however much waits.
   */
  flush(): void {
    this.pending = Math.trunc(this.pending);
    if (this.pending === 0) return this.clearTimer();
    if (this.timer === null) this.pump();
  }

  /** Drops what waits. */
  cancel(): void {
    this.clearTimer();
    this.pending = 0;
  }

  private pump(): void {
    if (this.timer !== null) return;
    const wait = this.lastSent + this.interval - this.deps.now();
    if (wait <= 0) {
      this.emit();
      // a remainder over one message's bound goes next time
      if (Math.abs(Math.trunc(this.pending)) >= 1) this.arm(this.interval);
      return;
    }
    this.arm(wait);
  }

  private arm(ms: number): void {
    this.timer = this.deps.schedule(() => {
      this.timer = null;
      this.pump();
    }, ms);
  }

  private clearTimer(): void {
    if (this.timer !== null) this.deps.unschedule(this.timer);
    this.timer = null;
  }

  private emit(): void {
    const whole = Math.trunc(this.pending);
    if (whole === 0) return;
    const lines = Math.max(-MAX_SCROLL_LINES, Math.min(MAX_SCROLL_LINES, whole));
    this.pending -= lines;
    this.lastSent = this.deps.now();
    this.deps.send(lines);
  }
}

/** Lines a second below which a lifted finger leaves the pane where it is. */
export const FLING_MIN_SPEED = 8;
/** How fast a fling slows: its speed falls to 1/e every this many ms (iOS lists, about). */
export const FLING_DECAY_MS = 325;
/** Touch movement older than this does not count towards a flick's speed. */
export const FLING_WINDOW_MS = 100;
/** The fling's step: one display frame. */
export const FLING_STEP_MS = 16;

/**
 * Speed of a drag in lines a second, from the movement of its last FLING_WINDOW_MS (a finger that
 * stopped before lifting has none).
 */
export class DragVelocity {
  private samples: { at: number; lines: number }[] = [];

  add(lines: number, at: number): void {
    const last = this.samples.at(-1);
    // A reversal brakes the old flick; only movement in the new direction sets its speed.
    if (last && last.lines * lines < 0) this.samples = [{ at: last.at, lines: 0 }];
    this.samples.push({ at, lines });
    this.samples = this.samples.filter((s) => at - s.at <= FLING_WINDOW_MS);
  }

  /** Lines a second at `at` (positive: up); 0 without recent movement. */
  speed(at: number): number {
    const recent = this.samples.filter((s) => at - s.at <= FLING_WINDOW_MS);
    if (recent.length < 2) return 0;
    const span = at - recent[0]!.at;
    if (span <= 0) return 0;
    // The first sample's movement happened before this interval, not within it.
    return (recent.slice(1).reduce((sum, s) => sum + s.lines, 0) * 1000) / span;
  }

  clear(): void {
    this.samples = [];
  }
}

/**
 * Carries a flick on after the finger lifts: `add` gets the lines of each frame, the speed decaying
 * exponentially (FLING_DECAY_MS) until it falls under FLING_MIN_SPEED, then `done` runs. A new
 * touch stops it (`stop`, without `done`).
 */
export class Fling {
  private speed = 0;
  private last = 0;
  private timer: unknown = null;

  constructor(
    private readonly deps: {
      add: (lines: number) => void;
      done: () => void;
      now: () => number;
      schedule: (fn: () => void, ms: number) => unknown;
      unschedule: (handle: unknown) => void;
    },
  ) {}

  /** Starts at `speed` lines a second; false (and `done` at once) when too slow to fling. */
  start(speed: number): boolean {
    this.stop();
    if (!Number.isFinite(speed) || Math.abs(speed) < FLING_MIN_SPEED) {
      this.deps.done();
      return false;
    }
    this.speed = speed;
    this.last = this.deps.now();
    this.timer = this.deps.schedule(() => this.step(), FLING_STEP_MS);
    return true;
  }

  stop(): void {
    if (this.timer !== null) this.deps.unschedule(this.timer);
    this.timer = null;
    this.speed = 0;
  }

  get running(): boolean {
    return this.timer !== null;
  }

  private step(): void {
    this.timer = null;
    const now = this.deps.now();
    const dt = Math.max(0, now - this.last);
    this.last = now;
    // the distance of this frame under exponential decay, then the speed left
    const k = Math.exp(-dt / FLING_DECAY_MS);
    this.deps.add((this.speed * FLING_DECAY_MS * (1 - k)) / 1000);
    this.speed *= k;
    if (Math.abs(this.speed) < FLING_MIN_SPEED) {
      this.speed = 0;
      this.deps.done();
      return;
    }
    this.timer = this.deps.schedule(() => this.step(), FLING_STEP_MS);
  }
}
