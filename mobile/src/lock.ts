// The app lock (Settings › Security): Face ID / fingerprint, with the device passcode as
// fallback, on launch and after 5 minutes in the background, and before Run when the user asks
// for it. A pure state machine; src/runtime.ts wires it to AppState and
// expo-local-authentication, the tests drive it with a fake clock.

import type { Settings } from "./stores";

export const LOCK_AFTER_BACKGROUND_MS = 5 * 60_000;

export type BiometricKind = "face" | "fingerprint" | "iris";

/**
 * The switch label for the device's biometrics ("Require Face ID", "Require fingerprint",
 * "Require Face ID or fingerprint"); null when it has none.
 */
export function biometricLabel(kinds: readonly BiometricKind[], platform: string): string | null {
  const ios = platform === "ios";
  const names: string[] = [];
  if (kinds.includes("face")) names.push(ios ? "Face ID" : "face unlock");
  if (kinds.includes("fingerprint")) names.push(ios ? "Touch ID" : "fingerprint");
  if (kinds.includes("iris")) names.push("iris");
  if (names.length === 0) return null;
  return `Require ${names.join(" or ")}`;
}

/** Authenticate before Run: the lock is on, "Ask before running commands" is on, and so is "Confirm every command". */
export function needsAuthBeforeRun(settings: Settings): boolean {
  return settings.biometricLock && settings.authBeforeRun && settings.confirmEveryCommand;
}

/**
 * Keys that submit the line at a prompt (Return as `enter`, `ctrl+m` or `ctrl+j`), so they run a
 * command like Run does: they ask for the same authentication. Other keys do not.
 */
export const SUBMIT_KEYS: ReadonlySet<string> = new Set(["enter", "ctrl+m", "ctrl+j"]);

/** Authenticate before sending `keys`: one of them submits, and Run would ask too. */
export function keysNeedAuth(keys: readonly string[], settings: Settings): boolean {
  return needsAuthBeforeRun(settings) && keys.some((k) => SUBMIT_KEYS.has(k));
}

export type AuthResult = { ok: true } | { ok: false; message: string };

/**
 * What a failed expo-local-authentication attempt (`authenticateAsync` with the passcode
 * fallback on) means for the user. `passcode_not_set` (iOS) and `not_enrolled` (Android, when
 * the device has no secure lock screen: KeyguardManager#isDeviceSecure is false) both mean
 * nothing can be checked, and a lock that can never open would only lock the owner out, so
 * the check passes. With the fallback on, iOS never reports `not_enrolled` (the passcode
 * stands in for missing biometrics).
 */
export function authOutcome(error: string): AuthResult {
  switch (error) {
    case "passcode_not_set":
    case "not_enrolled":
      return { ok: true };
    case "user_cancel":
    case "system_cancel":
    case "app_cancel":
      return { ok: false, message: "Cancelled." };
    case "lockout":
      return { ok: false, message: "Too many attempts. Unlock the device with its passcode, then try again." };
    default:
      return { ok: false, message: "Not recognized. Try again." };
  }
}

export interface LockSnapshot {
  /** The content is hidden behind the lock screen until the user authenticates. */
  locked: boolean;
  /** The app is not in the foreground: a privacy cover hides the content (app switcher snapshot). */
  covered: boolean;
}

export class AppLock {
  private enabled = false;
  private loaded = false;
  private snap: LockSnapshot = { locked: false, covered: false };
  private backgroundSince: number | null = null;
  private readonly listeners = new Set<() => void>();
  private readonly now: () => number;

  constructor(now: () => number = Date.now) {
    this.now = now;
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getSnapshot = (): LockSnapshot => this.snap;

  /**
   * The settings are known (launch) or changed. The first call locks when the lock is on;
   * turning it on later does not lock (the user just authenticated to turn it on); turning it
   * off unlocks.
   */
  setEnabled(enabled: boolean): void {
    const launch = !this.loaded;
    this.loaded = true;
    this.enabled = enabled;
    if (!enabled) this.set({ locked: false, covered: false });
    else if (launch) this.set({ ...this.snap, locked: true });
  }

  /** The app left the foreground (`inactive` on iOS before the app switcher, or `background`). */
  leave(phase: "inactive" | "background"): void {
    if (phase === "background" && this.backgroundSince === null) this.backgroundSince = this.now();
    if (this.enabled) this.set({ ...this.snap, covered: true });
  }

  /** The app is in the foreground again: locks after 5 minutes in the background. */
  enter(): void {
    const since = this.backgroundSince;
    this.backgroundSince = null;
    const expired = since !== null && this.now() - since >= LOCK_AFTER_BACKGROUND_MS;
    this.set({ locked: this.snap.locked || (this.enabled && expired), covered: false });
  }

  /** The user authenticated. */
  unlock(): void {
    this.set({ ...this.snap, locked: false });
  }

  private set(next: LockSnapshot): void {
    if (next.locked === this.snap.locked && next.covered === this.snap.covered) return;
    this.snap = next;
    for (const fn of this.listeners) fn();
  }
}
