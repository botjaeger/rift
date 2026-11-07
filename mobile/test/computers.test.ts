import { describe, expect, it } from "vitest";
import type { HostView } from "../src/fleet";
import {
  agoCompact,
  EMPTY_FILTER,
  homeChoiceLabel,
  homeRows,
  homeSummary,
  quickChoices,
  sameTabRow,
  sheetCounts,
  sheetRows,
  stateText,
  tabKey,
  tabKind,
  tabRow,
  trackIdle,
  type RowContext,
} from "../src/now";
import type { Pane, Tab, Waiting } from "../src/protocol/types";

const pane = (id: number, extra: Partial<Pane> = {}): Pane => ({
  id,
  active: false,
  cwd: null,
  job: null,
  last_command: null,
  last_exit: null,
  ...extra,
});
const tab = (panes: Pane[], title = "rift", index = 0): Tab => ({ index, title, active: false, panes });
const host = (tabs: Tab[]): HostView =>
  ({
    record: { key: "A", name: "Sam’s MacBook Pro", addresses: [], permission: "operate", pairedAt: "" },
    conn: { status: "open" },
    welcome: null,
    windows: [{ pid: 1, tabs }],
    notice: null,
    lastSeen: null,
  }) as unknown as HostView;
const menu: Waiting = {
  kind: "choice",
  prompt: "Edit mobile/src/app/index.tsx?\nsecond line",
  choices: ["Yes", "Yes, allow all edits this session", "No"],
  selected: 0,
  numbered: true,
  context: [],
};
const NOW = 10_000_000;
const ctx = (extra: Partial<RowContext> = {}): RowContext => ({ now: NOW, multi: false, waitingSince: () => null, asked: [], ...extra });

const asking = tab([pane(1, { job: "claude", last_command: "claude", cwd: "/Users/me/zzz/rift", waiting: menu, git: { branch: "main", ahead: 0, behind: 0, dirty: false, files: 0, added: 0, removed: 0 } })], "RIFT");
const working = tab(
  [pane(2, { job: "claude", last_command: "claude", running: { started: (NOW - 236_000) / 1000, line: "pnpm test — 14 passed", title: "Quill engine" } })],
  "q",
  1,
);
const idleAgent = tab([pane(3, { job: "claude", last_command: "claude", running: { started: 1, line: "", title: "Code changes overview", idle: true } })], "c", 2);
const failed = tab([pane(4, { cwd: "/Users/me/xxx/flow", last_command: "psa", last_exit: 1 }), pane(5, { last_exit: 2 }), pane(6, { last_exit: 0 })], "flow", 3);
const quiet = tab([pane(7, { last_exit: 0 })], "zsh", 4);

describe("tabKind", () => {
  it("tells needs, working, failed and idle apart", () => {
    expect([asking, working, idleAgent, failed, quiet].map(tabKind)).toEqual(["needs", "working", "idle", "failed", "idle"]);
  });

  it("does not count a coding agent at its own prompt as working", () => {
    expect(tabKind(idleAgent)).toBe("idle");
    expect(tabKind(tab([pane(1, { job: "claude", running: { started: 1, line: "x", idle: true } }), pane(2, { job: "make" })]))).toBe("working");
  });

  it("does not call a pane failed while it runs something", () => {
    expect(tabKind(tab([pane(1, { job: "vim", last_exit: 1 })]))).toBe("working");
  });
});

describe("stateText", () => {
  it("writes the mockup's four texts", () => {
    expect(stateText("needs", { ageMs: 120_000, elapsedMs: null, failed: 0 })).toBe("Needs you · 2m");
    expect(stateText("working", { ageMs: null, elapsedMs: 236_000, failed: 0 })).toBe("Working 3:56");
    expect(stateText("idle", { ageMs: 12 * 60_000, elapsedMs: null, failed: 0 })).toBe("Idle 12m");
    expect(stateText("failed", { ageMs: null, elapsedMs: null, failed: 2 })).toBe("2 failed");
  });

  it("leaves the time out when it is unknown", () => {
    expect(stateText("needs", { ageMs: null, elapsedMs: null, failed: 0 })).toBe("Needs you");
    expect(stateText("working", { ageMs: null, elapsedMs: null, failed: 0 })).toBe("Working");
    expect(stateText("idle", { ageMs: null, elapsedMs: null, failed: 0 })).toBe("Idle");
  });

  it("ages in m, h and d", () => {
    expect([0, 59_999, 60_000, 3_600_000, 90_000_000].map(agoCompact)).toEqual(["now", "now", "1m", "1h", "1d"]);
  });
});

describe("tabRow", () => {
  const h = host([asking, working, idleAgent, failed]);
  const rows = (c: RowContext, idle = new Map<string, number>()) => h.windows[0]!.tabs.map((t) => tabRow(h, t, c, idle));

  it("titles a row by its tab; its line is what the lead pane does (its tile names the program); the short time goes beside the mark", () => {
    const [a, w, i, f] = rows(ctx({ waitingSince: () => NOW - 120_000 }), new Map([[tabKey("A", idleAgent), NOW - 12 * 60_000]]));
    expect(a).toMatchObject({ title: "RIFT", state: "Needs you · 2m", line: "Waiting for you · Edit mobile/src/app/index.tsx?", when: "2m", agent: true, panes: 1 });
    expect(a!.ask).toMatchObject({ pane: 1, program: "claude" });
    // an agent's session title is its line now, never the row's title
    expect(w).toMatchObject({ title: "q", state: "Working 3:56", line: "running · pnpm test — 14 passed", when: "3:56" });
    expect(i).toMatchObject({ title: "c", state: "Idle 12m", line: "Idle · Code changes overview", when: "12m" });
    // several panes: their count leads the line; the tab's words count its failures
    expect(f).toMatchObject({ title: "flow", state: "2 failed", line: "3 panes · psa · failed with 1", when: "✕ 1", agent: false, panes: 3 });
  });

  it("tells a row built again with what it draws from one that changed", () => {
    const c = ctx({ waitingSince: () => NOW - 120_000 });
    const [a1, w1] = rows(c);
    const [a2, w2] = rows(c);
    // built afresh from the same state: a new object that draws the same
    expect(a2).not.toBe(a1);
    expect(sameTabRow(a1!, a2!)).toBe(true);
    expect(sameTabRow(w1!, w2!)).toBe(true);
    // a second later the working clock moved: that row draws again, the asking one does not
    const later = rows(ctx({ now: NOW + 1000, waitingSince: () => NOW - 120_000 }));
    expect(sameTabRow(w1!, later[1]!)).toBe(false);
    expect(sameTabRow(a1!, later[0]!)).toBe(true);
    expect(sameTabRow(a1!, { ...a1!, title: "other" })).toBe(false);
    expect(sameTabRow(a1!, { ...a1!, when: "9m" })).toBe(false);
    expect(sameTabRow(a1!, { ...a1!, tab: { ...a1!.tab, pinned: !a1!.tab.pinned } })).toBe(false);
  });


  it("puts the failed pane's time beside a failed tab's mark, whichever pane speaks for it", () => {
    const split = tab([pane(20, { last_command: "make", last_exit: 2 }), pane(21, { active: true, cwd: "/Users/me/code", last_command: "ls", last_exit: 0 })], "build");
    const r = tabRow(host([split]), split, ctx(), new Map());
    expect(r).toMatchObject({ kind: "failed", when: "✕ 2", line: "2 panes · make · failed with 2", paneLine: "make · failed with 2" });
  });
  it("words a shell's command, an idle shell and an unknown time", () => {
    const shell = tab([pane(9, { job: "cargo", last_command: "cargo test", running: { started: null, line: "" } })], "api");
    expect(tabRow(host([shell]), shell, ctx(), new Map())).toMatchObject({ line: "cargo test · running", when: "" });
    const idleShell = tab([pane(10, { cwd: "/Users/me/code", last_command: "ls", last_exit: 0 })], "work");
    expect(tabRow(host([idleShell]), idleShell, ctx(), new Map())).toMatchObject({ kind: "idle", line: "Idle · last: ls", when: "" });
    // nothing run yet: where it is, unless the tab is named after that folder
    const there = tab([pane(13, { cwd: "/Users/me/code" })], "work");
    expect(tabRow(host([there]), there, ctx(), new Map()).line).toBe("Idle · ~/code");
    const home = tab([pane(12, { cwd: "/Users/me/code" })], "code");
    expect(tabRow(host([home]), home, ctx(), new Map()).line).toBe("Idle");
    const fresh = tab([pane(11, {})], "new");
    expect(tabRow(host([fresh]), fresh, ctx(), new Map()).line).toBe("Idle");
  });

  it("takes the start of a working pane from a Notify me when the computer sends none", () => {
    const t = tab([pane(8, { job: "cargo", last_command: "cargo build", running: { started: null, line: "" } })]);
    const hh = host([t]);
    const r = tabRow(hh, t, ctx({ asked: [{ host: "A", pane: 8, command: "cargo build", at: NOW - 72_000 }] }), new Map());
    expect(r.state).toBe("Working 1:12");
  });

  it("marks a view-only device so the pills stay away", () => {
    const hh = host([asking]);
    (hh.record as { permission: string }).permission = "view";
    expect(tabRow(hh, asking, ctx(), new Map()).operate).toBe(false);
  });

  it("keeps the idle time across a working → idle change (trackIdle)", () => {
    const since = new Map<string, number>();
    const seen = new Set<string>();
    const key = tabKey("A", idleAgent);
    trackIdle(since, seen, [{ key }], [], 1000);
    trackIdle(since, seen, [], [{ key }], 5000);
    expect(tabRow(h, idleAgent, ctx({ now: 5000 + 12 * 60_000 }), since).state).toBe("Idle 12m");
  });
});

describe("Portals summary and cards", () => {
  const all = [asking, working, idleAgent, failed, quiet].map((t) => ({ kind: tabKind(t), id: t.title }));

  it("counts the tabs by state in words, what needs you first, none at zero", () => {
    expect(homeSummary(all).map((p) => p.text)).toEqual(["1 needs you", "1 working", "1 failed", "2 idle"]);
    expect(homeSummary(all.filter((r) => r.kind === "idle")).map((p) => p.text)).toEqual(["2 idle"]);
    expect(homeSummary([])).toEqual([]);
  });

  it("shows the tabs that matter first, at most five, each state in tab order", () => {
    const order = homeRows([{ id: "a", kind: "idle" }, { id: "b", kind: "working" }, { id: "c", kind: "needs" }, { id: "d", kind: "idle" }, { id: "e", kind: "failed" }] as const);
    expect(order.map((r) => r.id)).toEqual(["c", "b", "e", "a", "d"]);
    const thirty = Array.from({ length: 30 }, (_, i) => ({ id: `t${i}`, kind: i === 20 ? ("needs" as const) : ("idle" as const) }));
    expect(homeRows(thirty).map((r) => r.id)).toEqual(["t20", "t0", "t1", "t2", "t3"]);
  });
});

describe("Tabs sheet", () => {
  const rows = [quiet, working, failed, asking, idleAgent].map((t) => ({ kind: tabKind(t), id: t.title }));

  it("keeps the computer's tab order, whatever state each tab is in", () => {
    expect(sheetRows(rows, "all").map((r) => r.id)).toEqual(["zsh", "q", "flow", "RIFT", "c"]);
  });

  it("filters one state and counts each", () => {
    expect(sheetRows(rows, "failed").map((r) => r.id)).toEqual(["flow"]);
    expect(sheetCounts(rows)).toEqual({ all: 5, needs: 1, working: 1, failed: 1, idle: 2 });
  });

  it("words the empty state", () => {
    expect(sheetRows(rows, "needs")).toHaveLength(1);
    expect(EMPTY_FILTER.needs).toEqual({ title: "Nothing needs you", line: "Agents that ask a question show here." });
    expect(sheetRows([], "failed")).toEqual([]);
  });
});

describe("answer pills", () => {
  it("reads Yes, Always, No as the mockup does", () => {
    const labels = quickChoices(menu)!.map((q) => homeChoiceLabel(q.label));
    expect(labels).toEqual(["Yes", "Always", "No"]);
    expect(homeChoiceLabel("Maybe")).toBe("Maybe");
  });
});
