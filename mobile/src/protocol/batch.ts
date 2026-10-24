// A small external store for React (useSyncExternalStore) that coalesces writes: state changes
// apply at once, listeners hear about them at most once per PUBLISH_INTERVAL_MS, so a flood of
// gateway frames does not re-render the UI per frame.

export const PUBLISH_INTERVAL_MS = 100;
/** A write that must show within a display frame (a terminal screen while it scrolls). */
export const FRAME_MS = 16;

export class BatchedStore<T> {
  private current: T;
  private published: T;
  private readonly listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** When the pending publish runs. */
  private due = Infinity;
  private lastPublish = -Infinity;

  constructor(
    initial: T,
    private readonly interval = PUBLISH_INTERVAL_MS,
    private readonly now: () => number = Date.now,
  ) {
    this.current = initial;
    this.published = initial;
  }

  /** The latest state, including writes not yet published. */
  peek(): T {
    return this.current;
  }

  /** The state React renders; stable between publishes. */
  getSnapshot = (): T => this.published;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /**
   * Applies `fn`; listeners hear of it at most `within` ms after the last publish (the store's
   * interval by default; FRAME_MS for a write that must show at once). A sooner write brings a
   * pending publish forward, never back.
   */
  update(fn: (state: T) => T, within = this.interval): void {
    const next = fn(this.current);
    if (next === this.current) return;
    this.current = next;
    const now = this.now();
    const due = Math.max(now, this.lastPublish + within);
    if (this.timer && this.due <= due) return;
    if (this.timer) clearTimeout(this.timer);
    this.due = due;
    this.timer = setTimeout(() => this.flush(), due - now);
  }

  /** Publishes pending writes now. */
  flush(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.due = Infinity;
    if (this.published === this.current) return;
    this.published = this.current;
    this.lastPublish = this.now();
    for (const l of [...this.listeners]) l();
  }
}
