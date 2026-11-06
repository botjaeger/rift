import { useActionScope } from "@/ui/useActionScope";
import { useLocalSearchParams, useRouter } from "expo-router";
import { memo, useEffect, useRef, useState, type RefObject } from "react";
import { FlatList, Pressable, ScrollView, Text, View, type ViewProps } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { EMPTY_FILTER, idleSince, seenWorking, sheetCounts, sameTabRow, sheetRows, tabKey, tabKind, tabRow, trackIdle, type RowContext, type SheetFilter, type TabRow } from "@/now";
import { canClose, canMove, canOperate } from "@/fleet";
import { panesText } from "@/paneView";
import { fleet, useApp, useLock } from "@/runtime";
import { Field } from "@/ui/Field";
import { FilterChip } from "@/ui/FilterChip";
import { Icon } from "@/ui/Icon";
import { LIST_ROW_H, ListRow } from "@/ui/ListRow";
import { canNewTab } from "@/ui/newTab";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { IconButton, PageHeader } from "@/ui/Screen";
import { ui, makeStyles, useColors } from "@/ui/theme";
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
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  // `from=home`: opened from a computer's card on Portals (Show all), with no pane behind it
  const { key, pane, from } = useLocalSearchParams<{ key: string; pane?: string; from?: string }>();
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
  // a new search or filter shows what it found from its top (its count, the first match), not
  // from wherever the whole list was scrolled to
  const list = useRef<FlatList<ListItem>>(null);
  useEffect(() => {
    list.current?.scrollToOffset({ offset: 0, animated: false });
  }, [filter, query]);
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
    // from Portals: the pane opens over it, so Back leads to Portals
    if (from === "home") {
      router.push({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(r.pane) } });
      return;
    }
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
  // the whole list: "Pinned", then "Tabs · Drag ≡ to reorder"; a narrowed one counts what it holds
  if (pinned.length > 0 && whole) {
    items.push({ kind: "cap", key: "cap:pinned", text: "Pinned" });
    pinned.forEach((r, i) => items.push({ kind: "row", key: r.key, r, list: "pinned", index: i, lifted: pinnedDrag.drag.active === i }));
  }
  const rest = whole ? others : shown;
  if (rest.length > 0) {
    items.push({ kind: "cap", key: "cap:others", text: whole ? (movable ? "Tabs · Drag ≡ to reorder" : "Tabs") : `${rest.length} ${rest.length === 1 ? "tab" : "tabs"}` });
    rest.forEach((r, i) => items.push({ kind: "row", key: r.key, r, list: whole ? "others" : "found", index: i, lifted: whole && othersDrag.drag.active === i }));
  }
  // why the grips went away, and how to get them back
  const narrowedNote = !whole && shown.length > 0 && canMove(host) && rows.length > 1;
  const showAll = () => {
    setFilter("all");
    setQuery("");
  };
  const empty = query.trim() ? { title: `No tab matches “${query.trim()}”`, line: "Search looks in names, commands and folders." } : EMPTY_FILTER[filter];

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
    <View style={[styles.fill, hidden && styles.hidden]} accessibilityElementsHidden={hidden} importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}>
      {/* ‹ Back as Panes has it; Split pane is in the pane's ⋯, New tab stays as + */}
      <PageHeader
        title="Tabs"
        subtitle={host ? `${host.record.name} · ${online ? `${rows.length} ${rows.length === 1 ? "tab" : "tabs"}` : "offline"}` : "Computer not found"}
        dot={online ? colors.success : colors.textDim}
        back={close}
        // a view-only device has no way to open a tab
        right={operate ? <IconButton glass icon="add" label="New tab" disabled={!canOpen} onPress={() => router.push({ pathname: "/new-tab", params: { key } })} /> : null}
      />

      <View style={styles.tools}>
        <Field
          icon="search"
          label="Search tabs"
          placeholder="Search by name, command or folder"
          value={query}
          onChangeText={setQuery}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
        />
        <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.filters} accessibilityRole="toolbar" accessibilityLabel="Show" keyboardShouldPersistTaps="handled">
          {FILTERS.map((f) => (
            <FilterChip key={f.filter} label={f.text} count={counts[f.filter]} selected={filter === f.filter} onPress={() => setFilter(f.filter)} />
          ))}
        </ScrollView>
      </View>

      {/* virtualized: a computer of a hundred tabs draws the screenful in view, not every row */}
      <FlatList
        ref={list}
        style={styles.fill}
        data={items}
        keyExtractor={(it) => it.key}
        renderItem={({ item }) =>
          item.kind === "cap" ? (
            <Text style={styles.cap} accessibilityRole="header">
              {item.text}
            </Text>
          ) : item.list === "pinned" ? (
            row(pinned, "pinned", item.r, item.index, pinnedDrag)
          ) : item.list === "others" ? (
            row(others, "others", item.r, item.index, othersDrag)
          ) : (
            row(shown, "found", item.r, item.index, othersDrag)
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
        ListFooterComponent={narrowedNote ? <Text style={styles.note}>Reordering works on the whole list; clear the search to drag.</Text> : null}
        // an empty filter says so, and offers the way back
        ListEmptyComponent={
          host && online ? (
            <View style={styles.emptyBox}>
              <Text style={styles.emptyTitle} accessibilityRole="header">
                {empty.title}
              </Text>
              <Text style={styles.empty}>{empty.line}</Text>
              {filter !== "all" || query.trim() ? <Button label="Show all tabs" size="small" onPress={showAll} /> : null}
            </View>
          ) : null
        }
      />
    </View>
  );
}

/** The pinned tabs, the others (the whole list, in the computer's order), or what a search or filter found. */
type ListName = "pinned" | "others" | "found";

type ListItem =
  | { kind: "cap"; key: string; text: string }
  | { kind: "row"; key: string; r: TabRow; list: ListName; index: number; lifted: boolean };

/**
 * The list's cell: the cell of the row being dragged draws above the cells after it (each cell is
 * a sibling of the others, so the row's own zIndex stays inside its cell). A component of its own:
 * FlatList would remount every cell for a new function each render. Never flattened and always
 * with a zIndex: a cell that became a native view only when lifted remounted the row under the
 * finger, and iOS ended the drag a few points in.
 */
function LiftCell({ item, style, children, ...rest }: ViewProps & { item: ListItem; index: number; cellKey?: string }) {
  const styles = useStyles();
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
    const colors = useColors();
    const styles = useStyles();
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
            lineCodes={r.codes}
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

const useStyles = makeStyles((colors) => ({
  pressed: { opacity: 0.75 },
  fill: { flex: 1, backgroundColor: colors.bg },
  hidden: { opacity: 0 },
  pad: { paddingHorizontal: SIDE },
  // the search field and the filters, on the header's chrome
  tools: { gap: 10, paddingTop: 12, paddingBottom: 12, paddingHorizontal: 14, backgroundColor: colors.chrome, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  filters: { gap: 6 },
  cap: { fontFamily: ui(500), color: colors.textMuted, fontSize: 12, letterSpacing: 0.5, textTransform: "uppercase", paddingHorizontal: SIDE + 4, paddingTop: 18, paddingBottom: 8, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  cell: { zIndex: 0 },
  liftedCell: { zIndex: 2, elevation: 6 },
  panes: { borderRadius: 6, borderWidth: 1, borderColor: colors.borderStrong, paddingHorizontal: 5, paddingVertical: 1 },
  panesText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 11 },
  emptyBox: { alignItems: "center", gap: 8, paddingVertical: 32, paddingHorizontal: SIDE },
  emptyTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 16, textAlign: "center" },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20, textAlign: "center", marginBottom: 4 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18, paddingVertical: 16, paddingHorizontal: SIDE, textAlign: "center" },
}));
