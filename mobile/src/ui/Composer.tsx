import * as Clipboard from "expo-clipboard";
import { useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  AccessibilityInfo,
  Keyboard,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { KeyCaptureView, type KeyEvent } from "../../modules/rift-keys";
import { commandProblem, type CompleteMsg } from "@/protocol/types";
import { draftKey } from "@/fleet";
import { applySticky, keyEventToAction, stickyKey } from "@/keyboard";
import { keysNeedAuth } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { chipRowMode, completionRows, NO_COMPLETIONS, NO_COMPLETIONS_PROGRAM } from "@/chips";
import { programSend } from "@/paneView";
import type { ChipItem, ChipOpen } from "@/promptChips";
import { showsKeyRow } from "@/terminal";
import { ChipRow } from "./ChipRow";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { insertedChar, plainPunctuation } from "./format";
import { Icon } from "./Icon";
import { ErrorText } from "./Notice";
import { useOpenShell } from "./useOpenShell";
import { colors, mono, TOUCH, ui } from "./theme";
import { toggleA11y } from "./toggle";

/** Suggestions shown above the command line (the P3 Pane board). */
const MAX_SUGGESTIONS = 6;
/** Wait this long after typing stops before asking the computer for completions. */
const COMPLETE_DELAY_MS = 200;

/**
 * The pane's command line (operate permission only; the P3 Pane board): one row above the field
 * that is the prompt chips while the field is empty (the desktop's, in its order, as many as fit and
 * "+N" for the rest; each opens its menu) and the completions while the user types (the desktop's, feature `complete`, and this
 * pane's history, typed part bright, the rest dim, with where each comes from); the round field
 * with its run button; and ⌃, which shows the control-key row (Settings › Keys) under the field,
 * so the field stays in sight above it while the keyboard is up (Return runs too; through the Confirm sheet only with Settings › Ask before
 * running on). While a program runs (`running`) the field talks to it instead: Return sends the
 * line as text, then Return as a key (feature `text`), with no Confirm sheet. A hardware
 * keyboard's ctrl/alt combinations, Escape, Tab and F-keys in the field go to the pane as keys.
 * While the host is not connected (`online` false) nothing can be sent; the draft stays editable.
 */
export function Composer({
  hostKey,
  paneId,
  running,
  program,
  online,
  chips,
  complete,
  recent,
  label,
  canType,
  waiting = false,
}: {
  hostKey: string;
  paneId: number;
  running: string | null;
  /** The running program's name ("claude"), for the field's placeholder. */
  program: string;
  online: boolean;
  /** The pane's prompt chips (promptChips.ts `chipItems`). */
  chips: ChipItem[];
  /** The computer completes command lines (feature `complete`). */
  complete: boolean;
  /** The pane's recent commands, newest first (suggestions without `complete`). */
  recent: string[];
  /** The pane's title, for the field's label ("Command for rift · pane 1"). */
  label: string;
  /** The computer takes typed text (feature `text`): the row's Paste works. */
  canType: boolean;
  /** The computer is not connected (PaneLoading board): the field is off and says it waits. */
  waiting?: boolean;
}) {
  const router = useRouter();
  const s = useApp();
  const draft = s.drafts[draftKey(hostKey, paneId)] ?? "";
  const [sticky, setSticky] = useState<Sticky>(NO_STICKY);
  const [error, setError] = useState<string | null>(null);
  // Return was pressed while not connected
  const [offlineTried, setOfflineTried] = useState(false);
  const shownError =
    error ?? (offlineTried && !online ? running !== null ? "Not connected. The message stays here; Return sends it once the computer is back." : "Not connected. The command stays here; Return runs it once the computer is back." : null);
  // a command on its way: a ref, not state, so a second Return before the re-render sends nothing
  const sending = useRef(false);
  // ⌃: the key row in place of the suggestions; Settings › Show keys opens it to start with
  const [keys, setKeys] = useState(() => showsKeyRow(s.settings.showKeys, false));
  const [completed, setCompleted] = useState<{ draft: string; msg: CompleteMsg } | null>(null);
  // the row leads with Hide keyboard while the keyboard is up
  const [keyboardUp, setKeyboardUp] = useState(false);
  useEffect(() => {
    const ios = Platform.OS === "ios";
    const show = Keyboard.addListener(ios ? "keyboardWillShow" : "keyboardDidShow", () => setKeyboardUp(true));
    const hide = Keyboard.addListener(ios ? "keyboardWillHide" : "keyboardDidHide", () => setKeyboardUp(false));
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);

  // the desktop's completions for the draft, then the pane's history
  const recentKey = recent.join("\n");
  useEffect(() => {
    if (!complete || !online) return;
    let live = true;
    const t = setTimeout(() => {
      void fleet.complete(hostKey, paneId, draft).then((r) => {
        if (!live || !r.ok) return;
        setCompleted({ draft, msg: r.msg });
      });
    }, COMPLETE_DELAY_MS);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [complete, online, hostKey, paneId, draft, recentKey]);

  const sendKeys = async (k: string[]) => {
    setError(null);
    // A key that submits the line runs a command like Run: it asks the same way (lock.ts).
    if (keysNeedAuth(k, s.settings)) {
      const auth = await authenticate(`Send ${k.join(" ")}`);
      if (!auth.ok) {
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    const r = await fleet.sendKeys(hostKey, paneId, k);
    if (!r.ok) setError(r.message);
  };

  const onChange = (typed: string) => {
    const text = plainPunctuation(typed);
    if (sticky.ctrl || sticky.alt || sticky.shift) {
      setSticky(NO_STICKY);
      const ch = insertedChar(draft, text);
      const key = ch ? stickyKey(ch, sticky) : null;
      if (key) {
        void sendKeys([key]);
        return; // the character is a key, not text
      }
    }
    setError(null);
    fleet.setDraft(hostKey, paneId, text);
  };

  const onOpen = useOpenShell(hostKey, paneId, setError);

  const onRowKey = (id: string) => {
    const key = applySticky(id, sticky);
    setSticky(NO_STICKY);
    void sendKeys([key]);
  };

  // the row's Paste: the phone's clipboard into the command line at the prompt, else typed
  // into the running program (the computer takes no text at its prompt)
  const onPaste = async () => {
    const text = await Clipboard.getStringAsync();
    if (!text) return;
    setError(null);
    if (running === null) return fleet.setDraft(hostKey, paneId, plainPunctuation(draft + text));
    const r = await fleet.sendText(hostKey, paneId, text);
    if (!r.ok) setError(r.message);
  };

  // hardware shortcuts taken from the command line (modules/rift-keys, shortcuts mode)
  const onShortcut = (e: KeyEvent) => {
    if (!online) return;
    const action = keyEventToAction(e);
    if (action && "keys" in action) onRowKey(action.keys[0]!);
  };

  // A program is running: the line goes to it as text, then Return as its own request (a program
  // such as Claude Code reads text with a Return in the same write as a paste, not a send).
  const sendToProgram = async () => {
    const p = programSend(draft, canType);
    if (!p.ok) {
      if (p.message) setError(p.message);
      return;
    }
    sending.current = true;
    setError(null);
    try {
      // the Return submits the line: it asks like Run does, before any text goes out
      if (keysNeedAuth(["enter"], s.settings)) {
        const auth = await authenticate("Send enter");
        if (!auth.ok) {
          if (auth.message !== "Cancelled.") setError(auth.message);
          return;
        }
      }
      const t = await fleet.sendText(hostKey, paneId, p.text);
      if (!t.ok) return setError(t.message);
      // the program has the text: the line is spent even if the Return fails
      fleet.clearDraft(hostKey, paneId, draft);
      const k = await fleet.sendKeys(hostKey, paneId, ["enter"]);
      if (!k.ok) return setError(k.message);
      AccessibilityInfo.announceForAccessibility("Sent");
    } finally {
      sending.current = false;
    }
  };

  const problem = commandProblem(draft);
  const run = async () => {
    if (sending.current) return;
    if (!online) {
      // Return did nothing at all before: say why, and that the line is kept (shown while offline)
      setOfflineTried(true);
      return;
    }
    setOfflineTried(false);
    if (running !== null) return void sendToProgram();
    if (problem) {
      if (problem !== "empty")
        setError(problem === "too_long" ? "The command is too long (2 KiB at most)." : "The command must be one line.");
      return;
    }
    if (s.settings.askBeforeRun) {
      // the sheet opened over the keyboard would take the full height, under the status bar
      Keyboard.dismiss();
      router.push({ pathname: "/confirm", params: { key: hostKey, pane: String(paneId) } });
      return;
    }
    sending.current = true;
    setError(null);
    const r = await fleet.run(hostKey, paneId, draft);
    sending.current = false;
    if (!r.ok) setError(r.message);
    else AccessibilityInfo.announceForAccessibility("Command sent");
  };

  const openMenu = (menu: ChipOpen) => {
    // the sheet opened over the keyboard would take the full height
    Keyboard.dismiss();
    router.push({ pathname: "/chips", params: { key: hostKey, pane: String(paneId), menu } });
  };
  const openAll = () => {
    Keyboard.dismiss();
    router.push({ pathname: "/pane-chips", params: { key: hostKey, pane: String(paneId) } });
  };
  const stickyName = [sticky.ctrl ? "ctrl" : null, sticky.alt ? "alt" : null, sticky.shift ? "shift" : null].filter(Boolean).join("+");
  const canRun = online && !waiting && draft.trim() !== "";
  // the computer's answer for this draft (or the last one, while the next is on its way), then the pane's history
  const typing = chipRowMode(draft) === "completions";
  const answer = complete && online ? completed : null;
  const rows = useMemo(
    () => (typing ? completionRows(draft, answer?.msg.items ?? [], answer?.msg.history ?? [], recentKey === "" ? [] : recentKey.split("\n"), MAX_SUGGESTIONS) : []),
    [typing, draft, answer, recentKey],
  );
  // "No completions" only once the computer has answered for this very draft
  const settled = !(complete && online) || completed?.draft === draft;
  const hasChips = chips.length > 0;

  return (
    <KeyCaptureView
      mode="shortcuts"
      // Tab completes for a program or the shell's line; with nothing to complete it moves focus
      captureTab={running !== null || draft !== ""}
      onKey={onShortcut}
      style={styles.wrap}
    >
      {typing ? (
        <ScrollView
          // keyed by the first completion: a new list opens at its start, not where the last one was scrolled
          key={`completions:${rows[0]?.text}`}
          horizontal
          showsHorizontalScrollIndicator={false}
          style={styles.chipStrip}
          contentContainerStyle={styles.chips}
          keyboardShouldPersistTaps="always"
          accessibilityRole="toolbar"
          accessibilityLabel="Completions"
        >
          {rows.length === 0 ? (
            settled ? (
              <Text style={styles.none} accessibilityLiveRegion="polite">
                {running !== null ? NO_COMPLETIONS_PROGRAM : NO_COMPLETIONS}
              </Text>
            ) : null
          ) : (
            rows.map((r) => (
              <Pressable
                key={r.text}
                accessibilityRole="button"
                accessibilityLabel={`${r.text}, ${r.source}`}
                accessibilityHint="Puts it in the command line"
                onPress={() => fleet.setDraft(hostKey, paneId, r.text)}
                hitSlop={{ top: 6, bottom: 6 }}
                style={({ pressed }) => [styles.chip, styles.suggestion, pressed && styles.pressed]}
              >
                <Text style={styles.chipText} numberOfLines={1}>
                  <Text style={styles.typed}>{r.typed}</Text>
                  <Text style={styles.rest}>{r.rest}</Text>
                </Text>
                <Text style={styles.source}>{r.source}</Text>
              </Pressable>
            ))
          )}
        </ScrollView>
      ) : hasChips ? (
        <ChipRow items={chips} onOpen={(c) => c.opens && openMenu(c.opens)} onMore={openAll} />
      ) : null}
      {shownError ? <ErrorText>{shownError}</ErrorText> : null}
      <View style={[styles.inputRow, waiting && styles.waiting]}>
        <Pressable
          {...toggleA11y(keys, waiting)}
          disabled={waiting}
          accessibilityLabel="Control keys"
          accessibilityHint="Shows the control keys above the command line"
          onPress={() => setKeys((k) => !k)}
          style={({ pressed }) => [styles.round, keys && styles.roundOn, pressed && styles.pressed]}
        >
          <Text style={[styles.keysLabel, keys && { color: colors.accent }]}>⌃</Text>
        </Pressable>
        <View style={styles.field}>
          <TextInput
            value={draft}
            onChangeText={onChange}
            onSubmitEditing={() => void run()}
            editable={!waiting}
            placeholder={
              waiting
                ? "Waiting for your computer…"
                : stickyName
                  ? `Type a letter for ${stickyName}+letter`
                  : running !== null
                    ? `Message for ${program}`
                    : "Command"
            }
            placeholderTextColor={colors.textMuted}
            autoCapitalize="none"
            autoCorrect={false}
            spellCheck={false}
            returnKeyType="send"
            submitBehavior="submit"
            accessibilityLabel={running !== null ? `Message for ${program} in ${label}` : `Command for ${label}`}
            accessibilityHint={
              running !== null ? "Return sends it to the program" : s.settings.askBeforeRun ? "Return shows the command first" : "Return runs it"
            }
            style={styles.input}
          />
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={running !== null ? "Send" : "Run"}
            accessibilityHint={running === null && s.settings.askBeforeRun ? "Shows the command first" : undefined}
            accessibilityState={{ disabled: !canRun }}
            disabled={!canRun}
            onPress={() => void run()}
            hitSlop={4}
            style={({ pressed }) => [styles.send, canRun && styles.sendOn, pressed && styles.pressed]}
          >
            <Icon name="run" size={18} color={colors.bg} />
          </Pressable>
        </View>
      </View>
      {keys ? (
        <ControlRow
          ids={s.settings.controlKeys}
          sticky={sticky}
          disabled={!online}
          onSticky={setSticky}
          onKey={onRowKey}
          onOpen={onOpen}
          onPaste={canType || running === null ? () => void onPaste() : undefined}
          onEdit={() => router.push("/control-keys")}
          onHideKeyboard={keyboardUp ? () => Keyboard.dismiss() : undefined}
        />
      ) : null}
    </KeyCaptureView>
  );
}

// The Pane board's command line: chrome ground, 8 / 12 pt padding, 8 pt between rows.
const styles = StyleSheet.create({
  wrap: {
    backgroundColor: colors.chrome,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
    paddingHorizontal: 12,
    paddingTop: 8,
    paddingBottom: 8,
    gap: 8,
  },
  pressed: { opacity: 0.7 },
  // the completions' strip reaches 6 pt past the 32 pt chips (a scroll view takes no touch outside it): a 44 pt touch
  chipStrip: { marginVertical: -6, flexGrow: 0 },
  chips: { gap: 6, paddingVertical: 6 },
  // a completion: 32 pt, 8 pt radius, mono 12, outlined
  chip: { height: 32, paddingHorizontal: 10, borderRadius: 8, flexDirection: "row", alignItems: "center", gap: 5, maxWidth: 260 },
  suggestion: { borderWidth: 1, borderColor: colors.glassRim },
  chipText: { fontFamily: mono, color: colors.text, fontSize: 12, flexShrink: 1 },
  // a completion: what was typed bright, the rest dim, the source small after it
  typed: { color: colors.textBright },
  rest: { color: colors.textMuted },
  source: { fontFamily: ui(400), color: colors.textDim, fontSize: 10 },
  none: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, alignSelf: "center" },
  inputRow: { flexDirection: "row", gap: 8, alignItems: "center" },
  // the pane has not opened: the board dims the whole row
  waiting: { opacity: 0.5 },
  // the board's 44 pt round button and the round field with its 36 pt run button
  round: {
    width: TOUCH,
    height: TOUCH,
    borderRadius: TOUCH / 2,
    borderWidth: 1,
    borderColor: colors.glassRim,
    backgroundColor: colors.surface,
    alignItems: "center",
    justifyContent: "center",
  },
  roundOn: { borderColor: colors.accent },
  keysLabel: { fontFamily: ui(500), color: colors.text, fontSize: 15 },
  field: {
    flex: 1,
    minWidth: 0,
    height: TOUCH,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 16,
    paddingRight: 4,
    borderRadius: TOUCH / 2,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
  },
  input: { flex: 1, minWidth: 0, height: TOUCH - 2, padding: 0, color: colors.textBright, fontFamily: mono, fontSize: 14 },
  // the board dims the run button to 40 % while the field is empty
  send: { width: 36, height: 36, borderRadius: 18, backgroundColor: `${colors.accent}66`, alignItems: "center", justifyContent: "center" },
  sendOn: { backgroundColor: colors.accent },
});
