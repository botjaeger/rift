// The pane's prompt chips on the phone: what the desktop draws above its command line (feature
// `prompt_chips`), or, from a desktop without it, today's folder and git chips. The row fits the
// chips it has room for and folds the rest into "+N". Pure.

import { isLogsCommand, isUseCommand, promptChips } from "./chips";
import type { PaneState } from "./protocol/store";
import type { ComposeContainer, Pane, PaneChip, RuntimeVersion } from "./protocol/types";
import type { ColorName } from "./ui/palette";

/** Which menu a chip opens; the first three are the folder, branch and changes menus (feature `chips`). */
export type ChipOpen = "dirs" | "branches" | "changes" | "pr" | "python" | "php" | "docker" | "elapsed";

/** What the gateway serves that decides the chips: `git`, `chips` (the folder / branch / changes menus), `prompt_chips`. */
export interface ChipFeatures {
  git: boolean;
  menus: boolean;
  promptChips: boolean;
}

/** A run of a chip's text with its colour (a palette name: the palette in use draws it). */
export interface ChipText {
  text: string;
  color: ColorName;
}

/** The glyphs the chips use (src/ui/Icon.tsx); a kind this app does not know gets `terminal`. */
export type ChipIcon =
  | "folder"
  | "branch"
  | "diff"
  | "user"
  | "laptop"
  | "code"
  | "pullRequest"
  | "docker"
  | "calendar"
  | "clock"
  | "hourglass"
  | "terminal";

export interface ChipItem {
  /** The chip's kind (`cwd`, `docker`, …): one per pane. */
  id: string;
  /** What the "This pane" sheet calls it. */
  name: string;
  icon: ChipIcon;
  iconColor: ColorName;
  /** What the desktop draws. */
  full: ChipText[];
  /** What the narrow row shows. */
  short: ChipText[];
  /** Needs a look: the "+N" chip shows a dot for it while it is folded away. */
  attention: boolean;
  /** The menu a tap opens; null: the chip only shows. */
  opens: ChipOpen | null;
  /** What a screen reader says for the chip. */
  spoken: string;
}

const NAMES: Record<string, string> = {
  cwd: "Folder",
  git_branch: "Branch",
  git_diff: "Changes",
  user: "User",
  host: "Host",
  python: "Python",
  php: "PHP",
  github_pr: "Pull request",
  docker: "Docker",
  date: "Date",
  time12: "Time",
  time24: "Time",
  elapsed: "Time on tab",
};

const ICONS: Record<string, ChipIcon> = {
  cwd: "folder",
  git_branch: "branch",
  git_diff: "diff",
  user: "user",
  host: "laptop",
  python: "code",
  php: "code",
  github_pr: "pullRequest",
  docker: "docker",
  date: "calendar",
  time12: "clock",
  time24: "clock",
  elapsed: "hourglass",
};

/** What a tap does, per chip kind (feature `chips` gates the first three). */
const OPENS: Record<string, ChipOpen> = {
  cwd: "dirs",
  git_branch: "branches",
  git_diff: "changes",
  github_pr: "pr",
  python: "python",
  php: "php",
  docker: "docker",
  elapsed: "elapsed",
};

const OPENS_SPOKEN: Record<ChipOpen, string> = {
  dirs: "Opens the sub-folders",
  branches: "Opens the branches",
  changes: "Opens the changes",
  pr: "Opens the pull request",
  python: "Opens the versions",
  php: "Opens the versions",
  docker: "Opens the containers",
  elapsed: "Opens the time card",
};

/** The desktop's colour roles in this app's palette (docs/ui.md; palette.ts maps `dim` / `muted` the other way round). */
const TONES: Record<string, ColorName> = {
  bright: "textBright",
  ui: "text",
  dim: "textMuted",
  muted: "textDim",
  success: "success",
  error: "error",
  warning: "warning",
  accent: "accent",
  magenta: "magenta",
  indigo: "indigo",
};

/** The chips that tell the time: the command line shows one as plain text after the chips, not as a chip. */
const TIMERS = new Set(["elapsed", "time12", "time24", "date"]);
export const isTimer = (kind: string): boolean => TIMERS.has(kind);

/** A timer's words after the chips: Time on tab as "last 2m" (the New Blocks board), the clock and the date as they are. */
export function timerText(c: Pick<ChipItem, "id" | "short">): string {
  const value = c.short.map((p) => p.text).join(" ");
  return c.id === "elapsed" ? `last ${value}` : value;
}

/** A tone's colour; a tone this app does not know (a newer desktop) is drawn as ordinary text. */
export const toneColor = (tone: string): ColorName => TONES[tone] ?? "text";

const styled = (parts: { text: string; tone: string }[]): ChipText[] => parts.map((p) => ({ text: p.text, color: toneColor(p.tone) }));

/** `2086` → `2.1k`, `970` → `970`, `12400` → `12k` (the desktop's short diff). */
export function compact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1)}k`;
  return `${Math.floor(n / 1000)}k`;
}

/** What a screen reader says for a chip's value; the glyphs the desktop draws (↑ ↓ •) are spelt out. */
export function spokenValue(kind: string, full: readonly ChipText[]): string {
  const t = full.map((p) => p.text);
  if (kind === "git_branch") {
    const marks = t.slice(1).join(" ");
    return [
      t[0],
      /↑(\d+)/.exec(marks)?.[1] ? `${/↑(\d+)/.exec(marks)![1]} ahead` : null,
      /↓(\d+)/.exec(marks)?.[1] ? `${/↓(\d+)/.exec(marks)![1]} behind` : null,
      marks.includes("•") ? "uncommitted changes" : null,
    ]
      .filter(Boolean)
      .join(", ");
  }
  if (kind === "git_diff") {
    const files = Number(t[0]);
    const added = /\+(\d+)/.exec(t.join(" "))?.[1];
    const removed = /[−-](\d+)/.exec(t.slice(1).join(" "))?.[1];
    if (Number.isFinite(files) && added !== undefined && removed !== undefined)
      return `${files} ${files === 1 ? "file" : "files"} changed, ${added} lines added, ${removed} removed`;
  }
  if (kind === "docker") {
    const m = /^(\d+)\/(\d+)$/.exec(t.at(-1) ?? "");
    if (m) return `${t.slice(0, -1).join(" ")}, ${m[1]} of ${m[2]} containers running`;
  }
  return t.join(", ");
}

function item(kind: string, w: { icon: string; full: ChipText[]; short: ChipText[]; attention: boolean }, features: ChipFeatures): ChipItem {
  // the folder, branch and changes menus need the `chips` feature; the rest come with `prompt_chips`
  const open = OPENS[kind] ?? null;
  const opens = open && (open === "dirs" || open === "branches" || open === "changes" ? features.menus : features.promptChips) ? open : null;
  const name = NAMES[kind] ?? kind;
  const value = spokenValue(kind, w.full);
  return {
    id: kind,
    name,
    icon: ICONS[kind] ?? "terminal",
    // the folder's glyph in the accent, the branch's and the rest in their tones (the New Blocks board)
    iconColor: kind === "cwd" ? "accent" : toneColor(w.icon),
    full: w.full,
    // the folder reads as its whole path, "~/code/project1" (the New Blocks board); the row folds what does not fit
    short: kind === "cwd" ? w.full : w.short,
    attention: w.attention,
    opens,
    spoken: `${name}: ${value}${w.attention ? ", needs attention" : ""}${opens ? `. ${OPENS_SPOKEN[opens]}` : ""}`,
  };
}

/**
 * The pane's chips. With the desktop's `prompt_chips` they are exactly what it draws, in its
 * order (none when it draws none); without, the folder, the branch and the changed files from
 * what the gateway does send (`cwd`, `git`), styled as the desktop styles them.
 */
export function chipItems(pane: Pane | null, state: PaneState | null, features: ChipFeatures): ChipItem[] {
  if (features.promptChips && pane?.chips) {
    return pane.chips
      .filter((c: PaneChip) => c.full.length > 0)
      .map((c) => item(c.kind, { icon: c.icon, full: styled(c.full), short: styled(c.short.length > 0 ? c.short : c.full), attention: c.attention === true }, features));
  }
  const c = promptChips(pane, state, features.git);
  const out: ChipItem[] = [];
  if (c.cwd) {
    out.push(
      item(
        "cwd",
        {
          icon: "accent",
          full: [{ text: c.cwd, color: "text" }],
          short: [{ text: c.cwd, color: "text" }],
          attention: false,
        },
        features,
      ),
    );
  }
  if (c.branch) {
    const b = c.branch;
    const marks = [b.ahead > 0 ? `↑${b.ahead}` : null, b.behind > 0 ? `↓${b.behind}` : null, b.dirty ? "•" : null].filter(Boolean).join(" ");
    out.push(
      item(
        "git_branch",
        {
          icon: "success",
          full: [{ text: b.name, color: "text" }, ...(marks ? [{ text: marks, color: "textDim" as const }] : [])],
          short: [{ text: b.name, color: "text" }, ...(b.dirty ? [{ text: "•", color: "textDim" as const }] : [])],
          attention: false,
        },
        features,
      ),
    );
  }
  if (c.diff) {
    const d = c.diff;
    out.push(
      item(
        "git_diff",
        {
          icon: "success",
          full: [
            { text: String(d.files), color: "text" },
            { text: "•", color: "textDim" },
            { text: `+${d.added}`, color: "success" },
            { text: `−${d.removed}`, color: "error" },
          ],
          short: [
            { text: String(d.files), color: "text" },
            { text: `+${compact(d.added)}`, color: "success" },
            { text: `−${compact(d.removed)}`, color: "error" },
          ],
          attention: false,
        },
        features,
      ),
    );
  }
  return out;
}

// --- fitting the row ----------------------------------------------------------------------------

/** The gap between chips, and a dot's size and the gap before it in the "+N" chip. */
export const CHIP_GAP = 6;
export const DOT_SIZE = 6;
export const DOT_GAP = 4;

/** The "+N" chip's width: its measured base (the digits and padding) plus a dot for each folded chip that needs attention. */
export const moreWidth = (base: number, dots: number): number => base + dots * (DOT_SIZE + DOT_GAP);

export interface Fit {
  /** The chips shown, from the start; the rest fold into "+N". */
  count: number;
  /** The last shown chip is cut (with …) to this width; otherwise null. */
  clamp: number | null;
}

/** The least room a chip cut short is worth showing in; below it the chip folds into "+N". */
export const MIN_CUT = 60;

/**
 * How many of the chips (`widths` as measured, `attention` per chip) fit one row of `avail` points:
 * all of them when they fit, else as many as leave room for the "+N" chip (`moreBase` wide, plus a
 * dot per folded chip that needs attention). The row never wraps and never scrolls, so a first
 * chip wider than all the room is cut to it rather than dropped.
 */
export function fitChips(widths: readonly number[], attention: readonly boolean[], avail: number, moreBase: number): Fit {
  const n = widths.length;
  if (n === 0) return { count: 0, clamp: null };
  const gaps = (k: number) => CHIP_GAP * Math.max(k - 1, 0);
  const sum = (k: number) => widths.slice(0, k).reduce((a, b) => a + b, 0);
  if (sum(n) + gaps(n) <= avail) return { count: n, clamp: null };
  // the chips that fit whole, leaving room for "+N"
  let k = 0;
  for (let j = n - 1; j >= 1; j--) {
    const dots = attention.slice(j).filter(Boolean).length;
    if (sum(j) + gaps(j) + CHIP_GAP + moreWidth(moreBase, dots) <= avail) {
      k = j;
      break;
    }
  }
  // the order stays: the next chip, cut with …, uses the room left when it is worth showing; else it folds
  const lead = sum(k) + (k > 0 ? gaps(k) + CHIP_GAP : 0);
  const rest = n - k - 1;
  const room = Math.floor(
    avail - lead - (rest > 0 ? CHIP_GAP + moreWidth(moreBase, attention.slice(k + 1).filter(Boolean).length) : 0),
  );
  if (room >= MIN_CUT) return { count: k + 1, clamp: room };
  if (k === 0) return { count: 1, clamp: Math.max(room, 48) };
  return { count: k, clamp: null };
}

/** What a screen reader says for "+N": how many, which, and which need attention. */
export function moreSpoken(hidden: readonly ChipItem[]): string {
  const names = hidden.map((c) => (c.attention ? `${c.name} (needs attention)` : c.name)).join(", ");
  return `${hidden.length} more: ${names}. Shows every chip`;
}

// --- the menus of the new chips -----------------------------------------------------------------

export interface VersionRow {
  label: string;
  /** Where it comes from. */
  source: string;
  current: boolean;
  /** Null: not offered (the command is not one the desktop's menu builds). */
  command: string | null;
}

/** The Python or PHP menu's rows for `query` (case-insensitive over the version and its source), as the desktop lists them. */
export function versionRows(versions: readonly RuntimeVersion[], query: string): VersionRow[] {
  const q = query.trim().toLowerCase();
  return versions
    .filter((v) => q === "" || v.version.toLowerCase().includes(q) || v.source.toLowerCase().includes(q))
    .map((v) => ({ label: v.version, source: v.source, current: v.current, command: isUseCommand(v.command) ? v.command : null }));
}

export interface ContainerRow {
  service: string;
  running: boolean;
  /** `8080→8080 · up 2 hours`, or the whole status when stopped. */
  detail: string;
  health: { word: string; color: ColorName } | null;
  /** Null: not offered. */
  command: string | null;
}

const HEALTH_COLORS = { healthy: "success", starting: "warning", unhealthy: "error" } as const satisfies Record<string, ColorName>;

/** The Docker menu's rows: the service, then its ports and uptime, then its health check's verdict. */
export function containerRows(containers: readonly ComposeContainer[]): ContainerRow[] {
  return containers.map((c) => ({
    service: c.service,
    running: c.running,
    detail: [c.ports.length > 0 ? c.ports.join(", ") : null, c.uptime].filter(Boolean).join(" · "),
    health: c.health ? { word: c.health, color: HEALTH_COLORS[c.health] } : null,
    command: isLogsCommand(c.logs) ? c.logs : null,
  }));
}

/** A PR url the phone opens in the browser: `https` only, one line. */
export const isPrUrl = (u: string): boolean => /^https:\/\/[^\s/]+\/\S*$/.test(u);

/** The Docker menu's title: `flow · 3 of 4 running`. */
export function dockerTitle(project: string | null, containers: readonly ComposeContainer[], error: string | null): string {
  const name = project ?? "Docker";
  if (error) return name;
  return `${name} · ${containers.filter((c) => c.running).length} of ${containers.length} running`;
}

/** The Time on tab card's body: the desktop's line, and for a running count what it counts. */
export function elapsedBody(line: string): string {
  return line.startsWith("Counted") ? `${line}. Counts only while a command runs in this tab.` : line;
}
