import { useActionScope } from "@/ui/useActionScope";
import { useIsFocused, usePathname, useRouter } from "expo-router";
import { memo, useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { primaryChoice } from "@/answer";
import { homeChoiceLabel, homeRows, homeSummary, idleSince, quickChoices, sameTabRow, seenWorking, tabKey, tabKind, tabRow, trackIdle, type RowContext, type TabRow } from "@/now";
import { canOperate, type HostView } from "@/fleet";
import { needsAuthBeforeRun } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { replacedPairing, restoreTarget } from "@/stores";
import { Tile } from "@/ui/Avatar";
import { Button } from "@/ui/Button";
import { lineColor } from "@/ui/ConnectionSheet";
import { hostLine, lastSeenAt, pairedText, updating } from "@/ui/connStatus";
import { hostDot } from "@/ui/hostDot";
import { FirstRun } from "@/ui/FirstRun";
import { Icon } from "@/ui/Icon";
import { confirmDestructive } from "@/ui/confirm";
import { RowMark } from "@/ui/ListRow";
import { canNewTab } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { ProgressBar } from "@/ui/ProgressBar";
import { NavRow } from "@/ui/Rows";
import { SwipeAction } from "@/ui/SwipeAction";
import { glyph, headerTop, IconButton, tabBottom } from "@/ui/Screen";
import { radius, ui, makeStyles, useColors } from "@/ui/theme";
import { useNow } from "@/ui/useNow";

/** The last pane is reopened once per app run, on the first launch of Portals. */
let restored = false;

const openPane = (router: ReturnType<typeof useRouter>, host: string, pane: number) =>
  router.push({ pathname: "/host/[key]/pane/[id]", params: { key: host, id: String(pane) } });

/** A coding agent's question answered in place (Main mockup): the pills, then "Open". */
function AnswerPills({ row, host }: { row: TabRow; host: HostView }) {
  const styles = useStyles();
  const router = useRouter();
  const s = useApp();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // a second tap before the re-render that disables the buttons sends nothing
  const sending = useRef(false);
  const ask = row.ask!;
  const scope = useActionScope(`${row.host}:${ask.pane}`);
  const quick = quickChoices(ask.waiting) ?? [];
  const primary = primaryChoice(ask.waiting);
  const send = async (index: number) => {
    if (sending.current) return;
    const current = scope.capture();
    sending.current = true;
    setError(null);
    if (needsAuthBeforeRun(s.settings)) {
      const auth = await authenticate(`Answer ${ask.program ?? "the program"} on ${host.record.name}`);
      if (!current()) { sending.current = false; return; }
      if (!auth.ok) {
        sending.current = false;
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    setBusy(true);
    const r = await fleet.answer(row.host, ask.pane, ask.waiting, { choice: index });
    sending.current = false;
    setBusy(false);
    if (!current()) return;
    if (!r.ok) setError(r.message);
    else AccessibilityInfo.announceForAccessibility("Answer sent");
  };
  return (
    <View style={styles.pillsBox}>
      <View style={styles.pills}>
        {quick.map((q) => (
          <Button
            key={q.index}
            label={homeChoiceLabel(q.label)}
            accessibilityLabel={q.opens ? `${ask.waiting.choices[q.index]}, opens the question` : ask.waiting.choices[q.index]}
            variant={q.index === primary ? "primary" : "secondary"}
            size="small"
            numberOfLines={1}
            disabled={busy}
            onPress={() =>
              q.opens
                ? router.push({ pathname: "/answer", params: { key: row.host, pane: String(ask.pane), reply: "1" } })
                : void send(q.index)
            }
          />
        ))}
        <View style={styles.flex} />
        <Button label="Open" accessibilityLabel={`Open ${row.title}`} variant="text" size="small" onPress={() => openPane(router, row.host, ask.pane)} />
      </View>
      {error ? <ErrorText>{error}</ErrorText> : null}
    </View>
  );
}

interface TabRowProps {
  row: TabRow;
  host: HostView;
  last: boolean;
  answerHere: boolean;
}

// memoized: Portals stays mounted under every pane, and its rows are built afresh on each publish
const TabRowView = memo(TabRowBody, (a: TabRowProps, b: TabRowProps) => a.last === b.last && a.answerHere === b.answerHere && a.host.record.name === b.host.record.name && sameTabRow(a.row, b.row));

function TabRowBody({ row, host, last, answerHere }: TabRowProps) {
  const styles = useStyles();
  const router = useRouter();
  const needs = row.kind === "needs";
  const question = row.ask ? row.ask.waiting.prompt.split("\n")[0]!.trim() : "";
  const inline = answerHere && needs && row.operate && row.ask?.waiting.kind === "choice" && quickChoices(row.ask.waiting) !== null;
  // the line carries the question ("Waiting for you · Edit x?"); the inline answer shows it
  // whole. The pane count goes beside the title ("project2 · 2 panes"), so the line says what it does.
  const line = row.paneLine || row.line;
  return (
    <View style={[styles.row, needs && styles.rowNeeds, !last && styles.rowRule, inline && styles.rowInline]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={[row.title, row.panes > 1 ? `${row.panes} panes` : null, row.state, inline ? question : line].filter(Boolean).join(", ")}
        accessibilityHint="Opens the tab"
        disabled={row.pane === null}
        onPress={() => row.pane !== null && openPane(router, row.host, row.pane)}
        style={({ pressed }) => [styles.rowMain, inline ? styles.rowMainInline : styles.rowMainCompact, pressed && styles.pressed]}
      >
        <View style={styles.tileBox}>
          <Tile agent={row.agent} program={row.program} />
        </View>
        <View style={styles.rowText}>
          <View style={styles.titleLine}>
            {/* the name is cut once, at its end; the pane count stays whole */}
            <Text style={styles.title} numberOfLines={1}>
              {row.title}
            </Text>
            {row.panes > 1 ? <Text style={styles.panes}>{` · ${row.panes} panes`}</Text> : null}
          </View>
          <Text style={styles.line} numberOfLines={1}>
            {line}
          </Text>
          {inline ? (
            <Text style={styles.question}>
              {question}
            </Text>
          ) : null}
        </View>
        {/* the desktop tab's mark, then since when (the state in words is in the label) */}
        <RowMark kind={row.kind} when={row.when} />
      </Pressable>
      {inline ? <AnswerPills row={row} host={host} /> : null}
    </View>
  );
}

function HostCard({ host, rows, now, answerHere, twin, replaced }: { host: HostView; rows: TabRow[]; now: number; answerHere: boolean; twin: boolean; replaced: boolean }) {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  const scope = useActionScope(host.record.key);
  const name = host.record.name;
  const online = host.conn.status === "open";
  const failed = host.conn.status === "failed";
  const tabs = host.windows.reduce((n, w) => n + w.tabs.length, 0);
  const line = hostLine(host.conn, now, online ? tabs : null, lastSeenAt(host), replaced);
  // the card's line stays grey when it cannot reach the computer: its fixes are the rows under it
  const lineTint = line.tone === "bad" ? colors.textMuted : lineColor(colors)[line.tone];
  // another pairing has the same name: each card says when it was paired, which tells them apart
  const paired = twin ? pairedText(host.record.pairedAt, now) : "";
  const lineText = paired ? `${line.text} · ${paired}` : line.text;
  // a computer that cannot be reached offers its fixes in place: Try again, and Unpair (an old pairing)
  const unreachable = (failed || host.conn.status === "closed" || (replaced && !online && host.conn.status !== "connecting")) && !updating(host.conn);
  const shown = homeRows(rows);
  // the permission of this connection, else the one stored at pairing
  const viewOnly = !canOperate(host);
  // Swipe left, Unpair: asks first; the computer leaves the list once this phone has forgotten it
  const unpairing = useRef(false);
  const unpair = async () => {
    // one question at a time: a second tap while the first is open asks nothing more
    if (unpairing.current) return;
    unpairing.current = true;
    try {
      await unpairAfterAsking();
    } finally {
      unpairing.current = false;
    }
  };
  const unpairAfterAsking = async () => {
    setError(null);
    const current = scope.capture();
    const ok = await confirmDestructive(
      `Unpair ${name}?`,
      "This phone forgets it. To use it again, scan a new pairing code from that computer’s Rift settings.",
      "Unpair",
    );
    if (!ok || !current()) return;
    try {
      await fleet.unpair(host.record.key);
    } catch {
      setError("This phone could not remove the computer. Try Unpair again.");
    }
  };
  // the New tab sheet chooses what the tab starts with and opens it
  const newTab = () => router.push({ pathname: "/new-tab", params: { key: host.record.key, from: "home" } });
  return (
    <SwipeAction label={`Unpair ${name}`} text="Unpair" radius={radius.card} onAction={() => void unpair()}>
      <View style={styles.card} accessibilityLabel={name}>
        <View style={[styles.head, rows.length > 0 && styles.headRule]}>
          <View style={[styles.dot, { backgroundColor: colors[hostDot(host)] }]} />
          <View style={styles.headText}>
            <View style={styles.nameLine}>
              <Text style={styles.name} numberOfLines={1}>
                {name}
              </Text>
              {viewOnly ? <Text style={styles.viewOnly}>view only</Text> : null}
            </View>
            {/* the line is a button: it opens the Connection sheet (every path, with why it is used or not) */}
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${lineText}. Connection details`}
              hitSlop={{ top: 13, bottom: 13, right: 16 }}
              onPress={() => router.push({ pathname: "/connection", params: { key: host.record.key } })}
              style={({ pressed }) => [styles.connButton, pressed && styles.pressed]}
            >
              <Text style={[styles.conn, { color: lineTint }]}>{lineText}</Text>
            </Pressable>
          </View>
          {/* on a computer it cannot reach too: the sheet says it is offline */}
          {!viewOnly && (!online || canNewTab(host)) ? (
            // says what it does: the header's round + pairs a computer, this opens a tab
            <Button
              label="New tab"
              accessibilityLabel={`New tab on ${name}`}
              size="small"
              icon={<Icon name="add" size={glyph(14, 12)} color={colors.text} />}
              onPress={newTab}
            />
          ) : null}
          {!unreachable && line.retry ? (
            <Button
              label="Try again"
              accessibilityLabel={`Try again to reach ${name}`}
              size="small"
              onPress={() => fleet.retry(host.record.key)}
              style={styles.tryAgain}
            />
          ) : null}
        </View>
        {line.progress ? (
          <View style={styles.progress}>
            <ProgressBar />
          </View>
        ) : null}
        {line.hint && !unreachable ? <Text style={styles.why}>{line.hint}</Text> : null}
        {host.notice ? <ErrorText style={styles.inset}>{host.notice}</ErrorText> : null}
        {error ? <ErrorText style={styles.inset}>{error}</ErrorText> : null}
        {unreachable ? (
          <View style={styles.fixes}>
            {failed && host.conn.failure === "auth" ? (
              <NavRow first accent chevron={false} label="Pair again" accessibilityLabel={`Pair ${name} again`} hint="Opens the camera to scan a new code" onPress={() => router.push("/scan")} />
            ) : line.retry ? (
              <NavRow first accent chevron={false} label="Try again" accessibilityLabel={`Try again to reach ${name}`} onPress={() => fleet.retry(host.record.key)} />
            ) : null}
            {/* the error red: it ends something; it asks first */}
            <NavRow
              first={!(failed && host.conn.failure === "auth") && !line.retry}
              destructive
              label="Unpair"
              accessibilityLabel={`Unpair ${name}`}
              hint="Asks first"
              onPress={() => void unpair()}
            />
          </View>
        ) : null}
        {online
          ? shown.map((r, i) => <TabRowView key={r.key} row={r} host={host} last={i === shown.length - 1} answerHere={answerHere} />)
          : null}
        {online && rows.length > shown.length ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Show all ${rows.length} tabs on ${name}`}
            onPress={() => router.push({ pathname: "/tabs", params: { key: host.record.key, from: "home" } })}
            style={({ pressed }) => [styles.showAll, pressed && styles.pressed]}
          >
            <Text style={styles.showAllText}>{`Show all ${rows.length} tabs`}</Text>
          </Pressable>
        ) : null}
      </View>
    </SwipeAction>
  );
}

/**
 * Portals (the home screen): one line that counts the tabs by state, then each paired computer
 * with its tabs in its own tab order (past five, the five that matter most) and a way to the rest.
 */
export default function PortalsScreen() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const s = useApp();
  const insets = useSafeAreaInsets();
  // the working clocks tick each second, else "Needs you · 2m" and "seen 2h ago" each 30 s
  // (and each second while a computer counts down to its next try)
  // (only while the page is on top: a covered page does not wake each second)
  const onTop = useIsFocused();
  const ticking =
    onTop &&
    (s.hosts.some((h) => h.conn.status === "open" && h.windows.some((w) => w.tabs.some((t) => tabKind(t) === "working"))) ||
      s.hosts.some((h) => h.conn.status === "closed" && h.conn.retryAt !== null));
  const now = useNow(ticking ? 1000 : 30_000);

  // which tab is idle since when (Idle 12m): set when a tab seen working turns idle
  const live = s.hosts.filter((h) => h.conn.status === "open");
  const kinds = live.flatMap((h) => h.windows.flatMap((w) => w.tabs.map((t) => ({ key: tabKey(h.record.key, t), kind: tabKind(t) }))));
  trackIdle(idleSince, seenWorking, kinds.filter((k) => k.kind === "working"), kinds.filter((k) => k.kind === "idle"), now);

  const multi = s.hosts.length > 1;
  const ctx: RowContext = { now, multi, waitingSince: (h, p, w) => fleet.waitingSince(h, p, w), asked: s.asked };
  const all = live.map((h) => ({ host: h, rows: h.windows.flatMap((w) => w.tabs.map((t) => tabRow(h, t, ctx))) }));
  // the counts are words in one line, not chips: they filter nothing (the Tabs page does)
  const summary = homeSummary(all.flatMap((c) => c.rows));

  // A cold start goes straight back to the pane open last, while its computer is still paired.
  // Only when Portals is on screen then: a pane already on top (this restore, a tapped alert)
  // is not pushed a second time, whose Back would then only lead to the first.
  const path = usePathname();
  useEffect(() => {
    if (restored || !s.ready) return;
    restored = true;
    if (path !== "/") return;
    // never to an old pairing of a computer paired again: it does not connect on its own
    const all = s.hosts.map((h) => h.record);
    const target = restoreTarget(
      s.lastView,
      all.filter((r) => !replacedPairing(r, all)),
    );
    if (!target || target.pane === null) return;
    router.push({ pathname: "/host/[key]/pane/[id]", params: { key: target.host, id: String(target.pane), restored: "1" } });
  }, [s.ready, s.lastView, s.hosts, router, path]);

  if (s.ready && !s.loadError && s.hosts.length === 0) return <FirstRun />;

  const rowsOf = (h: HostView) => all.find((c) => c.host.record.key === h.record.key)?.rows ?? [];
  const twin = (h: HostView) => s.hosts.some((o) => o !== h && o.record.name === h.record.name);
  const records = s.hosts.map((h) => h.record);
  return (
    <View style={styles.page}>
      <View style={[styles.header, { paddingTop: headerTop(insets.top) }]}>
        <View style={styles.titleRow}>
          <Text style={styles.heading} accessibilityRole="header">
            Portals
          </Text>
          {/* the root page keeps its large title; its two actions are the header's glass buttons */}
          <View style={styles.buttons}>
            <IconButton glass icon="settings" label="Settings" onPress={() => router.push("/settings")} />
            <IconButton glass icon="add" label="Pair a computer" onPress={() => router.push("/scan")} />
          </View>
        </View>
        {summary.length > 0 ? (
          <Text style={styles.summary} accessibilityLabel={summary.map((p) => p.text).join(", ")}>
            {summary.map((p, i) => (
              <Text key={p.kind} style={p.kind === "needs" ? { color: colors.warning } : undefined}>
                {i > 0 ? " · " : ""}
                {p.kind === "needs" ? "● " : ""}
                {p.text}
              </Text>
            ))}
          </Text>
        ) : null}
      </View>
      <ScrollView style={styles.flex} contentContainerStyle={[styles.cards, { paddingBottom: tabBottom(insets.bottom) }]}>
        {/* the keychain could not be read (locked, say): say so and offer to read it again, never a blank page */}
        {s.loadError ? (
          <View style={styles.loadError}>
            <ErrorText>{`Could not read this device’s keys: ${s.loadError}. Unlock the device, then try again.`}</ErrorText>
            <Button label="Try again" variant="primary" onPress={() => void fleet.retryInit()} />
          </View>
        ) : null}
        {s.hosts.map((h) => (
          <HostCard key={h.record.key} host={h} rows={rowsOf(h)} now={now} answerHere={!multi} twin={twin(h)} replaced={replacedPairing(h.record, records)} />
        ))}
      </ScrollView>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  flex: { flex: 1, minWidth: 0 },
  pressed: { opacity: 0.75 },
  // the #050505 page under the header's black (the New Portals board)
  page: { flex: 1, backgroundColor: colors.bg },
  // the New Portals board: 16 pt in, the summary 12 under the title; a scrolled card is cut 12 pt
  // under the summary, not at its baseline (the cards keep their 16 pt at rest)
  header: { paddingHorizontal: 16, paddingBottom: 12, gap: 12, backgroundColor: colors.chrome },
  titleRow: { flexDirection: "row", alignItems: "center", gap: 10 },
  // the Standard's large title: Inter 600 30/36
  heading: { flex: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 30, lineHeight: 36, letterSpacing: -0.5 },
  buttons: { flexDirection: "row", gap: 8 },
  summary: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  loadError: { gap: 12 },
  cards: { paddingTop: 4, paddingHorizontal: 16, gap: 14 },
  card: { borderRadius: radius.card, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  head: { flexDirection: "row", alignItems: "center", gap: 10, paddingLeft: 16, paddingRight: 12, paddingVertical: 12 },
  headRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  dot: { width: 9, height: 9, borderRadius: 5 },
  headText: { flex: 1, minWidth: 0 },
  nameLine: { flexDirection: "row", alignItems: "center", gap: 8 },
  name: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 17, lineHeight: 22 },
  // the status line wraps, never cut: its reason may be long
  connButton: { alignSelf: "flex-start", maxWidth: "100%" },
  conn: { fontFamily: ui(400), fontSize: 13, lineHeight: 17 },
  tryAgain: { marginRight: 8 },
  progress: { paddingHorizontal: 16, paddingBottom: 12 },
  // a fact, not a call for attention: orange is for "needs you" only
  viewOnly: {
    fontFamily: ui(500),
    color: colors.textMuted,
    fontSize: 11,
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
  },
  why: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18, paddingHorizontal: 16, paddingBottom: 14 },
  inset: { marginHorizontal: 16, marginBottom: 12 },
  row: { minHeight: 62 },
  rowNeeds: { backgroundColor: colors.tints.selected },
  rowRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  rowInline: { paddingBottom: 14 },
  // the New Portals board's row: 62 pt, 16 pt in, 12 at the end and between its parts
  rowMain: { flexDirection: "row", gap: 12, paddingLeft: 16, paddingRight: 12, alignItems: "flex-start" },
  rowMainInline: { paddingTop: 14, paddingBottom: 12 },
  rowMainCompact: { alignItems: "center", minHeight: 62 },
  tileBox: { flexShrink: 0 },
  rowText: { flex: 1, minWidth: 0, gap: 3 },
  titleLine: { flexDirection: "row", alignItems: "center", minWidth: 0 },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15, lineHeight: 20 },
  // "project2 · 2 panes" all in the title's 600 white
  panes: { flexShrink: 0, fontFamily: ui(600), color: colors.textBright, fontSize: 15, lineHeight: 20 },
  line: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  fixes: { borderTopWidth: 1, borderTopColor: colors.hairline },
  showAll: { minHeight: 44, alignItems: "center", justifyContent: "center", borderTopWidth: 1, borderTopColor: colors.hairline },
  showAllText: { fontFamily: ui(500), color: colors.accent, fontSize: 14 },
  question: { fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20, marginTop: 4 },
  pillsBox: { gap: 8, paddingLeft: 68, paddingRight: 16 },
  pills: { flexDirection: "row", alignItems: "center", gap: 8 },
}));

