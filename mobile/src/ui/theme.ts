// Rift's dark tokens (DESIGN.md; see palette.ts for the mapping and the contrast rules). UI text
// uses the system font; commands and output use mono.
import { Platform } from "react-native";

export { colors } from "./palette";

export const mono = Platform.select({ ios: "Menlo", android: "monospace", default: "Menlo, monospace" });

/** Minimum touch target (Apple HIG 44 pt). */
export const TOUCH = 44;
export const radius = { control: 4, card: 8 } as const;
