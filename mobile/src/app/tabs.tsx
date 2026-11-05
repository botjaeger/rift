import { useActionScope } from "@/ui/useActionScope";
import { useLocalSearchParams, useRouter } from "expo-router";
import { memo, useEffect, useRef, useState, type RefObject } from "react";
import { FlatList, Pressable, ScrollView, StyleSheet, Text, TextInput, View, type ViewProps } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { EMPTY_WHAT, idleSince, seenWorking, sheetCounts, sameTabRow, sheetRows, tabKey, tabKind, tabRow, trackIdle, type RowContext, type SheetFilter, type TabRow } from "@/now";
import { canClose, canMove, canOperate } from "@/fleet";
import { panesText } from "@/paneView";
import { fleet, useApp, useLock } from "@/runtime";
import { Icon } from "@/ui/Icon";
import { LIST_ROW_H, ListRow } from "@/ui/ListRow";
import { canNewPane, canNewTab } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { toggleA11y } from "@/ui/toggle";
import { colors, ui } from "@/ui/theme";
import { ReorderRow, useReorder, type Dragging } from "@/ui/ReorderRow";
import { useClose } from "@/ui/useClose";
import { useMove } from "@/ui/useMove";
import { useNow } from "@/ui/useNow";
import { useToast } from "@/ui/useToast";

const SIDE = 16;

const FILTERS: { filter: SheetFilter; text: string }[] = [
  { filter: "all", text: "All" },
  { filter: "needs", text: "Needs you" },
  { filter: "working", text: "Working" },
  { filter: "failed", text: "Failed" },
];

/** Whether a row answers the search: its tab's name, what its lead pane does, or any pane's folder. */
export function matchesSearch(r: TabRow, q: string): boolean {
  const s = q.trim().toLowerCase();
  if (!s) return true;
  return [r.title, r.line, ...r.tab.panes.map((p) => p.cwd ?? "")].some((t) => t.toLowerCase().includes(s));
}

/**
 * The Tabs page (opened from the pane's tab row): every tab of the computer as a row, in the
 * computer's tab order, pinned ones apart and first as on its tab bar. Each row: the grip that
 * drags it, the tab's name, what its lead pane does, its status mark and since when; ✕ closes it.
 * Search and the state chips narrow the list; the grips show only on the whole list, whose order
 * is the computer's.
 */
export default function Switcher() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
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
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const online = host?.conn.status === "open";
  const tabs = host && online ? host.windows.flatMap((w) => w.tabs) : [];
  const now = useNow(tabs.some((t) => tabKind(t) === "working") ? 1000 : 30_000);
  const kinds = tabs.map((t) => ({ key: tabKey(key, t), kind: tabKind(t) }));
  trackIdle(idleSince, seenWorking, kinds.filter((k) => k.kind === "working"), kinds.filter((k) => k.kind === "idle"), now);
  const ctx: RowContext = { now, multi: false, waitingSince: (h, p, w) => fleet.waitingSince(h, p, w), asked: s.asked };
  const rows = host ? tabs.map((t) => tabRow(host, t, ctx)) : [];
  const counts = sheetCounts(rows);
  const shown = sheetRows(rows, filter).filter((r) => matchesSearch(r, query));
  const here = Number(pane);

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
  const operate = !!host && canOperate(host);
  const canOpen = !!host && canNewTab(host);
  const canSplit = !!host && canNewPane(host, here);
  const { say, toast } = useToast();
  const closeTab = useClose(key, here, setError, say, true);
  const closeable = canClose(host);
  // the grip drags a row among its own kind (pinned tabs stay first on the computer); Move up /
  // Move down do the same for a screen reader. It takes the computer's place of the tab it is
  // dropped on. One drag state per list: a row's index counts within its own list.
  const pinnedDrag = useReorder();
  const othersDrag = useReorder();
  const move = useMove(key, setError, say);
  // only the whole list is in the computer's order
  const whole = filter === "all" && query.trim() === "";
  const movable = canMove(host) && whole && rows.length > 1;
  const pinned = shown.filter((r) => r.tab.pinned);
  const others = shown.filter((r) => !r.tab.pinned);
  const moveIn = (list: TabRow[]) => (from: number, to: number) => {
    const r = list[from];
    const dest = list[to];
    if (r && r.pane !== null && dest) move(r.pane, "tab", dest.tab.index, r.title, to, list.length);
  };

  // the rows' actions, read when used: the memoized rows keep the first callbacks they got
  const acts = useRef<RowActs>(null!);
  useEffect(() => {
    acts.current = {
      row: (k) => rows.find((r) => r.key === k),
      go,
      openPanes,
      close: (pane) => closeTab(pane, "tab"),
      move: (list, from, to) => moveIn(list === "pinned" ? pinned : others)(from, to),
    };
  });
  // the list's items: each section's caption, then its rows
  const items: ListItem[] = [];
  if (pinned.length > 0) {
    items.push({ kind: "cap", key: "cap:pinned", text: "Pinned", hint: false });
    pinned.forEach((r, i) => items.push({ kind: "row", key: r.key, r, list: "pinned", index: i, lifted: pinnedDrag.drag.active === i }));
  }
  if (others.length > 0) {
    items.push({ kind: "cap", key: "cap:others", text: pinned.length > 0 ? "Tabs" : `${shown.length} ${shown.length === 1 ? "tab" : "tabs"}`, hint: movable });
    others.forEach((r, i) => items.push({ kind: "row", key: r.key, r, list: "others", index: i, lifted: othersDrag.drag.active === i }));
  }

  const row = (list: TabRow[], name: ListName, r: TabRow, i: number, reorder: ReturnType<typeof useReorder>) => (
    <TabsRow
      key={r.key}
      r={r}
      list={name}
      index={i}
      count={list.length}
      canDrag={movable && r.pane !== null && list.length > 1}
      isHere={r.tab.panes.some((p) => p.id === here)}
      closeable={closeable}
      drag={reorder.drag}
      setDrag={reorder.setDrag}
      acts={acts}
    />
  );

  return (
    <View style={[styles.fill, { paddingTop: insets.top }, hidden && styles.hidden]} accessibilityElementsHidden={hidden} importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}>
      <View style={styles.header}>
        <View style={styles.titles}>
          <Text style={styles.title} accessibilityRole="header">
            Tabs
          </Text>
          <View style={styles.subtitle}>
            <View style={[styles.dot, { backgroundColor: online ? colors.success : colors.textDim }]} />
            <Text style={styles.subtitleText} numberOfLines={1}>
              {host ? `${host.record.name} · ${online ? `${rows.length} ${rows.length === 1 ? "tab" : "tabs"}` : "offline"}` : "Computer not found"}
            </Text>
          </View>
        </View>
        {/* a view-only device has no way to open a tab or a pane */}
        {operate ? (
          <>
            <Pressable accessibilityRole="button" accessibilityLabel="Split pane" accessibilityState={{ disabled: !canSplit }} disabled={!canSplit} hitSlop={4} onPress={() => router.push({ pathname: "/new-pane", params: { key, pane: String(here) } })} style={({ pressed }) => [styles.round, pressed && styles.pressed, !canSplit && styles.disabled]}>
              <Icon name="newPane" size={16} color={colors.text} />
            </Pressable>
            <Pressable accessibilityRole="button" accessibilityLabel="New tab" accessibilityState={{ disabled: !canOpen }} disabled={!canOpen} hitSlop={4} onPress={() => router.push({ pathname: "/new-tab", params: { key } })} style={({ pressed }) => [styles.round, pressed && styles.pressed, !canOpen && styles.disabled]}>
              <Icon name="add" size={16} color={colors.text} />
            </Pressable>
          </>
        ) : null}
        <Pressable accessibilityRole="button" accessibilityLabel="Close" hitSlop={4} onPress={close} style={({ pressed }) => [styles.round, pressed && styles.pressed]}>
          <Icon name="close" size={15} color={colors.text} />
        </Pressable>
      </View>

      <View style={styles.search}>
        <Icon name="search" size={15} color={colors.textMuted} />
        <TextInput
          accessibilityLabel="Search tabs"
          placeholder="Search by name, command or folder"
          placeholderTextColor={colors.textMuted}
          value={query}
          onChangeText={setQuery}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
          clearButtonMode="while-editing"
          style={styles.searchInput}
        />
      </View>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.filtersBox} contentContainerStyle={styles.filters} accessibilityRole="toolbar" accessibilityLabel="Show" keyboardShouldPersistTaps="handled">
        {FILTERS.map((f) => {
          const on = filter === f.filter;
          const label = `${f.text} ${counts[f.filter]}`;
          return (
            <Pressable key={f.filter} {...toggleA11y(on)} accessibilityLabel={label} onPress={() => setFilter(f.filter)} style={({ pressed }) => [styles.chip, on && styles.chipOn, pressed && styles.pressed]}>
              <Text style={[styles.chipText, on && styles.chipTextOn]}>{label}</Text>
            </Pressable>
          );
        })}
      </ScrollView>

      {/* virtualized: a computer of a hundred tabs draws the screenful in view, not every row */}
      <FlatList
        style={styles.fill}
        data={items}
        keyExtractor={(it) => it.key}
        renderItem={({ item }) =>
          item.kind === "cap" ? (
            <Text style={styles.cap} accessibilityRole="header">
              {item.text}
              {item.hint ? <Text style={styles.capHint}>{"  drag to reorder"}</Text> : null}
            </Text>
          ) : item.list === "pinned" ? (
            row(pinned, "pinned", item.r, item.index, pinnedDrag)
          ) : (
            row(others, "others", item.r, item.index, othersDrag)
          )
        }
        // the lifted row draws above the cells after it (each cell is a sibling of the others)
        CellRendererComponent={LiftCell}
        extraData={[pinnedDrag.drag, othersDrag.drag, movable, closeable, here]}
        initialNumToRender={14}
        scrollEnabled={pinnedDrag.scrollEnabled && othersDrag.scrollEnabled}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={{ paddingBottom: Math.max(insets.bottom, 16) }}
        ListHeaderComponent={
          <>
            {toast}
            {error ? (
              <View style={styles.pad}>
                <ErrorText>{error}</ErrorText>
              </View>
            ) : null}
          </>
        }
        ListEmptyComponent={<Text style={styles.empty}>{query.trim() ? `No tab matches “${query.trim()}”.` : `No tab is ${EMPTY_WHAT[filter]}.`}</Text>}
      />
    </View>
  );
}

type ListName = "pinned" | "others";

type ListItem =
  | { kind: "cap"; key: string; text: string; hint: boolean }
  | { kind: "row"; key: string; r: TabRow; list: ListName; index: number; lifted: boolean };

/**
 * The list's cell: the cell of the row being dragged draws above the cells after it (each cell is
 * a sibling of the others, so the row's own zIndex stays inside its cell). A component of its own:
 * FlatList would remount every cell for a new function each render. Never flattened and always
 * with a zIndex: a cell that became a native view only when lifted remounted the row under the
 * finger, and iOS ended the drag a few points in.
 */
function LiftCell({ item, style, children, ...rest }: ViewProps & { item: ListItem; index: number; cellKey?: string }) {
  return (
    <View {...rest} collapsable={false} style={[style, styles.cell, item.kind === "row" && item.lifted && styles.liftedCell]}>
      {children}
    </View>
  );
}

/** What a row does, read through a ref so a memoized row never calls the callbacks of an old render. */
interface RowActs {
  /** The row's latest build (the one the row drew from may be older but draws the same). */
  row: (key: string) => TabRow | undefined;
  go: (r: TabRow) => void;
  openPanes: (r: TabRow) => void;
  close: (pane: number) => void;
  move: (list: ListName, from: number, to: number) => void;
}

interface TabsRowProps {
  r: TabRow;
  list: ListName;
  index: number;
  count: number;
  canDrag: boolean;
  isHere: boolean;
  closeable: boolean;
  drag: Dragging;
  setDrag: (d: Dragging) => void;
  acts: RefObject<RowActs>;
}

// re-rendered only when what it draws changed: the page re-renders on every publish and clock tick
const TabsRow = memo(
  function TabsRow({ r, list, index, count, canDrag, isHere, closeable, drag, setDrag, acts }: TabsRowProps) {
    const latest = () => acts.current.row(r.key) ?? r;
    return (
      <ReorderRow grip index={index} count={count} rowH={LIST_ROW_H} enabled={canDrag} drag={drag} setDrag={setDrag} onMove={(from, to) => acts.current.move(list, from, to)}>
        {(handle) => (
          <ListRow
            handle={handle}
            movable={canDrag}
            title={r.title}
            marks={
              <>
                {r.tab.pinned ? <Icon name="pin" size={12} color={colors.textMuted} /> : null}
                {r.panes > 1 ? (
                  <Pressable accessibilityRole="button" accessibilityLabel={`${panesText(r.panes)} in ${r.title}`} hitSlop={10} onPress={() => acts.current.openPanes(latest())} style={({ pressed }) => [styles.panes, pressed && styles.pressed]}>
                    <Text style={styles.panesText}>{panesText(r.panes)}</Text>
                  </Pressable>
                ) : null}
              </>
            }
            line={r.paneLine}
            kind={r.kind}
            when={r.when}
            label={[r.title, isHere ? "current tab" : null, r.tab.pinned ? "pinned" : null, r.state, r.line].filter(Boolean).join(", ")}
            selected={isHere}
            onPress={() => acts.current.go(latest())}
            onClose={closeable && r.pane !== null ? () => acts.current.close(r.pane!) : undefined}
            closeLabel={`Close tab ${r.title}`}
          />
        )}
      </ReorderRow>
    );
  },
  (a, b) =>
    a.list === b.list &&
    a.index === b.index &&
    a.count === b.count &&
    a.canDrag === b.canDrag &&
    a.isHere === b.isHere &&
    a.closeable === b.closeable &&
    a.drag === b.drag &&
    a.setDrag === b.setDrag &&
    a.acts === b.acts &&
    sameTabRow(a.r, b.r),
);

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  fill: { flex: 1, backgroundColor: colors.bg },
  hidden: { opacity: 0 },
  pad: { paddingHorizontal: SIDE },
  header: { flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: SIDE, paddingTop: 8, paddingBottom: 12 },
  round: { width: 44, height: 44, borderRadius: 22, borderWidth: 1, borderColor: colors.glassRim, backgroundColor: colors.glass, alignItems: "center", justifyContent: "center" },
  titles: { flex: 1, minWidth: 0, gap: 3 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 22 },
  subtitle: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  subtitleText: { flexShrink: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  search: { height: 44, marginHorizontal: SIDE, marginBottom: 10, borderRadius: 12, borderWidth: 1, borderColor: colors.hairline, backgroundColor: colors.surface, flexDirection: "row", alignItems: "center", gap: 8, paddingLeft: 12 },
  searchInput: { flex: 1, height: 44, fontFamily: ui(400), color: colors.textBright, fontSize: 15, paddingRight: 12 },
  filtersBox: { flexGrow: 0 },
  filters: { gap: 6, paddingHorizontal: SIDE, paddingBottom: 10 },
  chip: { height: 32, paddingHorizontal: 12, borderRadius: 16, borderWidth: 1, borderColor: colors.glassRim, justifyContent: "center" },
  chipOn: { backgroundColor: colors.glassRim, borderColor: "transparent" },
  chipText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  chipTextOn: { color: colors.textBright },
  cap: { fontFamily: ui(600), color: colors.textMuted, fontSize: 12, letterSpacing: 0.6, textTransform: "uppercase", paddingHorizontal: SIDE, paddingTop: 14, paddingBottom: 6, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  capHint: { fontFamily: ui(400), letterSpacing: 0, textTransform: "none" },
  cell: { zIndex: 0 },
  liftedCell: { zIndex: 2, elevation: 6 },
  panes: { borderRadius: 6, borderWidth: 1, borderColor: colors.borderStrong, paddingHorizontal: 5, paddingVertical: 1 },
  panesText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 11 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingVertical: 24, paddingHorizontal: SIDE, textAlign: "center" },
});
