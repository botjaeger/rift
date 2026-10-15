import type { BottomTabBarProps } from "expo-router/js-tabs";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Icon, type IconName } from "./Icon";
import { colors, ui } from "./theme";

const TABS: Record<string, { label: string; icon: IconName }> = {
  index: { label: "Now", icon: "clock" },
  computers: { label: "Computers", icon: "computer" },
  settings: { label: "Settings", icon: "settings" },
};

/**
 * The mockups' floating "glass" tab bar (Android and web; iOS uses the system's liquid-glass
 * bar): a 62 pt pill 24 pt from the sides, the current tab on a lighter capsule in the accent.
 * Without a blur view in the app the pill is near opaque, so content scrolling under it stays
 * legible.
 */
export function FloatingTabBar({ state, navigation }: BottomTabBarProps) {
  const insets = useSafeAreaInsets();
  return (
    <View
      accessibilityRole="tablist"
      accessibilityLabel="Primary"
      style={[styles.bar, { bottom: Math.max(26, insets.bottom + 8) }]}
    >
      {state.routes.map((route, i) => {
        const tab = TABS[route.name] ?? { label: route.name, icon: "computer" as const };
        const focused = state.index === i;
        // accent text on the lighter capsule is 3.5:1, so the current tab's label is bright and
        // only its icon takes the accent (palette.ts)
        const color = focused ? colors.accent : colors.text;
        return (
          <Pressable
            key={route.key}
            accessibilityRole="tab"
            accessibilityLabel={tab.label}
            accessibilityState={{ selected: focused }}
            onPress={() => {
              const e = navigation.emit({ type: "tabPress", target: route.key, canPreventDefault: true });
              if (!focused && !e.defaultPrevented) navigation.navigate(route.name, route.params);
            }}
            style={({ pressed }) => [styles.item, focused && styles.itemOn, pressed && styles.pressed]}
          >
            <Icon name={tab.icon} size={22} color={color} />
            <Text style={[styles.label, { color: focused ? colors.textBright : colors.text }]}>{tab.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    position: "absolute",
    left: 24,
    right: 24,
    height: 62,
    padding: 5,
    gap: 2,
    flexDirection: "row",
    borderRadius: 31,
    backgroundColor: "rgba(44, 44, 48, 0.94)",
    borderWidth: 1,
    borderColor: "rgba(255, 255, 255, 0.14)",
    ...Platform.select({
      web: { boxShadow: "0 10px 30px rgba(0, 0, 0, 0.45)" },
      default: { elevation: 12, shadowColor: "#000", shadowOpacity: 0.45, shadowRadius: 15, shadowOffset: { width: 0, height: 10 } },
    }),
  },
  item: { flex: 1, alignItems: "center", justifyContent: "center", gap: 2, borderRadius: 26 },
  itemOn: { backgroundColor: "rgba(255, 255, 255, 0.1)" },
  pressed: { opacity: 0.7 },
  label: { fontFamily: ui(500), fontSize: 10 },
});
