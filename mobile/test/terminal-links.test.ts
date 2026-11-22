import { describe, expect, it, vi } from "vitest";
import { findAll, flat, load, react, render, uiDeps } from "./helpers/render";
import type { ScreenRun } from "../src/protocol/types";
import { readFileSync } from "node:fs";
import { textParts } from "../src/paneView";
import { linkAt, linkAtOffset, markLinks, openableLink, rowPieces, rowSegments, screenLinks, textLinks } from "../src/terminal";

const row = (...texts: string[]): ScreenRun[] => texts.map((t) => [t, null, null, 0]);

describe("screenLinks", () => {
  it("finds an http(s) address on a row and the cells it covers", () => {
    const links = screenLinks([row("open https://claude.ai/artifact/Jqy now")], 80);
    expect(links).toEqual([{ url: "https://claude.ai/artifact/Jqy", spans: [{ row: 0, col: 5, cells: 30 }] }]);
  });

  it("stops at a quote or bracket and drops trailing punctuation, as the desktop does", () => {
    expect(screenLinks([row('see "http://a.example/x".')], 80).map((l) => l.url)).toEqual(["http://a.example/x"]);
    expect(screenLinks([row("(https://a.example/y), then")], 80).map((l) => l.url)).toEqual(["https://a.example/y"]);
    expect(screenLinks([row("done: https://a.example/z.")], 80).map((l) => l.url)).toEqual(["https://a.example/z"]);
  });

  it("finds two links on one row, across runs of different colours", () => {
    const links = screenLinks([[["a https://x.example/1 ", null, null, 0], ["https://y.", "ff0000", null, 0], ["example/2", null, null, 0]]], 80);
    expect(links.map((l) => l.url)).toEqual(["https://x.example/1", "https://y.example/2"]);
  });

  it("follows an address the program wrapped onto the next row", () => {
    // 20 columns: the address fills row 0 to its last column and goes on at the start of row 1
    const lines = [row("go: https://a.exampl"), row("e/path/to/page more")];
    const [link] = screenLinks(lines, 20);
    expect(link!.url).toBe("https://a.example/path/to/page");
    expect(link!.spans).toEqual([
      { row: 0, col: 4, cells: 16 },
      { row: 1, col: 0, cells: 14 },
    ]);
  });

  it("does not join the next row when the address ends before the last column", () => {
    const [link] = screenLinks([row("go: https://a.example"), row("next line")], 40);
    expect(link!.url).toBe("https://a.example");
    expect(link!.spans).toHaveLength(1);
  });

  it("ignores a bare scheme and other schemes", () => {
    expect(screenLinks([row("https:// and ftp://x and javascript:alert(1)")], 80)).toEqual([]);
  });

  it("counts a wide character before the link as two cells", () => {
    const [link] = screenLinks([row("界 https://a.example")], 80);
    expect(link!.spans[0]!.col).toBe(3);
  });
});

describe("linkAt", () => {
  const links = screenLinks([row("x https://a.example/p y")], 80);
  it("answers for a cell inside the link only", () => {
    expect(linkAt(links, { row: 0, col: 2 })).toBe("https://a.example/p");
    expect(linkAt(links, { row: 0, col: 20 })).toBe("https://a.example/p");
    expect(linkAt(links, { row: 0, col: 0 })).toBeNull();
    expect(linkAt(links, { row: 0, col: 22 })).toBeNull();
    expect(linkAt(links, { row: 1, col: 2 })).toBeNull();
  });
});

describe("markLinks", () => {
  it("splits a piece at the link's edges so exactly its cells are marked", () => {
    const pieces = rowPieces(rowSegments(row("ab https://c.d ef"), null));
    const marked = markLinks(pieces, [{ col: 3, cells: 11 }]);
    expect(marked.map((p) => [p.text, p.col, p.cells, p.link])).toEqual([
      ["ab ", 0, 3, false],
      ["https://c.d", 3, 11, true],
      [" ef", 14, 3, false],
    ]);
  });

  it("leaves a row without links as it was", () => {
    const pieces = rowPieces(rowSegments(row("plain"), null));
    expect(markLinks(pieces, []).map((p) => p.link)).toEqual([false]);
  });
});

describe("openableLink", () => {
  it("opens web addresses only", () => {
    expect(openableLink("https://claude.ai/x")).toBe(true);
    expect(openableLink("http://a.example")).toBe(true);
    expect(openableLink("javascript:alert(1)")).toBe(false);
    expect(openableLink("file:///etc/passwd")).toBe(false);
    expect(openableLink("https://a b")).toBe(false);
  });
});

describe("links in block output (textLinks)", () => {
  it("finds each address as the screen does: stops at a quote or bracket, drops trailing punctuation", () => {
    const text = 'see https://example.com/docs, then (http://a.example/y).\nnext "https://b.example/z"';
    expect(textLinks(text).map((l) => [l.url, text.slice(l.start, l.end)])).toEqual([
      ["https://example.com/docs", "https://example.com/docs"],
      ["http://a.example/y", "http://a.example/y"],
      ["https://b.example/z", "https://b.example/z"],
    ]);
  });
  it("never runs over a line end, and a bare scheme is no link", () => {
    expect(textLinks("https://a.example\nrest").map((l) => l.url)).toEqual(["https://a.example"]);
    expect(textLinks("just https:// here")).toEqual([]);
  });
  it("a caret on a link's first or last character is in it; one beside it is not", () => {
    const text = "go https://a.example now";
    const links = textLinks(text);
    expect(linkAtOffset(links, 3)).toBe("https://a.example");
    expect(linkAtOffset(links, 3 + "https://a.example".length)).toBe("https://a.example");
    expect(linkAtOffset(links, 1)).toBeNull();
  });
  it("the drawn pieces keep every character, a match and a link each apart", () => {
    const text = "see https://main.example now main";
    const parts = textParts(text, "main", textLinks(text))!;
    expect(parts.map((p) => p.text).join("")).toBe(text);
    expect(parts.filter((p) => p.link).map((p) => p.text).join("")).toBe("https://main.example");
    expect(parts.filter((p) => p.mark).map((p) => p.text)).toEqual(["main", "main"]);
    expect(parts.find((p) => p.mark && p.link)?.text).toBe("main");
    expect(textParts("plain words", undefined, [])).toBeNull();
  });
});

describe("a program's screen keeps the computer's colours", () => {
  const view = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
  it("its wrappers paint no ground of their own over the screen's (white text vanished on a light phone theme)", () => {
    expect(view).toContain("  fill: { flex: 1 },");
    expect(view).toContain("backgroundColor: screenDefaults(screen, defaultColors(colors)).bg");
  });
});

// SelectableText run with its handlers: a tap on a link opens it, a hold selects (Android walk 2026-11-22)
describe("a link in block output, on Android", () => {
  const opened: string[] = [];
  const mount = (text: string, lines?: number) => {
    const paneView = { textParts, markParts: () => [] };
    const deps = uiDeps("android", {
      react: { ...react, useId: () => "field" },
      "react-native": {
        ...(uiDeps("android")["react-native"] as object),
        AccessibilityInfo: { announceForAccessibility: () => {} },
        Linking: { openURL: async (u: string) => void opened.push(u) },
      },
      "expo-clipboard": { setStringAsync: async () => {} },
      "@/haptics": { copied: () => {} },
      "@/paneView": paneView,
      "@/terminal": { linkAtOffset, openableLink, textLinks },
      "@/selection": { COPY_SETTLE_MS: 400, lineRange: () => ({}), selectedText: () => "", showsOwnCopied: () => false, textToCopy: () => null, wordRange: () => ({}) },
      "./Marked": { partChildren: () => null, useMarkStyle: () => ({ mark: {} }) },
      "./SelectionScope": { useSelectionOwner: () => undefined, useSelectionScope: () => null },
      "./useFlash": { COPIED_MS: 1600 },
    });
    const { SelectableText } = load("ui/SelectableText.tsx", deps) as { SelectableText: (p: object) => unknown };
    const tree = render({ type: SelectableText, props: { text, style: {}, lineHeight: 17, lines, children: [] } });
    const field = findAll(tree, (n) => n.type === "TextInput")[0]!;
    let focused = true;
    (field.props.ref as { current: unknown }).current = { isFocused: () => focused, blur: () => {} };
    const caret = (start: number, end = start) =>
      (field.props.onSelectionChange as (e: object) => void)({ nativeEvent: { selection: { start, end } } });
    return { tree, field, caret, unfocus: () => void (focused = false) };
  };
  const text = "docs: https://example.com/docs, then more";
  const inLink = text.indexOf("example");

  it("a tap's caret in a link opens it once no selection follows", () => {
    vi.useFakeTimers();
    opened.length = 0;
    const { caret } = mount(text);
    caret(inLink);
    expect(opened).toEqual([]);
    vi.advanceTimersByTime(200);
    expect(opened).toEqual(["https://example.com/docs"]);
    vi.useRealTimers();
  });
  it("a hold's caret, followed by its word selection, opens nothing", () => {
    vi.useFakeTimers();
    opened.length = 0;
    const { caret } = mount(text);
    caret(inLink);
    vi.advanceTimersByTime(13);
    caret(inLink - 0, inLink + 11);
    vi.advanceTimersByTime(1000);
    expect(opened).toEqual([]);
    vi.useRealTimers();
  });
  it("a caret the field gets unfocused (laid out, not touched) opens nothing", () => {
    vi.useFakeTimers();
    opened.length = 0;
    const { caret, unfocus } = mount("go https://example.com/end");
    unfocus();
    caret("go https://example.com/end".length);
    vi.advanceTimersByTime(1000);
    expect(opened).toEqual([]);
    vi.useRealTimers();
  });
  it("a cut field is clipped from outside and keeps its whole text's height (Android scrolled it to the caret)", () => {
    const { tree, field } = mount(text, 2);
    expect(flat(field.props.style).maxHeight).toBeUndefined();
    const clip = findAll(tree, (n) => n.type === "View" && flat(n.props.style).overflow === "hidden")[0]!;
    expect(flat(clip.props.style).maxHeight).toBe(34);
    expect(findAll(clip, (n) => n.type === "TextInput")).toHaveLength(1);
  });
});
