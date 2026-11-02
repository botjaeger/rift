import { useActionScope } from "@/ui/useActionScope";
import { useRef, useState } from "react";
import { AccessibilityInfo, Keyboard, Pressable, ScrollView, StyleSheet, Text, TextInput, View, useWindowDimensions } from "react-native";
import { primaryChoice, questionKey, replyChoice, type Pick } from "@/answer";
import { needsAuthBeforeRun } from "@/lock";
import {
  agoText,
  answerLabel,
  choiceCommand,
  askContext,
  askerName,
  hasMoreRows,
  otherChoices,
  visibleRows,
  type DiffRow,
} from "@/paneView";
import type { Waiting } from "@/protocol/types";
import { authenticate, fleet, useApp } from "@/runtime";
import { Icon } from "./Icon";
import { ErrorText } from "./Notice";
import { PromptText } from "./PromptText";
import { colors, mono, tints, ui } from "./theme";
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

function DiffLine({ row }: { row: DiffRow }) {
  const tinted = row.kind !== "kept";
  return (
    <View style={[styles.diffRow, row.kind === "added" && styles.diffAdded, row.kind === "removed" && styles.diffRemoved]}>
      <Text style={[styles.diffNum, { color: tinted ? colors.textMuted : colors.textDim }]}>{row.n ?? ""}</Text>
      <Text style={[styles.diffSign, { color: row.kind === "added" ? colors.success : row.kind === "removed" ? colors.textBright : colors.textDim }]}>
        {row.sign === "-" ? "−" : row.sign}
      </Text>
      <Text style={styles.diffText} numberOfLines={1}>
        {row.text}
      </Text>
    </View>
  );
}

/**
 * A coding agent's question docked at the bottom of its pane (the PaneAsking board): who asks and
 * since when, the question, the file and its diff (Full diff / Less), the first entry as the
 * primary answer and the next ones outlined, "No…" opening a field for what to do instead, and
 * the keys an agent's menus use. Minimized, it is a 52 pt bar. Answers go as the answer page's
 * do (answer.ts `answerPlan` through `fleet.answer`, after the authentication Settings asks for).
 */
export function AskCard({
  hostKey,
  paneId,
  waiting: w,
  program,
  bottomInset,
  onSay,
}: {
  hostKey: string;
  paneId: number;
  waiting: Waiting;
  program: string | null;
  /** The home indicator's room under the card. */
  bottomInset: number;
  /** A toast ("Sent: Yes"). */
  onSay: (text: string) => void;
}) {
  const s = useApp();
  const scope = useActionScope(`${hostKey}:${paneId}:${questionKey(w)}`);
  const { height } = useWindowDimensions();
  const [min, setMin] = useState(false);
  const [full, setFull] = useState(false);
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
  const rows = visibleRows(ctx, full);
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

  if (min)
    return (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${who} asks: ${w.prompt}. Answer`}
        accessibilityHint="Opens the question"
        onPress={() => setMin(false)}
        style={({ pressed }) => [styles.bar, { marginBottom: 8 + bottomInset }, pressed && styles.pressed]}
      >
        <View style={styles.dot} />
        <Text style={styles.barText} numberOfLines={1}>
          {who} asks: {w.prompt}
        </Text>
        <Text style={styles.barAnswer}>Answer</Text>
      </Pressable>
    );

  return (
    <View style={[styles.card, { paddingBottom: 8 + bottomInset }]} accessibilityLabel={`${who} asks: ${w.prompt}`}>
      <View style={styles.top}>
        <View style={styles.who}>
          <View style={styles.dot} />
          <Text style={styles.whoText}>{who} asks</Text>
        </View>
        <Text style={styles.since}>{since}</Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Minimize"
          onPress={() => setMin(true)}
          hitSlop={4}
          style={({ pressed }) => [styles.minimize, pressed && styles.pressed]}
        >
          <Icon name="chevronDown" size={16} color={colors.text} />
        </Pressable>
      </View>
      <ScrollView style={{ maxHeight: height * (others.length > MANY_ANSWERS ? 0.3 : 0.55) }} contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled" bounces={false}>
        <PromptText text={w.prompt} style={styles.title} monoSize={14} />
        {ctx.file || rows.length > 0 || ctx.meta.length > 0 ? (
          <View style={styles.context} accessible={false}>
            {ctx.file || ctx.rows.length > 0 ? (
              <View style={[styles.fileRow, (rows.length > 0 || ctx.meta.length > 0) && styles.fileRule]}>
                <Icon name="file" size={14} color={colors.textMuted} />
                <Text style={styles.file}>{ctx.file ?? "Changes"}</Text>
                {ctx.rows.length > 0 ? (
                  <>
                    <Text style={[styles.count, { color: colors.success }]}>+{ctx.added}</Text>
                    <Text style={[styles.count, { color: colors.text }]}>−{ctx.removed}</Text>
                  </>
                ) : null}
                {hasMoreRows(ctx) ? (
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={full ? "Less of the diff" : "Full diff"}
                    onPress={() => setFull((f) => !f)}
                    hitSlop={{ top: 8, bottom: 8 }}
                    style={({ pressed }) => [styles.diffToggle, pressed && styles.pressed]}
                  >
                    <Text style={styles.diffToggleText}>{full ? "Less" : "Full diff"}</Text>
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
      {replying && replyAt !== null ? (
        <View style={styles.answers}>
          <View style={styles.replyField}>
            <TextInput
              value={reply}
              onChangeText={setReply}
              autoFocus
              placeholder={`Tell ${who} what to do instead`}
              placeholderTextColor={colors.textMuted}
              accessibilityLabel={`Tell ${who} what to do instead`}
              returnKeyType="send"
              onSubmitEditing={() => void send({ choice: replyAt, reply }, "Sent: No")}
              style={styles.replyInput}
            />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Send ${w.choices[replyAt]}`}
              accessibilityState={{ disabled: busy }}
              disabled={busy}
              onPress={() => void send({ choice: replyAt, reply }, "Sent: No")}
              hitSlop={4}
              style={({ pressed }) => [styles.replySend, busy && styles.disabled, pressed && styles.pressed]}
            >
              <Icon name="run" size={18} color={colors.bg} />
            </Pressable>
          </View>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Back to the answers"
            onPress={() => {
              setReplying(false);
              setReply("");
            }}
            style={({ pressed }) => [styles.back, pressed && styles.pressed]}
          >
            <Text style={styles.backText}>Back to the answers</Text>
          </Pressable>
        </View>
      ) : (
        // a long menu scrolls here, so its last entries and the Keys (Esc leaves it) stay on the screen
        <ScrollView style={{ maxHeight: height * 0.34 }} contentContainerStyle={styles.answers} keyboardShouldPersistTaps="handled" nestedScrollEnabled>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={w.choices[primary]}
            accessibilityState={{ disabled: busy }}
            disabled={busy}
            onPress={() => void send({ choice: primary }, `Sent: ${answerLabel(w, primary)}`)}
            style={({ pressed }) => [styles.primary, busy && styles.disabled, pressed && styles.pressed]}
          >
            <Text style={styles.primaryText} numberOfLines={1}>
              {answerLabel(w, primary)}
            </Text>
          </Pressable>
          <View style={styles.others}>
            {others.map((i) => (
              <Pressable
                key={i}
                accessibilityRole="button"
                accessibilityLabel={w.choices[i]}
                accessibilityState={{ disabled: busy }}
                disabled={busy}
                onPress={() => (i === replyAt ? setReplying(true) : void send({ choice: i }, `Sent: ${answerLabel(w, i)}`))}
                style={({ pressed }) => [styles.outline, busy && styles.disabled, pressed && styles.pressed]}
              >
                <Text style={styles.outlineText} numberOfLines={1}>
                  {answerLabel(w, i)}
                </Text>
              </Pressable>
            ))}
          </View>
        </ScrollView>
      )}
      <View style={styles.keys} accessibilityRole="toolbar" accessibilityLabel="Keys">
        <Text style={styles.keysCaption}>Keys</Text>
        {KEYS.map((k) => (
          <Pressable
            key={k.key}
            accessibilityRole="button"
            accessibilityLabel={k.label}
            onPress={() => void sendKey(k.key, k.cap)}
            hitSlop={{ top: 5, bottom: 5 }}
            style={({ pressed }) => [styles.key, pressed && styles.pressed]}
          >
            <Text style={styles.keyText}>{k.cap}</Text>
          </Pressable>
        ))}
      </View>
    </View>
  );
}

/** Past this many answers under the primary one, the question gives up room to them. */
const MANY_ANSWERS = 4;

const styles = StyleSheet.create({
  pressed: { opacity: 0.7 },
  disabled: { opacity: 0.45 },
  // the minimized bar: 52 pt, a pill on a warm dark ground with the caution colour's border
  bar: {
    height: 52,
    marginHorizontal: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 16,
    borderRadius: 26,
    borderWidth: 1,
    borderColor: `${colors.warning}55`,
    backgroundColor: "#201808",
  },
  barText: { flex: 1, minWidth: 0, fontFamily: ui(400), color: colors.textBright, fontSize: 14 },
  barAnswer: { fontFamily: ui(600), color: colors.warning, fontSize: 14 },
  dot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.warning },
  card: {
    backgroundColor: colors.surface,
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    borderTopWidth: 1,
    borderTopColor: `${colors.warning}55`,
    paddingTop: 12,
    paddingHorizontal: 16,
    gap: 12,
  },
  top: { flexDirection: "row", alignItems: "center", gap: 8 },
  who: { height: 26, flexDirection: "row", alignItems: "center", gap: 6, paddingHorizontal: 10, borderRadius: 13, backgroundColor: tints.warningBadge },
  whoText: { fontFamily: ui(600), color: colors.warning, fontSize: 13 },
  since: { flex: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  minimize: { width: 36, height: 36, borderRadius: 18, backgroundColor: colors.glass, alignItems: "center", justifyContent: "center" },
  content: { gap: 6 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 17, lineHeight: 24 },
  context: { borderRadius: 12, backgroundColor: colors.chrome, overflow: "hidden" },
  fileRow: { flexDirection: "row", alignItems: "center", gap: 8, paddingVertical: 8, paddingLeft: 12, paddingRight: 6 },
  fileRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  runs: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17 },
  runsCommand: { fontFamily: mono, color: colors.textBright, fontSize: 12 },
  file: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.textBright, fontSize: 13 },
  count: { fontFamily: mono, fontSize: 12 },
  diffToggle: { height: 28, paddingHorizontal: 10, borderRadius: 14, borderWidth: 1, borderColor: colors.borderStrong, justifyContent: "center" },
  diffToggleText: { fontFamily: ui(500), color: colors.text, fontSize: 12 },
  diffBody: { maxHeight: 220, paddingVertical: 6 },
  diffRow: { flexDirection: "row", gap: 10, paddingHorizontal: 12 },
  diffAdded: { backgroundColor: tints.answerAdded },
  diffRemoved: { backgroundColor: tints.answerRemoved },
  diffNum: { width: 22, textAlign: "right", fontFamily: mono, fontSize: 12, lineHeight: 19 },
  diffSign: { fontFamily: mono, fontSize: 12, lineHeight: 19 },
  diffText: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.textBright, fontSize: 12, lineHeight: 19 },
  metaBody: { paddingVertical: 6, paddingHorizontal: 12, gap: 2 },
  meta: { fontFamily: mono, color: colors.textMuted, fontSize: 12, lineHeight: 19 },
  answers: { gap: 8 },
  primary: { height: 50, borderRadius: 25, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center", paddingHorizontal: 16 },
  primaryText: { fontFamily: ui(600), color: colors.bg, fontSize: 16 },
  others: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  outline: {
    flexGrow: 1,
    flexBasis: "45%",
    height: 46,
    borderRadius: 23,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 12,
  },
  outlineText: { fontFamily: ui(500), color: colors.text, fontSize: 14 },
  replyField: {
    height: 50,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 16,
    paddingRight: 5,
    borderRadius: 25,
    borderWidth: 1,
    borderColor: colors.accent,
    backgroundColor: colors.bg,
  },
  replyInput: { flex: 1, minWidth: 0, padding: 0, fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  replySend: { width: 40, height: 40, borderRadius: 20, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center" },
  back: { height: 36, alignItems: "center", justifyContent: "center" },
  backText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
  keys: { flexDirection: "row", alignItems: "center", gap: 6 },
  // textMuted: the mockup's textDim is 4.49:1 on a surface (palette.ts)
  keysCaption: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, marginRight: 2 },
  key: {
    height: 34,
    minWidth: 44,
    paddingHorizontal: 10,
    borderRadius: 17,
    borderWidth: 1,
    borderColor: colors.glassRim,
    backgroundColor: colors.bg,
    alignItems: "center",
    justifyContent: "center",
  },
  keyText: { fontFamily: ui(500), color: colors.text, fontSize: 13 },
});
