import { useActionScope } from "@/ui/useActionScope";
import { useRouter } from "expo-router";
import { useRef, useState } from "react";
import { AccessibilityInfo, Keyboard, Pressable, ScrollView, Text, TextInput, View, useWindowDimensions } from "react-native";
import { primaryChoice, questionKey, replyChoice, type Pick } from "@/answer";
import { needsAuthBeforeRun } from "@/lock";
import {
  agoText,
  answerLabel,
  choiceCommand,
  diffRowText,
  askContext,
  askerName,
  hasMoreRows,
  otherChoices,
  visibleRows,
  type DiffRow,
} from "@/paneView";
import type { Waiting } from "@/protocol/types";
import { authenticate, fleet, useApp } from "@/runtime";
import { Button } from "./Button";
import { ErrorText } from "./Notice";
import { PromptText } from "./PromptText";
import { SheetHeader } from "./SheetHeader";
import { mono, monoMedium, radius, tint, ui, makeStyles, useColors } from "./theme";
import { useNow } from "./useNow";

/** When each question was first seen here (the pane gives no time): its "2 min ago". */
const seenAt = new Map<string, number>();

/** The keys under the answers: the board's caption, the protocol name, what screen readers say. */
const KEYS = [
  { cap: "Esc", key: "esc", label: "Escape" },
  { cap: "⇧Tab", key: "shift+tab", label: "Shift Tab" },
  { cap: "↑", key: "up", label: "Up" },
  { cap: "↓", key: "down", label: "Down" },
] as const;

/** One changed line (the board's 30 pt row): its number in an 18 pt column, then "− text" on the line's tint. */
function DiffLine({ row }: { row: DiffRow }) {
  const styles = useStyles();
  return (
    <View style={[styles.diffRow, row.kind === "added" && styles.diffAdded, row.kind === "removed" && styles.diffRemoved]}>
      <Text style={styles.diffNum}>{row.n ?? ""}</Text>
      <Text style={styles.diffText} numberOfLines={1}>
        {diffRowText(row)}
      </Text>
    </View>
  );
}

/**
 * A coding agent's question docked at the bottom of its pane (the New Screen board): who asks and
 * since when, the question, the file and its changed lines (Full diff opens A change in full,
 * app/change.tsx), the first entry as the primary answer and the next ones outlined in equal
 * halves, each saying what happens; "No, explain" opens Tell it why (a reply box and Send, under a
 * sheet's header), and the keys an agent's menus use. Folded, it is a 48 pt bar in the same place,
 * under the screen, never over it. Answers go as the answer page's do (answer.ts `answerPlan`
 * through `fleet.answer`, after the authentication Settings asks for).
 */
export function AskCard({
  hostKey,
  paneId,
  waiting: w,
  program,
  where: tabName,
  bottomInset,
  onSay,
}: {
  hostKey: string;
  paneId: number;
  waiting: Waiting;
  program: string | null;
  /** The tab it asks in ("project4"), beside who asks. */
  where?: string;
  /** The home indicator's room under the card. */
  bottomInset: number;
  /** A toast ("Sent: Yes"). */
  onSay: (text: string) => void;
}) {
  const colors = useColors();
  const styles = useStyles();
  const s = useApp();
  const scope = useActionScope(`${hostKey}:${paneId}:${questionKey(w)}`);
  const router = useRouter();
  const { height } = useWindowDimensions();
  const [min, setMin] = useState(false);
  const [replying, setReplying] = useState(false);
  const [reply, setReply] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // a second tap before the re-render that disables the buttons sends nothing
  const sending = useRef(false);

  const where = `${hostKey}:${paneId}:${questionKey(w)}`;
  const [first] = useState(() => {
    const old = seenAt.get(where);
    if (old !== undefined) return old;
    const at = Date.now();
    seenAt.set(where, at);
    return at;
  });
  const now = useNow(30_000);
  const since = agoText(now - first);

  const who = askerName(program);
  const ctx = askContext(w.context);
  const rows = visibleRows(ctx, false);
  const primary = primaryChoice(w);
  const replyAt = replyChoice(w);
  const others = otherChoices(w, primary);

  const send = async (pick: Pick, said: string) => {
    if (sending.current) return;
    const current = scope.capture();
    sending.current = true;
    setError(null);
    if (needsAuthBeforeRun(s.settings)) {
      const auth = await authenticate(`Answer ${program ?? "the program"}`);
      if (!current()) { sending.current = false; return; }
      if (!auth.ok) {
        sending.current = false;
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    setBusy(true);
    const r = await fleet.answer(hostKey, paneId, w, pick);
    sending.current = false;
    setBusy(false);
    if (!current()) return;
    if (!r.ok) {
      setError(r.message);
      return;
    }
    setReply("");
    Keyboard.dismiss();
    onSay(said);
  };
  const sendKey = async (key: string, cap: string) => {
    setError(null);
    const r = await fleet.sendKeys(hostKey, paneId, [key]);
    if (!r.ok) setError(r.message);
    else {
      AccessibilityInfo.announceForAccessibility(`Sent ${cap}`);
      onSay(`Sent ${cap}`);
    }
  };

  // folded: the 48 pt bar in a black strip under a hairline, where the keys are (the board)
  if (min)
    return (
      <View style={[styles.strip, { paddingBottom: 8 + bottomInset }]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${who} asks: ${w.prompt}${/[.?!:]$/.test(w.prompt.trim()) ? "" : "."} Answer`}
          accessibilityHint="Opens the question"
          onPress={() => setMin(false)}
          style={({ pressed }) => [styles.bar, pressed && styles.pressed]}
        >
          <View style={styles.dot} />
          <Text style={styles.barText} numberOfLines={1}>
            {who} asks: {w.prompt}
          </Text>
          <Text style={styles.barAnswer}>Answer</Text>
        </Pressable>
      </View>
    );

  // No, explain: Tell it why, the reply as a sheet of its own in the card's place; ✕ goes back to the answers
  if (replying && replyAt !== null)
    return (
      <View style={[styles.card, { paddingBottom: 8 + bottomInset }]}>
        <View>
          <SheetHeader
            title="Tell it why"
            subtitle={[who, tabName].filter(Boolean).join(" · ")}
            onClose={() => {
              setReplying(false);
              setReply("");
            }}
          />
        </View>
        {/* the board: a 120 pt box with an accent rim, its lines wrapping, and Send full width under it */}
        <View style={styles.body}>
          <TextInput
            keyboardAppearance={colors.dark ? "dark" : "light"}
            value={reply}
            onChangeText={setReply}
            autoFocus
            multiline
            // Return sends, as it did in the one-line field: the reply goes as one message
            submitBehavior="submit"
            placeholder="What should it do instead?"
            placeholderTextColor={colors.textMuted}
            accessibilityLabel={`Tell ${who} what to do instead`}
            returnKeyType="send"
            onSubmitEditing={() => void send({ choice: replyAt, reply }, "Sent: No")}
            style={styles.replyBox}
          />
          <Button
            label="Send"
            variant="primary"
            accessibilityLabel={`Send ${w.choices[replyAt]}`}
            disabled={busy}
            onPress={() => void send({ choice: replyAt, reply }, "Sent: No")}
          />
          {error ? <ErrorText>{error}</ErrorText> : null}
        </View>
      </View>
    );

  return (
    <View style={[styles.card, { paddingBottom: 8 + bottomInset, minHeight: Math.round(Math.min(CARD_H, height * (CARD_H / 874))) }]} accessibilityLabel={`${who} asks: ${w.prompt}`}>
      {/* the standard sheet header (the Agent question board): who asks, where and since when; ✕ folds it */}
      <View>
        <SheetHeader title={`${who} asks`} subtitle={[tabName, since].filter(Boolean).join(" · ")} closeLabel="Fold the question" onClose={() => setMin(true)} />
      </View>
      {/* the New Screen board: under the header, 16 pt in, everything 12 apart */}
      <View style={styles.body}>
        <ScrollView style={{ maxHeight: height * (others.length > MANY_ANSWERS ? 0.3 : 0.55) }} contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled" bounces={false}>
          <PromptText text={w.prompt} style={styles.title} monoSize={14} />
          {ctx.file || rows.length > 0 || ctx.meta.length > 0 ? (
            <View style={styles.context} accessible={false}>
              {ctx.file || ctx.rows.length > 0 ? (
                <View style={[styles.fileRow, (rows.length > 0 || ctx.meta.length > 0) && styles.fileRule]}>
                  <Text style={styles.file} numberOfLines={1} ellipsizeMode="head">
                    {ctx.file ?? "Changes"}
                  </Text>
                  {ctx.rows.length > 0 ? (
                    <Text style={styles.count}>
                      <Text style={{ color: colors.success }}>+{ctx.added}</Text> <Text style={{ color: colors.error }}>−{ctx.removed}</Text>
                    </Text>
                  ) : null}
                  {hasMoreRows(ctx) ? (
                    <Pressable
                      accessibilityRole="button"
                      accessibilityLabel="Full diff"
                      accessibilityHint="Opens the change in full"
                      hitSlop={{ top: 14, bottom: 14, left: 8, right: 8 }}
                      onPress={() => router.push({ pathname: "/change", params: { key: hostKey, pane: String(paneId), program: program ?? "" } })}
                      style={({ pressed }) => pressed && styles.pressed}
                    >
                      <Text style={styles.fullDiff}>Full diff</Text>
                    </Pressable>
                  ) : null}
                </View>
              ) : null}
              {rows.length > 0 ? (
                <ScrollView style={styles.diffBody} nestedScrollEnabled>
                  {rows.map((r, i) => (
                    <DiffLine key={i} row={r} />
                  ))}
                </ScrollView>
              ) : ctx.meta.length > 0 ? (
                <View style={styles.metaBody}>
                  {ctx.meta.map((l, i) => (
                    <Text key={i} style={styles.meta} numberOfLines={2}>
                      {l}
                    </Text>
                  ))}
                </View>
              ) : null}
            </View>
          ) : null}
        </ScrollView>
        {error ? <ErrorText>{error}</ErrorText> : null}
        {w.choices.map((c, i) => {
          const command = choiceCommand(c);
          return command ? (
            <Text key={i} style={styles.runs} accessibilityLabel={`${answerLabel(w, i)} runs ${command}`}>
              {answerLabel(w, i)} runs: <Text style={styles.runsCommand}>{command}</Text>
            </Text>
          ) : null;
        })}
        {/* a long menu scrolls here, so its last entries and the Keys (Esc leaves it) stay on the screen */}
        <ScrollView style={{ maxHeight: height * 0.34 }} contentContainerStyle={styles.answers} keyboardShouldPersistTaps="handled" nestedScrollEnabled>
          <Button
            label={answerLabel(w, primary)}
            accessibilityLabel={w.choices[primary]}
            variant="primary"
            numberOfLines={1}
            disabled={busy}
            onPress={() => void send({ choice: primary }, `Sent: ${answerLabel(w, primary)}`)}
          />
          <View style={styles.others}>
            {others.map((i) => (
              <Button
                key={i}
                label={answerLabel(w, i)}
                accessibilityLabel={w.choices[i]}
                numberOfLines={1}
                disabled={busy}
                onPress={() => (i === replyAt ? setReplying(true) : void send({ choice: i }, `Sent: ${answerLabel(w, i)}`))}
                style={styles.outline}
              />
            ))}
          </View>
        </ScrollView>
        <View style={styles.keys} accessibilityRole="toolbar" accessibilityLabel="Keys">
          <Text style={styles.keysCaption}>Keys</Text>
          {KEYS.map((k) => (
            <Pressable
              key={k.key}
              accessibilityRole="button"
              accessibilityLabel={k.label}
              onPress={() => void sendKey(k.key, k.cap)}
              hitSlop={{ top: 7, bottom: 7 }}
              style={({ pressed }) => [styles.key, pressed && styles.pressed]}
            >
              <Text style={styles.keyText}>{k.cap}</Text>
            </Pressable>
          ))}
        </View>
      </View>
    </View>
  );
}

/** Past this many answers under the primary one, the question gives up room to them. */
const MANY_ANSWERS = 4;
/** The question card's height on the board's 874 pt phone (shorter phones in proportion); more content grows it. */
const CARD_H = 598;

const useStyles = makeStyles((colors) => ({
  pressed: { opacity: 0.7 },
  // folded (the board): a black strip under a hairline, the 48 pt bar in it, radius 12, on the
  // caution colour's wash with its border
  strip: { backgroundColor: colors.chrome, borderTopWidth: 1, borderTopColor: colors.hairline, paddingTop: 10, paddingHorizontal: 12 },
  bar: {
    minHeight: 48,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 14,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: tint(colors.warning, 0x66),
    backgroundColor: tint(colors.warning, 0x1f),
  },
  barText: { flex: 1, minWidth: 0, fontFamily: ui(500), color: colors.textBright, fontSize: 14, lineHeight: 19 },
  barAnswer: { fontFamily: ui(600), color: colors.warning, fontSize: 14 },
  dot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.warning },
  // the board's card: the surface, its top corners round, a neutral rim along the top
  card: {
    backgroundColor: colors.surface,
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    borderTopWidth: 1,
    borderTopColor: colors.borderStrong,
  },
  body: { paddingHorizontal: 16, gap: 12 },
  content: { gap: 12 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 19, lineHeight: 25 },
  // the diff: the page's ground in a 10 pt box with a hairline, its header 32 pt
  context: { borderRadius: radius.field, backgroundColor: colors.bg, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  fileRow: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: 8, paddingHorizontal: 12 },
  fileRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  runs: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17 },
  runsCommand: { fontFamily: mono, color: colors.textBright, fontSize: 12 },
  file: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.text, fontSize: 12, lineHeight: 16 },
  count: { fontFamily: monoMedium, fontSize: 12, lineHeight: 16 },
  fullDiff: { fontFamily: ui(600), color: colors.accent, fontSize: 12, lineHeight: 16 },
  diffBody: { maxHeight: 220 },
  // 30 pt rows: 6 over and under the 18 pt line
  diffRow: { flexDirection: "row", paddingVertical: 6, paddingHorizontal: 12 },
  diffAdded: { backgroundColor: colors.tints.answerAdded },
  diffRemoved: { backgroundColor: colors.tints.answerRemoved },
  diffNum: { width: 18, fontFamily: mono, color: colors.textMuted, fontSize: 12, lineHeight: 18 },
  diffText: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.text, fontSize: 12, lineHeight: 18 },
  metaBody: { paddingVertical: 6, paddingHorizontal: 12, gap: 2 },
  meta: { fontFamily: mono, color: colors.textMuted, fontSize: 12, lineHeight: 18 },
  answers: { gap: 12 },
  // the outlined answers in equal halves, 8 apart
  others: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  outline: { flexGrow: 1, flexBasis: "45%", paddingHorizontal: 12 },
  // Tell it why: a 120 pt box on the page's ground with the accent rim, Inter 16/22
  replyBox: {
    height: 120,
    paddingHorizontal: 12,
    paddingTop: 10,
    paddingBottom: 10,
    borderRadius: radius.field,
    borderWidth: 1,
    borderColor: colors.accent,
    backgroundColor: colors.bg,
    fontFamily: ui(400),
    color: colors.textBright,
    fontSize: 16,
    lineHeight: 22,
    textAlignVertical: "top",
  },
  keys: { flexDirection: "row", alignItems: "center", gap: 6 },
  // textMuted: the mockup's textDim is 4.49:1 on a surface (palette.ts)
  keysCaption: { fontFamily: ui(500), color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  // the board's keys: 30 pt, 8 pt corners, mono 12 on the surface
  key: {
    height: 30,
    paddingHorizontal: 10,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.surface,
    alignItems: "center",
    justifyContent: "center",
  },
  keyText: { fontFamily: monoMedium, color: colors.text, fontSize: 12 },
}));
