import { memo, useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View, type LayoutChangeEvent } from "react-native";
import { firstLine, parseTable } from "@/paneView";
import type { Block } from "@/protocol/types";
import { CardFrame, CardHead, Pill } from "./BlockCards";
import { formatDuration } from "./format";
import { colors, mono, ui } from "./theme";

/**
 * Whether a touch is on output that scrolls sideways: the pane's swipe between panes leaves that
 * drag to the scroll (the native scroll moves whatever the responders decide).
 */
export const sideways = { touching: false };

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
}: {
  text: string;
  wrap: boolean;
  ground: string;
  size?: number;
  lineHeight?: number;
}) {
  const [view, setView] = useState({ width: 0, content: 0, x: 0 });
  const look = { fontSize: size, lineHeight };
  if (wrap) {
    return (
      <Text style={[styles.output, look]} selectable>
        {text}
      </Text>
    );
  }
  const scrolls = view.content > view.width + 1;
  const thumb = scrolls ? Math.max(0.1, view.width / view.content) : 1;
  const at = scrolls ? Math.min(1, view.x / (view.content - view.width)) : 0;
  return (
    <View style={styles.sideways}>
      <View
        accessible
        accessibilityLabel={text}
        accessibilityHint="Scrolls sideways"
        onTouchStart={() => (sideways.touching = scrolls)}
        onTouchEnd={() => (sideways.touching = false)}
        onTouchCancel={() => (sideways.touching = false)}
      >
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
          <Text style={[styles.output, look]} selectable>
            {text}
          </Text>
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
}

/**
 * A command that ended without a failure (the Pane board): its first output line under the
 * command, "✓ 1.2s" at the right. Output that is a table (`docker ps`) is a list with a
 * "Raw" / "As list" pill. The whole output is in the block sheet.
 */
export const BlockView = memo(function BlockView({ block, onActions, wrap }: Props) {
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
      head={<CardHead command={block.command} status={status} color={block.exit === null ? colors.textMuted : colors.success} />}
      pills={rows ? <Pill label={raw ? "As list" : "Raw"} color={colors.textMuted} onPress={() => setRaw((r) => !r)} /> : undefined}
    >
      {rows && !raw ? (
        <View accessibilityRole="list" style={styles.list}>
          {rows.map((r, i) => (
            <View key={i} accessible accessibilityLabel={[r.name, r.meta, r.state].filter(Boolean).join(", ")} style={styles.row}>
              <View style={styles.dot} />
              <View style={styles.rowText}>
                <Text style={styles.rowName}>{r.name}</Text>
                {r.meta ? <Text style={styles.rowMeta}>{r.meta}</Text> : null}
              </View>
              {r.state ? <Text style={styles.rowState}>{r.state}</Text> : null}
            </View>
          ))}
        </View>
      ) : rows ? (
        <Output text={block.output.trimEnd()} wrap={wrap} ground={colors.surface} size={11.5} lineHeight={17} />
      ) : line ? (
        <Text style={styles.line} numberOfLines={2}>
          {line}
        </Text>
      ) : null}
    </CardFrame>
  );
});

const styles = StyleSheet.create({
  output: { color: colors.fg, fontFamily: mono },
  sideways: { gap: 5 },
  fade: { position: "absolute", top: 0, bottom: 0, right: 0, width: 28, flexDirection: "row" },
  fadeStep: { flex: 1 },
  track: { height: 3, borderRadius: 2, backgroundColor: colors.hairline },
  thumb: { position: "absolute", top: 0, bottom: 0, borderRadius: 2, backgroundColor: colors.textMuted },
  line: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 19 },
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
});
