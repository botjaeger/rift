// Android and web: a regular bottom tab bar in Rift's chrome colors (iOS uses _layout.ios.tsx).
import { SymbolView, type SymbolViewProps } from "expo-symbols";
import { Tabs } from "expo-router";
import type { ColorValue } from "react-native";
import { colors } from "@/ui/theme";

function icon(name: SymbolViewProps["name"]) {
  function TabIcon({ color, size }: { color: ColorValue; size: number }) {
    return <SymbolView name={name} tintColor={color} size={size} />;
  }
  return TabIcon;
}

export default function TabsLayout() {
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarActiveTintColor: colors.accent,
        tabBarInactiveTintColor: colors.textMuted,
        tabBarStyle: { backgroundColor: colors.chrome, borderTopColor: colors.hairline },
        sceneStyle: { backgroundColor: colors.bg },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{ title: "Hosts", tabBarIcon: icon({ ios: "desktopcomputer", android: "computer", web: "computer" }) }}
      />
      <Tabs.Screen
        name="activity"
        options={{ title: "Activity", tabBarIcon: icon({ ios: "bell", android: "notifications", web: "notifications" }) }}
      />
      <Tabs.Screen
        name="settings"
        options={{ title: "Settings", tabBarIcon: icon({ ios: "gearshape", android: "settings", web: "settings" }) }}
      />
    </Tabs>
  );
}
