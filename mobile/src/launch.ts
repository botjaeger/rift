// The New tab and New pane sheets: what a new tab or pane starts with (a shell, or a coding
// agent typed into it), whether the start asks first, and the order it happens in. Pure; the
// sheets pass the phone's `open`, `run`, timer and Face ID in.

import type { ActionResult } from "./fleet";
import { needsAuthBeforeRun } from "./lock";
import type { OpenKind } from "./protocol/types";
import type { Settings } from "./stores";

export interface Program {
  id: "shell" | "claude" | "codex" | "opencode";
  title: string;
  /** What is typed into the new shell (its name); null: the shell alone. */
  command: string | null;
}

/** The sheets' rows, in order. */
export const PROGRAMS: readonly Program[] = [
  { id: "shell", title: "Shell", command: null },
  { id: "claude", title: "Claude Code", command: "claude" },
  { id: "codex", title: "Codex", command: "codex" },
  { id: "opencode", title: "OpenCode", command: "opencode" },
];

/** The row's second line. */
export const programLine = (p: Program, _kind: OpenKind): string =>
  p.command === null ? "A plain shell" : `Opens a shell, then runs ${p.command}`;

/**
 * What starting `p` asks first: `confirm` shows the command before it is typed (Settings › Ask
 * before running), `auth` is Face ID before that (as any Run). A shell types nothing: neither.
 */
export function launchAsks(p: Program, settings: Settings): { confirm: boolean; auth: boolean } {
  if (p.command === null) return { confirm: false, auth: false };
  return { confirm: settings.askBeforeRun, auth: needsAuthBeforeRun(settings) };
}

/**
 * What makes a sheet's visit the same one. A New tab sheet's base pane is the host's active pane,
 * which the tab it opens changes: counting it ended the visit mid-launch, so the program was never
 * typed and the phone stayed on the sheet. A New pane sheet's pane is its route's.
 */
export const launchVisit = (host: string, kind: OpenKind, pane: number | null): string =>
  kind === "pane" ? `${host}:pane:${pane}` : `${host}:tab`;

/** A new shell takes a command once it shows its prompt: a `busy` refusal is tried again this often, this long apart. */
export const READY_TRIES = 10;
export const READY_WAIT_MS = 400;

/** `run` until the new shell is ready for it (the computer says `busy` until its prompt shows). */
export async function runWhenReady(
  run: () => Promise<ActionResult>,
  wait: (ms: number) => Promise<void>,
  current: () => boolean,
  tries = READY_TRIES,
): Promise<ActionResult> {
  let r = await run();
  for (let i = 1; i < tries && !r.ok && r.code === "busy" && current(); i++) {
    await wait(READY_WAIT_MS);
    if (!current()) break;
    r = await run();
  }
  return r;
}

/** `pane`: the new pane to go to; `message`: why not (null: the user cancelled). */
export type Opened = { ok: true; pane: number | undefined } | { ok: false; message: string | null };

export type Launched =
  | { ok: true; pane: number | undefined }
  /** Nothing opened, or the user cancelled (`message` null). */
  | { ok: false; message: string | null; opened?: undefined }
  /** The pane opened and the program did not start in it. */
  | { ok: false; message: string; opened: number };

/** Opens (`open`), then types `program` into the new pane. */
export async function launchProgram(
  program: Program,
  d: {
    open: () => Promise<Opened>;
    run: (pane: number, command: string) => Promise<ActionResult>;
    wait: (ms: number) => Promise<void>;
    current: () => boolean;
  },
): Promise<Launched> {
  const o = await d.open();
  if (!o.ok) return o;
  const command = program.command;
  if (command === null) return o;
  if (o.pane === undefined) return { ok: false, message: `The computer did not say which pane it opened, so ${command} was not started.` };
  const pane = o.pane;
  const r = await runWhenReady(() => d.run(pane, command), d.wait, d.current);
  if (r.ok) return o;
  return { ok: false, message: `The new pane is open, but ${command} did not start. ${r.message}`, opened: pane };
}
