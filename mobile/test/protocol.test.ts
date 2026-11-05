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
  isKeyName,
  isNtfyUrl,
  parseServerFrame,
  textProblem,
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
      "branches",
      "changes",
      "chip",
      "clipboard",
      "closing",
      "complete",
      "diff",
      "dirs",
      "done",
      "error",
      "older",
      "output",
      "pong",
      "screen",
      "screen_end",
      "snapshot",
      "topology",
      "update",
      "welcome",
    ]);
    expect(server).toHaveLength(33);
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
      "branches",
      "changes",
      "chip",
      "close",
      "complete",
      "diff",
      "dirs",
      "fit",
      "forget",
      "keys",
      "mouse",
      "move",
      "notify",
      "older",
      "open",
      "output",
      "paste",
      "ping",
      "push",
      "reset_elapsed",
      "run",
      "screen",
      "scroll",
      "start",
      "subscribe",
      "text",
      "unsubscribe",
      "upload",
      "visible",
    ]);
    expect(client).toHaveLength(40);
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
    ["push with an FCM token", { t: "push", token: "fcm:abc", long_ms: null, show_command: false, failures_only: false }],
    [
      "push with an empty token",
      {
        t: "push",
        token: "ExponentPushToken[]",
        long_ms: null,
        show_command: false,
        failures_only: false,
      },
    ],
    [
      "push over a day",
      {
        t: "push",
        token: "ExpoPushToken[a]",
        long_ms: 86_400_001,
        show_command: false,
        failures_only: false,
      },
    ],
    ["push without show_command", { t: "push", token: null, long_ms: null, failures_only: false }],
    ["push without failures_only", { t: "push", token: null, long_ms: null, show_command: false }],
    ["notify without a pane", { t: "notify" }],
    ["visible without on", { t: "visible" }],
    ["visible with a string", { t: "visible", on: "false" }],
  ])("rejects %s", (_name, msg) => {
    expect(isClientMsg(msg)).toBe(false);
  });

  it.each([
    "http://ntfy.sh/topic",
    "https://ntfy.sh/",
    "https://u:p@ntfy.sh/topic",
    "https://ntfy.sh/topic?x=1",
    "https://ntfy.sh/topic#x",
    "https://ntfy.sh/a/b",
    "https://ntfy.sh:0/topic",
    "https://ntfy.sh:65536/topic",
    "https://-ntfy.sh/topic",
    "https://ntfy..sh/topic",
    `https://ntfy.sh/${"a".repeat(65)}`,
    `https://${"h".repeat(200)}.com/t`,
    // request forgery: local names and special IPv4 addresses (as push.rs refuses them)
    "https://localhost/t",
    "https://LocalHost:8080/t",
    "https://ntfy.localhost/t",
    "https://127.0.0.1/t",
    "https://127.9.9.9:443/t",
    "https://0.0.0.0/t",
    "https://0.1.2.3/t",
    "https://169.254.169.254/t",
    "https://255.255.255.255/t",
    "https://224.0.0.1/t",
    "https://239.255.255.250/t",
    "https://127.1/t",
    "https://2130706433/t",
    "https://0x7f.0.0.1/t",
    "https://0177.0.0.1/t",
    "https://ntfy.256/t",
  ])("rejects push with the ntfy URL %s", (ntfy) => {
    expect(isNtfyUrl(ntfy)).toBe(false);
    expect(isClientMsg({ t: "push", token: null, ntfy, long_ms: null, show_command: false, failures_only: false })).toBe(false);
  });

  it.each([
    ["text with a line break", { t: "text", id: "x1", pane: 1, text: ":wq\n" }],
    ["text with an escape", { t: "text", id: "x1", pane: 1, text: "\u001b:q" }],
    ["empty text", { t: "text", id: "x1", pane: 1, text: "" }],
    ["text over 1 KiB", { t: "text", id: "x1", pane: 1, text: "é".repeat(513) }],
    ["text without an id", { t: "text", pane: 1, text: "a" }],
    ["fit with only cols", { t: "fit", id: "f1", pane: 1, cols: 64, rows: null }],
    ["fit with no size", { t: "fit", id: "f1", pane: 1 }],
    ["fit of zero columns", { t: "fit", id: "f1", pane: 1, cols: 0, rows: 10 }],
    ["fit with a fractional size", { t: "fit", id: "f1", pane: 1, cols: 64.5, rows: 10 }],
    ["fit without an id", { t: "fit", pane: 1, cols: 64, rows: 40 }],
    ["screen with an unknown mode", { t: "screen", pane: 1, mode: "always" }],
    ["screen without a pane", { t: "screen", mode: "on" }],
    ["output asking for no lines", { t: "output", id: "o1", pane: 1, block: 2, from: 0, limit: 0 }],
    ["output asking for 501 lines", { t: "output", id: "o1", pane: 1, block: 2, from: 0, limit: 501 }],
    ["output from a negative line", { t: "output", id: "o1", pane: 1, block: 2, from: -1, limit: 10 }],
  ])("rejects %s", (_name, msg) => {
    expect(isClientMsg(msg)).toBe(false);
  });

  it("fit: a size, or both null to release", () => {
    expect(isClientMsg({ t: "fit", id: "f1", pane: 12, cols: 64, rows: 40 })).toBe(true);
    expect(isClientMsg({ t: "fit", id: "f2", pane: 12, cols: null, rows: null })).toBe(true);
  });

  it("text: one line without control characters, at most 1 KiB of UTF-8", () => {
    expect(isClientMsg({ t: "text", id: "x1", pane: 1, text: "é".repeat(512) })).toBe(true);
    expect(textProblem("ü:wq")).toBeNull();
    expect(textProblem("a\tb")).toBe("control");
    expect(textProblem("\u007f")).toBe("control");
    // C1 controls too: the gateway refuses them (char::is_control)
    expect(textProblem("a\u009bb")).toBe("control");
    expect(textProblem("\u0080")).toBe("control");
    expect(textProblem("\u00a0é")).toBeNull();
    expect(textProblem("x".repeat(1025))).toBe("too_long");
    expect(textProblem("")).toBe("empty");
    expect(isClientMsg({ t: "output", id: "o1", pane: 1, block: 2, from: 40, limit: 500 })).toBe(true);
    expect(isClientMsg({ t: "screen", pane: 1, mode: "off" })).toBe(true);
  });

  it("push: an ntfy URL like the gateway's valid_ntfy, or none (left out for older gateways)", () => {
    const push = (ntfy?: string | null) => ({
      t: "push",
      token: null,
      ...(ntfy !== undefined ? { ntfy } : {}),
      long_ms: null,
      show_command: false,
      failures_only: false,
    });
    for (const ok of [
      "https://ntfy.sh/rift-x7Kq2mP9vT4wL8nR3sY6hZ",
      "https://ntfy.example.com:8443/a_b-C",
      "https://10.0.0.2/t",
      "https://192.168.1.20:8080/t",
      "https://172.16.0.5/t",
      "https://100.101.102.103/t",
      "https://localhost.example.com/t",
      "https://ntfy.my-tailnet.ts.net/t",
      "https://8.8.8.8/t",
      `https://ntfy.sh/${"a".repeat(64)}`,
    ]) {
      expect(isNtfyUrl(ok), ok).toBe(true);
      expect(isClientMsg(push(ok)), ok).toBe(true);
    }
    expect(isClientMsg(push(null))).toBe(true);
    expect(isClientMsg(push())).toBe(true);
    expect(isClientMsg(push(5 as never))).toBe(false);
  });

  it("commandProblem names the problem", () => {
    expect(commandProblem("cargo test")).toBeNull();
    expect(commandProblem("x".repeat(2048))).toBeNull();
    expect(commandProblem("é".repeat(1025))).toBe("too_long"); // 2050 UTF-8 bytes
    expect(commandProblem("a\tb")).toBe("control");
    expect(commandProblem("ls\u0085x")).toBe("control");
    expect(commandProblem("ls\u009f")).toBe("control");
    expect(commandProblem("")).toBe("empty");
  });
});

describe("isKeyName (the key grammar)", () => {
  it.each([
    "esc",
    "tab",
    "enter",
    "backspace",
    "delete",
    "insert",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "f1",
    "f12",
    "shift+tab",
    "ctrl+c",
    "ctrl+z",
    "alt+b",
    "ctrl+alt+x",
    "ctrl+space",
    "alt+space",
    "ctrl+alt+delete",
    "shift+f5",
    "ctrl+alt+shift+up",
    "ctrl+shift+enter",
    "alt+shift+f12",
  ])("accepts %s", (name) => {
    expect(isKeyName(name)).toBe(true);
  });

  it.each([
    ["a plain letter (text)", "a"],
    ["plain space (text)", "space"],
    ["an upper-case letter", "ctrl+A"],
    ["modifiers out of order", "shift+ctrl+c"],
    ["alt before ctrl", "alt+ctrl+c"],
    ["a repeated modifier", "ctrl+ctrl+c"],
    ["shift with a letter", "shift+a"],
    ["ctrl+shift with a letter", "ctrl+shift+a"],
    ["a digit", "ctrl+1"],
    ["f13", "f13"],
    ["f0", "f0"],
    ["a bare modifier", "ctrl"],
    ["a dangling plus", "ctrl+"],
    ["an unknown name", "escape"],
    ["a capitalised name", "Esc"],
    ["meta", "meta+c"],
    ["empty", ""],
    ["a symbol", "ctrl+["],
  ])("rejects %s", (_why, name) => {
    expect(isKeyName(name)).toBe(false);
  });

  it("the keys fixture is all valid names, and keys reject one bad name among good ones", () => {
    expect(isClientMsg({ t: "keys", id: "k1", pane: 1, keys: ["ctrl+c", "shift+a"] })).toBe(false);
  });
});

describe("decodeServer Phase 3", () => {
  const screen = server.find((f) => f.name === "screen.json")!.json as Record<string, unknown>;

  it("a welcome without features decodes with none", () => {
    const w = { t: "welcome", v: 2, host: "h", os: "macos", version: "0", device: "d", permission: "operate" };
    expect(decodeServer(w)).toEqual({ ...w, features: [] });
    expect(decodeServer({ ...w, features: ["screen", "future"] })).toMatchObject({ features: ["screen", "future"] });
  });

  it.each([
    ["welcome with features not a list", { t: "welcome", v: 2, host: "h", os: "m", version: "0", device: "d", permission: "view", features: "screen" }],
    ["welcome with a non-string feature", { t: "welcome", v: 2, host: "h", os: "m", version: "0", device: "d", permission: "view", features: [1] }],
    ["screen wider than 300 columns", { ...screen, cols: 301 }],
    ["screen taller than 120 rows", { ...screen, rows: 121 }],
    ["screen row past its rows", { ...screen, lines: [[24, []]] }],
    ["screen run with an upper-case colour", { ...screen, lines: [[0, [["x", "1E1E1E", null, 0]]]] }],
    ["screen run with a short colour", { ...screen, lines: [[0, [["x", "fff", null, 0]]]] }],
    ["screen run with three fields", { ...screen, lines: [[0, [["x", null, null]]]] }],
    ["screen run with negative flags", { ...screen, lines: [[0, [["x", null, null, -1]]]] }],
    ["screen with a cursor of one number", { ...screen, cursor: [3] }],
    ["screen with the cursor below its rows", { ...screen, cursor: [24, 0] }],
    ["screen with the cursor past its columns", { ...screen, cursor: [0, 80] }],
    ["screen with a huge cursor column", { ...screen, cursor: [0, 1e12] }],
    ["screen row with more than two characters per column", { ...screen, lines: [[0, [["x".repeat(100), null, null, 0], ["y".repeat(61), null, null, 0]]]] }],
    ["screen without full", { ...screen, full: undefined }],
    ["screen_end without a pane", { t: "screen_end" }],
    ["output without total", { t: "output", id: "o1", pane: 1, block: 2, from: 0, text: "" }],
    ["output with a text that is not a string", { t: "output", id: "o1", pane: 1, block: 2, from: 0, text: 3, total: 1 }],
  ])("rejects %s", (_name, json) => {
    expect(decodeServer(json)).toBeNull();
  });

  it("accepts a hidden cursor and an unstyled full screen", () => {
    expect(decodeServer({ ...screen, cursor: null, full: true, lines: [[0, [["top", null, null, 0]]]] })).not.toBeNull();
  });

  it("accepts the cursor in the last cell and a row of exactly two characters per column", () => {
    expect(decodeServer({ ...screen, cursor: [23, 79] })).not.toBeNull();
    expect(decodeServer({ ...screen, lines: [[0, [["x".repeat(100), null, null, 0], ["😀".repeat(60), null, null, 0]]]] })).not.toBeNull();
  });

  it("a no_block refusal is an ack error", () => {
    const ack = server.find((f) => f.name === "ack-no-block.json")!.json;
    expect(decodeServer(ack)).toMatchObject({ ok: false, error: "no_block" });
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
    [
      "topology tab with a non-boolean pinned",
      { t: "topology", windows: [{ pid: 1, tabs: [{ index: 0, title: "", active: true, pinned: "yes", panes: [] }] }] },
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
    ["closing without a reason", { t: "closing" }],
    ["closing with an unknown reason", { t: "closing", reason: "crash" }],
    ["closing with a negative back_in_s", { t: "closing", reason: "update", back_in_s: -1 }],
    ["closing with a fractional back_in_s", { t: "closing", reason: "update", back_in_s: 1.5 }],
    ["closing with a null back_in_s", { t: "closing", reason: "update", back_in_s: null }],
    ["closing with a string back_in_s", { t: "closing", reason: "update", back_in_s: "60" }],
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

describe("waiting, running and complete (phone-first)", () => {
  const topo = (pane: Record<string, unknown>) => ({
    t: "topology",
    windows: [{ pid: 1, tabs: [{ index: 0, title: "t", active: true, panes: [{ id: 1, active: true, cwd: null, job: null, last_command: null, last_exit: null, ...pane }] }] }],
  });
  const menu = { kind: "choice", prompt: "Q?", choices: ["Yes", "No"], selected: 0, numbered: true, context: ["x"] };

  it("takes a pane's waiting and running within their bounds", () => {
    expect(decodeServer(topo({ waiting: menu, running: { started: 1, line: "ok" } }))).not.toBeNull();
    expect(decodeServer(topo({ running: { started: null, line: "" } }))).not.toBeNull();
    const bad = [
      { waiting: { ...menu, kind: "maybe" } },
      { waiting: { ...menu, prompt: "q".repeat(301) } },
      { waiting: { ...menu, choices: Array(10).fill("c") } },
      { waiting: { ...menu, choices: ["Yes", "c".repeat(201)] } },
      { waiting: { ...menu, context: Array(13).fill("l") } },
      { waiting: { ...menu, prompt: "two\nlines" } },
      { waiting: { ...menu, selected: 2 } },
      { waiting: { ...menu, choices: [] } },
      // a password never carries what the screen showed around it
      { waiting: { ...menu, kind: "password", choices: [], context: ["secret"] } },
      { running: { started: -1, line: "" } },
      { running: { started: 1, line: "l".repeat(201) } },
    ];
    for (const b of bad) expect(decodeServer(topo(b)), JSON.stringify(b)).toBeNull();
  });

  it("takes a complete answer within its bounds; a request's cursor is inside its line", () => {
    const ok = { t: "complete", id: "k", pane: 1, items: [{ text: "ls", kind: "command" }], history: [{ text: "ls -l", at: null }], more: false };
    expect(decodeServer(ok)).toEqual(ok);
    expect(decodeServer({ ...ok, items: [{ text: "ls", kind: "alias" }] })).toBeNull();
    expect(decodeServer({ ...ok, items: Array(51).fill({ text: "ls", kind: "command" }) })).toBeNull();
    expect(decodeServer({ ...ok, history: [{ text: "x".repeat(1025), at: 1 }] })).toBeNull();
    expect(decodeServer({ ...ok, history: [{ text: "a\nb", at: 1 }] })).toBeNull();
    expect(isClientMsg({ t: "complete", id: "k", pane: 1, line: "é", cursor: 2 })).toBe(true);
    expect(isClientMsg({ t: "complete", id: "k", pane: 1, line: "é", cursor: 3 })).toBe(false);
    expect(isClientMsg({ t: "complete", id: "k", pane: 1, line: "x".repeat(1025), cursor: 0 })).toBe(false);
  });
});
