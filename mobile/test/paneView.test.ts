import { describe, expect, it } from "vitest";
import {
  answerLabel,
  fitPlaceholder,
  messagePlaceholder,
  askContext,
  bareScreen,
  cardKind,
  clampZoom,
  failureMessage,
  firstLine,
  cardLine,
  moreLines,
  hasMoreRows,
  isDoubleTap,
  lastLines,
  otherChoices,
  outputLines,
  paneLine,
  paneRows,
  paneState,
  parseTable,
  cardMore,
  pinchZoom,
  sheetChip,
  liveBlock,
  visibleRows,
  waitingOthers,
  zoomStep,
  fitFontSize,
  fitSize,
  programSend,
  runsAgent,
  screenView,
  boxImages,
  partSend,
} from "../src/paneView";
import type { Block, Pane, Tab, Waiting } from "../src/protocol/types";

const pane = (id: number, extra: Partial<Pane> = {}): Pane => ({ id, active: false, cwd: null, job: null, last_command: null, last_exit: null, ...extra });
const tab = (panes: Pane[]): Tab => ({ index: 0, title: "flow", active: true, panes });
const block = (extra: Partial<Block> = {}): Block => ({
  id: 1,
  command: "ls",
  state: "done",
  exit: 0,
  duration_ms: 1200,
  cwd: null,
  branch: null,
  output: "",
  dropped: 0,
  restored: false,
  ...extra,
});
const waiting = (choices: string[], extra: Partial<Waiting> = {}): Waiting => ({
  kind: "choice",
  prompt: "Edit this file?",
  choices,
  selected: 0,
  numbered: true,
  context: [],
  ...extra,
});

describe("zoom", () => {
  it("clamps to 60 % .. 200 %, and a bad number is 100 %", () => {
    expect(clampZoom(0.1)).toBe(0.6);
    expect(clampZoom(5)).toBe(2);
    expect(clampZoom(NaN)).toBe(1);
    expect(clampZoom(1.3)).toBe(1.3);
  });
  it("a pinch scales the start zoom by the distance, within the limits", () => {
    expect(pinchZoom({ distance: 100, zoom: 1 }, 150)).toBeCloseTo(1.5);
    expect(pinchZoom({ distance: 100, zoom: 1.5 }, 400)).toBe(2);
    expect(pinchZoom({ distance: 100, zoom: 1 }, 10)).toBe(0.6);
    // a zero start distance leaves it
    expect(pinchZoom({ distance: 0, zoom: 1.2 }, 50)).toBe(1.2);
  });
  it("the buttons step by 15 % and stop at the limits", () => {
    expect(zoomStep(1, 1)).toBeCloseTo(1.15);
    expect(zoomStep(1, -1)).toBeCloseTo(1 / 1.15);
    expect(zoomStep(2, 1)).toBe(2);
    expect(zoomStep(0.6, -1)).toBe(0.6);
  });
  it("two taps within 300 ms are a double tap", () => {
    expect(isDoubleTap(1000, 1250)).toBe(true);
    expect(isDoubleTap(1000, 1400)).toBe(false);
    expect(isDoubleTap(null, 1000)).toBe(false);
  });
});

describe("the picker and the Panes sheet", () => {
  it("names a pane's state", () => {
    expect(paneState(pane(1, { waiting: waiting(["Yes"]) }))).toEqual({ text: "Needs you", tone: "needs" });
    expect(paneState(pane(1, { job: "vim" }))).toEqual({ text: "Running", tone: "running" });
    expect(paneState(pane(1, { last_exit: 1 }))).toEqual({ text: "✕ 1", tone: "failed" });
    expect(paneState(pane(1, { last_exit: 0 }))).toEqual({ text: "Idle", tone: "idle" });
  });
  it("shows the question, the latest line, the failed command, or the folder", () => {
    expect(paneLine(pane(1, { waiting: waiting(["Yes"], { prompt: "Edit x?" }) }))).toBe("Edit x?");
    expect(paneLine(pane(1, { job: "docker", running: { started: 1, line: "GET /health 200" } }))).toBe("GET /health 200");
    expect(paneLine(pane(1, { last_exit: 2, last_command: "phpunit" }))).toBe("phpunit");
    expect(paneLine(pane(1, { cwd: "/Users/me/flow", last_command: "ls" }))).toBe("~/flow");
  });
  it("lists the tab's panes with their marks", () => {
    const rows = paneRows(tab([pane(7, { job: "claude", waiting: waiting(["Yes"]) }), pane(9, { last_command: "psql" })]));
    expect(rows.map((r) => [r.id, r.number, r.name, r.mark, r.logo, r.claude, r.tone])).toEqual([
      [7, 1, "claude", null, "claude", true, "needs"],
      [9, 2, "psql", ">_", null, false, "idle"],
    ]);
  });
  it("counts the other panes that wait, not the open one", () => {
    const t = tab([pane(1, { waiting: waiting(["Yes"]) }), pane(2, { waiting: waiting(["Yes"]) }), pane(3)]);
    expect(waitingOthers(t, 1)).toBe(1);
    expect(waitingOthers(t, 3)).toBe(2);
  });
});

describe("block cards", () => {
  it("tells success, failure and running apart; an unknown exit is a success", () => {
    expect(cardKind(block())).toBe("ok");
    expect(cardKind(block({ exit: 127 }))).toBe("fail");
    expect(cardKind(block({ state: "running", exit: null }))).toBe("run");
    expect(cardKind(block({ exit: null }))).toBe("ok");
  });
  it("shows the line a search matched, and says how many more lines there are", () => {
    const out = "On branch main\n\nChanges not staged:\n\tmodified:   a.txt\n";
    expect(cardLine(out)).toBe("On branch main");
    expect(cardLine(out, "MODIFIED")).toBe("modified:   a.txt");
    // a match only in the command (not the output): the first line
    expect(cardLine(out, "git")).toBe("On branch main");
    expect(cardLine("", "x")).toBe("");
    // empty lines are not counted; lines the computer dropped are
    expect(moreLines(out, 0)).toBe("2 more lines");
    expect(moreLines("one\ntwo", 0)).toBe("1 more line");
    expect(moreLines("one", 0)).toBeNull();
    expect(moreLines("", 0)).toBeNull();
    expect(moreLines("one", 1500)).toBe("1,500 more lines");
  });
  it("takes the first and the last lines without the empty ones", () => {
    expect(firstLine("\n  Already up to date.  \nmore\n")).toBe("Already up to date.");
    expect(firstLine("")).toBe("");
    expect(lastLines("a\n\nb\nc\n", 2)).toEqual(["b", "c"]);
  });
  it("splits `prog: path: message` and falls back to the last line", () => {
    expect(failureMessage("bash: scripts/prisma-admin.sh: No such file or directory\n")).toEqual({
      text: "No such file or directory",
      path: "scripts/prisma-admin.sh",
    });
    // a message that holds colons stays whole
    expect(failureMessage("zsh: ./run.sh: permission denied: x")).toEqual({ text: "permission denied: x", path: "./run.sh" });
    // the second part is words, not a path
    expect(failureMessage("fatal: not a git repository: /tmp/x")).toEqual({ text: "fatal: not a git repository: /tmp/x", path: null });
    expect(failureMessage("error: something broke\n\n")).toEqual({ text: "error: something broke", path: null });
    expect(failureMessage("")).toBeNull();
  });
});

const DOCKER = [
  "CONTAINER ID   IMAGE         PORTS   STATUS",
  "f96b9b3f4fcd   api-php       9000    Up 3 hours",
  "8ed95103daa0   mysql:8       3306    Up 3 hours",
  "2548c6ae33e1   postgres:16   5432    Up 3 hours",
].join("\n");

describe("parseTable", () => {
  it("reads docker ps columns as list rows (image, id and port, status)", () => {
    expect(parseTable(DOCKER)).toEqual([
      { name: "api-php", meta: "f96b9b3f · 9000", state: "Up 3 hours" },
      { name: "mysql:8", meta: "8ed95103 · 3306", state: "Up 3 hours" },
      { name: "postgres:16", meta: "2548c6ae · 5432", state: "Up 3 hours" },
    ]);
  });
  it("reads kubectl, with NAME as the name and STATUS as the state", () => {
    const k = ["NAME        READY   STATUS    AGE", "api-0       1/1     Running   3d", "web-7d9f    2/2     Running   5h"].join("\n");
    expect(parseTable(k)).toEqual([
      { name: "api-0", meta: "1/1 · 3d", state: "Running" },
      { name: "web-7d9f", meta: "2/2 · 5h", state: "Running" },
    ]);
  });
  it("reads a header and one row (docker ps -a with one container)", () => {
    expect(parseTable(DOCKER.split("\n").slice(0, 2).join("\n"))).toEqual([{ name: "api-php", meta: "f96b9b3f · 9000", state: "Up 3 hours" }]);
    // the one row misaligned is plain output
    expect(parseTable("CONTAINER ID   IMAGE         PORTS   STATUS\nf96b9b3f4fcd   api-php      9000     Up")).toBeNull();
  });
  it("says a card line is cut, with or without more lines", () => {
    expect(cardMore("1 more line", true)).toBe("Line continues · 1 more line");
    expect(cardMore(null, true)).toBe("Line continues");
    expect(cardMore("1 more line", false)).toBe("1 more line");
    expect(cardMore(null, false)).toBeNull();
  });
  it("keeps an empty middle column out of the meta", () => {
    const t = ["ID      NAME    PORTS   STATUS", "ab12    one             Up", "cd34    two     80      Up"].join("\n");
    expect(parseTable(t)?.map((r) => r.meta)).toEqual(["ab12", "cd34 · 80"]);
  });
  it("is null for output that is not a table", () => {
    expect(parseTable("Already up to date.")).toBeNull();
    expect(parseTable("total 8\n-rw-r--r--  1 me  staff  12 Oct  1 a.txt\n-rw-r--r--  1 me  staff  34 Oct  1 b.txt")).toBeNull();
    // a header with two columns, or one lower-case
    expect(parseTable("NAME   AGE\na   1\nb   2")).toBeNull();
    expect(parseTable("Name   Ready   Status\na   1/1   Up\nb   1/1   Up")).toBeNull();
    // a header alone
    expect(parseTable(DOCKER.split("\n")[0]!)).toBeNull();
    expect(parseTable("")).toBeNull();
  });
  it("negative control: one misaligned row, or a stray line, makes it plain output", () => {
    expect(parseTable(DOCKER)).not.toBeNull();
    const shifted = DOCKER.replace("8ed95103daa0   mysql:8       3306", "8ed95103daa0   mysql:8      3306 ");
    expect(parseTable(shifted)).toBeNull();
    expect(parseTable(`${DOCKER}\nnote: nothing to see here at all`)).toBeNull();
    // the first column empty
    expect(parseTable(DOCKER.replace("f96b9b3f4fcd", "            "))).toBeNull();
  });
});

describe("the block sheet", () => {
  it("names the status in its pill", () => {
    expect(sheetChip(block({ state: "running", exit: null, duration_ms: 134000 }))).toEqual({ text: "● Running 2:14", tone: "running" });
    expect(sheetChip(block({ exit: 127 }))).toEqual({ text: "✕ Failed with 127", tone: "failed" });
    expect(sheetChip(block())).toEqual({ text: "✓ Succeeded", tone: "ok" });
    expect(sheetChip(block({ exit: null }))).toEqual({ text: "Finished", tone: "done" });
  });
  it("keeps a running block's clock going between the gateway's updates", () => {
    const running = block({ state: "running", exit: null, duration_ms: 36000 });
    expect(sheetChip(liveBlock(running, 1000, 29000)).text).toBe("● Running 1:04");
    // a clock read before the update arrived never counts backwards
    expect(liveBlock(running, 5000, 4000).duration_ms).toBe(36000);
    // the pane's start wins over a duration that was already old when the sheet opened
    expect(liveBlock(running, 59000, 60000, 0).duration_ms).toBe(60000);
    const done = block({ duration_ms: 2400 });
    expect(liveBlock(done, 1000, 29000)).toBe(done);
  });
  it("counts the non-empty output lines", () => {
    expect(outputLines("a\n\nb\n")).toBe("2 lines");
    expect(outputLines("a")).toBe("1 line");
    expect(outputLines("")).toBe("0 lines");
  });
});

describe("the agent's question card", () => {
  const w = waiting(["Yes", "Yes, allow all edits during this session (shift+tab)", "No, and tell Claude what to do differently (esc)"]);
  it("labels the answers as the board does", () => {
    expect(w.choices.map((_, i) => answerLabel(w, i))).toEqual(["Yes", "Yes, this session", "No, explain"]);
    expect(answerLabel(waiting(["Allow once", "Yes, and don't ask again for npm", "No"]), 1)).toBe("Yes, always");
    expect(answerLabel(waiting(["Bash(aws:*)", "Bash(curl:*)"]), 1)).toBe("Bash(curl:*)");
    expect(answerLabel(waiting(["Apply the migration to the production database now"]), 0)).toBe("Apply the migration to…");
  });
  it("lists the entries under the primary one", () => {
    expect(otherChoices(w, 0)).toEqual([1, 2]);
    expect(otherChoices(w, 1)).toEqual([0, 2]);
  });
  it("reads the context: file, changed rows, kept rows", () => {
    const ctx = askContext([
      "Edit file",
      "mobile/src/app/(tabs)/computers.tsx",
      "116   <View style={styles.tabLine}>",
      "117 -  <Text>running</Text>",
      "117 +  <StateBadge />",
      "+let plain = 1",
      "--- header",
    ]);
    expect(ctx.file).toBe("mobile/src/app/(tabs)/computers.tsx");
    expect(ctx.added).toBe(2);
    expect(ctx.removed).toBe(1);
    expect(ctx.meta).toEqual(["Edit file", "--- header"]);
    expect(visibleRows(ctx, false).map((r) => [r.n, r.sign])).toEqual([[117, "-"], [117, "+"], [null, "+"]]);
    expect(visibleRows(ctx, true)).toHaveLength(4);
    expect(hasMoreRows(ctx)).toBe(true);
  });
  it("takes a path inside `Update(...)`, and no file from prose", () => {
    expect(askContext(["Update(src/x.ts)", "3 +a"]).file).toBe("src/x.ts");
    expect(askContext(["Do you want to proceed?"]).file).toBeNull();
    expect(hasMoreRows(askContext(["3 +a"]))).toBe(false);
  });
});

describe("fitting the program to the view", () => {
  it("the font is 11 pt at 100 % and follows the zoom within its limits", () => {
    expect(fitFontSize(1)).toBe(11);
    expect(fitFontSize(1.5)).toBe(16.5);
    expect(fitFontSize(9)).toBe(22);
    expect(fitFontSize(0.1)).toBe(6.6);
  });
  it("columns and rows are what fill the view at that font", () => {
    // 11 pt: 6.6 pt a cell, a 15 pt line (the program screen board)
    expect(fitSize(396, 560, 11)).toEqual({ cols: 60, rows: 37 });
    // a bigger font fits fewer
    expect(fitSize(396, 560, 16.5)).toEqual({ cols: 40, rows: 24 });
  });
  it("stays within the desktop's bounds, and waits for a measured view", () => {
    expect(fitSize(60, 30, 11)).toEqual({ cols: 20, rows: 5 });
    expect(fitSize(5000, 5000, 6.6)).toEqual({ cols: 300, rows: 120 });
    expect(fitSize(0, 500, 11)).toBeNull();
    expect(fitSize(300, -4, 11)).toBeNull();
    expect(fitSize(300, 500, 0)).toBeNull();
  });
});

describe("the Terminal view's default", () => {
  const running = { running: { started: null, line: "" } };
  it("an agent pane runs: Claude Code by name, or any session an agent titled", () => {
    expect(runsAgent(pane(1, { job: "claude" }))).toBe(true);
    expect(runsAgent(pane(1, { last_command: "claude", ...running }))).toBe(true);
    expect(runsAgent(pane(1, { last_command: "codex", running: { started: null, line: "", title: "Fix tests" } }))).toBe(true);
  });
  it("a shell, a plain program and an agent that quit are not", () => {
    expect(runsAgent(pane(1))).toBe(false);
    expect(runsAgent(pane(1, { job: "htop" }))).toBe(false);
    expect(runsAgent(pane(1, { last_command: "claude" }))).toBe(false);
  });
  it("TUI for the alternate screen, an agent, or a program no block holds; no switch to choose", () => {
    const running = { running: { started: null, line: "" } };
    // vim, htop, Claude fullscreen: alt wins whatever blocks say
    expect(screenView(pane(1, { job: "vim", ...running }), true, true).tui).toBe(true);
    // Claude classic on the primary screen
    expect(screenView(pane(1, { job: "claude", ...running }), false, true)).toEqual({ tui: true, pinned: true });
    // reattached program or shell without integration: running, but the last block is not it
    expect(screenView(pane(1, { job: "node", ...running }), false, false)).toEqual({ tui: true, pinned: true });
    expect(screenView(pane(1, running), false, false).tui).toBe(true);
    // a shell without integration: no blocks, no folder, no job; only its screen shows anything
    expect(screenView(pane(1), false, false, true)).toEqual({ tui: true, pinned: true });
    expect(screenView(pane(1, { cwd: "/x" }), false, false, true).tui).toBe(false);
    expect(screenView(pane(1), false, false).tui).toBe(false);
  });
  it("blocks for a shell at a prompt and for any program a block holds", () => {
    const running = { running: { started: null, line: "" } };
    expect(screenView(pane(1), false, false)).toEqual({ tui: false, pinned: false });
    expect(screenView(pane(1, { last_command: "ls" }), false, true).tui).toBe(false);
    // cargo build, tail -f, sleep 30: running, held by a running block
    expect(screenView(pane(1, { job: "cargo", ...running }), false, true)).toEqual({ tui: false, pinned: false });
    expect(screenView(pane(1, { job: "sleep", ...running }), false, true).tui).toBe(false);
  });
  it("alt on the alternate screen needs no pin: the gateway's auto mode sends it", () => {
    expect(screenView(pane(1, { job: "vim", running: { started: null, line: "" } }), true, true)).toEqual({ tui: true, pinned: false });
  });
  it("the desktop's agent flag names an agent whatever its job or title", () => {
    // Codex untitled, or an agent reattached with its version as the job
    expect(runsAgent(pane(1, { job: "2.1.308", last_command: "x", running: { started: null, line: "", agent: true } }))).toBe(true);
    expect(runsAgent(pane(1, { job: "node", running: { started: null, line: "", agent: false } }))).toBe(false);
  });
});

describe("the message input or the key bar", () => {
  const base = { fullScreen: true, asking: false, agent: false, pinned: true, alt: false };
  it("an agent gets the message input on either screen", () => {
    // Claude Code's fullscreen renderer is on the alternate screen, its classic one is not
    expect(bareScreen({ ...base, agent: true, alt: true })).toBe(true);
    expect(bareScreen({ ...base, agent: true, alt: false })).toBe(true);
  });
  it("vim, less and htop keep the key bar", () => {
    expect(bareScreen({ ...base, alt: true })).toBe(false);
    expect(bareScreen({ ...base, pinned: false, alt: true })).toBe(false);
  });
  it("a question, and the Terminal view at a prompt, show the input too; the blocks view never", () => {
    expect(bareScreen({ ...base, asking: true, alt: true })).toBe(true);
    expect(bareScreen({ ...base, alt: false })).toBe(true);
    expect(bareScreen({ ...base, fullScreen: false, agent: true })).toBe(false);
  });
});

describe("the command line while a program runs", () => {
  it("sends a line as text", () => {
    expect(programSend("done?", true)).toEqual({ ok: true, text: "done?" });
  });
  it("an empty line sends nothing and says nothing", () => {
    expect(programSend("  ", true)).toEqual({ ok: false, message: null });
  });
  it("a Rift without text says it is too old", () => {
    const r = programSend("done?", false);
    expect(r.ok).toBe(false);
    expect(r.ok === false && r.message).toMatch(/too old/);
  });
  it("several lines, tabs and long text go as one paste", () => {
    expect(programSend("a\r\nb", true, true)).toEqual({ ok: true, paste: "a\nb" });
    expect(programSend("a\tb", true, true)).toEqual({ ok: true, paste: "a\tb" });
    const long = "é".repeat(513);
    expect(programSend(long, true, true)).toEqual({ ok: true, paste: long });
  });
  it("without paste they go line by line, a tab never reaching a program that would complete it", () => {
    expect(programSend("a\n\tb", true, false)).toEqual({ ok: true, lines: ["a", "    b"] });
  });
  it("what no message can carry is refused here", () => {
    expect(programSend("a\u001b[2Jb", true, true)).toMatchObject({ ok: false });
    expect(programSend("x".repeat(48 * 1024 + 1), true, true)).toMatchObject({ ok: false, message: expect.stringMatching(/too long/) });
  });
  it("names each program's mark: an agent's own logo, Claude's only for Claude, >_ for the rest", async () => {
    const { programMark } = await import("../src/paneView");
    expect(programMark("claude").claude).toBe(true);
    for (const p of ["codex", "opencode", "vim", null]) expect(programMark(p).claude).toBe(false);
    expect(new Set(["claude", "codex", "opencode", "vim"].map((p) => programMark(p).logo ?? programMark(p).mark)).size).toBe(4);
  });
});

describe("messagePlaceholder", () => {
  it("names the program only when it fits on the box's one line", () => {
    expect(messagePlaceholder("claude", 390)).toBe("Message for claude");
    expect(messagePlaceholder("claude", 320)).toBe("Message");
    expect(messagePlaceholder("a-program-with-a-long-name", 402)).toBe("Message");
  });
});

describe("fitPlaceholder", () => {
  // 15 pt mono, 9 pt a character: the 402 pt phone leaves 218 pt, 24 characters
  it("keeps the whole placeholder only when it fits on the field's line", () => {
    expect(fitPlaceholder("Waiting for your computer…", "Waiting…", 402)).toBe("Waiting…");
    expect(fitPlaceholder("Waiting for your computer…", "Waiting…", 834)).toBe("Waiting for your computer…");
    expect(fitPlaceholder("x".repeat(24), "short", 402)).toBe("x".repeat(24));
    expect(fitPlaceholder("x".repeat(25), "short", 402)).toBe("short");
  });
});

describe("boxImages", () => {
  const rows = (...t: string[]) => t.map((s) => [[s, null, null, 0] as const]);
  it("counts the box below the last prompt, not the transcript", () => {
    expect(boxImages(rows("❯ [Image #1] old", "  ⎿ [Image #1]", "──", "❯ [Image #2] x [Image #3]", "──"))).toBe(2);
    expect(boxImages(rows("❯ [Image #1] old", "❯ "))).toBe(0);
    expect(boxImages(rows("no prompt [Image #1]"))).toBe(0);
  });
});

describe("partSend", () => {
  it("blanks between tokens go, and with no typing at all it says why rather than nothing", () => {
    expect(partSend(" ", true)).toEqual({ ok: true, lines: [" "] });
    expect(partSend(" ", false)).toEqual({ ok: false, message: expect.stringContaining("too old") });
  });
});
