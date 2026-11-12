import * as Clipboard from "expo-clipboard";
import { memo, useState, type ReactNode } from "react";
import { AccessibilityInfo, Pressable, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { failureMessage, lastLines, liveDuration, parseLogLines } from "@/paneView";
import type { Block } from "@/protocol/types";
import { Button } from "./Button";
import { formatClock } from "./format";
import { Icon } from "./Icon";
import { Marked } from "./Marked";
import { glyph } from "./Screen";
import { SelectableText } from "./SelectableText";
import { Spinner } from "./StatusMark";
import { mono, monoMedium, ui, makeStyles, useColors } from "./theme";
import { useNow } from "./useNow";

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
      <Icon name="check" size={glyph(14, 11)} color={colors.success} />
      <Text style={[styles.foldText, styles.foldMid]}>{count} earlier commands succeeded</Text>
      <Text style={styles.foldShow}>Show</Text>
    </Pressable>
  );
}

/**
 * For a search (`query`): the failed command's first output line that holds the words, as the
 * card's message, when its own message (the last line and the path it names) does not; else null.
 */
export function searchedLine(output: string, query: string | undefined): { text: string; path: null } | null {
  const q = query?.trim().toLowerCase();
  if (!q) return null;
  const fail = failureMessage(output);
  if (fail && (fail.text.toLowerCase().includes(q) || fail.path?.toLowerCase().includes(q))) return null;
  const line = output.split("\n").find((l) => l.toLowerCase().includes(q));
  return line ? { text: line.trim(), path: null } : null;
}

/** A running block's clock "4:12", ticking locally between gateway updates (liveDuration). */
export function Elapsed({ ms, started = null }: { ms: number; started?: number | null }) {
  const colors = useColors();
  const styles = useStyles();
  const now = useNow(1000);
  const [seen, setSeen] = useState({ ms, at: now });
  // a fresh duration from the gateway restarts the local count
  if (seen.ms !== ms) setSeen({ ms, at: now });
  return <Text style={[styles.status, { color: colors.accent }]}>{formatClock(liveDuration(ms, seen.at, now, started))}</Text>;
}

/**
 * A block's card (the New Blocks board): surface, 12 pt corners, a 1 pt border in the state's
 * colour, 10 / 12 pt in, 5 pt between its lines. The head (command and status) opens the block
 * sheet; the output under it is selectable text (selecting copies, SelectableText) and the pills
 * are their own buttons.
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
      {children}
      {pills ? <View style={styles.pills}>{pills}</View> : null}
    </View>
  );
}

/** The command on one line, and its status as plain mono text in the state's colour (the board's `cmd` and `status`). */
export function CardHead({ command, status, color, mark }: { command: string; status: string; color: string; mark?: string }) {
  const styles = useStyles();
  return (
    <>
      <Text style={styles.command} numberOfLines={1}>
        <Marked text={command || " "} query={mark} />
      </Text>
      {status ? <Text style={[styles.status, { color }]}>{status}</Text> : null}
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

/**
 * A failed command: `✕ 127`, its message and path, Run again (not for a view-only device: no
 * `onRunAgain`), Copy error. Search results (`mark`) show the matching lines only, no actions.
 */
export const FailedCard = memo(function FailedCard({ block, onActions, onRunAgain, mark }: CardProps & { onRunAgain?: (b: Block) => void }) {
  const colors = useColors();
  const styles = useStyles();
  // a search result shows the line that holds the words, when the message does not (the New Blocks board)
  const fail = searchedLine(block.output, mark) ?? failureMessage(block.output);
  return (
    <CardFrame
      ground={colors.surface}
      border={`${colors.error}88`}
      label={`${block.command || "Command"}, failed, exit ${block.exit}${fail ? `, ${fail.text}` : ""}`}
      onOpen={() => onActions(block.id)}
      head={<CardHead command={block.command} status={`✕ ${block.exit}`} color={colors.error} mark={mark} />}
      pills={
        mark ? undefined : <>
          {onRunAgain ? (
            <Button
              label="Run again"
              size="small"
              icon={<Icon name="rerun" size={glyph(14, 12)} color={colors.text} />}
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
      {fail ? <SelectableText text={fail.text} style={styles.output} lineHeight={17} mark={mark} /> : null}
      {fail?.path ? <SelectableText text={fail.path} style={styles.output} lineHeight={17} mark={mark} /> : null}
    </CardFrame>
  );
});

/** The running command: a spinner, its clock, the latest line, Stop (control C; not for a view-only device: no `onStop`) and the bell. */
export const RunningCard = memo(function RunningCard({
  block,
  onActions,
  notify,
  onBell,
  onStop,
  mark,
  started = null,
}: CardProps & {
  notify: boolean;
  onBell: (b: Block, on: boolean) => void;
  onStop?: () => void;
  /** When the pane's command started (the desktop's Unix seconds), for the clock. */
  started?: number | null;
}) {
  const colors = useColors();
  const styles = useStyles();
  const latest = lastLines(block.output, 1);
  const logs = parseLogLines(latest);
  return (
    <CardFrame
      ground={colors.tints.running}
      border={`${colors.accent}66`}
      label={`${block.command || "Command"}, running`}
      onOpen={() => onActions(block.id)}
      head={
        <>
          <View style={styles.runLine}>
            <Spinner size={15} style={styles.spinner} />
            <Text style={styles.command} numberOfLines={1}>
              <Marked text={block.command || " "} query={mark} />
            </Text>
          </View>
          <Elapsed ms={block.duration_ms} started={started} />
        </>
      }
      pills={
        mark ? undefined : <>
          {onStop ? (
            <Button
              label="Stop"
              accessibilityLabel="Stop, sends control C"
              variant="destructive"
              size="small"
              icon={<Icon name="stop" size={glyph(12, 9)} color={colors.error} />}
              onPress={onStop}
            />
          ) : null}
          {/* the accent's text on the tint over the running ground is 4.3:1, so the label stays in the text colour */}
          <Button
            label={notify ? "Notifies when done" : "Notify when done"}
            accessibilityLabel="Notify when done"
            size="small"
            pressed={notify}
            icon={<Icon name={notify ? "bellFill" : "bell"} size={glyph(14, 12)} color={notify ? colors.accent : colors.text} />}
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
                <SelectableText text={r.message} style={styles.logMessageText} lineHeight={17} lines={1} mark={mark} />
              </View>
              {r.time ? <Text style={styles.logTime}>{r.time}</Text> : null}
            </View>
          ))}
        </>
      ) : (
        latest.map((l, i) => (
          <SelectableText key={i} text={l} style={styles.output} lineHeight={17} lines={1} mark={mark} />
        ))
      )}
    </CardFrame>
  );
});

const useStyles = makeStyles((colors) => ({
  pressed: { opacity: 0.7 },
  // the folded successes: a 36 pt outlined row, 10 pt corners
  fold: {
    height: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  foldText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  foldMid: { flex: 1 },
  foldShow: { fontFamily: ui(600), color: colors.accent, fontSize: 13 },
  card: { borderRadius: 12, borderWidth: 1, overflow: "hidden", paddingVertical: 10, paddingHorizontal: 12, gap: 5 },
  head: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  // Run again / Copy error (Stop / Notify when done): 8 pt apart, 4 pt more above them
  pills: { flexDirection: "row", flexWrap: "wrap", gap: 8, paddingTop: 4 },
  // the running card's spinner, 6 pt before its command
  runLine: { flex: 1, minWidth: 0, flexDirection: "row", alignItems: "center", gap: 6 },
  spinner: { borderWidth: 2 },
  command: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.textBright, fontSize: 13, lineHeight: 18 },
  status: { fontFamily: monoMedium, fontSize: 11, lineHeight: 18, fontVariant: ["tabular-nums"] },
  // output lines: JetBrains Mono 12/17 in the text colour
  output: { fontFamily: mono, color: colors.text, fontSize: 12 },
  // the Pane board's log lines: source, then level · message · time
  logSource: { fontFamily: ui(400), color: colors.fg, fontSize: 11 },
  logRow: { flexDirection: "row", alignItems: "baseline", gap: 8 },
  logLevel: { width: 38, fontFamily: ui(600), fontSize: 11 },
  logMessage: { flex: 1, minWidth: 0 },
  logMessageText: { fontFamily: mono, color: colors.text, fontSize: 12 },
  logTime: { fontFamily: mono, color: colors.fg, fontSize: 12 },
}));
