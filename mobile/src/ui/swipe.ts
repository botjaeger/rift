// Where a sideways drag leaves a swipe-to-act row (SwipeAction). Free of React Native imports
// (tested in node).

/** Width of the revealed action. */
export const ACTION_W = 96;
/** A drag this far to the left leaves the action open; a shorter one closes it again. */
export const OPEN_AT = 48;

/** Where a sideways drag leaves the row: `dx` pixels from where it started, open or closed. */
export function settle(startOpen: boolean, dx: number): "open" | "closed" {
  const x = Math.min(0, Math.max(-ACTION_W, (startOpen ? -ACTION_W : 0) + dx));
  return x <= -OPEN_AT ? "open" : "closed";
}
