import { describe, expect, it, vi } from "vitest";
import { launchAsks, launchProgram, launchVisit, programLine, PROGRAMS, READY_TRIES, runWhenReady } from "../src/launch";
import { DEFAULT_SETTINGS } from "../src/stores";
import type { ActionResult } from "../src/fleet";

const shell = PROGRAMS[0]!;
const claude = PROGRAMS[1]!;
const busy: ActionResult = { ok: false, code: "busy", message: "a command is running" };

describe("the New tab and New pane rows", () => {
  it("lists a shell, Claude Code, Codex and OpenCode, in that order", () => {
    expect(PROGRAMS.map((p) => [p.title, p.command])).toEqual([
      ["Shell", null],
      ["Claude Code", "claude"],
      ["Codex", "codex"],
      ["OpenCode", "opencode"],
    ]);
  });
  it("words each row as the boards do", () => {
    expect(programLine(shell, "tab")).toBe("A plain shell");
    expect(programLine(shell, "pane")).toBe("A plain shell");
    expect(programLine(PROGRAMS[2]!, "pane")).toBe("Opens a shell, then runs codex");
  });
});

describe("what starting a program asks first", () => {
  const lockOn = { ...DEFAULT_SETTINGS, biometricLock: true, authBeforeRun: true };
  it("a shell types nothing, so it never asks", () => {
    expect(launchAsks(shell, { ...lockOn, askBeforeRun: true })).toEqual({ confirm: false, auth: false });
  });
  it("a program shows its name only with Ask before running", () => {
    expect(launchAsks(claude, { ...DEFAULT_SETTINGS, askBeforeRun: false }).confirm).toBe(false);
    expect(launchAsks(claude, { ...DEFAULT_SETTINGS, askBeforeRun: true }).confirm).toBe(true);
  });
  it("Face ID follows Run's own rule: lock, auth before run and Ask before running all on", () => {
    expect(launchAsks(claude, { ...lockOn, askBeforeRun: true }).auth).toBe(true);
    expect(launchAsks(claude, { ...lockOn, askBeforeRun: false }).auth).toBe(false);
    expect(launchAsks(claude, { ...DEFAULT_SETTINGS, askBeforeRun: true }).auth).toBe(false);
  });
});

describe("run once the new shell is ready", () => {
  it("tries again while the computer says busy, then succeeds", async () => {
    const run = vi.fn<() => Promise<ActionResult>>().mockResolvedValueOnce(busy).mockResolvedValueOnce(busy).mockResolvedValue({ ok: true });
    const wait = vi.fn(() => Promise.resolve());
    expect(await runWhenReady(run, wait, () => true)).toEqual({ ok: true });
    expect(run).toHaveBeenCalledTimes(3);
    expect(wait).toHaveBeenCalledTimes(2);
  });
  it("gives up after READY_TRIES", async () => {
    const run = vi.fn(() => Promise.resolve(busy));
    expect(await runWhenReady(run, () => Promise.resolve(), () => true)).toEqual(busy);
    expect(run).toHaveBeenCalledTimes(READY_TRIES);
  });
  it("does not retry another refusal", async () => {
    const run = vi.fn(() => Promise.resolve<ActionResult>({ ok: false, code: "forbidden", message: "view only" }));
    await runWhenReady(run, () => Promise.resolve(), () => true);
    expect(run).toHaveBeenCalledTimes(1);
  });
  it("stops once the sheet is gone", async () => {
    const run = vi.fn(() => Promise.resolve(busy));
    let alive = true;
    await runWhenReady(run, () => { alive = false; return Promise.resolve(); }, () => alive);
    expect(run).toHaveBeenCalledTimes(1);
  });
});

describe("open, then run", () => {
  const deps = (open: Awaited<ReturnType<Parameters<typeof launchProgram>[1]["open"]>>, run: ActionResult = { ok: true }) => {
    const runFn = vi.fn((_pane: number, _command: string) => Promise.resolve(run));
    return { runFn, d: { open: () => Promise.resolve(open), run: runFn, wait: () => Promise.resolve(), current: () => true } };
  };
  it("a shell is just the open", async () => {
    const { d, runFn } = deps({ ok: true, pane: 7 });
    expect(await launchProgram(shell, d)).toEqual({ ok: true, pane: 7 });
    expect(runFn).not.toHaveBeenCalled();
  });
  it("a program is typed into the new pane by its name", async () => {
    const { d, runFn } = deps({ ok: true, pane: 7 });
    expect(await launchProgram(claude, d)).toEqual({ ok: true, pane: 7 });
    expect(runFn).toHaveBeenCalledExactlyOnceWith(7, "claude");
  });
  it("types nothing when the open failed or was cancelled", async () => {
    const failed = deps({ ok: false, message: "no" });
    expect(await launchProgram(claude, failed.d)).toEqual({ ok: false, message: "no" });
    const cancelled = deps({ ok: false, message: null });
    expect(await launchProgram(claude, cancelled.d)).toEqual({ ok: false, message: null });
    expect(failed.runFn).not.toHaveBeenCalled();
    expect(cancelled.runFn).not.toHaveBeenCalled();
  });
  it("does not guess the pane when the computer named none", async () => {
    const { d, runFn } = deps({ ok: true, pane: undefined });
    const r = await launchProgram(claude, d);
    expect(r.ok).toBe(false);
    expect(runFn).not.toHaveBeenCalled();
  });
  it("keeps the opened pane when the program does not start, and says why", async () => {
    const { d } = deps({ ok: true, pane: 7 }, { ok: false, code: "forbidden", message: "view only" });
    const r = await launchProgram(claude, d);
    expect(r).toMatchObject({ ok: false, opened: 7 });
    expect((r as { message: string }).message).toContain("claude did not start");
    expect((r as { message: string }).message).toContain("view only");
  });
});

describe("launchVisit", () => {
  it("a tab's visit outlasts the active pane changing; a pane's is its pane", () => {
    expect(launchVisit("h", "tab", 3)).toBe(launchVisit("h", "tab", 4));
    expect(launchVisit("h", "pane", 3)).not.toBe(launchVisit("h", "pane", 4));
  });
});
