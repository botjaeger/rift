import { memo, useEffect, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View, type LayoutChangeEvent } from "react-native";
import {
  gapLines,
  heldLines,
  MAX_EARLIER_LINES,
  nextRequest,
  unloadedLines,
  type EarlierOutput,
} from "@/earlier";
import type { Block } from "@/protocol/types";
import { formatClock, formatDuration, tailLines } from "./format";
import { Icon } from "./Icon";
import { ErrorText } from "./Notice";
import { over } from "./palette";
import { colors, mono, radius, tints, ui } from "./theme";

/** Output longer than this shows only its last lines until "Show N more lines". */
export const COLLAPSED_LINES = 40;

/** `edge` is the left border; `text` the badge (red is too dark for text on bg, palette.ts). */
function stateOf(block: Block): { edge: string; text: string; badge: string; spoken: string } {
  if (block.state === "running") return { edge: colors.accent, text: colors.accent, badge: "running", spoken: "running" };
  if (block.exit === null) return { edge: colors.textDim, text: colors.textMuted, badge: "done", spoken: "done" };
  if (block.exit === 0) return { edge: colors.success, text: colors.success, badge: "✓ 0", spoken: "succeeded, exit 0" };
  return { edge: colors.error, text: colors.textBright, badge: `✕ ${block.exit}`, spoken: `failed, exit ${block.exit}` };
}

/** "running 4:12" (`bare`: "4:12"), ticking locally between gateway updates. */
export function Elapsed({ ms, bare }: { ms: number; bare?: boolean }) {
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
  return (
    <Text style={[styles.badge, { color: colors.accent }]}>
      {bare ? "" : "running "}
      {formatClock(ms + extra)}
    </Text>
  );
}

interface Props {
  block: Block;
  expanded: boolean;
  onToggle: (id: number) => void;
  /** Opens the block's actions (Run again, copy, share, …). */
  onActions: (id: number) => void;
  /** Earlier output lines loaded for this block (`output` feature). */
  earlier?: EarlierOutput;
  /** The gateway pages output (`output` feature): offer "Load earlier output". */
  canLoadEarlier: boolean;
  onLoadEarlier: (id: number) => void;
  /** Wrap long lines; off: the output scrolls sideways. */
  wrap: boolean;
  /** A running block's bell: on when a "Notify me" waits for it. */
  notify: boolean;
  onBell: (block: Block, on: boolean) => void;
}

const lines = (n: number) => `${n.toLocaleString("en-US")} ${n === 1 ? "line" : "lines"}`;

/** Above the tail: the earlier lines loaded, with what is missing before and after them. */
function Earlier({
  block,
  earlier,
  canLoad,
  onLoad,
  wrap,
  ground,
}: {
  block: Block;
  earlier: EarlierOutput | undefined;
  canLoad: boolean;
  onLoad: (id: number) => void;
  wrap: boolean;
  ground: string;
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
      {held > 0 ? <Output text={earlier!.text.endsWith("\n") ? earlier!.text.slice(0, -1) : earlier!.text} wrap={wrap} ground={ground} /> : null}
      {held > 0 && gap > 0 ? <Text style={styles.note}>{lines(gap)} not loaded</Text> : null}
    </>
  );
}

/**
 * Whether a touch is on output that scrolls sideways: the pane's swipe between panes leaves that
 * drag to the scroll (the native scroll moves whatever the responders decide).
 */
export const sideways = { touching: false };

/**
 * Output text: wrapped to the width, or (wrap off) as printed, scrolling sideways with a fade
 * at the right edge and a thin position bar under it (the PaneNoWrap board).
 */
function Output({ text, wrap, ground }: { text: string; wrap: boolean; ground: string }) {
  const [view, setView] = useState({ width: 0, content: 0, x: 0 });
  if (wrap) {
    return (
      <Text style={[styles.output, styles.outputPad]} selectable>
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
          <Text style={[styles.output, styles.outputPad]} selectable>
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

export const BlockView = memo(function BlockView({
  block,
  expanded,
  onToggle,
  onActions,
  earlier,
  canLoadEarlier,
  onLoadEarlier,
  wrap,
  notify,
  onBell,
}: Props) {
  const st = stateOf(block);
  const running = block.state === "running";
  // with earlier lines loaded the whole tail shows, so nothing is skipped between them
  const full = expanded || heldLines(earlier) > 0;
  const { shown, hidden } = full ? tailLines(block.output, Infinity) : tailLines(block.output, COLLAPSED_LINES);
  const name = block.command || "this command";
  // the ground the output fades into at its right edge
  const ground = running ? over(tints.running, colors.bg) : colors.bg;
  return (
    <View style={[styles.block, running ? styles.running : null, { borderLeftColor: st.edge }]}>
      <View style={styles.top}>
        {/* The command, its result and where it ran are one heading, so screen-reader users can
            jump from block to block; a long press (or the Actions action) opens the actions. */}
        <Pressable
          style={styles.heading}
          accessible
          accessibilityRole="header"
          accessibilityLabel={[
            // a running block's duration is only as fresh as its last update (the label does not
            // tick like the badge), so it is left out while running
            running
              ? `Command ${block.command || "(empty)"}, ${st.spoken}`
              : `Command ${block.command || "(empty)"}, ${st.spoken}, ${formatDuration(block.duration_ms)}`,
            block.cwd ? `in ${block.cwd}` : null,
            block.branch ? `on ${block.branch}` : null,
            block.restored ? "restored from an earlier session" : null,
          ]
            .filter(Boolean)
            .join(", ")}
          accessibilityActions={[{ name: "activate", label: "Actions" }, { name: "longpress", label: "Actions" }]}
          onAccessibilityAction={() => onActions(block.id)}
          onLongPress={() => onActions(block.id)}
        >
          <Text style={styles.command} selectable={false} numberOfLines={wrap ? undefined : 1}>
            {block.command || " "}
          </Text>
          {running ? (
            <Elapsed ms={block.duration_ms} />
          ) : (
            <Text style={[styles.badge, { color: st.text }]}>
              {st.badge} · {formatDuration(block.duration_ms)}
            </Text>
          )}
        </Pressable>
        {running ? (
          <Pressable
            accessibilityRole="togglebutton"
            accessibilityLabel="Notify me when it finishes"
            accessibilityState={{ checked: notify }}
            hitSlop={{ top: 8, bottom: 8, left: 6, right: 6 }}
            onPress={() => onBell(block, !notify)}
            style={({ pressed }) => [styles.bell, notify && styles.bellOn, pressed && styles.pressed]}
          >
            <Icon name={notify ? "bellFill" : "bell"} size={14} color={notify ? colors.accent : colors.textMuted} />
          </Pressable>
        ) : (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Actions for ${name}`}
            accessibilityHint="Run again, copy, share and more"
            hitSlop={{ top: 8, bottom: 8, left: 6, right: 6 }}
            onPress={() => onActions(block.id)}
            style={({ pressed }) => [styles.more, pressed && styles.pressed]}
          >
            <Text style={styles.moreLabel}>···</Text>
          </Pressable>
        )}
      </View>
      {block.restored ? <Text style={styles.note}>Restored from an earlier session</Text> : null}
      <Earlier block={block} earlier={earlier} canLoad={canLoadEarlier} onLoad={onLoadEarlier} wrap={wrap} ground={ground} />
      {hidden > 0 ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Show ${hidden} more lines`}
          onPress={() => onToggle(block.id)}
          style={styles.toggle}
        >
          <Text style={styles.toggleText}>Show {hidden.toLocaleString("en-US")} more lines</Text>
        </Pressable>
      ) : null}
      {shown !== "" ? <Output text={shown} wrap={wrap} ground={ground} /> : null}
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
  );
});

const styles = StyleSheet.create({
  // The Pane board: no card, a 2 pt edge in the state's colour; the running block a 3 pt
  // accent edge on an accent tint. 2 pt between blocks.
  block: {
    borderLeftWidth: 2,
    paddingTop: 8,
    paddingBottom: 10,
    paddingLeft: 12,
    gap: 5,
    marginTop: 2,
  },
  running: { borderLeftWidth: 3, paddingLeft: 11, backgroundColor: tints.running },
  top: { flexDirection: "row", alignItems: "center", gap: 8, paddingRight: 14 },
  heading: { flex: 1, flexDirection: "row", alignItems: "center", gap: 8, minHeight: 28 },
  command: { flex: 1, color: colors.textBright, fontFamily: mono, fontSize: 13, lineHeight: 18 },
  badge: { fontFamily: mono, fontSize: 11, fontVariant: ["tabular-nums"] },
  more: { width: 32, height: 28, alignItems: "center", justifyContent: "center" },
  moreLabel: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, letterSpacing: 1 },
  bell: { width: 32, height: 28, borderRadius: radius.control, alignItems: "center", justifyContent: "center" },
  bellOn: { backgroundColor: tints.bellOn },
  pressed: { opacity: 0.6 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, paddingRight: 14 },
  output: { color: colors.fg, fontFamily: mono, fontSize: 12, lineHeight: 17.4 },
  outputPad: { paddingRight: 14 },
  sideways: { gap: 5 },
  fade: { position: "absolute", top: 0, bottom: 0, right: 0, width: 28, flexDirection: "row" },
  fadeStep: { flex: 1 },
  track: { marginRight: 14, height: 3, borderRadius: 2, backgroundColor: colors.hairline },
  thumb: { position: "absolute", top: 0, bottom: 0, borderRadius: 2, backgroundColor: colors.textMuted },
  toggle: { minHeight: 32, justifyContent: "center", alignSelf: "flex-start" },
  toggleText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
});
