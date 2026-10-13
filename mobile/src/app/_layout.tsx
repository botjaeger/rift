import * as Notifications from "expo-notifications";
import { Stack, useRouter } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { useEffect } from "react";
import { Platform } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { tapTarget } from "@/notify";
import { fleet, startPlatformWatchers } from "@/runtime";
import { LockGate } from "@/ui/LockGate";
import { colors } from "@/ui/theme";

// A link straight to a screen (a notification tap, rift://host/…/pane/…) keeps the tabs below
// it, so Back leads to Hosts instead of out of the app.
export const unstable_settings = { initialRouteName: "(tabs)" };

/** Notifications already opened in this app run (kept across root remounts). */
const openedTaps = new Set<string>();

/** Opens the pane a tapped notification is about, once per notification. */
function useNotificationTaps() {
  const router = useRouter();
  useEffect(() => {
    if (Platform.OS === "web") return;
    const open = (r: Notifications.NotificationResponse | null) => {
      const target = tapTarget(r, openedTaps);
      if (!target) return;
      // Handled: a later launch-response read must not open it again.
      Notifications.clearLastNotificationResponse();
      router.push({ pathname: "/host/[key]/pane/[id]", params: { key: target.host, id: String(target.pane) } });
    };
    const sub = Notifications.addNotificationResponseReceivedListener(open);
    open(Notifications.getLastNotificationResponse());
    return () => sub.remove();
  }, [router]);
}

export default function RootLayout() {
  useEffect(() => {
    void fleet.init();
    return startPlatformWatchers();
  }, []);
  useNotificationTaps();

  return (
    <SafeAreaProvider>
      <StatusBar style="light" />
      <LockGate>
        <Stack
          screenOptions={{
            headerStyle: { backgroundColor: colors.chrome },
            headerTintColor: colors.textBright,
            headerTitleStyle: { color: colors.textBright },
            headerShadowVisible: false,
            contentStyle: { backgroundColor: colors.bg },
          }}
        >
          <Stack.Screen name="(tabs)" options={{ headerShown: false, title: "Hosts" }} />
          <Stack.Screen name="add" options={{ title: "Pair a desktop" }} />
          <Stack.Screen name="scan" options={{ title: "Scan pairing code" }} />
          <Stack.Screen name="pair" options={{ title: "Pair" }} />
          <Stack.Screen name="host/[key]/index" options={{ title: "Host" }} />
          <Stack.Screen name="host/[key]/pane/[id]" options={{ title: "Pane" }} />
          <Stack.Screen
            name="confirm"
            options={{
              title: "Run command",
              presentation: "formSheet",
              sheetAllowedDetents: [0.65, 1],
              sheetGrabberVisible: true,
              headerShown: false,
              contentStyle: { backgroundColor: colors.surface },
            }}
          />
        </Stack>
      </LockGate>
    </SafeAreaProvider>
  );
}
