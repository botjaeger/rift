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
  /** What the key does, for the long-press tip and the screen reader's hint ("Sends Ctrl+C: stops the running command"). */
  tip: string;
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
  tip: `Sends F${i + 1}: programs give it their own meaning`,
  label: `F${i + 1}`,
  spoken: `F${i + 1}`,
}));

/** Every key the row can hold, in the order Settings lists them. */
export const CONTROL_KEYS: readonly ControlKey[] = [
  { id: "ctrl+c", tip: "Sends Ctrl+C: stops the running command", label: "⌃C", icon: "interrupt", spoken: "Control C, interrupt", name: "Interrupt", danger: true },
  { id: "esc", tip: "Sends Escape: leaves a mode, cancels a prompt, closes a menu", label: "Esc", spoken: "Escape" },
  { id: "tab", tip: "Sends Tab: completes what you typed, or moves to the next field", label: "Tab", spoken: "Tab" },
  { id: "shift+tab", tip: "Sends Shift+Tab: moves back, or to the previous option", label: "⇧Tab", spoken: "Shift Tab", short: "Shift Tab" },
  { id: "ctrl", tip: "Holds Control for the next key you type or press: Ctrl, then R is Ctrl+R", label: "⌃", spoken: "Control, applies to the next key", name: "Control the next key", short: "Control", sticky: true },
  { id: "alt", tip: "Holds Alt for the next key you type or press: Alt, then B moves back a word", label: "⌥", spoken: "Alt, applies to the next key", name: "Alt the next key", short: "Alt", sticky: true },
  { id: "shift", tip: "Holds Shift for the next key you type or press", label: "⇧", spoken: "Shift, applies to the next key", name: "Shift the next key", short: "Shift", sticky: true },
  { id: "up", tip: "Sends the up arrow: the previous command, or up in a menu", label: "↑", icon: "up", spoken: "Up arrow", short: "Up" },
  { id: "down", tip: "Sends the down arrow: the next command, or down in a menu", label: "↓", icon: "down", spoken: "Down arrow", short: "Down" },
  { id: "left", tip: "Sends the left arrow: moves the cursor one character left", label: "←", icon: "left", spoken: "Left arrow", short: "Left" },
  { id: "right", tip: "Sends the right arrow: moves the cursor one character right", label: "→", icon: "right", spoken: "Right arrow", short: "Right" },
  { id: "home", tip: "Sends Home: jumps to the start of the line", label: "Home", icon: "home", spoken: "Home" },
  { id: "end", tip: "Sends End: jumps to the end of the line", label: "End", icon: "end", spoken: "End" },
  { id: "pageup", tip: "Sends Page Up: scrolls a screen up in a pager or editor", label: "PgUp", icon: "pageUp", spoken: "Page up" },
  { id: "pagedown", tip: "Sends Page Down: scrolls a screen down in a pager or editor", label: "PgDn", icon: "pageDown", spoken: "Page down" },
  { id: "backspace", tip: "Sends Backspace: deletes the character before the cursor", label: "⌫", icon: "backspace", spoken: "Backspace" },
  { id: "delete", tip: "Sends Delete: deletes the character under the cursor", label: "Del", icon: "deleteKey", spoken: "Delete" },
  { id: "insert", tip: "Sends Insert: switches insert and overwrite in editors", label: "Ins", spoken: "Insert" },
  { id: "ctrl+d", tip: "Sends Ctrl+D: ends input, and closes the shell on an empty line", label: "⌃D", spoken: "Control D, end of input", short: "End input" },
  { id: "ctrl+z", tip: "Sends Ctrl+Z: suspends the running command; fg resumes it", label: "⌃Z", spoken: "Control Z, suspend", short: "Suspend" },
  { id: "ctrl+l", tip: "Sends Ctrl+L: clears and redraws the screen", label: "⌃L", spoken: "Control L, redraw", short: "Redraw" },
  { id: "ctrl+r", tip: "Sends Ctrl+R: searches the command history", label: "⌃R", spoken: "Control R, search history", short: "History" },
  { id: "ctrl+a", tip: "Sends Ctrl+A: jumps to the start of the line", label: "⌃A", spoken: "Control A", short: "Line start" },
  { id: "ctrl+e", tip: "Sends Ctrl+E: jumps to the end of the line", label: "⌃E", spoken: "Control E", short: "Line end" },
  { id: "ctrl+u", tip: "Sends Ctrl+U: deletes from the cursor to the start of the line", label: "⌃U", spoken: "Control U", short: "Clear line" },
  { id: "ctrl+w", tip: "Sends Ctrl+W: deletes the word before the cursor", label: "⌃W", spoken: "Control W", short: "Delete word" },
  { id: "paste", tip: "Pastes the phone's clipboard: text as typed, an image (a screenshot) as a file the program attaches", label: "Paste", icon: "paste", spoken: "Paste the clipboard", name: "Paste", paste: true },
  { id: "new_tab", tip: "Opens a new tab on your computer", label: "+Tab", icon: "newTab", spoken: "New tab", name: "New tab on the computer", short: "New tab", open: "tab" },
  { id: "new_pane", tip: "Splits this pane: a new pane beside it on your computer", label: "+Pane", icon: "newPane", spoken: "New pane", name: "New pane beside this one", short: "New pane", open: "pane" },
  ...fkeys,
];

/** What Settings › Keys names a key ("Interrupt", "Up arrow"). */
export const keyName = (k: ControlKey) => k.name ?? k.spoken;

const BY_ID = new Map(CONTROL_KEYS.map((k) => [k.id, k]));

export function controlKey(id: string): ControlKey | undefined {
  return BY_ID.get(id);
}

/** The default row: Esc Tab ⌃C ⌃ ← ↑ ↓ → (Settings › Keys). */
export const DEFAULT_CONTROL_KEYS: readonly string[] = ["esc", "tab", "ctrl+c", "ctrl", "left", "up", "down", "right"];
/** The default before the arrows joined: a row stored as exactly this moves to the new default. */
export const OLD_DEFAULT_CONTROL_KEYS: readonly string[] = ["ctrl+c", "esc", "tab", "ctrl", "up", "down"];
/** Keys the row holds at most (the hide-keyboard key comes besides). */
export const MAX_ROW_KEYS = 8;

/**
 * The row as shown: `ids`, with Paste at the end where an image can be pasted (`images`: the
 * computer takes uploads and this device operates) and the row lacks it. A screenshot for Claude
 * Code or vim goes through Paste, so it is never left to Settings › Keys to add (the stored row
 * stays as the user set it).
 */
export function rowWithPaste(ids: readonly string[], images: boolean): readonly string[] {
  return images && !ids.includes("paste") ? [...ids, "paste"] : ids;
}

/** Whether the row has room for another key. */
export const canAddKey = (ids: readonly string[]) => ids.length < MAX_ROW_KEYS;

const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, i) => id === b[i]);

/**
 * A stored row: known ids, each once, in order; anything else (not an array) is the default, and
 * so is the previous default. A customised row is kept as stored, even longer than the maximum.
 */
export function validControlKeys(v: unknown): string[] {
  if (!Array.isArray(v)) return [...DEFAULT_CONTROL_KEYS];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of v) {
    if (typeof id !== "string" || !BY_ID.has(id) || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return sameIds(out, OLD_DEFAULT_CONTROL_KEYS) ? [...DEFAULT_CONTROL_KEYS] : out;
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

/** The Add chips of Settings › Keys: every unused key (the row holds 8, so no F1–F12 group). */
export function addChoices(ids: readonly string[]): { label: string; ids: string[] }[] {
  return CONTROL_KEYS.filter((k) => !ids.includes(k.id)).map((k) => ({ label: k.label, ids: [k.id] }));
}

/** `ids` with `id` added at the end (on; unchanged when the row is full) or removed (off). */
export function toggleKey(ids: readonly string[], id: string, on: boolean): string[] {
  if (on && (ids.includes(id) || !canAddKey(ids))) return [...ids];
  const without = ids.filter((k) => k !== id);
  return on && BY_ID.has(id) ? [...without, id] : without;
}

/** What a key is called in its long-press tip: "⌃C · Interrupt". */
export const keyTipTitle = (k: ControlKey) => `${k.label} · ${k.name ?? k.short ?? k.spoken}`;

/**
 * One touch on a key: a press that lasted long enough to show the tip is not a tap. The row calls
 * `begin` when the finger goes down, `longPress` when the tip shows and `release` when it lifts; `release` says whether the
 * key is sent (a short press) or not (the tip was shown).
 */
export class PressGate {
  private long = false;
  /** A finger went down: a new touch. */
  begin(): void {
    this.long = false;
  }
  longPress(): void {
    this.long = true;
  }
  release(): "send" | "none" {
    const sent = this.long ? "none" : "send";
    this.long = false;
    return sent;
  }
}
