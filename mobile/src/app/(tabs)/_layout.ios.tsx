// iOS: the system tab bar (UITabBarController), which iOS 26 draws as the floating liquid-glass
// bar of the mockups; its colours, labels and icons follow them (Android and web draw the
// same bar themselves, _layout.tsx).
import { NativeTabs } from "expo-router/unstable-native-tabs";
import { colors, ui } from "@/ui/theme";

export default function TabsLayout() {
  return (
    <NativeTabs
      tintColor={colors.accent}
      iconColor={{ default: colors.text, selected: colors.accent }}
      labelStyle={{ default: { fontFamily: ui(500), color: colors.text, fontSize: 10 }, selected: { color: colors.accent } }}
      // shrinks to the current tab while a page scrolls down, back on scrolling up
      minimizeBehavior="onScrollDown"
    >
      <NativeTabs.Trigger name="index" disableAutomaticContentInsets>
        <NativeTabs.Trigger.Icon sf={{ default: "clock", selected: "clock" }} />
        <NativeTabs.Trigger.Label>Now</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="computers" disableAutomaticContentInsets>
        <NativeTabs.Trigger.Icon sf={{ default: "desktopcomputer", selected: "desktopcomputer" }} />
        <NativeTabs.Trigger.Label>Computers</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="settings" disableAutomaticContentInsets>
        <NativeTabs.Trigger.Icon sf={{ default: "gearshape", selected: "gearshape" }} />
        <NativeTabs.Trigger.Label>Settings</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}
