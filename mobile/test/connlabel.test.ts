import { describe, expect, it } from "vitest";
import type { ConnState } from "../src/protocol/client";
import { describeConn } from "../src/ui/connLabel";

const base: ConnState = { status: "open", attempt: 0, retryInMs: null, error: null, failure: null, address: null, path: null, paths: [], since: null, retryAt: null, closing: null };

describe("describeConn: the path in host status and the pane banner", () => {
  it("names the LAN address or the relay host", () => {
    expect(describeConn({ ...base, path: { kind: "lan", address: "192.168.1.29:7878" } }).label).toBe("Connected · LAN 192.168.1.29");
    expect(describeConn({ ...base, path: { kind: "relay", url: "wss://relay.example.com" } }).label).toBe(
      "Connected · Relay relay.example.com",
    );
    expect(
      describeConn({ ...base, status: "connecting", attempt: 2, path: { kind: "relay", url: "wss://relay.example.com" } }).label,
    ).toBe("Reconnecting (attempt 3) · Relay relay.example.com…");
  });

  it("says Offline without a retry countdown", () => {
    expect(describeConn({ ...base, status: "offline", path: { kind: "lan", address: "10.0.0.5:7878" } }).label).toBe(
      "Offline: this device has no network",
    );
  });

  it("an update's wait is calm, and Rift quitting says what to do", () => {
    const closing = { reason: "update" as const, backInS: 60, at: 0 };
    expect(describeConn({ ...base, status: "closed", retryInMs: 3000, closing }).label).toBe(
      "Rift is updating on the computer · reconnecting by itself",
    );
    expect(describeConn({ ...base, status: "connecting", closing }).label).toBe("Rift is updating on the computer · reconnecting by itself");
    expect(describeConn({ ...base, status: "failed", failure: "host_quit" }).label).toBe(
      "Rift quit on this computer. Open Rift there, then tap Try again.",
    );
  });
});
