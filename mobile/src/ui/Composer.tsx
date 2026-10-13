import { useRouter } from "expo-router";
import { useState } from "react";
import { AccessibilityInfo, Keyboard, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { commandProblem } from "@/protocol/types";
import { draftKey } from "@/fleet";
import { keysNeedAuth } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { insertedLetter } from "./format";
import { ErrorText } from "./Notice";
import { colors, mono, radius, TOUCH } from "./theme";

interface KeyDef {
  label: string;
  key: string;
  spoken: string;
  danger?: boolean;
}

const KEYS_BEFORE: KeyDef[] = [
  { label: "^C", key: "ctrl+c", spoken: "Control C, interrupt", danger: true },
  { label: "esc", key: "esc", spoken: "Escape" },
  { label: "tab", key: "tab", spoken: "Tab" },
];
const KEYS_AFTER: KeyDef[] = [
  { label: "↑", key: "up", spoken: "Up arrow" },
  { label: "↓", key: "down", spoken: "Down arrow" },
  { label: "←", key: "left", spoken: "Left arrow" },
  { label: "→", key: "right", spoken: "Right arrow" },
];

function Key({
  def,
  onPress,
  active,
  disabled,
}: {
  def: KeyDef;
  onPress: () => void;
  active?: boolean;
  disabled: boolean;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={def.spoken}
      accessibilityState={{ disabled, ...(active === undefined ? {} : { selected: active }) }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.key,
        def.danger && styles.keyDanger,
        active && styles.keyActive,
        disabled && styles.disabled,
        pressed && styles.pressed,
      ]}
    >
      <Text style={[styles.keyLabel, active && { color: colors.bg }]}>{def.label}</Text>
    </Pressable>
  );
}

/**
 * The pane's command composer (operate permission only): a row of control keys sent with
 * `keys`, and a command line run with `run` (through the Confirm sheet unless that is off).
 * While the host is not connected (`online` false) nothing can be sent; the draft stays editable.
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
  const [ctrl, setCtrl] = useState(false);
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
    if (ctrl) {
      setCtrl(false);
      const letter = insertedLetter(draft, text);
      if (letter) {
        void sendKeys([`ctrl+${letter}`]);
        return; // the letter is a key, not text
      }
    }
    setError(null);
    fleet.setDraft(hostKey, paneId, text);
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

  return (
    <View style={styles.wrap}>
      {running ? (
        <Text style={styles.running}>
          Running <Text style={styles.mono}>{running}</Text> · keys go to the program
        </Text>
      ) : null}
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={styles.keys}
        keyboardShouldPersistTaps="always"
      >
        {KEYS_BEFORE.map((k) => (
          <Key key={k.key} def={k} disabled={!online} onPress={() => void sendKeys([k.key])} />
        ))}
        <Key
          disabled={!online}
          def={{ label: "ctrl", key: "ctrl", spoken: "Control, applies to the next letter you type" }}
          active={ctrl}
          onPress={() => setCtrl((c) => !c)}
        />
        {KEYS_AFTER.map((k) => (
          <Key key={k.key} def={k} disabled={!online} onPress={() => void sendKeys([k.key])} />
        ))}
      </ScrollView>
      {error ? <ErrorText style={styles.error}>{error}</ErrorText> : null}
      <View style={styles.inputRow}>
        <TextInput
          value={draft}
          onChangeText={onChange}
          onSubmitEditing={() => void run()}
          placeholder={ctrl ? "Type a letter for ctrl+letter" : "Command"}
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
    </View>
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
  // The eight keys share the row (≥ 44 pt each) and fit a 390 pt phone;
  // narrower screens scroll the row.
  keys: { gap: 3, flexGrow: 1 },
  key: {
    flexGrow: 1,
    minWidth: TOUCH,
    minHeight: TOUCH,
    paddingHorizontal: 4,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.control,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  // Interrupt is marked by a red border: red text on a surface is below 4.5:1 (palette.ts).
  keyDanger: { borderColor: colors.error },
  keyActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  keyLabel: { color: colors.text, fontFamily: mono, fontSize: 14 },
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
