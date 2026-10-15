import { StyleSheet, Text, View } from "react-native";
import { releaseName, type Release } from "@/changelog";
import { colors, radius, ui } from "./theme";

/** The release's date as people read it: `2026-10-15` → `Oct 15, 2026`. */
function releaseDate(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  if (!y || !m || !d) return date;
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/** One release's notes: its name and date, the summary, then each group's items. */
export function ReleaseCard({ release }: { release: Release }) {
  return (
    <View style={styles.card}>
      <View style={styles.head} accessible accessibilityRole="header">
        <Text style={styles.name}>{releaseName(release)}</Text>
        <Text style={styles.date}>{releaseDate(release.date)}</Text>
      </View>
      {release.summary ? <Text style={styles.summary}>{release.summary}</Text> : null}
      {release.groups.map((g) => (
        <View key={g.title} style={styles.group}>
          <Text style={styles.groupTitle} accessibilityRole="header">
            {g.title}
          </Text>
          {g.items.map((it) => (
            <Text key={it.title} style={styles.item}>
              <Text style={styles.itemTitle}>{it.title}</Text> — {it.text}
            </Text>
          ))}
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    padding: 16,
    gap: 12,
  },
  head: { flexDirection: "row", alignItems: "baseline", justifyContent: "space-between", gap: 12, flexWrap: "wrap" },
  name: { fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  date: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  summary: { fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20 },
  group: { gap: 6 },
  groupTitle: {
    fontFamily: ui(500),
    color: colors.textDim,
    fontSize: 12,
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  item: { fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20 },
  itemTitle: { fontFamily: ui(600), color: colors.textBright },
});
