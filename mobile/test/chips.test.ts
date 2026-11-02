import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  NO_COMPLETIONS,
  agoText,
  branchRows,
  branchText,
  chipCommand,
  chipRowMode,
  chipsSpoken,
  completionRows,
  completionSource,
  diffChipText,
  dirRows,
  promptChips,
  shellQuote,
  splitPath,
  splitTyped,
  tildePath,
} from "../src/chips";
import { decodeServer, type Block, type TopologyMsg } from "../src/protocol/types";
import { screenProgram, showsKeyRow, showsTerminal } from "../src/terminal";

const topology = decodeServer(
  JSON.parse(readFileSync(fileURLToPath(new URL("../../crates/rift-remote/fixtures/server/topology.json", import.meta.url)), "utf8")),
) as TopologyMsg;
const [pane12, pane13] = topology.windows[0]!.tabs[0]!.panes as [TopologyMsg["windows"][0]["tabs"][0]["panes"][0], TopologyMsg["windows"][0]["tabs"][0]["panes"][0]];

const block = (extra: Partial<Block>): Block => ({
  id: 1,
  command: "ls",
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
const state = (blocks: Block[]) => ({ blocks, seq: 1, hasOlder: false, oldest: null }) as never;

describe("prompt chips (the shared topology fixture)", () => {
  it("folder, branch with ahead/behind and the dirty dot, changed files", () => {
    const c = promptChips(pane12, null, true);
    expect(c).toEqual({
      cwd: "~/rift",
      branch: { name: "main", ahead: 1, behind: 0, dirty: true },
      diff: { files: 3, added: 42, removed: 7 },
    });
    expect(branchText(c.branch!)).toBe("main ↑1");
    expect(chipsSpoken(c)).toBe("In ~/rift; branch main, 1 ahead, uncommitted changes; 3 files changed, 42 lines added, 7 removed");
  });

  it("a pane outside a repository has no git chips; one with no known folder no folder chip", () => {
    expect(promptChips(pane13, null, true)).toEqual({ cwd: null, branch: null, diff: null });
  });

  it("a clean tree has no changed-files chip; behind shows ↓", () => {
    const c = promptChips({ ...pane12, git: { ...pane12.git!, dirty: false, files: 0, added: 0, removed: 0, ahead: 0, behind: 2 } }, null, true);
    expect(c.diff).toBeNull();
    expect(branchText(c.branch!)).toBe("main ↓2");
  });

  it("an older gateway (no git feature): the branch from the newest block, no ahead/behind, no diff", () => {
    const blocks = [block({ id: 1, branch: "old" }), block({ id: 2, branch: "feat/x", cwd: "/home/jo/w" })];
    const c = promptChips({ ...pane12, git: undefined }, state(blocks), false);
    expect(c).toEqual({ cwd: "~/rift", branch: { name: "feat/x", ahead: 0, behind: 0, dirty: false }, diff: null });
    // git data a gateway without the feature sent anyway is not trusted
    expect(promptChips(pane12, null, false).diff).toBeNull();
  });

  it("shortens home folders only", () => {
    expect(tildePath("/Users/me")).toBe("~");
    expect(tildePath("/home/jo/src/rift")).toBe("~/src/rift");
    expect(tildePath("/root/x")).toBe("~/x");
    expect(tildePath("/Users")).toBe("/Users");
    expect(tildePath("/opt/Users/me")).toBe("/opt/Users/me");
  });

  it("decodes git only when well formed", () => {
    const bad = JSON.parse(JSON.stringify(topology)) as { windows: { tabs: { panes: { git: unknown }[] }[] }[] };
    for (const git of [null, { branch: "main" }, { ...pane12.git, ahead: -1 }, { ...pane12.git, dirty: "yes" }]) {
      bad.windows[0]!.tabs[0]!.panes[0]!.git = git;
      expect(decodeServer(bad)).toBeNull();
    }
  });
});

describe("how long ago", () => {
  it("says how long ago", () => {
    expect(agoText(30_000)).toBe("just now");
    expect(agoText(5 * 60_000)).toBe("5 min ago");
    expect(agoText(2 * 3600_000 + 1)).toBe("2 h ago");
    expect(agoText(26 * 3600_000)).toBe("1 day ago");
    expect(agoText(72 * 3600_000)).toBe("3 days ago");
  });
});

describe("the pane picks its view by itself", () => {
  it("shows the screen only while a full-screen program runs, on a gateway with screen", () => {
    expect(showsTerminal({ alt: true }, true)).toBe(true);
    expect(showsTerminal({ alt: false }, true)).toBe(false);
    expect(showsTerminal(null, true)).toBe(false);
    expect(showsTerminal({ alt: true }, false)).toBe(false);
  });

  it("names the program in the banner", () => {
    expect(screenProgram("htop -d 5", "htop")).toBe("htop");
    expect(screenProgram("/usr/bin/vim notes.md", null)).toBe("vim");
    expect(screenProgram(null, "less")).toBe("less");
    expect(screenProgram("  ", null)).toBe("A program");
  });
});

describe("chip menu commands (as the desktop's chip_menu.rs builds them)", () => {
  it("quotes like the desktop's shell_quote", () => {
    expect(shellQuote("src")).toBe("src");
    expect(shellQuote("./my dir")).toBe("'./my dir'");
    expect(shellQuote("it's")).toBe("'it'\\''s'");
    expect(shellQuote("$(rm -rf ~)")).toBe("'$(rm -rf ~)'");
    expect(shellQuote("a;b")).toBe("'a;b'");
    expect(shellQuote("")).toBe("''");
  });

  it("cd, checkout and create, with untrusted names quoted", () => {
    expect(chipCommand({ kind: "cd", name: ".." })).toBe("cd ..");
    expect(chipCommand({ kind: "cd", name: "-odd" })).toBe("cd ./-odd");
    expect(chipCommand({ kind: "cd", name: "my dir" })).toBe("cd './my dir'");
    expect(chipCommand({ kind: "cd", name: "x`reboot`" })).toBe("cd './x`reboot`'");
    expect(chipCommand({ kind: "checkout", branch: "feat/chips" })).toBe("git checkout feat/chips");
    expect(chipCommand({ kind: "checkout", branch: "a$(b)" })).toBe("git checkout 'a$(b)'");
    expect(chipCommand({ kind: "create", branch: "new-one" })).toBe("git checkout -b new-one");
  });

  it("filters the menus; offers to create a branch only for a plausible new name", () => {
    expect(dirRows(true, ["crates", "docs"], "")).toEqual(["..", "crates", "docs"]);
    expect(dirRows(true, ["crates", "Docs"], "do")).toEqual(["Docs"]);
    expect(dirRows(false, ["a"], "")).toEqual(["a"]);
    expect(branchRows(["main", "feat"], "")).toEqual([{ name: "main", create: false }, { name: "feat", create: false }]);
    expect(branchRows(["main", "feat"], "fe")).toEqual([{ name: "feat", create: false }, { name: "fe", create: true }]);
    expect(branchRows(["main"], "main")).toEqual([{ name: "main", create: false }]);
    for (const bad of ["two words", "-x", "a:b", "x~1"]) expect(branchRows([], bad)).toEqual([]);
    expect(splitPath("crates/rift-pty/src/session.rs")).toEqual({ name: "session.rs", folder: "crates/rift-pty/src" });
    expect(splitPath("README.md")).toEqual({ name: "README.md", folder: "" });
  });

  it("decodes the chip answers only within their bounds", () => {
    const ok = { t: "dirs", id: "m", pane: 1, cwd: "/r", parent: true, dirs: ["a"], more: false };
    expect(decodeServer(ok)).toEqual(ok);
    expect(decodeServer({ ...ok, dirs: Array.from({ length: 501 }, (_, i) => `d${i}`) })).toBeNull();
    expect(decodeServer({ ...ok, dirs: ["a\nrm -rf ~"] })).toBeNull();
    expect(decodeServer({ ...ok, dirs: ["x".repeat(1025)] })).toBeNull();
    const diff = { t: "diff", id: "d", pane: 1, path: "a", staged: false, binary: false, truncated: false, lines: [["+", "x"]] };
    expect(decodeServer(diff)).toEqual(diff);
    expect(decodeServer({ ...diff, lines: [["!", "x"]] })).toBeNull();
    expect(decodeServer({ ...diff, lines: [["+", "y".repeat(1001)]] })).toBeNull();
    const changes = { t: "changes", id: "c", pane: 1, branch: "main", more: false, files: [{ path: "a", status: "M", added: 1, removed: 0, staged: false }] };
    expect(decodeServer(changes)).toEqual(changes);
    expect(decodeServer({ ...changes, files: [{ ...changes.files[0], status: "MM" }] })).toBeNull();
  });
});

describe("the key row", () => {
  it("shows under a full-screen program whatever the setting; elsewhere as the setting says", () => {
    expect(showsKeyRow(false, true)).toBe(true);
    expect(showsKeyRow(true, true)).toBe(true);
    expect(showsKeyRow(true, false)).toBe(true);
    expect(showsKeyRow(false, false)).toBe(false);
  });
});

describe("the chip row: prompt chips while the field is empty, completions while typing", () => {
  it("switches on whether anything is typed", () => {
    expect(chipRowMode("")).toBe("chips");
    expect(chipRowMode("g")).toBe("completions");
    expect(chipRowMode(" ")).toBe("completions");
    // and back as the field empties
    expect(["", "git", ""].map(chipRowMode)).toEqual(["chips", "completions", "chips"]);
  });

  it("the changed-files chip: the count with +adds −dels", () => {
    expect(diffChipText({ files: 3, added: 42, removed: 7 })).toBe("3 files +42 −7");
    expect(diffChipText({ files: 1, added: 1, removed: 0 })).toBe("1 file +1 −0");
    expect(diffChipText({ files: 2, added: 0, removed: 0 })).toBe("2 files");
  });

  it("typed part bright, remainder dim; a completion that rewrites the line shares only its start", () => {
    expect(splitTyped("git ch", "git checkout main")).toEqual({ typed: "git ch", rest: "eckout main" });
    expect(splitTyped("gti", "git status")).toEqual({ typed: "g", rest: "it status" });
    expect(splitTyped("x", "git status")).toEqual({ typed: "", rest: "git status" });
  });

  it("source labels: history, branch, folder", () => {
    expect(completionSource({ text: "cd src", kind: "path" })).toBe("folder");
    expect(completionSource({ text: "git checkout main", kind: "value" })).toBe("branch");
    expect(completionSource({ text: "make x", kind: "value" })).toBe("value");
    expect(completionSource({ text: "git status", kind: "history" })).toBe("history");
    expect(completionSource({ text: "git", kind: "command" })).toBe("command");
  });

  it("the rows: the desktop's items, its history, then this pane's recent commands; each line once, never the draft", () => {
    const rows = completionRows(
      "git c",
      [
        { text: "git checkout", kind: "subcommand" },
        { text: "git c", kind: "command" },
      ],
      [{ text: "git commit -m x" }, { text: "git checkout" }],
      ["git clone y", "ls", "git commit -m x"],
      6,
    );
    expect(rows.map((r) => [r.text, r.typed, r.source])).toEqual([
      ["git checkout", "git c", "subcommand"],
      ["git commit -m x", "git c", "history"],
      ["git clone y", "git c", "history"],
    ]);
    expect(rows[0]!.rest).toBe("heckout");
    expect(completionRows("g", [], [], ["g1", "g2", "g3"], 2)).toHaveLength(2);
  });

  it("no match: no rows, and the row says what Return does", () => {
    expect(completionRows("zzz", [], [], ["ls", "git status"], 6)).toEqual([]);
    expect(NO_COMPLETIONS).toBe("No completions · Run runs it as typed");
  });
});
