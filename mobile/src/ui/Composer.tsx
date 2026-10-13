import { useRouter } from "expo-router";
import { useState } from "react";
import { AccessibilityInfo, Keyboard, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { KeyCaptureView, type KeyEvent } from "../../modules/rift-keys";
import { commandProblem } from "@/protocol/types";
import { draftKey } from "@/fleet";
import { applySticky, keyEventToAction, stickyKey } from "@/keyboard";
import { keysNeedAuth } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { insertedChar } from "./format";
import { ErrorText } from "./Notice";
import { colors, mono, radius, TOUCH } from "./theme";

/**
 * The pane's command composer (operate permission only): the control-key row (Settings ›
 * Control keys) sent with `keys`, and a command line run with `run` (through the Confirm sheet
 * unless that is off). A hardware keyboard's ctrl/alt combinations, Escape, Tab and F-keys in
 * the command line go to the pane as keys too. While the host is not connected (`online`
 * false) nothing can be sent; the draft stays editable.
 */
export function Composer({
  hostKey,
  paneId,
  running,
  online,
}: {
  hostKey: string;
  paneId: number;
  running: string | null;
  online: boolean;
}) {
  const router = useRouter();
  const s = useApp();
  const draft = s.drafts[draftKey(hostKey, paneId)] ?? "";
  const [sticky, setSticky] = useState<Sticky>(NO_STICKY);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  const sendKeys = async (keys: string[]) => {
    setError(null);
    // A key that submits the line runs a command like Run: it asks the same way (lock.ts).
    if (keysNeedAuth(keys, s.settings)) {
      const auth = await authenticate(`Send ${keys.join(" ")}`);
      if (!auth.ok) {
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    const r = await fleet.sendKeys(hostKey, paneId, keys);
    if (!r.ok) setError(r.message);
  };

  const onChange = (text: string) => {
    if (sticky.ctrl || sticky.alt) {
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

  const onRowKey = (id: string) => {
    const key = applySticky(id, sticky);
    setSticky(NO_STICKY);
    void sendKeys([key]);
  };

  // hardware shortcuts taken from the command line (modules/rift-keys, shortcuts mode)
  const onShortcut = (e: KeyEvent) => {
    if (!online) return;
    const action = keyEventToAction(e);
    if (action && "keys" in action) onRowKey(action.keys[0]!);
  };

  const problem = commandProblem(draft);
  // Too long or multi-line stays pressable: the press says what is wrong.
  const runDisabled = !online || problem === "empty" || sending;
  const run = async () => {
    if (!online) return;
    if (problem) {
      if (problem !== "empty")
        setError(problem === "too_long" ? "The command is too long (2 KiB at most)." : "The command must be one line.");
      return;
    }
    if (s.settings.confirmEveryCommand) {
      // the sheet opened over the keyboard would take the full height, under the status bar
      Keyboard.dismiss();
      router.push({ pathname: "/confirm", params: { key: hostKey, pane: String(paneId) } });
      return;
    }
    setSending(true);
    setError(null);
    const r = await fleet.run(hostKey, paneId, draft);
    setSending(false);
    if (!r.ok) setError(r.message);
    else AccessibilityInfo.announceForAccessibility("Command sent");
  };

  const stickyName = [sticky.ctrl ? "ctrl" : null, sticky.alt ? "alt" : null].filter(Boolean).join("+");

  return (
    <KeyCaptureView
      mode="shortcuts"
      // Tab completes for a program or the shell's line; with nothing to complete it moves focus
      captureTab={running !== null || draft !== ""}
      onKey={onShortcut}
      style={styles.wrap}
    >
      {running ? (
        <Text style={styles.running}>
          Running <Text style={styles.mono}>{running}</Text> · keys go to the program
        </Text>
      ) : null}
      <ControlRow
        ids={s.settings.controlKeys}
        sticky={sticky}
        disabled={!online}
        onSticky={setSticky}
        onKey={onRowKey}
      />
      {error ? <ErrorText style={styles.error}>{error}</ErrorText> : null}
      <View style={styles.inputRow}>
        <TextInput
          value={draft}
          onChangeText={onChange}
          onSubmitEditing={() => void run()}
          placeholder={stickyName ? `Type a letter for ${stickyName}+letter` : "Command"}
          placeholderTextColor={colors.textMuted}
          autoCapitalize="none"
          autoCorrect={false}
          spellCheck={false}
          returnKeyType="send"
          submitBehavior="submit"
          accessibilityLabel="Command"
          style={styles.input}
        />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Run command"
          accessibilityState={{ disabled: runDisabled, busy: sending }}
          disabled={runDisabled}
          onPress={() => void run()}
          style={({ pressed }) => [styles.run, runDisabled && styles.disabled, pressed && styles.pressed]}
        >
          <Text style={styles.runLabel}>{sending ? "…" : "Run"}</Text>
        </Pressable>
      </View>
    </KeyCaptureView>
  );
}

const styles = StyleSheet.create({
  wrap: {
    backgroundColor: colors.chrome,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.hairline,
    paddingHorizontal: 6,
    paddingTop: 6,
    paddingBottom: 8,
    gap: 6,
  },
  running: { color: colors.textMuted, fontSize: 12, paddingHorizontal: 4 },
  mono: { fontFamily: mono, color: colors.accent },
  pressed: { opacity: 0.7 },
  error: { paddingHorizontal: 4 },
  inputRow: { flexDirection: "row", gap: 6, alignItems: "center" },
  input: {
    flex: 1,
    minHeight: TOUCH,
    paddingHorizontal: 12,
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
    color: colors.textBright,
    fontFamily: mono,
    fontSize: 14,
  },
  run: {
    minWidth: TOUCH + 16,
    minHeight: TOUCH,
    borderRadius: radius.control,
    backgroundColor: colors.accent,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 14,
  },
  disabled: { opacity: 0.45 },
  runLabel: { color: colors.bg, fontSize: 15, fontWeight: "600" },
});
