import { memo, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import type { Block } from "@/protocol/types";
import { formatDuration, tailLines } from "./format";
import { colors, mono, radius, TOUCH } from "./theme";

/** Output longer than this shows only its last lines until "Show all". */
export const COLLAPSED_LINES = 40;

/** `color` is the edge; `text` the badge (red is too dark for text on a surface, palette.ts). */
function stateOf(block: Block): { color: string; text: string; badge: string; spoken: string } {
  if (block.state === "running") return { color: colors.accent, text: colors.accent, badge: "", spoken: "running" };
  if (block.exit === null) return { color: colors.textDim, text: colors.textMuted, badge: "done", spoken: "done" };
  if (block.exit === 0) return { color: colors.success, text: colors.success, badge: "✓ 0", spoken: "succeeded, exit 0" };
  return { color: colors.error, text: colors.textBright, badge: `✕ ${block.exit}`, spoken: `failed, exit ${block.exit}` };
}

/** Elapsed time of a running block, ticking locally between gateway updates. */
function Elapsed({ ms }: { ms: number }) {
  const [base, setBase] = useState(ms);
  const [extra, setExtra] = useState(0);
  if (base !== ms) {
    // A fresh duration from the gateway restarts the local count.
    setBase(ms);
    setExtra(0);
  }
  useEffect(() => {
    const t = setInterval(() => setExtra((n) => n + 1000), 1000);
    return () => clearInterval(t);
  }, []);
  return <Text style={[styles.badge, { color: colors.accent }]}>{formatDuration(ms + extra)}</Text>;
}

interface Props {
  block: Block;
  expanded: boolean;
  onToggle: (id: number) => void;
}

export const BlockView = memo(function BlockView({ block, expanded, onToggle }: Props) {
  const st = stateOf(block);
  const { shown, hidden } = expanded ? tailLines(block.output, Infinity) : tailLines(block.output, COLLAPSED_LINES);
  return (
    <View style={styles.card}>
      <View style={[styles.edge, { backgroundColor: st.color }]} />
      <View style={styles.body}>
        {/* The command, its result and where it ran are one heading, so screen-reader users can
            jump from block to block, and the element is large enough to find by touch. */}
        <View
          style={styles.heading}
          accessible
          accessibilityRole="header"
          accessibilityLabel={[
            `Command ${block.command || "(empty)"}, ${st.spoken}, ${formatDuration(block.duration_ms)}`,
            block.cwd ? `in ${block.cwd}` : null,
            block.branch ? `on ${block.branch}` : null,
            block.restored ? "restored from an earlier session" : null,
          ]
            .filter(Boolean)
            .join(", ")}
        >
          <View style={styles.header}>
            <Text style={styles.command} selectable>
              {block.command || " "}
            </Text>
            {block.state === "running" ? (
              <Elapsed ms={block.duration_ms} />
            ) : (
              <Text style={[styles.badge, { color: st.text }]}>
                {st.badge} · {formatDuration(block.duration_ms)}
              </Text>
            )}
          </View>
          {block.cwd || block.restored ? (
            <Text style={styles.meta}>
              {[block.cwd, block.branch, block.restored ? "restored" : null].filter(Boolean).join(" · ")}
            </Text>
          ) : null}
        </View>
        {block.dropped > 0 ? (
          <Text style={styles.note}>
            {block.dropped} earlier {block.dropped === 1 ? "line" : "lines"} not shown
          </Text>
        ) : null}
        {hidden > 0 ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Show all output, ${hidden} more lines`}
            onPress={() => onToggle(block.id)}
            style={styles.toggle}
          >
            <Text style={styles.toggleText}>Show all ({hidden} more lines)</Text>
          </Pressable>
        ) : null}
        {shown !== "" ? (
          <Text style={styles.output} selectable>
            {shown}
          </Text>
        ) : null}
        {expanded && tailLines(block.output, COLLAPSED_LINES).hidden > 0 ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Show only the last ${COLLAPSED_LINES} lines`}
            onPress={() => onToggle(block.id)}
            style={styles.toggle}
          >
            <Text style={styles.toggleText}>Show less</Text>
          </Pressable>
        ) : null}
      </View>
    </View>
  );
});

const styles = StyleSheet.create({
  card: {
    flexDirection: "row",
    marginHorizontal: 12,
    marginVertical: 4,
    backgroundColor: colors.surface,
    borderRadius: radius.control,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  edge: { width: 2 },
  body: { flex: 1, paddingHorizontal: 12, paddingVertical: 8, gap: 4 },
  heading: { gap: 4, minHeight: TOUCH },
  header: { flexDirection: "row", flexWrap: "wrap", alignItems: "flex-start", columnGap: 8 },
  command: { flexGrow: 1, flexShrink: 1, flexBasis: 160, color: colors.textBright, fontFamily: mono, fontSize: 13, lineHeight: 18 },
  badge: { fontSize: 12, lineHeight: 18, fontVariant: ["tabular-nums"] },
  meta: { color: colors.textMuted, fontSize: 12 },
  note: { color: colors.textMuted, fontSize: 12, fontStyle: "italic" },
  output: { color: colors.text, fontFamily: mono, fontSize: 12, lineHeight: 16 },
  toggle: { minHeight: TOUCH, justifyContent: "center", alignSelf: "flex-start" },
  toggleText: { color: colors.accent, fontSize: 13, fontWeight: "500" },
});
