// iOS: the system tab bar (UITabBarController), which iOS 26 draws as the floating liquid-glass
// bar of the mockups; its colours, labels and icons follow them (Android and web draw the
// same bar themselves, _layout.tsx).
import { NativeTabs } from "expo-router/unstable-native-tabs";
import { usePathname } from "expo-router";
import { useApp } from "@/runtime";
import { colors, ui } from "@/ui/theme";

export default function TabsLayout() {
  const s = useApp();
  const path = usePathname();
  return (
    <NativeTabs
      // first run (no computer paired yet): Now is the Empty board, without the tab bar (only
      // there: unpairing the last computer from Settings must not strand the user)
      hidden={path === "/" && s.ready && !s.loadError && s.hosts.length === 0}
      tintColor={colors.accent}
      iconColor={{ default: colors.text, selected: colors.accent }}
      labelStyle={{ default: { fontFamily: ui(500), color: colors.text, fontSize: 10 }, selected: { color: colors.accent } }}
      minimizeBehavior="never"
    >
      <NativeTabs.Trigger name="index">
        <NativeTabs.Trigger.Icon sf={{ default: "clock", selected: "clock" }} />
        <NativeTabs.Trigger.Label>Now</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="computers">
        <NativeTabs.Trigger.Icon sf={{ default: "desktopcomputer", selected: "desktopcomputer" }} />
        <NativeTabs.Trigger.Label>Computers</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="settings">
        <NativeTabs.Trigger.Icon sf={{ default: "gearshape", selected: "gearshape" }} />
        <NativeTabs.Trigger.Label>Settings</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}
