// Rift's dark tokens (DESIGN.md; see palette.ts for the mapping and the contrast rules) and the
// mockups' faces: Inter for the UI, JetBrains Mono for commands and output. The fonts are
// embedded at build time (app.json, expo-font `fonts`; mobile/assets/fonts). Android takes no
// weight from fontWeight on a custom family, so every weight is a family of its own, and no
// style sets fontWeight (test/fonts.test.ts).
import { Platform } from "react-native";

export { colors, tints } from "./palette";

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
/** Keys and chips 4, small buttons and inputs 6, buttons 8, cards 12, sheets 16 (the mockups). */
export const radius = { control: 4, small: 6, button: 8, card: 12, sheet: 16 } as const;

/** `#rrggbb` with an alpha byte (`tint(colors.accent, 0x22)`): the mockups' tinted grounds. */
export const tint = (hex: string, alpha: number) => `${hex}${alpha.toString(16).padStart(2, "0")}`;
