// A small external store for React (useSyncExternalStore) that coalesces writes: state changes
// apply at once, listeners hear about them at most once per PUBLISH_INTERVAL_MS, so a flood of
// gateway frames does not re-render the UI per frame.

export const PUBLISH_INTERVAL_MS = 100;

export class BatchedStore<T> {
  private current: T;
  private published: T;
  private readonly listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | null = null;
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

  update(fn: (state: T) => T): void {
    const next = fn(this.current);
    if (next === this.current) return;
    this.current = next;
    if (this.timer) return;
    const wait = Math.max(0, this.lastPublish + this.interval - this.now());
    this.timer = setTimeout(() => this.flush(), wait);
  }

  /** Publishes pending writes now. */
  flush(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.published === this.current) return;
    this.published = this.current;
    this.lastPublish = this.now();
    for (const l of [...this.listeners]) l();
  }
}
