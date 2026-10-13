import { describe, expect, it } from "vitest";
import {
  LONG_COMMAND_MS,
  notificationText,
  pushFooter,
  shouldNotify,
  tapTarget,
  type NotifyFacts,
  type TapResponse,
} from "../src/notify";

const base: NotifyFacts = {
  appActive: true,
  viewing: null,
  host: "H",
  pane: 12,
  durationMs: LONG_COMMAND_MS,
  asked: false,
  enabled: true,
};
const f = (patch: Partial<NotifyFacts>) => shouldNotify({ ...base, ...patch });

describe("shouldNotify", () => {
  it("long command, enabled, user on another screen: notify", () => {
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

  it("short command: only when asked", () => {
    expect(f({ durationMs: LONG_COMMAND_MS - 1 })).toBe(false);
    expect(f({ durationMs: 10, asked: true })).toBe(true);
  });

  it("30 s exactly counts as long", () => {
    expect(f({ durationMs: 30_000 })).toBe(true);
  });

  it("notifications off: only an explicit ask notifies", () => {
    expect(f({ enabled: false })).toBe(false);
    expect(f({ enabled: false, asked: true })).toBe(true);
  });
});

describe("notificationText", () => {
  it("finished, failed, unknown exit; the command only when the setting is on", () => {
    expect(notificationText("studio-mac", 0, "cargo test", false)).toEqual({ title: "A command finished on studio-mac", body: "" });
    expect(notificationText("studio-mac", 101, "cargo test", false)).toEqual({
      title: "A command failed on studio-mac (exit 101)",
      body: "",
    });
    expect(notificationText("studio-mac", null, "cargo test", false).title).toBe("A command finished on studio-mac");
    expect(notificationText("studio-mac", 0, "cargo test", true).body).toBe("cargo test");
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

describe("pushFooter", () => {
  it("says where notifications come from and what push carries", () => {
    expect(pushFooter({ state: "on" })).toContain("only with the switch above, the command line. Never output.");
    expect(pushFooter({ state: "off" })).toContain("while it is open");
    expect(pushFooter({ state: "unavailable", reason: "No push here." })).toBe(
      "Notifications come from this app while it is open. No push here.",
    );
  });
});
