import * as Clipboard from "expo-clipboard";
import { memo, useState } from "react";
import { AccessibilityInfo, Pressable, StyleSheet, Text, View } from "react-native";
import type { Block } from "@/protocol/types";
import { Elapsed } from "./BlockView";
import { tailLines } from "./format";
import { colors, mono, radius, tints, ui } from "./theme";
import { toggleA11y } from "./toggle";

/** Lines of output a failed or running card shows until "All N lines". */
export const CARD_LINES = 4;

const lineCount = (b: Block) => {
  const body = b.output.endsWith("\n") ? b.output.slice(0, -1) : b.output;
  return b.dropped + (body === "" ? 0 : body.split("\n").length);
};

function SmallButton({
  label,
  onPress,
  color = colors.text,
  pressed: on,
  spoken,
}: {
  label: string;
  onPress: () => void;
  color?: string;
  /** A toggle's state (aria-pressed): on draws it on the accent tint without a border. */
  pressed?: boolean;
  spoken?: string;
}) {
  return (
    <Pressable
      {...(on === undefined ? { accessibilityRole: "button" as const } : toggleA11y(on))}
      accessibilityLabel={spoken ?? label}
      onPress={onPress}
      hitSlop={{ top: 6, bottom: 6 }}
      style={({ pressed }) => [styles.small, on ? styles.smallOn : styles.smallOutline, pressed && styles.pressed]}
    >
      <Text style={[styles.smallText, { color: on ? colors.accent : color }]}>{label}</Text>
    </Pressable>
  );
}

/** "✓ 6 earlier commands succeeded · Show": the folded successes (now.ts `foldBlocks`). */
export function FoldRow({ count, onShow }: { count: number; onShow: () => void }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${count} earlier commands succeeded`}
      accessibilityHint="Shows them"
      hitSlop={{ top: 2, bottom: 2 }}
      onPress={onShow}
      style={({ pressed }) => [styles.fold, pressed && styles.pressed]}
    >
      <Text style={styles.foldCheck}>✓</Text>
      <Text style={[styles.foldText, styles.foldMid]}>{count} earlier commands succeeded</Text>
      <Text style={styles.foldText}>Show</Text>
    </Pressable>
  );
}

interface CardProps {
  block: Block;
  /** Opens the block's actions (a long press, as on any block). */
  onActions: (id: number) => void;
}

/** The output a card shows: its last lines, or all it holds. */
function CardOutput({ block, all }: { block: Block; all: boolean }) {
  const { shown } = tailLines(block.output, all ? Infinity : CARD_LINES);
  return shown === "" ? null : (
    <Text style={styles.output} selectable>
      {shown}
    </Text>
  );
}

/** A failed command (the P3 Pane board): its last lines, Run again (not for a view-only device: no `onRunAgain`), Copy error, All N lines. */
export const FailedCard = memo(function FailedCard({ block, onActions, onRunAgain }: CardProps & { onRunAgain?: (b: Block) => void }) {
  const [all, setAll] = useState(false);
  const n = lineCount(block);
  return (
    <View style={[styles.card, styles.failed]}>
      <Pressable
        accessibilityRole="header"
        accessibilityLabel={`Command ${block.command || "(empty)"}, failed, exit ${block.exit}`}
        accessibilityActions={[{ name: "longpress", label: "Actions" }]}
        onAccessibilityAction={() => onActions(block.id)}
        onLongPress={() => onActions(block.id)}
        style={styles.head}
      >
        <Text style={styles.command} numberOfLines={2}>
          {block.command || " "}
        </Text>
        <Text style={styles.exit}>✕ {block.exit}</Text>
      </Pressable>
      <CardOutput block={block} all={all} />
      <View style={styles.actions}>
        {onRunAgain ? <SmallButton label="Run again" onPress={() => onRunAgain(block)} /> : null}
        {/* nothing to copy from a command that printed nothing (`false`) */}
        {block.output !== "" ? (
          <SmallButton
            label="Copy error"
            spoken="Copy error: copies the output"
            onPress={() =>
              void Clipboard.setStringAsync(block.output).then(
                () => AccessibilityInfo.announceForAccessibility("Output copied"),
                () => AccessibilityInfo.announceForAccessibility("Could not copy the output"),
              )
            }
          />
        ) : null}
        {n > CARD_LINES ? (
          <SmallButton label={all ? "Fewer lines" : `All ${n.toLocaleString("en-US")} lines`} onPress={() => setAll((a) => !a)} />
        ) : null}
      </View>
    </View>
  );
});

/** The running command (the P3 Pane board): its last lines, Notify when done, Stop (^C) (not for a view-only device: no `onStop`). */
export const RunningCard = memo(function RunningCard({
  block,
  onActions,
  notify,
  onBell,
  onStop,
}: CardProps & { notify: boolean; onBell: (b: Block, on: boolean) => void; onStop?: () => void }) {
  return (
    <View style={[styles.card, styles.running]}>
      <Pressable
        accessibilityRole="header"
        accessibilityLabel={`Command ${block.command || "(empty)"}, running`}
        accessibilityActions={[{ name: "longpress", label: "Actions" }]}
        onAccessibilityAction={() => onActions(block.id)}
        onLongPress={() => onActions(block.id)}
        style={styles.head}
      >
        <Text style={styles.command} numberOfLines={2}>
          {block.command || " "}
        </Text>
        <Elapsed ms={block.duration_ms} bare />
      </Pressable>
      <CardOutput block={block} all={false} />
      <View style={styles.actions}>
        <SmallButton label="Notify when done" pressed={notify} onPress={() => onBell(block, !notify)} />
        {onStop ? <SmallButton label="Stop (^C)" spoken="Stop (^C), sends control C" color={colors.warning} onPress={onStop} /> : null}
      </View>
    </View>
  );
});

const styles = StyleSheet.create({
  pressed: { opacity: 0.7 },
  fold: {
    marginHorizontal: 12,
    minHeight: 40,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 12,
    borderRadius: radius.button,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  foldCheck: { fontFamily: ui(400), color: colors.success, fontSize: 13 },
  foldText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  foldMid: { flex: 1 },
  card: { marginHorizontal: 12, borderRadius: 10, borderWidth: 1, paddingVertical: 10, paddingHorizontal: 12, gap: 6 },
  failed: { backgroundColor: colors.surface, borderColor: `${colors.error}66` },
  running: { backgroundColor: tints.running, borderColor: `${colors.accent}66` },
  head: { flexDirection: "row", alignItems: "center", gap: 8 },
  command: { flex: 1, fontFamily: mono, color: colors.textBright, fontSize: 13 },
  exit: { fontFamily: mono, color: colors.textBright, fontSize: 11 },
  output: { fontFamily: mono, color: colors.fg, fontSize: 12, lineHeight: 17.4 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  small: { height: 32, paddingHorizontal: 10, borderRadius: radius.small, justifyContent: "center" },
  smallOutline: { borderWidth: 1, borderColor: colors.borderStrong },
  smallOn: { backgroundColor: tints.accentBadge },
  smallText: { fontFamily: ui(400), fontSize: 12 },
});
