import { Platform, type AccessibilityRole, type AccessibilityState } from "react-native";

/**
 * A toggle button's role and state (the bell, ⌃, Search output): iOS and Android's
 * `togglebutton` with `checked`; on the web a `button` with `aria-pressed`, because
 * react-native-web passes "togglebutton" through as the element's role, which is no ARIA role
 * (screen readers then announce no button at all).
 */
export function toggleA11y(on: boolean, disabled = false): { accessibilityRole: AccessibilityRole; accessibilityState: AccessibilityState } {
  if (Platform.OS === "web") {
    // `aria-pressed` is not in React Native's prop types; react-native-web renders it
    return { accessibilityRole: "button", accessibilityState: { disabled }, ...({ "aria-pressed": on } as object) };
  }
  return { accessibilityRole: "togglebutton", accessibilityState: { checked: on, disabled } };
}
