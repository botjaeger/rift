import { useRouter } from "expo-router";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { agoText } from "@/chips";
import { canOperate, type HostView } from "@/fleet";
import { tabLast, tabPane, tabState } from "@/now";
import { fleet, useApp } from "@/runtime";
import { tabTitle } from "@/topology";
import { Button } from "@/ui/Button";
import { describeConn } from "@/ui/connLabel";
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

function Computer({ host, now, open, onToggle }: { host: HostView; now: number; open: boolean; onToggle: () => void }) {
  const router = useRouter();
  const online = host.conn.status === "open";
  const failed = host.conn.status === "failed";
  const why = !online && host.conn.status !== "connecting" ? describeConn(host.conn).label : null;
  const tabs = host.windows.flatMap((w) => w.tabs);
  const expanded = online && open;
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
            const st = STATE[tabState(t)];
            const pane = tabPane(t);
            return (
              <Pressable
                key={`${i}:${t.index}`}
                accessibilityRole="button"
                accessibilityLabel={[tabTitle(t), tabLast(t), st?.label].filter(Boolean).join(", ")}
                accessibilityHint="Opens the tab"
                disabled={pane === null}
                onPress={() =>
                  pane !== null && router.push({ pathname: "/host/[key]/pane/[id]", params: { key: host.record.key, id: String(pane) } })
                }
                style={({ pressed }) => [styles.tab, pressed && styles.pressed]}
              >
                <View style={styles.tabText}>
                  <Text style={styles.tabName} numberOfLines={1}>
                    {tabTitle(t)}
                  </Text>
                  <Text style={styles.tabLast} numberOfLines={1}>
                    {tabLast(t) || " "}
                  </Text>
                </View>
                {st ? <Text style={[styles.state, { color: st.color }]}>{st.label}</Text> : null}
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
  const now = useNow(60_000);
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
    <TabScreen title="Computers" accessory={pair} gap={10} large>
      {s.hosts.length === 0 && s.ready ? <Text style={styles.empty}>No computer paired yet.</Text> : null}
      {s.hosts.map((h) => (
        <Computer key={h.record.key} host={h} now={now} open={open.has(h.record.key)} onToggle={() => toggle(h.record.key)} />
      ))}
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
  tab: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    paddingHorizontal: 14,
    minHeight: 52,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  tabText: { flex: 1, minWidth: 0, gap: 2 },
  tabName: { fontFamily: ui(400), color: colors.textBright, fontSize: 14 },
  tabLast: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  state: { fontFamily: ui(400), fontSize: 12 },
  inset: { marginHorizontal: 14, marginBottom: 12, gap: 8 },
  why: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
});
