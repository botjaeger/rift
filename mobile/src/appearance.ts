// Which theme the app shows (Settings › Appearance), free of React Native imports. `computer`:
// the theme of the computer in view (the pane's or host page's computer; elsewhere the last one
// opened), else the theme the app last showed, else Dark Default. `phone`: Light Default or Dark
// Default as the phone is. `theme`: that builtin theme. The choice stays on the phone.

import type { Theme } from "./protocol/types";
import type { Appearance } from "./stores";
import { BUILTIN_THEMES } from "./themes.data";
import { DARK_DEFAULT, LIGHT_DEFAULT } from "./ui/palette";

export interface ThemeSources {
  /** The theme of the computer in view (absent: none in view, or it sent none). */
  computer?: Theme | null;
  /** The computer's theme the app last showed. */
  last: Theme | null;
  /** The phone's appearance (`useColorScheme`); null when the system does not say. */
  scheme: string | null | undefined;
}

export function resolveTheme(appearance: Appearance, from: ThemeSources): Theme {
  switch (appearance.mode) {
    case "phone":
      return from.scheme === "light" ? LIGHT_DEFAULT : DARK_DEFAULT;
    case "theme":
      return BUILTIN_THEMES.find((t) => t.name === appearance.theme) ?? DARK_DEFAULT;
    case "computer":
      return from.computer ?? from.last ?? DARK_DEFAULT;
  }
}

/** Settings › Appearance's row value ("Same as the computer"). */
export function appearanceLabel(appearance: Appearance): string {
  switch (appearance.mode) {
    case "phone":
      return "Match this phone";
    case "theme":
      return appearance.theme;
    case "computer":
      return "Same as the computer";
  }
}

/**
 * What "Same as the computer" says under it: the computer in view and its theme ("studio-mac
 * uses Dracula now"), what is shown for one that sends none, or Dark Default before any has.
 */
export function computerDetail(inView: { name: string; theme?: Theme | null } | null, last: Theme | null): string {
  const shown = (last ?? DARK_DEFAULT).name;
  if (inView?.theme) return `${inView.name} uses ${inView.theme.name} now`;
  if (inView) return `${inView.name} sends no theme (update Rift there): ${shown}`;
  return last ? `${shown}, from the computer last opened` : `${shown} until a computer sends its theme`;
}
