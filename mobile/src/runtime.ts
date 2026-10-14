// The app's Fleet, wired to the platform: expo-secure-store (keychain / keystore; memory only on
// web), expo-crypto randomness, the device name, local notifications, network changes
// (expo-network), the app state, and the app lock (expo-local-authentication; on Android also
// FLAG_SECURE through expo-screen-capture).

import Constants from "expo-constants";
import { getRandomBytes } from "expo-crypto";
import * as Device from "expo-device";
import * as LocalAuthentication from "expo-local-authentication";
import * as Network from "expo-network";
import * as Notifications from "expo-notifications";
import * as ScreenCapture from "expo-screen-capture";
import * as SecureStore from "expo-secure-store";
import { useSyncExternalStore } from "react";
import { AppState as RNAppState, Platform } from "react-native";
import { Connectivity, type NetworkSnapshot } from "./connectivity";
import { Fleet, type AppState, type Notifier } from "./fleet";
import { AppLock, authOutcome, biometricLabel, type AuthResult, type BiometricKind, type LockSnapshot } from "./lock";
import { liveActivities } from "../modules/rift-live";
import { livePlan, type LiveShown } from "./live";
import { ASK_CATEGORY, ASK_OPEN_CATEGORY, pushTokenProblem } from "./notify";
import { MemoryStore, type KeyValueStore } from "./stores";

/** Cryptographic randomness: expo-crypto (the OS generator), or Web Crypto. Never Math.random. */
export function randomBytes(n: number): Uint8Array {
  const c = (
    globalThis as {
      crypto?: { getRandomValues?: (a: Uint8Array) => Uint8Array };
    }
  ).crypto;
  if (Platform.OS === "web" && c?.getRandomValues) return c.getRandomValues(new Uint8Array(n));
  return getRandomBytes(n);
}

const secureStore: KeyValueStore = {
  get: (key) => SecureStore.getItemAsync(key),
  set: (key, value) => SecureStore.setItemAsync(key, value),
  delete: (key) => SecureStore.deleteItemAsync(key),
};

const notificationsSupported = Platform.OS === "ios" || Platform.OS === "android";

if (notificationsSupported) {
  // Shown in the foreground too: the app's own notifications already exclude the pane on screen;
  // a push alert about that pane is dropped here.
  Notifications.setNotificationHandler({
    handleNotification: async (n) => {
      const data = n.request.content.data as { host?: unknown; pane?: unknown } | undefined;
      const onScreen =
        typeof data?.host === "string" && typeof data.pane === "number" && fleet.isViewing(data.host, data.pane);
      return {
        shouldShowBanner: !onScreen,
        shouldShowList: !onScreen,
        shouldPlaySound: false,
        shouldSetBadge: false,
      };
    },
  });
}

/** The EAS project push tokens belong to (app.json `extra.eas.projectId`); null in a build without one. */
const pushProjectId: string | null =
  (Constants.expoConfig?.extra as { eas?: { projectId?: string } } | undefined)?.eas?.projectId ??
  Constants.easConfig?.projectId ??
  null;

/**
 * Asks the OS for this device's push token (once notifications are allowed) and gives it to the
 * fleet, which sends it to every computer; without one the app notifies only while it runs.
 */
async function refreshPushToken(): Promise<void> {
  if (!notificationsSupported) return;
  if (!(await Notifications.getPermissionsAsync()).granted) {
    fleet.setPushToken(null);
    return;
  }
  if (!pushProjectId) {
    fleet.setPushToken(null, "this build has no push project.");
    return;
  }
  try {
    fleet.setPushToken((await Notifications.getExpoPushTokenAsync({ projectId: pushProjectId })).data);
  } catch (e) {
    // the raw error (a Java stack line naming FirebaseApp and a docs URL) filled Settings: say
    // it plainly there and keep the detail in the log
    const detail = e instanceof Error ? e.message : String(e);
    console.warn(`push token: ${detail}`);
    fleet.setPushToken(null, pushTokenProblem(detail));
  }
}

if (notificationsSupported) {
  // "Needs you" alerts (the P6 Lock board): Yes and No answer from the lock screen once the
  // phone is unlocked (Face ID), and open the app, which sends the answer (and asks for Face ID
  // itself first with Ask before running on); Open shows the question.
  const open = { identifier: "open", buttonTitle: "Open", options: { opensAppToForeground: true } };
  const answer = (identifier: string, buttonTitle: string) => ({
    identifier,
    buttonTitle,
    options: { opensAppToForeground: true, isAuthenticationRequired: true },
  });
  void Notifications.setNotificationCategoryAsync(ASK_CATEGORY, [answer("yes", "Yes"), answer("no", "No"), open]).catch(() => undefined);
  void Notifications.setNotificationCategoryAsync(ASK_OPEN_CATEGORY, [open]).catch(() => undefined);
}

const notifier: Notifier = {
  async requestPermission() {
    if (!notificationsSupported) return false;
    const current = await Notifications.getPermissionsAsync();
    if (current.granted) return true;
    if (!current.canAskAgain) return false;
    const granted = (await Notifications.requestPermissionsAsync()).granted;
    if (granted) void refreshPushToken();
    return granted;
  },
  async notify({ title, body, data, category }) {
    if (!notificationsSupported) return;
    if (!(await Notifications.getPermissionsAsync()).granted) return;
    // Posted on this device (alerts while the app is closed come through push instead).
    await Notifications.scheduleNotificationAsync({
      content: { title, body: body || undefined, data, ...(category ? { categoryIdentifier: category } : {}) },
      trigger: null,
    });
  },
  async dismissAsk(host, pane, ask) {
    if (!notificationsSupported) return;
    for (const n of await Notifications.getPresentedNotificationsAsync()) {
      const data = n.request.content.data as { host?: unknown; pane?: unknown; ask?: unknown } | undefined;
      if (data?.host === host && data.pane === pane && data.ask === ask) await Notifications.dismissNotificationAsync(n.request.identifier);
    }
  },
};

export const fleet = new Fleet({
  kv: Platform.OS === "web" ? new MemoryStore() : secureStore,
  randomBytes,
  platform: Platform.OS,
  systemDeviceName: Device.deviceName ?? Device.modelName ?? "Phone",
  notifier,
});

/** The app state, re-rendering at most once per publish interval. */
export function useApp(): AppState {
  return useSyncExternalStore(fleet.store.subscribe, fleet.store.getSnapshot, fleet.store.getSnapshot);
}

// --- network and app state -----------------------------------------------------------------

export const connectivity = new Connectivity({
  suspend: (status) => fleet.suspendConnections(status),
  resume: (opts) => fleet.resumeConnections(opts),
  hasPendingNotify: () => fleet.hasPendingNotify,
});
fleet.onPendingNotifyChange(() => connectivity.notifyPendingChanged());

/** A network's identity: its type and, on a device, its address (Wi-Fi A → Wi-Fi B changes it). */
async function snapshotOf(state: Network.NetworkState): Promise<NetworkSnapshot> {
  const connected = state.isConnected !== false && state.type !== Network.NetworkStateType.NONE;
  let ip = "";
  // On the web expo-network asks a public service for the address: never call it there.
  if (connected && Platform.OS !== "web") {
    try {
      ip = await Network.getIpAddressAsync();
    } catch {
      // The type alone still tells cellular from Wi-Fi.
    }
  }
  return { connected, id: `${state.type ?? "?"}|${ip}` };
}

export const appLock = new AppLock();

let watching = false;

/** Starts the network, app-state and lock watchers (once, from the root layout). */
export function startPlatformWatchers(): () => void {
  if (watching) return () => undefined;
  watching = true;
  // Events are applied in arrival order even though each one awaits its address.
  let chain: Promise<void> = Promise.resolve();
  const feed = (state: Network.NetworkState) => {
    chain = chain.then(async () => connectivity.network(await snapshotOf(state))).catch(() => undefined);
  };
  void Network.getNetworkStateAsync().then(feed, () => undefined);
  const netSub = Network.addNetworkStateListener(feed);

  const onAppState = (s: string) => {
    fleet.setAppActive(s === "active");
    // `inactive` (Face ID prompt, Control Center, the first step to the background on iOS) is
    // still on screen; only `background` tells the computers to post about the followed pane.
    if (s === "active" || s === "background") fleet.setVisible(s === "active");
    if (s === "active") {
      appLock.enter();
      connectivity.appState("active");
    } else if (s === "background" || s === "inactive") {
      appLock.leave(s);
      connectivity.appState(s);
    }
  };
  const appSub = RNAppState.addEventListener("change", (s) => {
    onAppState(s);
    // Notifications may have been allowed or blocked in the system settings meanwhile.
    if (s === "active") void refreshPushToken();
  });
  void refreshPushToken();
  const tokenSub = notificationsSupported ? Notifications.addPushTokenListener(() => void refreshPushToken()) : null;
  // Android keeps the JavaScript running after Back closes the activity; when it is opened again
  // the root mounts after `active` was already sent, so start from the state it is in now (a
  // no-op on a first launch, which starts in the foreground).
  if (RNAppState.currentState === "active") onAppState("active");

  // The lock follows the setting once the settings are loaded (the first time locks at launch).
  let lockSetting: boolean | null = null;
  const syncLock = () => {
    const s = fleet.store.getSnapshot();
    if (!s.ready || s.settings.biometricLock === lockSetting) return;
    lockSetting = s.settings.biometricLock;
    appLock.setEnabled(lockSetting);
    // Android takes the Recents thumbnail at onPause, before the JS cover can render: with the
    // lock on, FLAG_SECURE blanks it (and blocks screenshots). iOS snapshots after `inactive`,
    // where the cover is already up, so screenshots stay allowed there.
    if (Platform.OS === "android") {
      void (
        lockSetting
          ? ScreenCapture.preventScreenCaptureAsync("app-lock")
          : ScreenCapture.allowScreenCaptureAsync("app-lock")
      ).catch(() => undefined);
    }
  };
  const unsubStore = fleet.store.subscribe(syncLock);
  syncLock();
  endLeftoverLive();
  const unsubLive = fleet.store.subscribe(syncLive);
  syncLive();

  return () => {
    netSub.remove();
    appSub.remove();
    tokenSub?.remove();
    unsubStore();
    unsubLive();
    watching = false;
  };
}

/** The Live Activities shown, by `liveKey` (live.ts). */
const liveShown = new Map<string, LiveShown>();

/** Starts, updates and ends the lock screen's Live Activities as the "Notify me" waits change (live.ts). */
function syncLive(): void {
  if (Platform.OS !== "ios" || !liveActivities.supported() || !liveListed) return;
  const s = fleet.store.getSnapshot();
  const plan = livePlan(liveShown, s.asked, s.hosts, s.activity);
  for (const st of plan.start) {
    const shown: LiveShown = { wait: st.wait, id: null, line: st.line };
    liveShown.set(st.key, shown);
    void liveActivities.start(st.command, st.place, st.startedAt, st.line).then((id) => {
      shown.id = id;
      // the wait ended while it started: end it now, with how it ended
      if (id && shown.ended) void liveActivities.end(id, shown.ended.exit, shown.line);
    });
  }
  for (const u of plan.update) {
    liveShown.get(u.key)!.line = u.line;
    void liveActivities.update(u.id, u.line);
  }
  for (const e of plan.end) {
    liveShown.get(e.key)!.ended = { exit: e.exit };
    liveShown.delete(e.key);
    if (e.id) void liveActivities.end(e.id, e.exit, e.line);
  }
}

/**
 * Ends the activities a previous run of the app left on the lock screen (it was closed or
 * killed before their command finished): nothing would ever update or end them.
 */
function endLeftoverLive(): void {
  // once per run of the JavaScript (the watchers restart with a remounted root)
  if (liveListing || Platform.OS !== "ios" || !liveActivities.supported()) return;
  liveListing = true;
  // this run starts none until they are listed, so every one listed is left over
  void liveActivities.ids().then((ids) => {
    for (const id of ids) void liveActivities.end(id, null, "");
    liveListed = true;
    syncLive();
  });
}

/** The leftover activities were ended (`endLeftoverLive`): this run's may start. */
let liveListed = false;
let liveListing = false;

export function useLock(): LockSnapshot {
  return useSyncExternalStore(appLock.subscribe, appLock.getSnapshot, appLock.getSnapshot);
}

// --- app lock ----------------------------------------------------------------------------------

export interface BiometricSupport {
  /** The switch label ("Require Face ID"); null when the lock is not available. */
  label: string | null;
  /** Why the lock is not available; null when it is. */
  reason: string | null;
}

const KIND: Record<number, BiometricKind> = {
  [LocalAuthentication.AuthenticationType.FINGERPRINT]: "fingerprint",
  [LocalAuthentication.AuthenticationType.FACIAL_RECOGNITION]: "face",
  [LocalAuthentication.AuthenticationType.IRIS]: "iris",
};

export async function biometricSupport(): Promise<BiometricSupport> {
  if (Platform.OS === "web") {
    return {
      label: null,
      reason: "The app lock needs Face ID or a fingerprint, which the web version cannot use.",
    };
  }
  // each platform's own names, as biometricLabel uses
  const ios = Platform.OS === "ios";
  try {
    if (!(await LocalAuthentication.hasHardwareAsync())) {
      return {
        label: null,
        reason: `This device has no ${ios ? "Face ID or Touch ID" : "face unlock or fingerprint sensor"}, so the app lock is not available.`,
      };
    }
    if (!(await LocalAuthentication.isEnrolledAsync())) {
      return {
        label: null,
        reason: `Set up ${ios ? "Face ID or Touch ID" : "face unlock or a fingerprint"} in the device's settings to use the app lock.`,
      };
    }
    const kinds = (await LocalAuthentication.supportedAuthenticationTypesAsync())
      .map((t) => KIND[t])
      .filter((k) => k !== undefined);
    const label = biometricLabel(kinds, Platform.OS) ?? "Require biometrics";
    return { label, reason: null };
  } catch {
    return {
      label: null,
      reason: "The app lock is not available on this device.",
    };
  }
}

export type { AuthResult };

/** Face ID / fingerprint, falling back to the device passcode. Outcomes: `authOutcome` (lock.ts). */
export async function authenticate(promptMessage: string): Promise<AuthResult> {
  if (Platform.OS === "web") return { ok: false, message: "Not available on the web." };
  try {
    const r = await LocalAuthentication.authenticateAsync({
      promptMessage,
      cancelLabel: "Cancel",
      disableDeviceFallback: false,
    });
    return r.success ? { ok: true } : authOutcome(r.error);
  } catch (e) {
    return { ok: false, message: e instanceof Error ? e.message : String(e) };
  }
}
