// A finger dragging the Terminal view scrolls the pane on the computer (feature `scroll`,
// docs/remote-protocol.md): its movement becomes lines, coalesced and sent at most every
// SCROLL_INTERVAL_MS, so a fling is a few `scroll` messages, never one per touch event, and stays
// under the gateway's rate (20 a second).

import { MAX_SCROLL_LINES } from "./protocol/types";

/** At most one `scroll` this often (12 a second, under the gateway's 20). */
export const SCROLL_INTERVAL_MS = 80;
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
    return this.deps.intervalMs ?? SCROLL_INTERVAL_MS;
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
