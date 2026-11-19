import { useRouter } from "expo-router";
import { useEffect } from "react";
import { Pressable, Text, View } from "react-native";
import type { ConnState } from "@/protocol/client";
import { fleet, useApp } from "@/runtime";
import { replacedPairing } from "@/stores";
import { Button } from "./Button";
import { hostLine, lastSeenAt, pathLine, pathRows, type LineTone, type PathTone } from "./connStatus";
import { Icon } from "./Icon";
import { glyph } from "./Screen";
import { SheetFrame } from "./SheetFrame";
import { fromPalette, radius, ui, makeStyles, useColors } from "./theme";
import { useNow } from "./useNow";

export const lineColor = fromPalette((colors): Record<LineTone, string> => ({
  muted: colors.textMuted,
  busy: colors.accent,
  bad: colors.error,
  // textDim is 4.3:1 on the sheet and the card; the muted grey keeps AA
  dim: colors.textMuted,
}));

const noteColor = fromPalette((colors): Record<PathTone, string> => ({
  ok: colors.textMuted,
  busy: colors.accent,
  bad: colors.error,
  off: colors.textMuted,
}));

const TONE_WORD: Record<PathTone, string> = { ok: "in use", busy: "connecting", bad: "failed", off: "not used" };

/** The speed row's line: the last ping's round trip once one came back. */
const speedLine = (conn: Pick<ConnState, "paths">) => {
  const ms = conn.paths.find((p) => p.state === "ok")?.latencyMs;
  return ms ? `Last round trip ${ms} ms` : "Round trip, traffic and typing delay";
};

/**
 * "Connection" (the New Portals board): how a computer is being reached, one row per path (this
 * network's address, the relay) with where and why in one line, the one in use ticked; that it
 * is end-to-end encrypted; "Try again now" when it cannot be reached, "Test again" while
 * connected, at the sheet's foot. Opened from Portals' status line and the computer menu.
 */
export function ConnectionSheet({ hostKey }: { hostKey: string }) {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === hostKey);
  const ticking = host?.conn.status === "closed";
  const now = useNow(1000, ticking);
  const gone = !host;
  const close = () => (router.canGoBack() ? router.back() : router.replace("/"));
  // the computer was unpaired while its sheet was open: nothing left to show
  useEffect(() => {
    if (gone) close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gone]);
  if (!host) return null;
  const tabs = host.conn.status === "open" ? host.windows.reduce((n, w) => n + w.tabs.length, 0) : null;
  const line = hostLine(host.conn, now, tabs, lastSeenAt(host), replacedPairing(host.record, s.hosts.map((h) => h.record)));
  const open = host.conn.status === "open";
  const rows = pathRows(host.conn);
  return (
    // the board's height, Test again (Try again now) pinned 24 pt above the screen's end
    <SheetFrame
      board="connection"
      title="Connection"
      subtitle={`${host.record.name} · ${open ? "connected" : line.text}`}
      subtitleColor={open ? colors.textMuted : lineColor(colors)[line.tone]}
      onClose={close}
      footer={
        // Connected: Test again tries every path again, this network first (a phone back home leaves the relay)
        open ? (
          <Button label="Test again" accessibilityHint="Connects again, trying this network first" onPress={() => fleet.retry(hostKey, { lanFirst: true })} />
        ) : line.retry ? (
          <Button label="Try again now" variant="primary" onPress={() => fleet.retry(hostKey)} />
        ) : null
      }
    >
      <View style={styles.card}>
        {rows.map((r) => (
          <View key={r.key} style={[styles.row, styles.rule]} accessible accessibilityLabel={`${r.title}, ${r.addr}, ${TONE_WORD[r.tone]}. ${r.note}`}>
            <View style={styles.rowText}>
              <Text style={styles.rowTitle}>{r.title}</Text>
              <Text style={[styles.line, { color: noteColor(colors)[r.tone] }]}>{pathLine(r)}</Text>
            </View>
            {r.tone === "ok" ? <Icon name="check" size={glyph(16, 11)} color={colors.accent} /> : null}
          </View>
        ))}
        {open ? (
          <Pressable
            style={({ pressed }) => [styles.row, styles.rule, pressed && { backgroundColor: colors.faint }]}
            accessibilityRole="button"
            accessibilityLabel={`Connection speed, ${speedLine(host.conn)}`}
            accessibilityHint="Round trip, traffic, and how long typing and scrolling take to show"
            onPress={() => router.push({ pathname: "/speed", params: { key: hostKey } })}
          >
            <View style={styles.lockTile}>
              <Icon name="wifi" size={glyph(12, 11)} color={colors.textMuted} />
            </View>
            <View style={styles.rowText}>
              <Text style={styles.rowTitle}>Connection speed</Text>
              <Text style={styles.line}>{speedLine(host.conn)}</Text>
            </View>
            <Icon name="chevronRight" size={glyph(14, 11)} color={colors.textMuted} />
          </Pressable>
        ) : null}
        <View style={styles.row} accessible>
          <View style={styles.lockTile}>
            <Icon name="lock" size={glyph(12, 11)} color={colors.textMuted} />
          </View>
          <View style={styles.rowText}>
            <Text style={styles.rowTitle}>End-to-end encrypted</Text>
            <Text style={styles.line}>The relay passes bytes it cannot read.</Text>
          </View>
        </View>
      </View>
    </SheetFrame>
  );
}

// The board: a 14 pt card 16 pt in, 68 pt rows (52 and 8 above and under), Test again 48 pt at the foot.
const useStyles = makeStyles((colors) => ({
  card: { borderWidth: 1, borderColor: colors.hairline, borderRadius: radius.card, overflow: "hidden", maxWidth: 560, alignSelf: "stretch" },
  row: { minHeight: 68, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 16, paddingVertical: 8 },
  rule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  rowText: { flex: 1, minWidth: 0, gap: 2 },
  rowTitle: { fontFamily: ui(400), color: colors.text, fontSize: 16, lineHeight: 21 },
  line: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  lockTile: { width: 28, height: 28, borderRadius: 8, backgroundColor: colors.faint, alignItems: "center", justifyContent: "center" },
}));
