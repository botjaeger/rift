import { memo, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import {
  gapLines,
  heldLines,
  MAX_EARLIER_LINES,
  nextRequest,
  unloadedLines,
  type EarlierOutput,
} from "@/earlier";
import type { Block } from "@/protocol/types";
import { formatDuration, tailLines } from "./format";
import { ErrorText } from "./Notice";
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
  /** Opens the block's actions (copy, share, run again, …). */
  onActions: (id: number) => void;
  /** Earlier output lines loaded for this block (`output` feature). */
  earlier?: EarlierOutput;
  /** The gateway pages output (`output` feature): offer "Load earlier output". */
  canLoadEarlier: boolean;
  onLoadEarlier: (id: number) => void;
}

const lines = (n: number) => `${n.toLocaleString("en-US")} ${n === 1 ? "line" : "lines"}`;

/** Above the tail: the earlier lines loaded, with what is missing before and after them. */
function Earlier({
  block,
  earlier,
  canLoad,
  onLoad,
}: {
  block: Block;
  earlier: EarlierOutput | undefined;
  canLoad: boolean;
  onLoad: (id: number) => void;
}) {
  const unloaded = unloadedLines(block, earlier);
  const held = heldLines(earlier);
  const gap = gapLines(block, earlier);
  const capped = held >= MAX_EARLIER_LINES;
  const loading = earlier?.loading ?? false;
  return (
    <>
      {canLoad && unloaded > 0 && nextRequest(block, earlier) ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={loading ? "Loading earlier output" : `Load earlier output, ${lines(unloaded)}`}
          accessibilityHint="Loads up to 500 lines at a time"
          accessibilityState={{ disabled: loading, busy: loading }}
          disabled={loading}
          onPress={() => onLoad(block.id)}
          style={styles.toggle}
        >
          <Text style={styles.toggleText}>
            {loading ? "Loading earlier output…" : `Load earlier output (${lines(unloaded)})`}
          </Text>
        </Pressable>
      ) : !canLoad && block.dropped > 0 ? (
        <Text style={styles.note}>{lines(block.dropped)} earlier not shown</Text>
      ) : null}
      {earlier?.gone ? (
        <Text style={styles.note}>
          {earlier.from > 0 ? `${lines(earlier.from)} before these: ` : ""}These lines are gone from the computer's
          scrollback.
        </Text>
      ) : null}
      {capped && unloaded > 0 ? (
        <Text style={styles.note}>
          Showing {lines(held)} of earlier output, the most kept per block; the rest stays on the computer.
        </Text>
      ) : null}
      {earlier?.error ? <ErrorText>{earlier.error}</ErrorText> : null}
      {held > 0 ? (
        <Text style={styles.output} selectable>
          {earlier!.text.endsWith("\n") ? earlier!.text.slice(0, -1) : earlier!.text}
        </Text>
      ) : null}
      {held > 0 && gap > 0 ? <Text style={styles.note}>{lines(gap)} not loaded</Text> : null}
    </>
  );
}

export const BlockView = memo(function BlockView({
  block,
  expanded,
  onToggle,
  onActions,
  earlier,
  canLoadEarlier,
  onLoadEarlier,
}: Props) {
  const st = stateOf(block);
  // with earlier lines loaded the whole tail shows, so nothing is skipped between them
  const full = expanded || heldLines(earlier) > 0;
  const { shown, hidden } = full ? tailLines(block.output, Infinity) : tailLines(block.output, COLLAPSED_LINES);
  return (
    <View style={styles.card}>
      <View style={[styles.edge, { backgroundColor: st.color }]} />
      <View style={styles.body}>
        <View style={styles.top}>
          {/* The command, its result and where it ran are one heading, so screen-reader users can
              jump from block to block, and the element is large enough to find by touch. */}
          <View
            style={styles.heading}
            accessible
            accessibilityRole="header"
            accessibilityLabel={[
              // a running block's duration is only as fresh as its last update (the label does not
              // tick like the badge), so it is left out while running
              block.state === "running"
                ? `Command ${block.command || "(empty)"}, ${st.spoken}`
                : `Command ${block.command || "(empty)"}, ${st.spoken}, ${formatDuration(block.duration_ms)}`,
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
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Actions for ${block.command || "this command"}`}
            accessibilityHint="Copy, share, run again and more"
            onPress={() => onActions(block.id)}
            style={({ pressed }) => [styles.more, pressed && styles.pressed]}
          >
            <Text style={styles.moreLabel}>⋯</Text>
          </Pressable>
        </View>
        <Earlier block={block} earlier={earlier} canLoad={canLoadEarlier} onLoad={onLoadEarlier} />
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
        {expanded && heldLines(earlier) === 0 && tailLines(block.output, COLLAPSED_LINES).hidden > 0 ? (
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
  top: { flexDirection: "row", alignItems: "flex-start", gap: 4 },
  heading: { flex: 1, gap: 4, minHeight: TOUCH },
  more: {
    minWidth: TOUCH,
    minHeight: TOUCH,
    marginRight: -10,
    marginTop: -8,
    alignItems: "center",
    justifyContent: "center",
  },
  moreLabel: { color: colors.textMuted, fontSize: 20, fontWeight: "700" },
  pressed: { opacity: 0.6 },
  header: { flexDirection: "row", flexWrap: "wrap", alignItems: "flex-start", columnGap: 8 },
  command: { flexGrow: 1, flexShrink: 1, flexBasis: 160, color: colors.textBright, fontFamily: mono, fontSize: 13, lineHeight: 18 },
  badge: { fontSize: 12, lineHeight: 18, fontVariant: ["tabular-nums"] },
  meta: { color: colors.textMuted, fontSize: 12 },
  note: { color: colors.textMuted, fontSize: 12, fontStyle: "italic" },
  output: { color: colors.text, fontFamily: mono, fontSize: 12, lineHeight: 16 },
  toggle: { minHeight: TOUCH, justifyContent: "center", alignSelf: "flex-start" },
  toggleText: { color: colors.accent, fontSize: 13, fontWeight: "500" },
});
