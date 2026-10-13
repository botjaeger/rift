import { useRouter } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { fleet, useApp } from "@/runtime";
import type { HostView } from "@/fleet";
import { Button } from "@/ui/Button";
import { HostTree } from "@/ui/HostTree";
import { osName } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { TabScreen } from "@/ui/Screen";
import { StatusLine } from "@/ui/StatusLine";
import { colors, mono, radius, TOUCH } from "@/ui/theme";

function HostCard({ host }: { host: HostView }) {
  const router = useRouter();
  const key = host.record.key;
  const failed = host.conn.status === "failed";
  return (
    <View style={styles.card}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${host.record.name}${host.record.permission === "view" ? ", view only" : ""}`}
        accessibilityHint="Opens this computer"
        onPress={() => router.push({ pathname: "/host/[key]", params: { key } })}
        style={({ pressed }) => [styles.hostHeader, pressed && styles.pressed]}
      >
        <View style={styles.hostTitleRow}>
          <Text style={styles.hostName}>{host.record.name}</Text>
          {host.record.permission === "view" ? <Text style={styles.badge}>View only</Text> : null}
        </View>
        {host.welcome ? (
          <Text style={styles.hostMeta}>
            {osName(host.welcome.os)} · Rift {host.welcome.version}
          </Text>
        ) : null}
      </Pressable>
      {/* Outside the button, so its live region is announced on its own. */}
      <View style={styles.status}>
        <StatusLine conn={host.conn} />
        {host.notice ? <ErrorText>{host.notice}</ErrorText> : null}
      </View>
      {failed && host.conn.failure !== "auth" ? (
        <Button label="Try again" onPress={() => fleet.retry(key)} style={styles.inset} />
      ) : null}
      {failed && host.conn.failure === "auth" ? (
        <Button label="Pair again" onPress={() => router.push("/add")} style={styles.inset} />
      ) : null}
      {host.windows.length > 0 || host.conn.status === "open" ? (
        <View style={styles.inset}>
          <HostTree
            host={host}
            onOpenPane={(id) =>
              router.push({
                pathname: "/host/[key]/pane/[id]",
                params: { key, id: String(id) },
              })
            }
          />
        </View>
      ) : null}
    </View>
  );
}

export default function HostsScreen() {
  const router = useRouter();
  const s = useApp();
  const pairButton = (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Pair a desktop"
      onPress={() => router.push("/add")}
      style={({ pressed }) => [styles.pairButton, pressed && styles.pressed]}
    >
      <Text style={styles.pairText}>Pair</Text>
    </Pressable>
  );

  return (
    <TabScreen title="Hosts" accessory={s.hosts.length > 0 ? pairButton : undefined}>
      {s.loadError ? (
        <View style={styles.loadError}>
          <ErrorText>
            Could not read this device&apos;s keys: {s.loadError}. Unlock the device, then try again.
          </ErrorText>
          <Button label="Try again" variant="primary" onPress={() => void fleet.retryInit()} />
        </View>
      ) : null}
      {s.ready && !s.loadError && s.hosts.length === 0 ? (
        <View style={styles.emptyState}>
          <Text style={styles.emptyTitle}>No paired desktops</Text>
          <Text style={styles.emptyText}>
            On your computer, open Rift Settings › Remote access › Pair a phone (or run{" "}
            <Text style={styles.code}>rift-terminal remote pair</Text>), then scan the code it shows.
          </Text>
          <Button label="Pair a desktop" variant="primary" onPress={() => router.push("/add")} />
        </View>
      ) : null}
      {s.hosts.map((h) => (
        <HostCard key={h.record.key} host={h} />
      ))}
    </TabScreen>
  );
}

const styles = StyleSheet.create({
  card: {
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    paddingBottom: 12,
    gap: 12,
    overflow: "hidden",
  },
  hostHeader: {
    paddingHorizontal: 16,
    paddingTop: 14,
    gap: 2,
    minHeight: TOUCH,
  },
  status: { paddingHorizontal: 16, gap: 6 },
  hostTitleRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 8,
  },
  hostName: {
    color: colors.textBright,
    fontSize: 20,
    fontWeight: "600",
    flexShrink: 1,
  },
  badge: {
    color: colors.warning,
    fontSize: 11,
    fontWeight: "600",
    borderWidth: 1,
    borderColor: colors.warning,
    borderRadius: radius.control,
    paddingHorizontal: 6,
    paddingVertical: 1,
  },
  hostMeta: { color: colors.textMuted, fontSize: 13 },
  inset: { marginHorizontal: 12 },
  pressed: { opacity: 0.75 },
  pairButton: {
    minHeight: TOUCH,
    minWidth: TOUCH,
    paddingHorizontal: 12,
    justifyContent: "center",
    alignItems: "center",
  },
  pairText: { color: colors.accent, fontSize: 17, fontWeight: "600" },
  loadError: { gap: 12 },
  emptyState: { gap: 12, paddingVertical: 24 },
  emptyTitle: { color: colors.textBright, fontSize: 17, fontWeight: "600" },
  emptyText: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  code: { fontFamily: mono, color: colors.text },
});
