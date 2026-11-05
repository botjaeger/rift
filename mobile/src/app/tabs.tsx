import { useActionScope } from "@/ui/useActionScope";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { dragLift } from "@/haptics";
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
import { canClose, canMove, canOperate } from "@/fleet";
import { paneRows, panesText } from "@/paneView";
import { fleet, useApp, useLock } from "@/runtime";
import { Tile } from "@/ui/Avatar";
import { Icon } from "@/ui/Icon";
import { canNewPane, canNewTab } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { toggleA11y } from "@/ui/toggle";
import { colors, mono, tint, tints, ui } from "@/ui/theme";
import { ReorderRow, useReorder } from "@/ui/ReorderRow";
import { useClose } from "@/ui/useClose";
import { useMove } from "@/ui/useMove";
import { useNow } from "@/ui/useNow";
import { useToast } from "@/ui/useToast";

/** An Arrange row's height and the gap under it (`styles.row`, `styles.rows`). */
const ROW_H = 64;
const ROW_GAP = 8;
/** The cards' gap and the page's side margin. */
const GAP = 10;
const SIDE = 16;

const FILTERS: { filter: SheetFilter; text: string }[] = [
  { filter: "all", text: "All" },
  { filter: "needs", text: "Needs you" },
  { filter: "working", text: "Working" },
  { filter: "failed", text: "Failed" },
];

/** The state pill: its text ("Needs you", "✕ 2 failed", "Working 3:56", "Idle 12m") and tints. */
const LOOK: Record<TabRow["kind"], { bg: string; fg: string; border: string }> = {
  needs: { bg: tints.warningBadge, fg: colors.warning, border: tint(colors.warning, 0x66) },
  working: { bg: tints.accentBadge, fg: colors.accent, border: colors.hairline },
  failed: { bg: tints.errorBadge, fg: colors.textBright, border: colors.hairline },
  idle: { bg: "#ffffff0f", fg: colors.textMuted, border: colors.hairline },
};

const pillText = (r: TabRow) => (r.kind === "needs" ? "Needs you" : r.kind === "failed" ? `✕ ${r.state}` : r.state);

/**
 * The Switcher (the Switcher boards; opened from a pane's tab bar): every tab of the computer as a
 * card with its panes' last lines, in the computer's tab order. A full page, not a sheet: nothing
 * here fights a drag. Arrange (or a long press on a card) lists the tabs to drag into another
 * order, pinned ones apart, with Close on each.
 */
export default function Switcher() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { width } = useWindowDimensions();
  const s = useApp();
  const { key, pane } = useLocalSearchParams<{ key: string; pane?: string }>();
  const scope = useActionScope(`${key}:${pane}`);
  const lock = useLock();

  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked && scope.leave()) router.back();
  }, [lock.locked, router, scope]);
  const hidden = lock.covered || lock.locked;
  const host = s.hosts.find((h) => h.record.key === key);
  const [filter, setFilter] = useState<SheetFilter>("all");
  const [arranging, setArranging] = useState(false);
  const [error, setError] = useState<string | null>(null);
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
  const paneCount = tabs.reduce((n, t) => n + t.panes.length, 0);

  const close = () => {
    if (scope.leave()) {
      if (router.canGoBack()) router.back();
      else router.replace("/");
    }
  };
  const go = (r: TabRow) => {
    if (r.pane === null) return;
    scope.leave();
    router.dismiss();
    // the pane already open stays; another replaces it, so Back leads where it did
    if (r.tab.panes.some((p) => p.id === here)) return;
    router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(r.pane) } });
  };
  // a tab of several panes: its own page, the split's panes in their order
  const openPanes = (r: TabRow) => {
    const first = r.tab.panes.find((p) => p.id === here) ?? r.tab.panes[0];
    if (first) router.push({ pathname: "/panes", params: { key, pane: String(first.id), from: "switcher" } });
  };
  const canOpen = !!host && canNewTab(host);
  const canSplit = !!host && canNewPane(host, here);
  const { say, toast } = useToast();
  const closeTab = useClose(key, here, setError, say, true);
  const closeable = canClose(host);
  // Arrange: a long press lifts a row and drags it among its own kind (pinned tabs stay first on
  // the computer); Move up / Move down do the same for a screen reader. It takes the computer's
  // place of the tab it is dropped on.
  // one drag state per list: a row's index counts within its own list, so a shared state lifted
  // the row at the same index in the other list too
  const pinnedDrag = useReorder();
  const othersDrag = useReorder();
  const move = useMove(key, setError, say);
  const movable = canMove(host) && rows.length > 1;
  const pinned = rows.filter((r) => r.tab.pinned);
  const others = rows.filter((r) => !r.tab.pinned);
  const moveIn = (list: TabRow[]) => (from: number, to: number) => {
    const r = list[from];
    const dest = list[to];
    if (r && r.pane !== null && dest) move(r.pane, "tab", dest.tab.index, r.title, to, list.length);
  };
  const startArrange = () => {
    if (!movable) return;
    dragLift();
    setFilter("all");
    setArranging(true);
  };
  const cardW = (width - SIDE * 2 - GAP) / 2;

  const row = (list: TabRow[], r: TabRow, i: number, reorder: ReturnType<typeof useReorder>) => {
    const isHere = r.tab.panes.some((p) => p.id === here);
    const look = LOOK[r.kind];
    return (
      <ReorderRow key={r.key} index={i} count={list.length} rowH={ROW_H + ROW_GAP} enabled={movable && r.pane !== null && list.length > 1} drag={reorder.drag} setDrag={reorder.setDrag} onMove={moveIn(list)}>
        {(handle) => (
          <View style={[styles.row, isHere && styles.rowHere]}>
            {/* not a button: a tap does nothing here; Move up / Move down are its actions */}
            <View
              accessible
              accessibilityLabel={[r.title, isHere ? "current tab" : null, pillText(r), panesText(r.panes)].filter(Boolean).join(", ")}
              accessibilityHint={list.length > 1 ? "Hold, then drag to move" : undefined}
              {...handle}
              style={styles.rowMain}
            >
              <Tile agent={r.agent} program={r.program} size={36} radius={10} promptSize={12} />
              <View style={styles.rowText}>
                <Text style={styles.rowTitle} numberOfLines={1}>
                  {r.title}
                </Text>
                <Text style={styles.rowLine} numberOfLines={1}>
                  {[r.line, r.panes > 1 ? panesText(r.panes) : null].filter(Boolean).join(" · ")}
                </Text>
              </View>
              <View style={[styles.pill, { backgroundColor: look.bg }]}>
                <Text style={[styles.pillText, { color: look.fg }]} numberOfLines={1}>
                  {pillText(r)}
                </Text>
              </View>
              <Icon name="grip" size={16} color={colors.textMuted} />
            </View>
            {closeable && r.pane !== null ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Close tab ${r.title}`}
                onPress={() => closeTab(r.pane!, "tab")}
                style={({ pressed }) => [styles.rowClose, pressed && styles.pressed]}
              >
                <Icon name="close" size={14} color={colors.textMuted} />
              </Pressable>
            ) : null}
          </View>
        )}
      </ReorderRow>
    );
  };

  const card = (r: TabRow) => {
    const isHere = r.tab.panes.some((p) => p.id === here);
    const look = LOOK[r.kind];
    const panes = paneRows(r.tab).slice(0, 4);
    return (
      <Pressable
        key={r.key}
        accessibilityRole="button"
        accessibilityLabel={[r.title, isHere ? "current tab" : null, r.tab.pinned ? "pinned" : null, pillText(r), panesText(r.panes)].filter(Boolean).join(", ")}
        accessibilityState={{ selected: isHere }}
        accessibilityHint={movable ? "Hold to arrange the tabs" : undefined}
        // the card is one element for a screen reader: its "N panes" link is an action on it
        accessibilityActions={r.panes > 1 ? [{ name: "panes", label: "Show the panes" }] : undefined}
        onAccessibilityAction={(e) => {
          if (e.nativeEvent.actionName === "panes") openPanes(r);
        }}
        disabled={tabPane(r.tab) === null}
        onPress={() => go(r)}
        onLongPress={movable ? startArrange : undefined}
        delayLongPress={450}
        style={({ pressed }) => [
          styles.card,
          { width: cardW, borderColor: isHere ? colors.accent : r.kind === "needs" ? look.border : colors.hairline },
          isHere && { backgroundColor: tints.running },
          pressed && styles.pressed,
        ]}
      >
        <View style={[styles.previews, panes.length > 2 && styles.previewsGrid]} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
          {panes.map((p) => (
            <View key={p.id} style={[styles.preview, panes.length === 2 && styles.previewHalf, panes.length > 2 && styles.previewQuarter]}>
              <Text style={styles.previewHead} numberOfLines={1}>
                <Text style={{ color: p.claude ? colors.warning : colors.accent }}>{p.mark === ">_" ? "❯" : p.mark}</Text> {p.name}
              </Text>
              <Text style={[styles.previewLine, p.tone === "needs" && { color: colors.warning }, p.tone === "failed" && { color: colors.error }]} numberOfLines={panes.length > 2 ? 1 : 3}>
                {p.line || p.state}
              </Text>
            </View>
          ))}
        </View>
        <View style={styles.cardFoot}>
          <View style={styles.cardTitleLine}>
            {r.tab.pinned ? <Icon name="pin" size={11} color={colors.textMuted} /> : null}
            <Text style={styles.cardTitle} numberOfLines={1}>
              {r.title}
            </Text>
            {isHere ? <Text style={styles.here}>Here</Text> : null}
          </View>
          <View style={styles.cardMeta}>
            <View style={[styles.pill, { backgroundColor: look.bg }]}>
              <Text style={[styles.pillText, { color: look.fg }]} numberOfLines={1}>
                {pillText(r)}
              </Text>
            </View>
            {r.panes > 1 ? (
              <Pressable accessibilityRole="button" accessibilityLabel={`${panesText(r.panes)} in ${r.title}`} hitSlop={12} onPress={() => openPanes(r)} style={({ pressed }) => [styles.panesLink, pressed && styles.pressed]}>
                <Text style={styles.panesLinkText}>{panesText(r.panes)}</Text>
              </Pressable>
            ) : (
              <Text style={styles.panesText}>1 pane</Text>
            )}
          </View>
        </View>
      </Pressable>
    );
  };

  return (
    <View style={[styles.fill, { paddingTop: insets.top }, hidden && styles.hidden]} accessibilityElementsHidden={hidden} importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}>
      {arranging ? (
        <View style={styles.header}>
          <Text style={[styles.title, styles.fillRow]} accessibilityRole="header">
            Arrange tabs
          </Text>
          <Pressable accessibilityRole="button" accessibilityLabel="Done" hitSlop={4} onPress={() => setArranging(false)} style={({ pressed }) => [styles.done, pressed && styles.pressed]}>
            <Text style={styles.doneText}>Done</Text>
          </Pressable>
        </View>
      ) : (
        <View style={styles.header}>
          <Pressable accessibilityRole="button" accessibilityLabel="Close" hitSlop={4} onPress={close} style={({ pressed }) => [styles.round, pressed && styles.pressed]}>
            <Icon name="chevronDown" size={16} color={colors.text} />
          </Pressable>
          <View style={styles.titles}>
            <Text style={styles.title} accessibilityRole="header" numberOfLines={1}>
              {host ? host.record.name : "Computer not found"}
            </Text>
            <View style={styles.subtitle}>
              <View style={[styles.dot, { backgroundColor: online ? colors.success : colors.textDim }]} />
              <Text style={styles.subtitleText} numberOfLines={1}>
                {host ? (online ? `${path} · ${rows.length} ${rows.length === 1 ? "tab" : "tabs"} · ${panesText(paneCount)}` : "offline") : ""}
              </Text>
            </View>
          </View>
          {movable ? (
            <Pressable accessibilityRole="button" accessibilityLabel="Arrange tabs" hitSlop={4} onPress={startArrange} style={({ pressed }) => [styles.outline, pressed && styles.pressed]}>
              <Icon name="grip" size={14} color={colors.text} />
              <Text style={styles.outlineText}>Arrange</Text>
            </Pressable>
          ) : null}
        </View>
      )}

      {arranging ? (
        <Text style={styles.hint}>Hold a row, then drag it. {host?.record.name ?? "The computer"}’s tab bar moves with it.</Text>
      ) : (
        <View style={styles.filters} accessibilityRole="toolbar" accessibilityLabel="Show">
          {FILTERS.map((f) => {
            const on = filter === f.filter;
            const label = `${f.text} ${counts[f.filter]}`;
            return (
              <Pressable key={f.filter} {...toggleA11y(on)} accessibilityLabel={label} onPress={() => setFilter(f.filter)} style={({ pressed }) => [styles.chip, on && styles.chipOn, pressed && styles.pressed]}>
                <Text style={[styles.chipText, on && styles.chipTextOn]}>{label}</Text>
              </Pressable>
            );
          })}
        </View>
      )}

      <ScrollView style={styles.fill} scrollEnabled={pinnedDrag.scrollEnabled && othersDrag.scrollEnabled} contentContainerStyle={styles.body}>
        {toast}
        {error ? <ErrorText>{error}</ErrorText> : null}
        {arranging ? (
          <>
            {pinned.length > 0 ? (
              <>
                <Text style={styles.cap} accessibilityRole="header">
                  Pinned
                </Text>
                <View style={styles.rows} accessibilityRole="list">
                  {pinned.map((r, i) => row(pinned, r, i, pinnedDrag))}
                </View>
                <Text style={styles.cap} accessibilityRole="header">
                  Tabs
                </Text>
              </>
            ) : null}
            <View style={styles.rows} accessibilityRole="list">
              {others.map((r, i) => row(others, r, i, othersDrag))}
            </View>
            {pinned.length > 0 ? <Text style={styles.note}>A pinned tab stays among the pinned ones, as on the computer.</Text> : null}
          </>
        ) : (
          <View style={styles.grid} accessibilityRole="list">
            {shown.map(card)}
            {shown.length === 0 ? <Text style={styles.empty}>No tab is {EMPTY_WHAT[filter]}.</Text> : null}
          </View>
        )}
      </ScrollView>

      {/* a view-only device has no way to open a tab */}
      {!arranging && host && canOperate(host) ? (
        <View style={[styles.footer, { paddingBottom: Math.max(insets.bottom, 16) }]}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="New tab"
            accessibilityState={{ disabled: !canOpen }}
            disabled={!canOpen}
            onPress={() => router.push({ pathname: "/new-tab", params: { key } })}
            style={({ pressed }) => [styles.footButton, pressed && styles.pressed, !canOpen && styles.disabled]}
          >
            <Icon name="add" size={16} color={colors.text} />
            <Text style={styles.footText}>New tab</Text>
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Split pane"
            accessibilityState={{ disabled: !canSplit }}
            disabled={!canSplit}
            onPress={() => router.push({ pathname: "/new-pane", params: { key, pane: String(here) } })}
            style={({ pressed }) => [styles.footButton, pressed && styles.pressed, !canSplit && styles.disabled]}
          >
            <Icon name="newPane" size={16} color={colors.text} />
            <Text style={styles.footText}>Split pane</Text>
          </Pressable>
        </View>
      ) : (
        <View style={{ height: Math.max(insets.bottom, 16) }} />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  fill: { flex: 1, backgroundColor: colors.bg },
  fillRow: { flex: 1 },
  hidden: { opacity: 0 },
  header: { flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: SIDE, paddingTop: 8, paddingBottom: 12 },
  round: { width: 44, height: 44, borderRadius: 22, borderWidth: 1, borderColor: colors.glassRim, backgroundColor: colors.glass, alignItems: "center", justifyContent: "center" },
  titles: { flex: 1, minWidth: 0, gap: 3 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 22 },
  subtitle: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  subtitleText: { flexShrink: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  outline: { height: 40, paddingHorizontal: 14, borderRadius: 20, borderWidth: 1, borderColor: colors.borderStrong, flexDirection: "row", alignItems: "center", gap: 6 },
  outlineText: { fontFamily: ui(500), color: colors.text, fontSize: 14 },
  done: { height: 40, paddingHorizontal: 18, borderRadius: 20, backgroundColor: colors.accent, justifyContent: "center" },
  doneText: { fontFamily: ui(600), color: colors.bg, fontSize: 14 },
  hint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 19, paddingHorizontal: SIDE, paddingBottom: 14 },
  filters: { flexDirection: "row", flexWrap: "wrap", gap: 6, paddingHorizontal: SIDE, paddingBottom: 14 },
  chip: { height: 32, paddingHorizontal: 12, borderRadius: 16, borderWidth: 1, borderColor: colors.glassRim, justifyContent: "center" },
  chipOn: { backgroundColor: colors.glassRim, borderColor: "transparent" },
  chipText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  chipTextOn: { color: colors.textBright },
  body: { paddingHorizontal: SIDE, paddingBottom: 16, gap: 10 },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: GAP },
  card: { borderRadius: 18, borderWidth: 1, backgroundColor: colors.surface, padding: 8, gap: 10 },
  previews: { height: 96, flexDirection: "row", gap: 4 },
  previewsGrid: { flexWrap: "wrap" },
  preview: { flex: 1, borderRadius: 10, backgroundColor: colors.bg, borderWidth: 1, borderColor: colors.hairline, padding: 7, gap: 3, overflow: "hidden" },
  previewHalf: { flexBasis: 0 },
  previewQuarter: { flexBasis: "45%", flexGrow: 1, height: 46, padding: 5 },
  previewHead: { fontFamily: mono, color: colors.text, fontSize: 10 },
  previewLine: { fontFamily: mono, color: colors.textMuted, fontSize: 10, lineHeight: 14 },
  cardFoot: { gap: 6, paddingHorizontal: 4, paddingBottom: 4 },
  cardTitleLine: { flexDirection: "row", alignItems: "center", gap: 5 },
  cardTitle: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  here: { fontFamily: ui(600), color: colors.accent, fontSize: 10, paddingHorizontal: 6, paddingVertical: 1, borderRadius: 4, backgroundColor: tints.accentBadge, overflow: "hidden" },
  cardMeta: { flexDirection: "row", alignItems: "center", gap: 6, flexWrap: "wrap" },
  pill: { height: 22, paddingHorizontal: 8, borderRadius: 11, justifyContent: "center", flexShrink: 1 },
  pillText: { fontFamily: ui(600), fontSize: 12 },
  panesText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 11 },
  panesLink: { minHeight: 22, justifyContent: "center" },
  panesLinkText: { fontFamily: ui(500), color: colors.accent, fontSize: 11 },
  cap: { fontFamily: ui(600), color: colors.textMuted, fontSize: 12, letterSpacing: 0.6, textTransform: "uppercase", paddingTop: 4 },
  rows: { gap: ROW_GAP },
  row: { height: ROW_H, flexDirection: "row", alignItems: "center", borderRadius: 16, borderWidth: 1, borderColor: colors.hairline, backgroundColor: colors.surface },
  rowHere: { borderColor: colors.accent },
  rowMain: { flex: 1, minWidth: 0, flexDirection: "row", alignItems: "center", gap: 12, paddingLeft: 12, paddingRight: 6 },
  rowText: { flex: 1, minWidth: 0, gap: 3 },
  rowTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  rowLine: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  rowClose: { width: 44, height: 44, marginRight: 4, alignItems: "center", justifyContent: "center" },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17, padding: 12, borderRadius: 14, backgroundColor: "#ffffff08" },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingVertical: 16, paddingHorizontal: 4 },
  footer: { flexDirection: "row", gap: GAP, paddingHorizontal: SIDE, paddingTop: 12, borderTopWidth: 1, borderTopColor: colors.hairline, backgroundColor: colors.chrome },
  footButton: { flex: 1, height: 50, borderRadius: 25, borderWidth: 1, borderColor: colors.borderStrong, flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8 },
  footText: { fontFamily: ui(500), color: colors.text, fontSize: 15 },
});
