import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { AVATAR_COLORS } from "../src/avatar";
import { BUILTIN_THEMES } from "../src/themes.data";
import {
  CAMERA_TEXT,
  MARK_CONTRAST,
  avatarInk,
  DARK_DEFAULT,
  LIGHT_DEFAULT,
  MIN_CONTRAST,
  TEXT_ON,
  TINTED_TEXT,
  contrast,
  lift,
  luminance,
  over,
  paletteFrom,
  type ColorName,
  type Ground,
} from "../src/ui/palette";

const dark = paletteFrom(DARK_DEFAULT);

describe("Dark Default is the palette the app always had", () => {
  it("keeps every colour and tint", () => {
    expect(dark).toMatchObject({
      bg: "#050505",
      chrome: "#000000",
      surface: "#1e1e1e",
      hairline: "#ffffff1a",
      textBright: "#ffffff",
      text: "#e6e6e6",
      textMuted: "#9b9b9b",
      textDim: "#828282",
      accent: "#19aad8",
      success: "#1ca05a",
      error: "#ff8272",
      warning: "#e5a01a",
      magenta: "#ff8ffd",
      indigo: "#8c9cea",
      fg: "#ffffff",
      onAccent: "#050505",
      borderStrong: "#ffffff24",
      glass: "#ffffff14",
      glassRim: "#ffffff1f",
      rim: "#ffffff3d",
      floating: "#1e1e1ef0",
      camera: "#000000",
      viewfinder: "#121212",
      faint: "#ffffff0f",
      switchOff: "#ffffff20",
      raised: "#262626",
      menu: "#2b2b2b",
      popover: "#282828",
      segmentOn: "#373737",
      askGround: "#201808",
      dark: true,
      name: "Dark Default",
    });
    expect(dark.tints).toEqual({
      accentBadge: "#19aad81a",
      successBadge: "#1ca05a14",
      warningBadge: "#e5a01a1f",
      errorBadge: "#ff827233",
      chipFill: "#ffffff0a",
      running: "#19aad814",
      selected: "#19aad80f",
      bellOn: "#19aad833",
      diffAdded: "#1ca05a1f",
      diffRemoved: "#ff827226",
      answerAdded: "#1ca05a1f",
      answerRemoved: "#ff82721f",
      match: "#ffd70055",
    });
  });
});

describe.each(BUILTIN_THEMES.map((t) => [t.name, t] as const))("text contrast in %s (WCAG AA, 4.5:1)", (_name, theme) => {
  const colors = paletteFrom(theme);
  const pairs = Object.entries(TEXT_ON).flatMap(([ground, fgs]) => fgs.map((fg) => [fg, ground as Ground] as const));

  it.each(pairs)("%s on %s", (fg, ground) => {
    expect(contrast(colors[fg], colors[ground])).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it.each(TINTED_TEXT.map((t) => [t.fg, t.tint, t.ground, t.under] as const))("%s on %s over %s (under %s)", (fg, tint, ground, under) => {
    const base = under ? over(colors.tints[under], colors[ground]) : colors[ground];
    expect(contrast(colors[fg], over(colors.tints[tint], base))).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it("is dark or light as its ground is", () => {
    expect(colors.dark).toBe(theme.dark);
    expect(colors.bg).toBe(`#${theme.bg}`);
  });
});

describe("contrast lifting", () => {
  it("leaves a colour that reads as it is", () => {
    expect(lift("#ffffff", ["#000000"], "#e6e6e6")).toBe("#ffffff");
  });

  it("moves a colour that falls short toward the text colour until it reads on every ground", () => {
    // Light Default's text_muted (#888888) is 3.5:1 on white
    const lifted = lift("#888888", ["#ffffff", "#f3f3f3"], "#292929");
    expect(lifted).not.toBe("#888888");
    expect(contrast(lifted, "#ffffff")).toBeGreaterThanOrEqual(MIN_CONTRAST);
    expect(contrast(lifted, "#f3f3f3")).toBeGreaterThanOrEqual(MIN_CONTRAST);
    // the least it takes: one step less still falls short
    expect(contrast(lifted, "#f3f3f3")).toBeLessThan(5.2);
  });

  it("goes on past the text colour to black or white when that is not enough", () => {
    // #888888 is only 1.4:1 on #666666: on toward white
    const lifted = lift("#777777", ["#666666"], "#888888");
    expect(contrast(lifted, "#666666")).toBeGreaterThanOrEqual(MIN_CONTRAST);
    expect(luminance(lifted)).toBeGreaterThan(luminance("#888888"));
  });

  it("lifts Light Default's muted text, only as far as it needs", () => {
    const light = paletteFrom(LIGHT_DEFAULT);
    expect(light.textDim).not.toBe("#888888");
    expect(light.bg).toBe("#ffffff");
    // tokens that already read stay the theme's
    expect(light.text).toBe("#292929");
    expect(light.textBright).toBe("#000000");
    // the accent reads on white (4.6:1) but not on the chrome's grey: it is darkened a little
    expect(contrast("#007dad", light.chrome)).toBeLessThan(MIN_CONTRAST);
    expect(contrast(light.accent, light.chrome)).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it("a malformed token takes Dark Default's", () => {
    const broken = paletteFrom({ ...DARK_DEFAULT, accent: "nope", ansi: ["x"] });
    expect(broken.accent).toBe(dark.accent);
    expect(broken.magenta).toBe(dark.magenta);
  });
});

describe("avatars", () => {
  it("keep their colour in Dark Default and show in every theme (3:1, a decorative mark)", () => {
    for (const c of AVATAR_COLORS) expect(avatarInk(c, dark)).toBe(c);
    for (const t of BUILTIN_THEMES) {
      const p = paletteFrom(t);
      for (const c of AVATAR_COLORS) {
        const ink = avatarInk(c, p);
        for (const under of [p.bg, p.surface])
          expect(contrast(ink, over(`${c}2e`, under)), `${t.name} ${c}`).toBeGreaterThanOrEqual(MARK_CONTRAST);
      }
    }
    // a pale yellow on Light Default is lifted
    expect(avatarInk("#fefdc2", paletteFrom(LIGHT_DEFAULT))).not.toBe("#fefdc2");
  });

  it("a coding agent's logo keeps its own colours (a brand mark is never recoloured), on its tile's tint", () => {
    // the logos are images (ui/AgentLogo.tsx): only OpenCode's one-colour mark takes a colour, the accent
    const logo = readFileSync(new URL("../src/ui/AgentLogo.tsx", import.meta.url), "utf8");
    expect(logo).toContain('logo === "opencode" ? colors.accent : undefined');
    expect(logo).not.toContain("avatarInk");
    const avatar = readFileSync(new URL("../src/ui/Avatar.tsx", import.meta.url), "utf8");
    expect(avatar).toContain('logo === "claude" ? `${CLAUDE_COLOR}1f`');
    const panes = readFileSync(new URL("../src/app/panes.tsx", import.meta.url), "utf8");
    expect(panes).toContain("lead={<Tile ");
    expect(panes).not.toContain("CLAUDE_COLOR");
  });
});

describe("other grounds", () => {
  it("camera pages read on the camera's black (Dark Default, whatever the theme)", () => {
    for (const fg of CAMERA_TEXT) expect(contrast(dark[fg], dark.camera), fg).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it("lays a tint over its ground", () => {
    expect(over("#ffffff80", "#000000")).toBe("#808080");
  });

  it("the Updated pill's text reads on its ground", () => {
    const view = readFileSync(new URL("../src/ui/TerminalView.tsx", import.meta.url), "utf8");
    const fg = /ageText: \{[^}]*color: colors\.(\w+)/.exec(view)![1] as ColorName;
    expect(/\bage: \{[^}]*backgroundColor: colors\.chrome/.test(view)).toBe(true);
    // a strip above the screen: floating, it covered the start of the program's first row
    expect(/\bage: \{[^}]*position: "absolute"/.test(view)).toBe(false);
    for (const t of BUILTIN_THEMES) {
      const c = paletteFrom(t);
      expect(contrast(c[fg], c.chrome), t.name).toBeGreaterThanOrEqual(MIN_CONTRAST);
    }
  });

  it("a Search output match reads on every card: the plain card and the running one (its tint over the page)", () => {
    for (const t of BUILTIN_THEMES) {
      const c = paletteFrom(t);
      expect(contrast(c.textBright, over(c.tints.match, c.surface)), `${t.name} surface`).toBeGreaterThanOrEqual(MIN_CONTRAST);
      expect(contrast(c.textBright, over(c.tints.match, over(c.tints.running, c.bg))), `${t.name} running`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    }
    // the mark draws its text bright on the match tint (ui/Marked.tsx)
    const marked = readFileSync(new URL("../src/ui/Marked.tsx", import.meta.url), "utf8");
    expect(marked).toMatch(/mark: \{ backgroundColor: colors\.tints\.match, color: colors\.textBright \}/);
  });

  it("computes WCAG ratios (black/white 21, equal colors 1)", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#569cd6", "#569cd6")).toBe(1);
  });

  it("the excluded pair really falls short in Dark Default (so it stays excluded)", () => {
    expect(contrast(dark.textDim, dark.surface)).toBeLessThan(4.5);
    expect(TEXT_ON.surface).not.toContain("textDim");
  });

  it("a field's placeholder is the boards' grey (text_muted), lifted only as far as it must to read on the surface", () => {
    // Dark Default's #828282 is 4.33:1 on the surface: a step lighter
    expect(contrast("#828282", dark.surface)).toBeLessThan(MIN_CONTRAST);
    expect(dark.placeholder).not.toBe("#828282");
    expect(luminance(dark.placeholder)).toBeLessThan(luminance(dark.textMuted));
    for (const t of BUILTIN_THEMES) {
      const c = paletteFrom(t);
      expect(contrast(c.placeholder, c.surface), t.name).toBeGreaterThanOrEqual(MIN_CONTRAST);
    }
  });
});
