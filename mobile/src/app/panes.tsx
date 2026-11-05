import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { canClose, canMove, canOperate } from "@/fleet";
import { CLAUDE_COLOR, panesText, paneRows, type StateTone } from "@/paneView";
import { useApp, useLock } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { Icon } from "@/ui/Icon";
import { canNewPane } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { colors, mono, monoMedium, tints, ui } from "@/ui/theme";
import { ReorderRow, useReorder } from "@/ui/ReorderRow";
import { useClose } from "@/ui/useClose";
import { useMove } from "@/ui/useMove";
import { useToast } from "@/ui/useToast";

/** A row's height (`styles.row`): rows sit edge to edge. */
const ROW_H = 56;

/** The state's text colour on the list (the current row's accent tint included). */
const TONE: Record<StateTone, string> = {
  running: colors.accent,
  needs: colors.warning,
  failed: colors.text,
  idle: colors.textMuted,
};

/**
 * The Panes page (the TabPanes board; a full page, opened from the pane picker or a Switcher card):
 * the tab's panes, each with its mark, name, last line and state; the open one on the accent tint.
 * A tap opens that pane in place of this one; a long press drags it to another place.
 */
export default function PanesScreen() {
  const { key, pane: paneParam, from } = useLocalSearchParams<{ key: string; pane: string; from?: string }>();
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
  const [error, setError] = useState<string | null>(null);
  const { say, toast } = useToast();
  const closePane = useClose(key, paneId, setError, say, true);
  const closeable = canClose(host);
  // a long press lifts a row and drags it to another place; Move up / Move down do the same for a screen reader
  const reorder = useReorder();
  const move = useMove(key, setError, say);
  const movable = canMove(host) && rows.length > 1;
  const onMove = (from: number, to: number) => {
    const r = rows[from];
    if (r) move(r.id, "pane", to, r.name, to, rows.length);
  };

  // another pane takes this one's place: Back still leads where it led. Opened from the
  // Switcher, both pages close.
  const pick = (id: number) => {
    router.dismiss(from === "switcher" ? 2 : 1);
    if (id !== paneId) router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(id) } });
  };

  // the New pane sheet chooses what the pane starts with and opens it; both sheets close back here
  const newPane = () => router.push({ pathname: "/new-pane", params: { key, pane: String(paneId) } });
  const canOpen = !!host && canNewPane(host, paneId);

  return (
    // the page's ScrollView as its root (it was a form sheet's, where iOS takes the first scroll view
    // as the sheet's content)
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      scrollEnabled={reorder.scrollEnabled}
      contentContainerStyle={[styles.content, { paddingTop: insets.top + 8, paddingBottom: Math.max(30, insets.bottom + 2) }]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <View style={styles.head}>
        <Pressable accessibilityRole="button" accessibilityLabel="Back" hitSlop={4} onPress={() => router.back()} style={({ pressed }) => [styles.back, pressed && styles.pressed]}>
          <Icon name={from === "switcher" ? "back" : "chevronDown"} size={16} color={colors.text} />
        </Pressable>
        <View style={styles.titles}>
          <Text style={styles.title} accessibilityRole="header" numberOfLines={1}>
            {loc ? tabTitle(loc.tab) : "Panes"}
          </Text>
          {loc ? <Text style={styles.count}>{`${panesText(rows.length)}${movable ? " · hold a row, then drag it" : ""}`}</Text> : null}
        </View>
      </View>
      {toast}
      {error ? <ErrorText>{error}</ErrorText> : null}
      {loc ? (
        <View style={styles.list} accessibilityRole="list" accessibilityLabel={`Panes in ${tabTitle(loc.tab)}`}>
          {rows.map((r, i) => {
            const on = r.id === paneId;
            return (
              <ReorderRow key={r.id} index={i} count={rows.length} rowH={ROW_H} enabled={movable} drag={reorder.drag} setDrag={reorder.setDrag} onMove={onMove}>
              {(handle) => (
              <View style={[styles.row, i < rows.length - 1 && styles.rule, on && styles.rowOn]}>
              <Pressable
                accessibilityRole="button"
                accessibilityState={{ selected: on }}
                accessibilityLabel={`Pane ${r.number} of ${rows.length}: ${r.name}, ${r.state}${r.line ? `. ${r.line}` : ""}`}
                onPress={() => pick(r.id)}
                {...handle}
                style={({ pressed }) => [styles.rowPress, pressed && styles.pressed]}
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
              {closeable ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={`Close pane ${r.name}`}
                  onPress={() => closePane(r.id, "pane")}
                  style={({ pressed }) => [styles.rowClose, pressed && styles.pressed]}
                >
                  <Icon name="close" size={14} color={colors.textMuted} />
                </Pressable>
              ) : null}
              </View>
              )}
              </ReorderRow>
            );
          })}
        </View>
      ) : (
        <Text style={styles.gone}>This pane has closed.</Text>
      )}
      {/* a view-only device has no way to open a pane */}
      {host && loc && canOperate(host) ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`New pane in ${tabTitle(loc.tab)}`}
          accessibilityState={{ disabled: !canOpen }}
          disabled={!canOpen}
          onPress={newPane}
          style={({ pressed }) => [styles.newPane, pressed && styles.pressed, !canOpen && styles.disabled]}
        >
          <Icon name="add" size={17} color={colors.text} />
          <Text style={styles.newPaneText}>New pane</Text>
        </Pressable>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { paddingHorizontal: 16, gap: 14 },
  hidden: { opacity: 0 },
  pressed: { opacity: 0.7 },
  head: { flexDirection: "row", alignItems: "center", gap: 10, paddingBottom: 4 },
  back: { width: 44, height: 44, borderRadius: 22, borderWidth: 1, borderColor: colors.glassRim, backgroundColor: colors.glass, alignItems: "center", justifyContent: "center" },
  titles: { flex: 1, minWidth: 0, gap: 3 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 22 },
  count: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  list: { borderRadius: 16, borderWidth: 1, borderColor: colors.hairline, backgroundColor: colors.surface, overflow: "hidden" },
  row: { height: 56, flexDirection: "row", alignItems: "center" },
  rowPress: { flex: 1, minWidth: 0, height: 56, flexDirection: "row", alignItems: "center", gap: 12, paddingLeft: 14, paddingRight: 6 },
  // the row's ✕: a 44 pt target
  rowClose: { width: 44, height: 44, marginRight: 4, alignItems: "center", justifyContent: "center" },
  rule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  rowOn: { backgroundColor: tints.running },
  tile: { width: 32, height: 32, borderRadius: 9, alignItems: "center", justifyContent: "center" },
  mark: { fontFamily: monoMedium, fontSize: 12 },
  text: { flex: 1, minWidth: 0, gap: 2 },
  name: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  line: { fontFamily: mono, color: colors.textDim, fontSize: 12 },
  state: { fontFamily: ui(500), fontSize: 12 },
  disabled: { opacity: 0.45 },
  newPane: {
    height: 50,
    borderRadius: 25,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  newPaneText: { fontFamily: ui(500), color: colors.text, fontSize: 15 },
  gone: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
});
