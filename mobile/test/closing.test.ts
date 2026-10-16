import { describe, expect, it } from "vitest";
import { afterClose, closeConfirm, closedText, closesCurrent, paneBusy, runningCount } from "../src/closing";
import type { Pane, Tab, Window } from "../src/protocol/types";

const pane = (id: number, extra: Partial<Pane> = {}): Pane => ({
  id,
  active: false,
  cwd: null,
  job: null,
  last_command: null,
  last_exit: null,
  ...extra,
});
const tab = (index: number, panes: Pane[]): Tab => ({ index, title: `t${index}`, active: false, panes });
const win = (tabs: Tab[]): Window => ({ pid: 1, tabs }) as Window;

describe("asking before a close", () => {
  const busy = pane(2, { job: "npm run dev" });
  const t = tab(0, [pane(1), busy, pane(3, { running: { started: 1, line: "x" } })]);

  it("counts the running commands a close ends: one pane's, or every busy pane of the tab", () => {
    expect(paneBusy(busy)).toBe(true);
    expect(paneBusy(pane(1))).toBe(false);
    expect(runningCount(t, 1, "pane")).toBe(0);
    expect(runningCount(t, 2, "pane")).toBe(1);
    expect(runningCount(t, 1, "tab")).toBe(2);
  });

  it("nothing running: no question, closes at once; running: the question with the right button", () => {
    expect(closeConfirm("pane", "rift · pane 1", 0)).toBeNull();
    expect(closeConfirm("tab", "rift", 1)).toEqual({
      title: "Close “rift”?",
      message: "1 running command ends on your computer.",
      action: "Close tab",
    });
    expect(closeConfirm("tab", "rift", 2)!.message).toBe("2 running commands end on your computer.");
    expect(closeConfirm("pane", "rift · pane 2", 1)!.action).toBe("Close pane");
  });

  it("the toast", () => {
    expect(closedText("pane", "rift · pane 2")).toBe("Pane “rift · pane 2” closed");
    expect(closedText("tab", "rift")).toBe("Tab “rift” closed");
  });
});

describe("where the screen goes after a close", () => {
  const w = win([
    tab(0, [pane(1), pane(2)]),
    tab(1, [pane(3, { active: false }), pane(4, { active: true })]),
    tab(2, [pane(5)]),
  ]);

  it("closing a pane: the pane beside it, the next one first", () => {
    expect(afterClose([w], 1, "pane")).toBe(2);
    expect(afterClose([w], 2, "pane")).toBe(1);
  });

  it("closing a tab, or a tab's last pane: the neighbouring tab's active pane", () => {
    expect(afterClose([w], 1, "tab")).toBe(4);
    expect(afterClose([w], 5, "pane")).toBe(4);
    expect(afterClose([w], 5, "tab")).toBe(4);
    expect(afterClose([w], 3, "tab")).toBe(5);
  });

  it("nothing left: null, so the screen goes to Computers", () => {
    expect(afterClose([win([tab(0, [pane(1)])])], 1, "pane")).toBeNull();
    expect(afterClose([win([tab(0, [pane(1), pane(2)])])], 1, "tab")).toBeNull();
    expect(afterClose([w], 99, "pane")).toBeNull();
  });

  it("another window's tab will do when the window has no other", () => {
    const other = win([tab(0, [pane(7)])]);
    expect(afterClose([win([tab(0, [pane(1)])]), other], 1, "pane")).toBe(7);
  });

  it("a close from a sheet leaves the screen alone unless it closes the pane on screen", () => {
    const t0 = w.tabs[0]!;
    expect(closesCurrent(t0, 1, 2, "pane")).toBe(false);
    expect(closesCurrent(t0, 1, 1, "pane")).toBe(true);
    expect(closesCurrent(t0, 1, 2, "tab")).toBe(true);
    expect(closesCurrent(w.tabs[1]!, 1, 3, "tab")).toBe(false);
  });
});
