import { describe, expect, it } from "vitest";
import type { ScreenRun } from "../src/protocol/types";
import { linkAt, markLinks, openableLink, rowPieces, rowSegments, screenLinks } from "../src/terminal";

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
