import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { canClose, canMove, canOperate } from "@/fleet";
import { paneKind, paneSummary, type RowContext } from "@/now";
import { CLAUDE_COLOR, panesText, paneRows } from "@/paneView";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { Icon } from "@/ui/Icon";
import { LIST_ROW_H, ListRow } from "@/ui/ListRow";
import { canNewPane } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { colors, monoMedium, tints, ui } from "@/ui/theme";
import { ReorderRow, useReorder } from "@/ui/ReorderRow";
import { useClose } from "@/ui/useClose";
import { useMove } from "@/ui/useMove";
import { useNow } from "@/ui/useNow";
import { useToast } from "@/ui/useToast";

/**
 * The Panes page (a full page, opened from the pane row or a tab's "N panes"): the tab's panes as
 * the Tabs page's rows, each with its program's tile, name, what it does, its status mark and
 * since when; the open one on the accent tint. A tap opens that pane in place of this one; the
 * grip drags it to another place.
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
  const now = useNow(loc?.tab.panes.some((p) => paneKind(p) === "working") ? 1000 : 30_000);
  const ctx: RowContext = { now, multi: false, waitingSince: (h, p, w) => fleet.waitingSince(h, p, w), asked: s.asked };
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
          {loc ? <Text style={styles.count}>{`${panesText(rows.length)}${movable ? " · drag to reorder" : ""}`}</Text> : null}
        </View>
      </View>
      {toast}
      {error ? <ErrorText>{error}</ErrorText> : null}
      {loc ? (
        <View style={styles.list} accessibilityRole="list" accessibilityLabel={`Panes in ${tabTitle(loc.tab)}`}>
          {rows.map((r, i) => {
            const on = r.id === paneId;
            const p = loc.tab.panes[i]!;
            const sum = paneSummary(key, p, ctx);
            return (
              <ReorderRow key={r.id} grip index={i} count={rows.length} rowH={LIST_ROW_H} enabled={movable} drag={reorder.drag} setDrag={reorder.setDrag} onMove={onMove}>
                {(handle) => (
                  <ListRow
                    handle={handle}
                    movable={movable}
                    lead={
                      <View style={[styles.tile, { backgroundColor: r.claude ? `${CLAUDE_COLOR}1f` : tints.accentBadge }]}>
                        <Text style={[styles.mark, { color: r.claude ? CLAUDE_COLOR : colors.accent }]}>{r.mark}</Text>
                      </View>
                    }
                    title={r.name}
                    // the title names the program: the line says what it does
                    line={sum.activity}
                    kind={sum.kind}
                    when={sum.when}
                    label={`Pane ${r.number} of ${rows.length}: ${r.name}, ${sum.state}, ${sum.activity}`}
                    selected={on}
                    onPress={() => pick(r.id)}
                    onClose={closeable ? () => closePane(r.id, "pane") : undefined}
                    closeLabel={`Close pane ${r.name}`}
                  />
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
  // edge to edge, as the Tabs page's rows
  list: { marginHorizontal: -16, borderTopWidth: 1, borderTopColor: colors.hairline },
  tile: { width: 32, height: 32, borderRadius: 9, alignItems: "center", justifyContent: "center" },
  mark: { fontFamily: monoMedium, fontSize: 12 },
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
