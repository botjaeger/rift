import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useRef, useState } from "react";
import {
  AccessibilityInfo,
  Keyboard,
  KeyboardAvoidingView,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { askingTitle, contextLine, keepAsked, primaryChoice, replyChoice, sameQuestion, type Pick } from "@/answer";
import { programName } from "@/now";
import { canOperate } from "@/fleet";
import { needsAuthBeforeRun } from "@/lock";
import type { Waiting } from "@/protocol/types";
import { authenticate, fleet, useApp } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { PromptText } from "@/ui/PromptText";
import { PageHeader } from "@/ui/Screen";
import { colors, mono, tints, ui } from "@/ui/theme";

/** The agent's lines above its question, a diff drawn as the desktop's review panel draws one. */
function Context({ lines }: { lines: string[] }) {
  return (
    <View style={styles.context} accessible accessibilityLabel={`What it is about: ${lines.join(". ")}`}>
      {lines.map((l, i) => {
        const c = contextLine(l);
        return (
          <Text
            key={i}
            style={[
              styles.ctxLine,
              i === 0 && styles.ctxFirst,
              i === lines.length - 1 && styles.ctxLast,
              c.kind === "meta" && styles.ctxMeta,
              c.kind === "added" && styles.ctxAdded,
              c.kind === "removed" && styles.ctxRemoved,
            ]}
            numberOfLines={1}
          >
            {c.text || " "}
          </Text>
        );
      })}
    </View>
  );
}

/**
 * Answering a pane that waits for the user (the P2 Reply board): an agent's menu as buttons,
 * with "No, and tell it what to do" as a field; a password or other question as a field.
 */
export default function AnswerScreen() {
  const { key, pane: paneParam, reply, from } = useLocalSearchParams<{ key: string; pane: string; reply?: string; from?: string }>();
  const paneId = Number(paneParam);
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const live = loc?.pane.waiting;
  // the question as first shown: an answer goes only to it (fleet.answer checks it still asks)
  const at = `${key}:${paneId}`;
  const [asked, setAsked] = useState<{ where: string; w: Waiting | undefined }>(() => ({ where: at, w: live }));
  const next = keepAsked(asked, at, live);
  if (next !== asked) setAsked(next);
  const w = next.w;
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  // a second tap before the re-render that disables the buttons sends nothing
  const sending = useRef(false);
  const [error, setError] = useState<string | null>(null);

  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));
  // opened from the pane: back to it, not a second copy of it (whose Back would lead to the first)
  const openPane = () =>
    from === "pane" ? back() : router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(paneId) } });
  const program = loc ? programName(loc.pane) : null;
  const where = [host?.record.name, loc ? tabTitle(loc.tab) : null, loc ? `pane ${loc.paneIndex + 1}` : null].filter(Boolean).join(" · ");
  const operate = host ? canOperate(host) : false;
  const gone = !!w && !sameQuestion(live, w);

  const send = async (pick: Pick) => {
    if (!w || sending.current) return;
    sending.current = true;
    setError(null);
    if (needsAuthBeforeRun(s.settings)) {
      const auth = await authenticate(`Answer ${program ?? "the program"} on ${host?.record.name ?? "the computer"}`);
      if (!auth.ok) {
        sending.current = false;
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    setBusy(true);
    const r = await fleet.answer(key, paneId, w, pick);
    setBusy(false);
    sending.current = false;
    if (!r.ok) {
      setError(r.message);
      return;
    }
    setText("");
    Keyboard.dismiss();
    AccessibilityInfo.announceForAccessibility("Answer sent");
    back();
  };

  const replyAt = w ? replyChoice(w) : null;
  const primary = w ? primaryChoice(w) : 0;
  const numbered = w?.kind === "choice" && w.numbered;
  const label = (i: number) => (numbered ? `${i + 1} · ${w!.choices[i]}` : w!.choices[i]!);
  const name = program ?? "the program";

  return (
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      <Stack.Screen options={{ headerShown: false }} />
      <PageHeader
        chrome
        title={w ? askingTitle(w, program) : "Nothing is waiting"}
        titleSize={15}
        subtitle={where}
        subtitleLines={1}
        back={back}
        backLabel={from === "pane" ? "Back to the pane" : "Back to Portals"}
        right={
          <Pressable
            accessibilityRole="link"
            accessibilityLabel="Open pane"
            onPress={openPane}
            style={({ pressed }) => [styles.openPane, pressed && styles.pressed]}
          >
            <Text style={styles.openPaneText}>Open pane</Text>
          </Pressable>
        }
      />
      <ScrollView style={styles.fill} contentContainerStyle={styles.body} keyboardShouldPersistTaps="handled">
        {w ? (
          <>
            <PromptText text={w.prompt} style={styles.prompt} monoSize={14} />
            {w.context.length > 0 ? <Context lines={w.context} /> : null}
            <Text style={styles.note}>
              {w.kind === "password"
                ? `Typed into ${name} on ${host?.record.name ?? "the computer"}. This phone keeps no copy.`
                : w.kind === "choice"
                  ? "The last lines of the pane, as the agent printed them. Open pane for everything."
                  : "The program asked for input. Open pane to see what it printed."}
            </Text>
          </>
        ) : (
          <Text style={styles.note}>Nothing in this pane waits for an answer. Open pane to see it.</Text>
        )}
        {w && gone ? (
          <Text style={styles.gone} accessibilityLiveRegion="polite">
            This question was answered, or the program moved on.
          </Text>
        ) : null}
      </ScrollView>
      {w && !gone ? (
        <View style={[styles.answer, { paddingBottom: Math.max(16, insets.bottom + 0) }]} accessibilityLabel="Answer">
          {error ? <ErrorText>{error}</ErrorText> : null}
          {!operate ? (
            <Text style={styles.viewOnly}>View only: this device cannot answer here.</Text>
          ) : w.kind === "choice" ? (
            <>
              {w.choices.map((_, i) =>
                i === replyAt ? null : (
                  <Pressable
                    key={i}
                    accessibilityRole="button"
                    accessibilityLabel={label(i)}
                    accessibilityState={{ disabled: busy }}
                    disabled={busy}
                    onPress={() => void send({ choice: i })}
                    style={({ pressed }) => [styles.choice, i === primary ? styles.choicePrimary : styles.choiceOutline, pressed && styles.pressed]}
                  >
                    <Text style={i === primary ? styles.choicePrimaryText : styles.choiceText} numberOfLines={2}>
                      {label(i)}
                    </Text>
                  </Pressable>
                ),
              )}
              {replyAt !== null ? (
                <View style={styles.replyRow}>
                  <TextInput
                    value={text}
                    onChangeText={setText}
                    autoFocus={reply === "1"}
                    placeholder={`${numbered ? `${replyAt + 1} · ` : ""}No, and tell ${name} what to do…`}
                    placeholderTextColor={colors.textMuted}
                    accessibilityLabel={`Tell ${name} what to do instead`}
                    accessibilityHint={`Sends ${numbered ? replyAt + 1 : "No"}, then what you typed`}
                    returnKeyType="send"
                    // Return on an empty field sends nothing: the send button sends a bare No
                    onSubmitEditing={() => text.trim() !== "" && void send({ choice: replyAt, reply: text })}
                    style={styles.replyInput}
                  />
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={text.trim() ? "Send No and the reply" : `Send ${w.choices[replyAt]}`}
                    accessibilityState={{ disabled: busy }}
                    disabled={busy}
                    onPress={() => void send({ choice: replyAt, reply: text })}
                    style={({ pressed }) => [styles.sendButton, pressed && styles.pressed]}
                  >
                    <Icon name="run" color={colors.accent} />
                  </Pressable>
                </View>
              ) : null}
            </>
          ) : (
            <View style={styles.replyRow}>
              <TextInput
                value={text}
                onChangeText={setText}
                autoFocus
                secureTextEntry={w.kind === "password"}
                autoCapitalize="none"
                autoCorrect={false}
                spellCheck={false}
                // the computer's password: never offered to, or saved by, this phone's password manager
                textContentType="none"
                autoComplete="off"
                importantForAutofill="noExcludeDescendants"
                placeholder={w.kind === "password" ? "Password" : "Answer"}
                placeholderTextColor={colors.textMuted}
                accessibilityLabel={w.kind === "password" ? w.prompt : `Answer for ${name}`}
                returnKeyType="send"
                onSubmitEditing={() => text !== "" && void send({ text })}
                style={styles.replyInput}
              />
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Send"
                accessibilityState={{ disabled: busy || text === "" }}
                disabled={busy || text === ""}
                onPress={() => void send({ text })}
                style={({ pressed }) => [styles.sendButton, (busy || text === "") && styles.disabled, pressed && styles.pressed]}
              >
                <Icon name="run" color={colors.accent} />
              </Pressable>
            </View>
          )}
        </View>
      ) : null}
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  openPane: { height: 44, paddingHorizontal: 12, justifyContent: "center" },
  openPaneText: { fontFamily: ui(400), color: colors.accent, fontSize: 14 },
  body: { padding: 16, gap: 14 },
  prompt: { fontFamily: ui(400), color: colors.textBright, fontSize: 17, lineHeight: 24.6 },
  context: { borderRadius: 10, backgroundColor: colors.chrome, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  ctxLine: { fontFamily: mono, fontSize: 12, lineHeight: 19.2, color: colors.fg, paddingHorizontal: 12 },
  ctxFirst: { paddingTop: 6 },
  ctxLast: { paddingBottom: 6 },
  ctxMeta: { fontFamily: mono, fontSize: 11, color: colors.textMuted },
  ctxAdded: { backgroundColor: tints.answerAdded, color: colors.textBright },
  ctxRemoved: { backgroundColor: tints.answerRemoved, color: colors.textBright },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18.9 },
  gone: { fontFamily: ui(400), color: colors.warning, fontSize: 14 },
  answer: {
    backgroundColor: colors.chrome,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
    paddingTop: 12,
    paddingHorizontal: 16,
    gap: 10,
  },
  viewOnly: { fontFamily: ui(400), color: colors.textDim, fontSize: 13, textAlign: "center", paddingVertical: 6 },
  choice: { minHeight: 52, borderRadius: 10, alignItems: "center", justifyContent: "center", paddingHorizontal: 14, paddingVertical: 6 },
  choicePrimary: { backgroundColor: colors.accent },
  choiceOutline: { borderWidth: 1, borderColor: colors.borderStrong },
  choicePrimaryText: { fontFamily: ui(600), color: colors.bg, fontSize: 16, textAlign: "center" },
  choiceText: { fontFamily: ui(400), color: colors.text, fontSize: 16, textAlign: "center" },
  replyRow: { flexDirection: "row", gap: 8, alignItems: "center" },
  replyInput: {
    flex: 1,
    height: 48,
    paddingHorizontal: 12,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontFamily: ui(400),
    fontSize: 15,
  },
  sendButton: { width: 48, height: 48, borderRadius: 10, backgroundColor: colors.surface, alignItems: "center", justifyContent: "center" },
});
