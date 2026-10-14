// The control-key row above the command line and under the terminal screen: which keys it
// holds and in what order (Settings › Control keys). Pure; the ids are stored in settings.

import type { OpenKind } from "./protocol/types";

/**
 * A key of the row: a protocol key name sent with `keys`, a sticky modifier for the next key, or
 * an action on the computer (`open`: a new tab or pane, feature `open`).
 */
export interface ControlKey {
  /** Stored id: the protocol key name, `ctrl` / `alt` for the sticky modifiers, `new_tab` / `new_pane`. */
  id: string;
  label: string;
  /** What screen readers say. */
  spoken: string;
  /** What Settings › Keys calls it, when shorter than `spoken` ("Interrupt"). */
  name?: string;
  /** Applies to the next letter typed or key pressed instead of being sent. */
  sticky?: boolean;
  /** Interrupts the program: marked in the row. */
  danger?: boolean;
  /** Opens a new tab or pane on the computer instead of sending a key. */
  open?: OpenKind;
}

const fkeys: ControlKey[] = Array.from({ length: 12 }, (_, i) => ({
  id: `f${i + 1}`,
  label: `F${i + 1}`,
  spoken: `F${i + 1}`,
}));

/** Every key the row can hold, in the order Settings lists them. */
export const CONTROL_KEYS: readonly ControlKey[] = [
  { id: "ctrl+c", label: "^C", spoken: "Control C, interrupt", name: "Interrupt", danger: true },
  { id: "esc", label: "esc", spoken: "Escape" },
  { id: "tab", label: "tab", spoken: "Tab" },
  { id: "shift+tab", label: "⇧tab", spoken: "Shift Tab" },
  { id: "ctrl", label: "ctrl", spoken: "Control, applies to the next key", name: "Control the next key", sticky: true },
  { id: "alt", label: "alt", spoken: "Alt, applies to the next key", name: "Alt the next key", sticky: true },
  { id: "up", label: "↑", spoken: "Up arrow" },
  { id: "down", label: "↓", spoken: "Down arrow" },
  { id: "left", label: "←", spoken: "Left arrow" },
  { id: "right", label: "→", spoken: "Right arrow" },
  { id: "home", label: "home", spoken: "Home" },
  { id: "end", label: "end", spoken: "End" },
  { id: "pageup", label: "pgup", spoken: "Page up" },
  { id: "pagedown", label: "pgdn", spoken: "Page down" },
  { id: "backspace", label: "⌫", spoken: "Backspace" },
  { id: "delete", label: "del", spoken: "Delete" },
  { id: "insert", label: "ins", spoken: "Insert" },
  { id: "ctrl+d", label: "^D", spoken: "Control D, end of input" },
  { id: "ctrl+z", label: "^Z", spoken: "Control Z, suspend" },
  { id: "ctrl+l", label: "^L", spoken: "Control L, redraw" },
  { id: "ctrl+r", label: "^R", spoken: "Control R, search history" },
  { id: "ctrl+a", label: "^A", spoken: "Control A" },
  { id: "ctrl+e", label: "^E", spoken: "Control E" },
  { id: "ctrl+u", label: "^U", spoken: "Control U" },
  { id: "ctrl+w", label: "^W", spoken: "Control W" },
  { id: "new_tab", label: "+tab", spoken: "New tab", name: "New tab on the computer", open: "tab" },
  { id: "new_pane", label: "+pane", spoken: "New pane", name: "New pane beside this one", open: "pane" },
  ...fkeys,
];

const BY_ID = new Map(CONTROL_KEYS.map((k) => [k.id, k]));

export function controlKey(id: string): ControlKey | undefined {
  return BY_ID.get(id);
}

/** The default row: ^C esc tab ctrl ↑ ↓ (Settings › Keys). */
export const DEFAULT_CONTROL_KEYS: readonly string[] = ["ctrl+c", "esc", "tab", "ctrl", "up", "down"];

/** A stored row: known ids, each once, in order; anything else (not an array) is the default. */
export function validControlKeys(v: unknown): string[] {
  if (!Array.isArray(v)) return [...DEFAULT_CONTROL_KEYS];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of v) {
    if (typeof id !== "string" || !BY_ID.has(id) || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

/** `ids` with the key at `index` moved one place up (-1) or down (+1); unchanged at an end. */
export function moveKey(ids: readonly string[], index: number, by: -1 | 1): string[] {
  const to = index + by;
  if (index < 0 || index >= ids.length || to < 0 || to >= ids.length) return [...ids];
  const next = [...ids];
  [next[index], next[to]] = [next[to]!, next[index]!];
  return next;
}

/** `ids` with the key at `from` moved to `to` (drag to reorder); clamped to the row. */
export function moveKeyTo(ids: readonly string[], from: number, to: number): string[] {
  if (from < 0 || from >= ids.length) return [...ids];
  const target = Math.max(0, Math.min(ids.length - 1, to));
  const next = [...ids];
  const [id] = next.splice(from, 1);
  next.splice(target, 0, id!);
  return next;
}

/** The Add chips of Settings › Keys: unused keys, the F-keys as one "F1–F12" while none is in the row. */
export function addChoices(ids: readonly string[]): { label: string; ids: string[] }[] {
  const unused = CONTROL_KEYS.filter((k) => !ids.includes(k.id));
  const f = unused.filter((k) => /^f\d+$/.test(k.id));
  const allF = f.length === 12;
  const out: { label: string; ids: string[] }[] = [];
  for (const k of unused) {
    if (allF && /^f\d+$/.test(k.id)) {
      if (k.id === "f1") out.push({ label: "F1–F12", ids: f.map((x) => x.id) });
      continue;
    }
    out.push({ label: k.label, ids: [k.id] });
  }
  return out;
}

/** `ids` with `id` added at the end (on) or removed (off). */
export function toggleKey(ids: readonly string[], id: string, on: boolean): string[] {
  if (on && ids.includes(id)) return [...ids];
  const without = ids.filter((k) => k !== id);
  return on && BY_ID.has(id) ? [...without, id] : without;
}
