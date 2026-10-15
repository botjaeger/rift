import { useRouter } from "expo-router";
import { useState } from "react";
import { Platform, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { avatarFor } from "@/avatar";
import { agoText } from "@/chips";
import { canOperate, type HostView } from "@/fleet";
import { programName, tabLast, tabMatches, tabPane, tabStarted, tabState } from "@/now";
import { fleet, useApp } from "@/runtime";
import { tabTitle } from "@/topology";
import { Button } from "@/ui/Button";
import { Avatar } from "@/ui/Avatar";
import { describeConn } from "@/ui/connLabel";
import { formatClock } from "@/ui/format";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { TabScreen } from "@/ui/Screen";
import { useNow } from "@/ui/useNow";
import { colors, mono, TOUCH, ui } from "@/ui/theme";

const STATE: Record<ReturnType<typeof tabState>, { label: string; color: string } | null> = {
  running: { label: "running", color: colors.accent },
  failed: { label: "failed", color: colors.textBright },
  needs: { label: "needs you", color: colors.warning },
  idle: null,
};

/** "LAN" / "Relay" while connected, "2 tabs · Relay" collapsed, "seen 2 h ago" while away. */
function hostRight(host: HostView, now: number, collapsed: boolean): string {
  if (host.conn.status === "open") {
    const path = host.conn.path?.kind === "relay" ? "Relay" : "LAN";
    if (!collapsed) return path;
    const tabs = host.windows.reduce((n, w) => n + w.tabs.length, 0);
    return `${tabs} ${tabs === 1 ? "tab" : "tabs"} · ${path}`;
  }
  if (host.conn.status === "connecting") return "connecting";
  const seen = host.lastSeen ?? host.record.seenAt ?? null;
  return seen !== null ? `seen ${agoText(now - seen)}` : "not connected yet";
}

function Computer({
  host,
  now,
  open,
  onToggle,
  query,
}: {
  host: HostView;
  now: number;
  open: boolean;
  onToggle: () => void;
  /** Computers' search: only the tabs it finds, the computer open. */
  query: string;
}) {
  const router = useRouter();
  const online = host.conn.status === "open";
  const failed = host.conn.status === "failed";
  const why = !online && host.conn.status !== "connecting" ? describeConn(host.conn).label : null;
  const searching = query.trim() !== "";
  const tabs = host.windows.flatMap((w) => w.tabs).filter((t) => tabMatches(t, query));
  const expanded = online && (open || searching);
  const right = hostRight(host, now, online && !open);
  // the permission of this connection, else the one stored at pairing
  const viewOnly = !canOperate(host);
  return (
    <View style={[styles.card, !online && styles.away]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={[host.record.name, online ? "online" : "offline", viewOnly ? "view only" : null, right]
          .filter(Boolean)
          .join(", ")}
        accessibilityState={online ? { expanded } : { disabled: true }}
        accessibilityHint={online ? (expanded ? "Hides its tabs" : "Shows its tabs") : undefined}
        disabled={!online}
        onPress={onToggle}
        style={({ pressed }) => [styles.head, pressed && styles.pressed]}
      >
        <View style={[styles.dot, { backgroundColor: online ? colors.success : colors.textMuted }]} />
        <Text style={styles.name} numberOfLines={1}>
          {host.record.name}
        </Text>
        {viewOnly ? <Text style={styles.viewOnly}>view only</Text> : null}
        <Text style={styles.right}>{right}</Text>
      </Pressable>
      {expanded
        ? tabs.map((t, i) => {
            const state = tabState(t);
            const st = STATE[state];
            const pane = tabPane(t);
            const p = t.panes.find((x) => x.id === pane) ?? t.panes[0];
            const started = tabStarted(t);
            const time = state === "running" && started !== null ? formatClock(now - started) : null;
            // a dot for what wants a look: a question, a failure
            const attention = state === "needs" || state === "failed";
            const failedExit = t.panes.find((x) => x.last_exit !== null && x.last_exit !== 0)?.last_exit;
            const label = state === "failed" && failedExit != null ? `✕ ${failedExit}` : st?.label;
            return (
              <Pressable
                key={`${i}:${t.index}`}
                accessibilityRole="button"
                accessibilityLabel={[tabTitle(t), tabLast(t), st?.label, time ? `for ${time}` : null].filter(Boolean).join(", ")}
                accessibilityHint="Opens the tab"
                disabled={pane === null}
                onPress={() =>
                  pane !== null && router.push({ pathname: "/host/[key]/pane/[id]", params: { key: host.record.key, id: String(pane) } })
                }
                style={({ pressed }) => [styles.tab, pressed && styles.pressed]}
              >
                <View style={[styles.unread, attention && styles.unreadOn]} />
                <Avatar avatar={avatarFor(p?.cwd, tabTitle(t), p ? programName(p) : null)} />
                <View style={styles.tabText}>
                  <View style={styles.tabLine}>
                    <Text style={styles.tabName} numberOfLines={1}>
                      {tabTitle(t)}
                    </Text>
                    {time ? <Text style={styles.tabTime}>{time}</Text> : null}
                  </View>
                  <View style={styles.tabLine}>
                    <Text style={styles.tabLast} numberOfLines={1}>
                      {tabLast(t) || " "}
                    </Text>
                    {label ? <Text style={[styles.state, { color: st!.color }]}>{label}</Text> : null}
                  </View>
                </View>
                <Icon name="forward" size={14} color={colors.textMuted} />
              </Pressable>
            );
          })
        : null}
      {host.notice ? <ErrorText style={styles.inset}>{host.notice}</ErrorText> : null}
      {failed ? (
        <View style={styles.inset}>
          {why ? <Text style={styles.why}>{why}</Text> : null}
          {host.conn.failure === "auth" ? (
            <Button label="Pair again" size="small" onPress={() => router.push("/scan")} />
          ) : (
            <Button label="Try again" size="small" onPress={() => fleet.retry(host.record.key)} />
          )}
        </View>
      ) : null}
    </View>
  );
}

/** Computers (the P4 board): each paired computer, the open one with its tabs inline. */
export default function ComputersScreen() {
  const router = useRouter();
  const s = useApp();
  // "seen 2 h ago" moves on
  const [query, setQuery] = useState("");
  // the running tabs' clocks tick each second, else "seen 2 h ago" each minute
  const ticking = s.hosts.some((h) => h.windows.some((w) => w.tabs.some((t) => tabStarted(t) !== null)));
  const now = useNow(ticking ? 1000 : 60_000);
  // which computers show their tabs: the first connected one until the user picks
  const [picked, setPicked] = useState<ReadonlySet<string> | null>(null);
  const first = s.hosts.find((h) => h.conn.status === "open")?.record.key;
  const open = picked ?? new Set(first ? [first] : []);
  const toggle = (key: string) => {
    const next = new Set(open);
    if (!next.delete(key)) next.add(key);
    setPicked(next);
  };
  const pair = (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Pair a computer"
      onPress={() => router.push("/scan")}
      style={({ pressed }) => [styles.pair, pressed && styles.pressed]}
    >
      <Icon name="add" color={colors.text} />
    </Pressable>
  );
  return (
    <TabScreen
      title="Computers"
      accessory={pair}
      gap={10}
      large
      overlay={
        s.hosts.length > 0 ? (
          <View style={styles.search}>
            <Icon name="search" size={17} color={colors.textMuted} />
            <TextInput
              value={query}
              onChangeText={setQuery}
              placeholder="Search tabs and commands"
              placeholderTextColor={colors.textMuted}
              autoCapitalize="none"
              autoCorrect={false}
              returnKeyType="search"
              clearButtonMode="while-editing"
              accessibilityLabel="Search tabs and commands"
              style={[styles.searchInput, Platform.OS === "web" && styles.noRing]}
            />
          </View>
        ) : null
      }
    >
      {s.hosts.length === 0 && s.ready ? <Text style={styles.empty}>No computer paired yet.</Text> : null}
      {s.hosts.map((h) => (
        <Computer key={h.record.key} host={h} now={now} open={open.has(h.record.key)} onToggle={() => toggle(h.record.key)} query={query} />
      ))}
      {query.trim() !== "" &&
      !s.hosts.some((h) => h.conn.status === "open" && h.windows.some((w) => w.tabs.some((t) => tabMatches(t, query)))) ? (
        <Text style={styles.empty} accessibilityLiveRegion="polite">
          No tab holds “{query.trim()}”.
        </Text>
      ) : null}
    </TabScreen>
  );
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  pair: { width: 40, height: 40, borderRadius: 20, backgroundColor: colors.surface, alignItems: "center", justifyContent: "center" },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingHorizontal: 8 },
  card: { borderRadius: 14, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  away: { opacity: 0.75 },
  head: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: 12, paddingHorizontal: 14, minHeight: TOUCH + 4 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  name: { flex: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 16 },
  right: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
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
  // the Computers board's tab rows: a dot for what wants a look, the avatar, two lines, ›
  tab: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    paddingLeft: 8,
    paddingRight: 12,
    minHeight: 60,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  unread: { width: 6, height: 6, borderRadius: 3, marginRight: -6 },
  unreadOn: { backgroundColor: colors.accent },
  tabText: { flex: 1, minWidth: 0, gap: 3 },
  tabLine: { flexDirection: "row", alignItems: "baseline", gap: 8 },
  tabName: { flex: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  tabTime: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, fontVariant: ["tabular-nums"] },
  tabLast: { flex: 1, fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  state: { fontFamily: ui(400), fontSize: 12 },
  // the floating search bar over the page (the Computers board)
  search: {
    height: 44,
    borderRadius: 22,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 16,
    backgroundColor: colors.floating,
    borderWidth: 1,
    borderColor: "rgba(255, 255, 255, 0.14)",
    boxShadow: "0 10px 30px rgba(0, 0, 0, 0.45)",
  },
  searchInput: { flex: 1, minWidth: 0, height: 42, padding: 0, color: colors.textBright, fontFamily: ui(400), fontSize: 15 },
  // web only: Chrome draws `outline-style: auto` whatever its width
  noRing: { outlineStyle: "none" as "solid" },
  inset: { marginHorizontal: 14, marginBottom: 12, gap: 8 },
  why: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
});
