import { Stack, useFocusEffect, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ntfyUnsupported } from "@/fleet";
import { fleet, useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { describeConn } from "@/ui/connLabel";
import { HostTree } from "@/ui/HostTree";
import { ErrorText } from "@/ui/Notice";
import { IconButton, PageHeader } from "@/ui/Screen";
import { colors, ui } from "@/ui/theme";

/** A computer's tabs (the Host board): each tab a card of its panes. */
export default function HostScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { key } = useLocalSearchParams<{ key: string }>();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);

  // on screen: a cold start comes back here
  useFocusEffect(
    useCallback(() => {
      if (fleet.state.hosts.some((h) => h.record.key === key)) void fleet.setLastView({ host: key, pane: null });
    }, [key]),
  );

  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));

  if (!host) {
    return (
      <View style={styles.fill}>
        <Stack.Screen options={{ headerShown: false }} />
        <PageHeader title="Host" back={back} backLabel="Back" />
        <Text style={styles.empty}>{s.ready ? "This computer is not paired." : "Loading…"}</Text>
      </View>
    );
  }

  const name = host.record.name;
  const conn = describeConn(host.conn);
  const failed = host.conn.status === "failed";
  const notifications = host.record.notifications !== false;
  const notes = [
    s.settings.ntfyEnabled && ntfyUnsupported([host]).length > 0
      ? "Update Rift on this computer to get its alerts through ntfy."
      : null,
    host.record.permission === "view" ? "View only: this phone can watch this computer but not run commands." : null,
  ].filter(Boolean);
  return (
    <View style={styles.fill}>
      <Stack.Screen options={{ headerShown: false }} />
      <PageHeader
        title={name}
        subtitle={conn.label}
        dot={conn.color}
        back={back}
        backLabel="Back"
        right={
          <IconButton
            icon={notifications ? "bell" : "bellOff"}
            color={colors.textMuted}
            label={`Notifications from ${name}: ${notifications ? "on" : "off"}`}
            hint={
              s.settings.notifyPolicy === "off"
                ? "Settings › Notify for is Nothing; only commands you asked about notify."
                : "Off: only commands you asked about notify, here and while the app is closed."
            }
            pressed={notifications}
            onPress={() => void fleet.setHostNotifications(host.record.key, !notifications)}
          />
        }
      />
      <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}>
        {host.notice || failed || host.conn.status === "closed" || notes.length > 0 ? (
          <View style={styles.status}>
            {host.notice ? <ErrorText>{host.notice}</ErrorText> : null}
            {notes.map((n) => (
              <Text key={n} style={styles.note}>
                {n}
              </Text>
            ))}
            {failed && host.conn.failure === "auth" ? (
              <Button label="Pair again" variant="primary" onPress={() => router.push("/scan")} />
            ) : failed || host.conn.status === "closed" ? (
              <Button label="Try again now" onPress={() => fleet.retry(host.record.key)} />
            ) : null}
          </View>
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
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.chrome },
  content: { maxWidth: 720, width: "100%", alignSelf: "center" },
  status: { gap: 10, paddingHorizontal: 20, paddingTop: 10 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, padding: 20 },
});
