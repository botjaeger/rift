import { describe, expect, it } from "vitest";
import {
  MAX_EARLIER_LINES,
  countLines,
  emptyEarlier,
  fetchRange,
  gapLines,
  heldLines,
  mergeRange,
  nextRequest,
  unloadedLines,
  type EarlierOutput,
  type PageResult,
} from "../src/earlier";
import type { Block } from "../src/protocol/types";

const block = (dropped: number, extra: Partial<Block> = {}): Block => ({
  id: 86,
  command: "cargo build",
  state: "done",
  exit: 0,
  duration_ms: 1,
  cwd: null,
  branch: null,
  output: "tail\n",
  dropped,
  restored: false,
  ...extra,
});

/** Lines `from` … `end - 1` as the computer would send them: "L<n>\n". */
const linesText = (from: number, end: number) => Array.from({ length: end - from }, (_, i) => `L${from + i}\n`).join("");

/**
 * A computer holding lines `held` … `total - 1` of the block, answering `limit` lines but at
 * most `maxPerAnswer` (its 64 KiB cut).
 */
function computer(total: number, opts: { held?: number; maxPerAnswer?: number } = {}) {
  const asked: [number, number][] = [];
  const page = async (from: number, limit: number): Promise<PageResult> => {
    asked.push([from, limit]);
    const start = Math.max(from, opts.held ?? 0);
    const end = Math.min(total, from + limit, start + (opts.maxPerAnswer ?? Infinity));
    return { ok: true, from: start, text: start >= end ? "" : linesText(start, end) };
  };
  return { page, asked };
}

describe("counting lines", () => {
  it("a trailing line break ends the last line; an unfinished one counts", () => {
    expect(countLines("")).toBe(0);
    expect(countLines("a\n")).toBe(1);
    expect(countLines("a\nb")).toBe(2);
    expect(countLines("\n\n")).toBe(2);
  });
});

describe("nextRequest", () => {
  it("the first press asks for the 500 lines before the tail", () => {
    expect(nextRequest(block(1234), undefined)).toEqual({ from: 734, limit: 500, fill: false });
    expect(nextRequest(block(38), undefined)).toEqual({ from: 0, limit: 38, fill: false });
    expect(nextRequest(block(0), undefined)).toBeNull();
  });

  it("then the 500 before the first held line, down to line 0", () => {
    const e: EarlierOutput = { ...emptyEarlier(block(1234)), from: 734, text: linesText(734, 1234) };
    expect(nextRequest(block(1234), e)).toEqual({ from: 234, limit: 500, fill: false });
    expect(nextRequest(block(1234), { ...e, from: 234 })).toEqual({ from: 0, limit: 234, fill: false });
    expect(nextRequest(block(1234), { ...e, from: 0 })).toBeNull();
  });

  it("first fills the lines between the held ones and a tail that moved on", () => {
    const e: EarlierOutput = { ...emptyEarlier(block(1000)), from: 500, end: 1000, text: linesText(500, 1000) };
    expect(gapLines(block(1100), e)).toBe(100);
    expect(nextRequest(block(1100), e)).toEqual({ from: 1000, limit: 100, fill: true });
  });

  it("stops at 5,000 held lines, and asks only for the room left", () => {
    const at = (from: number) => ({ ...emptyEarlier(block(9000)), from, end: 9000 });
    expect(nextRequest(block(9000), at(9000 - MAX_EARLIER_LINES))).toBeNull();
    expect(nextRequest(block(9000), at(9000 - MAX_EARLIER_LINES + 100))).toEqual({ from: 4000, limit: 100, fill: false });
  });

  it("nothing more before lines that are gone", () => {
    const e = { ...emptyEarlier(block(1000)), from: 700, gone: true };
    expect(nextRequest(block(1000), e)).toBeNull();
    expect(unloadedLines(block(1000), e)).toBe(0);
  });

  it("counts what is not held for the button", () => {
    expect(unloadedLines(block(1234), undefined)).toBe(1234);
    const e = { ...emptyEarlier(block(1000)), from: 500, end: 1000 };
    expect(unloadedLines(block(1100), e)).toBe(600);
    expect(heldLines(e)).toBe(500);
  });
});

describe("fetchRange", () => {
  it("returns the asked lines", async () => {
    const c = computer(2000);
    expect(await fetchRange(c.page, 734, 1234)).toEqual({ ok: true, from: 734, text: linesText(734, 1234), gone: false });
    expect(c.asked).toEqual([[734, 500]]);
  });

  it("follows an answer cut at 64 KiB with further requests until the range is complete", async () => {
    const c = computer(2000, { maxPerAnswer: 180 });
    const r = await fetchRange(c.page, 734, 1234);
    expect(r).toEqual({ ok: true, from: 734, text: linesText(734, 1234), gone: false });
    expect(c.asked).toEqual([
      [734, 500],
      [914, 320],
      [1094, 140],
    ]);
  });

  it("starts later, marked gone, when the computer no longer holds the first lines", async () => {
    const c = computer(2000, { held: 900 });
    expect(await fetchRange(c.page, 734, 1234)).toEqual({ ok: true, from: 900, text: linesText(900, 1234), gone: true });
  });

  it("all gone: nothing, starting at the end", async () => {
    const c = computer(2000, { held: 1500 });
    expect(await fetchRange(c.page, 734, 1234)).toEqual({ ok: true, from: 1234, text: "", gone: true });
  });

  it("ends a line cut at 64 KiB, so it does not glue to the next one", async () => {
    // line 0 is longer than one answer: the first page holds part of it, without its line break
    const pages: Record<number, PageResult> = {
      0: { ok: true, from: 0, text: "a".repeat(10) },
      1: { ok: true, from: 1, text: "b\n" },
    };
    const r = await fetchRange(async (from) => pages[from]!, 0, 2);
    expect(r).toEqual({ ok: true, from: 0, text: "aaaaaaaaaa\nb\n", gone: false });
    if (r.ok) expect(countLines(r.text)).toBe(2);
    // the last line of the range cut too: still ended (the held lines or the tail follow)
    const cut = await fetchRange(async (from) => ({ ok: true, from, text: "c".repeat(5) }), 7, 8);
    expect(cut).toEqual({ ok: true, from: 7, text: "ccccc\n", gone: false });
  });

  it("never keeps more lines than asked", async () => {
    const page = async (from: number): Promise<PageResult> => ({ ok: true, from, text: linesText(from, from + 600) });
    expect(await fetchRange(page, 0, 10)).toEqual({ ok: true, from: 0, text: linesText(0, 10), gone: false });
  });

  it("an error, or a computer that sends nothing, fails the load", async () => {
    const refuse = async (): Promise<PageResult> => ({ ok: false, code: "no_block", message: "gone" });
    expect(await fetchRange(refuse, 0, 10)).toMatchObject({ ok: false, code: "no_block" });
    const empty = async (from: number): Promise<PageResult> => ({ ok: true, from, text: "" });
    expect(await fetchRange(empty, 0, 10)).toMatchObject({ ok: false, code: "incomplete" });
  });

  it("lines that vanish between two pages drop what came before them", async () => {
    let n = 0;
    const page = async (from: number, limit: number): Promise<PageResult> => {
      n++;
      // first page cut at 100 lines; by the second the scrollback lost up to line 300
      if (n === 1) return { ok: true, from, text: linesText(from, from + 100) };
      const start = Math.max(from, 300);
      return { ok: true, from: start, text: linesText(start, from + limit) };
    };
    expect(await fetchRange(page, 0, 500)).toEqual({ ok: true, from: 300, text: linesText(300, 500), gone: true });
  });
});

describe("mergeRange", () => {
  it("puts earlier pages before the held lines", () => {
    const b = block(1234);
    const first = mergeRange(b, undefined, { from: 734, limit: 500, fill: false }, { from: 734, text: linesText(734, 1234), gone: false });
    expect(first).toMatchObject({ from: 734, end: 1234, gone: false, loading: false });
    const second = mergeRange(b, first, { from: 234, limit: 500, fill: false }, { from: 234, text: linesText(234, 734), gone: false });
    expect(second).toMatchObject({ from: 234, end: 1234, text: linesText(234, 1234) });
  });

  it("appends a fill, and a fill whose start is gone keeps only what reaches the tail", () => {
    const b = block(1100);
    const e: EarlierOutput = { ...emptyEarlier(block(1000)), from: 500, end: 1000, text: linesText(500, 1000) };
    expect(mergeRange(b, e, { from: 1000, limit: 100, fill: true }, { from: 1000, text: linesText(1000, 1100), gone: false })).toMatchObject({
      from: 500,
      end: 1100,
      text: linesText(500, 1100),
    });
    expect(mergeRange(b, e, { from: 1000, limit: 100, fill: true }, { from: 1050, text: linesText(1050, 1100), gone: true })).toMatchObject({
      from: 1050,
      end: 1100,
      gone: true,
      text: linesText(1050, 1100),
    });
  });
});
