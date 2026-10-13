import { StyleSheet, Text, View } from "react-native";
import type { ConnState } from "@/protocol/client";
import { describeConn } from "./connLabel";
import { colors } from "./theme";

/**
 * The live connection status with its path ("Connected · LAN · 192.168.1.29"). As the pane's
 * `banner` it is a full-width strip; screen readers hear changes politely.
 */
export function StatusLine({ conn, banner = false }: { conn: ConnState; banner?: boolean }) {
  const { label, color } = describeConn(conn);
  return (
    <View
      style={[styles.row, banner && styles.banner]}
      accessible
      accessibilityRole="text"
      accessibilityLiveRegion="polite"
      accessibilityLabel={`Connection: ${label}`}
    >
      <View style={[styles.dot, { backgroundColor: color }]} />
      <Text style={[styles.text, banner && styles.bannerText]}>{label}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", gap: 8, paddingVertical: 6 },
  banner: {
    paddingHorizontal: 16,
    backgroundColor: colors.chrome,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.hairline,
  },
  dot: { width: 8, height: 8, borderRadius: 4 },
  text: { color: colors.textMuted, fontSize: 13, flexShrink: 1 },
  bannerText: { fontSize: 12 },
});
