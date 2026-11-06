import * as Clipboard from "expo-clipboard";
import { memo, useEffect, useState, type ReactNode } from "react";
import { AccessibilityInfo, Pressable, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { failureMessage, lastLines, parseLogLines } from "@/paneView";
import type { Block } from "@/protocol/types";
import { Button } from "./Button";
import { formatClock } from "./format";
import { Icon } from "./Icon";
import { Marked } from "./Marked";
import { SelectableText } from "./SelectableText";
import { Spinner } from "./StatusMark";
import { mono, radius, ui, makeStyles, useColors } from "./theme";

/** "✓ 3 earlier commands succeeded … Show": the folded successes (now.ts `foldBlocks`), the board's 36 pt pill. */
export function FoldRow({ count, onShow }: { count: number; onShow: () => void }) {
  const colors = useColors();
  const styles = useStyles();
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
  const colors = useColors();
  const styles = useStyles();
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
  const styles = useStyles();
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
export function CardHead({ command, status, badge, color, mark }: { command: string; status: string; badge?: string; color: string; mark?: string }) {
  const styles = useStyles();
  return (
    <>
      <Text style={styles.command}>
        <Marked text={command || " "} query={mark} />
      </Text>
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
  /** The Search output query: its matches in the command and output are marked. */
  mark?: string;
}

/** A failed command: `✕ 127`, its message and path, Run again (not for a view-only device: no `onRunAgain`), Copy error. */
export const FailedCard = memo(function FailedCard({ block, onActions, onRunAgain, mark }: CardProps & { onRunAgain?: (b: Block) => void }) {
  const colors = useColors();
  const styles = useStyles();
  const fail = failureMessage(block.output);
  return (
    <CardFrame
      ground={colors.surface}
      border={`${colors.error}55`}
      label={`${block.command || "Command"}, failed, exit ${block.exit}${fail ? `, ${fail.text}` : ""}`}
      onOpen={() => onActions(block.id)}
      head={<CardHead command={block.command} status={`✕ ${block.exit}`} badge={colors.tints.errorBadge} color={colors.textBright} mark={mark} />}
      pills={
        <>
          {onRunAgain ? (
            <Button
              label="Run again"
              size="small"
              icon={<Icon name="rerun" size={14} color={colors.text} />}
              onPress={() => onRunAgain(block)}
            />
          ) : null}
          {/* nothing to copy from a command that printed nothing (`false`) */}
          {block.output !== "" ? (
            <Button
              label="Copy error"
              size="small"
              accessibilityLabel="Copy error: copies the output"
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
      {fail ? <SelectableText text={fail.text} style={styles.failText} lineHeight={19} mark={mark} /> : null}
      {fail?.path ? <SelectableText text={fail.path} style={styles.sub} lineHeight={16} mark={mark} /> : null}
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
  mark,
}: CardProps & { notify: boolean; onBell: (b: Block, on: boolean) => void; onStop?: () => void }) {
  const colors = useColors();
  const styles = useStyles();
  const latest = lastLines(block.output, 2);
  const logs = parseLogLines(latest);
  return (
    <CardFrame
      ground={colors.tints.running}
      border={`${colors.accent}55`}
      label={`${block.command || "Command"}, running`}
      onOpen={() => onActions(block.id)}
      head={
        <>
          <Spinner style={styles.spinner} />
          <Text style={styles.command}>
            <Marked text={block.command || " "} query={mark} />
          </Text>
          <Elapsed ms={block.duration_ms} />
        </>
      }
      pills={
        <>
          {onStop ? (
            <Button
              label="Stop"
              accessibilityLabel="Stop, sends control C"
              variant="destructive"
              size="small"
              icon={<Icon name="stop" size={12} color={colors.error} />}
              onPress={onStop}
            />
          ) : null}
          {/* the accent's text on the tint over the running ground is 4.3:1, so the label stays in the text colour */}
          <Button
            label={notify ? "Notifies when done" : "Notify when done"}
            accessibilityLabel="Notify when done"
            size="small"
            pressed={notify}
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
                <SelectableText text={r.message} style={styles.logMessageText} lineHeight={18} lines={1} mark={mark} />
              </View>
              {r.time ? <Text style={styles.logTime}>{r.time}</Text> : null}
            </View>
          ))}
        </>
      ) : (
        latest.map((l, i) => (
          <SelectableText key={i} text={l} style={styles.latest} lineHeight={18} lines={1} mark={mark} />
        ))
      )}
    </CardFrame>
  );
});

const useStyles = makeStyles((colors) => ({
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
}));
