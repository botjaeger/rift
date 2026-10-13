import { Pressable, StyleSheet, Text, View } from "react-native";
import type { HostView } from "@/fleet";
import type { Pane } from "@/protocol/types";
import { tabTitle } from "@/topology";
import { colors, mono, radius, TOUCH } from "./theme";

function ExitBadge({ exit }: { exit: number | null }) {
  if (exit === null) return null;
  const ok = exit === 0;
  return (
    <Text style={[styles.exit, { color: ok ? colors.success : colors.error }]} importantForAccessibility="no">
      {ok ? "✓ 0" : `✕ ${exit}`}
    </Text>
  );
}

function paneSpoken(pane: Pane, title: string): string {
  const parts = [`Pane ${title}`];
  if (pane.cwd) parts.push(`in ${pane.cwd}`);
  if (pane.job) parts.push(`running ${pane.job}`);
  if (pane.active) parts.push("active");
  if (pane.last_exit !== null) parts.push(pane.last_exit === 0 ? "last command succeeded" : `last command failed with exit ${pane.last_exit}`);
  return parts.join(", ");
}

function PaneRow({ pane, onOpen }: { pane: Pane; onOpen: (id: number) => void }) {
  const title = pane.last_command ?? pane.job ?? "(no command yet)";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={paneSpoken(pane, title)}
      accessibilityHint="Opens the pane's command blocks"
      onPress={() => onOpen(pane.id)}
      style={({ pressed }) => [styles.pane, pressed && styles.pressed]}
    >
      <View style={[styles.activeDot, { backgroundColor: pane.active ? colors.accent : "transparent" }]} />
      <View style={styles.paneText}>
        <Text style={styles.command} numberOfLines={1}>
          {title}
        </Text>
        {pane.cwd ? (
          <Text style={styles.cwd} numberOfLines={1}>
            {pane.cwd}
          </Text>
        ) : null}
      </View>
      {pane.job ? <Text style={styles.job}>running</Text> : <ExitBadge exit={pane.last_exit} />}
    </Pressable>
  );
}

/** A host's windows → tabs → panes. */
export function HostTree({ host, onOpenPane }: { host: HostView; onOpenPane: (id: number) => void }) {
  if (host.conn.status === "open" && host.windows.length === 0) {
    return <Text style={styles.empty}>No Rift windows are open on {host.record.name}.</Text>;
  }
  const many = host.windows.length > 1;
  return (
    <View style={styles.tree}>
      {host.windows.map((w, wi) => (
        <View key={w.pid} style={styles.window}>
          {many ? (
            <Text style={styles.section} accessibilityRole="header">
              Window {wi + 1}
            </Text>
          ) : null}
          {w.tabs.map((tab) => (
            <View key={tab.index} style={styles.tab}>
              <Text style={[styles.tabTitle, tab.active && styles.tabActive]} numberOfLines={1} accessibilityRole="header">
                {tabTitle(tab)}
              </Text>
              {tab.panes.map((p) => (
                <PaneRow key={p.id} pane={p} onOpen={onOpenPane} />
              ))}
            </View>
          ))}
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  tree: { gap: 12 },
  window: { gap: 8 },
  empty: { color: colors.textMuted, fontSize: 14 },
  section: { color: colors.textDim, fontSize: 12, fontWeight: "600", textTransform: "uppercase", letterSpacing: 0.5 },
  tab: {
    backgroundColor: colors.chrome,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  tabTitle: { color: colors.textMuted, fontSize: 13, fontWeight: "500", paddingHorizontal: 12, paddingVertical: 8 },
  tabActive: { color: colors.textBright },
  pane: {
    minHeight: TOUCH + 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  // Opacity, not a surface fill: the red exit badge needs the chrome ground (palette.ts).
  pressed: { opacity: 0.7 },
  activeDot: { width: 6, height: 6, borderRadius: 3 },
  paneText: { flex: 1, gap: 2 },
  command: { color: colors.text, fontFamily: mono, fontSize: 13 },
  cwd: { color: colors.textDim, fontFamily: mono, fontSize: 12 },
  job: { color: colors.accent, fontSize: 12 },
  exit: { fontSize: 12, fontVariant: ["tabular-nums"] },
});
