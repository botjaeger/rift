import { describe, expect, it } from "vitest";
import {
  AppLock,
  LOCK_AFTER_BACKGROUND_MS,
  SubmitAuthHold,
  authOutcome,
  biometricLabel,
  isSubmitKey,
  keysNeedAuth,
  needsAuthBeforeRun,
} from "../src/lock";
import { DEFAULT_SETTINGS, MemoryStore, loadSettings, saveSettings } from "../src/stores";

function clock() {
  let t = 1_000_000;
  return { now: () => t, advance: (ms: number) => (t += ms) };
}

describe("AppLock timing", () => {
  it("locks at launch when the lock is on, and not when it is off", () => {
    const on = new AppLock();
    on.setEnabled(true);
    expect(on.getSnapshot().locked).toBe(true);
    const off = new AppLock();
    off.setEnabled(false);
    expect(off.getSnapshot().locked).toBe(false);
  });

  it("locks after 5 minutes in the background, not before", () => {
    const c = clock();
    const lock = new AppLock(c.now);
    lock.setEnabled(true);
    lock.unlock();
    lock.leave("background");
    c.advance(LOCK_AFTER_BACKGROUND_MS - 1);
    lock.enter();
    expect(lock.getSnapshot().locked).toBe(false);
    lock.leave("background");
    c.advance(LOCK_AFTER_BACKGROUND_MS);
    lock.enter();
    expect(lock.getSnapshot().locked).toBe(true);
  });

  it("counts from the background, not from inactive (app switcher) moments", () => {
    const c = clock();
    const lock = new AppLock(c.now);
    lock.setEnabled(true);
    lock.unlock();
    lock.leave("inactive");
    c.advance(10 * 60_000);
    lock.enter();
    expect(lock.getSnapshot().locked).toBe(false);
    lock.leave("inactive");
    lock.leave("background");
    c.advance(LOCK_AFTER_BACKGROUND_MS);
    lock.leave("background"); // a repeated event keeps the first time
    lock.enter();
    expect(lock.getSnapshot().locked).toBe(true);
  });

  it("covers the content whenever the app leaves the foreground, with the lock on only", () => {
    const lock = new AppLock();
    lock.setEnabled(true);
    lock.unlock();
    lock.leave("inactive");
    expect(lock.getSnapshot()).toEqual({ locked: false, covered: true });
    lock.enter();
    expect(lock.getSnapshot()).toEqual({ locked: false, covered: false });

    const off = new AppLock();
    off.setEnabled(false);
    off.leave("background");
    expect(off.getSnapshot()).toEqual({ locked: false, covered: false });
  });

  it("turning the lock on later does not lock; turning it off unlocks", () => {
    const c = clock();
    const lock = new AppLock(c.now);
    lock.setEnabled(false);
    lock.setEnabled(true);
    expect(lock.getSnapshot().locked).toBe(false);
    lock.leave("background");
    c.advance(LOCK_AFTER_BACKGROUND_MS);
    lock.enter();
    expect(lock.getSnapshot().locked).toBe(true);
    lock.setEnabled(false);
    expect(lock.getSnapshot().locked).toBe(false);
  });

  it("with the lock off, 5 minutes away never locks", () => {
    const c = clock();
    const lock = new AppLock(c.now);
    lock.setEnabled(false);
    lock.leave("background");
    c.advance(60 * 60_000);
    lock.enter();
    expect(lock.getSnapshot().locked).toBe(false);
  });

  it("notifies subscribers only on a change", () => {
    const lock = new AppLock();
    let n = 0;
    lock.subscribe(() => n++);
    lock.setEnabled(true);
    lock.setEnabled(true);
    lock.unlock();
    lock.unlock();
    expect(n).toBe(2);
  });
});

describe("authentication before Run", () => {
  it("needs the lock, 'Run with Face ID' and 'Ask before running'", () => {
    const s = { ...DEFAULT_SETTINGS, biometricLock: true, askBeforeRun: true };
    expect(needsAuthBeforeRun(s)).toBe(true);
    expect(needsAuthBeforeRun(DEFAULT_SETTINGS)).toBe(false); // lock off by default
    expect(needsAuthBeforeRun({ ...s, authBeforeRun: false })).toBe(false);
    // Ask before running is off by default: nothing is shown before Run, so nothing asks
    expect(needsAuthBeforeRun({ ...s, askBeforeRun: false })).toBe(false);
    expect(needsAuthBeforeRun({ ...DEFAULT_SETTINGS, biometricLock: true })).toBe(false);
  });

  it("settings: the lock defaults off, asking before Run defaults on, both persist", async () => {
    expect(DEFAULT_SETTINGS).toMatchObject({ biometricLock: false, authBeforeRun: true });
    const kv = new MemoryStore();
    expect(await loadSettings(kv)).toMatchObject({ biometricLock: false, authBeforeRun: true });
    await saveSettings(kv, { ...DEFAULT_SETTINGS, biometricLock: true, authBeforeRun: false });
    expect(await loadSettings(kv)).toMatchObject({ biometricLock: true, authBeforeRun: false });
    // Settings saved before Phase 2 get the defaults.
    await kv.set("rift.settings", JSON.stringify({ confirmEveryCommand: false }));
    expect(await loadSettings(kv)).toMatchObject({ askBeforeRun: false, biometricLock: false, authBeforeRun: true });
  });
});

describe("biometricLabel", () => {
  it.each([
    [["face"], "ios", "Require Face ID"],
    [["fingerprint"], "ios", "Require Touch ID"],
    [["fingerprint"], "android", "Require fingerprint"],
    [["face", "fingerprint"], "android", "Require face unlock or fingerprint"],
    [["iris"], "android", "Require iris"],
  ] as const)("%j on %s → %s", (kinds, platform, label) => {
    expect(biometricLabel(kinds, platform)).toBe(label);
  });

  it("is null without biometrics", () => {
    expect(biometricLabel([], "ios")).toBeNull();
  });
});

describe("authOutcome", () => {
  it("lets the owner in when the device has nothing to check: no passcode (iOS), no secure lock screen (Android)", () => {
    expect(authOutcome("passcode_not_set")).toEqual({ ok: true });
    expect(authOutcome("not_enrolled")).toEqual({ ok: true });
  });

  it("reports a cancel, a lockout, and anything else as a failure to retry", () => {
    for (const e of ["user_cancel", "system_cancel", "app_cancel"]) expect(authOutcome(e)).toEqual({ ok: false, message: "Cancelled." });
    expect(authOutcome("lockout")).toMatchObject({ ok: false, message: expect.stringContaining("Too many attempts") });
    for (const e of ["authentication_failed", "not_available", "unknown"]) {
      expect(authOutcome(e)).toEqual({ ok: false, message: "Not recognized. Try again." });
    }
  });
});

describe("keysNeedAuth", () => {
  const on = { ...DEFAULT_SETTINGS, biometricLock: true, authBeforeRun: true, askBeforeRun: true };

  it("asks before keys that submit a line, when Run would ask", () => {
    for (const k of ["enter", "ctrl+m", "ctrl+j"]) expect(keysNeedAuth([k], on)).toBe(true);
    expect(keysNeedAuth(["up", "ctrl+m"], on)).toBe(true);
  });

  it("never for other keys, nor when Run would not ask", () => {
    for (const k of ["ctrl+c", "esc", "tab", "up", "ctrl+d", "alt+m", "f5", "ctrl+space"])
      expect(keysNeedAuth([k], on)).toBe(false);
    expect(keysNeedAuth(["enter"], DEFAULT_SETTINGS)).toBe(false);
    expect(keysNeedAuth(["enter"], { ...on, askBeforeRun: false })).toBe(false);
  });

  it("Return with any modifiers submits too (the Phase 3 key names)", () => {
    for (const k of ["shift+enter", "ctrl+enter", "alt+enter", "ctrl+alt+shift+enter", "ctrl+alt+m", "ctrl+alt+j"])
      expect(isSubmitKey(k)).toBe(true);
  });
});

describe("SubmitAuthHold (the terminal view)", () => {
  const on = { ...DEFAULT_SETTINGS, biometricLock: true, authBeforeRun: true, askBeforeRun: true };

  it("while a program runs: asks for the first submit key, then holds until the view closes or the app leaves", () => {
    const hold = new SubmitAuthHold();
    expect(hold.needs(["up"], on, true)).toBe(false);
    expect(hold.needs(["enter"], on, true)).toBe(true);
    hold.grant(true);
    expect(hold.needs(["enter"], on, true)).toBe(false);
    expect(hold.needs(["ctrl+m"], on, true)).toBe(false);
    hold.end();
    expect(hold.needs(["enter"], on, true)).toBe(true);
  });

  it("at the pane's prompt a submit key runs a command: it always asks", () => {
    const hold = new SubmitAuthHold();
    hold.grant(true); // authenticated in vim
    expect(hold.needs(["enter"], on, false)).toBe(true); // vim quit: the shell's prompt
    expect(hold.needs(["up"], on, false)).toBe(false);
    expect(hold.needs(["enter"], on, true)).toBe(false); // a program again: still held
    const prompt = new SubmitAuthHold();
    prompt.grant(false); // authenticated at the prompt: no hold
    expect(prompt.needs(["enter"], on, false)).toBe(true);
    expect(prompt.needs(["enter"], on, true)).toBe(true);
  });

  it("never asks when Run would not, and each view has its own", () => {
    expect(new SubmitAuthHold().needs(["enter"], DEFAULT_SETTINGS, false)).toBe(false);
    const a = new SubmitAuthHold();
    a.grant(true);
    expect(new SubmitAuthHold().needs(["enter"], on, true)).toBe(true);
  });
});
