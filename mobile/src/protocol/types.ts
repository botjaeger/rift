// Wire format of the Rift remote gateway, version 2 (docs/remote-protocol.md). Every file in
// crates/rift-remote/fixtures/ decodes with these guards (test/protocol.test.ts).

export const PROTOCOL_VERSION = 2;
/** Noise prologue; both sides mix it in, so a peer speaking another version fails the handshake. */
export const PROLOGUE = "rift-remote/2";

export type BlockState = "running" | "done";

export interface Block {
  id: number;
  command: string;
  state: BlockState;
  exit: number | null;
  duration_ms: number;
  cwd: string | null;
  branch: string | null;
  output: string;
  dropped: number;
  restored: boolean;
}

export interface Pane {
  id: number;
  active: boolean;
  cwd: string | null;
  job: string | null;
  last_command: string | null;
  last_exit: number | null;
}

export interface Tab {
  index: number;
  title: string;
  active: boolean;
  panes: Pane[];
}

export interface Window {
  pid: number;
  tabs: Tab[];
}

export type ErrorCode = "auth" | "version" | "bad_request" | "busy" | "forbidden" | "no_pane" | "too_large" | "rate";

export type Permission = "operate" | "view";

/** Keys `keys` may carry (docs/remote-protocol.md). */
export const KEY_NAMES: readonly string[] = [
  ..."abcdefghijklmnopqrstuvwxyz".split("").map((c) => `ctrl+${c}`),
  "esc",
  "tab",
  "shift+tab",
  "enter",
  "backspace",
  "up",
  "down",
  "left",
  "right",
  "home",
  "end",
  "pageup",
  "pagedown",
];
const KEY_SET = new Set(KEY_NAMES);
export const MAX_KEYS_PER_MESSAGE = 20;
/** UTF-8 bytes of a `run` command (one line, no control characters). */
export const MAX_COMMAND_BYTES = 2048;
/** Client messages larger than this close the connection. */
export const MAX_CLIENT_MESSAGE_BYTES = 4096;
export const MAX_ID_CHARS = 64;

// Handshake payloads.
export interface GatewayHello {
  v: number;
  host: string;
  os: string;
  version: string;
}
export interface ClientHello {
  v: number;
  name: string;
  platform: string;
  /** The pairing secret (base64url), only on the pairing connection. */
  pair?: string;
}

// Client → gateway.
export interface SubscribeMsg {
  t: "subscribe";
  pane: number;
  after: number | null;
}
export interface OlderRequestMsg {
  t: "older";
  pane: number;
  before: number;
}
export interface UnsubscribeMsg {
  t: "unsubscribe";
}
export interface RunMsg {
  t: "run";
  id: string;
  pane: number;
  command: string;
}
export interface KeysMsg {
  t: "keys";
  id: string;
  pane: number;
  keys: string[];
}
export interface ForgetMsg {
  t: "forget";
}
export interface PingMsg {
  t: "ping";
  n: number;
}
/** Alerts while the app is closed: an Expo push token, or null to stop. */
export interface PushMsg {
  t: "push";
  token: string | null;
  /** Alert for every command at least this long; null: only for `notify`. */
  long_ms: number | null;
  show_command: boolean;
}
/** Alert when the next command in `pane` finishes ("Notify me"). */
export interface NotifyMsg {
  t: "notify";
  pane: number;
}
export type ClientMsg =
  SubscribeMsg | OlderRequestMsg | UnsubscribeMsg | RunMsg | KeysMsg | ForgetMsg | PingMsg | PushMsg | NotifyMsg;

// Gateway → client.
export interface WelcomeMsg {
  t: "welcome";
  v: number;
  host: string;
  os: string;
  version: string;
  device: string;
  permission: Permission;
}
export interface TopologyMsg {
  t: "topology";
  windows: Window[];
}
export interface SnapshotMsg {
  t: "snapshot";
  pane: number;
  seq: number;
  blocks: Block[];
  has_older: boolean;
}
export interface UpdateMsg {
  t: "update";
  pane: number;
  seq: number;
  blocks: Block[];
  removed: number[];
}
export interface OlderMsg {
  t: "older";
  pane: number;
  blocks: Block[];
  has_older: boolean;
}
export interface AckMsg {
  t: "ack";
  id: string;
  ok: boolean;
  error?: ErrorCode | (string & {});
  message?: string;
}
export interface DoneMsg {
  t: "done";
  pane: number;
  block: number;
  command: string;
  exit: number | null;
  duration_ms: number;
}
export interface PongMsg {
  t: "pong";
  n: number;
}
export interface ErrorMsg {
  t: "error";
  code: ErrorCode | (string & {});
  message: string;
}
export type ServerMsg =
  WelcomeMsg | TopologyMsg | SnapshotMsg | UpdateMsg | OlderMsg | AckMsg | DoneMsg | PongMsg | ErrorMsg;

type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const isUint = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const isInt = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v);
const isStr = (v: unknown): v is string => typeof v === "string";
const isBool = (v: unknown): v is boolean => typeof v === "boolean";
const orNull =
  <T>(g: (v: unknown) => v is T) =>
  (v: unknown): v is T | null =>
    v === null || g(v);
const arrayOf =
  <T>(g: (v: unknown) => v is T) =>
  (v: unknown): v is T[] =>
    Array.isArray(v) && v.every(g);

const isStrOrNull = orNull(isStr);
const isIntOrNull = orNull(isInt);
const isOptStr = (v: unknown) => v === undefined || isStr(v);

export function isBlock(v: unknown): v is Block {
  return (
    isObj(v) &&
    isUint(v.id) &&
    isStr(v.command) &&
    (v.state === "running" || v.state === "done") &&
    isIntOrNull(v.exit) &&
    isUint(v.duration_ms) &&
    isStrOrNull(v.cwd) &&
    isStrOrNull(v.branch) &&
    isStr(v.output) &&
    isUint(v.dropped) &&
    isBool(v.restored)
  );
}

function isPane(v: unknown): v is Pane {
  return (
    isObj(v) &&
    isUint(v.id) &&
    isBool(v.active) &&
    isStrOrNull(v.cwd) &&
    isStrOrNull(v.job) &&
    isStrOrNull(v.last_command) &&
    isIntOrNull(v.last_exit)
  );
}

function isTab(v: unknown): v is Tab {
  return isObj(v) && isUint(v.index) && isStr(v.title) && isBool(v.active) && arrayOf(isPane)(v.panes);
}

function isWindow(v: unknown): v is Window {
  return isObj(v) && isUint(v.pid) && arrayOf(isTab)(v.tabs);
}

const isBlocks = arrayOf(isBlock);

/**
 * Validates one gateway message (already JSON-parsed). Returns null for a malformed object and
 * for an unknown `t` (the client ignores types it does not know, for forward compatibility).
 */
export function decodeServer(json: unknown): ServerMsg | null {
  if (!isObj(json)) return null;
  const m = json;
  switch (m.t) {
    case "welcome":
      return isUint(m.v) &&
        isStr(m.host) &&
        isStr(m.os) &&
        isStr(m.version) &&
        isStr(m.device) &&
        (m.permission === "operate" || m.permission === "view")
        ? (m as unknown as WelcomeMsg)
        : null;
    case "topology":
      return arrayOf(isWindow)(m.windows) ? (m as unknown as TopologyMsg) : null;
    case "snapshot":
      return isUint(m.pane) && isUint(m.seq) && isBlocks(m.blocks) && isBool(m.has_older)
        ? (m as unknown as SnapshotMsg)
        : null;
    case "update":
      return isUint(m.pane) && isUint(m.seq) && isBlocks(m.blocks) && arrayOf(isUint)(m.removed)
        ? (m as unknown as UpdateMsg)
        : null;
    case "older":
      return isUint(m.pane) && isBlocks(m.blocks) && isBool(m.has_older) ? (m as unknown as OlderMsg) : null;
    case "ack":
      return isStr(m.id) && isBool(m.ok) && isOptStr(m.error) && isOptStr(m.message) && (m.ok || isStr(m.error))
        ? (m as unknown as AckMsg)
        : null;
    case "done":
      return isUint(m.pane) && isUint(m.block) && isStr(m.command) && isIntOrNull(m.exit) && isUint(m.duration_ms)
        ? (m as unknown as DoneMsg)
        : null;
    case "pong":
      return isUint(m.n) ? (m as unknown as PongMsg) : null;
    case "error":
      return isStr(m.code) && isStr(m.message) ? (m as unknown as ErrorMsg) : null;
    default:
      return null;
  }
}

/** Parses one decoded JSON message; null when it is not JSON or not a known, well-formed message. */
export function parseServerFrame(text: string): ServerMsg | null {
  try {
    return decodeServer(JSON.parse(text));
  } catch {
    return null;
  }
}

const isId = (v: unknown): v is string => isStr(v) && v.length > 0 && v.length <= MAX_ID_CHARS;

/** Shape check for messages this app sends; the tests run it over the client fixtures. */
export function isClientMsg(v: unknown): v is ClientMsg {
  if (!isObj(v)) return false;
  switch (v.t) {
    case "subscribe":
      return isUint(v.pane) && (v.after === null || isUint(v.after));
    case "older":
      return isUint(v.pane) && isUint(v.before);
    case "unsubscribe":
      return true;
    case "run":
      return isId(v.id) && isUint(v.pane) && isStr(v.command) && commandProblem(v.command) === null;
    case "keys":
      return (
        isId(v.id) &&
        isUint(v.pane) &&
        Array.isArray(v.keys) &&
        v.keys.length > 0 &&
        v.keys.length <= MAX_KEYS_PER_MESSAGE &&
        v.keys.every((k) => isStr(k) && KEY_SET.has(k))
      );
    case "forget":
      return true;
    case "ping":
      return isUint(v.n);
    case "push":
      return (
        (v.token === null || (isStr(v.token) && isPushToken(v.token))) &&
        (v.long_ms === null || (isUint(v.long_ms) && v.long_ms <= MAX_LONG_MS)) &&
        typeof v.show_command === "boolean"
      );
    case "notify":
      return isUint(v.pane);
    default:
      return false;
  }
}

/** Longest `push.long_ms` the gateway accepts (a day). */
export const MAX_LONG_MS = 24 * 3600 * 1000;

/** An Expo push token as the gateway accepts it (`crates/rift-remote/src/push.rs`). */
export function isPushToken(t: string): boolean {
  return t.length <= 200 && /^(ExponentPushToken|ExpoPushToken)\[[A-Za-z0-9_-]+\]$/.test(t);
}

/** Why `command` cannot be sent with `run`, or null when it can. */
export function commandProblem(command: string): "empty" | "too_long" | "control" | null {
  if (command.trim() === "") return "empty";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(command)) return "control";
  if (utf8Length(command) > MAX_COMMAND_BYTES) return "too_long";
  return null;
}

function utf8Length(s: string): number {
  let n = 0;
  for (const ch of s) {
    const cp = ch.codePointAt(0)!;
    n += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
  }
  return n;
}

export function isGatewayHello(v: unknown): v is GatewayHello {
  return isObj(v) && isUint(v.v) && isStr(v.host) && isStr(v.os) && isStr(v.version);
}

export function isClientHello(v: unknown): v is ClientHello {
  return isObj(v) && isUint(v.v) && isStr(v.name) && isStr(v.platform) && isOptStr(v.pair);
}
