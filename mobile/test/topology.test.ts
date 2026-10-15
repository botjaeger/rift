import { describe, expect, it } from "vitest";
import { redirectSystemPath } from "../src/app/+native-intent";
import { reducePane } from "../src/protocol/store";
import type { Block, Pane, Tab, Window } from "../src/protocol/types";
import { findPane, paneBranch, paneTitle, runningCommand, tabTitle } from "../src/topology";

const pane = (id: number, extra: Partial<Pane> = {}): Pane => ({
  id,
  active: false,
  cwd: "/r",
  job: null,
  last_command: null,
  last_exit: null,
  ...extra,
});
const windows: Window[] = [
  { pid: 1, tabs: [{ index: 0, title: "rift", active: true, panes: [pane(12), pane(13)] }] },
  { pid: 2, tabs: [{ index: 3, title: "", active: true, panes: [pane(20)] }] },
];
const block = (id: number, extra: Partial<Block> = {}): Block => ({
  id,
  command: `cmd ${id}`,
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
const state = (blocks: Block[]) => reducePane(null, { t: "snapshot", pane: 12, seq: 1, blocks, has_older: false });

describe("topology helpers", () => {
  it("finds panes and titles them as tab · pane", () => {
    expect(paneTitle(findPane(windows, 13)!)).toBe("rift · pane 2");
    expect(paneTitle(findPane(windows, 20)!)).toBe("Tab 4 · pane 1");
    expect(findPane(windows, 99)).toBeNull();
  });

  it("runningCommand prefers the live block, then the topology job", () => {
    expect(runningCommand(pane(12), state([block(1), block(2, { state: "running", command: "vim" })]))).toBe("vim");
    expect(runningCommand(pane(12, { job: "cargo" }), state([block(1)]))).toBeNull();
    expect(runningCommand(pane(12, { job: "cargo" }), null)).toBe("cargo");
    expect(runningCommand(pane(12), null)).toBeNull();
  });

  it("paneBranch is the newest reported branch", () => {
    expect(paneBranch(state([block(1, { branch: "main" }), block(2, { branch: "dev" }), block(3)]))).toBe("dev");
    expect(paneBranch(null)).toBeNull();
  });
});

describe("deep links", () => {
  it("routes a pairing link to the confirmation with the whole link", () => {
    const link = "rift-remote://pair?a=10.0.0.5:7878&k=K&s=S&n=mac&p=operate";
    expect(redirectSystemPath({ path: link, initial: true })).toBe(`/pair?link=${encodeURIComponent(link)}`);
    expect(redirectSystemPath({ path: "/host/abc", initial: false })).toBe("/host/abc");
  });
});

describe("tabTitle", () => {
  const tab = (title: string, index = 0) => ({ title, index, panes: [] }) as unknown as Tab;
  it("drops a coding agent's status mark from the title", () => {
    expect(["✳ review", "◐ Code changes overview", "⠂ build", "✳\uFE0F RIFT"].map((t) => tabTitle(tab(t)))).toEqual([
      "review",
      "Code changes overview",
      "build",
      "RIFT",
    ]);
  });
  it("keeps other titles, and names an untitled tab by its number", () => {
    expect(tabTitle(tab("flow"))).toBe("flow");
    expect(tabTitle(tab("C++ build"))).toBe("C++ build");
    expect(tabTitle(tab("", 2))).toBe("Tab 3");
    expect(tabTitle(tab("✳", 0))).toBe("Tab 1");
  });
});
