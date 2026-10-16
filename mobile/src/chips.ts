// What the pane's chips above the command line show, as the desktop's prompt chips do: the
// folder, the branch with ahead/behind and a dirty dot, and the changed files. Also the
// facts on a host's card (Hosts). Pure.

import type { PaneState } from "./protocol/store";
import type { Pane } from "./protocol/types";
import { paneBranch } from "./topology";

/** Which of the folder, branch and changes menus a chip opens (feature `chips`). */
export type ChipMenuKind = "dirs" | "branches" | "changes";

export interface PromptChips {
  /** The pane's directory, home shortened to `~`; null when unknown. */
  cwd: string | null;
  branch: { name: string; ahead: number; behind: number; dirty: boolean } | null;
  /** The uncommitted diff (`3 • +42 −7`); null when clean or unknown. */
  diff: { files: number; added: number; removed: number } | null;
}

/**
 * `/Users/me/rift` → `~/rift` (macOS and Linux home folders, and root's); anything else as is.
 * The phone does not know the computer's home, so it goes by where homes live.
 */
export function tildePath(path: string): string {
  const m = /^(\/Users\/[^/]+|\/home\/[^/]+|\/root)(\/.*)?$/.exec(path);
  return m ? `~${m[2] ?? ""}` : path;
}

/**
 * The chips for `pane`. The git chips come from the gateway's `git` (feature `git`); without it
 * the branch falls back to the newest block's branch (no ahead/behind, no dirty dot) and the
 * changed-files chip is hidden.
 */
export function promptChips(pane: Pane | null, state: PaneState | null, gitFeature: boolean): PromptChips {
  const cwd = pane?.cwd ?? state?.blocks.at(-1)?.cwd ?? null;
  const git = gitFeature ? (pane?.git ?? null) : null;
  if (git) {
    return {
      cwd: cwd ? tildePath(cwd) : null,
      branch: { name: git.branch, ahead: git.ahead, behind: git.behind, dirty: git.dirty },
      diff: git.dirty && git.files > 0 ? { files: git.files, added: git.added, removed: git.removed } : null,
    };
  }
  // with the feature, no `git` means the pane is not in a repository
  const fallback = gitFeature ? null : paneBranch(state);
  return {
    cwd: cwd ? tildePath(cwd) : null,
    branch: fallback ? { name: fallback, ahead: 0, behind: 0, dirty: false } : null,
    diff: null,
  };
}

/** `main ↑1 ↓2` (arrows only when non-zero). */
export function branchText(b: { name: string; ahead: number; behind: number }): string {
  return [b.name, b.ahead > 0 ? `↑${b.ahead}` : null, b.behind > 0 ? `↓${b.behind}` : null].filter(Boolean).join(" ");
}

/** What a screen reader says for the chips, in one line. */
export function chipsSpoken(c: PromptChips): string {
  const parts: string[] = [];
  if (c.cwd) parts.push(`In ${c.cwd}`);
  if (c.branch) {
    const b = c.branch;
    parts.push(
      [
        `branch ${b.name}`,
        b.ahead > 0 ? `${b.ahead} ahead` : null,
        b.behind > 0 ? `${b.behind} behind` : null,
        b.dirty ? "uncommitted changes" : null,
      ]
        .filter(Boolean)
        .join(", "),
    );
  }
  if (c.diff) {
    const d = c.diff;
    parts.push(`${d.files} ${d.files === 1 ? "file" : "files"} changed, ${d.added} lines added, ${d.removed} removed`);
  }
  return parts.join("; ");
}

/**
 * A word for the shell, as the desktop quotes it (crates/rift-terminal/src/input.rs
 * `shell_quote`): plain when it is only safe characters, else single-quoted with each `'`
 * written as `'\''`. Names from the computer are untrusted: they only ever reach a command
 * through this.
 */
export function shellQuote(s: string): string {
  if (s !== "" && /^[A-Za-z0-9/._\-+,:@%~=]+$/.test(s)) return s;
  return `'${s.replace(/'/g, "'\\''")}'`;
}

const QUOTED = String.raw`'(?:[^'\n\r]|'\\'')*'`;
const PLAIN = String.raw`[A-Za-z0-9/._\-+,:@%~=]+`;
const USE = new RegExp(`^rift-use (?:php|python|pyenv) ${QUOTED}$`);
const LOGS = new RegExp(`^docker compose --project-directory (?:${QUOTED}|${PLAIN}) logs --follow --tail 200 (?:${QUOTED}|${PLAIN})$`);

/** `rift-use <tool> '<path or version>'`: what the desktop's Python and PHP menus run. */
export const isUseCommand = (c: string): boolean => USE.test(c);
/** `docker compose --project-directory <dir> logs --follow --tail 200 <service>`: what its Docker menu runs. */
export const isLogsCommand = (c: string): boolean => LOGS.test(c);

/**
 * A command the desktop's version and container menus run. The desktop sends the whole command,
 * so only the two shapes its menus build are accepted: anything else, from a gateway that
 * misbehaves or a link, is no row and no command.
 */
export const isMenuCommand = (c: string): boolean => USE.test(c) || LOGS.test(c);

/** What a chip menu row does (the desktop's `ChipAction`); `run` is a version or container pick (`isMenuCommand`). */
export type ChipAction =
  | { kind: "cd"; name: string }
  | { kind: "checkout"; branch: string }
  | { kind: "create"; branch: string }
  | { kind: "run"; command: string };

/**
 * The command a picked row runs, as the desktop's chip menus build it
 * (crates/rift-terminal/src/views/chip_menu.rs `command`): `cd ..`, `cd ./<name>` (`./` keeps
 * a name starting with `-` from reading as an option), `git checkout <branch>` and
 * `git checkout -b <branch>`.
 */
export function chipCommand(a: ChipAction): string {
  switch (a.kind) {
    case "cd":
      return a.name === ".." ? "cd .." : `cd ${shellQuote(`./${a.name}`)}`;
    case "checkout":
      return `git checkout ${shellQuote(a.branch)}`;
    case "create":
      return `git checkout -b ${shellQuote(a.branch)}`;
    case "run":
      return a.command;
  }
}

/** The branch menu's rows for `query`: matching branches, then "Create" when no branch is named so. */
export function branchRows(branches: readonly string[], query: string): { name: string; create: boolean }[] {
  const q = query.trim();
  const rows = branches.filter((b) => b.toLowerCase().includes(q.toLowerCase())).map((name) => ({ name, create: false }));
  // a plausible new name only: git refuses the rest anyway, but the row should not offer them
  if (q !== "" && !branches.includes(q) && /^[^\s~^:?*[\\]+$/.test(q) && !q.startsWith("-")) rows.push({ name: q, create: true });
  return rows;
}

/** The folder menu's rows for `query`: the parent (only without a query), then matching names. */
export function dirRows(parent: boolean, dirs: readonly string[], query: string): string[] {
  const q = query.trim().toLowerCase();
  return [...(parent && q === "" ? [".."] : []), ...dirs.filter((d) => d.toLowerCase().includes(q))];
}

/** A changed file's name and folder (`crates/rift-pty/src/session.rs` → `session.rs`, `crates/rift-pty/src`). */
export function splitPath(path: string): { name: string; folder: string } {
  const i = path.lastIndexOf("/");
  return i === -1 ? { name: path, folder: "" } : { name: path.slice(i + 1), folder: path.slice(0, i) };
}

/** "2 h ago", "5 min ago", "just now", "3 days ago". */
export function agoText(ms: number): string {
  const min = Math.floor(ms / 60_000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.floor(h / 24);
  return d === 1 ? "1 day ago" : `${d} days ago`;
}

// --- the chip row's two faces ------------------------------------------------------------------

/** What the row above the command field shows: the prompt chips while it is empty, completions while typing. */
export const chipRowMode = (draft: string): "chips" | "completions" => (draft === "" ? "chips" : "completions");

/** The completions row when nothing matches. */
export const NO_COMPLETIONS = "No completions · Return runs it as typed";
/** The same while a program runs: the line goes to it, not to the shell. */
export const NO_COMPLETIONS_PROGRAM = "No completions · Return sends it as typed";

/** The changed-files chip's text: `3 files +42 −7` (the counts the gateway sent for the diff). */
export function diffChipText(d: { files: number; added: number; removed: number }): string {
  const files = `${d.files} ${d.files === 1 ? "file" : "files"}`;
  return d.added > 0 || d.removed > 0 ? `${files} +${d.added} −${d.removed}` : files;
}

/** One completion as the row draws it: what is typed (bright), the rest (dim), and where it comes from. */
export interface CompletionRow {
  /** The whole resulting command line (what tapping fills in). */
  text: string;
  typed: string;
  rest: string;
  /** "history", "branch", "folder" (or the desktop's kind: "command", "subcommand", "option", "value"). */
  source: string;
}

/** Splits `text` at what `draft` already typed: the shared start, bright; the remainder, dim. */
export function splitTyped(draft: string, text: string): { typed: string; rest: string } {
  let n = 0;
  while (n < draft.length && n < text.length && draft[n] === text[n]) n++;
  return { typed: text.slice(0, n), rest: text.slice(n) };
}

/** A completion's source label: path → folder, a value of a `git` command → branch, history, else the desktop's kind. */
export function completionSource(c: { text: string; kind: string }): string {
  if (c.kind === "path") return "folder";
  if (c.kind === "value" && /^git\s/.test(c.text)) return "branch";
  return c.kind;
}

/**
 * The completions for `draft`: the desktop's items, then its history, then the pane's own recent
 * commands (`recent`, newest first) that start with it; each line once, never the draft itself,
 * at most `max`.
 */
export function completionRows(
  draft: string,
  items: readonly { text: string; kind: string }[],
  history: readonly { text: string }[],
  recent: readonly string[],
  max: number,
): CompletionRow[] {
  const out: CompletionRow[] = [];
  const add = (text: string, source: string) => {
    if (text === draft || out.length >= max || out.some((r) => r.text === text)) return;
    out.push({ text, ...splitTyped(draft, text), source });
  };
  for (const c of items) add(c.text, completionSource(c));
  for (const h of history) add(h.text, "history");
  for (const r of recent) if (r.startsWith(draft)) add(r, "history");
  return out;
}
