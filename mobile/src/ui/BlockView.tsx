import { memo, useMemo, useState } from "react";
import { ScrollView, Text, View, type LayoutChangeEvent } from "react-native";
import { firstLine, parseTable } from "@/paneView";
import type { Block } from "@/protocol/types";
import { CardFrame, CardHead } from "./BlockCards";
import { Button } from "./Button";
import { formatDuration } from "./format";
import { Marked } from "./Marked";
import { SelectableText } from "./SelectableText";
import { mono, ui, makeStyles, useColors } from "./theme";

/**
 * Output text: wrapped to the width, or (wrap off) as printed, scrolling sideways with a fade
 * at the right edge and a thin position bar under it (the PaneNoWrap board).
 */
export function Output({
  text,
  wrap,
  ground,
  size = 12,
  lineHeight = 17.4,
  mark,
}: {
  text: string;
  wrap: boolean;
  ground: string;
  size?: number;
  lineHeight?: number;
  /** The Search output query: its matches are marked. */
  mark?: string;
}) {
  const styles = useStyles();
  const [view, setView] = useState({ width: 0, content: 0, x: 0 });
  if (wrap) {
    return (
      <SelectableText text={text} style={{ ...styles.output, fontSize: size }} lineHeight={lineHeight} mark={mark} />
    );
  }
  const scrolls = view.content > view.width + 1;
  const thumb = scrolls ? Math.max(0.1, view.width / view.content) : 1;
  const at = scrolls ? Math.min(1, view.x / (view.content - view.width)) : 0;
  return (
    <View style={styles.sideways}>
      <View accessible accessibilityLabel={text} accessibilityHint="Scrolls sideways">
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          onLayout={(e: LayoutChangeEvent) => {
            const width = e.nativeEvent.layout.width;
            setView((v) => (v.width === width ? v : { ...v, width }));
          }}
          onContentSizeChange={(content) => setView((v) => (v.content === content ? v : { ...v, content }))}
          onScroll={(e) => {
            const x = e.nativeEvent.contentOffset.x;
            setView((v) => ({ ...v, x }));
          }}
          scrollEventThrottle={32}
        >
          <SelectableText text={text} style={{ ...styles.output, fontSize: size }} lineHeight={lineHeight} mark={mark} />
        </ScrollView>
        {scrolls && at < 1 ? (
          <View pointerEvents="none" style={styles.fade}>
            {[0.15, 0.35, 0.6, 0.85].map((o) => (
              <View key={o} style={[styles.fadeStep, { opacity: o, backgroundColor: ground }]} />
            ))}
          </View>
        ) : null}
      </View>
      {scrolls ? (
        <View style={styles.track} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
          <View style={[styles.thumb, { width: `${thumb * 100}%`, left: `${at * (1 - thumb) * 100}%` }]} />
        </View>
      ) : null}
    </View>
  );
}

interface Props {
  block: Block;
  /** Opens the block sheet. */
  onActions: (id: number) => void;
  /** Wrap long lines; off: the raw output scrolls sideways. */
  wrap: boolean;
  /** The Search output query: its matches in the command and output are marked. */
  mark?: string;
}

/**
 * A command that ended without a failure (the Pane board): its first output line under the
 * command, "✓ 1.2s" at the right. Output that is a table (`docker ps`) is a list with a
 * "Raw" / "As list" pill. The whole output is in the block sheet.
 */
export const BlockView = memo(function BlockView({ block, onActions, wrap, mark }: Props) {
  const colors = useColors();
  const styles = useStyles();
  const [raw, setRaw] = useState(false);
  // the tail of a long output may have lost its header: only whole output is a table
  const rows = useMemo(() => (block.dropped === 0 ? parseTable(block.output) : null), [block.dropped, block.output]);
  const line = firstLine(block.output);
  const status = block.exit === null ? "done" : `✓ ${formatDuration(block.duration_ms)}`;
  const state = block.exit === null ? "finished" : "succeeded";
  return (
    <CardFrame
      ground={colors.surface}
      border={block.exit === null ? colors.hairline : `${colors.success}55`}
      label={`${block.command || "Command"}, ${state}${block.restored ? ", restored from an earlier session" : ""}`}
      onOpen={() => onActions(block.id)}
      head={<CardHead command={block.command} status={status} color={block.exit === null ? colors.textMuted : colors.success} mark={mark} />}
      pills={rows ? <Button label={raw ? "As list" : "Raw"} size="small" onPress={() => setRaw((r) => !r)} /> : undefined}
    >
      {rows && !raw ? (
        <View accessibilityRole="list" style={styles.list}>
          {rows.map((r, i) => (
            <View key={i} accessible accessibilityLabel={[r.name, r.meta, r.state].filter(Boolean).join(", ")} style={styles.row}>
              <View style={styles.dot} />
              <View style={styles.rowText}>
                <Text style={styles.rowName}>
                  <Marked text={r.name} query={mark} />
                </Text>
                {r.meta ? (
                  <Text style={styles.rowMeta}>
                    <Marked text={r.meta} query={mark} />
                  </Text>
                ) : null}
              </View>
              {r.state ? (
                <Text style={styles.rowState}>
                  <Marked text={r.state} query={mark} />
                </Text>
              ) : null}
            </View>
          ))}
        </View>
      ) : rows ? (
        <Output text={block.output.trimEnd()} wrap={wrap} ground={colors.surface} size={11.5} lineHeight={17} mark={mark} />
      ) : line ? (
        <SelectableText text={line} style={styles.line} lineHeight={17} lines={2} mark={mark} />
      ) : null}
    </CardFrame>
  );
});

const useStyles = makeStyles((colors) => ({
  output: { color: colors.fg, fontFamily: mono },
  sideways: { gap: 5 },
  fade: { position: "absolute", top: 0, bottom: 0, right: 0, width: 28, flexDirection: "row" },
  fadeStep: { flex: 1 },
  track: { height: 3, borderRadius: 2, backgroundColor: colors.hairline },
  thumb: { position: "absolute", top: 0, bottom: 0, borderRadius: 2, backgroundColor: colors.textMuted },
  // the first output line: JetBrains Mono 12/17 in the text colour (the New Blocks board)
  line: { fontFamily: mono, color: colors.text, fontSize: 12 },
  // the list runs edge to edge in the card: its rows carry the card's 12 pt gutter
  list: { marginHorizontal: -12 },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingVertical: 8,
    paddingHorizontal: 12,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  dot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.success },
  rowText: { flex: 1, minWidth: 0, gap: 2 },
  rowName: { fontFamily: mono, color: colors.textBright, fontSize: 14 },
  // textMuted: the mockup's textDim is 4.49:1 on a surface (palette.ts)
  rowMeta: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  rowState: { fontFamily: ui(400), color: colors.success, fontSize: 12 },
}));
