import { useRouter } from "expo-router";
import { useEffect } from "react";
import { StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { fleet, useApp } from "@/runtime";
import { Button } from "./Button";
import { hostLine, pathRows, type LineTone, type PathTone } from "./connStatus";
import { Icon } from "./Icon";
import { IconButton } from "./Screen";
import { colors, mono, tint, ui } from "./theme";
import { useNow } from "./useNow";

export const LINE_COLOR: Record<LineTone, string> = {
  muted: colors.textMuted,
  busy: colors.accent,
  bad: colors.error,
  // textDim is 4.3:1 on the sheet and the card; the muted grey keeps AA
  dim: colors.textMuted,
};

const TONE_COLOR: Record<PathTone, string> = {
  ok: colors.success,
  busy: colors.accent,
  bad: colors.error,
  off: colors.textMuted,
};

const NOTE_COLOR: Record<PathTone, string> = {
  ok: colors.textMuted,
  busy: colors.accent,
  bad: colors.error,
  off: colors.textMuted,
};

const TONE_WORD: Record<PathTone, string> = { ok: "in use", busy: "connecting", bad: "failed", off: "not used" };

/**
 * "Connection" (the RelayPhone board): how a computer is being reached, one row per path (this
 * network's address, the relay) with its state and why, and "Try again now" when it cannot be
 * reached. Shared by Portals' status line and the pane's status strip, which open it as a sheet.
 */
export function ConnectionSheet({ hostKey }: { hostKey: string }) {
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
  const line = hostLine(host.conn, now, tabs);
  const rows = pathRows(host.conn);
  return (
    <View style={[styles.content, { paddingBottom: Math.max(28, insets.bottom + 2) }]}>
      <View style={styles.head}>
        <View style={styles.headText}>
          <Text style={styles.title} accessibilityRole="header">
            Connection
          </Text>
          <Text style={[styles.line, { color: LINE_COLOR[line.tone] }]}>
            {line.text}
          </Text>
        </View>
        <IconButton glass icon="close" label="Close" onPress={close} />
      </View>
      <View style={styles.group}>
        {rows.map((r) => (
          <View key={r.key} style={styles.row} accessible accessibilityLabel={`${r.title}, ${r.addr}, ${TONE_WORD[r.tone]}. ${r.note}`}>
            <View style={[styles.badge, { backgroundColor: tint(TONE_COLOR[r.tone], 0x26) }]}>
              <Text style={[styles.mark, { color: TONE_COLOR[r.tone] }]}>{r.mark}</Text>
            </View>
            <View style={styles.rowText}>
              <Text style={styles.rowTitle}>{r.title}</Text>
              <Text style={styles.addr}>{r.addr}</Text>
              <Text style={[styles.note, { color: NOTE_COLOR[r.tone] }]}>{r.note}</Text>
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
      {line.retry ? (
        <Button
          label="Try again now"
          variant="primary"
          size="large"
          onPress={() => fleet.retry(hostKey)}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: 18, paddingHorizontal: 16, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center", backgroundColor: colors.surface },
  head: { flexDirection: "row", alignItems: "center", gap: 8 },
  headText: { flex: 1, minWidth: 0, gap: 2 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 20 },
  line: { fontFamily: ui(400), fontSize: 13 },
  group: { borderWidth: 1, borderColor: colors.hairline, borderRadius: 12 },
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
});
