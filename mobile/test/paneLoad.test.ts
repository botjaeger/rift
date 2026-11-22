import { describe, expect, it } from "vitest";
import { NO_ANSWER, type ConnState, type PathStatus } from "../src/protocol/client";
import type { Endpoint } from "../src/protocol/relay";
import { asSentence, openingTitle, paneLoad, SLOW_MS, type PaneLoadInput } from "../src/ui/paneLoad";

const LAN: Endpoint = { kind: "lan", address: "192.168.0.149:7878" };
const RELAY: Endpoint = { kind: "relay", url: "wss://rift-relay.playhaws.com" };
const path = (p: Endpoint, extra: Partial<PathStatus> = {}): PathStatus => ({
  path: p,
  state: "idle",
  error: null,
  waitedMs: null,
  latencyMs: null,
  reached: false,
  ...extra,
});
const conn = (extra: Partial<ConnState> = {}): ConnState => ({
  status: "open",
  attempt: 0,
  retryInMs: null,
  error: null,
  failure: null,
  address: null,
  path: null,
  paths: [path(LAN), path(RELAY)],
  since: null,
  retryAt: null,
  closing: null,
  ...extra,
});

const T0 = 1_000_000;
const input = (extra: Partial<PaneLoadInput> = {}): PaneLoadInput => ({
  conn: conn(),
  now: T0 + 3_000,
  waitingSince: T0,
  hostName: "Sam’s MacBook Pro",
  view: "blocks",
  opening: openingTitle("3.16.0 updates pull", null),
  paneKnown: true,
  loaded: false,
  heldSince: null,
  ...extra,
});

const reaching = conn({
  status: "connecting",
  path: RELAY,
  since: T0,
  paths: [path(LAN, { state: "failed", error: NO_ANSWER, waitedMs: 1_500 }), path(RELAY, { state: "trying" })],
});
const connectedRelay = conn({ path: RELAY, paths: [path(LAN), path(RELAY, { state: "ok", latencyMs: 84 })] });
const unregistered = conn({
  status: "closed",
  retryInMs: 8_000,
  retryAt: T0 + 3_000 + 8_000,
  path: RELAY,
  paths: [
    path(LAN, { state: "failed", error: NO_ANSWER }),
    path(RELAY, { state: "failed", error: "the gateway did not answer", reached: true }),
  ],
});

describe("paneLoad: the PaneLoading board's states", () => {
  it("Reaching: a busy strip with the wait, the first step current, the rest to do", () => {
    const l = paneLoad(input({ conn: reaching }));
    expect(l.strip).toEqual({ text: "Connecting through the relay", tone: "busy", busy: true, elapsed: "0:03" });
    expect(l.card).toEqual({
      title: "Opening 3.16.0 updates pull",
      hint: null,
      actions: false,
      steps: [
        { tone: "now", mark: "◌", label: "Reach Sam’s MacBook Pro", note: "Not on this network · through the relay" },
        { tone: "todo", mark: "○", label: "Open this pane", note: "" },
        { tone: "todo", mark: "○", label: "Load its blocks", note: "" },
      ],
    });
    expect(l.held).toBe(false);
  });

  it("Loading blocks (the New Blocks board): no strip, reached ✓, the blocks loading ◌, the screen to come ○", () => {
    const l = paneLoad(input({ conn: connectedRelay }));
    expect(l.strip).toBeNull();
    expect(l.card).toEqual({
      title: "Opening 3.16.0 updates pull",
      hint: null,
      actions: false,
      steps: [
        { tone: "done", mark: "✓", label: "Reached Sam’s MacBook Pro through the relay", note: "" },
        { tone: "now", mark: "◌", label: "Loading the pane’s blocks…", note: "" },
        { tone: "todo", mark: "○", label: "The screen", note: "" },
      ],
    });
    // on the computer's network
    const lan = conn({ status: "open", path: LAN, paths: [path(LAN, { state: "ok", latencyMs: 4 })] });
    expect(paneLoad(input({ conn: lan })).card!.steps[0]!.label).toBe("Reached Sam’s MacBook Pro on this network");
  });

  it("omits the count of blocks, which the app is not told", () => {
    const l = paneLoad(input({ conn: connectedRelay }));
    expect(JSON.stringify(l)).not.toMatch(/\d+ of \d+/);
  });

  it("Agent screen: the screen loading, titled by the program", () => {
    const l = paneLoad(input({ conn: connectedRelay, view: "screen", opening: openingTitle("rift", "claude") }));
    expect(l.card!.title).toBe("Opening claude");
    expect(l.card!.steps.slice(1)).toEqual([{ tone: "now", mark: "◌", label: "Loading the screen…", note: "" }]);
  });

  it("waits for the tab list before the pane can be opened", () => {
    const l = paneLoad(input({ conn: connectedRelay, paneKnown: false }));
    expect(l.card!.steps.map((s) => s.tone)).toEqual(["done", "now", "todo"]);
    expect(l.card!.steps[1]!.note).toBe("Waiting for the computer’s tab list");
    expect(l.card!.steps[2]!.label).toBe("The pane’s blocks");
  });

  it("Slow: after 15 s the card says so, with Try again and Details", () => {
    const l = paneLoad(input({ conn: { ...reaching, paths: [reaching.paths[0]!, path(RELAY, { state: "trying", reached: true })] }, now: T0 + SLOW_MS }));
    expect(l.strip).toMatchObject({ text: "Still connecting through the relay", elapsed: "0:15" });
    expect(l.card).toMatchObject({
      title: "Taking longer than usual",
      hint: "Rift on your computer may be asleep, closed, or still joining the relay. This keeps trying.",
      actions: true,
    });
    expect(l.card!.steps[0]!.note).toBe("Relay answered · waiting for your computer");
    // one second earlier it is not slow yet
    expect(paneLoad(input({ conn: reaching, now: T0 + SLOW_MS - 1 })).card!.title).toBe("Opening 3.16.0 updates pull");
  });

  it("a wait past an hour reads as a clock with hours, as a running block's", () => {
    expect(paneLoad(input({ conn: reaching, now: T0 + 3_600_000 })).strip!.elapsed).toBe("1:00:00");
    expect(paneLoad(input({ conn: reaching, now: T0 + 3_723_000 })).strip!.elapsed).toBe("1:02:03");
  });

  it("counts the wait from the later of the page opening and the connection starting", () => {
    // connecting since T0, the page opened 20 s later: 3 s in, not slow
    const l = paneLoad(input({ conn: reaching, waitingSince: T0 + 20_000, now: T0 + 23_000 }));
    expect(l.strip!.elapsed).toBe("0:03");
    expect(l.card!.title).not.toBe("Taking longer than usual");
  });

  it("Can’t reach: the real reason in full, the hint, Try again and Details", () => {
    const l = paneLoad(input({ conn: unregistered }));
    expect(l.strip).toEqual({ text: "Can’t reach your computer · retry 1 of 5 in 8 s", tone: "bad", busy: false, elapsed: "", retry: false });
    expect(l.card).toMatchObject({
      title: "Reconnecting to Sam’s MacBook Pro",
      actions: true,
      noRetry: true,
      leave: "Cancel",
      hint: "The relay answered, but your computer is not registered there. Rift retries 5 times, then stops and waits for you.",
    });
    expect(l.card!.steps[0]).toEqual({ tone: "now", mark: "◌", label: "Reach Sam’s MacBook Pro", note: "Retry 1 of 5 in 8 s" });
  });

  it("the Reconnecting hint states the reason as a sentence, whatever its shape", () => {
    const hint = (error: string | null) => paneLoad(input({ conn: conn({ status: "closed", retryInMs: 4000, retryAt: T0 + 4000, error, paths: [] }) })).card!.hint;
    expect(hint("connection error")).toBe("Connection error. Rift retries 5 times, then stops and waits for you.");
    expect(hint("Could not reach the computer.")).toBe("Could not reach the computer. Rift retries 5 times, then stops and waits for you.");
    expect(hint(null)).toBe("Could not reach the computer. Rift retries 5 times, then stops and waits for you.");
    expect(asSentence("  ")).toBe("");
  });

  it("Gave up: Try again, Back to Portals and Details, and no more retrying is promised", () => {
    const l = paneLoad(input({ conn: conn({ status: "failed", failure: "gave_up", attempt: 5 }) }));
    expect(l.strip).toEqual({ text: "Can’t reach Sam’s MacBook Pro", tone: "bad", busy: false, elapsed: "", retry: true });
    expect(l.card).toMatchObject({
      title: "Can’t reach Sam’s MacBook Pro",
      actions: true,
      leave: "Back to Portals",
      hint: "Rift stopped trying. Check that Sam’s MacBook Pro is on and Rift is running there, then try again.",
    });
    expect(l.card!.noRetry).toBeUndefined();
    expect(l.card!.steps).toEqual([{ tone: "bad", mark: "✕", label: "Reach Sam’s MacBook Pro", note: "No answer after 5 retries" }]);
    // Cancel sooner: just "Stopped"
    expect(paneLoad(input({ conn: conn({ status: "failed", failure: "gave_up", attempt: 1 }) })).card!.steps[0]!.note).toBe("Stopped");
  });

  it("Updating: the computer restarts Rift, so the card says when it is back, with Cancel and Details", () => {
    const closing = { reason: "update" as const, backInS: 60, at: T0 };
    const waiting = conn({ status: "closed", retryInMs: 3000, retryAt: T0 + 6000, attempt: 0, closing });
    const l = paneLoad(input({ conn: waiting }));
    expect(l.strip).toEqual({ text: "Sam’s MacBook Pro is updating Rift", tone: "busy", busy: true, elapsed: "" });
    expect(l.card).toEqual({
      title: "Sam’s MacBook Pro is updating Rift",
      steps: [
        { tone: "now", mark: "◌", label: "Reach Sam’s MacBook Pro", note: "Back in about a minute" },
        { tone: "todo", mark: "○", label: "Open this pane", note: "" },
        { tone: "todo", mark: "○", label: "Load its blocks", note: "" },
      ],
      hint: "Nothing is wrong. Your tabs come back by themselves.",
      actions: true,
      noRetry: true,
      leave: "Cancel",
    });
    // the same while a round is being tried, never a retry counter
    const trying = paneLoad(input({ conn: conn({ status: "connecting", path: LAN, since: T0, closing }) }));
    expect(trying.card!.title).toBe("Sam’s MacBook Pro is updating Rift");
    expect(JSON.stringify(trying)).not.toMatch(/retry|Can’t/);
    // later: a moment, then longer than expected (never a countdown at 0)
    expect(paneLoad(input({ conn: waiting, now: T0 + 45_000 })).card!.steps[0]!.note).toBe("Back in a moment");
    expect(paneLoad(input({ conn: waiting, now: T0 + 60_000 })).card!.steps[0]!.note).toBe("Taking a little longer than expected");
    expect(paneLoad(input({ conn: waiting, now: T0 + 170_000 })).card!.steps[0]!.note).toBe("Taking a little longer than expected");
    // blocks in memory stay shown under the strip
    expect(paneLoad(input({ conn: waiting, heldSince: T0 }))).toMatchObject({ card: null, held: true, strip: { text: "Sam’s MacBook Pro is updating Rift" } });
    // the update's wait is over: the usual wording again
    expect(paneLoad(input({ conn: conn({ ...waiting, closing: null }) })).card!.title).toBe("Reconnecting to Sam’s MacBook Pro");
  });

  it("Rift quit: says so, with Try again, Back to Portals and Details", () => {
    const l = paneLoad(input({ conn: conn({ status: "failed", failure: "host_quit", error: "Rift quit on the computer" }) }));
    expect(l.strip).toEqual({ text: "Rift quit on Sam’s MacBook Pro", tone: "quiet", busy: false, elapsed: "", retry: false });
    expect(l.card).toEqual({
      title: "Rift quit on Sam’s MacBook Pro",
      steps: [{ tone: "bad", mark: "✕", label: "Reach Sam’s MacBook Pro", note: "Rift quit on this computer" }],
      hint: "Open Rift on the computer, then tap Try again.",
      actions: true,
      leave: "Back to Portals",
    });
  });

  it("a failure that stopped the client names its reason in full", () => {
    const l = paneLoad(input({ conn: conn({ status: "failed", failure: "key_mismatch", error: "ignored" }) }));
    expect(l.strip!.text).toBe("Can’t reach Sam’s MacBook Pro");
    expect(l.card!.steps[0]!.note).toBe("A different computer answered at this address (its key does not match). Not connected.");
    expect(l.card!.leave).toBe("Back to Portals");
  });

  it("a computer that refused this device offers Details only: trying again cannot help", () => {
    expect(paneLoad(input({ conn: conn({ status: "failed", failure: "auth" }) })).card).toMatchObject({ actions: true, noRetry: true });
    expect(paneLoad(input({ conn: conn({ status: "failed", failure: "key_mismatch" }) })).card!.noRetry).toBe(false);
  });

  it("Reconnecting with blocks in memory: they are shown dimmed under a strip saying how old they are", () => {
    const l = paneLoad(input({ conn: reaching, now: T0 + 5_000, heldSince: T0 - 2 * 60_000 + 5_000 }));
    expect(l.strip).toEqual({ text: "Reconnecting · showing blocks from 2 min ago", tone: "busy", busy: true, elapsed: "0:05" });
    expect(l.card).toBeNull();
    expect(l.held).toBe(true);
  });

  it("held blocks stay shown when the computer cannot be reached or the phone is offline, with the reason in the strip", () => {
    const failed = paneLoad(input({ conn: unregistered, heldSince: T0 }));
    expect(failed).toMatchObject({ card: null, held: true, strip: { tone: "bad", text: "Can’t reach your computer · retry 1 of 5 in 8 s" } });
    const off = paneLoad(input({ conn: conn({ status: "offline" }), heldSince: T0 }));
    expect(off).toMatchObject({ card: null, held: true, strip: { text: "Offline · this phone has no network" } });
  });

  it("an offline phone with nothing held waits for a network", () => {
    const l = paneLoad(input({ conn: conn({ status: "offline" }) }));
    expect(l.card).toMatchObject({ hint: "Rift reconnects as soon as this phone is back online.", actions: false });
    expect(l.card!.steps[0]).toMatchObject({ tone: "now", note: "Waiting for a network" });
  });

  it("an open, loaded pane shows nothing", () => {
    expect(paneLoad(input({ conn: connectedRelay, loaded: true, heldSince: T0 }))).toEqual({ strip: null, card: null, held: false });
  });

  it("a connection held in the background says Not connected without a card of failures", () => {
    const l = paneLoad(input({ conn: conn({ status: "closed" }) }));
    expect(l.strip!.text).toBe("Not connected");
    expect(l.card!.actions).toBe(false);
  });

  it("openingTitle never shows the placeholder Pane", () => {
    expect(openingTitle(null, null)).toBe("Opening this pane");
    expect(openingTitle("rift", null)).toBe("Opening rift");
    expect(openingTitle("rift", "claude")).toBe("Opening claude");
  });
});
