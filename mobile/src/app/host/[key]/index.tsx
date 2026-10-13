import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { ntfyUnsupported } from "@/fleet";
import { pathLabel } from "@/protocol/relay";
import { fleet, useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { HostTree } from "@/ui/HostTree";
import { osName } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { SwitchRow } from "@/ui/Rows";
import { Section } from "@/ui/Screen";
import { StatusLine } from "@/ui/StatusLine";
import { colors, mono, radius } from "@/ui/theme";

export default function HostScreen() {
  const router = useRouter();
  const { key } = useLocalSearchParams<{ key: string }>();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);

  if (!host) {
    return (
      <View style={[styles.fill, styles.content]}>
        <Stack.Screen options={{ title: "Host" }} />
        <Text style={styles.empty}>{s.ready ? "This computer is not paired." : "Loading…"}</Text>
      </View>
    );
  }

  const failed = host.conn.status === "failed";
  return (
    <ScrollView style={styles.fill} contentContainerStyle={styles.content}>
      <Stack.Screen options={{ title: host.record.name }} />
      <View style={styles.card}>
        <Text style={styles.name} accessibilityRole="header">
          {host.record.name}
        </Text>
        <Text style={styles.meta}>
          {[
            host.welcome ? `${osName(host.welcome.os)} · Rift ${host.welcome.version}` : null,
            host.record.permission === "view" ? "View only" : "Operate",
          ]
            .filter(Boolean)
            .join(" · ")}
        </Text>
        <Text style={styles.address}>
          {[
            ...host.record.addresses,
            ...(host.record.relay ? [pathLabel({ kind: "relay", url: host.record.relay })] : []),
          ].join(", ")}
        </Text>
        <StatusLine conn={host.conn} />
        {host.notice ? <ErrorText>{host.notice}</ErrorText> : null}
      </View>
      {failed && host.conn.failure === "auth" ? (
        <Button label="Pair again" variant="primary" onPress={() => router.push("/add")} />
      ) : failed || host.conn.status === "closed" ? (
        <Button label="Try again now" onPress={() => fleet.retry(host.record.key)} />
      ) : null}
      <HostTree
        host={host}
        onOpenPane={(id) =>
          router.push({
            pathname: "/host/[key]/pane/[id]",
            params: { key: host.record.key, id: String(id) },
          })
        }
      />
      <Section
        footer={
          (s.settings.notifyPolicy === "off"
            ? "Notifications are off in Settings › Notifications; only “Notify me” notifies."
            : "Finished commands on this computer, as Settings › Notifications says. Off: only “Notify me”.") +
          (s.settings.ntfyEnabled && ntfyUnsupported([host]).length > 0
            ? " Update Rift on this computer to get its alerts through ntfy."
            : "")
        }
      >
        <SwitchRow
          first
          label="Notifications"
          value={host.record.notifications !== false}
          onChange={(v) => void fleet.setHostNotifications(host.record.key, v)}
        />
      </Section>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: {
    padding: 16,
    gap: 16,
    maxWidth: 720,
    width: "100%",
    alignSelf: "center",
  },
  card: {
    padding: 16,
    gap: 4,
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  name: { color: colors.textBright, fontSize: 20, fontWeight: "600" },
  meta: { color: colors.textMuted, fontSize: 13 },
  address: { color: colors.textMuted, fontSize: 12, fontFamily: mono },
  empty: { color: colors.textMuted, fontSize: 14 },
});
