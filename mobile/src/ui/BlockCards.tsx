import * as Clipboard from "expo-clipboard";
import { memo, useEffect, useState, type ReactNode } from "react";
import { AccessibilityInfo, Pressable, StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { failureMessage, lastLines, parseLogLines } from "@/paneView";
import type { Block } from "@/protocol/types";
import { formatClock } from "./format";
import { Icon } from "./Icon";
import { SelectableText } from "./SelectableText";
import { Spinner } from "./StatusMark";
import { colors, mono, radius, tints, ui } from "./theme";
import { toggleA11y } from "./toggle";

/** A small action under a card (the boards' 32 pt pill): an outline, the caution colour, or (`on`) the accent tint. */
export function Pill({
  label,
  spoken,
  icon,
  color = colors.text,
  border = colors.borderStrong,
  on,
  pressed: toggle,
  onPress,
}: {
  label: string;
  spoken?: string;
  /** Drawn before the label. */
  icon?: ReactNode;
  color?: string;
  border?: string;
  /** The accent tint ground (a bell that is on). */
  on?: boolean;
  /** A toggle's state (aria-pressed). */
  pressed?: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      {...(toggle === undefined ? { accessibilityRole: "button" as const } : toggleA11y(toggle))}
      accessibilityLabel={spoken ?? label}
      onPress={onPress}
      hitSlop={{ top: 6, bottom: 6, left: 2, right: 2 }}
      style={({ pressed }) => [styles.pill, { borderColor: border }, on && styles.pillOn, pressed && styles.pressed]}
    >
      {icon}
      <Text style={[styles.pillText, { color }]}>{label}</Text>
    </Pressable>
  );
}

/** "✓ 3 earlier commands succeeded … Show": the folded successes (now.ts `foldBlocks`), the board's 36 pt pill. */
export function FoldRow({ count, onShow }: { count: number; onShow: () => void }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${count} earlier commands succeeded`}
      accessibilityHint="Shows them"
      hitSlop={{ top: 4, bottom: 4 }}
      onPress={onShow}
      style={({ pressed }) => [styles.fold, pressed && styles.pressed]}
    >
      <Icon name="check" size={14} color={colors.success} />
      <Text style={[styles.foldText, styles.foldMid]}>{count} earlier commands succeeded</Text>
      <Text style={styles.foldShow}>Show</Text>
    </Pressable>
  );
}

/** A running block's clock "4:12", ticking locally between gateway updates. */
export function Elapsed({ ms }: { ms: number }) {
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
  return <Text style={[styles.status, { color: colors.accent }]}>{formatClock(ms + extra)}</Text>;
}

/**
 * A block's card (the Pane board): surface, a 1 pt border in the state's colour at 33 %, no
 * left edge. The head (command and status) opens the block sheet; the output under it is selectable text
 * (selecting copies, SelectableText) and the pills are their own buttons.
 */
export function CardFrame({
  ground,
  border,
  label,
  onOpen,
  head,
  children,
  pills,
}: {
  ground: string;
  border: string;
  /** What the card is, for screen readers: "git pull, succeeded". */
  label: string;
  onOpen: () => void;
  head: ReactNode;
  children?: ReactNode;
  pills?: ReactNode;
}) {
  const frame: StyleProp<ViewStyle> = [styles.card, { backgroundColor: ground, borderColor: border }];
  return (
    <View style={frame}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${label}: full output and actions`}
        onPress={onOpen}
        style={({ pressed }) => pressed && styles.pressed}
      >
        <View style={styles.head}>{head}</View>
      </Pressable>
      {children ? <View style={styles.body}>{children}</View> : <View style={styles.bodyEmpty} />}
      {pills ? <View style={styles.pills}>{pills}</View> : null}
    </View>
  );
}

/** The command, wrapping, and its status badge (the board's `cmd` and `status`). */
export function CardHead({ command, status, badge, color }: { command: string; status: string; badge?: string; color: string }) {
  return (
    <>
      <Text style={styles.command}>{command || " "}</Text>
      {status ? (
        <Text style={[styles.status, { color }, badge ? { backgroundColor: badge } : null]}>{status}</Text>
      ) : null}
    </>
  );
}

interface CardProps {
  block: Block;
  /** Opens the block sheet. */
  onActions: (id: number) => void;
}

/** A failed command: `✕ 127`, its message and path, Run again (not for a view-only device: no `onRunAgain`), Copy error. */
export const FailedCard = memo(function FailedCard({ block, onActions, onRunAgain }: CardProps & { onRunAgain?: (b: Block) => void }) {
  const fail = failureMessage(block.output);
  return (
    <CardFrame
      ground={colors.surface}
      border={`${colors.error}55`}
      label={`${block.command || "Command"}, failed, exit ${block.exit}${fail ? `, ${fail.text}` : ""}`}
      onOpen={() => onActions(block.id)}
      head={<CardHead command={block.command} status={`✕ ${block.exit}`} badge={tints.errorBadge} color={colors.textBright} />}
      pills={
        <>
          {onRunAgain ? (
            <Pill
              label="Run again"
              icon={<Icon name="rerun" size={14} color={colors.text} />}
              onPress={() => onRunAgain(block)}
            />
          ) : null}
          {/* nothing to copy from a command that printed nothing (`false`) */}
          {block.output !== "" ? (
            <Pill
              label="Copy error"
              spoken="Copy error: copies the output"
              onPress={() =>
                void Clipboard.setStringAsync(block.output).then(
                  () => AccessibilityInfo.announceForAccessibility("Error copied"),
                  () => AccessibilityInfo.announceForAccessibility("Could not copy the output"),
                )
              }
            />
          ) : null}
        </>
      }
    >
      {fail ? <SelectableText text={fail.text} style={styles.failText} lineHeight={19} /> : null}
      {fail?.path ? <SelectableText text={fail.path} style={styles.sub} lineHeight={16} /> : null}
    </CardFrame>
  );
});

/** The running command: a spinner, its clock, the latest two lines, Stop (control C; not for a view-only device: no `onStop`) and the bell. */
export const RunningCard = memo(function RunningCard({
  block,
  onActions,
  notify,
  onBell,
  onStop,
}: CardProps & { notify: boolean; onBell: (b: Block, on: boolean) => void; onStop?: () => void }) {
  const latest = lastLines(block.output, 2);
  const logs = parseLogLines(latest);
  return (
    <CardFrame
      ground={tints.running}
      border={`${colors.accent}55`}
      label={`${block.command || "Command"}, running`}
      onOpen={() => onActions(block.id)}
      head={
        <>
          <Spinner style={styles.spinner} />
          <Text style={styles.command}>{block.command || " "}</Text>
          <Elapsed ms={block.duration_ms} />
        </>
      }
      pills={
        <>
          {onStop ? (
            <Pill
              label="Stop"
              spoken="Stop, sends control C"
              color={colors.warning}
              border={`${colors.warning}66`}
              icon={<Icon name="stop" size={12} color={colors.warning} />}
              onPress={onStop}
            />
          ) : null}
          <Pill
            label={notify ? "Notifies when done" : "Notify when done"}
            spoken="Notify when done"
            pressed={notify}
            on={notify}
            // the accent's text on the tint over the running ground is 4.3:1, so the label stays bright
            color={colors.textBright}
            border={notify ? `${colors.accent}33` : colors.borderStrong}
            icon={<Icon name={notify ? "bellFill" : "bell"} size={14} color={notify ? colors.accent : colors.text} />}
            onPress={() => onBell(block, !notify)}
          />
        </>
      }
    >
      {logs ? (
        <>
          {logs.source ? <Text style={styles.logSource}>{logs.source} · latest</Text> : null}
          {logs.rows.map((r, i) => (
            <View key={i} style={styles.logRow}>
              <Text style={[styles.logLevel, { color: r.level === "WARN" || r.level === "ERROR" || r.level === "FATAL" ? colors.textBright : colors.accent }]}>
                {r.level}
              </Text>
              <View style={styles.logMessage}>
                <SelectableText text={r.message} style={styles.logMessageText} lineHeight={18} lines={1} />
              </View>
              {r.time ? <Text style={styles.logTime}>{r.time}</Text> : null}
            </View>
          ))}
        </>
      ) : (
        latest.map((l, i) => (
          <SelectableText key={i} text={l} style={styles.latest} lineHeight={18} lines={1} />
        ))
      )}
    </CardFrame>
  );
});

const styles = StyleSheet.create({
  pressed: { opacity: 0.7 },
  fold: {
    height: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 14,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  foldText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  foldMid: { flex: 1 },
  foldShow: { fontFamily: ui(500), color: colors.accent, fontSize: 13 },
  card: { borderRadius: radius.card, borderWidth: 1, overflow: "hidden" },
  head: { flexDirection: "row", alignItems: "flex-start", gap: 8, paddingTop: 10, paddingHorizontal: 12 },
  body: { gap: 4, paddingTop: 6, paddingBottom: 10, paddingHorizontal: 12 },
  bodyEmpty: { height: 10 },
  pills: { flexDirection: "row", flexWrap: "wrap", gap: 6, paddingHorizontal: 12, paddingBottom: 10 },
  spinner: {
    width: 14,
    height: 14,
    marginTop: 2,
    borderRadius: 7,
    borderWidth: 2.5,
    borderColor: colors.accent,
    borderLeftColor: "transparent",
  },
  command: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.textBright, fontSize: 13, lineHeight: 18 },
  status: { fontFamily: mono, fontSize: 12, lineHeight: 18, paddingHorizontal: 6, borderRadius: 4, overflow: "hidden", fontVariant: ["tabular-nums"] },
  failText: { fontFamily: ui(400), color: colors.textBright, fontSize: 14 },
  sub: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  latest: { fontFamily: mono, color: colors.fg, fontSize: 13 },
  // the Pane board's log lines: source, then level · message · time
  logSource: { fontFamily: ui(400), color: colors.fg, fontSize: 11 },
  logRow: { flexDirection: "row", alignItems: "baseline", gap: 8 },
  logLevel: { width: 38, fontFamily: ui(600), fontSize: 11 },
  logMessage: { flex: 1, minWidth: 0 },
  logMessageText: { fontFamily: mono, color: colors.textBright, fontSize: 13 },
  logTime: { fontFamily: mono, color: colors.fg, fontSize: 12 },
  pill: {
    height: 32,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 12,
    borderRadius: 16,
    borderWidth: 1,
  },
  pillOn: { backgroundColor: tints.accentBadge },
  pillText: { fontFamily: ui(500), fontSize: 13 },
});
