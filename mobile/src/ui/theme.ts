// The app's palette at runtime (palette.ts makes it from a theme; the root layout provides the
// one Settings › Appearance resolves to) and the mockups' faces: Inter for the UI, JetBrains
// Mono for commands and output. The fonts are embedded at build time (app.json, expo-font
// `fonts`; mobile/assets/fonts). Android takes no weight from fontWeight on a custom family, so
// every weight is a family of its own, and no style sets fontWeight (test/fonts.test.ts).
//
// Colours are never read at module load: a component reads them with `useColors()`, and its
// styles come from `const useStyles = makeStyles((colors) => ({ ... }))`, called in the component
// as `const styles = useStyles()` (made once per palette).
import { createContext, createElement, useContext, type ReactNode } from "react";
import { Platform, StyleSheet } from "react-native";
import type { Theme } from "../protocol/types";
import { DARK_DEFAULT, paletteFrom, withAlpha, type Palette } from "./palette";

export type { Palette } from "./palette";

const palettes = new Map<string, Palette>();

/** The palette of `theme`, made once per theme (the same object while the theme is the same). */
export function paletteOf(theme: Theme): Palette {
  const key = JSON.stringify(theme);
  let p = palettes.get(key);
  if (!p) {
    p = paletteFrom(theme);
    // a computer could send many: keep the few in use
    if (palettes.size >= 32) palettes.clear();
    palettes.set(key, p);
  }
  return p;
}

const PaletteContext = createContext<Palette>(paletteOf(DARK_DEFAULT));

/** Gives `palette` to everything under it (the root layout; a camera page's Dark Default). */
export function PaletteProvider({ palette, children }: { palette: Palette; children?: ReactNode }) {
  return createElement(PaletteContext.Provider, { value: palette }, children);
}

/** The palette in use: every colour by name (`colors.accent`), `tints`, `dark`, `name`. */
export function useColors(): Palette {
  return useContext(PaletteContext);
}

/**
 * A style sheet made from the palette: `const useStyles = makeStyles((colors) => ({ ... }))` at
 * module level, `const styles = useStyles()` in the component. Each palette's sheet is made
 * once.
 */
export function makeStyles<T extends StyleSheet.NamedStyles<T> | StyleSheet.NamedStyles<unknown>>(
  factory: (colors: Palette) => T & StyleSheet.NamedStyles<unknown>,
): () => T {
  const made = new WeakMap<Palette, T>();
  return function useStyles(): T {
    const colors = useContext(PaletteContext);
    let sheet = made.get(colors);
    if (!sheet) {
      sheet = StyleSheet.create(factory(colors));
      made.set(colors, sheet);
    }
    return sheet;
  };
}

/** Each face's family name: its PostScript name, which is also its file name (iOS and Android). */
export const FONT_FACES = {
  regular: "Inter-Regular",
  medium: "Inter-Medium",
  semibold: "Inter-SemiBold",
  mono: "JetBrainsMono-Regular",
  monoMedium: "JetBrainsMono-Medium",
  monoBold: "JetBrainsMono-Bold",
} as const;

/** On the web the faces are not embedded: fall back to the same families, then the system's. */
const face = (name: string, fallback: string) => (Platform.OS === "web" ? `${name}, ${fallback}` : name);

/**
 * A value made from the palette (a map of colours by tone), once per palette:
 * `const toneColor = fromPalette((colors) => ({ ok: colors.success }))`, then
 * `toneColor(colors).ok` in the component.
 */
export function fromPalette<T>(make: (colors: Palette) => T): (colors: Palette) => T {
  const made = new WeakMap<Palette, T>();
  return (colors) => {
    if (!made.has(colors)) made.set(colors, make(colors));
    return made.get(colors)!;
  };
}

/** Inter at the mockups' weights: 400 (text), 500 (labels), 600 (titles, primary buttons). */
export function ui(weight: 400 | 500 | 600 = 400): string {
  const name = weight === 600 ? FONT_FACES.semibold : weight === 500 ? FONT_FACES.medium : FONT_FACES.regular;
  return face(name, "Inter, system-ui, sans-serif");
}

export const mono = face(FONT_FACES.mono, "'JetBrains Mono', Menlo, monospace");
export const monoMedium = face(FONT_FACES.monoMedium, "'JetBrains Mono', Menlo, monospace");
/** Bold terminal text (a run's bold flag). */
export const monoBold = face(FONT_FACES.monoBold, "'JetBrains Mono', Menlo, monospace");

/** Minimum touch target (Apple HIG 44 pt). */
export const TOUCH = 44;
/**
 * One radius per role (the Standard board): keys and small chips 6, buttons and fields 10, cards
 * 14, sheets 20; filter chips are 32 pt pills (16); icon buttons are round.
 */
export const radius = { control: 6, small: 6, button: 10, field: 10, chip: 16, card: 14, sheet: 20 } as const;

/** `#rrggbb` with an alpha byte (`tint(colors.accent, 0x22)`): the mockups' tinted grounds. */
export const tint = withAlpha;
