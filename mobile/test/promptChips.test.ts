import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isMenuCommand, chipCommand } from "../src/chips";
import {
  CHIP_GAP,
  DOT_GAP,
  MIN_CUT,
  DOT_SIZE,
  chipItems,
  compact,
  containerRows,
  dockerTitle,
  elapsedBody,
  fitChips,
  folderName,
  isPrUrl,
  moreSpoken,
  moreWidth,
  spokenValue,
  toneColor,
  versionRows,
  type ChipFeatures,
} from "../src/promptChips";
import { decodeServer, type ComposeContainer, type Pane, type PaneChip, type RuntimeVersion, type TopologyMsg } from "../src/protocol/types";

const ON: ChipFeatures = { git: true, menus: true, promptChips: true };

const fixture = (name: string) =>
  JSON.parse(readFileSync(fileURLToPath(new URL(`../../crates/rift-remote/fixtures/server/${name}`, import.meta.url)), "utf8")) as unknown;

const pane = (chips?: PaneChip[], extra: Partial<Pane> = {}): Pane => ({
  id: 1,
  active: true,
  cwd: "/Users/me/xxx/flow",
  job: null,
  last_command: null,
  last_exit: null,
  ...(chips ? { chips } : {}),
  ...extra,
});

const wire = (kind: string, icon: string, full: [string, string][], short?: [string, string][], attention?: boolean): PaneChip => ({
  kind,
  icon,
  full: full.map(([text, tone]) => ({ text, tone })),
  short: (short ?? full).map(([text, tone]) => ({ text, tone })),
  ...(attention ? { attention } : {}),
});

describe("the desktop's chips on the phone", () => {
  const chips = [
    wire("cwd", "dim", [["~/xxx/flow", "bright"]], [["flow", "bright"]]),
    wire("git_branch", "success", [["release/3.16.0", "ui"], ["↑2 •", "muted"]], [["release/3.16.0", "ui"], ["•", "muted"]]),
    wire("docker", "accent", [["flow", "bright"], ["3/4", "success"]], [["3/4", "success"]], true),
    wire("user", "accent", [["me", "bright"]]),
    wire("elapsed", "dim", [["6h 33m", "ui"]]),
  ];

  it("keeps the desktop's order, kinds and attention", () => {
    const items = chipItems(pane(chips), null, ON);
    expect(items.map((c) => c.id)).toEqual(["cwd", "git_branch", "docker", "user", "elapsed"]);
    expect(items.map((c) => c.attention)).toEqual([false, false, true, false, false]);
    expect(items.map((c) => c.name)).toEqual(["Folder", "Branch", "Docker", "User", "Time on tab"]);
  });

  it("draws the short value, with the full one for the sheet", () => {
    const [cwd, branch] = chipItems(pane(chips), null, ON);
    expect(cwd!.short.map((p) => p.text)).toEqual(["flow"]);
    expect(cwd!.full.map((p) => p.text)).toEqual(["~/xxx/flow"]);
    expect(branch!.short.map((p) => p.text)).toEqual(["release/3.16.0", "•"]);
  });

  it("styles each part by its tone in the app's palette (the desktop's dim / muted are this palette's textMuted / textDim)", () => {
    const [, branch, docker] = chipItems(pane(chips), null, ON);
    expect(branch!.iconColor).toBe("success");
    expect(branch!.short.map((p) => p.color)).toEqual(["text", "textDim"]);
    expect(docker!.iconColor).toBe("accent");
    expect(docker!.short[0]!.color).toBe("success");
    expect(toneColor("dim")).toBe("textMuted");
    expect(toneColor("muted")).toBe("textDim");
    expect(toneColor("magenta")).toBe("magenta");
    expect(toneColor("indigo")).toBe("indigo");
    // a tone from a newer desktop reads as ordinary text
    expect(toneColor("chartreuse")).toBe("text");
  });

  it("opens a menu for the chips that have one, and only shows the rest", () => {
    const opens = Object.fromEntries(chipItems(pane(chips), null, ON).map((c) => [c.id, c.opens]));
    expect(opens).toEqual({ cwd: "dirs", git_branch: "branches", docker: "docker", user: null, elapsed: "elapsed" });
    // the folder and branch menus need the `chips` feature
    const noMenus = Object.fromEntries(chipItems(pane(chips), null, { ...ON, menus: false }).map((c) => [c.id, c.opens]));
    expect(noMenus.cwd).toBeNull();
    expect(noMenus.docker).toBe("docker");
  });

  it("spells the glyphs out for a screen reader", () => {
    const items = chipItems(pane(chips), null, ON);
    expect(items[1]!.spoken).toBe("Branch: release/3.16.0, 2 ahead, uncommitted changes. Opens the branches");
    expect(items[2]!.spoken).toBe("Docker: flow, 3 of 4 containers running, needs attention. Opens the containers");
    expect(items[3]!.spoken).toBe("User: me");
    const diff = chipItems(
      pane([wire("git_diff", "dim", [["102", "ui"], ["•", "muted"], ["+2086", "success"], ["−970", "error"]])]),
      null,
      ON,
    )[0]!;
    expect(diff.spoken).toBe("Changes: 102 files changed, 2086 lines added, 970 removed. Opens the changes");
    expect(spokenValue("git_diff", [{ text: "1", color: "text" }, { text: "•", color: "text" }, { text: "+3", color: "text" }, { text: "−0", color: "text" }])).toBe(
      "1 file changed, 3 lines added, 0 removed",
    );
  });

  it("shows none when the desktop draws none (no fallback to the folder and branch)", () => {
    expect(chipItems(pane([], { git: { branch: "main", ahead: 0, behind: 0, dirty: true, files: 2, added: 3, removed: 1 } }), null, ON)).toEqual([]);
  });

  it("a chip of a kind this app does not know still shows", () => {
    const [c] = chipItems(pane([wire("weather", "dim", [["rain", "bright"]])]), null, ON);
    expect(c).toMatchObject({ id: "weather", name: "weather", icon: "terminal", opens: null });
  });

  it("a chip the desktop sent without a short value shows its full one", () => {
    const [c] = chipItems(pane([{ ...wire("user", "accent", [["me", "bright"]]), short: [] }]), null, ON);
    expect(c!.short.map((p) => p.text)).toEqual(["me"]);
  });
});

describe("a desktop without prompt_chips", () => {
  const git = { branch: "release/3.16.0", ahead: 2, behind: 0, dirty: true, files: 102, added: 2086, removed: 970 };
  const old: ChipFeatures = { git: true, menus: true, promptChips: false };

  it("falls back to the folder, the branch and the changes, styled like the desktop's chips", () => {
    const items = chipItems(pane(undefined, { git }), null, old);
    expect(items.map((c) => c.id)).toEqual(["cwd", "git_branch", "git_diff"]);
    expect(items.map((c) => c.short.map((p) => p.text).join(" "))).toEqual(["flow", "release/3.16.0 •", "102 +2.1k −970"]);
    expect(items[0]!.full[0]!.text).toBe("~/xxx/flow");
    expect(items[1]!.full.map((p) => p.text)).toEqual(["release/3.16.0", "↑2 •"]);
    expect(items[2]!.short[1]!.color).toBe("success");
    expect(items[2]!.short[2]!.color).toBe("error");
    expect(items.map((c) => c.opens)).toEqual(["dirs", "branches", "changes"]);
  });

  it("ignores chips a desktop sent without advertising the feature", () => {
    const items = chipItems(pane([wire("docker", "accent", [["flow", "bright"]])], { git }), null, old);
    expect(items.map((c) => c.id)).toEqual(["cwd", "git_branch", "git_diff"]);
  });

  it("without git either, the folder alone", () => {
    expect(chipItems(pane(), null, { git: false, menus: false, promptChips: false }).map((c) => [c.id, c.opens])).toEqual([["cwd", null]]);
  });

  it("a clean tree has no changes chip", () => {
    const clean = { ...git, dirty: false, files: 0, added: 0, removed: 0 };
    expect(chipItems(pane(undefined, { git: clean }), null, old).map((c) => c.id)).toEqual(["cwd", "git_branch"]);
  });
});

describe("short values", () => {
  it("compacts line counts", () => {
    expect([970, 999, 1000, 2086, 9999, 10_000, 12_400].map(compact)).toEqual(["970", "999", "1.0k", "2.1k", "10.0k", "10k", "12k"]);
  });
  it("names a folder by its last component", () => {
    expect([folderName("~/xxx/flow"), folderName("~"), folderName("/"), folderName("/usr/")]).toEqual(["flow", "~", "/", "usr"]);
  });
});

describe("fitting the row", () => {
  const moreBase = 40;
  const shownWidth = (w: number[], count: number) => w.slice(0, count).reduce((a, b) => a + b, 0) + CHIP_GAP * Math.max(count - 1, 0);

  it("shows every chip that fits, with no +N", () => {
    expect(fitChips([80, 90, 70], [false, false, false], 80 + 90 + 70 + 2 * CHIP_GAP, moreBase)).toEqual({ count: 3, clamp: null });
  });

  it("folds the rest into +N and leaves room for it", () => {
    const w = [80, 90, 70, 60];
    const avail = 280;
    const fit = fitChips(w, [false, false, false, false], avail, moreBase);
    expect(fit.clamp).toBeNull();
    expect(fit.count).toBe(2);
    // chips + gaps + the +N chip fit the row
    expect(shownWidth(w, fit.count) + CHIP_GAP + moreWidth(moreBase, 0)).toBeLessThanOrEqual(avail);
    // and one more would not
    expect(shownWidth(w, fit.count + 1) + CHIP_GAP + moreWidth(moreBase, 0)).toBeGreaterThan(avail);
  });

  it("makes room for a dot per folded chip that needs attention", () => {
    const w = [100, 100, 60];
    // without attention two chips + a bare +N just fit; with two dots on the folded ones they do not
    const avail = 100 + CHIP_GAP + 100 + CHIP_GAP + moreWidth(moreBase, 0);
    expect(fitChips(w, [false, false, false], avail, moreBase).count).toBe(2);
    const dotted = fitChips([100, 100, 60, 60], [false, false, true, true], avail, moreBase);
    // the second chip no longer fits whole; it is cut to what the two dots leave (146 - 6 - (40 + 20))
    expect(dotted).toEqual({ count: 2, clamp: 80 });
    expect(moreWidth(moreBase, 2) - moreWidth(moreBase, 0)).toBe(2 * (DOT_SIZE + DOT_GAP));
  });

  it("cuts a first chip wider than the row instead of dropping it, and never exceeds the row", () => {
    const fit = fitChips([500, 60], [false, true], 340, moreBase);
    expect(fit.count).toBe(1);
    expect(fit.clamp).not.toBeNull();
    expect(fit.clamp! + CHIP_GAP + moreWidth(moreBase, 1)).toBeLessThanOrEqual(340);
    // alone, it needs no +N: it takes the whole row
    expect(fitChips([500], [false], 340, moreBase)).toEqual({ count: 1, clamp: 340 });
  });

  it("uses the room left by cutting the next chip with … when that is at least MIN_CUT, else folds it", () => {
    // two chips whole (80 + 6 + 90), then 70 more would need +N: 176 + 6 + 6 + 40 = 228 used, room left
    const w = [80, 90, 200, 60];
    const roomFor = (avail: number) => avail - (80 + CHIP_GAP + 90) - CHIP_GAP - CHIP_GAP - moreWidth(moreBase, 0);
    const wide = 228 + MIN_CUT; // leaves exactly MIN_CUT for the third chip
    expect(roomFor(wide)).toBe(MIN_CUT);
    expect(fitChips(w, [false, false, false, false], wide, moreBase)).toEqual({ count: 3, clamp: MIN_CUT });
    // one point less and it folds
    expect(fitChips(w, [false, false, false, false], wide - 1, moreBase)).toEqual({ count: 2, clamp: null });
    // a dot on a chip behind the cut one takes room from the cut chip
    expect(fitChips(w, [false, false, false, true], wide, moreBase)).toEqual({ count: 2, clamp: null });
  });

  it("has nothing to fit when there are no chips", () => {
    expect(fitChips([], [], 340, moreBase)).toEqual({ count: 0, clamp: null });
  });

  it("never overflows, whatever the widths (200 random rows)", () => {
    let seed = 7;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
    for (let n = 0; n < 200; n++) {
      const count = 1 + Math.floor(rnd() * 12);
      const w = Array.from({ length: count }, () => 40 + Math.floor(rnd() * 160));
      const att = w.map(() => rnd() < 0.3);
      const avail = 200 + Math.floor(rnd() * 250);
      const fit = fitChips(w, att, avail, 36);
      // the shown chips (the last one cut to its clamp), then +N when any are folded, fit the row
      const shown = w.slice(0, fit.count);
      if (fit.clamp !== null) shown[shown.length - 1] = Math.min(shown[shown.length - 1]!, fit.clamp);
      const hidden = count - fit.count;
      const dots = att.slice(fit.count).filter(Boolean).length;
      const used = shown.reduce((a, b) => a + b, 0) + CHIP_GAP * Math.max(shown.length - 1, 0) + (hidden > 0 ? CHIP_GAP + moreWidth(36, dots) : 0);
      // a lone chip squeezed below the floor is the one case allowed to overflow (never, in practice: rows are wider)
      expect(used, JSON.stringify({ w, att, avail, fit })).toBeLessThanOrEqual(avail);
    }
  });

  it("speaks the folded chips and which need attention", () => {
    const items = chipItems(
      pane([wire("python", "warning", [["3.12.4", "bright"]]), wire("docker", "accent", [["flow", "bright"], ["3/4", "success"]], undefined, true)]),
      null,
      ON,
    );
    expect(moreSpoken(items)).toBe("2 more: Python, Docker (needs attention). Shows every chip");
  });
});

describe("the menus of the new chips", () => {
  const v = (version: string, command: string, current = false): RuntimeVersion => ({ version, source: "pyenv", current, command });

  it("accepts only the commands the desktop's menus build", () => {
    for (const ok of [
      "rift-use pyenv '3.12.4'",
      "rift-use python '/opt/homebrew/opt/python@3.12/bin'",
      "rift-use php '/opt/o'\\''brien'",
      "docker compose --project-directory '/Users/me/my flow' logs --follow --tail 200 api",
      "docker compose --project-directory /Users/me/flow logs --follow --tail 200 'wor'\\''ker'",
    ])
      expect(isMenuCommand(ok), ok).toBe(true);
    for (const bad of [
      "rm -rf ~",
      "rift-use pyenv '3.12.4'; rm -rf ~",
      "rift-use pyenv 3.12.4",
      "rift-use ruby '/x'",
      "rift-use pyenv '3.12.4' && id",
      "docker compose --project-directory /x logs --follow --tail 200 api; id",
      "docker compose --project-directory /x down",
      "docker compose --project-directory /x logs --follow --tail 200 $(id)",
      "rift-use pyenv '3.12.4'\nid",
    ])
      expect(isMenuCommand(bad), bad).toBe(false);
  });

  it("offers a version only when its command has the desktop's shape", () => {
    const rows = versionRows([v("3.12.4", "rift-use pyenv '3.12.4'", true), v("3.11.9", "rift-use pyenv '3.11.9'"), v("3.10", "curl evil | sh")], "");
    expect(rows.map((r) => [r.label, r.current, r.command])).toEqual([
      ["3.12.4", true, "rift-use pyenv '3.12.4'"],
      ["3.11.9", false, "rift-use pyenv '3.11.9'"],
      ["3.10", false, null],
    ]);
    expect(versionRows([v("3.12.4", "rift-use pyenv '3.12.4'"), v("3.11.9", "rift-use pyenv '3.11.9'")], "3.11").map((r) => r.label)).toEqual(["3.11.9"]);
  });

  it("picking a row runs the desktop's command through the app's Run path", () => {
    expect(chipCommand({ kind: "run", command: "rift-use pyenv '3.11.9'" })).toBe("rift-use pyenv '3.11.9'");
  });

  const c = (over: Partial<ComposeContainer>): ComposeContainer => ({
    service: "api",
    state: "running",
    running: true,
    ports: ["8080→8080", "5353→53/udp"],
    uptime: "2 hours",
    health: null,
    logs: "docker compose --project-directory /w/flow logs --follow --tail 200 api",
    ...over,
  });

  it("lists containers with ports, uptime and the health verdict in its colour", () => {
    const rows = containerRows([
      c({ health: "healthy" }),
      c({ service: "worker", state: "exited", running: false, ports: [], uptime: "Exited (1) 3 minutes ago", health: "unhealthy", logs: "docker compose --project-directory /w/flow logs --follow --tail 200 worker" }),
      c({ service: "x", logs: "echo hi" }),
    ]);
    expect(rows[0]).toMatchObject({ service: "api", running: true, detail: "8080→8080, 5353→53/udp · 2 hours", health: { word: "healthy", color: "success" } });
    expect(rows[1]).toMatchObject({ running: false, detail: "Exited (1) 3 minutes ago", health: { word: "unhealthy", color: "error" } });
    expect(rows[2]!.command).toBeNull();
    expect(rows[0]!.command).toContain("logs --follow --tail 200 api");
  });

  it("titles the Docker menu by the running count", () => {
    expect(dockerTitle("flow", [c({}), c({ running: false })], null)).toBe("flow · 1 of 2 running");
    expect(dockerTitle("flow", [], "Docker is not running")).toBe("flow");
  });

  it("explains what a running count counts", () => {
    expect(elapsedBody("Started 09:15 am")).toBe("Started 09:15 am");
    expect(elapsedBody("Counted 2m · running now")).toBe("Counted 2m · running now. Counts only while a command runs in this tab.");
  });

  it("opens a pull request only over https", () => {
    expect(isPrUrl("https://github.com/me/flow/pull/42")).toBe(true);
    for (const bad of ["http://github.com/x/1", "javascript:alert(1)", "file:///etc/passwd", "https://", "https://x.y/a b", "rift-remote://pair"])
      expect(isPrUrl(bad), bad).toBe(false);
  });
});

describe("the wire", () => {
  it("decodes the fixture's chips into the row", () => {
    const topology = decodeServer(fixture("topology-chips.json")) as TopologyMsg;
    const p = topology.windows[0]!.tabs[0]!.panes[0]!;
    const items = chipItems(p, null, ON);
    expect(items.map((c) => c.id)).toEqual(["cwd", "git_branch", "git_diff", "docker"]);
    expect(items[3]).toMatchObject({ attention: true, opens: "docker" });
    expect(items[2]!.short.map((x) => x.text)).toEqual(["102", "+2.1k", "−970"]);
  });

  const withChips = (chips: unknown) => ({ t: "topology", windows: [{ pid: 1, tabs: [{ index: 0, title: "t", active: true, panes: [{ id: 1, active: true, cwd: null, job: null, last_command: null, last_exit: null, chips }] }] }] });

  it("rejects chips outside the bounds or of the wrong shape", () => {
    const ok = { kind: "user", icon: "accent", full: [{ text: "me", tone: "bright" }], short: [{ text: "me", tone: "bright" }] };
    expect(decodeServer(withChips([ok]))).not.toBeNull();
    expect(decodeServer(withChips([{ ...ok, kind: "unknown_new_kind", icon: "unknown_tone" }]))).not.toBeNull();
    expect(decodeServer(withChips(Array.from({ length: 17 }, () => ok)))).toBeNull();
    expect(decodeServer(withChips([{ ...ok, full: Array.from({ length: 7 }, () => ({ text: "x", tone: "ui" })) }]))).toBeNull();
    expect(decodeServer(withChips([{ ...ok, full: [{ text: "x".repeat(201), tone: "ui" }] }]))).toBeNull();
    expect(decodeServer(withChips([{ ...ok, full: [{ text: "a\nb", tone: "ui" }] }]))).toBeNull();
    expect(decodeServer(withChips([{ ...ok, attention: "yes" }]))).toBeNull();
    expect(decodeServer(withChips("docker"))).toBeNull();
  });

  it("rejects a chip answer of the wrong shape", () => {
    const base = { t: "chip", id: "m", pane: 1, more: false };
    expect(decodeServer({ ...base, kind: "pr", number: 1, state: "open", url: "https://x.y/z" })).not.toBeNull();
    expect(decodeServer({ ...base, kind: "pr", number: 1, state: "weird", url: "https://x.y/z" })).toBeNull();
    expect(decodeServer({ ...base, kind: "docker", project: null, error: null, containers: [{ service: "a" }] })).toBeNull();
    expect(decodeServer({ ...base, kind: "python", current: null, versions: [{ version: "3", source: "s", current: false, command: "c\nd" }] })).toBeNull();
    expect(decodeServer({ ...base, kind: "nope" })).toBeNull();
  });
});
