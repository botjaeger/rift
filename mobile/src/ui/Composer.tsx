import { useRouter } from "expo-router";
import { useEffect, useMemo, useState } from "react";
import {
  AccessibilityInfo,
  ActionSheetIOS,
  Alert,
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
import { commandProblem } from "@/protocol/types";
import { draftKey } from "@/fleet";
import { applySticky, keyEventToAction, stickyKey } from "@/keyboard";
import { keysNeedAuth } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { branchText, chipsSpoken, type ChipMenuKind, type PromptChips } from "@/chips";
import { showsKeyRow } from "@/terminal";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { insertedChar, plainPunctuation } from "./format";
import { ErrorText } from "./Notice";
import { useOpenShell } from "./useOpenShell";
import { colors, mono, radius, TOUCH } from "./theme";

/** Suggestions shown above the command line (the P3 Pane board). */
const MAX_SUGGESTIONS = 6;
/** Wait this long after typing stops before asking the computer for completions. */
const COMPLETE_DELAY_MS = 200;

/** The folder button's two lines: the folder's name, the branch with its dirty dot. */
function folderLines(chips: PromptChips): { name: string; branch: string | null } | null {
  if (!chips.cwd) return null;
  const name = chips.cwd === "~" ? "~" : (chips.cwd.split("/").filter(Boolean).pop() ?? chips.cwd);
  const b = chips.branch;
  return { name, branch: b ? `${branchText(b)}${b.dirty ? " ●" : ""}` : null };
}

/**
 * The pane's command line (operate permission only; the P3 Pane board): suggestions (the
 * desktop's completions and this pane's history, feature `complete`; else its recent commands),
 * the folder button (the chip menus: folder, branch, changed files), the field, run with the
 * return key (through the Confirm sheet only with Settings › Ask before running on), and ⌃,
 * which shows the control-key row (Settings › Keys) in place of the suggestions. A hardware
 * keyboard's ctrl/alt combinations, Escape, Tab and F-keys in the field go to the pane as keys.
 * While the host is not connected (`online` false) nothing can be sent; the draft stays editable.
 */
export function Composer({
  hostKey,
  paneId,
  running,
  online,
  chips,
  chipMenus,
  complete,
  recent,
  label,
}: {
  hostKey: string;
  paneId: number;
  running: string | null;
  online: boolean;
  chips: PromptChips;
  /** The computer serves the chips' menus (feature `chips`): the folder button opens them. */
  chipMenus: boolean;
  /** The computer completes command lines (feature `complete`). */
  complete: boolean;
  /** The pane's recent commands, newest first (suggestions without `complete`). */
  recent: string[];
  /** The pane's title, for the field's label ("Command for rift · pane 1"). */
  label: string;
}) {
  const router = useRouter();
  const s = useApp();
  const draft = s.drafts[draftKey(hostKey, paneId)] ?? "";
  const [sticky, setSticky] = useState<Sticky>(NO_STICKY);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  // ⌃: the key row in place of the suggestions; Settings › Show keys opens it to start with
  const [keys, setKeys] = useState(() => showsKeyRow(s.settings.showKeys, false));
  const [completed, setCompleted] = useState<{ draft: string; list: string[] } | null>(null);

  // the desktop's completions for the draft, then the pane's history; else its recent commands
  const recentKey = recent.join("\n");
  const fromRecent = useMemo(
    () => (recentKey === "" ? [] : recentKey.split("\n").filter((c) => c.startsWith(draft) && c !== draft)),
    [recentKey, draft],
  );
  useEffect(() => {
    if (!complete || !online) return;
    let live = true;
    const t = setTimeout(() => {
      void fleet.complete(hostKey, paneId, draft).then((r) => {
        if (!live || !r.ok) return;
        const out: string[] = [];
        for (const x of [...r.msg.items.map((i) => i.text), ...r.msg.history.map((h) => h.text)])
          if (x !== draft && !out.includes(x)) out.push(x);
        setCompleted({ draft, list: out });
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

  const onOpen = useOpenShell(hostKey, paneId, setError);

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
  const run = async () => {
    if (!online || sending) return;
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
    setSending(true);
    setError(null);
    const r = await fleet.run(hostKey, paneId, draft);
    setSending(false);
    if (!r.ok) setError(r.message);
    else AccessibilityInfo.announceForAccessibility("Command sent");
  };

  const openMenu = (menu: ChipMenuKind) => {
    // the sheet opened over the keyboard would take the full height
    Keyboard.dismiss();
    router.push({ pathname: "/chips", params: { key: hostKey, pane: String(paneId), menu } });
  };
  // the folder button: the folder's menu, or a choice of the chips' menus in a repository
  const pickMenu = () => {
    const menus: { label: string; menu: ChipMenuKind }[] = [{ label: "Change folder", menu: "dirs" }];
    if (chips.branch) menus.push({ label: "Switch branch", menu: "branches" });
    if (chips.diff) menus.push({ label: `Changed files (${chips.diff.files})`, menu: "changes" });
    if (menus.length === 1) return openMenu("dirs");
    if (Platform.OS === "ios")
      ActionSheetIOS.showActionSheetWithOptions(
        { options: [...menus.map((m) => m.label), "Cancel"], cancelButtonIndex: menus.length, userInterfaceStyle: "dark" },
        (i) => i < menus.length && openMenu(menus[i]!.menu),
      );
    else Alert.alert(chipsSpoken(chips), undefined, menus.map((m) => ({ text: m.label, onPress: () => openMenu(m.menu) })), { cancelable: true });
  };

  const folder = folderLines(chips);
  const stickyName = [sticky.ctrl ? "ctrl" : null, sticky.alt ? "alt" : null].filter(Boolean).join("+");
  // the computer's answer for this draft (or the last one, while the next is on its way)
  const suggested = complete && online ? (completed?.list ?? []) : fromRecent;
  const shown = suggested.slice(0, MAX_SUGGESTIONS);

  return (
    <KeyCaptureView
      mode="shortcuts"
      // Tab completes for a program or the shell's line; with nothing to complete it moves focus
      captureTab={running !== null || draft !== ""}
      onKey={onShortcut}
      style={styles.wrap}
    >
      {keys ? (
        <ControlRow
          ids={s.settings.controlKeys}
          sticky={sticky}
          disabled={!online}
          onSticky={setSticky}
          onKey={onRowKey}
          onOpen={onOpen}
          onEdit={() => router.push("/control-keys")}
        />
      ) : shown.length > 0 ? (
        <View accessibilityRole="toolbar" accessibilityLabel="Suggestions">
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            style={styles.suggestionStrip}
            contentContainerStyle={styles.suggestions}
            keyboardShouldPersistTaps="always"
          >
            {shown.map((c) => (
              <Pressable
                key={c}
                accessibilityRole="button"
                accessibilityLabel={c}
                accessibilityHint="Puts it in the command line"
                onPress={() => fleet.setDraft(hostKey, paneId, c)}
                hitSlop={{ top: 6, bottom: 6 }}
                style={({ pressed }) => [styles.suggestion, pressed && styles.pressed]}
              >
                <Text style={styles.suggestionText} numberOfLines={1}>
                  {c}
                </Text>
              </Pressable>
            ))}
          </ScrollView>
        </View>
      ) : null}
      {error ? <ErrorText>{error}</ErrorText> : null}
      <View style={styles.inputRow}>
        {folder ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Folder ${chips.cwd}${chips.branch ? ` on ${chips.branch.name}` : ""}${chipMenus ? ", change" : ""}`}
            accessibilityHint={chipMenus ? (chips.branch ? "Folder, branch or changed files" : "Opens the folder's sub-folders") : undefined}
            disabled={!chipMenus}
            onPress={pickMenu}
            style={({ pressed }) => [styles.folder, pressed && styles.pressed]}
          >
            <Text style={styles.folderName} numberOfLines={1}>
              {folder.name}
            </Text>
            {folder.branch ? (
              <Text style={styles.folderBranch} numberOfLines={1}>
                {folder.branch}
              </Text>
            ) : null}
          </Pressable>
        ) : null}
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
          accessibilityLabel={`Command for ${label}`}
          accessibilityHint={s.settings.askBeforeRun ? "Return shows the command first" : "Return runs it"}
          style={styles.input}
        />
        <Pressable
          accessibilityRole="togglebutton"
          accessibilityLabel="Keys"
          accessibilityHint="Shows the control keys above the command line"
          accessibilityState={{ checked: keys }}
          onPress={() => setKeys((k) => !k)}
          style={({ pressed }) => [styles.keysButton, keys && styles.keysOn, pressed && styles.pressed]}
        >
          <Text style={[styles.keysLabel, keys && { color: colors.accent }]}>⌃</Text>
        </Pressable>
      </View>
    </KeyCaptureView>
  );
}

// The P3 Pane board's command line: chrome ground, 8 / 12 pt padding, 8 pt between rows.
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
  // the strip reaches 6 pt past the 32 pt chips (a scroll view takes no touch outside it): a 44 pt touch
  suggestionStrip: { marginVertical: -6 },
  suggestions: { gap: 6, paddingVertical: 6 },
  suggestion: {
    height: 32,
    paddingHorizontal: 10,
    borderRadius: 16,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
    justifyContent: "center",
    maxWidth: 260,
  },
  suggestionText: { fontFamily: mono, color: colors.text, fontSize: 12 },
  inputRow: { flexDirection: "row", gap: 8, alignItems: "center" },
  folder: {
    height: TOUCH,
    maxWidth: 120,
    paddingHorizontal: 10,
    borderRadius: radius.button,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
    justifyContent: "center",
  },
  folderName: { fontFamily: mono, color: colors.text, fontSize: 12 },
  folderBranch: { fontFamily: mono, color: colors.success, fontSize: 11 },
  input: {
    flex: 1,
    minWidth: 0,
    height: TOUCH,
    paddingHorizontal: 12,
    borderRadius: radius.button,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontFamily: mono,
    fontSize: 14,
  },
  keysButton: {
    width: TOUCH,
    height: TOUCH,
    borderRadius: radius.button,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
    alignItems: "center",
    justifyContent: "center",
  },
  keysOn: { borderColor: colors.accent },
  keysLabel: { fontFamily: mono, color: colors.textMuted, fontSize: 13 },
});
