// Over-the-air updates while the app runs. The launch check is expo-updates' own
// (`checkAutomatically: ON_LOAD`); an app left open or brought back from the background would
// otherwise only see a new update on its next cold start, so this checks again every
// OTA_CHECK_INTERVAL_MS and on every return to the foreground, downloads what it finds, and
// the root offers a restart once it is ready (UpdatePrompt).

/** How often an open app looks for a new update. */
export const OTA_CHECK_INTERVAL_MS = 15 * 60_000;
/**
 * A check that has not answered by then is reported as failed (a captive or dead network; on the
 * simulator a stalled HTTP/3 request took 60 s to fail). The request itself runs on, and the
 * native queue holds later checks behind it.
 */
export const OTA_CHECK_TIMEOUT_MS = 10_000;

/** The part of expo-updates the updater uses. */
export interface OtaUpdates {
  checkForUpdateAsync(): Promise<{ isAvailable: boolean }>;
  fetchUpdateAsync(): Promise<unknown>;
}

export type OtaOutcome = "downloaded" | "none" | "busy" | "failed";

/**
 * A check that downloads a newer update when there is one. One runs at a time: a call while
 * another is still checking or downloading returns "busy" and starts nothing.
 */
export function otaChecker(updates: OtaUpdates, timeoutMs = OTA_CHECK_TIMEOUT_MS): () => Promise<OtaOutcome> {
  let running = false;
  return async () => {
    if (running) return "busy";
    running = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error("update check timed out")), timeoutMs);
      });
      const check = await Promise.race([updates.checkForUpdateAsync(), timeout]);
      clearTimeout(timer);
      if (!check.isAvailable) return "none";
      await updates.fetchUpdateAsync();
      return "downloaded";
    } catch {
      return "failed";
    } finally {
      clearTimeout(timer);
      running = false;
    }
  };
}

/** The last check, for Settings › About: what it found and when it ended. */
export interface OtaStatus {
  state: "idle" | "checking" | Exclude<OtaOutcome, "busy">;
  /** When the last check ended (ms since the epoch), null before the first. */
  at: number | null;
}

/**
 * The app's update checks (automatic and Check now share one): runs `check` and keeps its
 * status for the screens. Without a check (development builds, the web) `run` does nothing.
 */
export class OtaChecks {
  private snap: OtaStatus = { state: "idle", at: null };
  private readonly subs = new Set<() => void>();

  constructor(
    private readonly check: (() => Promise<OtaOutcome>) | null,
    private readonly now: () => number = Date.now,
  ) {}

  /** Whether this build can update over the air at all. */
  get enabled(): boolean {
    return this.check !== null;
  }

  readonly subscribe = (f: () => void) => {
    this.subs.add(f);
    return () => {
      this.subs.delete(f);
    };
  };

  readonly getSnapshot = (): OtaStatus => this.snap;

  async run(): Promise<OtaOutcome | null> {
    if (!this.check) return null;
    const at = this.snap.at;
    this.set({ state: "checking", at });
    const r = await this.check();
    // another check is under way and reports for both
    if (r !== "busy") this.set({ state: r, at: this.now() });
    return r;
  }

  private set(s: OtaStatus) {
    this.snap = s;
    for (const f of this.subs) f();
  }
}

/**
 * Whether to offer a restart for the downloaded update `id`: once per update, and only while
 * the app is unlocked (the lock screen comes first). Records the offer in `offered`.
 */
export function shouldOfferRestart(pending: boolean, id: string | undefined, locked: boolean, offered: Set<string>): boolean {
  if (!pending || !id || locked || offered.has(id)) return false;
  offered.add(id);
  return true;
}
