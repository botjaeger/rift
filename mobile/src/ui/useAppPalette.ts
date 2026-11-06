// The palette Settings › Appearance resolves to (appearance.ts), for the root layout to provide.
import { useMemo } from "react";
import { useColorScheme } from "react-native";
import { resolveTheme } from "@/appearance";
import { useApp } from "@/runtime";
import { paletteOf, type Palette } from "./theme";

/**
 * The app's palette now: with "Same as the computer", the theme of the computer in view (the
 * pane's or host page's; elsewhere the last one opened), else the last one shown; with "Match
 * this phone", Light or Dark Default as the phone is; else the chosen builtin theme.
 */
export function useAppPalette(): Palette {
  const s = useApp();
  const scheme = useColorScheme();
  const inView = s.lastView ? s.hosts.find((h) => h.record.key === s.lastView!.host)?.theme : undefined;
  const theme = resolveTheme(s.settings.appearance, { computer: inView, last: s.lastTheme, scheme });
  return useMemo(() => paletteOf(theme), [theme]);
}
