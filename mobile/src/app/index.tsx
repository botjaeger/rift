import { useActionScope } from "@/ui/useActionScope";
import { useIsFocused, usePathname, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { primaryChoice } from "@/answer";
import { filterHome, homeChoiceLabel, homeCounts, idleSince, quickChoices, seenWorking, tabKey, tabKind, tabRow, trackIdle, type HomeFilter, type RowContext, type TabRow } from "@/now";
import { canOperate, type HostView } from "@/fleet";
import { needsAuthBeforeRun } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { restoreTarget } from "@/stores";
import { Tile } from "@/ui/Avatar";
import { LINE_COLOR } from "@/ui/ConnectionSheet";
import { hostLine, updating } from "@/ui/connStatus";
import { FirstRun } from "@/ui/FirstRun";
import { Icon } from "@/ui/Icon";
import { confirmDestructive } from "@/ui/confirm";
import { canNewTab, openNewTab } from "@/ui/newTab";
import { ErrorText } from "@/ui/Notice";
import { ProgressBar } from "@/ui/ProgressBar";
import { SwipeAction } from "@/ui/SwipeAction";
import { tabBottom, titleTop } from "@/ui/Screen";
import { toggleA11y } from "@/ui/toggle";
import { colors, mono, tint, ui } from "@/ui/theme";
import { useNow } from "@/ui/useNow";

/** The last pane is reopened once per app run, on the first launch of Portals. */
let restored = false;

const STATE_COLOR: Record<TabRow["kind"], string> = {
  needs: colors.warning,
  working: colors.accent,
  failed: colors.text,
  idle: colors.textMuted,
};

const CHIPS: { filter: HomeFilter; text: (n: number) => string; color: string }[] = [
  { filter: "needs", text: (n) => `${n} needs you`, color: colors.warning },
  { filter: "working", text: (n) => `${n} working`, color: colors.accent },
  { filter: "idle", text: (n) => `${n} idle`, color: colors.textMuted },
];

const openPane = (router: ReturnType<typeof useRouter>, host: string, pane: number) =>
  router.push({ pathname: "/host/[key]/pane/[id]", params: { key: host, id: String(pane) } });

/** The status dot: green while connected, the accent while trying, red when it cannot be reached. */
function dotColor(host: HostView): string {
  // an update restart is not a fault, and a quit is the user's own doing
  if (updating(host.conn)) return colors.accent;
  if (host.conn.failure === "host_quit") return colors.textDim;
  switch (host.conn.status) {
    case "open":
      return colors.success;
    case "connecting":
      return colors.accent;
    case "closed":
      return host.conn.retryAt !== null || host.conn.retryInMs !== null ? colors.error : colors.textDim;
    case "failed":
      return colors.error;
    default:
      return colors.textDim;
  }
}

/** A coding agent's question answered in place (Main mockup): the pills, then "Open". */
function AnswerPills({ row, host }: { row: TabRow; host: HostView }) {
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
          <Pressable
            key={q.index}
            accessibilityRole="button"
            accessibilityLabel={q.opens ? `${ask.waiting.choices[q.index]}, opens the question` : ask.waiting.choices[q.index]}
            accessibilityState={{ disabled: busy }}
            disabled={busy}
            hitSlop={{ top: 4, bottom: 4 }}
            onPress={() =>
              q.opens
                ? router.push({ pathname: "/answer", params: { key: row.host, pane: String(ask.pane), reply: "1" } })
                : void send(q.index)
            }
            style={({ pressed }) => [styles.pill, q.index === primary ? styles.pillPrimary : styles.pillOutline, pressed && styles.pressed]}
          >
            <Text
              style={q.index === primary ? styles.pillPrimaryText : styles.pillText}
              numberOfLines={1}
              adjustsFontSizeToFit
              minimumFontScale={0.75}
            >
              {homeChoiceLabel(q.label)}
            </Text>
          </Pressable>
        ))}
        <View style={styles.flex} />
        <Pressable
          accessibilityRole="link"
          accessibilityLabel={`Open ${row.title}`}
          hitSlop={{ top: 4, bottom: 4, left: 8, right: 8 }}
          onPress={() => openPane(router, row.host, ask.pane)}
          style={({ pressed }) => [styles.open, pressed && styles.pressed]}
        >
          <Text style={styles.openText}>Open</Text>
        </Pressable>
      </View>
      {error ? <ErrorText>{error}</ErrorText> : null}
    </View>
  );
}

function TabRowView({ row, host, last, answerHere }: { row: TabRow; host: HostView; last: boolean; answerHere: boolean }) {
  const router = useRouter();
  const needs = row.kind === "needs";
  const question = row.ask ? row.ask.waiting.prompt.split("\n")[0]!.trim() : "";
  const inline = answerHere && needs && row.operate && row.ask?.waiting.kind === "choice" && quickChoices(row.ask.waiting) !== null;
  // compact rows show the question as their second line
  const line = needs && !inline && question ? question : row.line;
  return (
    <View style={[styles.row, needs && styles.rowNeeds, !last && styles.rowRule, inline && styles.rowInline]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={[row.title, row.state, inline ? question : line].filter(Boolean).join(", ")}
        accessibilityHint="Opens the tab"
        disabled={row.pane === null}
        onPress={() => row.pane !== null && openPane(router, row.host, row.pane)}
        style={({ pressed }) => [styles.rowMain, inline ? styles.rowMainInline : styles.rowMainCompact, pressed && styles.pressed]}
      >
        <View style={styles.tileBox}>
          <Tile agent={row.agent} />
        </View>
        <View style={styles.rowText}>
          <View style={styles.titleLine}>
            <Text style={styles.title} numberOfLines={1}>
              {row.title}
            </Text>
            <Text style={[styles.state, { color: STATE_COLOR[row.kind] }]} numberOfLines={1}>
              {row.state}
            </Text>
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
      </Pressable>
      {inline ? <AnswerPills row={row} host={host} /> : null}
    </View>
  );
}

function HostCard({ host, rows, now, answerHere }: { host: HostView; rows: TabRow[]; now: number; answerHere: boolean }) {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  // one at a time: a second tap while the first is out would open two
  const out = useRef(false);
  const scope = useActionScope(host.record.key);
  const name = host.record.name;
  const online = host.conn.status === "open";
  const failed = host.conn.status === "failed";
  const tabs = host.windows.reduce((n, w) => n + w.tabs.length, 0);
  const line = hostLine(host.conn, now, online ? tabs : null);
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
  const newTab = async () => {
    if (out.current) return;
    out.current = true;
    setError(null);
    try {
      const current = scope.capture();
      const r = await openNewTab(host, current);
      if (!current()) return;
      if (!r.ok) return r.message ? setError(r.message) : undefined;
      AccessibilityInfo.announceForAccessibility("New tab opened");
      if (r.pane !== undefined) openPane(router, host.record.key, r.pane);
    } finally {
      out.current = false;
    }
  };
  return (
    <SwipeAction label={`Unpair ${name}`} text="Unpair" radius={16} onAction={() => void unpair()}>
      <View style={styles.card} accessibilityLabel={name}>
        <View style={[styles.head, rows.length > 0 && styles.headRule]}>
          <View style={[styles.dot, { backgroundColor: dotColor(host) }]} />
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
              accessibilityLabel={`${line.text}. Connection details`}
              hitSlop={{ top: 13, bottom: 13, right: 16 }}
              onPress={() => router.push({ pathname: "/connection", params: { key: host.record.key } })}
              style={({ pressed }) => [styles.connButton, pressed && styles.pressed]}
            >
              <Text style={[styles.conn, { color: LINE_COLOR[line.tone] }]}>{line.text}</Text>
              <Icon name="forward" size={12} color={LINE_COLOR[line.tone]} />
            </Pressable>
          </View>
          {online && canNewTab(host) ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`New tab on ${name}`}
              onPress={() => void newTab()}
              style={({ pressed }) => [styles.headIcon, pressed && styles.pressed]}
            >
              <Icon name="newTab" size={20} color={colors.textMuted} />
            </Pressable>
          ) : null}
          {failed && host.conn.failure === "auth" ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Pair ${name} again`}
              onPress={() => router.push("/scan")}
              style={({ pressed }) => [styles.headText2, pressed && styles.pressed]}
            >
              <Text style={styles.headAction}>Pair again</Text>
            </Pressable>
          ) : line.retry ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Try again to reach ${name}`}
              onPress={() => fleet.retry(host.record.key)}
              hitSlop={{ top: 4, bottom: 4 }}
              style={({ pressed }) => [styles.tryAgain, pressed && styles.pressed]}
            >
              <Text style={styles.tryAgainText}>Try again</Text>
            </Pressable>
          ) : null}
        </View>
        {line.progress ? (
          <View style={styles.progress}>
            <ProgressBar />
          </View>
        ) : null}
        {line.hint ? <Text style={styles.why}>{line.hint}</Text> : null}
        {host.notice ? <ErrorText style={styles.inset}>{host.notice}</ErrorText> : null}
        {error ? <ErrorText style={styles.inset}>{error}</ErrorText> : null}
        {online
          ? rows.map((r, i) => <TabRowView key={r.key} row={r} host={host} last={i === rows.length - 1} answerHere={answerHere} />)
          : null}
      </View>
    </SwipeAction>
  );
}

/** Portals (the home screen): the state chips, then each paired computer with its tabs. */
export default function PortalsScreen() {
  const router = useRouter();
  const s = useApp();
  const insets = useSafeAreaInsets();
  const [picked, setPicked] = useState<HomeFilter | null>(null);
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
  const counts = homeCounts(all.flatMap((c) => c.rows));
  // a state that ran out of tabs no longer filters
  const filter = picked !== null && counts[picked] > 0 ? picked : null;

  // A cold start goes straight back to the pane open last, while its computer is still paired.
  // Only when Portals is on screen then: a pane already on top (this restore, a tapped alert)
  // is not pushed a second time, whose Back would then only lead to the first.
  const path = usePathname();
  useEffect(() => {
    if (restored || !s.ready) return;
    restored = true;
    if (path !== "/") return;
    const target = restoreTarget(
      s.lastView,
      s.hosts.map((h) => h.record),
    );
    if (!target || target.pane === null) return;
    router.push({ pathname: "/host/[key]/pane/[id]", params: { key: target.host, id: String(target.pane), restored: "1" } });
  }, [s.ready, s.lastView, s.hosts, router, path]);

  if (s.ready && !s.loadError && s.hosts.length === 0) return <FirstRun />;

  const rowsOf = (h: HostView) => filterHome(all.find((c) => c.host.record.key === h.record.key)?.rows ?? [], filter);
  const shown = s.hosts.filter((h) => filter === null || rowsOf(h).length > 0);
  return (
    <View style={[styles.page, { paddingTop: titleTop(insets.top) }]}>
      <View style={styles.header}>
        <View style={styles.titleRow}>
          <Text style={styles.heading} accessibilityRole="header">
            Portals
          </Text>
          <View style={styles.buttons}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Settings"
              onPress={() => router.push("/settings")}
              style={({ pressed }) => [styles.glass, pressed && styles.pressed]}
            >
              <Icon name="settings" size={20} color={colors.text} />
            </Pressable>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Pair a computer"
              onPress={() => router.push("/scan")}
              style={({ pressed }) => [styles.glass, pressed && styles.pressed]}
            >
              <Icon name="add" size={20} color={colors.text} />
            </Pressable>
          </View>
        </View>
        <View style={styles.chips} accessibilityRole="toolbar" accessibilityLabel="Show tabs that">
          {CHIPS.filter((c) => counts[c.filter] > 0).map((c) => {
            const on = filter === c.filter;
            // the needs chip stands out while nothing is filtered: it is what wants a look
            const lit = on || (c.filter === "needs" && filter === null);
            const label = c.text(counts[c.filter]);
            return (
              <Pressable
                key={c.filter}
                {...toggleA11y(on)}
                accessibilityLabel={label}
                onPress={() => setPicked(on ? null : c.filter)}
                style={({ pressed }) => [
                  styles.chip,
                  lit && { borderColor: tint(c.color, 0x55), backgroundColor: c.filter === "idle" ? colors.glassRim : tint(c.color, 0x1f) },
                  pressed && styles.pressed,
                ]}
              >
                {lit && c.filter !== "idle" ? <View style={[styles.chipDot, { backgroundColor: c.color }]} /> : null}
                <Text style={[styles.chipText, lit && { color: c.filter === "idle" ? colors.textBright : c.color }]}>{label}</Text>
              </Pressable>
            );
          })}
        </View>
      </View>
      <ScrollView style={styles.flex} contentContainerStyle={[styles.cards, { paddingBottom: tabBottom(insets.bottom) }]}>
        {shown.map((h) => (
          <HostCard key={h.record.key} host={h} rows={rowsOf(h)} now={now} answerHere={!multi} />
        ))}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, minWidth: 0 },
  pressed: { opacity: 0.75 },
  page: { flex: 1, backgroundColor: colors.chrome },
  header: { paddingHorizontal: 20, gap: 10 },
  titleRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  heading: { fontFamily: ui(600), color: colors.textBright, fontSize: 32, letterSpacing: -0.5 },
  buttons: { flexDirection: "row", gap: 8 },
  glass: {
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
    alignItems: "center",
    justifyContent: "center",
  },
  chips: { flexDirection: "row", gap: 8 },
  chip: {
    height: 32,
    paddingHorizontal: 12,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.glassRim,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  chipDot: { width: 7, height: 7, borderRadius: 4 },
  chipText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  cards: { paddingTop: 16, paddingHorizontal: 16, gap: 12 },
  card: { borderRadius: 16, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  head: { flexDirection: "row", alignItems: "center", gap: 10, paddingLeft: 16, paddingRight: 6, minHeight: 60, paddingVertical: 8 },
  headRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  dot: { width: 9, height: 9, borderRadius: 5 },
  headText: { flex: 1, minWidth: 0, gap: 2 },
  nameLine: { flexDirection: "row", alignItems: "center", gap: 8 },
  name: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  // the status line wraps, never cut: its reason may be long
  connButton: { alignSelf: "flex-start", flexDirection: "row", alignItems: "center", gap: 6, maxWidth: "100%" },
  conn: { flexShrink: 1, fontFamily: ui(400), fontSize: 13, lineHeight: 18 },
  tryAgain: { height: 36, paddingHorizontal: 14, borderRadius: 18, borderWidth: 1, borderColor: colors.borderStrong, alignItems: "center", justifyContent: "center", marginRight: 8 },
  tryAgainText: { fontFamily: ui(400), color: colors.textBright, fontSize: 14 },
  progress: { paddingHorizontal: 16, paddingBottom: 12 },
  viewOnly: {
    fontFamily: ui(500),
    color: colors.warning,
    fontSize: 11,
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 4,
    borderWidth: 1,
    borderColor: `${colors.warning}55`,
  },
  headIcon: { width: 44, height: 44, alignItems: "center", justifyContent: "center" },
  headText2: { minWidth: 44, height: 44, paddingHorizontal: 10, alignItems: "center", justifyContent: "center" },
  headAction: { fontFamily: ui(500), color: colors.accent, fontSize: 14 },
  why: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18, paddingHorizontal: 16, paddingBottom: 14 },
  inset: { marginHorizontal: 16, marginBottom: 12 },
  row: { minHeight: 64 },
  rowNeeds: { backgroundColor: `${colors.warning}0d` },
  rowRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  rowInline: { paddingBottom: 14 },
  rowMain: { flexDirection: "row", gap: 12, paddingLeft: 16, paddingRight: 12, paddingTop: 12, paddingBottom: 12, alignItems: "flex-start" },
  rowMainInline: { paddingTop: 14 },
  rowMainCompact: { alignItems: "center", minHeight: 64 },
  tileBox: { flexShrink: 0 },
  rowText: { flex: 1, minWidth: 0, gap: 3 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 8 },
  title: { flex: 1, minWidth: 0, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  state: { fontFamily: ui(500), fontSize: 12 },
  line: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  question: { fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20, marginTop: 4 },
  pillsBox: { gap: 8, paddingLeft: 68, paddingRight: 16 },
  pills: { flexDirection: "row", alignItems: "center", gap: 8 },
  pill: { height: 36, borderRadius: 18, alignItems: "center", justifyContent: "center" },
  pillPrimary: { backgroundColor: colors.accent, paddingHorizontal: 16 },
  pillOutline: { borderWidth: 1, borderColor: colors.borderStrong, paddingHorizontal: 14 },
  pillPrimaryText: { fontFamily: ui(600), color: colors.bg, fontSize: 14 },
  pillText: { fontFamily: ui(500), color: colors.text, fontSize: 14 },
  open: { height: 36, justifyContent: "center" },
  openText: { fontFamily: ui(500), color: colors.accent, fontSize: 14 },
});

