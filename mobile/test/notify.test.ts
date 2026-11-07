import { describe, expect, it } from "vitest";
import {
  isNtfyTopic,
  LONG_COMMAND_MS,
  newNtfyTopic,
  normalizeNtfyServer,
  NTFY_SERVER,
  ntfyFooter,
  ntfyManualSteps,
  ntfyServerChanged,
  ntfySubscribeFallback,
  NTFY_TOPIC_LEN,
  ntfySubscribeLink,
  ntfyUrl,
  pushCovers,
  pushFooter,
  pushTokenProblem,
  pushRegistration,
  shouldNotify,
  tapTarget,
  type NotifyFacts,
  type TapResponse,
} from "../src/notify";
import { isNtfyUrl } from "../src/protocol/types";

const base: NotifyFacts = {
  appActive: true,
  viewing: null,
  host: "H",
  pane: 12,
  durationMs: LONG_COMMAND_MS,
  exit: 0,
  asked: false,
  policy: "all",
  afterMs: LONG_COMMAND_MS,
  muted: false,
};
const f = (patch: Partial<NotifyFacts>) => shouldNotify({ ...base, ...patch });

describe("shouldNotify", () => {
  it("long command, policy all, user on another screen: notify", () => {
    expect(f({})).toBe(true);
  });

  it("the user is watching that pane in the foreground: never", () => {
    expect(f({ viewing: { host: "H", pane: 12 } })).toBe(false);
    expect(f({ viewing: { host: "H", pane: 12 }, asked: true })).toBe(false);
  });

  it("the same pane id on another host, or another pane, is not watching", () => {
    expect(f({ viewing: { host: "other", pane: 12 } })).toBe(true);
    expect(f({ viewing: { host: "H", pane: 13 } })).toBe(true);
  });

  it("app in the background counts as not watching even with the pane open", () => {
    expect(f({ appActive: false, viewing: { host: "H", pane: 12 } })).toBe(true);
  });

  it("shorter than Notify after: only when asked; exactly Notify after counts", () => {
    expect(f({ durationMs: LONG_COMMAND_MS - 1 })).toBe(false);
    expect(f({ durationMs: 10, asked: true })).toBe(true);
    expect(f({ durationMs: 30_000 })).toBe(true);
    expect(f({ afterMs: 10_000, durationMs: 10_000 })).toBe(true);
    expect(f({ afterMs: 300_000, durationMs: 299_999 })).toBe(false);
  });

  it("policy off: only an explicit ask notifies", () => {
    expect(f({ policy: "off" })).toBe(false);
    expect(f({ policy: "off", asked: true })).toBe(true);
  });

  it("policy failures: an exit status other than 0 only (none reported is not a failure)", () => {
    expect(f({ policy: "failures", exit: 0 })).toBe(false);
    expect(f({ policy: "failures", exit: null })).toBe(false);
    expect(f({ policy: "failures", exit: 1 })).toBe(true);
    expect(f({ policy: "failures", exit: -9 })).toBe(true);
    expect(f({ policy: "failures", exit: 0, asked: true })).toBe(true);
    expect(f({ policy: "all", exit: 0 })).toBe(true);
  });

  it("a muted computer notifies only for an explicit ask", () => {
    expect(f({ muted: true, exit: 1 })).toBe(false);
    expect(f({ muted: true, asked: true })).toBe(true);
  });
});

describe("pushRegistration", () => {
  it("follows the same rules as the app's own notifications", () => {
    expect(pushRegistration("off", 30_000, false)).toEqual({ longMs: null, failuresOnly: false });
    expect(pushRegistration("all", 30_000, false)).toEqual({ longMs: 30_000, failuresOnly: false });
    expect(pushRegistration("failures", 10_000, false)).toEqual({ longMs: 10_000, failuresOnly: true });
    expect(pushRegistration("all", 60_000, true)).toEqual({ longMs: null, failuresOnly: false });
    expect(pushRegistration("failures", 60_000, true)).toEqual({ longMs: null, failuresOnly: true });
    // a gateway without `push_failures` would alert for successes: no long_ms there
    expect(pushRegistration("failures", 10_000, false, false)).toEqual({ longMs: null, failuresOnly: true });
    expect(pushRegistration("all", 10_000, false, false)).toEqual({ longMs: 10_000, failuresOnly: false });
  });
});

describe("pushCovers (when a push host alerts by itself)", () => {
  const done = (durationMs: number, exit: number | null, askSent = false) => ({ askSent, durationMs, exit });
  it("a Notify me the computer received, whatever the registration", () => {
    expect(pushCovers({ longMs: null, failuresOnly: false }, done(1, 0, true))).toBe(true);
  });
  it("a long command by long_ms, failed only for failures_only", () => {
    const all = { longMs: 30_000, failuresOnly: false };
    const failures = { longMs: 30_000, failuresOnly: true };
    expect(pushCovers(all, done(30_000, 0))).toBe(true);
    expect(pushCovers(all, done(29_999, 1))).toBe(false);
    expect(pushCovers(failures, done(60_000, 2))).toBe(true);
    expect(pushCovers(failures, done(60_000, 0))).toBe(false);
    expect(pushCovers(failures, done(60_000, null))).toBe(false);
  });
  it("no long_ms (policy off, muted, or a gateway without push_failures): only a received Notify me", () => {
    const none = pushRegistration("failures", 10_000, false, false);
    expect(pushCovers(none, done(600_000, 2))).toBe(false);
    expect(pushCovers(none, done(600_000, 2, true))).toBe(true);
  });
  it("ntfy only: not the followed pane (the computer sends no ntfy banner about it) unless asked", () => {
    const ntfy = { longMs: 30_000, failuresOnly: false, ntfyOnly: true };
    expect(pushCovers(ntfy, { ...done(60_000, 2), followed: true })).toBe(false);
    expect(pushCovers(ntfy, { ...done(60_000, 2), followed: false })).toBe(true);
    expect(pushCovers(ntfy, { ...done(60_000, 2, true), followed: true })).toBe(true);
    // with a token the computer still sends it (through Expo)
    expect(pushCovers({ longMs: 30_000, failuresOnly: false }, { ...done(60_000, 2), followed: true })).toBe(true);
  });
});

describe("tapTarget", () => {
  const tap = (identifier: string, data: unknown = { host: "H", pane: 12 }): TapResponse => ({
    notification: { request: { identifier, content: { data } } },
  });

  it("opens each tapped notification once, however often it is delivered", () => {
    const handled = new Set<string>();
    expect(tapTarget(tap("n1"), handled)).toEqual({ host: "H", pane: 12 });
    expect(tapTarget(tap("n1"), handled)).toBeNull(); // the launch response after the listener
    expect(tapTarget(tap("n2"), handled)).toEqual({ host: "H", pane: 12 });
  });

  it("ignores no response and notifications without a pane", () => {
    const handled = new Set<string>();
    expect(tapTarget(null, handled)).toBeNull();
    expect(tapTarget(tap("x", { host: "H" }), handled)).toBeNull();
    expect(tapTarget(tap("y", null), handled)).toBeNull();
  });
});

describe("ntfy", () => {
  it("a new topic is rift- and 22 base64url characters of fresh randomness", () => {
    let n = 0;
    const counter = (len: number) => new Uint8Array(len).fill(n++);
    const a = newNtfyTopic(counter);
    const b = newNtfyTopic(counter);
    expect(a).toMatch(/^rift-[A-Za-z0-9_-]{22}$/);
    expect(a).not.toBe(b);
    expect(isNtfyTopic(a)).toBe(true);
    expect(isNtfyUrl(`${NTFY_SERVER}/${a}`)).toBe(true);
    for (const bad of ["", "a b", "a/b", "x".repeat(65), "tópico"]) expect(isNtfyTopic(bad), bad).toBe(false);
  });

  it("a server as typed becomes https://host[:port], or null when the gateway would refuse it", () => {
    expect(normalizeNtfyServer("ntfy.example.com")).toBe("https://ntfy.example.com");
    expect(normalizeNtfyServer("  HTTPS://Ntfy.Example.com:8443/ ")).toBe("https://ntfy.example.com:8443");
    expect(normalizeNtfyServer("https://ntfy.sh")).toBe(NTFY_SERVER);
    for (const bad of ["", "http://ntfy.example.com", "ntfy://ntfy.sh", "https://ntfy.sh/path", "https://u:p@ntfy.sh", "https://ntfy.sh?x", "https://ntfy.sh:99999"]) {
      expect(normalizeNtfyServer(bad), bad).toBeNull();
    }
  });

  it("the topic URL, the subscribe link and the manual steps", () => {
    expect(ntfyUrl({ ntfyServer: NTFY_SERVER, ntfyEnabled: true, ntfyTopic: null })).toBeNull();
    expect(ntfyUrl({ ntfyServer: NTFY_SERVER, ntfyEnabled: false, ntfyTopic: "rift-a" })).toBeNull();
    expect(ntfyUrl({ ntfyServer: NTFY_SERVER, ntfyEnabled: true, ntfyTopic: "rift-a" })).toBe("https://ntfy.sh/rift-a");
    expect(ntfyUrl({ ntfyServer: "http://x", ntfyEnabled: true, ntfyTopic: "rift-a" })).toBeNull();
    expect(ntfySubscribeLink(NTFY_SERVER, "rift-a")).toBe("ntfy://ntfy.sh/rift-a");
    expect(ntfySubscribeLink("https://n.example.com:8443", "rift-a")).toBe("ntfy://n.example.com:8443/rift-a");
    expect(ntfyManualSteps(NTFY_SERVER)).toBe(
      "The topic is copied. In the ntfy app, tap +, paste it as the topic, then tap Subscribe.",
    );
    expect(ntfyManualSteps("https://n.example.com")).toBe(
      "The topic is copied. In the ntfy app, tap +, paste it as the topic, turn on “Use another server” and enter https://n.example.com, then tap Subscribe.",
    );
  });

  it("the footer says who sees what (the Settings board's words), the command line only when shown", () => {
    expect(ntfyFooter(NTFY_SERVER)).toBe("ntfy.sh sees each alert's computer name, exit status and a link to the pane. Never output.");
    expect(ntfyFooter(NTFY_SERVER, true)).toBe(
      "ntfy.sh sees each alert's computer name, exit status, a link to the pane and, with “Show the command”, the command line. Never output.",
    );
    expect(ntfyFooter("https://n.example.com")).toContain("n.example.com sees");
  });

  it("a server near the length limit is refused when a topic would not fit after it (as the gateway checks)", () => {
    // 200-character URL limit: "https://" + host + "/" + 27-character topic
    const fits = `${"h".repeat(160)}.com`; // 8 + 164 + 1 + 27 = 200
    const long = `${"h".repeat(161)}.com`;
    const s = normalizeNtfyServer(fits)!;
    expect(s).not.toBeNull();
    expect(ntfyUrl({ ntfyServer: s, ntfyEnabled: true, ntfyTopic: newNtfyTopic((n) => new Uint8Array(n)) })).not.toBeNull();
    expect(normalizeNtfyServer(long)).toBeNull();
    expect(newNtfyTopic((n) => new Uint8Array(n))).toHaveLength(NTFY_TOPIC_LEN);
  });

  it("subscribe fallback: install on Android, copy and steps on iPhone; a server change says subscribe again", () => {
    expect(ntfySubscribeFallback("android", NTFY_SERVER)).toEqual({
      copy: false,
      text: "Install ntfy from Google Play or F-Droid first, then tap Subscribe in ntfy again.",
    });
    expect(ntfySubscribeFallback("ios", NTFY_SERVER)).toEqual({ copy: true, text: ntfyManualSteps(NTFY_SERVER) });
    expect(ntfyServerChanged("https://n.example.com")).toBe(
      "Subscribe again in ntfy: the topic is the same, but it is now on n.example.com.",
    );
  });
});

describe("pushTokenProblem", () => {
  it("never shows the system's raw error", () => {
    // captured from an Android 15 emulator build without google-services.json
    const firebase =
      "Unable to get Firebase Messaging instance. Did you configure `googleServicesFile` path in app config? Make sure to complete the guide at https://docs.expo.dev/push-notifications/fcm-credentials/ : Default FirebaseApp is not initialized in this process dev.botjaeger.rift. Make sure to call FirebaseApp.initializeApp(Context) first.";
    expect(pushTokenProblem(firebase)).toBe("this build is not set up for Android push.");
    expect(pushTokenProblem("SERVICE_NOT_AVAILABLE")).toBe("this phone gave no push token.");
    for (const d of [firebase, "SERVICE_NOT_AVAILABLE"]) {
      expect(pushTokenProblem(d)).not.toMatch(/https?:|java|Firebase|SERVICE/);
      // reads on in the footer's sentence
      expect(pushFooter({ state: "unavailable", reason: pushTokenProblem(d) })).toMatch(/^Without it, notifications come only while this app is open: [a-z].*\.$/);
    }
  });
});

describe("pushFooter", () => {
  it("with ntfy on: ntfy delivery, no push error and no 'while it is open'", () => {
    const f = pushFooter({ state: "unavailable", reason: "The system gave no push token (x)." }, true);
    expect(f).toBe("While the app is closed, your computers send these through the ntfy app.");
  });

  it("says where notifications come from and what push carries", () => {
    expect(pushFooter({ state: "on" })).toContain("only with “Show the command”, the command line. Never output.");
    expect(pushFooter({ state: "off" })).toContain("while it is open");
    const unavailable = pushFooter({ state: "unavailable", reason: "this build has no push project." });
    // the While the app is closed board's footer, under Alerts through ntfy ("it")
    expect(unavailable).toBe("Without it, notifications come only while this app is open: this build has no push project.");
    expect(pushFooter({ state: "unavailable", reason: pushTokenProblem("SERVICE_NOT_AVAILABLE") })).toBe(
      "Without it, notifications come only while this app is open: this phone gave no push token.",
    );
  });
});
