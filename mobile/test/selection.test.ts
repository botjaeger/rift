import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { COPY_SETTLE_MS, selectedText, showsOwnCopied, textToCopy } from "../src/selection";

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
