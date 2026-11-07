// The key block under the command line and under the terminal screen: which keys it holds and
// in what order (Settings › Keys), the arrow pad beside it, and a held arrow's repeat. Pure; the
// ids are stored in settings.

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
  /** A modifier's icon drawn before the label ("⇧" before "Tab"): no font glyph for a symbol. */
  lead?: IconName;
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
  { id: "shift+tab", tip: "Sends Shift+Tab: moves back, or to the previous option", label: "⇧Tab", lead: "shift", spoken: "Shift Tab", short: "Back tab" },
  { id: "enter", tip: "Sends Return: confirms a menu or a picker in the program. With text in the message box, a new line there", label: "↵", icon: "enter", spoken: "Return", name: "Return", short: "Return" },
  { id: "ctrl+j", tip: "Starts a new line in the message, without sending it", label: "New line", icon: "newLine", spoken: "New line", name: "New line", short: "New line" },
  { id: "ctrl", tip: "Holds Control for the next key you type or press: Ctrl, then R is Ctrl+R", label: "⌃", icon: "control", spoken: "Control, held for the next key", short: "Control", sticky: true },
  { id: "alt", tip: "Holds Alt for the next key you type or press: Alt, then B moves back a word", label: "⌥", icon: "option", spoken: "Alt, held for the next key", short: "Alt", sticky: true },
  { id: "shift", tip: "Holds Shift for the next key you type or press", label: "⇧", icon: "shift", spoken: "Shift, held for the next key", short: "Shift", sticky: true },
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
  { id: "paste", tip: "Puts the phone's clipboard in the message box: text, or an image or file (a screenshot) the program attaches on Send", label: "Paste", icon: "paste", spoken: "Paste the clipboard", name: "Paste", paste: true },
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

/** The arrow pad's keys, in its order (↑ above ← ↓ →): beside the key block, never in it. */
export const ARROW_KEYS: readonly string[] = ["up", "left", "down", "right"];
const isArrow = (id: string) => ARROW_KEYS.includes(id);

/**
 * The default key block (the Keys redesign board): Esc Tab ⌃ Return Paste, then the keyboard key,
 * which is always the block's last. ⌃C is ⌃ then C; New line is in Settings › Keys, not here: the phone keyboard's Return is a new line.
 */
export const DEFAULT_CONTROL_KEYS: readonly string[] = ["esc", "tab", "ctrl", "enter", "paste"];
/** The default before the arrows joined: a row stored as exactly this moves to the new default. */
export const OLD_DEFAULT_CONTROL_KEYS: readonly string[] = ["ctrl+c", "esc", "tab", "ctrl", "up", "down"];
/** The default before Return and New line joined: a row stored as exactly this moves too. */
const ARROWS_DEFAULT_CONTROL_KEYS: readonly string[] = ["esc", "tab", "ctrl+c", "ctrl", "left", "up", "down", "right"];
/** The default with New line after Return: a row stored as exactly this moves too. */
const NEWLINE_DEFAULT_CONTROL_KEYS: readonly string[] = ["esc", "tab", "ctrl+c", "ctrl", "up", "down", "enter", "ctrl+j"];
/** The one-line row's default, before the key block and arrow pad: a row stored as exactly this moves too. */
export const ROW_DEFAULT_CONTROL_KEYS: readonly string[] = ["esc", "tab", "ctrl+c", "ctrl", "up", "down", "enter"];
/** Places in the key block, the keyboard key's included (Settings › Keys: "6 of 8 in the key block"). */
export const BLOCK_SLOTS = 8;
/** Keys Settings › Keys can put in the block: every place but the keyboard key's. */
export const MAX_ROW_KEYS = BLOCK_SLOTS - 1;

/**
 * The block as shown: `ids`, with Paste at the end where an image can be pasted (`images`: the
 * computer takes uploads and this device operates) and the block lacks it. A screenshot for Claude
 * Code or vim starts with Paste, so it is never left to Settings › Keys to add (the stored list
 * stays as the user set it).
 */
export function rowWithPaste(ids: readonly string[], images: boolean): readonly string[] {
  return images && !ids.includes("paste") ? [...ids, "paste"] : ids;
}

/**
 * The key block's columns for `count` keys (the keyboard key included): 3 x 2 as the board draws
 * six, wider when Settings › Keys holds more, so the block keeps its two rows and its 88 pt.
 */
export const blockColumns = (count: number): number => Math.max(3, Math.ceil(count / 2));

/** Whether the block has room for another key. */
export const canAddKey = (ids: readonly string[]) => ids.length < MAX_ROW_KEYS;

const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, i) => id === b[i]);

/**
 * A stored list: known ids, each once, in order; anything else (not an array) is the default, and
 * so is any earlier default. The arrows leave the list: the arrow pad always has them, so a key
 * nobody chose to drop is never lost. A customised list is kept otherwise, even longer than the maximum.
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
  const earlier = [OLD_DEFAULT_CONTROL_KEYS, ARROWS_DEFAULT_CONTROL_KEYS, NEWLINE_DEFAULT_CONTROL_KEYS, ROW_DEFAULT_CONTROL_KEYS];
  return earlier.some((d) => sameIds(out, d)) ? [...DEFAULT_CONTROL_KEYS] : out.filter((id) => !isArrow(id));
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

/** What a search in Settings › Keys matches a key by: its label, its names, its id ("pgup", "page", "f5"). */
const searchText = (k: ControlKey) => [k.label, k.name, k.short, k.spoken, k.id].filter(Boolean).join(" ").toLowerCase();

/** The function keys' one Add chip ("F1–F12"): a tap lays out the twelve. */
export const FKEYS_CHIP = "F1–F12";

/**
 * Add a key's order (the Keys, adding board): the default block's keys first (offered once
 * removed), then the board's chips (⇧Tab, ⌥, ⇧, Home, End, Page up, Page down, ⌃D ⌃Z ⌃R ⌃L ⌃U,
 * the function keys), then the keys the app had before that board (Interrupt, New line, ⌫, Del,
 * Ins, ⌃A ⌃E ⌃W, New tab, New pane).
 */
const ADD_ORDER: readonly string[] = [
  ...DEFAULT_CONTROL_KEYS,
  "shift+tab", "alt", "shift", "home", "end", "pageup", "pagedown",
  "ctrl+d", "ctrl+z", "ctrl+r", "ctrl+l", "ctrl+u",
  ...fkeys.map((k) => k.id),
  "ctrl+c", "ctrl+j", "backspace", "delete", "insert", "ctrl+a", "ctrl+e", "ctrl+w", "new_tab", "new_pane",
];
const rank = (id: string) => {
  const i = ADD_ORDER.indexOf(id);
  return i < 0 ? ADD_ORDER.length : i;
};
const isFKey = (id: string) => /^f\d+$/.test(id);

/**
 * Settings › Keys' Add a key: every key not in the block and not on the arrow pad, matching
 * `query` (any word of its label or names; empty: all of them), in the board's order. With no
 * query the function keys left to add are one chip (`FKEYS_CHIP`, its `ids` all of them) unless
 * `everyFKey`; a search finds each one.
 */
export function addChoices(ids: readonly string[], query = "", everyFKey = false): { label: string; ids: string[] }[] {
  const q = query.trim().toLowerCase();
  const keys = CONTROL_KEYS.filter((k) => !ids.includes(k.id) && !isArrow(k.id) && (q === "" || searchText(k).includes(q))).sort((a, b) => rank(a.id) - rank(b.id));
  const out: { label: string; ids: string[] }[] = [];
  for (const k of keys) {
    if (q === "" && !everyFKey && isFKey(k.id)) {
      const group = out.find((c) => c.label === FKEYS_CHIP);
      if (group) group.ids.push(k.id);
      else out.push({ label: FKEYS_CHIP, ids: [k.id] });
    } else out.push({ label: k.label, ids: [k.id] });
  }
  return out;
}

/** `ids` with `id` added at the end (on; unchanged when the row is full) or removed (off). */
export function toggleKey(ids: readonly string[], id: string, on: boolean): string[] {
  if (on && (ids.includes(id) || !canAddKey(ids))) return [...ids];
  const without = ids.filter((k) => k !== id);
  return on && BY_ID.has(id) && !isArrow(id) ? [...without, id] : without;
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

/** A held arrow repeats after this long (milliseconds), as a hardware keyboard's does. */
export const REPEAT_DELAY_MS = 400;
/** Then once every this long. */
export const REPEAT_EVERY_MS = 80;

/**
 * A held arrow on the pad: the key goes once when the finger goes down (`start`), again after
 * `REPEAT_DELAY_MS`, then every `REPEAT_EVERY_MS` until `stop` (the finger lifts, the touch is
 * cancelled, the key is disabled, the view goes away or the app leaves the foreground). `tap` is
 * the press that follows: it sends nothing after a touch (the touch sent already) and once for a
 * screen reader's activation, which has no touch.
 */
export class KeyRepeat {
  private delay: ReturnType<typeof setTimeout> | null = null;
  private every: ReturnType<typeof setInterval> | null = null;
  private touched = false;
  // a touch that ends without a press (the finger slid off) forgets itself once the release
  // handler has run, so the next screen reader activation still goes
  private forget: ReturnType<typeof setTimeout> | null = null;
  start(fire: () => void): void {
    this.stop();
    this.cancelForget();
    this.touched = true;
    fire();
    this.delay = setTimeout(() => {
      this.delay = null;
      this.every = setInterval(fire, REPEAT_EVERY_MS);
    }, REPEAT_DELAY_MS);
  }
  stop(): void {
    if (this.delay !== null) clearTimeout(this.delay);
    if (this.every !== null) clearInterval(this.every);
    this.delay = null;
    this.every = null;
    if (this.touched && this.forget === null) {
      this.forget = setTimeout(() => {
        this.forget = null;
        this.touched = false;
      }, 0);
    }
  }
  tap(fire: () => void): void {
    this.cancelForget();
    if (!this.touched) fire();
    this.touched = false;
  }
  private cancelForget(): void {
    if (this.forget !== null) clearTimeout(this.forget);
    this.forget = null;
  }
  /** Whether a repeat is running (tests). */
  get held(): boolean {
    return this.delay !== null || this.every !== null;
  }
}
