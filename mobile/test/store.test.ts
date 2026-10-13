import { describe, expect, it } from "vitest";
import {
  atCapacity,
  canLoadOlder,
  emptyPane,
  MAX_BLOCKS,
  MAX_OUTPUT_CHARS,
  reducePane,
  type PaneState,
} from "../src/protocol/store";
import type { Block } from "../src/protocol/types";

const block = (id: number, output = `out ${id}\n`, extra: Partial<Block> = {}): Block => ({
  id,
  command: `cmd ${id}`,
  state: "done",
  exit: 0,
  duration_ms: 1,
  cwd: null,
  branch: null,
  output,
  dropped: 0,
  restored: false,
  ...extra,
});

const snap = (blocks: Block[], seq = 1, has_older = false) =>
  reducePane(null, { t: "snapshot", pane: 1, seq, blocks, has_older })!;

describe("reducePane", () => {
  it("snapshot replaces the pane", () => {
    const a = snap([block(1), block(2)]);
    const b = reducePane(a, { t: "snapshot", pane: 1, seq: 9, blocks: [block(5)], has_older: true })!;
    expect(b.blocks.map((x) => x.id)).toEqual([5]);
    expect(b.seq).toBe(9);
    expect(b.hasOlder).toBe(true);
  });

  it("update upserts by id, appends new ids in order, removes `removed`", () => {
    const a = snap([block(1), block(2), block(3)]);
    const b = reducePane(a, {
      t: "update",
      pane: 1,
      seq: 2,
      blocks: [block(2, "changed\n"), block(4)],
      removed: [1],
    })!;
    expect(b.blocks.map((x) => x.id)).toEqual([2, 3, 4]);
    expect(b.blocks[0]!.output).toBe("changed\n");
    expect(b.blocks[1]).toBe(a.blocks[2]); // untouched blocks keep identity (memoized rows)
    expect(b.seq).toBe(2);
  });

  it("older prepends blocks it does not hold", () => {
    const a = snap([block(10), block(11)], 1, true);
    const b = reducePane(a, { t: "older", pane: 1, blocks: [block(8), block(9), block(10)], has_older: false })!;
    expect(b.blocks.map((x) => x.id)).toEqual([8, 9, 10, 11]);
    expect(b.hasOlder).toBe(false);
  });

  it("ignores messages for another pane", () => {
    const a = snap([block(1)]);
    expect(reducePane(a, { t: "update", pane: 2, seq: 2, blocks: [block(9)], removed: [] })).toBe(a);
    expect(reducePane(null, { t: "update", pane: 1, seq: 2, blocks: [block(9)], removed: [] })).toBeNull();
  });

  it("caps the block count, dropping the oldest and marking has_older", () => {
    const a = snap(Array.from({ length: MAX_BLOCKS + 50 }, (_, i) => block(i + 1)));
    expect(a.blocks.length).toBe(MAX_BLOCKS);
    expect(a.blocks[0]!.id).toBe(51);
    expect(a.hasOlder).toBe(true);
  });

  it("caps total output, dropping the oldest blocks first", () => {
    const big = "x".repeat(60 * 1024) + "\n";
    const a = snap(Array.from({ length: 10 }, (_, i) => block(i + 1, big)));
    expect(a.outputChars).toBeLessThanOrEqual(MAX_OUTPUT_CHARS);
    expect(a.blocks.map((b) => b.id)).toEqual([7, 8, 9, 10]);
    expect(a.hasOlder).toBe(true);
  });

  it("truncates the head of a single block larger than the cap and counts the lines", () => {
    const line = "y".repeat(1023) + "\n"; // 1 KiB per line
    const a = snap([block(1, line.repeat(300))]);
    expect(a.blocks).toHaveLength(1);
    expect(a.outputChars).toBeLessThanOrEqual(MAX_OUTPUT_CHARS);
    expect(a.blocks[0]!.dropped).toBe(300 - 256);
    expect(a.blocks[0]!.output.startsWith("y")).toBe(true);
  });

  it("stays bounded over 10 000 updates", () => {
    let s: PaneState | null = reducePane(null, { t: "snapshot", pane: 1, seq: 0, blocks: [], has_older: false });
    let maxBlocks = 0;
    let maxChars = 0;
    for (let seq = 1; seq <= 10_000; seq++) {
      // Each update finishes one block and starts the next, with ~2 KiB of output each.
      s = reducePane(s, {
        t: "update",
        pane: 1,
        seq,
        blocks: [block(seq, `${"z".repeat(2047)}\n`), block(seq + 1, "", { state: "running", exit: null })],
        removed: [],
      });
      maxBlocks = Math.max(maxBlocks, s!.blocks.length);
      maxChars = Math.max(maxChars, s!.outputChars);
    }
    expect(maxBlocks).toBeLessThanOrEqual(MAX_BLOCKS);
    expect(maxChars).toBeLessThanOrEqual(MAX_OUTPUT_CHARS);
    expect(s!.seq).toBe(10_000);
    expect(s!.blocks.at(-1)!.id).toBe(10_001);
    expect(s!.outputChars).toBe(s!.blocks.reduce((n, b) => n + b.output.length, 0));
  });

  it("emptyPane starts empty", () => {
    expect(emptyPane(3)).toEqual({ pane: 3, seq: 0, blocks: [], hasOlder: false, outputChars: 0, olderBlocked: false });
  });
});

describe("canLoadOlder", () => {
  const big = "x".repeat(64 * 1024 - 1) + "\n"; // 64 Ki chars: four blocks fill the output cap

  it("is true below both caps when the gateway has older blocks", () => {
    expect(canLoadOlder(snap([block(1), block(2)], 1, true))).toBe(true);
  });

  it("is false without has_older, and for no pane", () => {
    expect(canLoadOlder(snap([block(1)], 1, false))).toBe(false);
    expect(canLoadOlder(null)).toBe(false);
  });

  it("is false at the block cap", () => {
    const s = snap(Array.from({ length: MAX_BLOCKS }, (_, i) => block(i + 1)), 1, true);
    expect(s.blocks).toHaveLength(MAX_BLOCKS);
    expect(atCapacity(s)).toBe(true);
    expect(canLoadOlder(s)).toBe(false);
  });

  it("is false at the output cap", () => {
    const s = snap([block(1, big), block(2, big), block(3, big), block(4, big)], 1, true);
    expect(s.outputChars).toBe(MAX_OUTPUT_CHARS);
    expect(s.blocks.length).toBeLessThan(MAX_BLOCKS);
    expect(canLoadOlder(s)).toBe(false);
  });

  it("is false once an older reply kept nothing, until room frees", () => {
    // 3 big blocks + a small one: room for less than one more big block.
    const s = snap([block(10, big), block(11, big), block(12, big), block(13)], 1, true);
    expect(canLoadOlder(s)).toBe(true);
    const after = reducePane(s, { t: "older", pane: 1, blocks: [block(9, big)], has_older: true })!;
    expect(after.blocks.map((b) => b.id)).toEqual([10, 11, 12, 13]); // trimmed straight back off
    expect(canLoadOlder(after)).toBe(false);
    const freed = reducePane(after, { t: "update", pane: 1, seq: 2, blocks: [], removed: [10] })!;
    expect(canLoadOlder(freed)).toBe(true);
  });

  it("stays true after an older reply that kept blocks", () => {
    const s = snap([block(10), block(11)], 1, true);
    const after = reducePane(s, { t: "older", pane: 1, blocks: [block(8), block(9)], has_older: true })!;
    expect(after.blocks[0]!.id).toBe(8);
    expect(canLoadOlder(after)).toBe(true);
  });
});
