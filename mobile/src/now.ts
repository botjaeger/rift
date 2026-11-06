// The phone-first views' models (the P1 Now, P3 Pane and P4 Computers boards): what needs the
// user, what runs, what finished; how a pane's blocks fold; the pane pager. Pure.

import type { ActivityItem, AskedView, HostView } from "./fleet";
import { replyChoice } from "./answer";
import type { Block, Pane, Tab, Waiting } from "./protocol/types";
import { tildePath } from "./chips";
import { tabTitle } from "./topology";
import { formatClock } from "./ui/format";

/** A pane waiting for the user (Now › Needs you). */
export interface NeedsItem {
  key: string;
  host: string;
  hostName: string;
  pane: number;
  tab: string;
  /** 1-based, within its tab. */
  paneNumber: number;
  /** What runs there ("claude", "sudo"), or null. */
  program: string | null;
  /** The command that asks ("sudo apt upgrade"), or null. */
  command: string | null;
  /** A coding agent's name for its session ("Code changes overview"), or null. */
  title: string | null;
  /** The pane's folder and branch, for its avatar and place. */
  cwd: string | null;
  branch: string | null;
  waiting: Waiting;
  /** This device may answer (operate permission). */
  operate: boolean;
}

/** A pane running a command (Now › Working), or a coding agent at its own prompt (Now › Idle). */
export interface RunningRow {
  key: string;
  host: string;
  hostName: string;
  pane: number;
  tab: string;
  command: string;
  /** Epoch milliseconds it started (the computer's clock, else when asked from here), or null. */
  started: number | null;
  /** Its latest line, "" when unknown. */
  line: string;
  /** A "Notify me" waits for it. */
  notifies: boolean;
  /** The pane's folder and program, for its avatar. */
  cwd: string | null;
  program: string | null;
  /** A coding agent's name for its session, or null: the row's title in place of the command. */
  title: string | null;
  branch: string | null;
}

export interface NowSections {
  needs: NeedsItem[];
  /** Working: a command runs, or a coding agent works. */
  running: RunningRow[];
  /** A coding agent waits at its own prompt (gateways that send `running.idle`). */
  idle: RunningRow[];
  /** Failures first, then the rest; each newest first. */
  finished: ActivityItem[];
}

const failed = (a: { exit: number | null }) => a.exit !== null && a.exit !== 0;

/**
 * What runs in the pane, as people call it: its foreground program, unless that is no name
 * (Claude Code's process is called by its version, "2.1.296"): then the first word of its
 * command's last part ("cd x && claude" → "claude"); with no program (a shell builtin such as
 * `read`), the command's first word.
 */
export function programName(p: Pick<Pane, "job" | "last_command">): string | null {
  const parts = p.last_command?.split(/;|&&|\|\||\|/).map((x) => x.trim()).filter(Boolean) ?? [];
  const first = (s: string | undefined) => s?.split(/\s+/)[0]?.split("/").at(-1) || null;
  if (!p.job) return first(parts[0]);
  if (/^[\d.]+$/.test(p.job)) return first(parts.at(-1)) ?? p.job;
  return p.job;
}

/**
 * Now's lists over the connected computers and this run's finished commands; `only`: one
 * computer's (its key), else every computer's.
 */
export function nowSections(
  hosts: readonly HostView[],
  activity: readonly ActivityItem[],
  asked: readonly AskedView[],
  only: string | null = null,
): NowSections {
  const needs: NeedsItem[] = [];
  const running: RunningRow[] = [];
  const idle: RunningRow[] = [];
  for (const h of hosts) {
    if (h.conn.status !== "open" || (only !== null && h.record.key !== only)) continue;
    const host = h.record.key;
    const hostName = h.record.name;
    const operate = (h.welcome?.permission ?? h.record.permission) === "operate";
    for (const w of h.windows)
      for (const t of w.tabs)
        for (const [i, p] of t.panes.entries()) {
          const key = `${host}:${p.id}`;
          if (p.waiting) {
            needs.push({
              key,
              host,
              hostName,
              pane: p.id,
              tab: tabTitle(t),
              paneNumber: i + 1,
              program: programName(p),
              command: p.last_command,
              title: p.running?.title || null,
              cwd: p.cwd,
              branch: p.git?.branch ?? null,
              waiting: p.waiting,
              operate,
            });
            continue;
          }
          if (!p.running && !p.job) continue;
          const command = p.last_command ?? p.job ?? "";
          const ask = asked.find((a) => a.host === host && a.pane === p.id && a.command === command);
          const started = p.running?.started != null ? p.running.started * 1000 : (ask?.at ?? null);
          (p.running?.idle ? idle : running).push({
            key,
            host,
            hostName,
            pane: p.id,
            tab: tabTitle(t),
            command,
            started,
            line: p.running?.line ?? "",
            notifies: ask !== undefined,
            cwd: p.cwd,
            program: programName(p),
            title: p.running?.title || null,
            branch: p.git?.branch ?? null,
          });
        }
  }
  const finished = activity
    .filter((a) => only === null || a.host === only)
    .sort((a, b) => Number(failed(b)) - Number(failed(a)) || b.at - a.at);
  return { needs, running, idle, finished };
}

/**
 * When each idle agent stopped working (Now › Idle "12 min"), by row key: set when a row seen
 * working turns idle, dropped when it works again or leaves; an agent idle since before the app
 * saw it work has no time.
 */
export function trackIdle(
  since: Map<string, number>,
  seenWorking: Set<string>,
  working: readonly { key: string }[],
  idle: readonly { key: string }[],
  now: number,
): void {
  for (const r of working) {
    seenWorking.add(r.key);
    since.delete(r.key);
  }
  const idleKeys = new Set(idle.map((r) => r.key));
  for (const key of [...since.keys()]) if (!idleKeys.has(key)) since.delete(key);
  for (const key of idleKeys) if (!since.has(key) && seenWorking.has(key)) since.set(key, now);
}

/**
 * The agent card's short buttons (Now › Needs you: "Yes", "Yes, always", "No…"): one per
 * choice, or null when the menu has more than three (the card then only opens the answer).
 * A "No" choice opens the answer screen, where it can carry what to do instead. Null too when two
 * buttons would read the same (the answer screen shows the whole entries).
 */
export function quickChoices(w: Waiting): { label: string; index: number; opens: boolean }[] | null {
  // a button's short label would hide the command an entry runs: those are answered where it shows
  if (w.kind !== "choice" || w.choices.length > 3 || w.choices.some((c) => /`[^`]+`/.test(c))) return null;
  const reply = replyChoice(w);
  const quick = w.choices.map((c, index) => {
    const t = c.trim();
    if (index === reply) return { label: "No…", index, opens: true };
    if (/^yes\b/i.test(t) && /\b(always|don't ask|do not ask|accept edits|this session|all edits)\b/i.test(t))
      return { label: "Yes, always", index, opens: false };
    const short = t.split(/[,(]/)[0]!.trim();
    return { label: short.length > 14 ? `${short.slice(0, 13)}…` : short, index, opens: false };
  });
  return new Set(quick.map((q) => q.label)).size === quick.length ? quick : null;
}

/** "now", "1 min", "4 min", "2 h", "yesterday", "3 d": how long ago, Now and Run style. */
export function agoShort(ms: number): string {
  const min = Math.floor(Math.max(0, ms) / 60_000);
  if (min < 1) return "now";
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h`;
  const d = Math.floor(h / 24);
  return d === 1 ? "yesterday" : `${d} d`;
}

// --- the pane ------------------------------------------------------------------------------

const succeeded = (b: Block) => b.state === "done" && b.exit === 0;

/** The pane's list: blocks, with one row standing for a run of folded successes. */
export type PaneItem = { kind: "block"; block: Block } | { kind: "fold"; count: number; ids: number[] };

/**
 * Successes folded away (the P3 Pane board: "✓ 6 earlier commands succeeded · Show"): every
 * unbroken run of at least two succeeded blocks becomes one row, except the newest block,
 * which always shows. Failures and the running block stay as they are. `blocks` oldest
 * first; `open`: unfolded.
 */
export function foldBlocks(blocks: readonly Block[], open = false): PaneItem[] {
  if (open) return blocks.map((block) => ({ kind: "block", block }));
  const items: PaneItem[] = [];
  const last = blocks.length - 1;
  for (let i = 0; i <= last; ) {
    let end = i;
    while (end < last && succeeded(blocks[end]!)) end++;
    if (end - i >= 2) {
      const ids = blocks.slice(i, end).map((b) => b.id);
      items.push({ kind: "fold", count: ids.length, ids });
      i = end;
    } else {
      items.push({ kind: "block", block: blocks[i]! });
      i++;
    }
  }
  return items;
}

/** What a pane is called in the pager ("claude"): its program, else its last command's first word. */
export function paneName(p: Pane, index: number): string {
  return programName(p) ?? `pane ${index + 1}`;
}

/** The pane `dir` steps away (1: the next, -1: the one before), or null at either end. */
export function swipeTarget(tab: Tab, paneId: number, dir: 1 | -1): number | null {
  const i = tab.panes.findIndex((p) => p.id === paneId);
  const next = tab.panes[i + dir];
  return i === -1 || !next ? null : next.id;
}

/** A tab in the strip and on Portals: failed (a pane's last command failed and nothing runs), running, waiting. */
export function tabState(tab: Tab): "needs" | "running" | "failed" | "idle" {
  if (tab.panes.some((p) => p.waiting)) return "needs";
  if (tab.panes.some((p) => p.running || p.job)) return "running";
  if (tab.panes.some((p) => p.last_exit !== null && p.last_exit !== 0)) return "failed";
  return "idle";
}

/** The pane a tab opens: its active pane, else its first. */
export function tabPane(tab: Tab): number | null {
  return (tab.panes.find((p) => p.active) ?? tab.panes[0])?.id ?? null;
}

/** A tab's last line on Portals: what waits, runs or ran last. */
export function tabLast(tab: Tab): string {
  const p = tab.panes.find((x) => x.waiting) ?? tab.panes.find((x) => x.running || x.job) ?? tab.panes.find((x) => x.active) ?? tab.panes[0];
  if (!p) return "";
  if (p.waiting) return "waiting for you";
  return p.last_command ?? p.job ?? "";
}

/** Portals' search: a tab whose title, a pane's command, program or folder holds `query` (any case). */
export function tabMatches(tab: Tab, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q === "") return true;
  const hay = [tabTitle(tab), ...tab.panes.flatMap((p) => [p.last_command, p.job, p.cwd, p.waiting?.prompt])];
  return hay.some((x) => x?.toLowerCase().includes(q));
}

/** When the tab's running command started (epoch ms, the computer's clock), or null. */
export function tabStarted(tab: Tab): number | null {
  const started = tab.panes.find((p) => p.running?.started != null)?.running?.started;
  return started != null ? started * 1000 : null;
}

// --- Portals and the Tabs sheet --------------------------------------------------------------
// One row per tab, in a state: what needs the user, what works, what failed, what waits.

/** A tab's state on Portals and the Tabs sheet. */
export type TabKind = "needs" | "working" | "failed" | "idle";

/** Idle agents' "since" (Idle 12m), kept across remounts for the app run. */
export const idleSince = new Map<string, number>();
export const seenWorking = new Set<string>();

const isLive = (p: Pane) => !!(p.running || p.job);
const isWorking = (p: Pane) => isLive(p) && !p.running?.idle;

/** Panes whose last command failed and that run nothing now. */
export function failedCount(tab: Tab): number {
  return tab.panes.filter((p) => !isLive(p) && p.last_exit !== null && p.last_exit !== 0).length;
}

/** Needs you, then working, then failed; a coding agent at its own prompt (or a quiet shell) is idle. */
export function tabKind(tab: Tab): TabKind {
  if (tab.panes.some((p) => p.waiting)) return "needs";
  if (tab.panes.some(isWorking)) return "working";
  if (failedCount(tab) > 0) return "failed";
  return "idle";
}

/** The pane that speaks for the tab: the one that asks, else works, else idles, else the active one. */
export function focusPane(tab: Tab): Pane | undefined {
  return (
    tab.panes.find((p) => p.waiting) ??
    tab.panes.find(isWorking) ??
    tab.panes.find((p) => p.running?.idle) ??
    tab.panes.find((p) => p.active) ??
    tab.panes[0]
  );
}

/** Coding agents get the ✳ tile, everything else the `>_` one. */
export const isAgentProgram = (program: string | null) => program === "claude" || program === "codex" || program === "opencode";

/** "now", "2m", "3h", "4d": a short age for the state texts. */
export function agoCompact(ms: number): string {
  const min = Math.floor(Math.max(0, ms) / 60_000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

/** "Needs you · 2m", "Working 3:56", "Idle 12m", "2 failed": a row's state text (time parts only when known). */
export function stateText(kind: TabKind, o: { ageMs: number | null; elapsedMs: number | null; failed: number }): string {
  switch (kind) {
    case "needs":
      return o.ageMs === null ? "Needs you" : `Needs you · ${agoCompact(o.ageMs)}`;
    case "working":
      return o.elapsedMs === null ? "Working" : `Working ${formatClock(o.elapsedMs)}`;
    case "failed":
      return `${o.failed} failed`;
    case "idle":
      return o.ageMs === null ? "Idle" : `Idle ${agoCompact(o.ageMs)}`;
  }
}


/** One tab's row on Portals and in the Tabs sheet. */
export interface TabRow {
  /** `host:pane` of the pane that speaks for the tab, else `host:tabN`. */
  key: string;
  host: string;
  hostName: string;
  tab: Tab;
  /** The pane it opens, or null. */
  pane: number | null;
  title: string;
  kind: TabKind;
  agent: boolean;
  /** The program in the row's pane, for its mark. */
  program: string | null;
  /** The state in words ("Working 3:56"), for screen readers. */
  state: string;
  /** What the pane that speaks for the tab does, its pane count first: "2 panes · claude · Waiting for you · Allow edits?". */
  line: string;
  /** The same without the pane count (a list that shows the count as its own mark). */
  paneLine: string;
  /** The commands, paths and output in the lines, drawn mono (PaneSummary `codes`). */
  codes: string[];
  /** The short time beside the status mark: "3:56", "2m", "✕ 101", or "". */
  when: string;
  /** The question of the asking pane, with the pane it was asked in. */
  ask: { pane: number; waiting: Waiting; program: string | null } | null;
  /** Whether this device may answer it. */
  operate: boolean;
  panes: number;
}

export interface RowContext {
  now: number;
  /** Several computers on screen: the lines drop what the computer's name already says. */
  multi: boolean;
  /** Epoch ms a question was first seen (fleet.waitingSince). */
  waitingSince: (host: string, pane: number, w: Waiting) => number | null;
  asked: readonly AskedView[];
}

const firstLine = (s: string) => s.split("\n")[0]!.trim();

/** One pane's state: what it asks, runs, failed at or waits on. */
export function paneKind(p: Pane): TabKind {
  if (p.waiting) return "needs";
  if (isWorking(p)) return "working";
  if (!isLive(p) && p.last_exit !== null && p.last_exit !== 0) return "failed";
  return "idle";
}

/** One pane as a row says it: its state, what it does (its program left out) and since when. */
export interface PaneSummary {
  kind: TabKind;
  program: string | null;
  agent: boolean;
  /** In words, its command the only code: "Waiting for you · Allow edits?", "cargo test · running", "cargo test · failed with 101", "Idle · last: ls". */
  activity: string;
  /** "Needs you · 2m": the state in words, for screen readers. */
  state: string;
  /** "2m", "3:56", "✕ 101", or "" when unknown. */
  when: string;
  /** The commands, paths and output in `activity` (and the program in its line): drawn mono, the words in Inter. */
  codes: string[];
}

/** A pane's summary; `key`: its idle-since key (`host:pane`). */
export function paneSummary(host: string, p: Pane, ctx: RowContext, idle: ReadonlyMap<string, number> = idleSince): PaneSummary {
  const kind = paneKind(p);
  const program = programName(p);
  const agent = isAgentProgram(program) || !!p.running?.title;
  const cwd = p.cwd ? tildePath(p.cwd) : null;
  const command = p.last_command ?? p.job ?? "";
  let elapsedMs: number | null = null;
  if (kind === "working") {
    const at =
      p.running?.started != null
        ? p.running.started * 1000
        : (ctx.asked.find((a) => a.host === host && a.pane === p.id && a.command === command)?.at ?? null);
    elapsedMs = at === null ? null : ctx.now - at;
  }
  const since = kind === "needs" && p.waiting ? ctx.waitingSince(host, p.id, p.waiting) : kind === "idle" ? (idle.get(`${host}:${p.id}`) ?? null) : null;
  const ageMs = since === null ? null : ctx.now - since;
  const state = stateText(kind, { ageMs, elapsedMs, failed: kind === "failed" ? 1 : 0 });
  const detail = p.running?.line || p.running?.title || "";
  let activity: string;
  if (kind === "needs" && p.waiting) activity = ["Waiting for you", firstLine(p.waiting.prompt)].filter(Boolean).join(" · ");
  else if (kind === "working") activity = agent ? ["running", detail].filter(Boolean).join(" · ") : [command || p.job, "running", p.running?.line].filter(Boolean).join(" · ");
  else if (kind === "failed") activity = `${command} · failed with ${p.last_exit}`;
  else if (agent) activity = ["Idle", p.running?.title || "waiting for your prompt"].join(" · ");
  // an idle shell: the command it ran last, else where it is
  else activity = p.last_command ? `Idle · last: ${p.last_command}` : cwd ? `Idle · ${cwd}` : "Idle";
  const when =
    kind === "working" ? (elapsedMs === null ? "" : formatClock(elapsedMs)) : kind === "failed" ? `✕ ${p.last_exit}` : ageMs === null ? "" : agoCompact(ageMs);
  // what each line holds besides words: a question and an idle agent's title are words
  const said =
    kind === "needs"
      ? []
      : kind === "working"
        ? [agent ? null : command || p.job, p.running?.line]
        : kind === "failed"
          ? [command]
          : agent
            ? []
            : [p.last_command ? p.last_command : cwd];
  const codes = [agent ? program : null, ...said].filter((c): c is string => !!c);
  return { kind, program, agent, activity, state, when, codes };
}

/** A pane's line: its program, then what it does (a shell's command already names itself). */
export function paneLine(s: PaneSummary): string {
  return s.agent && s.program ? `${s.program} · ${s.activity}` : s.activity;
}

/**
 * Whether two builds of a row draw the same. Rows are built afresh on every publish of the app's
 * store and every clock tick; a list re-renders only the rows this says changed (a long list
 * re-rendered whole kept a phone's JS thread busy, and taps and drags waited behind it).
 */
export function sameTabRow(a: TabRow, b: TabRow): boolean {
  return (
    a.key === b.key &&
    a.host === b.host &&
    a.hostName === b.hostName &&
    a.pane === b.pane &&
    a.title === b.title &&
    a.kind === b.kind &&
    a.agent === b.agent &&
    a.program === b.program &&
    a.state === b.state &&
    a.line === b.line &&
    a.paneLine === b.paneLine &&
    a.codes.join("\0") === b.codes.join("\0") &&
    a.when === b.when &&
    a.operate === b.operate &&
    a.panes === b.panes &&
    a.tab.pinned === b.tab.pinned &&
    a.tab.index === b.tab.index &&
    a.ask?.pane === b.ask?.pane &&
    a.ask?.program === b.ask?.program &&
    a.ask?.waiting === b.ask?.waiting
  );
}

/** The tab's row. `idle`: the since-map of idle agents (`idleSince`). */
export function tabRow(h: HostView, tab: Tab, ctx: RowContext, idle: ReadonlyMap<string, number> = idleSince): TabRow {
  const host = h.record.key;
  const p = focusPane(tab);
  const kind = tabKind(tab);
  const pane = tabPane(tab) ?? p?.id ?? null;
  const key = tabKey(host, tab);
  const s = p ? paneSummary(host, p, ctx, idle) : null;
  const failed = failedCount(tab);
  // the tab's words come from its state (several panes can fail); the line from its lead pane
  const state = kind === "failed" ? stateText(kind, { ageMs: null, elapsedMs: null, failed }) : (s?.state ?? stateText(kind, { ageMs: null, elapsedMs: null, failed }));
  // the mark is the tab's: a failed tab whose speaking pane did not fail shows a failed pane's
  // time, and its line ("make · failed with 2"), so the words never say Idle beside a red ✕
  const failing = kind === "failed" && s?.kind !== "failed" ? tab.panes.find((q) => !isLive(q) && q.last_exit !== null && q.last_exit !== 0) : undefined;
  const fs = failing ? paneSummary(host, failing, ctx, idle) : null;
  const when = fs ? fs.when : (s?.when ?? "");
  const said = fs ?? s;
  const more = tab.panes.length > 1 ? `${tab.panes.length} panes · ` : "";
  // a tab named after its folder ("~", "rift") says no more for the folder leading its line
  const title = tabTitle(tab);
  const place = p?.cwd ? tildePath(p.cwd) : null;
  if (s && place && (place === title || place.split("/").at(-1) === title) && s.activity === `Idle · ${place}`) s.activity = "Idle";
  return {
    key,
    host,
    hostName: h.record.name,
    tab,
    pane,
    title,
    kind,
    agent: s?.agent ?? false,
    program: s?.program ?? null,
    state,
    line: said ? more + paneLine(said) : more.replace(/ · $/, ""),
    paneLine: said ? paneLine(said) : "",
    codes: said?.codes ?? [],
    when,
    ask: p?.waiting ? { pane: p.id, waiting: p.waiting, program: s?.program ?? null } : null,
    operate: (h.welcome?.permission ?? h.record.permission) === "operate",
    panes: tab.panes.length,
  };
}


/** Portals' summary line, a count per state in words and none of them zero: "1 needs you", "2 working", "1 failed", "27 idle". */
export function homeSummary(rows: readonly { kind: TabKind }[]): { kind: TabKind; text: string }[] {
  const n = sheetCounts(rows);
  const words: Record<TabKind, string> = { needs: "needs you", working: "working", failed: "failed", idle: "idle" };
  return (["needs", "working", "failed", "idle"] as const).filter((k) => n[k] > 0).map((k) => ({ kind: k, text: `${n[k]} ${words[k]}` }));
}

/** How many tabs a computer's card on Portals shows; the rest are a tap away (Show all). */
export const HOME_ROWS = 5;

const RANK: Record<TabKind, number> = { needs: 0, working: 1, failed: 2, idle: 3 };

/** A card's rows: what needs you first, then what works, what failed, what idles (each in tab order), the first `max`. */
export function homeRows<T extends { kind: TabKind }>(rows: readonly T[], max = HOME_ROWS): T[] {
  return [...rows].sort((a, b) => RANK[a.kind] - RANK[b.kind]).slice(0, max);
}

/** The Tabs sheet's filter. */
export type SheetFilter = "all" | TabKind;

export function sheetCounts(rows: readonly { kind: TabKind }[]): Record<SheetFilter, number> {
  const n = { all: rows.length, needs: 0, working: 0, failed: 0, idle: 0 };
  for (const r of rows) n[r.kind]++;
  return n;
}

/** The sheet's rows in the computer's tab order, filtered to one state or not. */
export function sheetRows<T extends { kind: TabKind }>(rows: readonly T[], f: SheetFilter): T[] {
  return f === "all" ? [...rows] : rows.filter((r) => r.kind === f);
}

/** What the Tabs page says when a filter holds nothing: that it is empty, and what would show there. */
export const EMPTY_FILTER: Record<SheetFilter, { title: string; line: string }> = {
  all: { title: "No tabs", line: "This computer has no tabs open in Rift." },
  needs: { title: "Nothing needs you", line: "Agents that ask a question show here." },
  working: { title: "Nothing is working", line: "Commands that run show here." },
  failed: { title: "Nothing failed", line: "Commands that fail show here." },
  idle: { title: "Nothing is idle", line: "Tabs that wait for a command show here." },
};

/** The mockup's answer pills: "Yes", "Always", "No" (the quick choices' own labels are longer). */
export function homeChoiceLabel(label: string): string {
  return label === "Yes, always" ? "Always" : label === "No…" ? "No" : label;
}

/** A tab's row key (`host:pane` of the pane that speaks for it): the keys of `trackIdle`. */
export function tabKey(host: string, tab: Tab): string {
  return `${host}:${focusPane(tab)?.id ?? `tab${tab.index}`}`;
}
