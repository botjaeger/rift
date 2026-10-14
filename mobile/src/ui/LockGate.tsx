import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { appLock, authenticate, useLock } from "@/runtime";
import { Button } from "./Button";
import { ErrorText } from "./Notice";
import { colors, ui } from "./theme";

/**
 * The app lock over everything (Settings › Security). While locked, the screens stay mounted
 * (connections and drafts survive) but are hidden from sight and from screen readers. While the
 * app is not in the foreground, a plain cover hides them too, so the app switcher snapshot
 * shows no terminal output.
 */
export function LockGate({ children }: { children: ReactNode }) {
  const { locked, covered } = useLock();
  const hidden = locked || covered;
  return (
    <View style={styles.fill}>
      <View
        style={styles.fill}
        accessibilityElementsHidden={hidden}
        importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
      >
        {children}
      </View>
      {locked ? <LockScreen covered={covered} /> : covered ? <View style={styles.cover} /> : null}
    </View>
  );
}

function LockScreen({ covered }: { covered: boolean }) {
  const insets = useSafeAreaInsets();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const asked = useRef(false);

  const unlock = useCallback(async () => {
    setBusy(true);
    setError(null);
    const r = await authenticate("Unlock Rift");
    setBusy(false);
    if (r.ok) appLock.unlock();
    else if (r.message !== "Cancelled.") setError(r.message);
  }, []);

  // Ask once by itself when the lock screen comes to the foreground; the button asks again.
  useEffect(() => {
    if (covered || asked.current) return;
    asked.current = true;
    void unlock();
  }, [covered, unlock]);

  return (
    <ScrollView
      style={styles.lock}
      contentContainerStyle={[styles.lockContent, { paddingTop: insets.top + 24, paddingBottom: insets.bottom + 24 }]}
      accessibilityViewIsModal
    >
      <Text style={styles.title} accessibilityRole="header">
        Rift is locked
      </Text>
      <Text style={styles.text}>Your terminal sessions stay hidden until you unlock.</Text>
      {error ? <ErrorText>{error}</ErrorText> : null}
      <Button
        label={busy ? "Unlocking…" : "Unlock"}
        variant="primary"
        disabled={busy}
        accessibilityHint="Asks for Face ID, a fingerprint or the device passcode"
        onPress={() => void unlock()}
      />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  cover: { ...StyleSheet.absoluteFill, backgroundColor: colors.bg },
  lock: { ...StyleSheet.absoluteFill, backgroundColor: colors.bg },
  lockContent: { flexGrow: 1, justifyContent: "center", padding: 24, gap: 16, maxWidth: 480, width: "100%", alignSelf: "center" },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 22 },
  text: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15, lineHeight: 21 },
});
