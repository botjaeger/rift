import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BACKGROUND_GRACE_MS, Connectivity, NOTIFY_GRACE_MS } from "../src/connectivity";

/** A fake network and app state driving the machine; records what it tells the Fleet. */
function setup() {
  const calls: string[] = [];
  let pending = false;
  const c = new Connectivity(
    {
      suspend: (status) => calls.push(`suspend:${status}`),
      resume: ({ lanFirst, force }) => calls.push(`resume${lanFirst ? ":lan" : ""}${force ? ":force" : ""}`),
      hasPendingNotify: () => pending,
    },
    {
      setTimeout: (fn, ms) => setTimeout(fn, ms),
      clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
      now: () => Date.now(),
    },
  );
  const wifi = (ip = "192.168.1.50") => c.network({ connected: true, id: `WIFI|${ip}` });
  const cellular = () => c.network({ connected: true, id: "CELLULAR|10.64.0.2" });
  const none = () => c.network({ connected: false, id: "NONE|" });
  return {
    c,
    calls,
    wifi,
    cellular,
    none,
    setPending: (p: boolean) => {
      pending = p;
      c.notifyPendingChanged();
    },
  };
}

describe("Connectivity: network changes", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("the first network report changes nothing", () => {
    const t = setup();
    t.wifi();
    expect(t.calls).toEqual([]);
    expect(t.c.mode).toBe("online");
  });

  it("another network drops the sockets and reconnects at once, LAN first (cellular → home Wi-Fi)", () => {
    const t = setup();
    t.cellular();
    t.wifi();
    expect(t.calls).toEqual(["resume:lan:force"]);
    t.wifi("192.168.7.3"); // another Wi-Fi network
    expect(t.calls).toEqual(["resume:lan:force", "resume:lan:force"]);
  });

  it("the same network reported again does nothing", () => {
    const t = setup();
    t.wifi();
    t.wifi();
    t.wifi();
    expect(t.calls).toEqual([]);
  });

  it("no network: suspends as Offline once and does not spin; resumes when it returns", () => {
    const t = setup();
    t.wifi();
    t.none();
    t.none();
    expect(t.calls).toEqual(["suspend:offline"]);
    expect(t.c.mode).toBe("offline");
    vi.advanceTimersByTime(10 * 60_000);
    expect(t.calls).toEqual(["suspend:offline"]);
    t.wifi(); // the same network coming back is still a change
    expect(t.calls).toEqual(["suspend:offline", "resume:lan:force"]);
    expect(t.c.mode).toBe("online");
  });

  it("starting without a network suspends as Offline", () => {
    const t = setup();
    t.none();
    expect(t.calls).toEqual(["suspend:offline"]);
  });
});

describe("Connectivity: background and foreground", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("closes the sockets 30 s after the app goes to the background, and reconnects on foreground", () => {
    const t = setup();
    t.wifi();
    t.c.appState("background");
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS - 1);
    expect(t.calls).toEqual([]);
    expect(t.c.mode).toBe("background-grace");
    vi.advanceTimersByTime(1);
    expect(t.calls).toEqual(["suspend:closed"]);
    expect(t.c.mode).toBe("suspended");
    t.c.appState("active");
    expect(t.calls).toEqual(["suspend:closed", "resume"]);
  });

  it("coming back within 30 s keeps the sockets; foreground still reconnects anything waiting", () => {
    const t = setup();
    t.wifi();
    t.c.appState("background");
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS - 1);
    t.c.appState("active");
    vi.advanceTimersByTime(10 * 60_000);
    expect(t.calls).toEqual(["resume"]);
  });

  it("inactive (app switcher, system sheet) does not count as background", () => {
    const t = setup();
    t.c.appState("inactive");
    vi.advanceTimersByTime(10 * 60_000);
    t.c.appState("active");
    expect(t.calls).toEqual([]);
  });

  it("a pending 'Notify me' keeps the sockets up to 3 minutes", () => {
    const t = setup();
    t.wifi();
    t.setPending(true);
    t.c.appState("background");
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS);
    expect(t.calls).toEqual([]);
    vi.advanceTimersByTime(NOTIFY_GRACE_MS - BACKGROUND_GRACE_MS - 1);
    expect(t.calls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(t.calls).toEqual(["suspend:closed"]);
  });

  it("closes as soon as the notification is answered once past 30 s", () => {
    const t = setup();
    t.wifi();
    t.setPending(true);
    t.c.appState("background");
    vi.advanceTimersByTime(60_000);
    expect(t.calls).toEqual([]);
    t.setPending(false);
    expect(t.calls).toEqual(["suspend:closed"]);
  });

  it("answered within the first 30 s: closes at 30 s as usual", () => {
    const t = setup();
    t.wifi();
    t.setPending(true);
    t.c.appState("background");
    vi.advanceTimersByTime(10_000);
    t.setPending(false);
    expect(t.calls).toEqual([]);
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS - 10_000);
    expect(t.calls).toEqual(["suspend:closed"]);
  });

  it("a network change while suspended waits for the foreground, then restarts LAN first", () => {
    const t = setup();
    t.cellular();
    t.c.appState("background");
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS);
    t.wifi();
    expect(t.calls).toEqual(["suspend:closed"]);
    t.c.appState("active");
    expect(t.calls).toEqual(["suspend:closed", "resume:lan:force"]);
  });

  it("a network change during the 30 s grace reconnects at once", () => {
    const t = setup();
    t.cellular();
    t.c.appState("background");
    vi.advanceTimersByTime(5_000);
    t.wifi();
    expect(t.calls).toEqual(["resume:lan:force"]);
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS);
    expect(t.calls).toEqual(["resume:lan:force", "suspend:closed"]);
  });

  it("foreground while offline stays Offline (no retries)", () => {
    const t = setup();
    t.wifi();
    t.c.appState("background");
    vi.advanceTimersByTime(BACKGROUND_GRACE_MS);
    t.none();
    t.c.appState("active");
    expect(t.calls).toEqual(["suspend:closed", "suspend:offline"]);
    expect(t.c.mode).toBe("offline");
    t.wifi();
    expect(t.calls.at(-1)).toBe("resume:lan:force");
  });

  it("a repeated background event does not restart the 30 s", () => {
    const t = setup();
    t.c.appState("background");
    vi.advanceTimersByTime(20_000);
    t.c.appState("background");
    vi.advanceTimersByTime(10_000);
    expect(t.calls).toEqual(["suspend:closed"]);
  });
});
