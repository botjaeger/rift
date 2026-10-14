// Answering a pane that waits for the user (the P2 Reply board): what to send for a pick, and
// how the agent's context lines are drawn. Pure.

import type { Waiting } from "./protocol/types";

/** One step of an answer: text typed into the program, or keys pressed, then a pause. */
export type AnswerStep = { text: string } | { keys: string[] } | { pauseMs: number };

/** What the user picked: a menu entry (0-based), a reply after "No", or a line of text. */
export type Pick = { choice: number } | { choice: number; reply: string } | { text: string };

/** Pause between picking "No" and typing what to do instead: the agent first redraws its input. */
export const REPLY_PAUSE_MS = 600;

/**
 * The steps that answer `w` with `pick`: a numbered entry is its digit (the agents pick it at
 * once), an unnumbered one is arrow keys from the selected entry, then Enter; a reply follows
 * its "No" after a pause and ends with Enter; a password or other text is typed, then Enter.
 * Null when the pick does not fit the menu.
 */
export function answerPlan(w: Waiting, pick: Pick): AnswerStep[] | null {
  if ("text" in pick) {
    if (w.kind === "choice" || pick.text === "") return null;
    return [{ text: pick.text }, { keys: ["enter"] }];
  }
  if (w.kind !== "choice" || !Number.isInteger(pick.choice) || pick.choice < 0 || pick.choice >= w.choices.length) return null;
  const steps: AnswerStep[] = [];
  if (w.numbered) steps.push({ text: String(pick.choice + 1) });
  else {
    const d = pick.choice - w.selected;
    const keys = Array<string>(Math.abs(d)).fill(d > 0 ? "down" : "up");
    steps.push({ keys: [...keys, "enter"] });
  }
  if ("reply" in pick && pick.reply.trim() !== "") steps.push({ pauseMs: REPLY_PAUSE_MS }, { text: pick.reply.trim() }, { keys: ["enter"] });
  return steps;
}

/** The choice a "No, and tell it what to do" field stands for: the last entry, when it starts with "No". */
export function replyChoice(w: Waiting): number | null {
  if (w.kind !== "choice") return null;
  const last = w.choices.length - 1;
  return last >= 1 && /^no\b/i.test(w.choices[last]!.trim()) ? last : null;
}

/**
 * The entries an alert's Yes and No buttons pick (the P6 Lock board): the first "Yes…" and the
 * first "No…" of a menu; null when it has not both (the alert then only opens the question).
 */
export function alertAnswers(w: Waiting): { yes: number; no: number } | null {
  if (w.kind !== "choice") return null;
  const yes = w.choices.findIndex((c) => /^yes\b/i.test(c.trim()));
  const no = w.choices.findIndex((c) => /^no\b/i.test(c.trim()));
  return yes === -1 || no === -1 ? null : { yes, no };
}

/** The entry drawn as the primary answer: the first "Yes", else the first. */
export function primaryChoice(w: Waiting): number {
  const i = w.choices.findIndex((c) => /^yes\b/i.test(c.trim()));
  return i === -1 ? 0 : i;
}

/**
 * One question as a string: its kind, wording, choices and the lines above it (the same wording
 * asked again about other lines is another question). An alert carries it as `data.ask`.
 */
export const questionKey = (w: Waiting) => [w.kind, w.prompt, ...w.choices, ...w.context].join("\n");

/** Whether `a` and `b` are the same question (an answer goes only to the question it was given for). */
export function sameQuestion(a: Waiting | undefined, b: Waiting): boolean {
  return !!a && questionKey(a) === questionKey(b);
}

/**
 * The question the answer page answers: the first one seen at `where` (host and pane), so an
 * answer never goes to a question that replaced it; one that was not loaded yet when the page
 * opened (a cold start from an alert) is taken once it arrives, and another pane starts over.
 */
export function keepAsked(
  prev: { where: string; w: Waiting | undefined },
  where: string,
  live: Waiting | undefined,
): { where: string; w: Waiting | undefined } {
  if (prev.where === where && (prev.w !== undefined || live === undefined)) return prev;
  return { where, w: live };
}

/** How a context line is drawn: a diff line added or removed, kept, or the agent's own text. */
export type ContextKind = "added" | "removed" | "kept" | "meta";

/**
 * A context line's kind and text. Agents print diffs with a line number first (Claude Code:
 * ` 12 +let x`, ` 12 -let x`, ` 12  let x`) or plain (`+let x`); anything else is meta (the
 * "Edit file" heading, a path, a hunk header).
 */
export function contextLine(line: string): { kind: ContextKind; text: string } {
  const m = /^\s*(\d+)\s([+\- ])(.*)$/.exec(line);
  if (m) return { kind: m[2] === "+" ? "added" : m[2] === "-" ? "removed" : "kept", text: `${m[2]}${m[3]}` };
  if (/^[+-](?![+-])/.test(line)) return { kind: line[0] === "+" ? "added" : "removed", text: line };
  return { kind: "meta", text: line.trim() };
}

/** The answer page's title: "claude is asking", "sudo needs a password", "build needs your input". */
export function askingTitle(w: Waiting, program: string | null): string {
  const who = program || "The command";
  if (w.kind === "password") return `${who} needs a password`;
  if (w.kind === "input") return `${who} needs your input`;
  return `${who} is asking`;
}
