import { describe, expect, it } from "vitest";
import { NO_ANSWER, type ConnState, type PathStatus } from "../src/protocol/client";
import type { Endpoint } from "../src/protocol/relay";
import { agoText, elapsedText, failNote, hostLine, pathRows, retrySeconds } from "../src/ui/connStatus";

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
  ...extra,
});

const NOW = 1_000_000;

// The five states of the RelayPhone board, as the client reports them.
const onLan = conn({ path: LAN, paths: [path(LAN, { state: "ok", latencyMs: 6 }), path(RELAY)] });
const searching = conn({
  status: "connecting",
  path: RELAY,
  since: NOW,
  paths: [path(LAN, { state: "failed", error: NO_ANSWER, waitedMs: 1_500 }), path(RELAY, { state: "trying" })],
});
const viaRelay = conn({ path: RELAY, paths: [path(LAN), path(RELAY, { state: "ok", latencyMs: 84 })] });
const failing = conn({
  status: "closed",
  retryInMs: 8_000,
  retryAt: NOW + 8_000,
  path: RELAY,
  paths: [
    path(LAN, { state: "failed", error: NO_ANSWER, waitedMs: 1_500 }),
    path(RELAY, { state: "failed", error: "Expected HTTP 101 response but was '404 Not Found'", reached: false }),
  ],
});
const offline = conn({ status: "offline", paths: [path(LAN), path(RELAY)] });

describe("hostLine: the status line of a computer's card", () => {
  it("Same Wi-Fi", () => {
    expect(hostLine(onLan, NOW, 4)).toMatchObject({ text: "Connected · this network · 4 tabs", tone: "muted", progress: false, retry: false, hint: null });
  });
  it("Finding it: LAN failed, relay being tried", () => {
    expect(hostLine(searching, NOW, null)).toMatchObject({ text: "Not on this network · trying the relay…", tone: "busy", progress: true, retry: false });
  });
  it("Away (relay)", () => {
    expect(hostLine(viaRelay, NOW, 4)).toMatchObject({ text: "Connected · through the relay · 4 tabs", tone: "muted" });
  });
  it("Relay down: counts down, offers Try again, explains", () => {
    expect(hostLine(failing, NOW, null)).toMatchObject({
      text: "Can’t reach it · retrying in 8 s",
      tone: "bad",
      retry: true,
      progress: false,
      hint: "Rift on your computer may be closed, asleep, or not connected to the relay. Your tabs come back when it answers.",
    });
    expect(hostLine(failing, NOW + 5_200, null).text).toBe("Can’t reach it · retrying in 3 s");
    expect(hostLine(failing, NOW + 9_000, null).text).toBe("Can’t reach it · retrying now");
  });
  it("No network", () => {
    expect(hostLine(offline, NOW, null)).toMatchObject({
      text: "Offline · this phone has no network",
      tone: "dim",
      retry: false,
      hint: "Rift reconnects as soon as this phone is back online.",
    });
  });
  it("counts one tab in the singular", () => {
    expect(hostLine(onLan, NOW, 1).text).toBe("Connected · this network · 1 tab");
  });
  it("does not mention a relay the computer does not have", () => {
    const noRelay = conn({ status: "closed", retryInMs: 500, retryAt: NOW + 500, paths: [path(LAN, { state: "failed", error: NO_ANSWER })] });
    expect(hostLine(noRelay, NOW, null).hint).toBe("Rift on your computer may be closed or asleep. Your tabs come back when it answers.");
  });
  it("looks on this network first, or goes to the relay at once when that was the last path that worked", () => {
    expect(hostLine(conn({ status: "connecting", path: LAN, paths: [path(LAN, { state: "trying" }), path(RELAY)] }), NOW, null).text).toBe("Looking on this network…");
    expect(hostLine(conn({ status: "connecting", path: RELAY, paths: [path(LAN), path(RELAY, { state: "trying" })] }), NOW, null).text).toBe("Connecting through the relay…");
  });
  it("a failure that needs the user keeps its full reason and offers no Try again for auth", () => {
    const auth = hostLine(conn({ status: "failed", failure: "auth" }), NOW, null);
    expect(auth).toMatchObject({ text: "Can’t connect to it", retry: false, hint: "This computer no longer accepts this device. Pair it again." });
    expect(hostLine(conn({ status: "failed", failure: "key_mismatch" }), NOW, null).retry).toBe(true);
  });
  it("a client held in the background says Not connected, with no countdown", () => {
    expect(hostLine(conn({ status: "closed" }), NOW, null)).toMatchObject({ text: "Not connected", retry: false });
  });
});

describe("pathRows: the Connection sheet", () => {
  const rows = (c: ConnState) => pathRows(c).map((r) => [r.title, r.addr, r.mark, r.note]);

  it("Same Wi-Fi: this network in use with its latency, the relay not needed", () => {
    expect(rows(onLan)).toEqual([
      ["This network", "192.168.0.149:7878", "✓", "In use · 6 ms"],
      ["Relay", "rift-relay.playhaws.com", "–", "Not needed on the same network"],
    ]);
  });
  it("Finding it: no answer on this network (with the wait), the relay connecting", () => {
    expect(rows(searching)).toEqual([
      ["This network", "192.168.0.149:7878", "✕", "No answer on this network (2 s)"],
      ["Relay", "rift-relay.playhaws.com", "…", "Connecting…"],
    ]);
  });
  it("Away (relay): this network not used, the relay in use with its latency", () => {
    expect(rows(viaRelay)).toEqual([
      ["This network", "192.168.0.149:7878", "–", "Not on the same network"],
      ["Relay", "rift-relay.playhaws.com", "✓", "In use · 84 ms"],
    ]);
  });
  it("Relay down: the relay refusing an unregistered gateway", () => {
    expect(rows(failing)).toEqual([
      ["This network", "192.168.0.149:7878", "–", "Not on the same network"],
      ["Relay", "rift-relay.playhaws.com", "✕", "The relay answered, but your computer is not registered there"],
    ]);
  });
  it("No network: nothing is tried", () => {
    expect(rows(offline)).toEqual([
      ["This network", "192.168.0.149:7878", "–", "Waiting for a network"],
      ["Relay", "rift-relay.playhaws.com", "–", "Waiting for a network"],
    ]);
  });
  it("shows no latency before one is measured, and never a made-up one", () => {
    const c = conn({ path: RELAY, paths: [path(LAN), path(RELAY, { state: "ok", latencyMs: null })] });
    expect(pathRows(c)[1]!.note).toBe("In use");
  });
  it("a relay that is up while the computer has not joined", () => {
    const c = conn({ status: "connecting", path: RELAY, paths: [path(LAN, { state: "failed", error: NO_ANSWER }), path(RELAY, { state: "trying", reached: true })] });
    expect(pathRows(c)[1]).toMatchObject({ mark: "…", note: "Relay up · your computer has not joined" });
  });
  it("shows the real error of a path, not a made-up cause", () => {
    const c = conn({ status: "closed", retryInMs: 1, paths: [path(LAN, { state: "failed", error: "connect ECONNREFUSED" }), path(RELAY)] });
    expect(pathRows(c)[0]).toMatchObject({ mark: "✕", note: "connect ECONNREFUSED" });
  });
  it("lists one row per LAN address", () => {
    const second: Endpoint = { kind: "lan", address: "10.0.0.5:7878" };
    const c = conn({ path: second, paths: [path(LAN), path(second, { state: "ok", latencyMs: 3 }), path(RELAY)] });
    expect(pathRows(c).map((r) => r.addr)).toEqual(["192.168.0.149:7878", "10.0.0.5:7878", "rift-relay.playhaws.com"]);
  });
});

describe("failNote", () => {
  it("a relay with no answer at all names the wait; one that opened names the missing computer", () => {
    expect(failNote(path(RELAY, { state: "failed", error: NO_ANSWER, waitedMs: 12_000 }))).toBe("No answer from the relay (12 s)");
    expect(failNote(path(RELAY, { state: "failed", error: NO_ANSWER, reached: true }))).toBe("The relay answered, but your computer is not registered there");
    expect(failNote(path(RELAY, { state: "failed", error: "the gateway did not answer", reached: true }))).toBe("The relay answered, but your computer is not registered there");
  });
  it("the relay's 404 for an absent gateway, as each platform reports it, reads as not registered", () => {
    const wording = "The relay answered, but your computer is not registered there";
    // iOS close reason and Android failure message, captured; the relay's own body is "gateway offline"
    for (const error of ["Received bad response code from server: 404.", "Expected HTTP 101 response but was '404 Not Found'", "gateway offline"])
      expect(failNote(path(RELAY, { state: "failed", error }))).toBe(wording);
    // a refused connection is the relay itself being down: its own words
    expect(failNote(path(RELAY, { state: "failed", error: "The operation couldn’t be completed. Connection refused" }))).toBe(
      "The operation couldn’t be completed. Connection refused",
    );
  });
  it("any other error is shown as it is", () => {
    expect(failNote(path(RELAY, { state: "failed", error: "Expected HTTP 101 response but was '429 Too Many Requests'" }))).toBe(
      "Expected HTTP 101 response but was '429 Too Many Requests'",
    );
    expect(failNote(path(LAN, { state: "failed", error: "gateway stopped responding" }))).toBe("gateway stopped responding");
  });
});

describe("times", () => {
  it("retrySeconds counts down from retryAt, and falls back to the delay", () => {
    expect(retrySeconds({ retryAt: NOW + 8_000, retryInMs: 8_000 }, NOW)).toBe(8);
    expect(retrySeconds({ retryAt: NOW + 8_000, retryInMs: 8_000 }, NOW + 7_001)).toBe(1);
    expect(retrySeconds({ retryAt: null, retryInMs: 2_500 }, NOW)).toBe(3);
  });
  it("elapsedText and agoText", () => {
    expect(elapsedText(3_900)).toBe("0:03");
    expect(elapsedText(65_000)).toBe("1:05");
    expect(agoText(20_000)).toBe("a moment ago");
    expect(agoText(2 * 60_000 + 1)).toBe("2 min ago");
    expect(agoText(3 * 3_600_000)).toBe("3 h ago");
    expect(agoText(2 * 86_400_000)).toBe("2 d ago");
  });
});
