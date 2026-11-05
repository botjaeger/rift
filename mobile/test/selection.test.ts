import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { barStaysLow, barTop, tapEndsSelection, COPY_SETTLE_MS, countLabel, lineRange, promptText, selectedText, selectionCount, showsOwnCopied, textToCopy } from "../src/selection";

describe("the bar's Line and To prompt", () => {
  const text = "first line\nsecond line\nthird";
  it("Line takes the whole lines a selection touches", () => {
    expect(lineRange(text, { start: 13, end: 15 })).toEqual({ start: 11, end: 22 });
    expect(selectedText(text, lineRange(text, { start: 3, end: 15 }))).toBe("first line\nsecond line");
    expect(selectedText(text, lineRange(text, { start: 25, end: 24 }))).toBe("third");
    // a selection ending right after a line break does not take the next line
    expect(selectedText(text, lineRange(text, { start: 0, end: 11 }))).toBe("first line");
    expect(lineRange("", { start: 0, end: 0 })).toEqual({ start: 0, end: 0 });
    expect(lineRange("ab", { start: -4, end: 99 })).toEqual({ start: 0, end: 2 });
    // output that starts with a blank line: the first line is that empty line, not the next
    expect(lineRange("\nsecond", { start: 0, end: 0 })).toEqual({ start: 0, end: 0 });
    expect(selectedText("\nsecond", lineRange("\nsecond", { start: 1, end: 3 }))).toBe("second");
  });
  it("To prompt joins lines with spaces: nothing it puts in the command line runs by itself", () => {
    expect(promptText("  cargo test\n\n  --workspace \n")).toBe("cargo test --workspace");
    expect(promptText("one")).toBe("one");
    expect(promptText(" \n ")).toBe("");
    expect(promptText("a\r\nb")).toBe("a b");
  });
});

describe("where the selection bar goes", () => {
  it("over block output: just above the field, else just under it, never off the output", () => {
    // room above: bar of 100 ends 8 above the field
    expect(barTop(500, 540, 700, 100)).toBe(392);
    // a field at the top: under it
    expect(barTop(40, 80, 700, 100)).toBe(88);
    // a field filling the output: the bottom
    expect(barTop(-300, 1200, 700, 100)).toBe(592);
    // an output smaller than the bar still keeps the margin
    expect(barTop(10, 20, 50, 100)).toBe(8);
    // the field scrolled out below the output: the bar stays on it, at its bottom
    expect(barTop(900, 940, 700, 100)).toBe(592);
    // scrolled out above: at its top
    expect(barTop(-200, -160, 700, 100)).toBe(8);
  });
  it("on Android, where the system's own menu sits just above the selection: under it, else the top", () => {
    // room under the field
    expect(barTop(200, 240, 700, 100, true)).toBe(248);
    // a field at the bottom of the output: the top, away from the system menu
    expect(barTop(560, 600, 700, 100, true)).toBe(8);
  });
  it("over a program's screen: at the bottom unless the selection reaches the bar's room", () => {
    // 40 rows in 800 pt: the bottom 120 pt are the last 6 rows
    expect(barStaysLow(10, 40, 800)).toBe(true);
    expect(barStaysLow(33, 40, 800)).toBe(true);
    expect(barStaysLow(34, 40, 800)).toBe(false);
    expect(barStaysLow(39, 40, 800)).toBe(false);
    // not laid out yet, or no rows
    expect(barStaysLow(5, 40, 0)).toBe(true);
    expect(barStaysLow(0, 0, 800)).toBe(true);
  });
});

describe("a tap that ends the selection", () => {
  // field 500..540, bar 392..492
  it("ends it on a tap away from the field and off the bar", () => {
    expect(tapEndsSelection(650, 500, 540, 392, 492)).toBe(true);
    expect(tapEndsSelection(200, 500, 540, 392, 492)).toBe(true);
  });
  it("keeps it on the field, its handles' slop, and the bar", () => {
    expect(tapEndsSelection(520, 500, 540, 392, 492)).toBe(false);
    expect(tapEndsSelection(565, 500, 540, 392, 492)).toBe(false);
    expect(tapEndsSelection(400, 500, 540, 392, 492)).toBe(false);
    expect(tapEndsSelection(569, 500, 540, 392, 492)).toBe(true);
  });
});

describe("a selection's size", () => {
  it("counts lines and characters as code points, line breaks left out", () => {
    expect(selectionCount("")).toEqual({ lines: 0, chars: 0 });
    expect(selectionCount("world")).toEqual({ lines: 1, chars: 5 });
    expect(selectionCount("ab\ncd\n")).toEqual({ lines: 3, chars: 4 });
    expect(selectionCount("日本 👍🏽")).toEqual({ lines: 1, chars: 5 });
  });
  it("says it in words, singular for one", () => {
    expect(countLabel({ lines: 1, chars: 1 })).toBe("1 line · 1 character");
    expect(countLabel({ lines: 4, chars: 137 })).toBe("4 lines · 137 characters");
  });
});

const source = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

describe("selecting copies", () => {
  it("copies a selection, never a blank one: a tap overwrites nothing", () => {
    expect(textToCopy("world", null)).toBe("world");
    expect(textToCopy("", "kept")).toBeNull();
    expect(textToCopy("  \n ", "kept")).toBeNull();
  });
  it("does not copy again what the last selection copied", () => {
    expect(textToCopy("world", "world")).toBeNull();
    expect(textToCopy("world!", "world")).toBe("world!");
  });
  it("reads a field's selection in either direction, clamped to its text", () => {
    expect(selectedText("hello world", { start: 6, end: 11 })).toBe("world");
    expect(selectedText("hello world", { start: 11, end: 6 })).toBe("world");
    expect(selectedText("hello", { start: -3, end: 99 })).toBe("hello");
    expect(selectedText("hello", { start: 2, end: 2 })).toBe("");
    // the text changed after the selection: what the field shows now is copied
    expect(selectedText("ab", { start: 0, end: 10 })).toBe("ab");
  });
  it("waits for the handles to stop before blocks copy", () => {
    expect(COPY_SETTLE_MS).toBe(250);
  });
  it("shows its own Copied except where Android 13 and later draws one", () => {
    expect(showsOwnCopied("ios", "18.4")).toBe(true);
    expect(showsOwnCopied("android", 32)).toBe(true);
    expect(showsOwnCopied("android", 33)).toBe(false);
    expect(showsOwnCopied("android", 35)).toBe(false);
  });
});

describe("where selecting copies", () => {
  it("the terminal screen has no Copy button; the hold, a lifted handle and All copy", () => {
    const t = source("src/ui/TerminalView.tsx");
    expect(t).not.toContain('accessibilityLabel="Copy"');
    expect(t).not.toContain("copySelection");
    expect(t).toContain("copyRange(s, a, b, null)");
    expect(t).toContain("copyRange(shown, selection.a, selection.b, copiedText)");
    expect(t).toContain("copyRange(shown, a, b, copiedText)");
  });
  it("block output, failed message, running lines and the sheet are selectable text; the output does not open the sheet", () => {
    const frame = source("src/ui/BlockCards.tsx");
    expect(frame).toContain("<SelectableText");
    // only the head is the button that opens the sheet
    expect(frame).toMatch(/<View style=\{styles\.head\}>\{head\}<\/View>\s*<\/Pressable>/);
    expect(source("src/ui/BlockView.tsx")).toContain("<SelectableText");
    expect(source("src/ui/BlockView.tsx")).not.toContain("selectable>");
    expect(source("src/app/block.tsx")).toContain("<SelectableText");
  });
});
