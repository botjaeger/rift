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

/** A selection as it goes into a command line: its lines trimmed and joined by spaces, so nothing in it runs by itself. */
export const promptText = (selected: string): string =>
  selected
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .join(" ");

/** The whole lines a field's selection touches, as offsets into `value`. */
export function lineRange(value: string, sel: { start: number; end: number }): { start: number; end: number } {
  const from = Math.max(0, Math.min(sel.start, sel.end, value.length));
  const to = Math.min(value.length, Math.max(sel.start, sel.end, 0));
  // from 0 no search: lastIndexOf clamps -1 to 0 and would find a leading line break
  const start = from === 0 ? 0 : value.lastIndexOf("\n", from - 1) + 1;
  // a selection ending right after a line break ends on the line before it
  const lastChar = to > from && value[to - 1] === "\n" ? to - 1 : to;
  const nl = value.indexOf("\n", lastChar);
  return { start, end: nl < 0 ? value.length : nl };
}

/** How much a selection holds, as its bar says it: lines, and characters (code points, line breaks left out). */
export function selectionCount(text: string): { lines: number; chars: number } {
  if (text === "") return { lines: 0, chars: 0 };
  const lines = text.split("\n");
  return { lines: lines.length, chars: lines.reduce((n, l) => n + [...l].length, 0) };
}

/** The bar's words for a selection's size: "1 line · 5 characters". */
export function countLabel({ lines, chars }: { lines: number; chars: number }): string {
  return `${lines} ${lines === 1 ? "line" : "lines"} · ${chars} ${chars === 1 ? "character" : "characters"}`;
}

/**
 * Whether the app shows its own "Copied": not on Android 13 and later, whose system draws one for
 * every clipboard write (two would stack).
 */
export const showsOwnCopied = (os: string, version: string | number): boolean => !(os === "android" && Number(version) >= 33);

/** The height a selection bar takes over a program's screen, its 8 pt margin included. */
export const SELECTION_BAR_ROOM = 120;

/**
 * Whether the bar over a program's screen stays at the bottom (in thumb reach): yes unless the
 * selection's last row (0-based, of `rows`) falls in the bottom `SELECTION_BAR_ROOM` of a
 * `height`-point view, which the bar would cover. A view not laid out yet keeps it low.
 */
export function barStaysLow(lastRow: number, rows: number, height: number): boolean {
  if (rows <= 0 || height <= SELECTION_BAR_ROOM) return true;
  return (lastRow + 1) / rows <= 1 - SELECTION_BAR_ROOM / height;
}

/**
 * Where the bar over block output goes (points from the output's top): just above the selected
 * field (`top`..`bottom`, in the same coordinates), else just under it, kept on the output's
 * `height`; a field running past both edges puts it at the bottom. `under` (Android, whose own
 * text menu takes the place just above a selection): just under the field, else the output's top.
 */
export function barTop(top: number, bottom: number, height: number, barH: number, under = false, gap = 8): number {
  const most = Math.max(gap, height - barH - gap);
  if (under) return bottom + gap + barH + gap <= height ? Math.max(gap, bottom + gap) : gap;
  const above = top - barH - gap;
  // never off the output: a field scrolled out of sight keeps its bar at the nearer edge
  if (above >= gap) return Math.min(above, most);
  return Math.max(gap, Math.min(bottom + gap, most));
}

/** How far (points) outside the selected field a touch still counts as on it: its handles stand out of it. */
export const HANDLE_SLOP = 28;

/**
 * Whether a tap (window y) ends the selection the bar shows: one away from the selected field
 * (`fieldTop`..`fieldBottom`, its handles' slop included) and off the bar (`barTop`..`barBottom`).
 * iOS hides the field's highlight on such a tap without telling the field, so the bar must go too.
 */
export function tapEndsSelection(y: number, fieldTop: number, fieldBottom: number, barTop: number, barBottom: number): boolean {
  const onField = y >= fieldTop - HANDLE_SLOP && y <= fieldBottom + HANDLE_SLOP;
  const onBar = y >= barTop && y <= barBottom;
  return !onField && !onBar;
}
