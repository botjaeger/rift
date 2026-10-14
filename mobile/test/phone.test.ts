import { describe, expect, it } from "vitest";
import { alertAnswers, answerPlan, askingTitle, contextLine, keepAsked, primaryChoice, replyChoice, sameQuestion } from "../src/answer";
import type { ActivityItem, AskedView, HostView } from "../src/fleet";
import { livePlan, liveKey } from "../src/live";
import { agoShort, foldBlocks, nowSections, pagerModel, programName, quickChoices, swipeTarget, tabLast, tabState } from "../src/now";
import { alertAuth } from "../src/lock";
import { tapTarget } from "../src/notify";
import { DEFAULT_SETTINGS } from "../src/stores";
import type { Block, Pane, Tab, Waiting } from "../src/protocol/types";

const block = (id: number, extra: Partial<Block> = {}): Block => ({
  id,
  command: `cmd${id}`,
  state: "done",
  exit: 0,
  duration_ms: 1,
  cwd: null,
  branch: null,
  output: "",
  dropped: 0,
  restored: false,
  ...extra,
});
const pane = (id: number, extra: Partial<Pane> = {}): Pane => ({
  id,
  active: false,
  cwd: null,
  job: null,
  last_command: null,
  last_exit: null,
  ...extra,
});
const tab = (panes: Pane[], title = "rift"): Tab => ({ index: 0, title, active: true, panes });
const host = (key: string, panes: Pane[], extra: Partial<HostView> = {}): HostView =>
  ({
    record: { key, name: `host-${key}`, addresses: ["10.0.0.1:1"], permission: "operate", pairedAt: "" },
    conn: { status: "open" },
    gateway: null,
    welcome: null,
    windows: [{ pid: 1, tabs: [tab(panes)] }],
    notice: null,
    lastSeen: null,
    ...extra,
  }) as unknown as HostView;
const menu: Waiting = {
  kind: "choice",
  prompt: "Do you want to make this edit to app.txt?",
  choices: ["Yes", "Yes, and switch to accept edits for this session (shift+tab)", "No"],
  selected: 0,
  numbered: true,
  context: [],
};
const trust: Waiting = { ...menu, prompt: "Trust this folder?", choices: ["No, exit", "Yes, I trust this folder"], numbered: false };
const password: Waiting = { kind: "password", prompt: "[sudo] password for me:", choices: [], selected: 0, numbered: false, context: [] };
const done = (id: string, exit: number | null, at: number, extra: Partial<ActivityItem> = {}): ActivityItem => ({
  id,
  host: "A",
  hostName: "host-A",
  pane: 1,
  block: 1,
  tab: "rift",
  command: id,
  exit,
  durationMs: 1,
  at,
  ...extra,
});

describe("Now", () => {
  it("splits panes into needs you, running and finished (failures first, then newest)", () => {
    const hosts = [
      host("A", [
        pane(1, { job: "claude", last_command: "claude", waiting: menu }),
        pane(2, { job: "cargo", last_command: "cargo test", running: { started: 100, line: "test x ... ok" } }),
        pane(3),
      ]),
      host("B", [pane(9, { job: "make", waiting: password })], { conn: { status: "closed" } } as Partial<HostView>),
    ];
    const asked: AskedView[] = [{ host: "A", pane: 2, command: "cargo test", at: 5 }];
    const s = nowSections(hosts, [done("ok-new", 0, 30), done("fail-old", 1, 10), done("ok-old", 0, 20), done("fail-new", 2, 25)], asked);
    expect(s.needs.map((n) => [n.pane, n.program, n.waiting.kind])).toEqual([[1, "claude", "choice"]]);
    expect(s.running).toEqual([
      expect.objectContaining({ pane: 2, command: "cargo test", started: 100_000, line: "test x ... ok", notifies: true }),
    ]);
    expect(s.finished.map((f) => f.id)).toEqual(["fail-new", "fail-old", "ok-new", "ok-old"]);
  });

  it("gives an agent's menu short buttons; No opens the reply; more than three: none", () => {
    expect(quickChoices(menu)).toEqual([
      { label: "Yes", index: 0, opens: false },
      { label: "Yes, always", index: 1, opens: false },
      { label: "No…", index: 2, opens: true },
    ]);
    // "No, exit" is an answer of its own, not "No, and tell it what to do"
    expect(quickChoices(trust)).toEqual([
      { label: "No", index: 0, opens: false },
      { label: "Yes", index: 1, opens: false },
    ]);
    expect(quickChoices({ ...menu, choices: ["a", "b", "c", "d"] })).toBeNull();
    expect(quickChoices(password)).toBeNull();
    // two buttons that would read the same: the card opens the answer page instead
    expect(quickChoices({ ...menu, choices: ["Yes, for this file", "Yes, for this folder", "No"] })).toBeNull();
    expect(quickChoices({ ...menu, choices: ["Continue with the plan", "Continue with changes", "No"] })).toBeNull();
  });

  it("names the program: its job, else (a version, or none) the command", () => {
    expect(programName({ job: "sudo", last_command: "sudo apt upgrade" })).toBe("sudo");
    expect(programName({ job: "2.1.296", last_command: "cd x && git pull; claude --resume" })).toBe("claude");
    expect(programName({ job: null, last_command: "read -s pw; echo done" })).toBe("read");
    expect(programName({ job: null, last_command: "/usr/bin/vim a" })).toBe("vim");
    expect(programName({ job: null, last_command: null })).toBeNull();
  });

  it("says how long ago, Now and Run style", () => {
    expect([0, 59_999, 60_000, 3_599_999, 3_600_000, 86_400_000, 3 * 86_400_000].map(agoShort)).toEqual([
      "now",
      "now",
      "1 min",
      "59 min",
      "1 h",
      "yesterday",
      "3 d",
    ]);
  });
});

describe("the pane", () => {
  const ok = (id: number) => block(id);
  const failed = (id: number) => block(id, { exit: 101 });
  const running = (id: number) => block(id, { state: "running", exit: null });

  it("folds the successes right before the newest failure", () => {
    const items = foldBlocks([ok(1), ok(2), ok(3), failed(4), running(5)]);
    expect(items.map((i) => (i.kind === "fold" ? `fold${i.count}` : i.block.id))).toEqual(["fold3", 4, 5]);
    expect(items[0]).toMatchObject({ ids: [1, 2, 3] });
  });

  it("folds before the running block; folds every run of successes but keeps the newest block", () => {
    const shape = (items: ReturnType<typeof foldBlocks>) => items.map((i) => (i.kind === "fold" ? `fold${i.count}` : i.block.id));
    expect(shape(foldBlocks([ok(1), ok(2), running(3)]))).toEqual(["fold2", 3]);
    expect(shape(foldBlocks([failed(1), ok(2), ok(3), failed(4), ok(5), ok(6)]))).toEqual([1, "fold2", 4, 5, 6]);
    // a failure followed by many successes: the successes fold too, the newest stays
    expect(shape(foldBlocks([failed(1), ok(2), ok(3), ok(4), ok(5)]))).toEqual([1, "fold3", 5]);
    expect(shape(foldBlocks([ok(1), ok(2), ok(3)]))).toEqual(["fold2", 3]);
  });

  it("does not fold one success, unknown exits, the newest block alone, or when open", () => {
    expect(foldBlocks([ok(1), failed(2)]).every((i) => i.kind === "block")).toBe(true);
    expect(foldBlocks([block(1, { exit: null }), block(2, { exit: null }), failed(3)]).every((i) => i.kind === "block")).toBe(true);
    expect(foldBlocks([ok(1), ok(2)]).every((i) => i.kind === "block")).toBe(true);
    expect(foldBlocks([ok(1), ok(2), failed(3)], true).every((i) => i.kind === "block")).toBe(true);
    expect(foldBlocks([])).toEqual([]);
  });

  it("pages between a tab's panes", () => {
    const t = tab([pane(1, { last_command: "cargo test" }), pane(2, { job: "2.1.9", last_command: "claude" }), pane(3)]);
    expect(pagerModel(t, 1)).toEqual({ text: "Pane 1 of 3 · swipe for claude", index: 0, count: 3 });
    expect(pagerModel(t, 3)).toEqual({ text: "Pane 3 of 3 · swipe for claude", index: 2, count: 3 });
    expect(pagerModel(tab([pane(1)]), 1)).toBeNull();
    expect([swipeTarget(t, 1, 1), swipeTarget(t, 1, -1), swipeTarget(t, 3, 1), swipeTarget(t, 3, -1)]).toEqual([2, null, null, 2]);
  });

  it("marks a tab: needs you, running, failed", () => {
    expect(tabState(tab([pane(1, { waiting: menu }), pane(2, { job: "x" })]))).toBe("needs");
    expect(tabState(tab([pane(1, { last_exit: 1 }), pane(2, { job: "x" })]))).toBe("running");
    expect(tabState(tab([pane(1, { last_exit: 1 })]))).toBe("failed");
    expect(tabState(tab([pane(1, { last_exit: 0 })]))).toBe("idle");
    expect(tabLast(tab([pane(1, { last_command: "ls" }), pane(2, { waiting: menu })]))).toBe("waiting for you");
  });
});

describe("answering", () => {
  it("plans a numbered pick as its digit, an unnumbered one as arrows and Enter", () => {
    expect(answerPlan(menu, { choice: 1 })).toEqual([{ text: "2" }]);
    expect(answerPlan(trust, { choice: 1 })).toEqual([{ keys: ["down", "enter"] }]);
    expect(answerPlan({ ...trust, selected: 1 }, { choice: 0 })).toEqual([{ keys: ["up", "enter"] }]);
    expect(answerPlan({ ...trust, selected: 1 }, { choice: 1 })).toEqual([{ keys: ["enter"] }]);
  });

  it("plans a reply after No with a pause, the text and Enter; a blank reply is the bare No", () => {
    expect(answerPlan(menu, { choice: 2, reply: "  use lowercase " })).toEqual([
      { text: "3" },
      { pauseMs: 600 },
      { text: "use lowercase" },
      { keys: ["enter"] },
    ]);
    expect(answerPlan(menu, { choice: 2, reply: "  " })).toEqual([{ text: "3" }]);
  });

  it("types a password or other text with Enter, and refuses picks that do not fit", () => {
    expect(answerPlan(password, { text: "s3cret" })).toEqual([{ text: "s3cret" }, { keys: ["enter"] }]);
    expect(answerPlan(password, { text: "" })).toBeNull();
    expect(answerPlan(password, { choice: 0 })).toBeNull();
    expect(answerPlan(menu, { text: "yes" })).toBeNull();
    expect(answerPlan(menu, { choice: 3 })).toBeNull();
    expect(answerPlan(menu, { choice: -1 })).toBeNull();
  });

  it("finds the reply, primary, and alert answers", () => {
    expect([replyChoice(menu), replyChoice(trust), replyChoice(password)]).toEqual([2, null, null]);
    expect([primaryChoice(menu), primaryChoice(trust), primaryChoice({ ...menu, choices: ["a", "b"] })]).toEqual([0, 1, 0]);
    expect([alertAnswers(menu), alertAnswers(trust), alertAnswers(password), alertAnswers({ ...menu, choices: ["Yes", "Skip"] })]).toEqual([
      { yes: 0, no: 2 },
      { yes: 1, no: 0 },
      null,
      null,
    ]);
  });

  it("reads the context's diff lines and titles the question", () => {
    expect([" 12 +let x", " 12 -let y", " 3  kept", "+plain", "Edit file", "@@ -1 +1 @@", "--flag"].map(contextLine)).toEqual([
      { kind: "added", text: "+let x" },
      { kind: "removed", text: "-let y" },
      { kind: "kept", text: " kept" },
      { kind: "added", text: "+plain" },
      { kind: "meta", text: "Edit file" },
      { kind: "meta", text: "@@ -1 +1 @@" },
      { kind: "meta", text: "--flag" },
    ]);
    expect([askingTitle(menu, "claude"), askingTitle(password, "sudo"), askingTitle({ ...password, kind: "input" }, null)]).toEqual([
      "claude is asking",
      "sudo needs a password",
      "The command needs your input",
    ]);
    expect(sameQuestion(menu, { ...menu })).toBe(true);
    expect(sameQuestion(menu, { ...menu, choices: ["Yes", "No"] })).toBe(false);
    expect(sameQuestion(undefined, menu)).toBe(false);
    // the same wording about other lines (another edit) is another question
    expect(sameQuestion({ ...menu, context: [" 2 +other"] }, menu)).toBe(false);
  });

  it("keeps the question first shown, takes one that loads later, and starts over for another pane", () => {
    const first = { where: "H:1", w: undefined };
    const loaded = keepAsked(first, "H:1", menu);
    expect(loaded).toEqual({ where: "H:1", w: menu });
    // the program moved on to another question: the page still answers (and checks) the first
    expect(keepAsked(loaded, "H:1", trust)).toBe(loaded);
    expect(keepAsked(loaded, "H:1", undefined)).toBe(loaded);
    expect(keepAsked(first, "H:1", undefined)).toBe(first);
    expect(keepAsked(loaded, "H:2", trust)).toEqual({ where: "H:2", w: trust });
  });

  it("an alert's answer asks for the lock's unlock, or Face ID with the lock on, whatever Ask before running says", () => {
    expect(alertAuth(DEFAULT_SETTINGS, false)).toBe("none");
    expect(alertAuth({ ...DEFAULT_SETTINGS, biometricLock: true, askBeforeRun: false }, false)).toBe("authenticate");
    expect(alertAuth({ ...DEFAULT_SETTINGS, biometricLock: true }, true)).toBe("unlock");
  });

  it("reads an alert's action and its question", () => {
    const r = (actionIdentifier: string | undefined, data: unknown, id = "n1") => ({
      actionIdentifier,
      notification: { request: { identifier: id, content: { data } } },
    });
    expect(tapTarget(r("yes", { host: "H", pane: 1, ask: "Q?" }), new Set())).toEqual({ host: "H", pane: 1, ask: "Q?", action: "yes" });
    expect(tapTarget(r("no", { host: "H", pane: 1, ask: "Q?" }), new Set())).toMatchObject({ action: "no" });
    expect(tapTarget(r("com.apple.UNNotificationDefaultActionIdentifier", { host: "H", pane: 1, ask: "Q?" }), new Set())).toMatchObject({
      action: "open",
    });
    // a finish alert has no question: it opens the pane as before
    expect(tapTarget(r("yes", { host: "H", pane: 1 }), new Set())).toEqual({ host: "H", pane: 1 });
  });
});

describe("Live Activities", () => {
  const a: AskedView = { host: "A", pane: 2, command: "cargo test", at: 5_000 };
  const hosts = [host("A", [pane(2, { job: "cargo", last_command: "cargo test", running: { started: 4, line: "test a ... ok" } })])];

  it("starts one per wait, with the computer's start time and place", () => {
    const plan = livePlan(new Map(), [a], hosts, []);
    expect(plan).toEqual({
      start: [{ key: liveKey(a), wait: a, command: "cargo test", place: "host-A · rift", startedAt: 4_000, line: "test a ... ok" }],
      update: [],
      end: [],
    });
  });

  it("updates a changed line, and ends a finished wait with its exit status", () => {
    const shown = new Map([[liveKey(a), { wait: a, id: "act1", line: "old" }]]);
    expect(livePlan(shown, [a], hosts, []).update).toEqual([{ key: liveKey(a), id: "act1", line: "test a ... ok" }]);
    expect(livePlan(new Map([[liveKey(a), { wait: a, id: "act1", line: "test a ... ok" }]]), [a], hosts, []).update).toEqual([]);
    const end = livePlan(shown, [], hosts, [done("cargo test", 101, 9, { pane: 2 })]).end;
    expect(end).toEqual([{ key: liveKey(a), id: "act1", exit: 101, line: "old" }]);
  });

  it("ends a multi-line command's activity with its exit status", () => {
    const multi: AskedView = { ...a, command: "for i in 1 2; do\n  make\ndone" };
    const shown = new Map([[liveKey(multi), { wait: multi, id: "act2", line: "" }]]);
    const end = livePlan(shown, [], hosts, [done(multi.command, 2, 9, { pane: 2, command: multi.command })]).end;
    expect(end).toEqual([{ key: liveKey(multi), id: "act2", exit: 2, line: "" }]);
  });
});
