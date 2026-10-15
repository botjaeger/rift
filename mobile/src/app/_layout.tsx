import * as Notifications from "expo-notifications";
import * as SplashScreen from "expo-splash-screen";
import { DarkTheme, Stack, ThemeProvider, useRouter } from "expo-router";
import { StatusBar } from "expo-status-bar";
import * as Updates from "expo-updates";
import { useCallback, useEffect, useState } from "react";
import { Alert, LogBox, Platform } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { alertAuth } from "@/lock";
import { tapTarget } from "@/notify";
import Constants from "expo-constants";
import { parseChangelog, releaseKey, releasesSince } from "@/changelog";
import { CHANGELOG } from "@/changelog.data";
import { shouldOfferRestart } from "@/ota";
import { APP_UPDATE } from "@/release";
import { appLock, authenticate, fleet, kv, startPlatformWatchers, useApp, useLock } from "@/runtime";
import { loadSeenRelease, saveSeenRelease } from "@/stores";
import { LockGate } from "@/ui/LockGate";
import { Splash } from "@/ui/Splash";
import { colors } from "@/ui/theme";

// expo-router builds its links from the first scheme ("rift"); "rift-remote" is the pairing
// link's scheme, kept on purpose, so the dev notice about several schemes is expected.
LogBox.ignoreLogs(["Linking found multiple possible URI schemes"]);

// The native splash (the ground only) stays until the animated one (ui/Splash.tsx) is on screen.
void SplashScreen.preventAutoHideAsync().catch(() => undefined);

// A link straight to a screen (a notification tap, rift://host/…/pane/…) keeps the tabs below
// it, so Back leads to Now instead of out of the app.
export const unstable_settings = { initialRouteName: "(tabs)" };

/** The navigators' own grounds (containers, cards, the tab bar's), dark like every page: the
 *  default theme is light, and its white showed around a page while it slid in or out. */
const navTheme = {
  ...DarkTheme,
  colors: {
    ...DarkTheme.colors,
    background: colors.bg,
    card: colors.chrome,
    border: colors.chrome,
    text: colors.textBright,
    primary: colors.accent,
  },
};

/** Notifications already opened in this app run (kept across root remounts). */
const openedTaps = new Set<string>();

/** Whether `ok()` holds within `ms` (checked every 100 ms). */
async function until(ok: () => boolean, ms: number): Promise<boolean> {
  for (let waited = 0; !ok(); waited += 100) {
    if (waited >= ms) return false;
    await new Promise((r) => setTimeout(r, 100));
  }
  return true;
}

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
      if (target.ask === undefined) {
        router.push({ pathname: "/host/[key]/pane/[id]", params: { key: target.host, id: String(target.pane) } });
        return;
      }
      // a "Needs you" alert: Yes / No answer it, anything else opens the question
      const question = () => router.push({ pathname: "/answer", params: { key: target.host, pane: String(target.pane) } });
      if (target.action === "open") return question();
      const action = target.action!;
      void (async () => {
        // a cold start: the settings, and with them the app lock, are read first
        if (!(await until(() => fleet.state.ready, 10_000))) return question();
        const auth = alertAuth(fleet.state.settings, appLock.getSnapshot().locked);
        // locked: the lock screen asks by itself; the answer waits for its unlock
        if (auth === "unlock" && !(await until(() => !appLock.getSnapshot().locked, 60_000))) return question();
        if (auth === "authenticate") {
          const r = await authenticate(`Answer ${action === "yes" ? "Yes" : "No"} on ${fleet.host(target.host)?.record.name ?? "your computer"}`);
          if (!r.ok) return question();
        }
        const r = await fleet.answerFromAlert(target.host, target.pane, target.ask!, action);
        // not sent (moved on, not connected): show what it asks now
        if (!r.ok) question();
      })();
    };
    const sub = Notifications.addNotificationResponseReceivedListener(open);
    open(Notifications.getLastNotificationResponse());
    return () => sub.remove();
  }, [router]);
}

/** Updates already offered in this app run (Later keeps Settings › About's Restart now). */
const offeredUpdates = new Set<string>();

/** Offers a restart once an over-the-air update has downloaded (ota.ts), after any unlock. */
function useUpdatePrompt() {
  const { isUpdatePending, downloadedUpdate } = Updates.useUpdates();
  const { locked } = useLock();
  useEffect(() => {
    if (!shouldOfferRestart(isUpdatePending, downloadedUpdate?.updateId, locked, offeredUpdates)) return;
    Alert.alert("Update ready", "Restart Rift to use the new version.", [
      { text: "Later", style: "cancel" },
      { text: "Restart", onPress: () => void Updates.reloadAsync() },
    ]);
  }, [isUpdatePending, downloadedUpdate?.updateId, locked]);
}

/**
 * What's New once per new release (an update or a new build): after the settings are read and
 * any unlock, the notes since the release last seen. A first launch only remembers the release.
 */
function useWhatsNew() {
  const router = useRouter();
  const { ready } = useApp();
  const { locked } = useLock();
  useEffect(() => {
    if (!ready || locked || whatsNewChecked) return;
    whatsNewChecked = true;
    void (async () => {
      const current = releaseKey(Constants.expoConfig?.version ?? "", APP_UPDATE);
      const seen = await loadSeenRelease(kv);
      if (seen === current) return;
      await saveSeenRelease(kv, current).catch(() => undefined);
      if (releasesSince(parseChangelog(CHANGELOG), seen, current).length > 0) {
        router.push({ pathname: "/whats-new", params: { seen: seen ?? "" } });
      }
    })();
  }, [ready, locked, router]);
}

/** Checked once per app run (kept across root remounts). */
let whatsNewChecked = false;

/** The splash plays once per app run (kept across root remounts). */
let splashPlayed = false;

export default function RootLayout() {
  useEffect(() => {
    void fleet.init();
    return startPlatformWatchers();
  }, []);
  useNotificationTaps();
  useUpdatePrompt();
  useWhatsNew();
  const [splash, setSplash] = useState(!splashPlayed);
  const splashDone = useCallback(() => {
    splashPlayed = true;
    setSplash(false);
  }, []);

  return (
    <SafeAreaProvider>
      <StatusBar style="light" />
      <ThemeProvider value={navTheme}>
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
            {/* Every page draws its own header, as the boards do (src/ui/Screen.tsx PageHeader). */}
            <Stack.Screen name="(tabs)" options={{ headerShown: false, title: "Now" }} />
            <Stack.Screen name="add" options={{ headerShown: false, title: "Paste a pairing link" }} />
            <Stack.Screen name="scan" options={{ headerShown: false, title: "Pair a computer", contentStyle: { backgroundColor: colors.camera } }} />
            <Stack.Screen name="pair" options={{ headerShown: false, title: "Pair" }} />
            <Stack.Screen name="host/[key]/index" options={{ headerShown: false, title: "Host" }} />
            {/* Back by the edge swipe only: iOS 26 takes a swipe anywhere as Back, which the pane's
                swipe between panes and its sideways-scrolling blocks need. */}
            <Stack.Screen name="host/[key]/pane/[id]" options={{ headerShown: false, title: "Pane", fullScreenGestureEnabled: false }} />
            <Stack.Screen name="control-keys" options={{ headerShown: false, title: "Keys" }} />
            <Stack.Screen name="answer" options={{ headerShown: false, title: "Answer" }} />
            <Stack.Screen
              name="run"
              options={{
                title: "Run a command",
                presentation: "formSheet",
                sheetAllowedDetents: [1],
                sheetGrabberVisible: true,
                sheetCornerRadius: 16,
                headerShown: false,
                contentStyle: { backgroundColor: colors.surface },
              }}
            />
            <Stack.Screen name="diagnostics" options={{ title: "Diagnostics" }} />
            <Stack.Screen name="changelog" options={{ headerShown: false, title: "What's New" }} />
            <Stack.Screen
              name="whats-new"
              options={{
                title: "What's New",
                presentation: "formSheet",
                sheetAllowedDetents: [0.75, 1],
                sheetGrabberVisible: true,
                sheetCornerRadius: 16,
                headerShown: false,
                contentStyle: { backgroundColor: colors.surface },
              }}
            />
            <Stack.Screen
              name="block"
              options={{
                title: "Command",
                presentation: "formSheet",
                sheetAllowedDetents: "fitToContents",
                sheetGrabberVisible: true,
                sheetCornerRadius: 16,
                headerShown: false,
                contentStyle: { backgroundColor: colors.surface },
              }}
            />
            <Stack.Screen
              name="chips"
              options={{
                title: "Prompt chip",
                presentation: "formSheet",
                sheetAllowedDetents: [0.75, 1],
                sheetGrabberVisible: true,
                sheetCornerRadius: 16,
                headerShown: false,
                contentStyle: { backgroundColor: colors.surface },
              }}
            />
            <Stack.Screen
              name="diff"
              options={{
                title: "Diff",
                presentation: "formSheet",
                sheetAllowedDetents: [1],
                sheetGrabberVisible: true,
                sheetCornerRadius: 16,
                headerShown: false,
                contentStyle: { backgroundColor: colors.surface },
              }}
            />
            <Stack.Screen
              name="confirm"
              options={{
                title: "Run command",
                presentation: "formSheet",
                sheetAllowedDetents: "fitToContents",
                sheetGrabberVisible: true,
                sheetCornerRadius: 16,
                headerShown: false,
                contentStyle: { backgroundColor: colors.surface },
              }}
            />
          </Stack>
        </LockGate>
      </ThemeProvider>
      {splash ? <Splash onDone={splashDone} /> : null}
    </SafeAreaProvider>
  );
}
