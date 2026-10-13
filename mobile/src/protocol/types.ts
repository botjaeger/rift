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

export type ErrorCode =
  "auth" | "version" | "bad_request" | "busy" | "forbidden" | "no_pane" | "no_block" | "too_large" | "rate";

export type Permission = "operate" | "view";

/** What a gateway serves beyond Phase 2 (`welcome.features`; absent: none). */
export type Feature = "screen" | "text" | "output" | "push_failures" | "push_ntfy";

/** Base keys that take any modifiers or none (docs/remote-protocol.md, `keys`). */
const NAMED_KEYS: ReadonlySet<string> = new Set([
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
  ...Array.from({ length: 12 }, (_, i) => `f${i + 1}`),
]);

/**
 * Whether `name` is a key `keys` may carry: optional `ctrl+`, `alt+`, `shift+` (in this order,
 * each at most once), then a named key (esc, tab, enter, backspace, delete, insert, arrows,
 * home, end, pageup, pagedown, f1 … f12), or `a` … `z` / `space` with ctrl and/or alt and
 * without shift (plain letters are `text`).
 */
export function isKeyName(name: string): boolean {
  const m = /^(ctrl\+)?(alt\+)?(shift\+)?([a-z0-9]+)$/.exec(name);
  if (!m) return false;
  const [, ctrl, alt, shift, base] = m;
  if (NAMED_KEYS.has(base!)) return true;
  return (base === "space" || /^[a-z]$/.test(base!)) && (!!ctrl || !!alt) && !shift;
}

export const MAX_KEYS_PER_MESSAGE = 20;
/** UTF-8 bytes of a `run` command (one line, no control characters). */
export const MAX_COMMAND_BYTES = 2048;
/** UTF-8 bytes of a `text` message (one line, no control characters). */
export const MAX_TEXT_BYTES = 1024;
/** Lines one `output` request may ask for. */
export const MAX_OUTPUT_LINES = 500;
/** The largest screen a gateway sends (a larger one is cut to its top-left part). */
export const MAX_SCREEN_COLS = 300;
export const MAX_SCREEN_ROWS = 120;
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
/**
 * Whether the app is on screen; only to a gateway with `push_ntfy` (a connection starts visible).
 * While not, the computer posts ntfy alerts about the pane this device follows again.
 */
export interface VisibleMsg {
  t: "visible";
  on: boolean;
}
export interface PingMsg {
  t: "ping";
  n: number;
}
/** Alerts while the app is closed: an Expo push token, an ntfy topic URL, both, or neither (stop). */
export interface PushMsg {
  t: "push";
  token: string | null;
  /** `https://<host>[:port]/<topic>`; only to a gateway with `push_ntfy` (others: left out). */
  ntfy?: string | null;
  /** Alert for every command at least this long; null: only for `notify`. */
  long_ms: number | null;
  show_command: boolean;
  /** The `long_ms` alerts only for commands that failed (an exit status other than 0). */
  failures_only: boolean;
}
/** Alert when the next command in `pane` finishes ("Notify me"). */
export interface NotifyMsg {
  t: "notify";
  pane: number;
}
/** Printable text typed into the program running in the pane (one line, no control characters). */
export interface TextMsg {
  t: "text";
  id: string;
  pane: number;
  text: string;
}
export type ScreenMode = "off" | "auto" | "on";
/** What the gateway sends of the subscribed pane's screen. */
export interface ScreenModeMsg {
  t: "screen";
  pane: number;
  mode: ScreenMode;
}
/** Lines `from` … `from + limit - 1` of a block's output. */
export interface OutputRequestMsg {
  t: "output";
  id: string;
  pane: number;
  block: number;
  from: number;
  limit: number;
}
export type ClientMsg =
  | SubscribeMsg
  | OlderRequestMsg
  | UnsubscribeMsg
  | RunMsg
  | KeysMsg
  | ForgetMsg
  | PingMsg
  | VisibleMsg
  | PushMsg
  | NotifyMsg
  | TextMsg
  | ScreenModeMsg
  | OutputRequestMsg;

// Gateway → client.
export interface WelcomeMsg {
  t: "welcome";
  v: number;
  host: string;
  os: string;
  version: string;
  device: string;
  permission: Permission;
  /** What this gateway serves beyond Phase 2; [] when it leaves the field out. Unknown names are kept. */
  features: string[];
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
/** `[text, fg, bg, flags]`: colours `rrggbb` or null (the default), flags 1 bold, 2 italic, 4 underline, 8 dim, 16 strikethrough. */
export type ScreenRun = [text: string, fg: string | null, bg: string | null, flags: number];
export const RUN_BOLD = 1;
export const RUN_ITALIC = 2;
export const RUN_UNDERLINE = 4;
export const RUN_DIM = 8;
export const RUN_STRIKE = 16;
/** The subscribed pane's visible screen: every row when `full`, else the rows that changed. */
export interface ScreenMsg {
  t: "screen";
  pane: number;
  cols: number;
  rows: number;
  alt: boolean;
  /** `[row, column]` while the program shows its cursor. */
  cursor: [number, number] | null;
  full: boolean;
  lines: [number, ScreenRun[]][];
}
/** Screens stopped (mode `auto` and the pane left the alternate screen, or mode `off`). */
export interface ScreenEndMsg {
  t: "screen_end";
  pane: number;
}
/** The answer to an `output` request: lines from `from` on (`from` is the first one it holds). */
export interface OutputMsg {
  t: "output";
  id: string;
  pane: number;
  block: number;
  from: number;
  text: string;
  /** The block's output lines so far. */
  total: number;
}
export type ServerMsg =
  | WelcomeMsg
  | TopologyMsg
  | SnapshotMsg
  | UpdateMsg
  | OlderMsg
  | AckMsg
  | DoneMsg
  | PongMsg
  | ErrorMsg
  | ScreenMsg
  | ScreenEndMsg
  | OutputMsg;

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

const isColor = (v: unknown) => v === null || (isStr(v) && /^[0-9a-f]{6}$/.test(v));

function isRun(v: unknown): v is ScreenRun {
  return Array.isArray(v) && v.length === 4 && isStr(v[0]) && isColor(v[1]) && isColor(v[2]) && isUint(v[3]);
}

/** Characters (code points) in a row's runs, stopping once past `max`. */
function rowChars(runs: ScreenRun[], max: number): number {
  let n = 0;
  for (const run of runs) {
    for (const _ of run[0]) if (++n > max) return n;
  }
  return n;
}

/**
 * A screen inside its own bounds: the cursor on the screen, and no row with more than two
 * characters per column (a wide character is one character in two columns; combining marks
 * add some), so a hostile gateway cannot make one row arbitrarily long.
 */
function isScreen(m: Obj): boolean {
  if (!(isUint(m.pane) && isUint(m.cols) && isUint(m.rows) && isBool(m.alt) && isBool(m.full))) return false;
  const rows = m.rows as number;
  const cols = m.cols as number;
  if (cols > MAX_SCREEN_COLS || rows > MAX_SCREEN_ROWS) return false;
  const c = m.cursor;
  if (c !== null && !(Array.isArray(c) && c.length === 2 && isUint(c[0]) && isUint(c[1]) && c[0] < rows && c[1] < cols))
    return false;
  return (
    Array.isArray(m.lines) &&
    m.lines.every(
      (l) =>
        Array.isArray(l) &&
        l.length === 2 &&
        isUint(l[0]) &&
        (l[0] as number) < rows &&
        arrayOf(isRun)(l[1]) &&
        rowChars(l[1] as ScreenRun[], cols * 2) <= cols * 2,
    )
  );
}

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
        (m.permission === "operate" || m.permission === "view") &&
        (m.features === undefined || arrayOf(isStr)(m.features))
        ? ({ ...m, features: m.features ?? [] } as unknown as WelcomeMsg)
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
    case "screen":
      return isScreen(m) ? (m as unknown as ScreenMsg) : null;
    case "screen_end":
      return isUint(m.pane) ? (m as unknown as ScreenEndMsg) : null;
    case "output":
      return isStr(m.id) && isUint(m.pane) && isUint(m.block) && isUint(m.from) && isStr(m.text) && isUint(m.total)
        ? (m as unknown as OutputMsg)
        : null;
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
        v.keys.every((k) => isStr(k) && isKeyName(k))
      );
    case "forget":
      return true;
    case "visible":
      return typeof v.on === "boolean";
    case "ping":
      return isUint(v.n);
    case "push":
      return (
        (v.token === null || (isStr(v.token) && isPushToken(v.token))) &&
        (v.ntfy === undefined || v.ntfy === null || (isStr(v.ntfy) && isNtfyUrl(v.ntfy))) &&
        (v.long_ms === null || (isUint(v.long_ms) && v.long_ms <= MAX_LONG_MS)) &&
        typeof v.show_command === "boolean" &&
        typeof v.failures_only === "boolean"
      );
    case "notify":
      return isUint(v.pane);
    case "text":
      return isId(v.id) && isUint(v.pane) && isStr(v.text) && textProblem(v.text) === null;
    case "screen":
      return isUint(v.pane) && (v.mode === "off" || v.mode === "auto" || v.mode === "on");
    case "output":
      return (
        isId(v.id) &&
        isUint(v.pane) &&
        isUint(v.block) &&
        isUint(v.from) &&
        isUint(v.limit) &&
        v.limit >= 1 &&
        v.limit <= MAX_OUTPUT_LINES
      );
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

/**
 * An ntfy topic URL as the gateway accepts it (`valid_ntfy` in crates/rift-remote/src/push.rs):
 * `https://<host>[:port]/<topic>`, topic `[A-Za-z0-9_-]{1,64}`, at most 200 characters, no
 * user, query or fragment.
 */
export function isNtfyUrl(u: string): boolean {
  if (u.length > 200) return false;
  const m = /^https:\/\/([A-Za-z0-9.-]+)(?::([0-9]{1,5}))?\/([A-Za-z0-9_-]{1,64})$/.exec(u);
  if (!m) return false;
  const host = m[1]!;
  if (/^[-.]|[-.]$|\.\./.test(host)) return false;
  if (m[2] !== undefined) {
    const port = Number(m[2]);
    if (port < 1 || port > 65535) return false;
  }
  return publicEnough(host.toLowerCase());
}

/**
 * Whether the gateway may post to `host` (`public_enough` in push.rs): never `localhost` /
 * `*.localhost`, nor an IPv4 address that is loopback, 0/8, link-local, broadcast or multicast;
 * a numeric last label (`127.1`, `2130706433`, `0x7f.1`) must be a plain dotted quad. Private LAN
 * and 100.64/10 (Tailscale) addresses stay allowed.
 */
function publicEnough(host: string): boolean {
  if (host === "localhost" || host.endsWith(".localhost")) return false;
  const last = host.slice(host.lastIndexOf(".") + 1);
  if (!/^[0-9]*$/.test(last) && !last.startsWith("0x")) return true;
  const quad = /^(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]{0,2})$/.exec(host);
  if (!quad) return false;
  const [a, b, c, d] = quad.slice(1).map(Number) as [number, number, number, number];
  if ([a, b, c, d].some((o) => o > 255)) return false;
  return !(a === 127 || a === 0 || (a === 169 && b === 254) || (a >= 224 && a <= 239) || (a === 255 && b === 255 && c === 255 && d === 255));
}

/** C0 and C1 control characters and DEL: the gateway refuses them in `run` and `text` (Rust's `char::is_control`). */
// eslint-disable-next-line no-control-regex
export const CONTROL_CHARS = /[\u0000-\u001f\u007f-\u009f]/;

/** Why `command` cannot be sent with `run`, or null when it can. */
export function commandProblem(command: string): "empty" | "too_long" | "control" | null {
  if (command.trim() === "") return "empty";
  if (CONTROL_CHARS.test(command)) return "control";
  if (utf8Length(command) > MAX_COMMAND_BYTES) return "too_long";
  return null;
}

/** Why `text` cannot be sent with `text`, or null when it can. */
export function textProblem(text: string): "empty" | "too_long" | "control" | null {
  if (text === "") return "empty";
  if (CONTROL_CHARS.test(text)) return "control";
  if (utf8Length(text) > MAX_TEXT_BYTES) return "too_long";
  return null;
}

export function utf8Length(s: string): number {
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
