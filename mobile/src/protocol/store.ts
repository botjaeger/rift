// Pane state as a pure reducer over gateway messages, bounded so a long-running session keeps
// a fixed memory ceiling: at most MAX_BLOCKS blocks and MAX_OUTPUT_CHARS characters of output
// per pane, oldest blocks dropped first.

import type { Block, OlderMsg, ScreenEndMsg, ScreenMsg, ScreenRun, SnapshotMsg, UpdateMsg } from "./types";

export const MAX_BLOCKS = 200;
/** 256 Ki characters (UTF-16 code units) of block output per pane. */
export const MAX_OUTPUT_CHARS = 256 * 1024;

export interface PaneState {
  pane: number;
  seq: number;
  /** Ascending by id: oldest first, newest last. */
  blocks: Block[];
  hasOlder: boolean;
  /** Sum of `output.length` over `blocks`. */
  outputChars: number;
  /** The last `older` reply kept none of its blocks (no room left); cleared once room frees. */
  olderBlocked: boolean;
}

export type PaneMsg = SnapshotMsg | UpdateMsg | OlderMsg;

export function emptyPane(pane: number): PaneState {
  return { pane, seq: 0, blocks: [], hasOlder: false, outputChars: 0, olderBlocked: false };
}

const byId = (a: Block, b: Block) => a.id - b.id;

/** Keeps the last `MAX_OUTPUT_CHARS` of `output`, counting the cut lines into `dropped`. */
function truncateHead(block: Block, max: number): Block {
  if (block.output.length <= max) return block;
  const cut = block.output.length - max;
  let start = block.output.indexOf("\n", cut - 1);
  start = start === -1 ? cut : start + 1;
  const head = block.output.slice(0, start);
  let lines = 0;
  for (let i = 0; i < head.length; i++) if (head.charCodeAt(i) === 10) lines++;
  if (!head.endsWith("\n")) lines++;
  return { ...block, output: block.output.slice(start), dropped: block.dropped + lines };
}

function bound(blocks: Block[], hasOlder: boolean): Pick<PaneState, "blocks" | "hasOlder" | "outputChars"> {
  let total = 0;
  for (const b of blocks) total += b.output.length;
  let first = 0;
  while (
    blocks.length - first > 1 &&
    (blocks.length - first > MAX_BLOCKS || total > MAX_OUTPUT_CHARS)
  ) {
    total -= blocks[first]!.output.length;
    first++;
  }
  let kept = first > 0 ? blocks.slice(first) : blocks;
  if (total > MAX_OUTPUT_CHARS && kept.length === 1) {
    kept = [truncateHead(kept[0]!, MAX_OUTPUT_CHARS)];
    total = kept[0]!.output.length;
  }
  return { blocks: kept, hasOlder: hasOlder || first > 0, outputChars: total };
}

/** Applies one gateway message for `msg.pane`. A message for another pane is ignored. */
export function reducePane(state: PaneState | null, msg: PaneMsg): PaneState | null {
  if (msg.t === "snapshot") {
    return {
      pane: msg.pane,
      seq: msg.seq,
      olderBlocked: false,
      ...bound([...msg.blocks].sort(byId), msg.has_older),
    };
  }
  if (!state || state.pane !== msg.pane) return state;
  if (msg.t === "update") {
    const removed = new Set(msg.removed);
    const incoming = new Map(msg.blocks.map((b) => [b.id, b]));
    const next: Block[] = [];
    for (const b of state.blocks) {
      if (removed.has(b.id)) continue;
      const fresh = incoming.get(b.id);
      if (fresh) incoming.delete(b.id);
      next.push(fresh ?? b);
    }
    let appended = false;
    for (const b of incoming.values()) {
      if (removed.has(b.id)) continue;
      next.push(b);
      appended = true;
    }
    if (appended) next.sort(byId);
    return {
      ...state,
      seq: msg.seq,
      olderBlocked: state.olderBlocked && msg.removed.length === 0,
      ...bound(next, state.hasOlder),
    };
  }
  // older: prepend the blocks this pane does not hold yet.
  const have = new Set(state.blocks.map((b) => b.id));
  const older = msg.blocks.filter((b) => !have.has(b.id));
  const merged = [...older, ...state.blocks].sort(byId);
  const result = bound(merged, msg.has_older);
  const keptAny = older.length > 0 && result.blocks[0]!.id < (state.blocks[0]?.id ?? Infinity);
  return { ...state, ...result, olderBlocked: older.length > 0 && !keptAny };
}

/** True when the pane is at either bound: older blocks would be trimmed straight back off. */
export function atCapacity(state: PaneState): boolean {
  return state.blocks.length >= MAX_BLOCKS || state.outputChars >= MAX_OUTPUT_CHARS;
}

/** Whether "Load earlier blocks" may fetch: the gateway has older blocks and they would be kept. */
export function canLoadOlder(state: PaneState | null): boolean {
  return state !== null && state.hasOlder && !state.olderBlocked && !atCapacity(state);
}

// --- terminal screen ---------------------------------------------------------------------------

/** The subscribed pane's screen as last sent: every row present (empty rows are []). */
export interface ScreenState {
  cols: number;
  rows: number;
  alt: boolean;
  cursor: [number, number] | null;
  /** `rows` entries; a row the last message did not change keeps its array (memoized rows). */
  lines: ScreenRun[][];
  /** The desktop theme's default text and ground colours (`rrggbb`); null from an older desktop. */
  fg: string | null;
  bg: string | null;
  /** Rows beside which a block's 2 px left rule is drawn, by row; empty from an older desktop. */
  rules: ReadonlyMap<number, string>;
  /** Shown from before a re-subscribe or reconnect: out of date until the next full screen. */
  stale: boolean;
}

const EMPTY_ROW: ScreenRun[] = [];
const NO_RULES: ReadonlyMap<number, string> = new Map();

/**
 * Applies one `screen` or `screen_end`: a full screen replaces, a diff patches its rows (a size
 * change without `full`, which the gateway never sends, resizes and keeps what fits), and
 * `screen_end` clears.
 */
export function reduceScreen(prev: ScreenState | null, msg: ScreenMsg | ScreenEndMsg): ScreenState | null {
  if (msg.t === "screen_end") return null;
  const base = msg.full || !prev ? [] : prev.lines;
  const lines: ScreenRun[][] = [];
  for (let r = 0; r < msg.rows; r++) lines.push(base[r] ?? EMPTY_ROW);
  for (const [row, runs] of msg.lines) if (row < msg.rows) lines[row] = runs;
  const stale = !msg.full && (prev?.stale ?? false);
  // colours and rules ride on every frame (diffs too): `rules` is the whole list, omitted when
  // there are none; absent colours (an older desktop) are the phone's own
  const rules = msg.rules && msg.rules.length > 0 ? new Map(msg.rules.filter(([row]) => row < msg.rows)) : NO_RULES;
  return {
    cols: msg.cols,
    rows: msg.rows,
    alt: msg.alt,
    cursor: msg.cursor,
    lines,
    fg: msg.fg ?? null,
    bg: msg.bg ?? null,
    rules,
    stale,
  };
}

/** The screen kept, out of date, across a re-subscribe or reconnect (the client's `onScreenStale`). */
export function staleScreen(prev: ScreenState | null): ScreenState | null {
  return prev && !prev.stale ? { ...prev, stale: true } : prev;
}
