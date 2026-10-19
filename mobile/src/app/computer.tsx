import { useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import { canOperate } from "@/fleet";
import { fleet, useApp } from "@/runtime";
import { osName } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { CardBox, Group, SettingsPage, SwitchRow, ValueRow } from "@/ui/Rows";
import { connWord, unpairFooter } from "@/ui/settingsText";
import { colors, ui } from "@/ui/theme";
import { useHostNotifications } from "@/ui/useHostNotifications";

/** Settings › a computer (the Settings board): how it connects, what this phone may do, its alerts, unpair. */
export default function ComputerScreen() {
  const router = useRouter();
  const { key } = useLocalSearchParams<{ key: string }>();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  // the first tap arms Unpair, the second runs it
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const notifications = useHostNotifications(key, setError);

  if (!host) {
    return (
      <SettingsPage title="Computer">
        <Text style={styles.empty}>{s.ready ? "This computer is not paired." : "Loading…"}</Text>
      </SettingsPage>
    );
  }

  const name = host.record.name;
  const connected = host.conn.status === "open";
  const os = host.welcome?.os ?? host.record.os;
  const operate = canOperate(host);
  const unpair = async () => {
    if (!armed) {
      setArmed(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await fleet.unpair(host.record.key);
      if (router.canGoBack()) router.back();
      else router.replace("/");
    } catch {
      setError("This phone could not remove the computer. Try Unpair again.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <SettingsPage title={name} notice={error ? <ErrorText style={styles.error}>{error}</ErrorText> : undefined}>
      <Group>
        <CardBox>
          <ValueRow first label="Connection" value={connWord(host.conn)} />
          <ValueRow label="System" value={os ? osName(os) : "Not known yet"} />
          <ValueRow label="This phone can" value={operate ? "Run commands" : "View only"} />
          <SwitchRow
            label="Notifications"
            detail="From this computer"
            value={host.record.notifications !== false}
            disabled={notifications.saving || busy}
            onChange={notifications.change}
          />
        </CardBox>
      </Group>

      <Group footer={unpairFooter(armed, name, connected)}>
        <CardBox>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={busy ? "Unpairing" : armed ? "Tap again to unpair" : `Unpair ${name}`}
            accessibilityHint={armed ? undefined : "Asks you to tap again"}
            accessibilityState={{ disabled: busy, busy }}
            disabled={busy}
            onPress={() => void unpair()}
            style={({ pressed }) => [styles.unpair, pressed && styles.pressed]}
          >
            {/* Destructive, but red text is below 4.5:1 on a surface (palette.ts): the caution token. */}
            <Text style={styles.unpairText}>{busy ? "Unpairing…" : armed ? "Tap again to unpair" : "Unpair this computer"}</Text>
          </Pressable>
        </CardBox>
      </Group>
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  error: { marginHorizontal: 16, marginBottom: 8 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15 },
  pressed: { opacity: 0.7 },
  unpair: { minHeight: 52, alignItems: "center", justifyContent: "center", paddingHorizontal: 14, paddingVertical: 8 },
  unpairText: { fontFamily: ui(400), color: colors.warning, fontSize: 15 },
});
