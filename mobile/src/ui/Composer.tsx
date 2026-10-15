import * as Clipboard from "expo-clipboard";
import { useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  AccessibilityInfo,
  ActionSheetIOS,
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
import { Icon } from "./Icon";
import { ErrorText } from "./Notice";
import { useOpenShell } from "./useOpenShell";
import { colors, mono, TOUCH } from "./theme";
import { toggleA11y } from "./toggle";

/** Suggestions shown above the command line (the P3 Pane board). */
const MAX_SUGGESTIONS = 6;
/** Wait this long after typing stops before asking the computer for completions. */
const COMPLETE_DELAY_MS = 200;

/** The folder line above the keys (the Pane board): the folder's path, the branch with its dirty dot. */
function folderLines(chips: PromptChips): { path: string; branch: string | null } | null {
  if (!chips.cwd) return null;
  const b = chips.branch;
  return { path: chips.cwd, branch: b ? `${branchText(b)}${b.dirty ? " ●" : ""}` : null };
}

/**
 * The pane's command line (operate permission only; the P3 Pane board): suggestions (the
 * desktop's completions and this pane's history, feature `complete`; else its recent commands),
 * the folder line (the chip menus: folder, branch, changed files), ⌃, which shows the
 * control-key row (Settings › Keys) in place of the suggestions, and the round field with its
 * run button (Return runs too; through the Confirm sheet only with Settings › Ask before
 * running on). A hardware
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
  canType,
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
  /** The computer takes typed text (feature `text`): the row's Paste works. */
  canType: boolean;
}) {
  const router = useRouter();
  const s = useApp();
  const draft = s.drafts[draftKey(hostKey, paneId)] ?? "";
  const [sticky, setSticky] = useState<Sticky>(NO_STICKY);
  const [error, setError] = useState<string | null>(null);
  // Return was pressed while not connected
  const [offlineTried, setOfflineTried] = useState(false);
  const shownError =
    error ?? (offlineTried && !online ? "Not connected. The command stays here; Return runs it once the computer is back." : null);
  // a command on its way: a ref, not state, so a second Return before the re-render sends nothing
  const sending = useRef(false);
  // ⌃: the key row in place of the suggestions; Settings › Show keys opens it to start with
  const [keys, setKeys] = useState(() => showsKeyRow(s.settings.showKeys, false));
  const [completed, setCompleted] = useState<{ draft: string; list: string[] } | null>(null);
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

  const problem = commandProblem(draft);
  const run = async () => {
    if (sending.current) return;
    if (!online) {
      // Return did nothing at all before: say why, and that the line is kept (shown while offline)
      setOfflineTried(true);
      return;
    }
    setOfflineTried(false);
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

  const openMenu = (menu: ChipMenuKind) => {
    // the sheet opened over the keyboard would take the full height
    Keyboard.dismiss();
    router.push({ pathname: "/chips", params: { key: hostKey, pane: String(paneId), menu } });
  };
  // the folder button: the folder's menu, or a choice of the chips' menus in a repository
  const menus: { label: string; menu: ChipMenuKind }[] = [{ label: "Change folder", menu: "dirs" }];
  if (chips.branch) menus.push({ label: "Switch branch", menu: "branches" });
  if (chips.diff) menus.push({ label: `Changed files (${chips.diff.files})`, menu: "changes" });
  // iOS: the system action sheet. Elsewhere a strip above the field: react-native-web's Alert
  // shows nothing, and Android's put the three menus in reverse order (its positive / negative /
  // neutral buttons) on a light dialog under the cut-off folder path.
  const [choosing, setChoosing] = useState(false);
  const pickMenu = () => {
    if (menus.length === 1) return openMenu("dirs");
    if (Platform.OS === "ios")
      ActionSheetIOS.showActionSheetWithOptions(
        { options: [...menus.map((m) => m.label), "Cancel"], cancelButtonIndex: menus.length, userInterfaceStyle: "dark" },
        (i) => i < menus.length && openMenu(menus[i]!.menu),
      );
    else setChoosing((c) => !c);
  };

  const folder = folderLines(chips);
  const stickyName = [sticky.ctrl ? "ctrl" : null, sticky.alt ? "alt" : null, sticky.shift ? "shift" : null].filter(Boolean).join("+");
  const canRun = online && draft.trim() !== "";
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
      {folder ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Folder ${chips.cwd}${chips.branch ? ` on ${chips.branch.name}` : ""}${chipMenus ? ", change" : ""}`}
          accessibilityHint={chipMenus ? (chips.branch ? "Folder, branch or changed files" : "Opens the folder's sub-folders") : undefined}
          accessibilityState={{ disabled: !chipMenus, ...(Platform.OS !== "ios" && menus.length > 1 ? { expanded: choosing } : {}) }}
          disabled={!chipMenus}
          onPress={pickMenu}
          hitSlop={{ top: 12, bottom: 6 }}
          style={({ pressed }) => [styles.folder, pressed && styles.pressed]}
        >
          <Icon name="folder" size={14} color={colors.textMuted} />
          <Text style={styles.folderPath} numberOfLines={1} ellipsizeMode="head">
            {folder.path}
          </Text>
          {folder.branch ? (
            <Text style={styles.folderBranch} numberOfLines={1}>
              {folder.branch}
            </Text>
          ) : null}
        </Pressable>
      ) : null}
      {choosing && menus.length > 1 ? (
        // keyed, as the suggestions below: without keys React reused one native strip for both,
        // and the suggestions opened at the menu strip's sideways scroll, their first one cut off
        <View key="menus" accessibilityRole="menu" accessibilityLabel={chipsSpoken(chips)}>
          <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.suggestionStrip} contentContainerStyle={styles.suggestions}>
            {menus.map((m) => (
              <Pressable
                key={m.menu}
                accessibilityRole="menuitem"
                accessibilityLabel={m.label}
                onPress={() => {
                  setChoosing(false);
                  openMenu(m.menu);
                }}
                hitSlop={{ top: 6, bottom: 6 }}
                style={({ pressed }) => [styles.suggestion, pressed && styles.pressed]}
              >
                <Text style={styles.suggestionText} numberOfLines={1}>
                  {m.label}
                </Text>
              </Pressable>
            ))}
          </ScrollView>
        </View>
      ) : keys ? (
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
      ) : shown.length > 0 ? (
        // a new first suggestion (a command just ran, a letter typed) starts the strip at its left end
        <View key={`suggestions:${shown[0]}`} accessibilityRole="toolbar" accessibilityLabel="Suggestions">
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
      {shownError ? <ErrorText>{shownError}</ErrorText> : null}
      <View style={styles.inputRow}>
        <Pressable
          {...toggleA11y(keys)}
          accessibilityLabel="Keys"
          accessibilityHint="Shows the control keys above the command line"
          onPress={() => setKeys((k) => !k)}
          hitSlop={2}
          style={({ pressed }) => [styles.round, keys && styles.roundOn, pressed && styles.pressed]}
        >
          <Text style={[styles.keysLabel, keys && { color: colors.accent }]}>⌃</Text>
        </Pressable>
        <View style={styles.field}>
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
            accessibilityRole="button"
            accessibilityLabel="Run"
            accessibilityHint={s.settings.askBeforeRun ? "Shows the command first" : undefined}
            accessibilityState={{ disabled: !canRun }}
            disabled={!canRun}
            onPress={() => void run()}
            hitSlop={5}
            style={({ pressed }) => [styles.send, canRun && styles.sendOn, pressed && styles.pressed]}
          >
            <Icon name="run" size={16} color={canRun ? colors.bg : colors.textMuted} />
          </Pressable>
        </View>
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
  // the folder line: 12 pt, its path cut at the start (the folder's own name is at the end)
  folder: { alignSelf: "flex-start", maxWidth: "100%", flexDirection: "row", alignItems: "center", gap: 6, paddingHorizontal: 4 },
  folderPath: { fontFamily: mono, color: colors.text, fontSize: 12, flexShrink: 1 },
  folderBranch: { fontFamily: mono, color: colors.success, fontSize: 12, flexShrink: 0, maxWidth: "45%" },
  // the board's 44 pt round buttons and the round field with its 34 pt run button
  round: {
    width: TOUCH,
    height: TOUCH,
    borderRadius: TOUCH / 2,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
    alignItems: "center",
    justifyContent: "center",
  },
  roundOn: { borderColor: colors.accent },
  keysLabel: { fontFamily: mono, color: colors.textMuted, fontSize: 15 },
  field: {
    flex: 1,
    minWidth: 0,
    height: TOUCH,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 14,
    paddingRight: 4,
    borderRadius: TOUCH / 2,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
  },
  input: { flex: 1, minWidth: 0, height: TOUCH - 2, padding: 0, color: colors.textBright, fontFamily: mono, fontSize: 14 },
  send: { width: 34, height: 34, borderRadius: 17, backgroundColor: colors.glassRim, alignItems: "center", justifyContent: "center" },
  sendOn: { backgroundColor: colors.accent },
});
