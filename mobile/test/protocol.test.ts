import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  commandProblem,
  decodeServer,
  isClientHello,
  isClientMsg,
  isGatewayHello,
  parseServerFrame,
  type ClientMsg,
} from "../src/protocol/types";

const fixtures = fileURLToPath(new URL("../../crates/rift-remote/fixtures/", import.meta.url));
const load = (dir: string) =>
  readdirSync(join(fixtures, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => ({
      name: f,
      json: JSON.parse(readFileSync(join(fixtures, dir, f), "utf8")) as unknown,
    }));

const server = load("server");
const client = load("client");
const handshake = load("handshake");

describe("server fixtures", () => {
  it("covers every gateway message type", () => {
    const types = new Set(server.map((f) => (f.json as { t: string }).t));
    expect([...types].sort()).toEqual([
      "ack",
      "done",
      "error",
      "older",
      "pong",
      "snapshot",
      "topology",
      "update",
      "welcome",
    ]);
  });

  it.each(server.map((f) => [f.name, f.json] as const))("decodes %s", (_name, json) => {
    const msg = decodeServer(json);
    expect(msg).not.toBeNull();
    expect(msg).toEqual(json);
  });
});

describe("client fixtures", () => {
  it("covers every client message type", () => {
    const types = new Set(client.map((f) => (f.json as { t: string }).t));
    expect([...types].sort()).toEqual([
      "forget",
      "keys",
      "notify",
      "older",
      "ping",
      "push",
      "run",
      "subscribe",
      "unsubscribe",
    ]);
  });

  it.each(client.map((f) => [f.name, f.json] as const))("%s is a ClientMsg", (_name, json) => {
    expect(isClientMsg(json)).toBe(true);
    // Round-trips through the static type the client sends.
    const typed: ClientMsg = json as ClientMsg;
    expect(JSON.parse(JSON.stringify(typed))).toEqual(json);
  });
});

describe("handshake fixtures", () => {
  it("gateway.json is a gateway hello", () => {
    const g = handshake.find((f) => f.name === "gateway.json")!.json;
    expect(isGatewayHello(g)).toBe(true);
    expect(g).toEqual({
      v: 2,
      host: "studio-mac",
      os: "macos",
      version: "0.1.0",
    });
  });

  it.each(["client.json", "client-pair.json"])("%s is a client hello", (name) => {
    const c = handshake.find((f) => f.name === name)!.json;
    expect(isClientHello(c)).toBe(true);
  });

  it("rejects malformed hellos", () => {
    expect(isGatewayHello({ v: 2, host: "h", os: "macos" })).toBe(false);
    expect(isClientHello({ v: 2, name: "n", platform: "ios", pair: 5 })).toBe(false);
    expect(isClientHello({ v: "2", name: "n", platform: "ios" })).toBe(false);
  });
});

describe("client message checks", () => {
  it.each([
    ["run with a newline", { t: "run", id: "r1", pane: 1, command: "ls\nrm -rf /" }],
    ["run with an escape", { t: "run", id: "r1", pane: 1, command: "ls\u001b[2J" }],
    ["run with an empty command", { t: "run", id: "r1", pane: 1, command: "  " }],
    ["run over 2 KiB", { t: "run", id: "r1", pane: 1, command: "x".repeat(2049) }],
    ["run with a 65-char id", { t: "run", id: "r".repeat(65), pane: 1, command: "ls" }],
    ["keys with an unknown key", { t: "keys", id: "k1", pane: 1, keys: ["ctrl+1"] }],
    ["keys with no keys", { t: "keys", id: "k1", pane: 1, keys: [] }],
    ["keys with 21 keys", { t: "keys", id: "k1", pane: 1, keys: Array(21).fill("up") }],
    ["the v1 hello", { t: "hello", v: 1, token: "x" }],
    ["push with an FCM token", { t: "push", token: "fcm:abc", long_ms: null, show_command: false }],
    [
      "push with an empty token",
      {
        t: "push",
        token: "ExponentPushToken[]",
        long_ms: null,
        show_command: false,
      },
    ],
    [
      "push over a day",
      {
        t: "push",
        token: "ExpoPushToken[a]",
        long_ms: 86_400_001,
        show_command: false,
      },
    ],
    ["push without show_command", { t: "push", token: null, long_ms: null }],
    ["notify without a pane", { t: "notify" }],
  ])("rejects %s", (_name, msg) => {
    expect(isClientMsg(msg)).toBe(false);
  });

  it("commandProblem names the problem", () => {
    expect(commandProblem("cargo test")).toBeNull();
    expect(commandProblem("x".repeat(2048))).toBeNull();
    expect(commandProblem("é".repeat(1025))).toBe("too_long"); // 2050 UTF-8 bytes
    expect(commandProblem("a\tb")).toBe("control");
    expect(commandProblem("")).toBe("empty");
  });
});

describe("decodeServer rejects", () => {
  const snapshot = server.find((f) => f.name === "snapshot.json")!.json as Record<string, unknown>;
  const block = (snapshot.blocks as Record<string, unknown>[])[0]!;

  it.each([
    ["not an object", 42],
    ["null", null],
    ["an array", [{ t: "pong", n: 1 }]],
    ["missing t", { n: 1 }],
    ["pong with string n", { t: "pong", n: "1" }],
    ["snapshot without seq", { ...snapshot, seq: undefined }],
    ["snapshot with negative seq", { ...snapshot, seq: -1 }],
    ["block with bad state", { ...snapshot, blocks: [{ ...block, state: "typing" }] }],
    ["block with fractional id", { ...snapshot, blocks: [{ ...block, id: 1.5 }] }],
    ["block missing output", { ...snapshot, blocks: [{ ...block, output: undefined }] }],
    ["update with non-numeric removed", { t: "update", pane: 1, seq: 2, blocks: [], removed: ["80"] }],
    [
      "topology pane missing id",
      {
        t: "topology",
        windows: [
          {
            pid: 1,
            tabs: [{ index: 0, title: "", active: true, panes: [{}] }],
          },
        ],
      },
    ],
    ["error without message", { t: "error", code: "auth" }],
    ["welcome without permission", { t: "welcome", v: 2, host: "h", os: "macos", version: "0", device: "d" }],
    [
      "welcome with an unknown permission",
      {
        t: "welcome",
        v: 2,
        host: "h",
        os: "macos",
        version: "0",
        device: "d",
        permission: "admin",
      },
    ],
    ["failed ack without error", { t: "ack", id: "r1", ok: false }],
    ["ack without id", { t: "ack", ok: true }],
    ["done with string exit", { t: "done", pane: 1, block: 2, command: "x", exit: "0", duration_ms: 1 }],
  ])("%s", (_name, json) => {
    expect(decodeServer(json)).toBeNull();
  });

  it("ignores an unknown t", () => {
    expect(decodeServer({ t: "telemetry", data: 1 })).toBeNull();
  });

  it("parseServerFrame rejects non-JSON", () => {
    expect(parseServerFrame("{not json")).toBeNull();
    expect(parseServerFrame('{"t":"pong","n":3}')).toEqual({ t: "pong", n: 3 });
  });

  it("isClientMsg rejects a malformed subscribe", () => {
    expect(isClientMsg({ t: "subscribe", pane: "12", after: null })).toBe(false);
    expect(isClientMsg({ t: "subscribe", pane: 12 })).toBe(false);
  });
});
