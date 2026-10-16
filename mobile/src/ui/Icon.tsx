import { SymbolView, type SymbolViewProps } from "expo-symbols";
import type { ColorValue } from "react-native";

/**
 * The mockups' line icons as system symbols (SF Symbols on iOS, Material Symbols on Android
 * and web): the app draws no SVG. Decorative: the control around an icon carries its label.
 */
const ICONS = {
  computer: { ios: "desktopcomputer", android: "computer", web: "computer" },
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
  // the prompt chips' glyphs (the desktop draws Nerd Font marks; the app draws no fonts of its own for them)
  diff: { ios: "plusminus", android: "difference", web: "difference" },
  user: { ios: "person", android: "person", web: "person" },
  laptop: { ios: "laptopcomputer", android: "laptop_mac", web: "laptop_mac" },
  code: { ios: "chevron.left.forwardslash.chevron.right", android: "code", web: "code" },
  pullRequest: { ios: "arrow.triangle.pull", android: "call_merge", web: "call_merge" },
  docker: { ios: "shippingbox", android: "deployed_code", web: "deployed_code" },
  calendar: { ios: "calendar", android: "calendar_today", web: "calendar_today" },
  clock: { ios: "clock", android: "schedule", web: "schedule" },
  hourglass: { ios: "hourglass", android: "hourglass_top", web: "hourglass_top" },
  keyboard: { ios: "keyboard", android: "keyboard", web: "keyboard" },
  run: { ios: "arrow.up", android: "arrow_upward", web: "arrow_upward" },
  scan: { ios: "qrcode.viewfinder", android: "qr_code_scanner", web: "qr_code_scanner" },
  // a pane's avatar and the Terminal view switch
  terminal: { ios: "apple.terminal", android: "terminal", web: "terminal" },
  // the key row's keys (the Keys and Pane boards)
  interrupt: { ios: "xmark.octagon", android: "dangerous", web: "dangerous" },
  up: { ios: "arrow.up", android: "arrow_upward", web: "arrow_upward" },
  down: { ios: "arrow.down", android: "arrow_downward", web: "arrow_downward" },
  left: { ios: "arrow.left", android: "arrow_back", web: "arrow_back" },
  right: { ios: "arrow.right", android: "arrow_forward", web: "arrow_forward" },
  home: { ios: "arrow.left.to.line", android: "first_page", web: "first_page" },
  end: { ios: "arrow.right.to.line", android: "last_page", web: "last_page" },
  pageUp: { ios: "chevron.up.2", android: "keyboard_double_arrow_up", web: "keyboard_double_arrow_up" },
  pageDown: { ios: "chevron.down.2", android: "keyboard_double_arrow_down", web: "keyboard_double_arrow_down" },
  backspace: { ios: "delete.left", android: "backspace", web: "backspace" },
  deleteKey: { ios: "delete.right", android: "backspace", web: "backspace" },
  paste: { ios: "doc.on.clipboard", android: "content_paste", web: "content_paste" },
  newTab: { ios: "plus.rectangle", android: "add_box", web: "add_box" },
  newPane: { ios: "rectangle.split.2x1", android: "vertical_split", web: "vertical_split" },
  hideKeyboard: { ios: "keyboard.chevron.compact.down", android: "keyboard_hide", web: "keyboard_hide" },
  editKeys: { ios: "slider.horizontal.3", android: "tune", web: "tune" },

  chevronDown: { ios: "chevron.down", android: "expand_more", web: "expand_more" },
  more: { ios: "ellipsis", android: "more_horiz", web: "more_horiz" },
  copy: { ios: "doc.on.doc", android: "content_copy", web: "content_copy" },
  edit: { ios: "pencil", android: "edit", web: "edit" },
  stop: { ios: "stop.fill", android: "stop", web: "stop" },
  rerun: { ios: "arrow.clockwise", android: "refresh", web: "refresh" },
  file: { ios: "doc", android: "description", web: "description" },
  check: { ios: "checkmark", android: "check", web: "check" },
  lock: { ios: "lock", android: "lock", web: "lock" },
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
