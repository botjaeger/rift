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

/**
 * Authenticate before Run ("Run with Face ID"): the lock is on, and so are "Ask before running"
 * and the switch under it. Without "Ask before running" nothing is shown before a command goes,
 * so nothing asks.
 */
export function needsAuthBeforeRun(settings: Settings): boolean {
  return settings.biometricLock && settings.authBeforeRun && settings.askBeforeRun;
}

/**
 * What an alert's Yes or No needs before it answers: the app lock's own unlock while the app is
 * locked, Face ID (or the passcode) whenever the lock is on, else nothing. Answering is the app's
 * work, so it never passes the lock, whatever Ask before running says.
 */
export function alertAuth(settings: Settings, locked: boolean): "unlock" | "authenticate" | "none" {
  if (locked) return "unlock";
  return settings.biometricLock || needsAuthBeforeRun(settings) ? "authenticate" : "none";
}

/**
 * Whether a key can submit the line at a prompt, so it runs a command like Run does and asks
 * for the same authentication: Return as `enter` with any modifiers, `ctrl+m` or `ctrl+j`
 * (alt too: that is Escape then Return). Other keys do not.
 */
export function isSubmitKey(key: string): boolean {
  return /^(ctrl\+)?(alt\+)?(shift\+)?enter$/.test(key) || /^ctrl\+(alt\+)?[mj]$/.test(key);
}

/**
 * Authenticate before sending `keys`: one of them submits, and Run would ask too. `agent`: the pane
 * runs a coding agent, where ctrl+j starts a new line and submits nothing.
 */
export function keysNeedAuth(keys: readonly string[], settings: Settings, agent = false): boolean {
  return needsAuthBeforeRun(settings) && keys.some((k) => isSubmitKey(k) && !(agent && k === "ctrl+j"));
}

/**
 * The terminal view's authentication for submit keys: asked like `keysNeedAuth` says, but once
 * the user authenticated while the pane runs a program it holds until the view closes or the
 * app leaves the foreground (`end`), so Enter in vim does not ask every time. At the pane's
 * prompt a submit key runs a command, so it always asks. One per terminal view.
 */
export class SubmitAuthHold {
  private held = false;

  /** Whether sending `keys` must authenticate first; `running`: the pane runs a program. */
  needs(keys: readonly string[], settings: Settings, running: boolean): boolean {
    return !(this.held && running) && keysNeedAuth(keys, settings);
  }

  /** The user authenticated; it holds only while the pane runs a program. */
  grant(running: boolean): void {
    this.held = running;
  }

  /** The view closed or the app left the foreground. */
  end(): void {
    this.held = false;
  }
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
