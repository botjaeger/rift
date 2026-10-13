// When the app's gateway connections may be open (docs/mobile-remote-plan.md, Phase 2: changing
// networks, battery). A pure state machine over two event sources, the network and the app
// state, that tells the Fleet to suspend or resume its connections. Free of React Native
// imports: src/runtime.ts feeds it expo-network and AppState, the tests feed it fakes.
//
// Rules:
// - The network changes (another network, or back from none): drop every socket and connect
//   again at once with the backoff reset, LAN addresses first, so moving from cellular (relay)
//   to the home Wi-Fi goes back to the direct path.
// - No network: close every socket and show Offline; nothing is retried until it returns.
// - The app goes to the background: sockets close 30 s later (battery). A pending "Notify me"
//   keeps them up to 3 minutes, and they close as soon as it is answered after the first 30 s.
// - The app comes to the foreground: connect now (backoff reset). Sockets that are still open
//   stay; after a network change while away, everything restarts LAN first.

export const BACKGROUND_GRACE_MS = 30_000;
export const NOTIFY_GRACE_MS = 3 * 60_000;

export interface NetworkSnapshot {
  /** Any network at all (Wi-Fi, cellular, …). */
  connected: boolean;
  /** Identifies the network (type and address); a different id is a different network. */
  id: string;
}

export type AppPhase = "active" | "inactive" | "background";

export interface ConnectivityActions {
  /** Close every socket and hold them closed; `offline` shows "Offline". */
  suspend(status: "offline" | "closed"): void;
  /** Connect again now, backoff reset; `force` drops open sockets too; `lanFirst` tries LAN first. */
  resume(opts: { lanFirst: boolean; force: boolean }): void;
  /** A "Notify me" is waiting for its command to finish. */
  hasPendingNotify(): boolean;
}

export interface Timers {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
  now(): number;
}

const defaultTimers: Timers = {
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  now: () => Date.now(),
};

export type ConnectivityMode = "online" | "offline" | "background-grace" | "suspended";

export class Connectivity {
  private readonly actions: ConnectivityActions;
  private readonly timers: Timers;
  /** null until the first network snapshot (assumed online). */
  private online: boolean | null = null;
  private networkId: string | null = null;
  private foreground = true;
  private backgroundSince = 0;
  private timer: unknown = null;
  /** The sockets were closed because the app is in the background. */
  private suspended = false;
  /** The network changed (or came back) while suspended: restart LAN first on foreground. */
  private changedWhileAway = false;

  constructor(actions: ConnectivityActions, timers: Timers = defaultTimers) {
    this.actions = actions;
    this.timers = timers;
  }

  get mode(): ConnectivityMode {
    if (this.online === false) return "offline";
    if (this.suspended) return "suspended";
    return this.foreground ? "online" : "background-grace";
  }

  network(n: NetworkSnapshot): void {
    if (!n.connected) {
      if (this.online === false) return;
      this.online = false;
      this.actions.suspend("offline");
      return;
    }
    const first = this.online === null;
    const changed = this.online === false || (this.networkId !== null && this.networkId !== n.id);
    this.online = true;
    this.networkId = n.id;
    if (first || !changed) return;
    if (this.suspended) {
      this.changedWhileAway = true;
      return;
    }
    this.actions.resume({ lanFirst: true, force: true });
  }

  appState(phase: AppPhase): void {
    // `inactive` is transient on iOS (app switcher, a system sheet): sockets stay as they are.
    if (phase === "inactive") return;
    if (phase === "background") {
      if (!this.foreground) return;
      this.foreground = false;
      this.backgroundSince = this.timers.now();
      this.arm(BACKGROUND_GRACE_MS);
      return;
    }
    if (this.foreground) return;
    this.foreground = true;
    this.disarm();
    const lanFirst = this.changedWhileAway;
    this.suspended = false;
    this.changedWhileAway = false;
    // Still no network: the hosts already show Offline (a background close keeps that label).
    if (this.online === false) return;
    this.actions.resume({ lanFirst, force: lanFirst });
  }

  /** The Fleet's pending "Notify me" list changed. */
  notifyPendingChanged(): void {
    if (this.foreground || this.suspended || this.actions.hasPendingNotify()) return;
    if (this.timers.now() - this.backgroundSince >= BACKGROUND_GRACE_MS) this.suspendNow();
  }

  private arm(ms: number): void {
    this.disarm();
    this.timer = this.timers.setTimeout(() => {
      this.timer = null;
      this.onGraceEnd();
    }, ms);
  }

  private disarm(): void {
    if (this.timer !== null) this.timers.clearTimeout(this.timer);
    this.timer = null;
  }

  private onGraceEnd(): void {
    if (this.foreground || this.suspended) return;
    const elapsed = this.timers.now() - this.backgroundSince;
    if (this.actions.hasPendingNotify() && elapsed < NOTIFY_GRACE_MS) {
      this.arm(NOTIFY_GRACE_MS - elapsed);
      return;
    }
    this.suspendNow();
  }

  private suspendNow(): void {
    this.disarm();
    this.suspended = true;
    // Offline already closed everything and says so; keep that label.
    if (this.online !== false) this.actions.suspend("closed");
  }
}
