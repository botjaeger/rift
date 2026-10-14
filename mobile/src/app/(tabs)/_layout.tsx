// Android and web: the mockups' floating tab bar (iOS uses _layout.ios.tsx, the system bar).
import { Tabs } from "expo-router";
import { FloatingTabBar } from "@/ui/FloatingTabBar";
import { colors } from "@/ui/theme";

export default function TabsLayout() {
  return (
    <Tabs
      tabBar={(props) => <FloatingTabBar {...props} />}
      screenOptions={{ headerShown: false, sceneStyle: { backgroundColor: colors.chrome } }}
    >
      <Tabs.Screen name="index" options={{ title: "Now" }} />
      <Tabs.Screen name="computers" options={{ title: "Computers" }} />
      <Tabs.Screen name="settings" options={{ title: "Settings" }} />
    </Tabs>
  );
}
