import { useLocalSearchParams, useRouter } from "expo-router";
import { useRef, useState } from "react";
import { AccessibilityInfo, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import {
  EMPTY_WHAT,
  idleSince,
  seenWorking,
  sheetCounts,
  sheetRows,
  tabKey,
  tabKind,
  tabPane,
  tabRow,
  trackIdle,
  type RowContext,
  type SheetFilter,
  type TabRow,
} from "@/now";
import { fleet, useApp } from "@/runtime";
import { Tile } from "@/ui/Avatar";
import { Icon } from "@/ui/Icon";
import { canNewTab, openNewTab } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { toggleA11y } from "@/ui/toggle";
import { colors, mono, tint, tints, ui } from "@/ui/theme";
import { useNow } from "@/ui/useNow";

const FILTERS: { filter: SheetFilter; text: string }[] = [
  { filter: "all", text: "All" },
  { filter: "needs", text: "Needs you" },
  { filter: "working", text: "Working" },
  { filter: "failed", text: "Failed" },
];

/** The state pill: its text ("Needs you", "✕ 2 failed", "Working 3:56", "Idle 12m") and tints. */
const LOOK: Record<TabRow["kind"], { bg: string; fg: string; border: string }> = {
  needs: { bg: tints.warningBadge, fg: colors.warning, border: tint(colors.warning, 0x55) },
  working: { bg: tints.accentBadge, fg: colors.accent, border: colors.hairline },
  failed: { bg: tints.errorBadge, fg: colors.textBright, border: colors.hairline },
  idle: { bg: "#ffffff0f", fg: colors.textMuted, border: colors.hairline },
};

const pillText = (r: TabRow) => (r.kind === "needs" ? "Needs you" : r.kind === "failed" ? `✕ ${r.state}` : r.state);

/** The Tabs sheet (opened from a pane's tab strip): every tab of the computer, needs you first. */
export default function TabsSheet() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const { key, pane } = useLocalSearchParams<{ key: string; pane?: string }>();
  const host = s.hosts.find((h) => h.record.key === key);
  const [filter, setFilter] = useState<SheetFilter>("all");
  const [error, setError] = useState<string | null>(null);
  // one at a time: a second tap while the first is out would open two
  const out = useRef(false);
  const online = host?.conn.status === "open";
  const tabs = host && online ? host.windows.flatMap((w) => w.tabs) : [];
  const now = useNow(tabs.some((t) => tabKind(t) === "working") ? 1000 : 30_000);
  const kinds = tabs.map((t) => ({ key: tabKey(key, t), kind: tabKind(t) }));
  trackIdle(idleSince, seenWorking, kinds.filter((k) => k.kind === "working"), kinds.filter((k) => k.kind === "idle"), now);
  const ctx: RowContext = { now, multi: true, waitingSince: (h, p, w) => fleet.waitingSince(h, p, w), asked: s.asked };
  const rows = host ? tabs.map((t) => tabRow(host, t, ctx)) : [];
  const counts = sheetCounts(rows);
  const shown = sheetRows(rows, filter);
  const here = Number(pane);
  const path = host?.conn.path?.kind === "relay" ? "Relay" : "LAN";

  const close = () => (router.canGoBack() ? router.back() : router.replace("/"));
  const go = (r: TabRow) => {
    if (r.pane === null) return;
    router.dismiss();
    // the pane already open stays; another replaces it, so Back leads where it did
    if (r.tab.panes.some((p) => p.id === here)) return;
    router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(r.pane) } });
  };
  const newTab = async () => {
    if (!host || out.current) return;
    out.current = true;
    setError(null);
    try {
      const r = await openNewTab(host);
      if (!r.ok) return r.message ? setError(r.message) : undefined;
      AccessibilityInfo.announceForAccessibility(`Opened a tab on ${host.record.name}`);
      router.dismiss();
      if (r.pane !== undefined) router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(r.pane) } });
    } finally {
      out.current = false;
    }
  };
  const canOpen = !!host && canNewTab(host);

  return (
    // The sheet's own ScrollView, as its root: iOS lays out a form sheet's first scroll view as the
    // sheet's content, so a list nested under the header was drawn over it.
    <ScrollView style={styles.fill} contentContainerStyle={[styles.sheet, { paddingBottom: Math.max(insets.bottom, 30) }]}>
      <View style={styles.header}>
        <View style={styles.titles}>
          <Text style={styles.title} accessibilityRole="header">
            Tabs
          </Text>
          <View style={styles.subtitle}>
            <View style={[styles.dot, { backgroundColor: online ? colors.success : colors.textDim }]} />
            <Text style={styles.subtitleText} numberOfLines={1}>
              {host ? `${host.record.name}${online ? ` · ${path}` : " · offline"}` : "Computer not found"}
            </Text>
          </View>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Close"
          hitSlop={4}
          onPress={close}
          style={({ pressed }) => [styles.close, pressed && styles.pressed]}
        >
          <Icon name="close" size={16} color={colors.text} />
        </Pressable>
      </View>

      <View style={styles.filters} accessibilityRole="toolbar" accessibilityLabel="Show">
        {FILTERS.map((f) => {
          const on = filter === f.filter;
          const label = `${f.text} ${counts[f.filter]}`;
          return (
            <Pressable
              key={f.filter}
              {...toggleA11y(on)}
              accessibilityLabel={label}
              onPress={() => setFilter(f.filter)}
              style={({ pressed }) => [styles.chip, on && styles.chipOn, pressed && styles.pressed]}
            >
              <Text style={[styles.chipText, on && styles.chipTextOn]}>{label}</Text>
            </Pressable>
          );
        })}
      </View>

      <View style={styles.listBody} accessibilityRole="list">
        {shown.map((r) => {
          const isHere = r.tab.panes.some((p) => p.id === here);
          const look = LOOK[r.kind];
          return (
            <Pressable
              key={r.key}
              accessibilityRole="button"
              accessibilityLabel={[r.title, isHere ? "current tab" : null, pillText(r), r.line, `${r.panes} ${r.panes === 1 ? "pane" : "panes"}`]
                .filter(Boolean)
                .join(", ")}
              accessibilityState={{ selected: isHere }}
              disabled={tabPane(r.tab) === null}
              onPress={() => go(r)}
              style={({ pressed }) => [
                styles.card,
                { borderColor: isHere ? tint(colors.accent, 0x55) : r.kind === "needs" ? look.border : colors.hairline },
                isHere && { backgroundColor: tints.running },
                pressed && styles.pressed,
              ]}
            >
              <Tile agent={r.agent} size={38} radius={11} promptSize={13} />
              <View style={styles.cardText}>
                <View style={styles.cardTitleLine}>
                  <Text style={styles.cardTitle} numberOfLines={1}>
                    {r.title}
                  </Text>
                  {isHere ? <Text style={styles.here}>Here</Text> : null}
                </View>
                <Text style={styles.cardLine} numberOfLines={1}>
                  {r.line}
                </Text>
              </View>
              <View style={styles.cardSide}>
                <View style={[styles.pill, { backgroundColor: look.bg }]}>
                  <Text style={[styles.pillText, { color: look.fg }]} numberOfLines={1}>
                    {pillText(r)}
                  </Text>
                </View>
                <Text style={styles.panes}>
                  {r.panes} {r.panes === 1 ? "pane" : "panes"}
                </Text>
              </View>
            </Pressable>
          );
        })}
        {shown.length === 0 ? <Text style={styles.empty}>No tab is {EMPTY_WHAT[filter]}.</Text> : null}
      </View>

      {error ? <ErrorText>{error}</ErrorText> : null}
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="New tab"
        accessibilityState={{ disabled: !canOpen }}
        disabled={!canOpen}
        onPress={() => void newTab()}
        style={({ pressed }) => [styles.newTab, pressed && styles.pressed, !canOpen && styles.disabled]}
      >
        <Icon name="add" size={17} color={colors.text} />
        <Text style={styles.newTabText}>New tab</Text>
      </Pressable>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  fill: { flex: 1, backgroundColor: colors.surface },
  sheet: { paddingTop: 20, paddingHorizontal: 16, gap: 12 },
  header: { flexDirection: "row", alignItems: "center", gap: 10 },
  titles: { flex: 1, minWidth: 0, gap: 3 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 20 },
  subtitle: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  subtitleText: { flexShrink: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  close: { width: 36, height: 36, borderRadius: 18, backgroundColor: colors.glass, alignItems: "center", justifyContent: "center" },
  filters: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  chip: {
    height: 32,
    paddingHorizontal: 12,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.glassRim,
    justifyContent: "center",
  },
  chipOn: { backgroundColor: colors.glassRim, borderColor: "transparent" },
  chipText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  chipTextOn: { color: colors.textBright },
  listBody: { gap: 6 },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    paddingHorizontal: 12,
    minHeight: 64,
    borderRadius: 14,
    borderWidth: 1,
    backgroundColor: colors.bg,
  },
  cardText: { flex: 1, minWidth: 0, gap: 4 },
  cardTitleLine: { flexDirection: "row", alignItems: "center", gap: 6 },
  cardTitle: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  here: {
    fontFamily: ui(600),
    color: colors.accent,
    fontSize: 11,
    paddingHorizontal: 6,
    paddingVertical: 1,
    borderRadius: 4,
    backgroundColor: tints.accentBadge,
    overflow: "hidden",
  },
  cardLine: { fontFamily: mono, color: colors.textDim, fontSize: 12 },
  cardSide: { alignItems: "flex-end", gap: 4, flexShrink: 0 },
  pill: { height: 22, paddingHorizontal: 8, borderRadius: 11, justifyContent: "center" },
  pillText: { fontFamily: ui(600), fontSize: 12 },
  panes: { fontFamily: ui(400), color: colors.textDim, fontSize: 11 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingVertical: 16, paddingHorizontal: 4 },
  newTab: {
    height: 50,
    borderRadius: 25,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  newTabText: { fontFamily: ui(500), color: colors.text, fontSize: 15 },
});
