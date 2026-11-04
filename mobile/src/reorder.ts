/** Drag-to-reorder maths for the Tabs and Panes sheets (rows of equal height, one dragged at a time). */

/** The place row `from` lands in when dragged `dy` points, rows `rowH` apart, `count` rows. */
export function slotAt(from: number, dy: number, rowH: number, count: number): number {
  if (count <= 0 || rowH <= 0) return from;
  return Math.min(count - 1, Math.max(0, from + Math.round(dy / rowH)));
}

/** How far row `i` slides to make room while `from` is dragged over `slot`. */
export function shiftOf(i: number, from: number, slot: number, rowH: number): number {
  if (i === from) return 0;
  if (from < slot && i > from && i <= slot) return -rowH;
  if (from > slot && i < from && i >= slot) return rowH;
  return 0;
}

/** Row `from` one place up (-1) or down (1), or null at the end of the list: a screen reader's Move up / Move down. */
export function stepTo(from: number, dir: -1 | 1, count: number): number | null {
  const to = from + dir;
  return to < 0 || to >= count ? null : to;
}

/** `list` with the item at `from` taken out and put at `to`. */
export function moved<T>(list: readonly T[], from: number, to: number): T[] {
  const out = list.slice();
  const [item] = out.splice(from, 1);
  out.splice(to, 0, item as T);
  return out;
}

/** Spoken after a move: "Moved build to position 2 of 5". */
export const movedText = (name: string, to: number, count: number) => `Moved ${name} to position ${to + 1} of ${count}`;
