import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { canClose, canMove, canOperate } from "@/fleet";
import { paneKind, paneSummary, programName, type RowContext } from "@/now";
import { panesText, paneRows } from "@/paneView";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { LIST_ROW_H, ListRow } from "@/ui/ListRow";
import { canNewPane } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { IconButton, PageHeader, SectionTitle } from "@/ui/Screen";
import { Tile } from "@/ui/Avatar";
import { ui, makeStyles } from "@/ui/theme";
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
  const styles = useStyles();
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
    <View style={[styles.fill, hidden && styles.hidden]} accessibilityElementsHidden={hidden} importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}>
      {/* the same header as Tabs: ‹ Back, the tab and its pane count, New pane as + */}
      <PageHeader
        title="Panes"
        subtitle={loc ? `${tabTitle(loc.tab)} · ${panesText(rows.length)}` : undefined}
        back={() => router.back()}
        // a view-only device has no way to open a pane
        right={
          host && loc && canOperate(host) ? (
            <IconButton glass icon="add" label={`New pane in ${tabTitle(loc.tab)}`} disabled={!canOpen} onPress={newPane} />
          ) : null
        }
      />
      <ScrollView style={styles.fill} scrollEnabled={reorder.scrollEnabled} contentContainerStyle={{ paddingBottom: Math.max(30, insets.bottom + 2) }}>
        {loc && movable ? <SectionTitle style={styles.cap}>Drag ≡ to reorder</SectionTitle> : null}
        {toast}
        {error ? (
          <View style={styles.pad}>
            <ErrorText>{error}</ErrorText>
          </View>
        ) : null}
        {loc ? (
          <View accessibilityRole="list" accessibilityLabel={`Panes in ${tabTitle(loc.tab)}`}>
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
                      lead={<Tile agent={r.claude} program={programName(p)} />}
                      title={r.name}
                      // the title names the program: the line says what it does
                      line={sum.activity}
                      kind={sum.kind}
                      when={sum.when}
                      label={`Pane ${r.number} of ${rows.length}: ${r.name}, ${sum.state}, ${sum.activity}`}
                      selected={on}
                      onPress={() => pick(r.id)}
                      onClose={closeable ? () => closePane(r.id, "pane") : undefined}
                      // the number tells two panes running the same program apart
                      closeLabel={`Close pane ${r.number}: ${r.name}`}
                    />
                  )}
                </ReorderRow>
              );
            })}
          </View>
        ) : (
          <Text style={[styles.pad, styles.gone]}>This pane has closed.</Text>
        )}
      </ScrollView>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  hidden: { opacity: 0 },
  pad: { paddingHorizontal: 16, paddingTop: 12 },
  cap: { paddingHorizontal: 16, paddingTop: 18, paddingBottom: 8 },
  gone: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
}));
