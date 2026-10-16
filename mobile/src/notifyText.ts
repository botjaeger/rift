// The words of the notifications the app posts itself (a finished command, an agent's question).
// Pure. A computer that holds this device's push token words its alerts the same way
// (crates/rift-remote/src/push.rs `alert_text`), except that it never sends output.

/** Longest command in a title (characters, "…" included). */
export const MAX_TITLE_COMMAND = 40;

/** Longest output line in a body. */
export const MAX_BODY_LINE = 120;

/** What stands for the command when "Show the command" is off. */
export const HIDDEN_COMMAND = "A command";

/** "113ms", "8.4s", "12s", "2m 5s", "1h 2m". */
export function durationText(ms: number): string {
  const t = Math.max(0, Math.floor(Number.isFinite(ms) ? ms : 0));
  if (t < 1000) return `${t}ms`;
  const tenths = Math.floor(t / 100);
  if (tenths < 100) return `${Math.floor(tenths / 10)}.${tenths % 10}s`;
  const s = Math.floor(t / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

/** `text` on one line, cut to `max` characters with a trailing "…". */
function oneLine(text: string, max: number): string {
  const chars = Array.from(text.split(/\s+/).filter(Boolean).join(" "));
  if (chars.length <= max) return chars.join("");
  return `${chars.slice(0, max - 1).join("").trimEnd()}…`;
}

/** The command for a title: one line, at most 40 characters; "A command" when hidden or empty. */
export function titleCommand(command: string, showCommand: boolean): string {
  if (!showCommand) return HIDDEN_COMMAND;
  return oneLine(command, MAX_TITLE_COMMAND) || HIDDEN_COMMAND;
}

/** "✓ 0", "✕ 101"; "✓" when the shell reported no status. */
export function exitMark(exit: number | null): string {
  if (exit === null) return "✓";
  return exit === 0 ? "✓ 0" : `✕ ${exit}`;
}

export interface DoneFacts {
  /** The computer's name. */
  host: string;
  /** The tab's title, when known. */
  tab: string | null;
  command: string;
  /** The command's exit status (null: the shell reported none). */
  exit: number | null;
  durationMs: number;
  /** The command's last output line, when known. */
  lastLine?: string | null;
  /** Settings › "Show the command". Off hides the command and the output line. */
  showCommand: boolean;
}

export interface NotificationParts {
  title: string;
  subtitle: string;
  /** Absent (not empty) when there is nothing to add. */
  body?: string;
}

/**
 * A finished command: "✓ 0 · cargo test" / "Finished in 8.4s · studio-mac · rift" and the last
 * output line; "✕ 101 · …" / "Failed after 8.4s · …" when it failed. With "Show the command"
 * off the command is "A command" and no output line is shown.
 */
export function doneText(f: DoneFacts): NotificationParts {
  const failed = f.exit !== null && f.exit !== 0;
  const took = durationText(f.durationMs);
  const out: NotificationParts = {
    title: `${exitMark(f.exit)} · ${titleCommand(f.command, f.showCommand)}`,
    subtitle: [failed ? `Failed after ${took}` : `Finished in ${took}`, f.host, f.tab].filter(Boolean).join(" · "),
  };
  const line = f.showCommand && f.lastLine ? oneLine(f.lastLine, MAX_BODY_LINE) : "";
  if (line) out.body = line;
  return out;
}

export interface AskFacts {
  /** The program that asks ("claude"), when known. */
  program: string | null;
  /** What it waits for: a choice or confirmation, a password, or other input. */
  kind: "choice" | "password" | "input";
  host: string;
  tab: string | null;
  /** The pane's number in its tab, 1-based. */
  pane: number;
  prompt: string;
}

/** An agent's question: "claude is asking" / "studio-mac · rift · pane 2" and the prompt. */
export function askText(f: AskFacts): NotificationParts {
  const who = f.program || "The command";
  const title =
    f.kind === "password" ? `${who} needs a password` : f.kind === "input" ? `${who} needs your input` : `${who} is asking`;
  const out: NotificationParts = {
    title,
    subtitle: [f.host, f.tab, `pane ${f.pane}`].filter(Boolean).join(" · "),
  };
  const prompt = oneLine(f.prompt, MAX_BODY_LINE * 2);
  if (prompt) out.body = prompt;
  return out;
}
