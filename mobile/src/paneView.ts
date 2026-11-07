// The pane screen's pure helpers (the Pane and PaneAsking boards): the picker and the Panes
// sheet's rows, what a block card says, the wide-table detector, the agent question's card,
// and the terminal view's zoom. No React, no native modules.

import { replyChoice } from "./answer";
import { agoShort, paneName, programName } from "./now";
import { pasteProblem, textProblem, type Block, type Pane, type Tab, type Waiting } from "./protocol/types";
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

const zoomListeners = new Set<(zoom: number) => void>();

/**
 * Keeps `zoom` (clamped) as the pinch does, and tells every view that follows it (the terminal
 * view, the ⋯ menu's Text size): the menu and a pinch change the same zoom. Returns the kept value.
 */
export function setKeptZoom(zoom: number): number {
  const z = clampZoom(zoom);
  keptZoom.value = z;
  for (const fn of zoomListeners) fn(z);
  return z;
}

/** Calls `fn` with the kept zoom whenever it changes; returns the unsubscribe. */
export function onKeptZoom(fn: (zoom: number) => void): () => void {
  zoomListeners.add(fn);
  return () => void zoomListeners.delete(fn);
}

// --- Search output: the matches marked ------------------------------------------------------

/** The most matches marked in one text: a short query over a long output stays cheap to draw. */
export const MAX_MARKS = 100;

/** A piece of a text: marked where it matches the search. */
export type MarkPart = { text: string; mark: boolean };

const escapeRegExp = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, (c) => `\\${c}`);

/**
 * `text` in pieces, each occurrence of `query` (trimmed, any case) marked, at most `max` of them
 * (the rest of the text after the last one stays plain); null when the query is empty or not in
 * the text, so the plain text is drawn as before. Joined, the pieces are `text` exactly: a
 * selection's offsets are the same.
 */
export function markParts(text: string, query: string, max = MAX_MARKS): MarkPart[] | null {
  const q = query.trim();
  if (q === "" || text === "") return null;
  const re = new RegExp(escapeRegExp(q), "gi");
  const parts: MarkPart[] = [];
  let at = 0;
  let count = 0;
  for (let m = re.exec(text); m !== null && count < max; m = re.exec(text)) {
    if (m.index > at) parts.push({ text: text.slice(at, m.index), mark: false });
    parts.push({ text: m[0], mark: true });
    at = m.index + m[0].length;
    count++;
  }
  if (count === 0) return null;
  if (at < text.length) parts.push({ text: text.slice(at), mark: false });
  return parts;
}

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

/**
 * Whether the pane runs a coding agent (Claude Code, Codex, …): what the desktop says
 * (`running.agent`, or the session title only an agent's pane carries), else the program's name.
 * Such a TUI may draw on either screen: Claude Code picks its renderer per launch (the classic one
 * on the primary screen, the fullscreen one on the alternate screen).
 */
export function runsAgent(p: Pane): boolean {
  if (!p.running && !p.job) return false;
  return p.running?.agent === true || p.running?.title !== undefined || programName(p) === "claude";
}

/**
 * Whether the pane's screen shows without its own key bar and keyboard, the pane's message input
 * (or the question to answer) under it: a question to answer; a coding agent, on either screen
 * (it is talked to in messages; the message input's ⌃ still opens the key row); and the Terminal
 * view at a shell's prompt. Any other full-screen program (vim, less, htop) keeps the key bar.
 */
export function bareScreen(o: { fullScreen: boolean; asking: boolean; agent: boolean; pinned: boolean; alt: boolean }): boolean {
  return o.fullScreen && (o.asking || o.agent || (o.pinned && !o.alt));
}

/**
 * What the pane shows, by one rule and no switch: the TUI (the program's screen) for the
 * alternate screen (`alt`: vim, htop, Claude Code's fullscreen renderer), a coding agent (Claude
 * Code's classic renderer draws on the primary screen), or a program no block holds (reattached,
 * or a shell without integration); blocks for everything else, `cargo build` and `tail -f`
 * included. `blocksHoldRunning`: the pane's last block is the running program, or the blocks are
 * not loaded yet (unknown counts as held, so the view does not flash). `pinned`: the rule, not the
 * alternate screen, asks for the screen, so the gateway must send it whatever runs (mode `on`).
 * The caller passes `alt` false and expects blocks on a gateway without `screen`. `noBlocks`: the
 * pane's blocks are loaded and empty; with no working folder either, the shell has no integration
 * (it reports neither), runs nothing the gateway can name, and only its screen shows anything.
 */
export function screenView(p: Pane, alt: boolean, blocksHoldRunning: boolean, noBlocks = false): { tui: boolean; pinned: boolean } {
  const pinned = runsAgent(p) || ((!!p.running || !!p.job) && !blocksHoldRunning) || (noBlocks && !p.cwd);
  return { tui: alt || pinned, pinned };
}

// --- the command line while a program runs -------------------------------------------------------

/**
 * What a hardware keyboard's key does in the command line (the phone keyboard's Return is a new line
 * and never reaches here): Return sends, Shift+Return starts a new line; null for any other key.
 */
export const fieldKey = (name: string): "send" | "newline" | null => (name === "enter" ? "send" : name === "shift+enter" ? "newline" : null);

/**
 * A command line placeholder for a screen `width` wide: `full` when it fits on the field's one line
 * (15 pt JetBrains Mono, 9 pt a character; the line leaves about 184 pt for the +, ⌃, Run and
 * gutters), else `short`. A placeholder longer than the line is cut mid-word.
 */
export function fitPlaceholder(full: string, short: string, width: number): string {
  return full.length * 9 <= width - 184 ? full : short;
}

/** The message box's placeholder for `program`: the program's name only when it fits. */
export function messagePlaceholder(program: string, width: number): string {
  return fitPlaceholder(`Message for ${program}`, "Message", width);
}

/**
 * What the command line says about a draft at a shell prompt: Rift runs one line (the gateway
 * refuses a `run` with a line break, nothing runs), so a draft with line breaks cannot be run and
 * Run stays off. Null when it can, and always for a message to a running program.
 */
export function commandNote(draft: string, running: boolean): string | null {
  return !running && /[\r\n]/.test(draft) ? "A command runs as one line. Take out the line breaks to run it." : null;
}

/** How a message goes to a running program: typed, as one paste, or line by line. */
export type ProgramSend =
  | { ok: true; text: string; paste?: undefined; lines?: undefined }
  | { ok: true; paste: string; text?: undefined; lines?: undefined }
  | { ok: true; lines: string[]; text?: undefined; paste?: undefined }
  | { ok: false; message: string | null };

const TOO_OLD_TO_TYPE = "Rift on this computer is too old to type into programs. Update it.";

/** A tab where a program takes one line at a time (it would complete there). */
const TAB_AS_SPACES = "    ";

/**
 * What Return does in the command line of a pane that runs a program: a line goes as text, then
 * Return as its own key. Several lines, tabs or a long message go as one paste (`canPaste`), else
 * line by line with the newline key between them. `message` null: nothing to send (an empty line).
 */
export function programSend(draft: string, canType: boolean, canPaste = false): ProgramSend {
  if (draft.trim() === "") return { ok: false, message: null };
  if (!canType) return { ok: false, message: TOO_OLD_TO_TYPE };
  if (textProblem(draft) === null) return { ok: true, text: draft };
  const text = draft.replace(/\r\n?/g, "\n");
  const problem = pasteProblem(text);
  if (problem === "too_long") return { ok: false, message: "That is too long to send at once." };
  if (problem) return { ok: false, message: "The message holds characters a program cannot be sent." };
  return canPaste ? { ok: true, paste: text } : { ok: true, lines: messageLines(text) };
}

/** How a draft's text between two tokens goes: as `programSend`, and blanks count (`[Image 1] [Image 2]`). */
export function partSend(text: string, canType: boolean, canPaste = false): ProgramSend {
  if (!canType) return { ok: false, message: TOO_OLD_TO_TYPE };
  return text.trim() !== "" ? programSend(text, canType, canPaste) : { ok: true, lines: messageLines(text) };
}

/** The `[Image #N]`s in Claude Code's input box on a pane's screen: below its last `❯`. */
export function boxImages(lines: readonly (readonly (readonly [string, ...unknown[]])[])[]): number {
  const rows = lines.map((runs) => runs.map((r) => r[0]).join(""));
  let at = rows.length - 1;
  while (at >= 0 && !rows[at]!.startsWith("❯")) at--;
  return at < 0 ? 0 : (rows.slice(at).join("\n").match(/\[Image #\d+\]/g) ?? []).length;
}

/** A message as the lines of a program that takes one at a time. */
export const messageLines = (text: string): string[] => text.replace(/\r\n?/g, "\n").replace(/\t/g, TAB_AS_SPACES).split("\n");

/** The gateway's refusal of several lines to a program without bracketed paste (they go line by line instead). */
export const isOneLineRefusal = (message: string) => message.includes("one line at a time");

/** A clipboard text longer than this (or over three lines) waits in the box as a token rather than as text. */
export const MAX_FIELD_PASTE_BYTES = 4096;

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

/** A coding agent drawn with its own logo (Lobe Icons, MIT: mobile/assets/agents). */
export type AgentLogo = "claude" | "codex" | "opencode";
const AGENT_LOGOS: readonly AgentLogo[] = ["claude", "codex", "opencode"];

/**
 * A program's mark: a coding agent's logo (Claude Code, Codex, OpenCode; `mark` null), else the
 * prompt mark `>_` as text (`logo` null).
 */
export function programMark(program: string | null): { mark: ">_" | null; logo: AgentLogo | null; claude: boolean } {
  const logo = AGENT_LOGOS.find((l) => l === program) ?? null;
  return { mark: logo ? null : ">_", logo, claude: logo === "claude" };
}

/** The pane's mark. */
export const paneMark = (p: Pane) => programMark(programName(p));

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
  mark: ">_" | null;
  logo: AgentLogo | null;
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
  return { text: `✕ Failed with ${b.exit}`, tone: "failed" };
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

/** The coding agents by their own names (as New tab lists them). */
const AGENT_NAMES: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode" };

/** Who asks, by name: "Claude Code" for the program "claude", else the program capitalized; "The command" when unknown. */
export function askerName(program: string | null): string {
  if (!program) return "The command";
  return AGENT_NAMES[program] ?? program.charAt(0).toUpperCase() + program.slice(1);
}

/**
 * An answer button's label, saying what happens: "No, explain" for the reply entry (it opens the
 * field for why), "Yes, this session" and "Yes, always"
 * for the Yes entries that extend the permission, else the entry up to its first comma or
 * bracket, cut at 24 characters.
 */
export function answerLabel(w: Waiting, i: number): string {
  const c = w.choices[i]!.trim();
  if (i === replyChoice(w)) return "No, explain";
  if (/^yes\b/i.test(c)) {
    if (/\bsession\b/i.test(c)) return "Yes, this session";
    if (/\b(always|don't ask|do not ask|accept edits|all edits)\b/i.test(c)) return "Yes, always";
  }
  const first = (t: string) => t.trim().split(/[,(]/)[0]!.trim() || t.trim();
  // entries alike up to their comma or bracket (`Bash(aws:*)`, `Bash(curl:*)`) keep the rest: it tells them apart
  const alike = w.choices.some((o, j) => j !== i && first(o) === first(c));
  const short = alike ? c : first(c);
  return short.length > 24 ? `${short.slice(0, 23).trimEnd()}…` : short;
}

/** The command an entry says it runs (`Update now (runs \u0060curl … | sh\u0060)`), which its short label leaves out; null without one. */
export function choiceCommand(choice: string): string | null {
  return /`([^`]+)`/.exec(choice)?.[1] ?? null;
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

/** A diff row's text as the card and A change in full draw it: "− TODO: ship it", "+ Done", a kept line as it is. */
export const diffRowText = (row: DiffRow) => (row.sign === " " ? row.text : `${row.sign === "-" ? "−" : "+"} ${row.text}`);

/**
 * The hunk header over A change in full: the agent's own ("@@ -1,4 +1,6 @@") when it shows one,
 * else made from the first numbered row and the rows' counts; null without line numbers.
 */
export function hunkHeader(ctx: AskContext): string | null {
  const given = ctx.meta.find((t) => t.startsWith("@@"));
  if (given) return given;
  const first = ctx.rows.find((r) => r.n !== null);
  if (!first) return null;
  const kept = ctx.rows.length - ctx.added - ctx.removed;
  return `@@ -${first.n},${kept + ctx.removed} +${first.n},${kept + ctx.added} @@`;
}

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
