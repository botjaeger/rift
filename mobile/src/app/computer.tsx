import { useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Text } from "react-native";
import { canOperate } from "@/fleet";
import { fleet, useApp } from "@/runtime";
import { pairedText } from "@/ui/connStatus";
import { osName } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { CardBox, Group, NavRow, SettingsPage, SwitchRow, ValueRow } from "@/ui/Rows";
import { placeWord, unpairFooter } from "@/ui/settingsText";
import { ui, makeStyles } from "@/ui/theme";
import { useHostNotifications } from "@/ui/useHostNotifications";
import { useNow } from "@/ui/useNow";

/**
 * Settings › a computer (the Settings board): its system and when it was paired under the name,
 * where it is, what this phone may do, its theme, its alerts; Unpair in the error red.
 */
export default function ComputerScreen() {
  const styles = useStyles();
  const router = useRouter();
  const { key } = useLocalSearchParams<{ key: string }>();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  // the first tap arms Unpair, the second runs it
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const notifications = useHostNotifications(key, setError);
  // the year of "paired 3 Nov" (left out for this year): a day is plenty
  const now = useNow(24 * 3_600_000);

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
    <SettingsPage
      title={name}
      subtitle={[os ? osName(os) : null, pairedText(host.record.pairedAt, now)].filter(Boolean).join(" · ") || undefined}
      notice={error ? <ErrorText style={styles.error}>{error}</ErrorText> : undefined}
    >
      <Group title="This computer">
        <CardBox>
          <ValueRow first label="Connection" value={placeWord(host.conn)} />
          <ValueRow label="Access" value={operate ? "Can run commands" : "View only"} />
          {/* the theme it sends (Settings › Appearance chooses whether the app follows it) */}
          <NavRow
            label="Theme"
            value={host.theme?.name ?? "None sent yet"}
            hint="Opens Appearance"
            onPress={() => router.push("/appearance")}
          />
          <SwitchRow
            label="Notifications"
            detail="Finished, failed and questions"
            value={host.record.notifications !== false}
            disabled={notifications.saving || busy}
            onChange={notifications.change}
          />
        </CardBox>
      </Group>

      <Group footer={unpairFooter(armed, name, connected)}>
        <CardBox>
          {/* destructive: the error red (palette.ts holds it to 4.5:1 on a surface) */}
          <NavRow
            first
            destructive
            label={busy ? "Unpairing…" : armed ? "Tap again to unpair" : "Unpair this computer"}
            accessibilityLabel={busy ? "Unpairing" : armed ? "Tap again to unpair" : `Unpair ${name}`}
            hint={armed ? undefined : "Asks you to tap again"}
            disabled={busy}
            onPress={() => void unpair()}
          />
        </CardBox>
      </Group>
    </SettingsPage>
  );
}

const useStyles = makeStyles((colors) => ({
  error: { marginHorizontal: 16, marginBottom: 8 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15 },
}));
