import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { AVATAR_COLORS, avatarFor } from "../src/avatar";
import { CLAUDE_COLOR, programMark } from "../src/paneView";
import { avatarInk } from "../src/ui/palette";
import { stubColors } from "./helpers/themeStub";
import { findAll, flat, load, render, textOf, uiDeps, type Node } from "./helpers/render";

describe("avatarFor", () => {
  it("gives a folder's panes one colour, wherever the path starts", () => {
    const a = avatarFor("/Users/me/zzz/rift", "tab", null);
    expect(avatarFor("~/zzz/rift", "other", "cargo")).toEqual(a);
    expect(AVATAR_COLORS).toContain(a.color);
  });

  it("spreads folders over the colours", () => {
    const names = ["rift", "flow", "harbor", "mobile", "quill", "ember", "orbit", "shift"];
    const used = new Set(names.map((n) => avatarFor(`~/${n}`, n, null).color));
    expect(used.size).toBeGreaterThan(3);
  });

  it("falls back to the title without a folder", () => {
    expect(avatarFor(null, "Tab 3", null)).toEqual(avatarFor(undefined, "Tab 3", null));
  });

  it("marks Claude Code with its logo", () => {
    expect(avatarFor("~/flow", "flow", "claude").logo).toBe("claude");
    expect(avatarFor("~/flow", "flow", "codex").logo).toBeNull();
  });
});

// The agents' logos (Lobe Icons, rendered to PNG in assets/agents) in every place a mark was
// drawn: the tiles, the round avatar and the pane picker; `>_` stays text for a shell.
describe("agent logos", () => {
  const root = new URL("../", import.meta.url);
  const src = (p: string) => readFileSync(new URL(`src/${p}`, root), "utf8");
  const deps = (extra: Record<string, unknown> = {}) =>
    uiDeps("ios", {
      "@/paneView": { programMark, CLAUDE_COLOR },
      "./palette": { avatarInk },
      "./AgentLogo": { AgentLogo: "AgentLogo" },
      ...extra,
    });
  const { Tile, Avatar } = load("ui/Avatar.tsx", deps()) as { Tile: (p: object) => unknown; Avatar: (p: object) => unknown };
  const tile = (p: object) => render({ type: Tile, props: { children: [], ...p } }) as Node;
  const logos = (n: Node) => findAll(n, (x) => x.type === "AgentLogo").map((x) => [x.props.logo, x.props.size]);

  it("names each agent's logo, the prompt mark only for a shell", () => {
    expect(["claude", "codex", "opencode"].map((p) => programMark(p).logo)).toEqual(["claude", "codex", "opencode"]);
    for (const p of ["claude", "codex", "opencode"]) expect(programMark(p).mark).toBeNull();
    for (const p of ["vim", "npm", null]) expect(programMark(p)).toEqual({ mark: ">_", logo: null, claude: false });
  });

  it("draws a list tile 36 pt (radius 10) with a 20 pt logo, Claude Code's on its orange tint", () => {
    const claude = tile({ agent: true, program: "claude" });
    expect(flat(claude.props.style)).toMatchObject({ width: 36, height: 36, borderRadius: 10, backgroundColor: `${CLAUDE_COLOR}1f` });
    expect(logos(claude)).toEqual([["claude", 20]]);
    expect(logos(tile({ agent: true, program: "codex" }))).toEqual([["codex", 20]]);
    // an agent the app has no logo for keeps Claude Code's
    expect(logos(tile({ agent: true, program: null }))).toEqual([["claude", 20]]);
    const shell = tile({ agent: false, program: null });
    expect(logos(shell)).toEqual([]);
    expect(textOf(shell)).toBe(">_");
    expect(flat(shell.props.style).backgroundColor).toBe(stubColors.tints.accentBadge);
    const prompt = findAll(shell, (n) => n.type === "Text")[0]!;
    expect(flat(prompt.props.style)).toMatchObject({ fontFamily: "monoMedium", fontSize: 12, color: stubColors.accent });
  });

  it("draws a sheet's tile 28 pt on the plain grey with a 16 pt logo", () => {
    const sheet = tile({ agent: true, program: "opencode", size: 28, radius: 8, logoSize: 16, plain: true });
    expect(flat(sheet.props.style)).toMatchObject({ width: 28, height: 28, borderRadius: 8, backgroundColor: stubColors.faint });
    expect(logos(sheet)).toEqual([["opencode", 16]]);
    expect(src("ui/NewSheet.tsx")).toContain("size={28} radius={8} logoSize={16} plain");
  });

  it("draws Claude Code's logo in the round avatar", () => {
    const avatar = render({ type: Avatar, props: { avatar: { color: "#19aad8", logo: "claude" }, children: [] } }) as Node;
    expect(logos(avatar)).toEqual([["claude", 20]]);
    expect(textOf(render({ type: Avatar, props: { avatar: { color: "#19aad8", logo: null }, children: [] } }))).toBe(">_");
  });

  it("ships each logo at 1x, 2x and 3x for 16 and 20 pt, with the MIT licence beside them", () => {
    for (const name of ["claude", "codex", "opencode"])
      for (const pt of [16, 20])
        for (const [suffix, scale] of [["", 1], ["@2x", 2], ["@3x", 3]] as const) {
          const png = readFileSync(new URL(`assets/agents/${name}-${pt}${suffix}.png`, root));
          // the PNG header's width and height
          expect([png.readUInt32BE(16), png.readUInt32BE(20)], `${name}-${pt}${suffix}`).toEqual([pt * scale, pt * scale]);
        }
    const licence = readFileSync(new URL("assets/agents/LICENSE", root), "utf8");
    expect(licence).toContain("MIT License");
    expect(licence).toContain("Copyright (c) 2023 LobeHub");
  });

  it("leaves no text mark (✳ ◆ ■) for an agent anywhere in the app", () => {
    const files = ["ui/Avatar.tsx", "ui/NewSheet.tsx", "app/panes.tsx", "app/index.tsx", "app/host/[key]/pane/[id].tsx", "paneView.ts", "avatar.ts"];
    for (const f of files) expect(src(f), f).not.toMatch(/\\u2733|\\u25C6|\\u25A0|[✳◆■]/);
    expect(src("app/host/[key]/pane/[id].tsx")).toContain("<AgentLogo logo={mark.logo} size={16} />");
    // OpenCode's one-colour mark takes the accent; the others keep their own colours
    expect(src("ui/AgentLogo.tsx")).toContain('logo === "opencode" ? colors.accent : undefined');
  });
});
