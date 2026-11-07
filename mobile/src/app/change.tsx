import { useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { askContext, askerName, diffRowText, hunkHeader, type DiffRow } from "@/paneView";
import { useApp } from "@/runtime";
import { findPane } from "@/topology";
import { PageHeader } from "@/ui/Screen";
import { mono, tint, makeStyles } from "@/ui/theme";

/** One line of the change, edge to edge on its tint (the board's 20 pt rows). */
function Line({ row }: { row: DiffRow }) {
  const styles = useStyles();
  return (
    <Text style={[styles.line, row.kind === "added" && styles.added, row.kind === "removed" && styles.removed]}>
      {diffRowText(row) || " "}
    </Text>
  );
}

/**
 * "A change in full" (the New Screen board), from a question card's Full diff: the file the agent
 * asks to change, "+3 −1 · from Claude Code", the hunk header strip, then every line of the change
 * (kept ones too) edge to edge. It shows the question as it was opened: answered meanwhile, the
 * change stays readable until Back.
 */
export default function ChangeScreen() {
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const p = useLocalSearchParams<{ key: string; pane: string; program?: string }>();
  const host = s.hosts.find((h) => h.record.key === p.key);
  const live = host ? findPane(host.windows, Number(p.pane))?.pane.waiting : undefined;
  // latched the first time the question is known (a restored route opens this page before the
  // host's panes load), then kept: answered meanwhile, the change stays readable
  const [seen, setSeen] = useState<string[] | null>(live ? live.context : null);
  if (seen === null && live) setSeen(live.context);
  const context = seen ?? [];
  const ctx = askContext(context);
  const hunk = hunkHeader(ctx);
  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));
  return (
    <View style={styles.fill}>
      <PageHeader
        title={ctx.file ?? "Changes"}
        subtitle={`+${ctx.added} −${ctx.removed} · from ${askerName(p.program || null)}`}
        back={back}
      />
      <ScrollView style={styles.fill} contentContainerStyle={{ paddingBottom: insets.bottom + 24 }}>
        {hunk ? <Text style={[styles.line, styles.hunk]}>{hunk}</Text> : null}
        {ctx.rows.map((r, i) => (
          <Line key={i} row={r} />
        ))}
      </ScrollView>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  // the program's black, as the screen the change came from
  fill: { flex: 1, backgroundColor: colors.chrome },
  line: { fontFamily: mono, color: colors.text, fontSize: 12, lineHeight: 20, paddingHorizontal: 14 },
  hunk: { color: colors.textMuted, backgroundColor: tint(colors.textBright, 0x0a) },
  added: { backgroundColor: tint(colors.success, 0x1f) },
  removed: { backgroundColor: tint(colors.error, 0x26) },
}));
