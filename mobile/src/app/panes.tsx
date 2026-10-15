import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { CLAUDE_COLOR, panesText, paneRows, type StateTone } from "@/paneView";
import { useApp, useLock } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { colors, mono, monoMedium, tints, ui } from "@/ui/theme";

/** The state's text colour on the list (the current row's accent tint included). */
const TONE: Record<StateTone, string> = {
  running: colors.accent,
  needs: colors.warning,
  failed: colors.text,
  idle: colors.textMuted,
};

/**
 * The Panes sheet (the Pane board's picker): the tab's panes, each with its mark, name, last line
 * and state; the open one on the accent tint. A tap opens that pane in place of this one.
 */
export default function PanesScreen() {
  const { key, pane: paneParam } = useLocalSearchParams<{ key: string; pane: string }>();
  const paneId = Number(paneParam);
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const lock = useLock();

  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);
  const hidden = lock.covered || lock.locked;

  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const rows = loc ? paneRows(loc.tab) : [];

  // another pane takes this one's place: Back still leads where it led
  const pick = (id: number) => {
    router.dismiss();
    if (id !== paneId) router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(id) } });
  };

  return (
    // The sheet's own ScrollView, as its root: iOS lays out a form sheet's first scroll view as the sheet's
    // content, so one nested under the title was drawn over it.
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      contentContainerStyle={[styles.content, { paddingBottom: Math.max(30, insets.bottom + 2) }]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <View style={styles.head}>
        <Text style={styles.title} accessibilityRole="header">
          Panes
        </Text>
        {loc ? <Text style={styles.count}>{`${tabTitle(loc.tab)} · ${panesText(rows.length)}`}</Text> : null}
      </View>
      {loc ? (
        <View style={styles.list} accessibilityRole="list" accessibilityLabel={`Panes in ${tabTitle(loc.tab)}`}>
          {rows.map((r, i) => {
            const on = r.id === paneId;
            return (
              <Pressable
                key={r.id}
                accessibilityRole="button"
                accessibilityState={{ selected: on }}
                accessibilityLabel={`Pane ${r.number} of ${rows.length}: ${r.name}, ${r.state}${r.line ? `. ${r.line}` : ""}`}
                onPress={() => pick(r.id)}
                style={({ pressed }) => [styles.row, i < rows.length - 1 && styles.rule, on && styles.rowOn, pressed && styles.pressed]}
              >
                <View style={[styles.tile, { backgroundColor: r.claude ? `${CLAUDE_COLOR}1f` : tints.accentBadge }]}>
                  <Text style={[styles.mark, { color: r.claude ? CLAUDE_COLOR : colors.accent }]}>{r.mark}</Text>
                </View>
                <View style={styles.text}>
                  <Text style={styles.name} numberOfLines={1}>
                    {r.name}
                  </Text>
                  {/* textMuted on the open row: textDim is 4.3:1 on the accent tint (palette.ts) */}
                  <Text style={[styles.line, on && { color: colors.textMuted }]} numberOfLines={1}>
                    {r.line}
                  </Text>
                </View>
                <Text style={[styles.state, { color: TONE[r.tone] }]}>{r.state}</Text>
              </Pressable>
            );
          })}
        </View>
      ) : (
        <Text style={styles.gone}>This pane has closed.</Text>
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  content: { paddingTop: 20, paddingHorizontal: 16, gap: 12 },
  hidden: { opacity: 0 },
  pressed: { opacity: 0.7 },
  head: { flexDirection: "row", alignItems: "baseline", gap: 8 },
  title: { flex: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 20 },
  count: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  list: { borderRadius: 12, backgroundColor: colors.bg, overflow: "hidden" },
  row: { height: 56, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 14 },
  rule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  rowOn: { backgroundColor: tints.running },
  tile: { width: 32, height: 32, borderRadius: 9, alignItems: "center", justifyContent: "center" },
  mark: { fontFamily: monoMedium, fontSize: 12 },
  text: { flex: 1, minWidth: 0, gap: 2 },
  name: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  line: { fontFamily: mono, color: colors.textDim, fontSize: 12 },
  state: { fontFamily: ui(500), fontSize: 12 },
  gone: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
});
