import { SymbolView, type SymbolViewProps } from "expo-symbols";
import type { ColorValue } from "react-native";

/**
 * The mockups' line icons as system symbols (SF Symbols on iOS, Material Symbols on Android
 * and web): the app draws no SVG. Decorative: the control around an icon carries its label.
 */
const ICONS = {
  computer: { ios: "desktopcomputer", android: "computer", web: "computer" },
  clock: { ios: "clock", android: "schedule", web: "schedule" },
  add: { ios: "plus", android: "add", web: "add" },
  back: { ios: "chevron.left", android: "arrow_back_ios_new", web: "arrow_back_ios_new" },
  forward: { ios: "chevron.right", android: "chevron_right", web: "chevron_right" },
  close: { ios: "xmark", android: "close", web: "close" },
  bell: { ios: "bell", android: "notifications", web: "notifications" },
  bellFill: { ios: "bell.fill", android: "notifications", web: "notifications" },
  bellOff: { ios: "bell.slash", android: "notifications_off", web: "notifications_off" },
  settings: { ios: "gearshape", android: "settings", web: "settings" },
  wrapOn: { ios: "return", android: "wrap_text", web: "wrap_text" },
  wrapOff: { ios: "text.alignleft", android: "format_align_left", web: "format_align_left" },
  search: { ios: "magnifyingglass", android: "search", web: "search" },
  folder: { ios: "folder", android: "folder", web: "folder" },
  branch: { ios: "arrow.triangle.branch", android: "fork_right", web: "fork_right" },
  keyboard: { ios: "keyboard", android: "keyboard", web: "keyboard" },
  run: { ios: "arrow.up", android: "arrow_upward", web: "arrow_upward" },
  scan: { ios: "qrcode.viewfinder", android: "qr_code_scanner", web: "qr_code_scanner" },
} as const satisfies Record<string, SymbolViewProps["name"]>;

export type IconName = keyof typeof ICONS;

export function Icon({ name, size = 18, color }: { name: IconName; size?: number; color: ColorValue }) {
  return (
    <SymbolView
      name={ICONS[name]}
      size={size}
      tintColor={color}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    />
  );
}
