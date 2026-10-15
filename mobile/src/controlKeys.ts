// The control-key row above the command line and under the terminal screen: which keys it
// holds and in what order (Settings › Control keys). Pure; the ids are stored in settings.

import type { OpenKind } from "./protocol/types";
import type { IconName } from "./ui/Icon";

/**
 * A key of the row: a protocol key name sent with `keys`, a sticky modifier for the next key, an
 * action on the computer (`open`: a new tab or pane, feature `open`), or the clipboard (`paste`).
 */
export interface ControlKey {
  /** Stored id: the protocol key name, `ctrl` / `alt` / `shift` for the sticky modifiers, `new_tab` / `new_pane`, `paste`. */
  id: string;
  /** What the round key shows when it has no icon ("Esc", "⌃", "⌃D", "F5"). */
  label: string;
  /** The key's icon (the Keys board's arrows, interrupt, paste); drawn instead of the label. */
  icon?: IconName;
  /** What screen readers say. */
  spoken: string;
  /** What Settings › Keys calls it, when shorter than `spoken` ("Interrupt"). */
  name?: string;
  /** The Add chip's word beside the key ("Left", "End input"), when not `name`. */
  short?: string;
  /** Applies to the next letter typed or key pressed instead of being sent. */
  sticky?: boolean;
  /** Interrupts the program: marked in the row. */
  danger?: boolean;
  /** Opens a new tab or pane on the computer instead of sending a key. */
  open?: OpenKind;
  /** Types the phone's clipboard into the pane (feature `text`). */
  paste?: boolean;
}

const fkeys: ControlKey[] = Array.from({ length: 12 }, (_, i) => ({
  id: `f${i + 1}`,
  label: `F${i + 1}`,
  spoken: `F${i + 1}`,
}));

/** Every key the row can hold, in the order Settings lists them. */
export const CONTROL_KEYS: readonly ControlKey[] = [
  { id: "ctrl+c", label: "⌃C", icon: "interrupt", spoken: "Control C, interrupt", name: "Interrupt", danger: true },
  { id: "esc", label: "Esc", spoken: "Escape" },
  { id: "tab", label: "Tab", spoken: "Tab" },
  { id: "shift+tab", label: "⇧Tab", spoken: "Shift Tab", short: "Shift Tab" },
  { id: "ctrl", label: "⌃", spoken: "Control, applies to the next key", name: "Control the next key", short: "Control", sticky: true },
  { id: "alt", label: "⌥", spoken: "Alt, applies to the next key", name: "Alt the next key", short: "Alt", sticky: true },
  { id: "shift", label: "⇧", spoken: "Shift, applies to the next key", name: "Shift the next key", short: "Shift", sticky: true },
  { id: "up", label: "↑", icon: "up", spoken: "Up arrow", short: "Up" },
  { id: "down", label: "↓", icon: "down", spoken: "Down arrow", short: "Down" },
  { id: "left", label: "←", icon: "left", spoken: "Left arrow", short: "Left" },
  { id: "right", label: "→", icon: "right", spoken: "Right arrow", short: "Right" },
  { id: "home", label: "Home", icon: "home", spoken: "Home" },
  { id: "end", label: "End", icon: "end", spoken: "End" },
  { id: "pageup", label: "PgUp", icon: "pageUp", spoken: "Page up" },
  { id: "pagedown", label: "PgDn", icon: "pageDown", spoken: "Page down" },
  { id: "backspace", label: "⌫", icon: "backspace", spoken: "Backspace" },
  { id: "delete", label: "Del", icon: "deleteKey", spoken: "Delete" },
  { id: "insert", label: "Ins", spoken: "Insert" },
  { id: "ctrl+d", label: "⌃D", spoken: "Control D, end of input", short: "End input" },
  { id: "ctrl+z", label: "⌃Z", spoken: "Control Z, suspend", short: "Suspend" },
  { id: "ctrl+l", label: "⌃L", spoken: "Control L, redraw", short: "Redraw" },
  { id: "ctrl+r", label: "⌃R", spoken: "Control R, search history", short: "History" },
  { id: "ctrl+a", label: "⌃A", spoken: "Control A", short: "Line start" },
  { id: "ctrl+e", label: "⌃E", spoken: "Control E", short: "Line end" },
  { id: "ctrl+u", label: "⌃U", spoken: "Control U", short: "Clear line" },
  { id: "ctrl+w", label: "⌃W", spoken: "Control W", short: "Delete word" },
  { id: "paste", label: "Paste", icon: "paste", spoken: "Paste the clipboard", name: "Paste", paste: true },
  { id: "new_tab", label: "+Tab", icon: "newTab", spoken: "New tab", name: "New tab on the computer", short: "New tab", open: "tab" },
  { id: "new_pane", label: "+Pane", icon: "newPane", spoken: "New pane", name: "New pane beside this one", short: "New pane", open: "pane" },
  ...fkeys,
];

/** What Settings › Keys names a key ("Interrupt", "Up arrow"). */
export const keyName = (k: ControlKey) => k.name ?? k.spoken;

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
