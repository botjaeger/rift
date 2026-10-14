import { Pressable, StyleSheet, Text, View } from "react-native";
import { tildePath } from "@/chips";
import type { HostView } from "@/fleet";
import type { Pane } from "@/protocol/types";
import { tabTitle } from "@/topology";
import { Icon } from "./Icon";
import { SectionTitle } from "./Screen";
import { colors, mono, radius, tints, ui } from "./theme";

/** A pane row's edge colour and badge (the Host board). */
function paneBadge(pane: Pane): { edge: string; text: string | null; color: string; bg: string } {
  if (pane.job) return { edge: colors.accent, text: "running", color: colors.accent, bg: tints.accentBadge };
  if (pane.last_exit === null) return { edge: colors.textDim, text: null, color: colors.textMuted, bg: "transparent" };
  if (pane.last_exit === 0) return { edge: colors.success, text: "✓ 0", color: colors.success, bg: tints.successBadge };
  return { edge: colors.error, text: `✕ ${pane.last_exit}`, color: colors.textBright, bg: tints.errorBadge };
}

function paneSpoken(pane: Pane, title: string): string {
  const parts = [`Pane ${title}`];
  if (pane.cwd) parts.push(`in ${tildePath(pane.cwd)}`);
  if (pane.git) parts.push(`on ${pane.git.branch}`);
  if (pane.job) parts.push(`running ${pane.job}`);
  if (pane.active) parts.push("active");
  if (pane.last_exit !== null && !pane.job)
    parts.push(pane.last_exit === 0 ? "last command succeeded" : `last command failed with exit ${pane.last_exit}`);
  return parts.join(", ");
}

function PaneRow({ pane, onOpen }: { pane: Pane; onOpen: (id: number) => void }) {
  const title = pane.last_command ?? pane.job ?? "(no command yet)";
  const b = paneBadge(pane);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={paneSpoken(pane, title)}
      accessibilityHint="Opens the pane"
      onPress={() => onOpen(pane.id)}
      style={({ pressed }) => [styles.pane, pressed && styles.pressed]}
    >
      <View style={[styles.edge, { backgroundColor: b.edge }]} />
      <View style={styles.paneText}>
        <Text style={styles.command} numberOfLines={1}>
          {title}
        </Text>
        {pane.cwd || pane.git ? (
          <View style={styles.where}>
            {pane.cwd ? (
              // cut at the start: the folder's own name is at the end of a long path
              <Text style={styles.cwd} numberOfLines={1} ellipsizeMode="head">
                {tildePath(pane.cwd)}
              </Text>
            ) : null}
            {pane.git ? (
              <Text style={styles.branch} numberOfLines={1}>
                {pane.git.branch}
              </Text>
            ) : null}
          </View>
        ) : null}
      </View>
      {b.text ? <Text style={[styles.badge, { color: b.color, backgroundColor: b.bg }]}>{b.text}</Text> : null}
      <Icon name="forward" size={14} color={colors.textMuted} />
    </Pressable>
  );
}

/** A host's tabs as cards of pane rows (the Host board), one group per window. */
export function HostTree({ host, onOpenPane }: { host: HostView; onOpenPane: (id: number) => void }) {
  if (host.conn.status === "open" && host.windows.length === 0) {
    return <Text style={styles.empty}>No Rift windows are open on {host.record.name}.</Text>;
  }
  const many = host.windows.length > 1;
  return (
    <View style={styles.tree}>
      {host.windows.map((w, wi) => (
        <View key={w.pid} style={styles.window}>
          <SectionTitle style={styles.title}>{many ? `Window ${wi + 1} · Tabs` : "Tabs"}</SectionTitle>
          <View style={styles.tabs}>
            {w.tabs.map((tab) => (
              <View key={tab.index} style={styles.tab}>
                <View style={styles.tabHead} accessible accessibilityRole="header">
                  <Text style={styles.tabTitle} numberOfLines={1}>
                    {tabTitle(tab)}
                  </Text>
                  {tab.active ? <Text style={styles.onScreen}>On screen</Text> : null}
                  <Text style={styles.count}>
                    {tab.panes.length} {tab.panes.length === 1 ? "pane" : "panes"}
                  </Text>
                </View>
                {tab.panes.map((p) => (
                  <PaneRow key={p.id} pane={p} onOpen={onOpenPane} />
                ))}
              </View>
            ))}
          </View>
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  tree: { gap: 18 },
  window: {},
  title: { marginTop: 10, marginBottom: 8, paddingHorizontal: 12 },
  tabs: { paddingHorizontal: 12, gap: 10 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingHorizontal: 20, paddingTop: 16 },
  tab: {
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  tabHead: { flexDirection: "row", alignItems: "center", gap: 10, paddingTop: 12, paddingBottom: 8, paddingHorizontal: 14 },
  tabTitle: { fontFamily: ui(600), flex: 1, minWidth: 0, color: colors.textBright, fontSize: 15 },
  onScreen: {
    fontFamily: ui(400),
    color: colors.accent,
    backgroundColor: tints.accentBadge,
    fontSize: 11,
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 10,
    overflow: "hidden",
  },
  count: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  pane: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  pressed: { opacity: 0.7 },
  edge: { width: 3, alignSelf: "stretch", borderRadius: 2 },
  paneText: { flex: 1, minWidth: 0, gap: 3 },
  command: { color: colors.textBright, fontFamily: mono, fontSize: 13 },
  where: { flexDirection: "row", gap: 8, overflow: "hidden" },
  cwd: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, flexShrink: 1 },
  // the path gives way first: a long path cut the branch down to its first letter
  branch: { fontFamily: ui(400), color: colors.success, fontSize: 12, flexShrink: 0, maxWidth: "45%" },
  badge: {
    fontFamily: mono,
    fontSize: 11,
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 10,
    overflow: "hidden",
  },
});
