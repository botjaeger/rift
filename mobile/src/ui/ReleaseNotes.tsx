import { Fragment } from "react";
import { StyleSheet, Text, View } from "react-native";
import { releaseKey, releaseName, type Release } from "@/changelog";
import { colors, mono, tints, ui } from "./theme";

/** The release's date as people read it: `2026-10-15` → `Oct 15, 2026`. */
function releaseDate(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  if (!y || !m || !d) return date;
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/** `v0.2.0 · update 1 · Oct 15, 2026`: a release as Rift's What's New names it. */
export const releaseLabel = (r: Release) => `v${releaseName(r)} · ${releaseDate(r.date)}`;

/** A group's color, as in Rift's What's New: meaning, not decoration. */
function groupColor(title: string): string {
  switch (title.toLowerCase()) {
    case "new":
    case "added":
      return colors.accent;
    case "improved":
    case "changed":
      return colors.success;
    case "fixed":
      return colors.warning;
    default:
      return colors.textMuted;
  }
}

/** The version pill beside a What's New title (Rift's header pill). */
export function ReleasePill({ release }: { release: Release }) {
  return (
    <View style={styles.pill}>
      <Text style={styles.pillText}>{releaseLabel(release)}</Text>
    </View>
  );
}

/**
 * Releases as Rift's What's New lays them out: each one's summary, then its groups titled in
 * their color with a matching dot per item; a rule and the release's name between releases.
 * `named` also names the first release (the sheet names it in its header pill instead).
 */
export function ReleaseList({ releases, named }: { releases: readonly Release[]; named: boolean }) {
  return (
    <View>
      {releases.map((r, i) => (
        <Fragment key={releaseKey(r.version, r.update)}>
          {i > 0 ? <View style={styles.rule} /> : null}
          {i > 0 || named ? (
            <Text style={styles.name} accessibilityRole="header">
              {releaseLabel(r)}
            </Text>
          ) : null}
          {r.summary ? <Text style={styles.summary}>{r.summary}</Text> : null}
          {r.groups.map((g) => {
            const color = groupColor(g.title);
            return (
              <View key={g.title} style={styles.group}>
                <Text style={[styles.groupTitle, { color }]} accessibilityRole="header">
                  {g.title}
                </Text>
                {g.items.map((it) => (
                  <View key={it.title} style={styles.item}>
                    <View style={[styles.dot, { backgroundColor: color }]} />
                    <Text style={styles.itemText}>
                      <Text style={styles.itemTitle}>{it.title}</Text>
                      <Text style={styles.dash}> — </Text>
                      {it.text}
                    </Text>
                  </View>
                ))}
              </View>
            );
          })}
        </Fragment>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  pill: { alignSelf: "flex-start", backgroundColor: tints.accentBadge, borderRadius: 4, paddingHorizontal: 8, paddingVertical: 3 },
  pillText: { fontFamily: mono, color: colors.accent, fontSize: 12 },
  rule: { height: 1, backgroundColor: colors.hairline, marginTop: 6, marginBottom: 20 },
  name: { fontFamily: ui(600), color: colors.textBright, fontSize: 15, marginBottom: 12 },
  summary: { fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20, marginBottom: 16 },
  group: { marginBottom: 10 },
  groupTitle: { fontFamily: ui(600), fontSize: 12, letterSpacing: 0.5, textTransform: "uppercase", marginBottom: 10 },
  item: { flexDirection: "row", gap: 10, marginBottom: 10 },
  dot: { width: 8, height: 8, borderRadius: 4, marginTop: 6 },
  itemText: { flex: 1, fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20 },
  itemTitle: { fontFamily: ui(600), color: colors.textBright },
  dash: { color: colors.textMuted },
});
