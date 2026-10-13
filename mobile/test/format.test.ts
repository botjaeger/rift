import { describe, expect, it } from "vitest";
import { formatDuration, insertedLetter, osName, tailLines } from "../src/ui/format";

describe("format", () => {
  it("osName", () => {
    expect(osName("macos")).toBe("macOS");
    expect(osName("linux")).toBe("Linux");
    expect(osName("haiku")).toBe("haiku");
  });

  it("formatDuration", () => {
    expect(formatDuration(12)).toBe("12ms");
    expect(formatDuration(8123)).toBe("8.1s");
    expect(formatDuration(252000)).toBe("4m 12s");
    expect(formatDuration(3_723_000)).toBe("1h 2m");
  });
  it("tailLines keeps the last lines and counts the hidden ones", () => {
    const text = Array.from({ length: 100 }, (_, i) => `l${i + 1}`).join("\n") + "\n";
    const { shown, hidden } = tailLines(text, 40);
    expect(hidden).toBe(60);
    expect(shown.split("\n")).toHaveLength(40);
    expect(shown.startsWith("l61\n")).toBe(true);
    expect(tailLines("a\nb\n", 40)).toEqual({ shown: "a\nb", hidden: 0 });
    expect(tailLines("", 40)).toEqual({ shown: "", hidden: 0 });
    expect(tailLines(text, Infinity).hidden).toBe(0);
  });
});

describe("insertedLetter (sticky ctrl)", () => {
  it("returns the one letter typed, lowercased, wherever the caret was", () => {
    expect(insertedLetter("", "c")).toBe("c");
    expect(insertedLetter("ls", "lsC")).toBe("c");
    expect(insertedLetter("ls", "lxs")).toBe("x");
  });
  it("is null for anything else", () => {
    expect(insertedLetter("ls", "ls1")).toBeNull();
    expect(insertedLetter("ls", "ls ")).toBeNull();
    expect(insertedLetter("ls", "lsab")).toBeNull(); // paste
    expect(insertedLetter("ls", "l")).toBeNull(); // delete
    expect(insertedLetter("ls", "xyz")).toBeNull();
  });
});
