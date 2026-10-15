// The phone-first views' models (the P1 Now, P3 Pane and P4 Computers boards): what needs the
// user, what runs, what finished; how a pane's blocks fold; the pane pager. Pure.

import type { ActivityItem, AskedView, HostView } from "./fleet";
import { replyChoice } from "./answer";
import type { Block, Pane, Tab, Waiting } from "./protocol/types";
import { tabTitle } from "./topology";

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

/** Rows Working shows before "N more working". */
export const WORKING_CAP = 3;

/** A computer's mark on Now (several paired): its colour, and a two-letter monogram on each avatar. */
export interface HostMark {
  color: string;
  short: string;
}

/** The computers' colours: the editor theme's syntax colours, the accent left to the app. */
export const HOST_COLORS = ["#4ec9b0", "#c586c0", "#dcdcaa", "#9cdcfe", "#ce9178", "#22c55e"] as const;

/**
 * "MacBook Pro" → "MP", "studio-mac" → "SM", "Sam’s MacBook" → "JM", "MacBook" → "MB",
 * "ubuntu" → "UB": the first letters of the first two words, else a one-word name's two capitals,
 * else its first two letters.
 */
export function monogram(name: string): string {
  const words = name
    .replace(/['’]s\b/g, "")
    .split(/[\s\-_.]+/)
    .filter(Boolean);
  if (words.length >= 2) return (words[0]![0]! + words[1]![0]!).toUpperCase();
  const word = words[0] ?? "?";
  const caps = word.match(/[A-Z]/g);
  return caps && caps.length >= 2 ? caps.slice(0, 2).join("") : word.slice(0, 2).toUpperCase();
}

/** Each paired computer's mark, by key: the colours in pairing order, so two never share one (up to six). */
export function hostMarks(hosts: readonly { record: { key: string; name: string } }[]): Map<string, HostMark> {
  return new Map(hosts.map((h, i) => [h.record.key, { color: HOST_COLORS[i % HOST_COLORS.length]!, short: monogram(h.record.name) }]));
}

/** The summary row under Working ("2 idle · 6 finished · 1 failed"), or null when all are empty. */
export function restSummary(idle: number, finished: readonly { exit: number | null }[]): { idle: number; finished: number; failed: number } | null {
  const bad = finished.filter(failed).length;
  const s = { idle, finished: finished.length - bad, failed: bad };
  return s.idle + s.finished + s.failed === 0 ? null : s;
}

/**
 * The agent card's short buttons (Now › Needs you: "Yes", "Yes, always", "No…"): one per
 * choice, or null when the menu has more than three (the card then only opens the answer).
 * A "No" choice opens the answer screen, where it can carry what to do instead. Null too when two
 * buttons would read the same (the answer screen shows the whole entries).
 */
export function quickChoices(w: Waiting): { label: string; index: number; opens: boolean }[] | null {
  if (w.kind !== "choice" || w.choices.length > 3) return null;
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

/** "14:02": a finished command's clock time. */
export function clockOf(at: number): string {
  const d = new Date(at);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
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

/**
 * The pager line under the tabs ("Pane 1 of 2 · swipe for claude") and its dots; null for a
 * tab with one pane. The swipe goes to the next pane, from the last one to the one before it.
 */
export function pagerModel(tab: Tab, paneId: number): { text: string; index: number; count: number } | null {
  const count = tab.panes.length;
  if (count < 2) return null;
  const index = Math.max(0, tab.panes.findIndex((p) => p.id === paneId));
  const other = index < count - 1 ? index + 1 : index - 1;
  return { text: `Pane ${index + 1} of ${count} · swipe for ${paneName(tab.panes[other]!, other)}`, index, count };
}

/** The pane a swipe lands on (`dir` 1: the next, -1: the one before), or null at either end. */
export function swipeTarget(tab: Tab, paneId: number, dir: 1 | -1): number | null {
  const i = tab.panes.findIndex((p) => p.id === paneId);
  const next = tab.panes[i + dir];
  return i === -1 || !next ? null : next.id;
}

/** A tab in the strip and on Computers: failed (a pane's last command failed and nothing runs), running, waiting. */
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

/** A tab's last line on Computers: what waits, runs or ran last. */
export function tabLast(tab: Tab): string {
  const p = tab.panes.find((x) => x.waiting) ?? tab.panes.find((x) => x.running || x.job) ?? tab.panes.find((x) => x.active) ?? tab.panes[0];
  if (!p) return "";
  if (p.waiting) return "waiting for you";
  return p.last_command ?? p.job ?? "";
}

/** Computers' search: a tab whose title, a pane's command, program or folder holds `query` (any case). */
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
