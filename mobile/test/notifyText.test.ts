import { describe, expect, it } from "vitest";
import { askText, doneText, durationText, exitMark, titleCommand, type DoneFacts } from "../src/notifyText";

const base: DoneFacts = {
  host: "studio-mac",
  tab: "rift",
  command: "cargo test",
  exit: 0,
  durationMs: 113,
  lastLine: "test result: ok",
  showCommand: true,
};

describe("durationText", () => {
  it("words ms, s, m s and h m", () => {
    const cases: [number, string][] = [
      [0, "0ms"],
      [113, "113ms"],
      [999, "999ms"],
      [1000, "1.0s"],
      [8400, "8.4s"],
      [9999, "9.9s"],
      [10_000, "10s"],
      [59_999, "59s"],
      [60_000, "1m 0s"],
      [125_000, "2m 5s"],
      [3_599_000, "59m 59s"],
      [3_600_000, "1h 0m"],
      [3_725_000, "1h 2m"],
      [90_000_000, "25h 0m"],
    ];
    for (const [ms, text] of cases) expect(durationText(ms), String(ms)).toBe(text);
  });

  it("survives a negative or non-finite duration", () => {
    expect(durationText(-5)).toBe("0ms");
    expect(durationText(Number.NaN)).toBe("0ms");
  });
});

describe("doneText", () => {
  it("success: tick and 0, finished in, place, last line", () => {
    expect(doneText(base)).toEqual({
      title: "✓ 0 · cargo test",
      subtitle: "Finished in 113ms · studio-mac · rift",
      body: "test result: ok",
    });
  });

  it("failure with an exit status", () => {
    expect(doneText({ ...base, exit: 101, durationMs: 8400 })).toEqual({
      title: "✕ 101 · cargo test",
      subtitle: "Failed after 8.4s · studio-mac · rift",
      body: "test result: ok",
    });
  });

  it("no exit status (a signal, or none reported): a tick without a number, not a failure", () => {
    const d = doneText({ ...base, exit: null, durationMs: 125_000 });
    expect(d.title).toBe("✓ · cargo test");
    expect(d.subtitle).toBe("Finished in 2m 5s · studio-mac · rift");
    expect(exitMark(null)).toBe("✓");
    expect(exitMark(130)).toBe("✕ 130");
  });

  it("cuts a long command to 40 characters and joins its lines", () => {
    const long = doneText({ ...base, command: "x".repeat(60) }).title;
    expect(long).toBe(`✓ 0 · ${"x".repeat(39)}…`);
    expect(Array.from(long.slice("✓ 0 · ".length))).toHaveLength(40);
    expect(titleCommand("x".repeat(40), true)).toBe("x".repeat(40));
    expect(titleCommand("git commit -m 'a\n  b'", true)).toBe("git commit -m 'a b'");
    // astral characters count once and are never split
    expect(Array.from(titleCommand("😀".repeat(50), true))).toHaveLength(40);
  });

  it("an empty command reads as A command", () => {
    expect(doneText({ ...base, command: "  \n " }).title).toBe("✓ 0 · A command");
  });

  it("Show the command off: no command and no output line", () => {
    expect(doneText({ ...base, exit: 2, showCommand: false })).toEqual({
      title: "✕ 2 · A command",
      subtitle: "Failed after 113ms · studio-mac · rift",
    });
  });

  it("no last line: no body", () => {
    for (const lastLine of [undefined, null, "", "   "]) expect(doneText({ ...base, lastLine })).not.toHaveProperty("body");
  });

  it("no tab: the place is the computer alone; a long line is cut", () => {
    const d = doneText({ ...base, tab: null, lastLine: "y".repeat(300) });
    expect(d.subtitle).toBe("Finished in 113ms · studio-mac");
    expect(Array.from(d.body ?? "")).toHaveLength(120);
    expect(d.body?.endsWith("…")).toBe(true);
  });
});

describe("askText", () => {
  const ask = { program: "claude", kind: "choice" as const, host: "studio-mac", tab: "rift", pane: 2, prompt: "Allow edit to src/a.ts?" };

  it("an agent's question", () => {
    expect(askText(ask)).toEqual({
      title: "claude is asking",
      subtitle: "studio-mac · rift · pane 2",
      body: "Allow edit to src/a.ts?",
    });
  });

  it("a password, other input, an unknown program, no tab, no prompt", () => {
    expect(askText({ ...ask, kind: "password", program: "sudo" }).title).toBe("sudo needs a password");
    expect(askText({ ...ask, kind: "input", program: "build" }).title).toBe("build needs your input");
    const bare = askText({ ...ask, program: null, tab: null, prompt: "" });
    expect(bare).toEqual({ title: "The command is asking", subtitle: "studio-mac · pane 2" });
  });
});
