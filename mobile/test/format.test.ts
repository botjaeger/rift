import { describe, expect, it } from "vitest";
import { formatClock, formatDuration, insertedChar, plainPunctuation, osName, tailLines } from "../src/ui/format";

describe("plainPunctuation", () => {
  it("undoes the keyboard's smart dashes and quotes", () => {
    expect(plainPunctuation("cargo fmt \u2014all")).toBe("cargo fmt --all");
    expect(plainPunctuation("git commit -m \u201cfix it\u201d && echo \u2018ok\u2019")).toBe("git commit -m \"fix it\" && echo 'ok'");
    expect(plainPunctuation("ls -la")).toBe("ls -la");
  });
});

describe("formatClock", () => {
  it("is m:ss, h:mm:ss past an hour", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(252_000)).toBe("4:12");
    expect(formatClock(3_723_000)).toBe("1:02:03");
  });
});

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

describe("insertedChar (sticky ctrl)", () => {
  it("returns the one character typed, wherever the caret was", () => {
    expect(insertedChar("", "c")).toBe("c");
    expect(insertedChar("ls", "lsC")).toBe("C");
    expect(insertedChar("ls", "lxs")).toBe("x");
    expect(insertedChar("ls", "ls1")).toBe("1");
    expect(insertedChar("ls", "ls ")).toBe(" ");
    expect(insertedChar("ls", "ls[")).toBe("[");
  });
  it("is null for anything else", () => {
    expect(insertedChar("ls", "lsab")).toBeNull(); // paste
    expect(insertedChar("ls", "l")).toBeNull(); // delete
    expect(insertedChar("ls", "xyz")).toBeNull();
  });
});
