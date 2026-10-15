// Keyboard input of the terminal view and the composer's hardware shortcuts, turned into
// protocol actions: `keys` (key names, docs/remote-protocol.md) or `text` (printable text, one
// line). Pure; the native key capture view (modules/rift-keys) delivers the events, and
// `InputBatcher` sends them at most once per frame.

import { CONTROL_CHARS, MAX_KEYS_PER_MESSAGE, MAX_TEXT_BYTES, isKeyName, utf8Length } from "./protocol/types";

/** A key press as the capture view reports it: `key` is a W3C KeyboardEvent.key value. */
export interface KeyEventLike {
  /** The key's character with shift only (Android, iOS), or what it typed (web). */
  key: string;
  code?: string;
  /**
   * The character the layout composes with every modifier held (AltGr+Q → "@", Option+2 → "™");
   * empty or absent when none is known.
   */
  char?: string;
  /** AltGr (right Alt on a layout that composes with it, or the web's AltGraph state). */
  altGraph?: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export type InputAction = { keys: string[] } | { text: string };

/** W3C key values → protocol base keys (those that take any modifiers). */
const NAMED: Record<string, string> = {
  Escape: "esc",
  Esc: "esc",
  Tab: "tab",
  Enter: "enter",
  Backspace: "backspace",
  Delete: "delete",
  Insert: "insert",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Home: "home",
  End: "end",
  PageUp: "pageup",
  PageDown: "pagedown",
  ...Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`F${i + 1}`, `f${i + 1}`])),
};

function withMods(base: string, ctrl: boolean, alt: boolean, shift: boolean): string {
  return `${ctrl ? "ctrl+" : ""}${alt ? "alt+" : ""}${shift ? "shift+" : ""}${base}`;
}

/** One printable character (a surrogate pair counts as one), or null. */
function printableChar(c: string | undefined): string | null {
  return c && [...c].length === 1 && !CONTROL_CHARS.test(c) ? c : null;
}

/**
 * The character a ctrl/alt key press types instead of being a key: AltGr and Option
 * combinations (AltGr+Q → "@", Option+2 → "™") compose a printable character other than the
 * key's own. With ctrl held it is a key, unless AltGr says the layout composed it (AltGr is
 * ctrl+alt on Windows). Null when the press is not composed text.
 */
export function composedText(e: KeyEventLike): string | null {
  if (e.meta || (!e.ctrl && !e.alt)) return null;
  const c = printableChar(e.char);
  if (c === null) return null;
  if (e.altGraph) return c;
  return !e.ctrl && c !== e.key ? c : null;
}

/**
 * One key press → an action, or null when it is nothing to send: Cmd/Meta combinations (left
 * to the system), lone modifiers, dead keys, and combinations the protocol has no name for.
 * Characters composed with AltGr / Option are text (`composedText`). Letters and space with
 * ctrl and/or alt become keys (shift dropped: `ctrl+A` is `ctrl+a`); `ctrl+[` is esc and
 * `ctrl+@` / `ctrl+2` is `ctrl+space`, as on a terminal; another ctrl/alt combination types
 * its composed character when it has one. Other characters are text.
 */
export function keyEventToAction(e: KeyEventLike): InputAction | null {
  if (e.meta) return null;
  const named = NAMED[e.key];
  if (named) {
    const name = withMods(named, e.ctrl, e.alt, e.shift);
    return isKeyName(name) ? { keys: [name] } : null;
  }
  // A printable key reports one character (a surrogate pair counts as one).
  if ([...e.key].length !== 1) return null;
  if (!e.ctrl && !e.alt) return textAction(e.key);
  const composed = composedText(e);
  if (composed !== null) return { text: composed };
  const ch = e.key.toLowerCase();
  let base: string | null = null;
  if (/^[a-z]$/.test(ch)) base = ch;
  else if (ch === " " || (e.ctrl && (ch === "@" || ch === "2"))) base = "space";
  else if (e.ctrl && !e.alt && ch === "[") return { keys: ["esc"] };
  const name = base === null ? null : withMods(base, e.ctrl, e.alt, false);
  if (name !== null && isKeyName(name)) return { keys: [name] };
  // no name for it: what the layout composed, if anything
  const c = printableChar(e.char);
  return c === null ? null : { text: c };
}

const CONTROL_KEY: Record<string, string> = {
  "\r": "enter",
  "\n": "enter",
  "\t": "tab",
  "\b": "backspace",
  "\u007f": "backspace",
  "\u001b": "esc",
};

function controlName(ch: string): string | null {
  const named = CONTROL_KEY[ch];
  if (named) return named;
  const cp = ch.codePointAt(0)!;
  // C0 control characters: ^A … ^Z (^H, ^I, ^J, ^M are handled above), ^@ is ctrl+space.
  if (cp === 0) return "ctrl+space";
  if (cp >= 1 && cp <= 26) return `ctrl+${String.fromCharCode(cp + 96)}`;
  return null;
}

function textAction(text: string): InputAction | null {
  return CONTROL_CHARS.test(text) ? null : { text };
}

/**
 * Text the keyboard inserted → actions in order: printable runs as text, line breaks, tabs,
 * backspace and escape characters as keys (the protocol's `text` carries no control
 * characters), other C0 controls as ctrl+letter; anything else unprintable is dropped. A
 * "\r\n" pair is one Enter.
 */
export function textToActions(text: string): InputAction[] {
  const out: InputAction[] = [];
  let run = "";
  const chars = [...text];
  for (let i = 0; i < chars.length; i++) {
    const ch = chars[i]!;
    const cp = ch.codePointAt(0)!;
    if (cp >= 0x20 && cp !== 0x7f && !(cp >= 0x80 && cp < 0xa0)) {
      run += ch;
      continue;
    }
    if (run) out.push({ text: run });
    run = "";
    if (ch === "\r" && chars[i + 1] === "\n") i++;
    const name = controlName(ch);
    if (name) out.push({ keys: [name] });
  }
  if (run) out.push({ text: run });
  return out;
}

/**
 * Sticky ctrl/alt/shift from the control row applied to the next key: `up` with ctrl is
 * `ctrl+up`, `tab` with shift is `shift+tab`, a letter typed with alt is `alt+b`. A key that
 * already carries the modifier keeps it; a combination the protocol has no name for is sent as
 * it was.
 */
export function applySticky(key: string, mods: { ctrl: boolean; alt: boolean; shift?: boolean }): string {
  const m = /^(ctrl\+)?(alt\+)?(shift\+)?(.+)$/.exec(key);
  if (!m) return key;
  const [, ctrl, alt, shift, base] = m;
  const name = withMods(base!, !!ctrl || mods.ctrl, !!alt || mods.alt, !!shift || !!mods.shift);
  return isKeyName(name) ? name : key;
}

/**
 * One character typed while sticky ctrl/alt is on, as a key name: a letter or space with the
 * modifiers, a line break, tab, backspace or escape as that key with them (`ctrl+enter`) when
 * the protocol has the name, else the plain key; `ctrl+[` is esc and `ctrl+@` is `ctrl+space`.
 * Null when the character takes no modifier (it is typed as text, the sticky dropped).
 */
export function stickyKey(ch: string, mods: { ctrl: boolean; alt: boolean }): string | null {
  if (!mods.ctrl && !mods.alt) return null;
  const [a, ...rest] = textToActions(ch);
  if (a && "keys" in a && rest.length === 0 && a.keys.length === 1) return applySticky(a.keys[0]!, mods);
  const c = ch.toLowerCase();
  if (c === " " || /^[a-z]$/.test(c)) return withMods(c === " " ? "space" : c, mods.ctrl, mods.alt, false);
  if (mods.ctrl && c === "[") return withMods("esc", false, mods.alt, false);
  if (mods.ctrl && c === "@") return withMods("space", true, mods.alt, false);
  return null;
}

/** Splits `text` into pieces of at most `MAX_TEXT_BYTES` UTF-8 bytes, between characters. */
export function splitText(text: string, max = MAX_TEXT_BYTES): string[] {
  const out: string[] = [];
  let cur = "";
  let bytes = 0;
  for (const ch of text) {
    const n = utf8Length(ch);
    if (bytes + n > max && cur) {
      out.push(cur);
      cur = "";
      bytes = 0;
    }
    cur += ch;
    bytes += n;
  }
  if (cur) out.push(cur);
  return out;
}

/**
 * Merges queued actions in order: adjacent text joined (then split at the 1 KiB limit),
 * adjacent keys joined up to 20 per message.
 */
export function mergeActions(actions: readonly InputAction[]): InputAction[] {
  const out: InputAction[] = [];
  for (const a of actions) {
    const prev = out[out.length - 1];
    if ("text" in a && prev && "text" in prev) prev.text += a.text;
    else if ("keys" in a && prev && "keys" in prev && prev.keys.length + a.keys.length <= MAX_KEYS_PER_MESSAGE)
      prev.keys = [...prev.keys, ...a.keys];
    else out.push("text" in a ? { text: a.text } : { keys: [...a.keys] });
  }
  return out.flatMap((a): InputAction[] => ("text" in a ? splitText(a.text).map((text) => ({ text })) : [a]));
}

/**
 * Fewest milliseconds between two sends: the gateway takes 20 `keys`/`text` messages a second
 * per connection, so held keys (auto-repeat) and fast typing go out batched, never one message
 * per character.
 */
export const MIN_SEND_INTERVAL_MS = 50;

/** What may hold input back: the connection's input rate, and authentication before a key. */
export interface InputGate {
  /** Milliseconds until a message could go out (0: now): the gateway's rate, paced by the client. */
  delay?(): number;
  /**
   * Null to send `action` now; else it waits (and everything after it) until the promise
   * resolves: true sends it, false drops it.
   */
  authorize?(action: InputAction): Promise<boolean> | null;
}

/**
 * Queues input and sends it merged, once per frame and at most every MIN_SEND_INTERVAL_MS, never
 * waiting for an answer. What cannot go out yet (the rate, or an authentication before a key)
 * stays queued, in order, and merges with what is typed meanwhile. `schedule` runs a callback
 * after the next frame (requestAnimationFrame) or after a delay.
 */
export class InputBatcher {
  private queue: InputAction[] = [];
  private scheduled = false;
  private authorizing = false;
  private lastSend = -Infinity;

  constructor(
    private readonly send: (action: InputAction) => void,
    private readonly schedule: (fn: () => void, delayMs: number) => void,
    private readonly now: () => number,
    private readonly gate: InputGate = {},
  ) {}

  push(...actions: InputAction[]): void {
    if (actions.length === 0) return;
    this.queue.push(...actions);
    this.arm(Math.max(0, this.lastSend + MIN_SEND_INTERVAL_MS - this.now()));
  }

  private arm(delayMs: number): void {
    if (this.scheduled || this.authorizing) return;
    this.scheduled = true;
    this.schedule(() => this.flush(), delayMs);
  }

  /**
   * Sends what is queued, as far as the gate lets it; `all` ignores the rate (the view closes:
   * the client paces what is left).
   */
  flush(all = false): void {
    this.scheduled = false;
    if (this.authorizing) return;
    this.queue = mergeActions(this.queue);
    while (this.queue.length > 0) {
      const wait = all ? 0 : (this.gate.delay?.() ?? 0);
      if (wait > 0) {
        this.arm(wait);
        return;
      }
      const a = this.queue.shift()!;
      this.lastSend = this.now();
      const auth = this.gate.authorize?.(a) ?? null;
      if (auth) {
        this.authorizing = true;
        void auth
          .catch(() => false)
          .then((ok) => {
            this.authorizing = false;
            if (ok) this.send(a);
            if (this.queue.length > 0) this.arm(0);
          });
        return;
      }
      this.send(a);
    }
  }
}

type SendResult = { ok: true } | { ok: false; code: string; message: string };

export interface TerminalInputDeps {
  sendKeys(keys: string[]): Promise<SendResult>;
  sendText(text: string): Promise<SendResult>;
  online(): boolean;
  /** Milliseconds until input could go out (the client's pacing; 0: now). */
  delay(): number;
  /** Null when `keys` may go out now; else resolves true once authenticated, false to drop them. */
  authorize(keys: string[]): Promise<boolean> | null;
  /** Each send's outcome as its answer comes: sent, `busy` (text at a prompt), or an error. */
  report(r: "ok" | "busy" | { error: string }): void;
  schedule(fn: () => void, delayMs: number): void;
  now(): number;
}

/**
 * The terminal view's input: batched (InputBatcher), sent in order without waiting for answers
 * (they only report errors and a prompt refusing text), held by the client's pacing and by
 * authentication before a submit key.
 */
export function terminalInput(deps: TerminalInputDeps): InputBatcher {
  return new InputBatcher(
    (a) => {
      if (!deps.online()) {
        deps.report({ error: "Not connected. Nothing was sent." });
        return;
      }
      const sent = "keys" in a ? deps.sendKeys(a.keys) : deps.sendText(a.text);
      void sent.then((r) =>
        deps.report(r.ok ? "ok" : "text" in a && r.code === "busy" ? "busy" : { error: r.message }),
      );
    },
    (fn, delayMs) => deps.schedule(fn, delayMs),
    () => deps.now(),
    { delay: () => deps.delay(), authorize: (a) => ("keys" in a ? deps.authorize(a.keys) : null) },
  );
}
