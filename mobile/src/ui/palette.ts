// The app's colours, made from a desktop theme's tokens (a computer's `theme`, or one of the
// builtin themes in themes.data.ts), free of React Native imports so the contrast test runs in
// node. Mapping: bg = bg, chrome = bg_chrome, surface = surface, hairline = border, textBright =
// text_bright, text = text_ui, textMuted = text_dim, textDim = text_muted, accent, success,
// error, warning, fg = fg (terminal text, block output), magenta = ANSI magenta. Derived:
// indigo (the accent mixed half with magenta, as the desktop mixes it), borderStrong / glass /
// glassRim / chipFill (text_bright at 14 / 8 / 12 / 4 %), floating (surface at 94 %), the
// raised grounds (text_bright laid over bg: faint 6 %, switchOff 16 %, skeleton 12 %, raised
// 13 % for the loading card, menu 15 % for menus and pickers, segmentOn 20 %), the tints
// (a token at low alpha) and onAccent (the ground's colour as text on a filled accent, success,
// warning or error ground). Every text colour is then held to 4.5:1 on each ground it is drawn
// on (TEXT_ON, TINTED_TEXT): one that falls short is lifted toward the text colour (`text`,
// then black or white), as the desktop does for its hint text. Dark Default needs no lift.

import type { Theme } from "../protocol/types";
import { BUILTIN_THEMES } from "../themes.data";

/** The app's colour names (the contrast pairs name them). */
export type ColorName =
  | "bg"
  | "chrome"
  | "surface"
  | "hairline"
  | "textBright"
  | "text"
  | "textMuted"
  | "textDim"
  | "accent"
  | "success"
  | "error"
  | "warning"
  | "magenta"
  | "indigo"
  | "fg"
  | "onAccent"
  | "borderStrong"
  | "glass"
  | "glassRim"
  | "floating"
  | "camera"
  | "viewfinder"
  | "selection"
  | "faint"
  | "switchOff"
  | "raised"
  | "menu"
  | "segmentOn"
  | "skeleton"
  | "askGround";

export type TintName =
  | "accentBadge"
  | "successBadge"
  | "warningBadge"
  | "errorBadge"
  | "chipFill"
  | "running"
  | "bellOn"
  | "diffAdded"
  | "diffRemoved"
  | "answerAdded"
  | "answerRemoved"
  | "match";

/** What `useColors()` gives: every colour by name, the tints, and the theme it came from. */
export type Palette = Readonly<Record<ColorName, string>> & {
  readonly tints: Readonly<Record<TintName, string>>;
  /** The theme's 16 ANSI colours (`#rrggbb`). */
  readonly ansi: readonly string[];
  /** A dark ground: light status bar and keyboard. */
  readonly dark: boolean;
  /** The theme's name ("Dark Default"). */
  readonly name: string;
};

/** Grounds text is drawn on (TEXT_ON's keys). */
export type Ground =
  | "bg"
  | "chrome"
  | "surface"
  | "menu"
  | "raised"
  | "askGround"
  | "accent"
  | "warning"
  | "error"
  | "success"
  | "camera";

/**
 * Every text colour the app draws, on each ground it is drawn on. test/contrast.test.ts holds
 * each pair to WCAG AA (4.5:1) in every builtin theme; paletteFrom lifts a colour that falls
 * short. One token is kept off a ground: `textDim` on surface (4.33 in Dark Default), where
 * `textMuted` is used instead. Camera pages draw on the camera's black in Dark Default whatever
 * the theme (CAMERA_TEXT).
 */
export const TEXT_ON: Readonly<Record<Exclude<Ground, "camera">, readonly ColorName[]>> = {
  bg: ["textBright", "text", "fg", "textMuted", "textDim", "accent", "success", "warning", "error", "magenta", "indigo"],
  chrome: ["textBright", "text", "textMuted", "textDim", "accent", "success", "warning", "error", "magenta", "indigo"],
  surface: ["textBright", "text", "textMuted", "accent", "success", "warning", "error"],
  // menus and key tips (Close pane and Close tab in the destructive red)
  menu: ["textBright", "textMuted", "warning", "error"],
  // the pane's loading card
  raised: ["textBright", "text", "textMuted", "accent", "error"],
  // a folded question
  askGround: ["textBright", "warning"],
  // a filled button or badge's label
  accent: ["onAccent"],
  // the count on a back badge (the panes that need you)
  warning: ["onAccent"],
  error: ["onAccent"],
  success: ["onAccent"],
};

/** What a camera page (Pair a computer) draws on the camera's black, in the Dark Default palette. */
export const CAMERA_TEXT: readonly ColorName[] = ["textBright", "text", "textMuted", "warning"];

/**
 * The mockups' tinted grounds (a token at low alpha over a card or page): the accent pills
 * ("On screen", a running pane's time, step numbers) and the badges of the Host board. The
 * mockups' `#19aad822` puts accent text at 4.34:1 on a surface, so the accent tint is `1a`
 * (4.58:1). TINTED_TEXT lists each text drawn on one; test/contrast.test.ts checks them.
 */
const TINT_OF: Readonly<Record<TintName, readonly [ColorName, number]>> = {
  accentBadge: ["accent", 0x1a],
  successBadge: ["success", 0x14],
  warningBadge: ["warning", 0x1f],
  errorBadge: ["error", 0x33],
  /** A prompt chip's fill (text_bright 4 %, on chrome), as the desktop's `chip_fill`. */
  chipFill: ["textBright", 0x0a],
  /** The running block's ground (on bg). */
  running: ["accent", 0x14],
  /** The bell that is on (on the running block's ground). */
  bellOn: ["accent", 0x33],
  /** A diff's added and removed lines (on the chrome reading pane), as the desktop review panel. */
  diffAdded: ["success", 0x1f],
  diffRemoved: ["error", 0x26],
  /** The answer page's context diff (the P2 Reply board), on the chrome box. */
  answerAdded: ["success", 0x1f],
  answerRemoved: ["error", 0x1f],
  /** A Search output match in a block's command or output (the New Blocks board's `#ffd70055`). */
  match: ["warning", 0x55],
};

/** Text on a tint over a ground; `under`: a second tint between them (a mark on the running card). */
export const TINTED_TEXT: readonly { fg: ColorName; tint: TintName; ground: "bg" | "chrome" | "surface"; under?: TintName }[] = [
  // the prompt chips' parts, on the chip fill over the command line's chrome ground
  ...(["textBright", "text", "textMuted", "textDim", "accent", "success", "warning", "error", "magenta", "indigo"] as const).map(
    (fg) => ({ fg, tint: "chipFill" as const, ground: "chrome" as const }),
  ),
  { fg: "accent", tint: "accentBadge", ground: "surface" },
  { fg: "accent", tint: "accentBadge", ground: "chrome" },
  // the Run sheet's first completion (surface) and its kind
  { fg: "textBright", tint: "accentBadge", ground: "surface" },
  { fg: "textMuted", tint: "accentBadge", ground: "surface" },
  { fg: "success", tint: "successBadge", ground: "surface" },
  { fg: "warning", tint: "warningBadge", ground: "surface" },
  { fg: "textBright", tint: "errorBadge", ground: "surface" },
  { fg: "textBright", tint: "running", ground: "bg" },
  { fg: "fg", tint: "running", ground: "bg" },
  { fg: "accent", tint: "running", ground: "bg" },
  { fg: "success", tint: "running", ground: "bg" },
  { fg: "fg", tint: "diffAdded", ground: "chrome" },
  { fg: "success", tint: "diffAdded", ground: "chrome" },
  { fg: "fg", tint: "diffRemoved", ground: "chrome" },
  { fg: "textBright", tint: "diffRemoved", ground: "chrome" },
  { fg: "textBright", tint: "answerAdded", ground: "chrome" },
  { fg: "textBright", tint: "answerRemoved", ground: "chrome" },
  // a Search output match on a block card, and on the running card (its tint over the page)
  { fg: "textBright", tint: "match", ground: "surface" },
  { fg: "textBright", tint: "match", ground: "bg", under: "running" },
];

/** The least contrast every text and ground pair keeps (WCAG AA). */
export const MIN_CONTRAST = 4.5;

const HEX = /^[0-9a-f]{6}([0-9a-f]{2})?$/i;

/** A wire colour (`rrggbb` / `rrggbbaa`) as `#rrggbb` / `#rrggbbaa`; null when it is not one. */
export function cssColor(wire: string): string | null {
  return HEX.test(wire) ? `#${wire.toLowerCase()}` : null;
}

const channel = (hex: string, i: number) => parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16);
const toHex = (rgb: readonly number[]) => `#${rgb.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;

/** `#rrggbbaa` over the opaque `ground`, as one opaque `#rrggbb` (`#rrggbb` stays as it is). */
export function over(rgba: string, ground: string): string {
  if (rgba.length === 7) return rgba;
  const a = parseInt(rgba.slice(7, 9), 16) / 255;
  return toHex([0, 1, 2].map((i) => channel(rgba, i) * a + channel(ground, i) * (1 - a)));
}

/** `#rrggbb` with an alpha byte: the mockups' tinted grounds. */
export const withAlpha = (hex: string, alpha: number) => `${hex.slice(0, 7)}${alpha.toString(16).padStart(2, "0")}`;

/** `a` moved `t` (0–1) of the way to `b` (opaque `#rrggbb`). */
export function mix(a: string, b: string, t: number): string {
  return toHex([0, 1, 2].map((i) => channel(a, i) + (channel(b, i) - channel(a, i)) * t));
}

/** WCAG 2 relative luminance of an `#rrggbb` color. */
export function luminance(hex: string): number {
  const ch = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = ch.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio of two opaque `#rrggbb` colors. */
export function contrast(a: string, b: string): number {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/**
 * `fg` as it reads on every one of `grounds` (opaque): itself when it does, else moved toward
 * `toward` (the text colour) and on toward black or white, whichever the grounds need, in small
 * steps until it holds MIN_CONTRAST on all of them.
 */
export function lift(fg: string, grounds: readonly string[], toward: string, minimum = MIN_CONTRAST): string {
  const reads = (c: string) => grounds.every((g) => contrast(c, g) >= minimum);
  if (reads(fg)) return fg;
  // the extreme the grounds have most contrast with, judged by the darkest and lightest of them
  const lum = grounds.map(luminance);
  const black = (Math.min(...lum) + 0.05) / 0.05;
  const white = 1.05 / (Math.max(...lum) + 0.05);
  const extreme = black >= white ? "#000000" : "#ffffff";
  const STEPS = 32;
  for (let i = 1; i <= STEPS; i++) {
    const c = mix(fg, toward, i / STEPS);
    if (reads(c)) return c;
  }
  for (let i = 1; i <= STEPS; i++) {
    const c = mix(toward, extreme, i / STEPS);
    if (reads(c)) return c;
  }
  return extreme;
}

/** The least contrast of a decorative mark (an avatar's `>_`) on its ground: WCAG's 3:1 for graphics. */
export const MARK_CONTRAST = 3;

/**
 * An avatar's mark colour (`color`, on an 18 % tint of it over the page or a card): itself where
 * it shows, lifted toward the text colour where it would not (a pale yellow on a light theme).
 */
export function avatarInk(color: string, colors: Pick<Palette, "bg" | "surface" | "text">): string {
  const ground = (under: string) => over(withAlpha(color, 0x2e), under);
  return lift(color, [ground(colors.bg), ground(colors.surface)], colors.text, MARK_CONTRAST);
}

/** `#rrggbb` of a ground token: one with alpha is laid over `under`. */
const opaque = (c: string, under: string) => over(c, under);

/**
 * The app's palette for `theme`: its tokens under the app's names, the derived colours and
 * tints, and every text colour lifted where a pair falls short. A token that is not a colour
 * takes Dark Default's.
 */
export function paletteFrom(theme: Theme): Palette {
  const base = DARK_DEFAULT;
  const tok = (k: keyof Omit<Theme, "name" | "dark" | "ansi">) => cssColor(theme[k]) ?? cssColor(base[k])!;
  const ansi = Array.from({ length: 16 }, (_, i) => cssColor(theme.ansi[i] ?? "") ?? cssColor(base.ansi[i]!)!).map((c) =>
    c.slice(0, 7),
  );
  const bg = opaque(tok("bg"), "#000000");
  const chrome = opaque(tok("bg_chrome"), bg);
  const surface = opaque(tok("surface"), bg);
  const textBright = opaque(tok("text_bright"), bg);
  const accent = opaque(tok("accent"), bg);
  const magenta = ansi[5]!;
  const raw: Record<ColorName, string> = {
    bg,
    chrome,
    surface,
    hairline: tok("border"),
    textBright,
    text: opaque(tok("text_ui"), bg),
    textMuted: opaque(tok("text_dim"), bg),
    textDim: opaque(tok("text_muted"), bg),
    accent,
    success: opaque(tok("success"), bg),
    error: opaque(tok("error"), bg),
    warning: opaque(tok("warning"), bg),
    magenta,
    // floor, as the desktop's integer mix: Dark Default's #8c9cea
    indigo: toHex([0, 1, 2].map((i) => Math.floor((channel(accent, i) + channel(magenta, i)) / 2))),
    fg: opaque(tok("fg"), bg),
    onAccent: bg,
    borderStrong: withAlpha(textBright, 0x24),
    glass: withAlpha(textBright, 0x14),
    glassRim: withAlpha(textBright, 0x1f),
    floating: withAlpha(surface, 0xf0),
    // the camera is black in every theme
    camera: "#000000",
    viewfinder: "#121212",
    selection: tok("selection"),
    faint: withAlpha(textBright, 0x0f),
    switchOff: withAlpha(textBright, 0x29),
    // Dark Default: #232323, #262626, #2b2b2b, #373737 (the boards' greys)
    skeleton: over(withAlpha(textBright, 0x1f), bg),
    raised: over(withAlpha(textBright, 0x22), bg),
    menu: over(withAlpha(textBright, 0x27), bg),
    segmentOn: over(withAlpha(textBright, 0x33), bg),
    // a question card's ground: the warning badge over the page (Dark Default #201808)
    askGround: over(withAlpha(opaque(tok("warning"), bg), 0x1f), bg),
  };
  const tints = Object.fromEntries(
    Object.entries(TINT_OF).map(([name, [color, alpha]]) => [name, withAlpha(raw[color], alpha)]),
  ) as Record<TintName, string>;
  // every opaque ground each text colour is drawn on: plain grounds, then tints over theirs
  const grounds = new Map<ColorName, string[]>();
  const add = (fg: ColorName, g: string) => grounds.set(fg, [...(grounds.get(fg) ?? []), g]);
  for (const [ground, fgs] of Object.entries(TEXT_ON)) for (const fg of fgs) add(fg, raw[ground as ColorName]);
  for (const t of TINTED_TEXT) add(t.fg, over(tints[t.tint], t.under ? over(tints[t.under], raw[t.ground]) : raw[t.ground]));
  const colors = { ...raw };
  for (const [fg, gs] of grounds) {
    // the text colours themselves head straight for black or white
    const toward = fg === "text" || fg === "textBright" || fg === "fg" || fg === "onAccent" ? raw[fg] : raw.text;
    colors[fg] = lift(raw[fg], gs, toward);
  }
  return { ...colors, tints, ansi, dark: theme.dark, name: theme.name };
}

const builtin = (name: string): Theme => {
  const t = BUILTIN_THEMES.find((x) => x.name === name);
  if (!t) throw new Error(`no builtin theme ${name}`);
  return t;
};

/** The desktop's default dark and light themes (Match this phone; Dark Default: no theme known). */
export const DARK_DEFAULT: Theme = builtin("Dark Default");
export const LIGHT_DEFAULT: Theme = builtin("Light Default");
