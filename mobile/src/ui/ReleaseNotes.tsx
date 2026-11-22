import { Fragment } from "react";
import { Text, View } from "react-native";
import { releaseKey, releaseName, type Release } from "@/changelog";
import { ui, makeStyles, useColors, type Palette } from "./theme";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** The release's day as the boards write it: `2026-11-07` → `7 Nov` (the year is left out). */
export function releaseDay(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  if (!y || !m || !d || m > 12) return date;
  return `${d} ${MONTHS[m - 1]}`;
}

/** `0.8.0 · update 2 · 7 Nov`: a release as the What's new sheet names it. */
export const releaseLabel = (r: Release) => `${releaseName(r)} · ${releaseDay(r.date)}`;

/** A group's color, as in Rift's What's New: meaning, not decoration. */
function groupColor(title: string, colors: Palette): string {
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

/**
 * Releases as the What's new boards lay them out, 10 pt apart: each one's name ("0.8.0 · update
 * 1" and its day, muted) and summary, then its groups titled in their colour, each item one
 * paragraph 15/22 led by a dot in that colour; a rule between releases. `named` also names the
 * first release (the sheet names it in its header's subtitle instead); `sheet`: the sheet's
 * larger group titles.
 */
export function ReleaseList({ releases, named, sheet }: { releases: readonly Release[]; named: boolean; sheet?: boolean }) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <View style={styles.list}>
      {releases.map((r, i) => (
        <Fragment key={releaseKey(r.version, r.update)}>
          {i > 0 ? <View style={styles.rule} /> : null}
          {i > 0 || named ? (
            <Text style={styles.name} accessibilityRole="header">
              {releaseName(r)} <Text style={styles.day}>{releaseDay(r.date)}</Text>
            </Text>
          ) : null}
          {r.summary ? <Text style={styles.summary}>{r.summary}</Text> : null}
          {r.groups.map((g, gi) => {
            const color = groupColor(g.title, colors);
            return (
              // by place: a release can list a group title twice (two "Improved")
              <Fragment key={`${gi}:${g.title}`}>
                <Text style={[styles.groupTitle, sheet && styles.sheetGroupTitle, { color }]} accessibilityRole="header">
                  {g.title}
                </Text>
                {g.items.map((it, ii) => (
                  <Text key={`${ii}:${it.title}`} style={styles.item} accessibilityLabel={`${it.title} — ${it.text}`}>
                    {/* a line's inline view takes no margin: its box holds the 8 pt after the dot */}
                    <View style={styles.lead}>
                      <View style={[styles.dot, { backgroundColor: color }]} />
                    </View>
                    <Text style={styles.itemTitle}>{it.title}</Text> — {it.text}
                  </Text>
                ))}
              </Fragment>
            );
          })}
        </Fragment>
      ))}
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  list: { gap: 10 },
  rule: { height: 1, backgroundColor: colors.hairline, marginVertical: 6 },
  name: { fontFamily: ui(600), color: colors.textBright, fontSize: 17, lineHeight: 22 },
  day: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  summary: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  groupTitle: { fontFamily: ui(600), fontSize: 12, lineHeight: 16, textTransform: "uppercase" },
  sheetGroupTitle: { fontFamily: ui(600), fontSize: 13, lineHeight: 18 },
  // the dot sits in the line, 2 pt over its baseline, 8 pt before the title
  lead: { width: 14, height: 8, justifyContent: "flex-start" },
  dot: { width: 6, height: 6, borderRadius: 3 },
  item: { fontFamily: ui(400), color: colors.text, fontSize: 15, lineHeight: 22 },
  itemTitle: { fontFamily: ui(600), color: colors.textBright },
}));
