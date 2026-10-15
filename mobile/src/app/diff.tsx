import { useLocalSearchParams, useRouter } from "expo-router";
import { memo, useEffect, useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { splitPath } from "@/chips";
import type { DiffLine, DiffMsg } from "@/protocol/types";
import { fleet, useLock } from "@/runtime";
import { ErrorText } from "@/ui/Notice";
import { IconButton } from "@/ui/Screen";
import { colors, mono, monoMedium, radius, tints, ui } from "@/ui/theme";

const STATUS_WORD: Record<string, string> = {
  M: "Modified",
  A: "Added",
  D: "Deleted",
  R: "Renamed",
  C: "Copied",
  T: "Type changed",
  "?": "Untracked",
  U: "Conflicted",
};

/** One diff line: a hunk header strip, or a line with its +/− mark on the review panel's washes. */
const Line = memo(function Line({ line }: { line: DiffLine }) {
  const [kind, text] = line;
  if (kind === "@")
    return (
      <Text style={[styles.code, styles.hunk]} numberOfLines={1}>
        {text}
      </Text>
    );
  const added = kind === "+";
  const removed = kind === "-";
  return (
    <View style={[styles.line, added && styles.added, removed && styles.removed]}>
      <Text style={[styles.mark, added && { color: colors.success }, removed && { color: colors.textBright }]}>
        {added ? "+" : removed ? "−" : " "}
      </Text>
      <Text style={styles.code}>{text === "" ? " " : text}</Text>
    </View>
  );
});

/**
 * A changed file's diff (the DiffView board), read-only: the desktop code review panel's hunks,
 * added lines on a green wash and removed ones on a red one with a bar at the edge, scrolling
 * both ways.
 */
export default function DiffScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { width } = useWindowDimensions();
  const lock = useLock();
  const p = useLocalSearchParams<{ key: string; pane: string; path: string; staged: string; status?: string; added?: string; removed?: string }>();
  const staged = p.staged === "1";
  const [diff, setDiff] = useState<DiffMsg | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);

  useEffect(() => {
    let live = true;
    void fleet.fileDiff(p.key, Number(p.pane), p.path, staged).then((r) => {
      if (!live) return;
      if (r.ok) setDiff(r.msg);
      else setError(r.message);
    });
    return () => {
      live = false;
    };
  }, [p.key, p.pane, p.path, staged]);

  const { name, folder } = splitPath(p.path ?? "");
  const hidden = lock.covered || lock.locked;
  const meta = [folder || null, STATUS_WORD[p.status ?? ""] ?? null, staged ? "staged" : "not staged"].filter(Boolean).join(" · ");
  return (
    // One vertical scroll view for the whole sheet: a native sheet takes its first scroll view
    // over as its own, so the header scrolls with the lines instead of sitting above them.
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      contentContainerStyle={styles.content}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <View style={styles.header}>
        <IconButton glass icon="back" label="Back to changes" onPress={() => router.back()} />
        <View style={styles.headText}>
          <Text style={styles.title} numberOfLines={1} accessibilityRole="header">
            {name}
          </Text>
          <Text style={styles.meta} numberOfLines={1}>
            {meta}
          </Text>
        </View>
        {p.added !== undefined ? (
          <Text style={styles.pill} accessibilityLabel={`${p.added} added, ${p.removed} removed`}>
            <Text style={{ color: colors.success }}>+{p.added}</Text> <Text style={{ color: colors.textBright }}>−{p.removed}</Text>
          </Text>
        ) : null}
      </View>
      <View style={[styles.pane, { paddingBottom: insets.bottom + 24 }]}>
        {error ? <ErrorText style={styles.notice}>{error}</ErrorText> : null}
        {!diff && !error ? (
          <View style={styles.loading} accessible accessibilityLabel="Loading the diff">
            <ActivityIndicator color={colors.accent} />
          </View>
        ) : null}
        {diff?.binary ? <Text style={styles.note}>A binary file: no lines to show.</Text> : null}
        {diff && !diff.binary && diff.lines.length === 0 ? <Text style={styles.note}>No lines changed (a mode or rename only).</Text> : null}
        {diff && diff.lines.length > 0 ? (
          <>
            <ScrollView horizontal accessibilityLabel="Diff, scrolls sideways">
              {/* at least the sheet's width, so a short line's wash reaches the edge */}
              <View style={{ minWidth: width }}>
                {diff.lines.map((l, i) => (
                  <Line key={i} line={l} />
                ))}
              </View>
            </ScrollView>
            {diff.truncated ? (
              <Text style={styles.note}>The diff is cut here: the phone shows 2,000 lines at most. The rest is on the computer.</Text>
            ) : null}
          </>
        ) : null}
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  content: { paddingTop: 16, flexGrow: 1 },
  hidden: { opacity: 0 },
  header: { flexDirection: "row", alignItems: "center", gap: 10, paddingLeft: 12, paddingRight: 12, paddingBottom: 12 },
  headText: { flex: 1, minWidth: 0, gap: 3 },
  title: { fontFamily: monoMedium, color: colors.textBright, fontSize: 15 },
  meta: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  pill: {
    fontFamily: mono,
    fontSize: 12,
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  pane: { flexGrow: 1, backgroundColor: colors.chrome, borderTopWidth: 1, borderTopColor: colors.hairline },
  notice: { margin: 12 },
  loading: { paddingVertical: 24 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, padding: 12 },
  line: { flexDirection: "row", borderLeftWidth: 3, borderLeftColor: "transparent", paddingRight: 10, paddingLeft: 4 },
  added: { backgroundColor: tints.diffAdded, borderLeftColor: colors.success },
  removed: { backgroundColor: tints.diffRemoved, borderLeftColor: colors.error },
  mark: { fontFamily: mono, width: 18, textAlign: "center", color: colors.textMuted, fontSize: 12, lineHeight: 17.4 },
  code: { fontFamily: mono, color: colors.fg, fontSize: 12, lineHeight: 17.4 },
  hunk: { color: colors.textMuted, backgroundColor: colors.surface, paddingVertical: 6, paddingLeft: 25, paddingRight: 10 },
});
