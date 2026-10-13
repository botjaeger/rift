import { useRouter } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";
import type { ActivityItem } from "@/fleet";
import { useApp } from "@/runtime";
import { formatDuration } from "@/ui/format";
import { TabScreen } from "@/ui/Screen";
import { colors, mono, radius, TOUCH } from "@/ui/theme";

function timeOf(at: number): string {
  const d = new Date(at);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function Row({ item }: { item: ActivityItem }) {
  const router = useRouter();
  const failed = item.exit !== null && item.exit !== 0;
  const result = item.exit === null ? "finished" : failed ? `failed, exit ${item.exit}` : "succeeded";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${item.command || "Command"} ${result} on ${item.hostName} after ${formatDuration(item.durationMs)}, at ${timeOf(item.at)}`}
      accessibilityHint="Opens the pane"
      onPress={() =>
        router.push({ pathname: "/host/[key]/pane/[id]", params: { key: item.host, id: String(item.pane) } })
      }
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
    >
      <View style={[styles.edge, { backgroundColor: failed ? colors.error : item.exit === 0 ? colors.success : colors.textDim }]} />
      <View style={styles.body}>
        <Text style={styles.command} numberOfLines={2}>
          {item.command || " "}
        </Text>
        <Text style={styles.meta}>
          {item.hostName} · {timeOf(item.at)}
        </Text>
      </View>
      <Text style={[styles.badge, { color: failed ? colors.textBright : item.exit === 0 ? colors.success : colors.textMuted }]}>
        {item.exit === null ? "done" : failed ? `✕ ${item.exit}` : "✓ 0"} · {formatDuration(item.durationMs)}
      </Text>
    </Pressable>
  );
}

export default function ActivityScreen() {
  const s = useApp();
  return (
    <TabScreen title="Activity">
      {s.activity.length === 0 ? (
        <Text style={styles.empty}>Commands that finish on your computers show up here while the app is connected.</Text>
      ) : (
        <View style={styles.list}>
          {s.activity.map((a) => (
            <Row key={a.id} item={a} />
          ))}
        </View>
      )}
    </TabScreen>
  );
}

const styles = StyleSheet.create({
  list: { gap: 4 },
  empty: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  row: {
    minHeight: TOUCH + 12,
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 10,
    backgroundColor: colors.surface,
    borderRadius: radius.control,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.hairline,
    overflow: "hidden",
    paddingRight: 12,
  },
  pressed: { opacity: 0.75 },
  edge: { width: 2, alignSelf: "stretch" },
  body: { flexGrow: 1, flexShrink: 1, flexBasis: 180, paddingVertical: 8, gap: 2 },
  command: { color: colors.textBright, fontFamily: mono, fontSize: 13 },
  meta: { color: colors.textMuted, fontSize: 12 },
  badge: { fontSize: 12, fontVariant: ["tabular-nums"], paddingVertical: 8 },
});
