// The pane screen's pure helpers (the Pane and PaneAsking boards): the picker and the Panes
// sheet's rows, what a block card says, the wide-table detector, the agent question's card,
// and the terminal view's zoom. No React, no native modules.

import { replyChoice } from "./answer";
import { agoShort, paneName, programName } from "./now";
import { textProblem, type Block, type Pane, type Tab, type Waiting } from "./protocol/types";
import { tildePath } from "./chips";
import { CELL_RATIO, LINE_RATIO } from "./terminal";
import { formatClock } from "./ui/format";

// --- the terminal view's zoom ---------------------------------------------------------------

export const ZOOM_MIN = 0.6;
export const ZOOM_MAX = 2;
/** One tap of − or +. */
export const ZOOM_STEP = 1.15;
/** Two taps this close (milliseconds) reset the zoom. */
export const DOUBLE_TAP_MS = 300;

export const clampZoom = (z: number) => (Number.isFinite(z) ? Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z)) : 1);

/** The zoom while two fingers move: the start zoom scaled by how far apart they are now. */
export function pinchZoom(start: { distance: number; zoom: number }, distance: number): number {
  return start.distance > 0 ? clampZoom((start.zoom * distance) / start.distance) : start.zoom;
}

/** The distance between two touches. */
export const touchDistance = (a: { pageX: number; pageY: number }, b: { pageX: number; pageY: number }) =>
  Math.hypot(a.pageX - b.pageX, a.pageY - b.pageY);

/** `dir` 1 (+) or -1 (−): one step. */
export const zoomStep = (z: number, dir: 1 | -1) => clampZoom(dir === 1 ? z * ZOOM_STEP : z / ZOOM_STEP);

export const zoomPercent = (z: number) => `${Math.round(z * 100)}%`;

/** Whether a tap ending at `now` completes a double tap that began with one ending at `prev` (null: none). */
export const isDoubleTap = (prev: number | null, now: number) => prev !== null && now - prev <= DOUBLE_TAP_MS;

/** The zoom the app keeps for its run: every terminal view starts at it. */
export const keptZoom = { value: 1 };

// --- fitting the program to the phone's view (feature `fit`) ------------------------------------

/** The font size at 100 % zoom while the program is fitted to the view; the zoom scales it. */
export const FIT_BASE_FONT = 11;
/** The desktop's bounds for a fit (it clamps to them too). */
export const FIT_COLS = { min: 20, max: 300 };
export const FIT_ROWS = { min: 5, max: 120 };
/** The desktop drops a fit not sent again within 30 s; it is re-sent this often (milliseconds). */
export const FIT_REFRESH_MS = 10_000;
/** A fit waits this long (milliseconds) after the size stops changing (a pinch, a rotation). */
export const FIT_DEBOUNCE_MS = 300;

/** The font size of the fitted grid: the base size scaled by the zoom, in tenths of a point. */
export const fitFontSize = (zoom: number) => Math.round(FIT_BASE_FONT * clampZoom(zoom) * 10) / 10;

/**
 * The columns and rows that fill `width` x `height` (points, padding already taken off) at
 * `fontSize`, within the desktop's bounds; null before the view is measured.
 */
export function fitSize(width: number, height: number, fontSize: number): { cols: number; rows: number } | null {
  if (!(width > 0) || !(height > 0) || !(fontSize > 0)) return null;
  const cols = Math.floor(width / (fontSize * CELL_RATIO));
  const rows = Math.floor(height / Math.round(fontSize * LINE_RATIO));
  return {
    cols: Math.min(FIT_COLS.max, Math.max(FIT_COLS.min, cols)),
    rows: Math.min(FIT_ROWS.max, Math.max(FIT_ROWS.min, rows)),
  };
}

// --- the Terminal view's default ------------------------------------------------------------

/** Whether the pane runs a coding agent (Claude Code, Codex, …): a TUI that never uses the alternate screen. */
export function runsAgent(p: Pane): boolean {
  if (!p.running && !p.job) return false;
  return p.running?.title !== undefined || programName(p) === "claude";
}

/**
 * Whether the Terminal view is on: always for a pane that runs an agent (its blocks and its screen
 * look the same, so it has no switch), else the user's choice for this pane while the screen is
 * open (null: none), off at a plain shell.
 */
export const terminalViewOn = (choice: boolean | null, agent: boolean) => agent || (choice ?? false);

/** Whether the Blocks | Terminal switch shows: the gateway sends screens and the pane is not an agent's. */
export const showsViewSwitch = (screenFeature: boolean, agent: boolean) => screenFeature && !agent;

// --- the command line while a program runs -------------------------------------------------------

/**
 * What Return does in the command line of a pane that runs a program: the draft goes to the
 * program as text, then Return as its own key. `message` null: nothing to send (an empty line).
 */
export function programSend(draft: string, canType: boolean): { ok: true; text: string } | { ok: false; message: string | null } {
  if (draft.trim() === "") return { ok: false, message: null };
  if (!canType) return { ok: false, message: "Rift on this computer is too old to type into programs. Update it." };
  const problem = textProblem(draft);
  if (problem === "too_long") return { ok: false, message: "That is too long to send at once." };
  if (problem) return { ok: false, message: "The message must be one line." };
  return { ok: true, text: draft };
}

// --- the picker and the Panes sheet ---------------------------------------------------------

/** Claude Code's mark colour (the boards' `#d97757`); other panes use the accent's prompt mark. */
export const CLAUDE_COLOR = "#d97757";

export type StateTone = "running" | "needs" | "failed" | "idle";

/** What a pane is doing, as the Panes sheet says it: Running, Needs you, ✕ N, Idle. */
export function paneState(p: Pane): { text: string; tone: StateTone } {
  if (p.waiting) return { text: "Needs you", tone: "needs" };
  if (p.running || p.job) return { text: "Running", tone: "running" };
  if (p.last_exit !== null && p.last_exit !== 0) return { text: `✕ ${p.last_exit}`, tone: "failed" };
  return { text: "Idle", tone: "idle" };
}

/** The pane's mark: Claude Code's ✳ in its orange, else the prompt mark `>_`. */
export function paneMark(p: Pane): { mark: string; claude: boolean } {
  const claude = programName(p) === "claude";
  return { mark: claude ? "\u2733\uFE0E" : ">_", claude };
}

/** The pane's last line in the Panes sheet: the question, the latest output line, the folder, or the last command. */
export function paneLine(p: Pane): string {
  if (p.waiting) return p.waiting.prompt;
  const state = paneState(p);
  if (state.tone === "running") return p.running?.line || p.last_command || p.job || "";
  if (state.tone === "failed") return p.last_command || "";
  return p.cwd ? tildePath(p.cwd) : (p.last_command ?? "");
}

export interface PaneRow {
  id: number;
  /** 1-based. */
  number: number;
  name: string;
  mark: string;
  claude: boolean;
  line: string;
  state: string;
  tone: StateTone;
  waiting: boolean;
}

/** The tab's panes as the Panes sheet lists them. */
export function paneRows(tab: Tab): PaneRow[] {
  return tab.panes.map((p, i) => {
    const s = paneState(p);
    return {
      id: p.id,
      number: i + 1,
      name: paneName(p, i),
      ...paneMark(p),
      line: paneLine(p),
      state: s.text,
      tone: s.tone,
      waiting: !!p.waiting,
    };
  });
}

/** The tab's other panes that wait for an answer (the picker's count). */
export const waitingOthers = (tab: Tab, paneId: number) => tab.panes.filter((p) => p.waiting && p.id !== paneId).length;

/** "3 panes", "1 pane". */
export const panesText = (n: number) => `${n} ${n === 1 ? "pane" : "panes"}`;

// --- block cards ----------------------------------------------------------------------------

export type CardKind = "ok" | "fail" | "run";

export function cardKind(b: Block): CardKind {
  if (b.state === "running") return "run";
  return b.exit !== null && b.exit !== 0 ? "fail" : "ok";
}

const nonEmpty = (output: string) =>
  output
    .split("\n")
    .map((l) => l.replace(/\r$/, "").trimEnd())
    .filter((l) => l.trim() !== "");

/** The first output line (a success card's one line), or "". */
export const firstLine = (output: string) => nonEmpty(output)[0]?.trim() ?? "";

/** The last `n` non-empty output lines (a running card's latest lines). */
export const lastLines = (output: string, n: number) => nonEmpty(output).slice(-n);

/** A path as a shell prints it in an error: has a slash or an extension, no spaces. */
const looksLikePath = (s: string) => /^[\w@~./()[\]+-]+$/.test(s) && /[/.]/.test(s);

/**
 * A failed card's message: for `prog: path: message` (`bash: x.sh: No such file or directory`)
 * the message and, apart, the path; for anything else the last non-empty line. Null when the
 * command printed nothing.
 */
export function failureMessage(output: string): { text: string; path: string | null } | null {
  const last = nonEmpty(output).at(-1)?.trim();
  if (!last) return null;
  const parts = last.split(": ");
  if (parts.length >= 3 && /^[\w.+-]+$/.test(parts[0]!) && looksLikePath(parts[1]!)) return { text: parts.slice(2).join(": "), path: parts[1]! };
  return { text: last, path: null };
}

// --- whitespace-aligned tables, drawn as a list ----------------------------------------------

export interface ListRow {
  name: string;
  meta: string;
  state: string;
}

const MAX_TABLE_ROWS = 100;
const HEADER_CELL = /^[A-Z][A-Z0-9_%/.-]*(?: [A-Z][A-Z0-9_%/.-]*)*$/;

/**
 * Output that is a table (`docker ps`, `kubectl get pods`) as list rows, or null. Conservative:
 * the first line is a header of at least three upper-case column names set apart by two or more
 * spaces, at least two more lines follow, and on every one of them each column starts where the
 * header's does (the character before it is a space) with a non-empty first column. One line
 * that does not fit, a column that spills over its neighbour, or a right-aligned column makes
 * it plain output.
 */
export function parseTable(output: string): ListRow[] | null {
  const lines = output.replace(/\r/g, "").split("\n");
  while (lines.length > 0 && lines.at(-1)!.trim() === "") lines.pop();
  if (lines.length < 3 || lines.length - 1 > MAX_TABLE_ROWS) return null;
  const header = lines[0]!;
  const cells = [...header.matchAll(/\S+(?: \S+)*/g)];
  if (cells.length < 3 || cells.length > 12 || header.startsWith(" ")) return null;
  if (!cells.every((c) => HEADER_CELL.test(c[0]))) return null;
  const starts = cells.map((c) => c.index);
  const names = cells.map((c) => c[0]);
  const rows: string[][] = [];
  for (const line of lines.slice(1)) {
    if (line.trim() === "" || line.includes("\t")) return null;
    for (const s of starts.slice(1)) if (line.length > s && line[s - 1] !== " ") return null;
    const row = starts.map((s, i) => line.slice(s, starts[i + 1]).trim());
    if (row[0] === "") return null;
    rows.push(row);
  }
  const hex = (v: string) => /^[0-9a-f]{8,}$/.test(v);
  const nameAt = (re: RegExp) => names.findIndex((n) => re.test(n));
  let nameCol = nameAt(/^NAMES?$/);
  if (nameCol === -1) nameCol = nameAt(/^IMAGE$/);
  if (nameCol === -1) nameCol = rows.every((r) => hex(r[0]!)) ? 1 : 0;
  let stateCol = nameAt(/^(STATUS|STATE)$/);
  if (stateCol === -1) stateCol = names.length - 1;
  if (stateCol === nameCol) stateCol = -1;
  return rows.map((r) => ({
    name: r[nameCol]!,
    meta: r
      .map((v, i) => (i === nameCol || i === stateCol || v === "" ? null : /^[0-9a-f]{12,}$/.test(v) ? v.slice(0, 8) : v))
      .filter((v): v is string => v !== null)
      .join(" · "),
    state: stateCol === -1 ? "" : r[stateCol]!,
  }));
}

// --- the block sheet ------------------------------------------------------------------------

export type ChipTone = "running" | "failed" | "ok" | "done";

/** The block sheet's status pill. */
export function sheetChip(b: Block): { text: string; tone: ChipTone } {
  if (b.state === "running") return { text: `● Running ${formatClock(b.duration_ms)}`, tone: "running" };
  if (b.exit === null) return { text: "Finished", tone: "done" };
  if (b.exit === 0) return { text: "✓ Succeeded", tone: "ok" };
  return { text: `✕ Failed · exit ${b.exit}`, tone: "failed" };
}

/** "just now", "4 min ago", "2 h ago", "yesterday". */
export function agoText(ms: number): string {
  const t = agoShort(ms);
  return t === "now" ? "just now" : t === "yesterday" ? t : `${t} ago`;
}

/** The count under "Output ·": "1 line", "12 lines" (empty lines are not counted). */
export function outputLines(output: string): string {
  const n = nonEmpty(output).length;
  return n === 1 ? "1 line" : `${n.toLocaleString("en-US")} lines`;
}

// --- the agent's question card --------------------------------------------------------------

/** "Claude asks" for the program "claude"; "Asks" when the program is unknown. */
export function askerName(program: string | null): string {
  return program ? program.charAt(0).toUpperCase() + program.slice(1) : "The command";
}

/**
 * An answer button's label: "No…" for the reply entry, "Yes, this session" and "Yes, always"
 * for the Yes entries that extend the permission, else the entry up to its first comma or
 * bracket, cut at 24 characters.
 */
export function answerLabel(w: Waiting, i: number): string {
  const c = w.choices[i]!.trim();
  if (i === replyChoice(w)) return "No…";
  if (/^yes\b/i.test(c)) {
    if (/\bsession\b/i.test(c)) return "Yes, this session";
    if (/\b(always|don't ask|do not ask|accept edits|all edits)\b/i.test(c)) return "Yes, always";
  }
  const short = c.split(/[,(]/)[0]!.trim() || c;
  return short.length > 24 ? `${short.slice(0, 23).trimEnd()}…` : short;
}

/** The entries under the primary one, in menu order, without the reply entry (it opens the field). */
export function otherChoices(w: Waiting, primary: number): number[] {
  return w.choices.map((_, i) => i).filter((i) => i !== primary);
}

export interface DiffRow {
  /** The line number, or null (a plain `+x` line). */
  n: number | null;
  sign: "+" | "-" | " ";
  text: string;
  kind: "added" | "removed" | "kept";
}

/** The agent's context lines, read as the card shows them: a file, its diff rows, the rest. */
export interface AskContext {
  file: string | null;
  added: number;
  removed: number;
  rows: DiffRow[];
  /** The other lines (a heading, a hunk header), trimmed, none empty. */
  meta: string[];
}

export function askContext(lines: readonly string[]): AskContext {
  const rows: DiffRow[] = [];
  const meta: string[] = [];
  for (const l of lines) {
    const m = /^\s*(\d+)\s([+\- ])(.*)$/.exec(l);
    if (m) {
      const sign = m[2] as DiffRow["sign"];
      rows.push({ n: Number(m[1]), sign, text: m[3]!, kind: sign === "+" ? "added" : sign === "-" ? "removed" : "kept" });
    } else if (/^[+-](?![+-])/.test(l)) {
      rows.push({ n: null, sign: l[0] as "+" | "-", text: l.slice(1), kind: l[0] === "+" ? "added" : "removed" });
    } else if (l.trim() !== "") meta.push(l.trim());
  }
  let file: string | null = null;
  for (const t of meta) {
    const inner = /^\w+\((.+)\)$/.exec(t)?.[1] ?? t;
    if (looksLikePath(inner)) {
      file = inner;
      break;
    }
  }
  return {
    file,
    added: rows.filter((r) => r.kind === "added").length,
    removed: rows.filter((r) => r.kind === "removed").length,
    rows,
    meta: meta.filter((t) => t !== file && !(file && t.endsWith(`(${file})`))),
  };
}

/** The rows the card shows: the changed lines, or (Full diff) the context lines too. */
export function visibleRows(ctx: AskContext, full: boolean): DiffRow[] {
  return full ? ctx.rows : ctx.rows.filter((r) => r.kind !== "kept");
}

/** Whether "Full diff" shows anything more. */
export const hasMoreRows = (ctx: AskContext) => ctx.rows.some((r) => r.kind === "kept");

// --- the running card's log lines ---------------------------------------------------------------

export interface LogRow {
  level: string;
  message: string;
  /** A trailing time ("41ms"), else "". */
  time: string;
}

const LOG_LEVEL = /^\[?(trace|debug|info|warn|warning|error|fatal)\]?:?\s+/i;
const LOG_TIME = /\s+(\d+(?:\.\d+)?(?:ms|s|µs))$/;

/**
 * Log lines as the Pane board's running card shows them: "api-1  | [info] GET /health 200 3ms" →
 * source "api-1", level "INFO", message "GET /health 200", time "3ms". Only when every line has a
 * level (after an optional "source |" prefix, one source for all); otherwise null and the lines
 * show as they are.
 */
export function parseLogLines(lines: readonly string[]): { source: string | null; rows: LogRow[] } | null {
  if (lines.length === 0) return null;
  let source: string | null | undefined;
  const rows: LogRow[] = [];
  for (const line of lines) {
    const m = /^(\S+)\s+\|\s?(.*)$/.exec(line);
    const from = m ? m[1]! : null;
    if (source !== undefined && from !== source) return null;
    source = from;
    const rest = (m ? m[2]! : line).trim();
    const lvl = LOG_LEVEL.exec(rest);
    if (!lvl) return null;
    const body = rest.slice(lvl[0].length);
    const t = LOG_TIME.exec(body);
    const level = lvl[1]!.toUpperCase();
    rows.push({ level: level === "WARNING" ? "WARN" : level, message: t ? body.slice(0, t.index) : body, time: t ? t[1]! : "" });
  }
  return { source: source ?? null, rows };
}
