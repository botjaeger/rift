// The tokens in a message box: `[Image 1]`, `[File note.pdf]`, `[Text 12 lines]` stand for a pasted
// item at their place in the draft, and the user types around them. Each token is a span of the
// draft with an item behind it, tracked by position: text that only looks like a token (typed, or
// left after its item went) is plain text. No imports: the rules are tested on their own.

/** A span of the draft that stands for the item `id`. */
export interface Token {
  id: number;
  /** Where it starts in the draft (UTF-16 units). */
  at: number;
  /** Its text in the draft, brackets included. */
  label: string;
}

export interface Selection {
  start: number;
  end: number;
}

export interface Reconciled {
  text: string;
  tokens: Token[];
  /** The items whose tokens the edit took. */
  gone: number[];
  /** Where the cursor belongs when the edit ended up other than as typed. */
  cursor?: number;
}

/** The token for the `n`th image of a box. */
export const imageLabel = (n: number) => `[Image ${n}]`;

/** The token for a file: its own name on one line. */
export const fileLabel = (name: string) => `[File ${name.replace(/[\r\n\t]+/g, " ")}]`;

const lineCount = (text: string) => text.split(/\r\n?|\n/).length;

/** The token for a pasted text. */
export function textLabel(text: string): string {
  const n = lineCount(text);
  return `[Text ${n} ${n === 1 ? "line" : "lines"}]`;
}

/** More lines than this, and a pasted text is a token. */
const MAX_FIELD_PASTE_LINES = 3;

/** Whether a pasted text becomes a token rather than going into the field as it is. */
export const pastesAsToken = (text: string, bytes: number, maxBytes: number) => lineCount(text) > MAX_FIELD_PASTE_LINES || bytes > maxBytes;

/** How a screen reader says a token: without its brackets. */
export const spokenLabel = (label: string) => label.slice(1, -1);

const byPlace = (a: Token, b: Token) => a.at - b.at;

/** One way to read `prev` becoming `next`: `prev[from, to)` was replaced by `next[from, next.length - (prev.length - to))`. */
interface Edit {
  from: number;
  to: number;
}

function earlyEdit(prev: string, next: string): Edit {
  const max = Math.min(prev.length, next.length);
  let from = 0;
  while (from < max && prev[from] === next[from]) from++;
  let tail = 0;
  while (tail < max - from && prev[prev.length - 1 - tail] === next[next.length - 1 - tail]) tail++;
  return { from, to: prev.length - tail };
}

function lateEdit(prev: string, next: string): Edit {
  const max = Math.min(prev.length, next.length);
  let tail = 0;
  while (tail < max && prev[prev.length - 1 - tail] === next[next.length - 1 - tail]) tail++;
  let from = 0;
  while (from < max - tail && prev[from] === next[from]) from++;
  return { from, to: prev.length - tail };
}

const touches = (e: Edit, t: Token) =>
  e.from === e.to ? e.from > t.at && e.from < t.at + t.label.length : e.from < t.at + t.label.length && e.to > t.at;

/**
 * The draft after an edit from `prev` to `next`. Deleting any part of a token deletes all of it and
 * its item. Text typed inside a token goes just after it, the token whole. An edit that repeats
 * characters reads two ways ("]]" less one): the reading that leaves the tokens alone wins.
 */
export function reconcile(prev: string, next: string, tokens: readonly Token[]): Reconciled {
  if (prev === next) return { text: next, tokens: [...tokens], gone: [] };
  const early = earlyEdit(prev, next);
  const late = lateEdit(prev, next);
  const edit = tokens.some((t) => touches(early, t)) && !tokens.some((t) => touches(late, t)) ? late : early;
  const added = next.slice(edit.from, next.length - (prev.length - edit.to));
  // text typed inside a token: after it
  if (edit.from === edit.to) {
    const inside = tokens.find((t) => touches(edit, t));
    if (inside) {
      const after = inside.at + inside.label.length;
      return {
        text: prev.slice(0, after) + added + prev.slice(after),
        tokens: tokens.map((t) => (t.at >= after ? { ...t, at: t.at + added.length } : t)),
        gone: [],
        cursor: after + added.length,
      };
    }
  }
  // what the edit reached, grown to whole tokens
  let from = edit.from;
  let to = edit.to;
  const gone: number[] = [];
  const kept: Token[] = [];
  for (const t of tokens) {
    if (touches(edit, t)) {
      gone.push(t.id);
      from = Math.min(from, t.at);
      to = Math.max(to, t.at + t.label.length);
    } else kept.push(t);
  }
  const shift = added.length - (to - from);
  return {
    text: prev.slice(0, from) + added + prev.slice(to),
    tokens: kept.map((t) => (t.at >= to ? { ...t, at: t.at + shift } : t)),
    gone,
    ...(gone.length > 0 ? { cursor: from + added.length } : {}),
  };
}

/** What one edit from `prev` to `next` put in, and over which span of `prev`. */
export function insertedBy(prev: string, next: string): { from: number; to: number; added: string } {
  const e = earlyEdit(prev, next);
  return { from: e.from, to: e.to, added: next.slice(e.from, next.length - (prev.length - e.to)) };
}

/** Where a cursor at `at` inside a token belongs (just after it); null when it is in no token. */
export function cursorOutOfToken(at: number, tokens: readonly Pick<Token, "at" | "label">[]): number | null {
  const t = tokens.find((k) => k.at >= 0 && at > k.at && at < k.at + k.label.length);
  return t ? t.at + t.label.length : null;
}

const editSize = (prev: string, next: string) => {
  const e = earlyEdit(prev, next);
  return e.to - e.from + (next.length - (prev.length - (e.to - e.from)));
};

/**
 * Whether the field's `next` was made from `typed`, the text it held before the draft corrected it to
 * `corrected`, not from the correction: a key that came before the field took the correction (a held
 * Backspace over a token). Reconciling it would bring back what the correction took, as plain text.
 */
export const staleField = (typed: string, corrected: string, next: string) => next !== corrected && editSize(typed, next) < editSize(corrected, next);

/** The draft with text put at the selection, whatever it holds. */
export function insertText(prev: string, tokens: readonly Token[], sel: Selection, text: string): Reconciled {
  const start = Math.min(sel.start, prev.length);
  const end = Math.min(sel.end, prev.length);
  const r = reconcile(prev, prev.slice(0, start) + text + prev.slice(end), tokens);
  return r.cursor === undefined ? { ...r, cursor: start + text.length } : r;
}

/** The draft with a new token at the selection; a cursor inside a token moves past it. */
export function placeToken(prev: string, tokens: readonly Token[], sel: Selection, id: number, label: string): Reconciled {
  const cleared = insertText(prev, tokens, sel, "");
  const cursor = cleared.cursor!;
  const inside = cleared.tokens.find((t) => cursor > t.at && cursor < t.at + t.label.length);
  const at = inside ? inside.at + inside.label.length : cursor;
  return {
    text: cleared.text.slice(0, at) + label + cleared.text.slice(at),
    tokens: [...cleared.tokens.map((t) => (t.at >= at ? { ...t, at: t.at + label.length } : t)), { id, at, label }].sort(byPlace),
    gone: cleared.gone,
    cursor: at + label.length,
  };
}

/** A draft in the order it goes out. */
export type DraftPart = { text: string } | { token: Token };

/** The draft cut at its tokens, nothing empty. */
export function draftParts(text: string, tokens: readonly Token[]): DraftPart[] {
  const parts: DraftPart[] = [];
  let at = 0;
  for (const t of [...tokens].sort(byPlace)) {
    if (t.at > at) parts.push({ text: text.slice(at, t.at) });
    parts.push({ token: t });
    at = t.at + t.label.length;
  }
  if (at < text.length) parts.push({ text: text.slice(at) });
  return parts;
}
