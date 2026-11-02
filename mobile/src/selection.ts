// Selecting copies: what a selection puts on the clipboard, and when. Pure; the terminal screen
// (ui/TerminalView.tsx) and block output (ui/SelectableText.tsx) share it.

/** Blocks copy this long (milliseconds) after the handles stop moving. */
export const COPY_SETTLE_MS = 250;

/**
 * The text a selection copies, or null when nothing should be written: a blank selection (a tap
 * copies nothing, so it never overwrites the clipboard) or text already copied (lifting a handle
 * where it started copies again nothing).
 */
export function textToCopy(selected: string, copied: string | null): string | null {
  return selected.trim() === "" || selected === copied ? null : selected;
}

/** The text of `value` between two offsets (a read-only field reports its selection as these), in either order. */
export function selectedText(value: string, sel: { start: number; end: number }): string {
  const from = Math.max(0, Math.min(sel.start, sel.end));
  const to = Math.min(value.length, Math.max(sel.start, sel.end));
  return value.slice(from, to);
}

/**
 * Whether the app shows its own "Copied": not on Android 13 and later, whose system draws one for
 * every clipboard write (two would stack).
 */
export const showsOwnCopied = (os: string, version: string | number): boolean => !(os === "android" && Number(version) >= 33);
