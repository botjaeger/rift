import { useRouter } from "expo-router";
import { useRef, useState } from "react";
import { AccessibilityInfo, StyleSheet, Text, View } from "react-native";
import { canOperate, hasFeature, type HostView } from "@/fleet";
import { authenticate, fleet } from "@/runtime";
import { Button } from "./Button";
import { ErrorText } from "./Notice";
import { colors, ui } from "./theme";

/** Whether `host` takes a shell from this phone while no Rift window is open (feature `start`). */
export function canStartTab(host: HostView): boolean {
  return (
    host.conn.status === "open" &&
    hasFeature(host, "start") &&
    canOperate(host) &&
    host.windows.every((w) => w.background)
  );
}

/**
 * "Open a tab" on a computer where Rift is closed: Face ID (or the passcode) first, then a shell
 * the computer keeps until Rift opens and shows it as a tab; goes to the new pane.
 */
export function StartTab({ host }: { host: HostView }) {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // one at a time: a second tap while the first is out would open two
  const out = useRef(false);
  const name = host.record.name;
  const start = async () => {
    if (out.current) return;
    out.current = true;
    setBusy(true);
    setError(null);
    try {
      const auth = await authenticate(`Open a tab on ${name}`);
      if (!auth.ok) {
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
      const r = await fleet.startShell(host.record.key);
      if (!r.ok) return setError(r.message);
      AccessibilityInfo.announceForAccessibility("New tab opened");
      if (r.pane !== undefined)
        router.push({ pathname: "/host/[key]/pane/[id]", params: { key: host.record.key, id: String(r.pane) } });
    } finally {
      out.current = false;
      setBusy(false);
    }
  };
  const any = host.windows.some((w) => w.background && w.tabs.length > 0);
  return (
    <View style={styles.box}>
      <Text style={styles.note}>
        {any
          ? `Rift is closed on ${name}. These tabs keep running and appear when you open Rift.`
          : `Rift is closed on ${name}. Open a tab here; it appears in Rift when you open it.`}
      </Text>
      <Button
        label={any ? "Open another tab" : "Open a tab"}
        variant={any ? "secondary" : "primary"}
        disabled={busy}
        accessibilityHint={`Asks for Face ID, then starts a shell on ${name}`}
        onPress={() => void start()}
      />
      {error ? <ErrorText>{error}</ErrorText> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  box: { gap: 10, paddingHorizontal: 20, paddingTop: 14 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
});
