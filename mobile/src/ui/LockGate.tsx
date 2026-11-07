import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Keyboard, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { appLock, authenticate, biometricSupport, useLock } from "@/runtime";
import { Button } from "./Button";
import { ErrorText } from "./Notice";
import { Mark } from "./Splash";
import { ui, makeStyles } from "./theme";

/**
 * The app lock over everything (Settings › Security). While locked, the screens stay mounted
 * (connections and drafts survive) but are hidden from sight and from screen readers. While the
 * app is not in the foreground, a plain cover hides them too, so the app switcher snapshot
 * shows no terminal output.
 */
export function LockGate({ children }: { children: ReactNode }) {
  const styles = useStyles();
  const { locked, covered } = useLock();
  const hidden = locked || covered;
  // A field focused when the app locked keeps the keyboard up over the lock screen, and its keys
  // still type into the hidden screen; a sheet the lock closes hands focus back to the field under
  // it afterwards. While locked, no keyboard stays up.
  useEffect(() => {
    if (!locked) return;
    Keyboard.dismiss();
    const sub = Keyboard.addListener("keyboardDidShow", () => Keyboard.dismiss());
    return () => sub.remove();
  }, [locked]);
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
  const styles = useStyles();
  const insets = useSafeAreaInsets();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const asked = useRef(false);
  // what unlocks it, in the button's words ("Unlock with Face ID")
  const [bio, setBio] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void biometricSupport().then((r) => live && setBio(r.label?.replace(/^Require /, "") ?? null));
    return () => {
      live = false;
    };
  }, []);

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
      // the same room over and under, so the mark and its lines sit in the middle of the screen (the board)
      contentContainerStyle={[styles.lockContent, { paddingVertical: Math.max(insets.top, insets.bottom) + 24 }]}
      accessibilityViewIsModal
    >
      {/* the mark, one line, one button */}
      <View style={styles.mark}>
        <Mark />
      </View>
      <Text style={styles.title} accessibilityRole="header">
        Rift is locked
      </Text>
      <Text style={styles.text}>Your tabs stay connected.</Text>
      {error ? <ErrorText>{error}</ErrorText> : null}
      <Button
        label={busy ? "Unlocking…" : bio ? `Unlock with ${bio}` : "Unlock"}
        variant="primary"
        disabled={busy}
        accessibilityHint="Asks for Face ID, a fingerprint or the device passcode"
        onPress={() => void unlock()}
      />
    </ScrollView>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1 },
  cover: { ...StyleSheet.absoluteFill, backgroundColor: colors.bg },
  lock: { ...StyleSheet.absoluteFill, backgroundColor: colors.bg },
  // the New Pairing board: centred, 16 pt apart, Unlock hugging its label
  lockContent: { flexGrow: 1, justifyContent: "center", alignItems: "center", padding: 24, gap: 16, maxWidth: 480, width: "100%", alignSelf: "center" },
  mark: { alignItems: "center" },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 20, lineHeight: 26, textAlign: "center" },
  text: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20, textAlign: "center" },
}));
