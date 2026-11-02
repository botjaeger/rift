import { describe, expect, it } from "vitest";
import { cursorOutOfToken, draftParts, fileLabel, imageLabel, insertText, pastesAsToken, placeToken, reconcile, spokenLabel, staleField, textLabel, type Token  } from "../src/draftTokens";

const T = (id: number, at: number, label: string): Token => ({ id, at, label });
const one = T(1, 4, "[Image 1]"); // "see [Image 1] now"
const DRAFT = "see [Image 1] now";

describe("labels", () => {
  it("names an image, a file and a text", () => {
    expect(imageLabel(3)).toBe("[Image 3]");
    expect(fileLabel("note.pdf")).toBe("[File note.pdf]");
    expect(fileLabel("a\nb.pdf")).toBe("[File a b.pdf]");
    expect(textLabel("a\nb\r\nc\rd")).toBe("[Text 4 lines]");
    expect(textLabel("x".repeat(5000))).toBe("[Text 1 line]");
    expect(spokenLabel("[Text 12 lines]")).toBe("Text 12 lines");
  });
  it("a text is a token past three lines or past the field's bytes, not at them", () => {
    expect(pastesAsToken("a\nb\nc", 5, 4096)).toBe(false);
    expect(pastesAsToken("a\nb\nc\nd", 7, 4096)).toBe(true);
    expect(pastesAsToken("x", 4096, 4096)).toBe(false);
    expect(pastesAsToken("x", 4097, 4096)).toBe(true);
  });
});

describe("reconcile", () => {
  it("leaves a token alone while the text around it changes, and moves it", () => {
    expect(reconcile(DRAFT, "look: " + DRAFT, [one])).toEqual({ text: "look: " + DRAFT, tokens: [T(1, 10, "[Image 1]")], gone: [] });
    expect(reconcile(DRAFT, DRAFT + "!", [one])).toEqual({ text: DRAFT + "!", tokens: [one], gone: [] });
    expect(reconcile(DRAFT, "see [Image 1]", [one]).tokens).toEqual([one]);
    expect(reconcile(DRAFT, "[Image 1] now", [one]).tokens).toEqual([T(1, 0, "[Image 1]")]);
  });
  it("backspace at a token's end takes all of it, the cursor where it began", () => {
    expect(reconcile(DRAFT, "see [Image 1 now", [one])).toEqual({ text: "see  now", tokens: [], gone: [1], cursor: 4 });
  });
  it("deleting any part of a token takes all of it", () => {
    for (let i = 4; i < 13; i++) {
      const r = reconcile(DRAFT, DRAFT.slice(0, i) + DRAFT.slice(i + 1), [one]);
      expect(r.text, `char ${i}`).toBe("see  now");
      expect(r.gone, `char ${i}`).toEqual([1]);
    }
  });
  it("a selection across a token and words takes the token, and keeps what replaced it", () => {
    expect(reconcile(DRAFT, "seX now", [one])).toEqual({ text: "seX now", tokens: [], gone: [1], cursor: 3 });
    expect(reconcile(DRAFT, "see [ImageX now", [one])).toEqual({ text: "see X now", tokens: [], gone: [1], cursor: 5 });
  });
  it("only the token touched goes; the one after it moves", () => {
    const two = [T(1, 0, "[Image 1]"), T(2, 10, "[Image 2]")];
    const r = reconcile("[Image 1] [Image 2]", "[Image 1 [Image 2]", two);
    expect(r).toEqual({ text: " [Image 2]", tokens: [T(2, 1, "[Image 2]")], gone: [1], cursor: 0 });
  });
  it("two tokens of one text are told apart by place", () => {
    const two = [T(1, 0, "[T]"), T(2, 3, "[T]")];
    expect(reconcile("[T][T]", "[T[T]", two)).toMatchObject({ text: "[T]", tokens: [T(2, 0, "[T]")], gone: [1] });
    expect(reconcile("[T][T]", "[T][T", two)).toMatchObject({ text: "[T]", tokens: [T(1, 0, "[T]")], gone: [2] });
  });
  it("text typed inside a token goes after it, the token whole", () => {
    expect(reconcile(DRAFT, "see [ImXage 1] now", [one])).toEqual({ text: "see [Image 1]X now", tokens: [one], gone: [], cursor: 14 });
  });
  it("a repeated character reads the way that spares the token", () => {
    const t = T(1, 0, "[T]");
    // a "[" typed just before the token
    expect(reconcile("[T]", "[[T]", [t])).toEqual({ text: "[[T]", tokens: [T(1, 1, "[T]")], gone: [] });
    // a "]" deleted just after it
    expect(reconcile("[T]]", "[T]", [t])).toEqual({ text: "[T]", tokens: [t], gone: [] });
  });
  it("a typed look-alike is plain text: no token, and it deletes a character at a time", () => {
    expect(reconcile("[Image 1]", "[Image 1", [])).toEqual({ text: "[Image 1", tokens: [], gone: [] });
    // beside a real token of the same text, only the real one is a token
    const real = T(1, 10, "[Image 1]");
    expect(reconcile("[Image 1] [Image 1]", "[Image 1 [Image 1]", [real])).toEqual({ text: "[Image 1 [Image 1]", tokens: [T(1, 9, "[Image 1]")], gone: [] });
  });
  it("a token whose item is gone stays as plain text", () => {
    expect(reconcile("[Image 1] x", "[Image 1] xy", [])).toEqual({ text: "[Image 1] xy", tokens: [], gone: [] });
    expect(draftParts("[Image 1] x", [])).toEqual([{ text: "[Image 1] x" }]);
  });
  it("a whole new draft takes every token", () => {
    expect(reconcile(DRAFT, "ls -l", [one])).toMatchObject({ text: "ls -l", tokens: [], gone: [1] });
    expect(reconcile(DRAFT, "", [one])).toMatchObject({ text: "", tokens: [], gone: [1] });
  });
});

describe("a key that came before the field took a correction", () => {
  // [Text 5 lines] backspaced to "x[Text 5 line", corrected to "x"
  const typed = "x[Text 5 line";
  it("is told from an edit of the corrected draft", () => {
    expect(staleField(typed, "x", "x[Text 5 lin")).toBe(true);
    expect(staleField(typed, "x", "x[Text 5 linee")).toBe(true);
    expect(staleField(typed, "x", "xy")).toBe(false);
    expect(staleField(typed, "x", "")).toBe(false);
    expect(staleField(typed, "x", "x")).toBe(false);
  });
});

describe("placing", () => {
  it("puts a token at the cursor and moves the ones after it", () => {
    const r = placeToken("a [X]b", [T(1, 2, "[X]")], { start: 1, end: 1 }, 2, "[Y]");
    expect(r).toEqual({ text: "a[Y] [X]b", tokens: [T(2, 1, "[Y]"), T(1, 5, "[X]")], gone: [], cursor: 4 });
  });
  it("replaces a selection, token included", () => {
    expect(placeToken(DRAFT, [one], { start: 0, end: 13 }, 2, "[Y]")).toEqual({ text: "[Y] now", tokens: [T(2, 0, "[Y]")], gone: [1], cursor: 3 });
  });
  it("a cursor inside a token goes past it", () => {
    expect(placeToken(DRAFT, [one], { start: 6, end: 6 }, 2, "[Y]")).toEqual({ text: "see [Image 1][Y] now", tokens: [one, T(2, 13, "[Y]")], gone: [], cursor: 16 });
  });
  it("clamps a cursor past the end", () => {
    expect(placeToken("ab", [], { start: 9, end: 9 }, 1, "[Y]").text).toBe("ab[Y]");
  });
  it("text inserted at the cursor keeps the tokens and says where the cursor is", () => {
    expect(insertText(DRAFT, [one], { start: 13, end: 13 }, "\n")).toEqual({ text: "see [Image 1]\n now", tokens: [one], gone: [], cursor: 14 });
    expect(insertText(DRAFT, [one], { start: 0, end: 0 }, "x")).toEqual({ text: "x" + DRAFT, tokens: [T(1, 5, "[Image 1]")], gone: [], cursor: 1 });
  });
});

describe("the draft in order", () => {
  it("cuts at the tokens and holds nothing empty", () => {
    expect(draftParts(DRAFT, [one])).toEqual([{ text: "see " }, { token: one }, { text: " now" }]);
    expect(draftParts("[Image 1]", [T(1, 0, "[Image 1]")])).toEqual([{ token: T(1, 0, "[Image 1]") }]);
  });
});

describe("a cursor set down in a token", () => {
  const tokens = [{ at: 3, label: "[Image 1]" }, { at: -1, label: "[Text 4 lines]" }];
  it("rests just after it", () => {
    expect(cursorOutOfToken(5, tokens)).toBe(12);
  });
  it("stays at either edge, and ignores a token with no place yet", () => {
    expect(cursorOutOfToken(3, tokens)).toBeNull();
    expect(cursorOutOfToken(12, tokens)).toBeNull();
    expect(cursorOutOfToken(0, tokens)).toBeNull();
  });
});
