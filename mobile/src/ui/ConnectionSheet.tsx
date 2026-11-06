import { useRouter } from "expo-router";
import { useEffect } from "react";
import { Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { fleet, useApp } from "@/runtime";
import { Button } from "./Button";
import { hostLine, lastSeenAt, pathRows, type LineTone, type PathTone } from "./connStatus";
import { Icon } from "./Icon";
import { SheetHeader } from "./SheetHeader";
import { fromPalette, mono, radius, tint, ui, makeStyles, useColors } from "./theme";
import { useNow } from "./useNow";

export const lineColor = fromPalette((colors): Record<LineTone, string> => ({
  muted: colors.textMuted,
  busy: colors.accent,
  bad: colors.error,
  // textDim is 4.3:1 on the sheet and the card; the muted grey keeps AA
  dim: colors.textMuted,
}));

const toneColor = fromPalette((colors): Record<PathTone, string> => ({
  ok: colors.success,
  busy: colors.accent,
  bad: colors.error,
  off: colors.textMuted,
}));

const noteColor = fromPalette((colors): Record<PathTone, string> => ({
  ok: colors.textMuted,
  busy: colors.accent,
  bad: colors.error,
  off: colors.textMuted,
}));

const TONE_WORD: Record<PathTone, string> = { ok: "in use", busy: "connecting", bad: "failed", off: "not used" };

/**
 * "Connection" (the RelayPhone board): how a computer is being reached, one row per path (this
 * network's address, the relay) with its state and why; "Try again now" when it cannot be
 * reached, "Test again" while connected. Shared by Portals' status line and the pane's status strip, which open it as a sheet.
 */
export function ConnectionSheet({ hostKey }: { hostKey: string }) {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
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
  const line = hostLine(host.conn, now, tabs, lastSeenAt(host));
  const open = host.conn.status === "open";
  const rows = pathRows(host.conn);
  return (
    <View style={[styles.sheet, { paddingBottom: Math.max(28, insets.bottom + 2) }]}>
      <SheetHeader title="Connection" subtitle={`${host.record.name} · ${line.text}`} subtitleColor={lineColor(colors)[line.tone]} onClose={close} />
      <View style={styles.content}>
        <View style={styles.group}>
          {rows.map((r) => (
            <View key={r.key} style={styles.row} accessible accessibilityLabel={`${r.title}, ${r.addr}, ${TONE_WORD[r.tone]}. ${r.note}`}>
              <View style={[styles.badge, { backgroundColor: tint(toneColor(colors)[r.tone], 0x26) }]}>
                <Text style={[styles.mark, { color: toneColor(colors)[r.tone] }]}>{r.mark}</Text>
              </View>
              <View style={styles.rowText}>
                <Text style={styles.rowTitle}>{r.title}</Text>
                <Text style={styles.addr}>{r.addr}</Text>
                <Text style={[styles.note, { color: noteColor(colors)[r.tone] }]}>{r.note}</Text>
              </View>
            </View>
          ))}
          <View style={styles.encrypted}>
            <View style={[styles.badge, styles.lockBadge]}>
              <Icon name="lock" size={12} color={colors.textMuted} />
            </View>
            <Text style={styles.encryptedText}>End-to-end encrypted. The relay passes bytes it cannot read.</Text>
          </View>
        </View>
        {/* connected: Test again tries every path again, this network first (a phone back home leaves the relay) */}
        {open ? (
          <Button label="Test again" accessibilityHint="Connects again, trying this network first" onPress={() => fleet.retry(hostKey, { lanFirst: true })} />
        ) : line.retry ? (
          <Button label="Try again now" variant="primary" onPress={() => fleet.retry(hostKey)} />
        ) : null}
      </View>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  sheet: { backgroundColor: colors.surface },
  content: { paddingHorizontal: 16, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  group: { borderWidth: 1, borderColor: colors.hairline, borderRadius: radius.card },
  row: { flexDirection: "row", alignItems: "flex-start", gap: 12, padding: 12, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  badge: { width: 22, height: 22, borderRadius: 11, alignItems: "center", justifyContent: "center", marginTop: 1 },
  mark: { fontFamily: ui(500), fontSize: 12 },
  rowText: { flex: 1, minWidth: 0, gap: 3 },
  rowTitle: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  addr: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  note: { fontFamily: ui(400), fontSize: 13, lineHeight: 18 },
  encrypted: { flexDirection: "row", alignItems: "center", gap: 12, padding: 12 },
  lockBadge: { backgroundColor: colors.glass, marginTop: 0 },
  encryptedText: { flex: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
}));
