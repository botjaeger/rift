import { useActionScope } from "@/ui/useActionScope";
import * as Clipboard from "expo-clipboard";
import { useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import {
  AccessibilityInfo,
  Keyboard,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import { clipboardFiles, KeyCaptureView, type KeyEvent, type PastedFile } from "../../modules/rift-keys";
import { commandProblem, utf8Length, type CompleteMsg } from "@/protocol/types";
import { draftKey, type ActionResult } from "@/fleet";
import { applySticky, keyEventToAction, splitText, stickyKey } from "@/keyboard";
import { keysNeedAuth } from "@/lock";
import { authenticate, fleet, useApp } from "@/runtime";
import { AT_PROMPT } from "@/upload";
import { cursorOutOfToken, insertedBy, pastesAsToken, spokenLabel, staleField } from "@/draftTokens";
import { isImageItem, nativeAttached, outHead, outParts, UPLOAD_GAP_MS, waitForImage, pending, PENDING_FULL, plainDraft, readClipboard, sendParts, type AttachRules, type Attached } from "@/pending";
import { chipRowMode, completionRows, NO_COMPLETIONS, NO_COMPLETIONS_PROGRAM, returnTakes } from "@/chips";
import { boxImages, commandNote, fieldKey, isOneLineRefusal, MAX_FIELD_PASTE_BYTES, messageLines, messagePlaceholder, partSend, programSend } from "@/paneView";
import type { ChipItem, ChipOpen } from "@/promptChips";
import { rowWithPaste } from "@/controlKeys";
import { showsKeyRow } from "@/terminal";
import { ChipRow } from "./ChipRow";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { usePending } from "./usePending";
import { insertedChar, plainPunctuation } from "./format";
import { Icon } from "./Icon";
import { ErrorText } from "./Notice";
import { useOpenShell } from "./useOpenShell";
import { mono, TOUCH, ui, makeStyles, useColors } from "./theme";

/** Suggestions shown under the command line's keys (the command line redesign board). */
const MAX_SUGGESTIONS = 6;
type Field = TextInput;
/** How long after a correction the field may still report a key made from its old text (a held Backspace). */
const STALE_FIELD_MS = 400;
const clock = () => Date.now();
/** How long a cursor the draft set holds against what the field reports. */
const PLACE_HOLD_MS = 150;
/** Wait this long after typing stops before asking the computer for completions. */
const COMPLETE_DELAY_MS = 200;

/**
 * The pane's command line (operate permission only; the command line redesign board), top to
 * bottom: the context above the field (the desktop's prompt chips, in its order, as many as fit and
 * "+N" for the rest; folder and branch are chips that open their menus, a timer is plain text);
 * one line: + (the Attach sheet: paste, photo, camera, file), ⌃, which shows the control keys (the
 * key block and arrow pad, ControlRow.tsx) under the field, so the field stays in sight above them
 * while the keyboard is up, and the round field with its run button; then the completions, closest to the keyboard (the desktop's, feature `complete`,
 * and this pane's history, typed part bright, the rest dim, with where each comes from), the first
 * marked: the phone keyboard's Return takes it at a prompt. Run runs; through the Confirm sheet only
 * with Settings › Ask before running on. While a program runs (`running`) the field says who it
 * types to; Send sends the
 * line as text, then Return as a key (feature `text`), with no Confirm sheet. The phone keyboard's Return is a new line; only Run or Send sends. A hardware
 * keyboard's Return sends and Shift+Return is a new line; its ctrl/alt combinations, Escape, Tab and F-keys in the field go to the pane as keys.
 * What Attach or Paste brings waits in the field: an image, a file or a long text is a token at the cursor
 * (draftTokens.ts), typed around, and Send sends the draft in order. While the host is not connected (`online` false) nothing can be sent; the draft stays editable.
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
  canUpload = false,
  canUploadFiles = false,
  canPaste = false,
  agent = false,
  compact = false,
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
  /** The computer takes pasted images (feature `upload`): Paste and Attach put the image in the draft. */
  canUpload?: boolean;
  /** The computer takes pasted files (feature `upload_files`). */
  canUploadFiles?: boolean;
  /** The computer takes several lines as one paste (feature `paste`). */
  canPaste?: boolean;
  /** The pane runs a coding agent: ctrl+j starts a new line there and submits nothing. */
  agent?: boolean;
  /** Little height to spare (landscape): the row above the field stays away while nothing is typed. */
  compact?: boolean;
  /** The computer is not connected (PaneLoading board): the field is off and says it waits. */
  waiting?: boolean;
}) {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const s = useApp();
  const { width: windowWidth } = useWindowDimensions();
  const scope = useActionScope(`${hostKey}:${paneId}`);
  const pk = draftKey(hostKey, paneId);
  const draft = s.drafts[pk] ?? "";
  const queued = usePending(pk);
  const [sticky, setSticky] = useState<Sticky>(NO_STICKY);
  const [error, setError] = useState<string | null>(null);
  // Return was pressed while not connected
  const [offlineTried, setOfflineTried] = useState(false);
  // why the last send stopped, from whichever screen sent it, until the next edit or send
  const shownError =
    error ?? (queued.failure || null) ?? (offlineTried && !online ? running !== null ? "Not connected. The message stays here; Send sends it once the computer is back." : "Not connected. The command stays here; Run runs it once the computer is back." : null);
  // a command on its way: a ref, not state, so a second Return before the re-render sends nothing
  const sending = useRef(false);
  // ⌨: the keys under the field; Settings › Keys on the command line opens them to start with
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
    if (!complete || !online || agent) return;
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
  }, [complete, online, agent, hostKey, paneId, draft, recentKey]);

  const sendKeys = async (k: string[]) => {
    const current = scope.capture();
    setError(null);
    // A key that submits the line runs a command like Run: it asks the same way (lock.ts).
    if (keysNeedAuth(k, s.settings, agent)) {
      const auth = await authenticate(`Send ${k.join(" ")}`);
      if (!current()) return;
      if (!auth.ok) {
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    const r = await fleet.sendKeys(hostKey, paneId, k);
    if (current() && !r.ok) setError(r.message);
  };

  // what the field held when a token's deletion last corrected it, until it shows the correction
  const corrected = useRef<{ typed: string; to: string; at: number; cursor?: number } | null>(null);
  const onChange = (typed: string) => {
    const text = plainPunctuation(typed);
    // at a prompt the phone keyboard's Return takes the first completion (marked): a command is one line
    const taken = running === null && !sticky.ctrl && !sticky.alt && !sticky.shift ? returnTakes(draft, text, rows[0]?.text) : null;
    if (taken !== null) {
      setError(null);
      fleet.setDraft(hostKey, paneId, taken);
      moveCursor(taken.length);
      return;
    }
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
    pending.takeFailure(pk);
    // the send in progress owns the draft, what went leaving it as it goes: only words added after it stay
    if (pending.view(pk).walking) {
      if (text.startsWith(liveDraft())) fleet.setDraft(hostKey, paneId, text);
      return;
    }
    // a key that came before the field took the last correction: it goes, the field takes the draft again
    const fixed = corrected.current;
    if (fixed && clock() - fixed.at < STALE_FIELD_MS && staleField(fixed.typed, fixed.to, text)) {
      // a letter made from the old text is still the user's: it goes in where the correction left the cursor
      const at = fixed.cursor ?? fixed.to.length;
      const late = text.length > fixed.typed.length ? insertedBy(fixed.typed, text).added : "";
      const to = fixed.to.slice(0, at) + late + fixed.to.slice(at);
      if (late !== "") {
        fleet.setDraft(hostKey, paneId, pending.edit(pk, to).text);
        corrected.current = { typed: text, to, at: fixed.at, cursor: at + late.length };
      }
      // and the cursor: the stale key moved it (typing went in before the space a token left)
      moveCursor(at + late.length);
      return;
    }
    // a long paste through the field itself (the phone's Paste menu, a hardware ⌘V) is a token like any other
    if (running !== null) {
      const now = liveDraft();
      const { from, to, added } = insertedBy(now, text);
      // iOS's smart insert pads a paste with a space at each end the clipboard never held
      const pasted = (Platform.OS === "ios" ? added.replace(/^ | $/g, "") : added).replace(/\r?\n$/, "");
      if (pastesAsToken(pasted, utf8Length(pasted), MAX_FIELD_PASTE_BYTES)) {
        const r = pending.edit(pk, now.slice(0, from) + now.slice(to));
        fleet.setDraft(hostKey, paneId, r.text);
        moveCursor(r.cursor ?? from);
        return pasteText(pasted);
      }
    }
    const r = pending.edit(pk, text);
    corrected.current = r.text !== text ? { typed: text, to: r.text, at: clock(), cursor: r.cursor } : null;
    fleet.setDraft(hostKey, paneId, r.text);
    if (r.cursor !== undefined) moveCursor(r.cursor);
  };

  const onOpen = useOpenShell(hostKey, paneId, setError);

  // Where the next pasted text or new line goes; `place` moves the field's cursor after one.
  const cursor = useRef<{ start: number; end: number } | null>(null);
  const [place, setPlace] = useState<{ start: number; end: number } | undefined>(undefined);
  // the field has had its moment to take `place`: from here the user's own cursor rules
  useEffect(() => {
    if (!place) return;
    const t = setTimeout(() => setPlace(undefined), PLACE_HOLD_MS);
    return () => clearTimeout(t);
  }, [place]);
  // the same as a ref: the field's report can come before the render that would show `place` to it
  const placing = useRef<{ start: number; end: number; until: number } | null>(null);
  const moveCursor = (at: number) => {
    cursor.current = { start: at, end: at };
    placing.current = { start: at, end: at, until: clock() + PLACE_HOLD_MS };
    setPlace({ start: at, end: at });
  };
  // the draft as it is now: a render's value lags a change made in the same tick
  const liveDraft = () => fleet.store.peek().drafts[pk] ?? "";
  const cursorIn = (text: string) => cursor.current ?? { start: text.length, end: text.length };
  const insert = (text: string) => {
    if (pending.view(pk).walking) return;
    // the draft may have changed under the cursor (sent, a completion taken)
    const now = liveDraft();
    const r = pending.insert(pk, now, cursorIn(now), text);
    fleet.setDraft(hostKey, paneId, r.text);
    moveCursor(r.cursor!);
  };

  const onRowKey = (id: string) => {
    // Shift+Return is a new line: ctrl+j reaches every agent, where Return with shift can submit
    const key = id === "shift+enter" ? "ctrl+j" : applySticky(id, sticky);
    setSticky(NO_STICKY);
    // only the send or run button sends: with text in the field Return goes on writing it, and
    // with an empty field it is the program's Return
    // (only new lines from the phone keyboard's Return count as nothing typed: ↵ still confirms a menu)
    const typed = draft.trim() !== "";
    if (typed && (key === "enter" || key === "shift+enter")) return insert("\n");
    if (typed && running !== null && key === "ctrl+j") return insert("\n");
    void sendKeys([key]);
  };

  // the row's Paste: also iOS's own paste button and the Attach sheet. Nothing pasted reaches the program
  // before Send. Text goes into the field; an image or file, or a long text, becomes a token at the cursor
  // (pending.ts), typed around. At a prompt there is no program: an image or file is refused, text is the command line's.
  const rules: AttachRules = { atPrompt: running === null, image: canUpload, file: canUploadFiles };
  const accept = (a: Attached) => {
    if (a.kind === "none") return;
    if (a.kind === "error") return setError(a.message);
    if (a.kind === "text") {
      if (!canType && running !== null) return;
      setError(null);
      return pending.giveText(pk, a.text);
    }
    setError(pending.add(pk, a.item) ? null : PENDING_FULL);
  };
  const onPaste = async () => {
    const current = scope.capture();
    const a = await readClipboard(Clipboard, clipboardFiles, rules);
    if (current()) accept(a);
  };
  const pasteText = (pasted: string) => {
    const text = pasted.replace(/\r?\n$/, "");
    if (running === null || !pastesAsToken(text, utf8Length(text), MAX_FIELD_PASTE_BYTES)) return insert(plainPunctuation(text));
    const body = text.replace(/\r\n?/g, "\n");
    const p = programSend(body, canType, canPaste);
    if (!p.ok) return setError(p.message);
    setError(pending.add(pk, { kind: "text", text: body }) ? null : PENDING_FULL);
  };
  // what arrived (here, from the Attach sheet or the screen's Paste) goes in at the cursor once the
  // field is on screen; the draft moved by something else takes its tokens with it
  useEffect(() => {
    // the send in progress: what arrives meanwhile waits for it to end
    if (pending.view(pk).walking) return;
    const now = liveDraft();
    if (pending.stale(pk, now)) {
      const r = pending.edit(pk, now);
      if (r.text !== now) fleet.setDraft(hostKey, paneId, r.text);
    }
    for (const item of pending.view(pk).items) {
      if (item.at >= 0) continue;
      const text = liveDraft();
      const label = plainPunctuation(item.label);
      const r = pending.place(pk, text, cursorIn(text), item.id, label);
      fleet.setDraft(hostKey, paneId, r.text);
      moveCursor(r.cursor!);
      AccessibilityInfo.announceForAccessibility(`${spokenLabel(label)} added to the message`);
    }
    if (pending.view(pk).text === "") return;
    const text = pending.takeText(pk);
    if (text) void Promise.resolve().then(() => pasteText(text));
  });
  const openAttach = () => {
    // the sheet opened over the keyboard would take the full height
    Keyboard.dismiss();
    router.push({ pathname: "/attach", params: { key: hostKey, pane: String(paneId), program: running !== null ? program : "", running: running !== null ? "1" : "0" } });
  };

  // hardware shortcuts taken from the command line (modules/rift-keys, shortcuts mode)
  const onShortcut = (e: KeyEvent) => {
    if (!online) return;
    const action = keyEventToAction(e);
    if (!action || !("keys" in action)) return;
    const name = action.keys[0]!;
    // a hardware keyboard's paste, on Android: the phone's clipboard, as the row's Paste (a program's Ctrl+V reads the computer's)
    if (name === "ctrl+v") return void onPaste();
    // reaching for undo while writing must not suspend the program
    if (name === "ctrl+z" && draft !== "") return;
    // a hardware Return sends (the phone keyboard's is a new line, in the field); Shift+Return writes one
    const k = fieldKey(name);
    if (k === "send") return void run();
    if (k === "newline") return insert("\n");
    onRowKey(name);
  };

  // Several lines: one paste where the program takes that, else line by line with New line between.
  const sendWhole = async (p: { paste?: string; lines?: string[] }): Promise<ActionResult> => {
    if (p.paste !== undefined) {
      const r = await fleet.sendPaste(hostKey, paneId, p.paste);
      if (r.ok || !isOneLineRefusal(r.message)) return r;
    }
    const lines = p.lines ?? messageLines(p.paste ?? "");
    for (let i = 0; i < lines.length; i++) {
      for (const text of lines[i] === "" ? [] : splitText(lines[i]!)) {
        const r = await fleet.sendText(hostKey, paneId, text);
        if (!r.ok) return r;
      }
      if (i < lines.length - 1) {
        const r = await fleet.sendKeys(hostKey, paneId, ["ctrl+j"]);
        if (!r.ok) return r;
      }
    }
    return { ok: true };
  };

  // A program is running: the line goes to it as text, then Return as its own request (a program
  // such as Claude Code reads text with a Return in the same write as a paste, not a send).
  const sendToProgram = async (authed = false) => {
    const current = scope.capture();
    const p = programSend(draft, canType, canPaste);
    if (!p.ok) {
      if (p.message) setError(p.message);
      return;
    }
    sending.current = true;
    setError(null);
    try {
      // the Return submits the line: it asks like Run does, before any text goes out
      if (!authed && keysNeedAuth(["enter"], s.settings)) {
        const auth = await authenticate("Send enter");
        if (!current()) return;
        if (!auth.ok) {
          if (auth.message !== "Cancelled.") setError(auth.message);
          return;
        }
      }
      const t = p.text !== undefined ? await fleet.sendText(hostKey, paneId, p.text) : await sendWhole(p);
      // Accepted text is spent even if this visit ended while its acknowledgement waited.
      // clearDraft only removes that exact draft, preserving anything typed since sending.
      if (t.ok) fleet.clearDraft(hostKey, paneId, draft);
      if (!current()) return;
      if (!t.ok) return setError(t.message);
      const k = await fleet.sendKeys(hostKey, paneId, ["enter"]);
      if (!current()) return;
      if (!k.ok) return setError(k.message);
      AccessibilityInfo.announceForAccessibility("Sent");
    } finally {
      sending.current = false;
    }
  };

  const problem = commandProblem(draft);
  const run = async () => {
    if (sending.current || pending.view(pk).walking) return;
    if (!online) {
      // Return did nothing at all before: say why, and that the line is kept (shown while offline)
      setOfflineTried(true);
      return;
    }
    setOfflineTried(false);
    if (running !== null) return void (queued.items.length > 0 ? sendWithItems() : sendToProgram());
    if (queued.items.some((i) => i.kind !== "text")) return setError(AT_PROMPT);
    if (queued.items.length > 0) {
      // the program ended with a long text waiting: at a prompt it is the command line's text
      const plain = plainDraft(draft, queued.items);
      pending.clearTokens(pk, plain);
      return fleet.setDraft(hostKey, paneId, plain);
    }
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
    const current = scope.capture();
    const r = await fleet.run(hostKey, paneId, draft);
    sending.current = false;
    if (!current()) return;
    if (!r.ok) setError(r.message);
    else AccessibilityInfo.announceForAccessibility("Command sent");
  };

  // Send with tokens in the draft: it goes in order, text as text, an image or file through the computer
  // (which pastes its path at the cursor), a long text as one paste, then Return as its own request (asked
  // once, before anything goes). What went leaves the box as it goes; a failure stops there, and the failed
  // part and all after it stay in the field, with a message naming it. Once begun it finishes without this
  // screen: leaving half way must not strand the message in the program's input with no Return.
  const sendWithItems = async () => {
    const current = scope.capture();
    const parts = outParts(draft, queued.items);
    const plans = new Map(parts.map((part) => [part, "text" in part ? partSend(part.text, canType, canPaste) : part.item.kind === "text" ? programSend(part.item.text, canType, canPaste) : null]));
    for (const plan of plans.values()) {
      if (plan && !plan.ok) {
        if (plan.message) setError(plan.message);
        return;
      }
    }
    sending.current = true;
    setError(null);
    try {
      if (keysNeedAuth(["enter"], s.settings)) {
        const auth = await authenticate("Send enter");
        if (!current()) return;
        if (!auth.ok) {
          if (auth.message !== "Cancelled.") setError(auth.message);
          return;
        }
      }
      if (!pending.beginWalk(pk)) return;
      let failure = "";
      // Claude Code's input box, for its images landing: only while this pane's screen is what the app follows
      const shown = () => {
        const p = fleet.store.peek().pane;
        return p?.host === hostKey && p.paneId === paneId && p.screen ? boxImages(p.screen.lines) : null;
      };
      let before: number | null = null;
      try {
        const r = await sendParts(
          parts,
          async (part) => {
            const plan = plans.get(part);
            if (plan && plan.ok) return plan.text !== undefined ? fleet.sendText(hostKey, paneId, plan.text) : sendWhole(plan);
            const item = "item" in part ? part.item : null;
            before = shown();
            return item && item.kind !== "text" ? fleet.uploadImage(hostKey, paneId, item.b64, item.kind === "file" ? item.ext : undefined) : { ok: true as const };
          },
          (part) => fleet.setDraft(hostKey, paneId, pending.spend(pk, liveDraft(), outHead(part))),
          (item) => (program === "claude" && isImageItem(item) ? waitForImage(before, shown) : new Promise((done) => setTimeout(done, UPLOAD_GAP_MS))),
        );
        if (!r.ok) failure = r.message;
        else {
          const k = await fleet.sendKeys(hostKey, paneId, ["enter"]);
          if (!k.ok) failure = k.message;
          else AccessibilityInfo.announceForAccessibility("Sent");
        }
      } finally {
        pending.endWalk(pk, failure);
      }
    } finally {
      sending.current = false;
    }
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
  const note = commandNote(draft, running !== null);
  const canRun = online && !waiting && draft.trim() !== "" && note === null;
  const canPasteHere = !waiting && online && (canType || canUpload || running === null);
  // the computer's answer for this draft (or the last one, while the next is on its way), then the pane's history
  const typing = chipRowMode(draft) === "completions" && !agent;
  const answer = complete && online ? completed : null;
  // (the React Compiler memoizes it; the field's Return reads it too)
  const rows = typing ? completionRows(draft, answer?.msg.items ?? [], answer?.msg.history ?? [], recentKey === "" ? [] : recentKey.split("\n"), MAX_SUGGESTIONS) : [];
  // "No completions" only once the computer has answered for this very draft
  const settled = !(complete && online) || completed?.draft === draft;
  const hasChips = chips.length > 0;
  // the field, for the keyboard key that opens the keyboard
  const input = useRef<Field>(null);
  const completions = typing ? (
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
        rows.map((r, i) => {
          // the first is marked: at a prompt the keyboard's Return takes it
          const first = i === 0 && running === null;
          return (
            <Pressable
              key={r.text}
              accessibilityRole="button"
              accessibilityLabel={`${r.text}, ${r.source}${first ? ", Return takes it" : ""}`}
              accessibilityHint="Puts it in the command line"
              onPress={() => fleet.setDraft(hostKey, paneId, r.text)}
              hitSlop={{ top: 6, bottom: 6 }}
              style={({ pressed }) => [styles.chip, first ? styles.firstSuggestion : styles.suggestion, pressed && styles.pressed]}
            >
              <Text style={styles.chipText} numberOfLines={1}>
                <Text style={styles.typed}>{r.typed}</Text>
                <Text style={first ? styles.typed : styles.rest}>{r.rest}</Text>
              </Text>
              <Text style={styles.source}>{r.source}</Text>
            </Pressable>
          );
        })
      )}
    </ScrollView>
  ) : null;

  return (
    <KeyCaptureView
      mode="shortcuts"
      // Tab completes for a program or the shell's line; with nothing to complete it moves focus
      captureTab={running !== null || draft !== ""}
      onKey={onShortcut}
      style={styles.wrap}
    >
      {hasChips && !compact ? <ChipRow items={chips} onOpen={(c) => c.opens && openMenu(c.opens)} onMore={openAll} /> : null}
      {shownError ? <ErrorText>{shownError}</ErrorText> : null}
      {note && !shownError ? <Text style={styles.none} accessibilityLiveRegion="polite">{note}</Text> : null}
      {queued.walking ? (
        <Text style={styles.none} accessibilityLiveRegion="polite">
          Sending to the computer…
        </Text>
      ) : null}
      <View style={[styles.inputRow, waiting && styles.waiting]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Attach"
          accessibilityHint="Paste, a photo, the camera or a file"
          accessibilityState={{ disabled: waiting }}
          disabled={waiting}
          onPress={openAttach}
          style={({ pressed }) => [styles.round, pressed && styles.pressed]}
        >
          <Icon name="add" size={20} color={colors.text} />
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Control keys"
          accessibilityHint="Shows the control keys under the command line"
          accessibilityState={{ expanded: keys, disabled: waiting }}
          disabled={waiting}
          onPress={() => setKeys((k) => !k)}
          style={({ pressed }) => [styles.round, keys && styles.roundOn, pressed && styles.pressed]}
        >
          <Icon name="control" size={18} weight="medium" color={keys ? colors.accent : colors.text} />
        </Pressable>
        <View style={styles.field}>
          <TextInput
            ref={input}
            keyboardAppearance={colors.dark ? "dark" : "light"}
            value={draft}
            onChangeText={onChange}
            multiline
            selection={place}
            onSelectionChange={(e) => {
              const sel = e.nativeEvent.selection;
              // a report from before the field took `place` (the key that was just corrected) says nothing
              const held = placing.current;
              if (held && clock() < held.until && (sel.start !== held.start || sel.end !== held.end)) return;
              if (place) setPlace(undefined);
              cursor.current = sel;
              // a cursor set down in a token rests after it: a token is one thing
              const out = sel.start === sel.end ? cursorOutOfToken(sel.start, pending.view(pk).items) : null;
              if (out !== null) moveCursor(out);
            }}
            editable={!waiting}
            placeholder={
              waiting
                ? "Waiting for your computer…"
                : stickyName
                  ? `Type a letter for ${stickyName}+letter`
                  : running !== null
                    ? messagePlaceholder(program, windowWidth)
                    : "Command"
            }
            placeholderTextColor={colors.textMuted}
            autoCapitalize="none"
            autoCorrect={false}
            spellCheck={false}
            // the phone keyboard's Return is a new line; only Send or Run sends (a hardware Return: onShortcut)
            submitBehavior="newline"
            accessibilityLabel={running !== null ? `Message for ${program} in ${label}` : `Command for ${label}`}
            accessibilityHint={
              running !== null ? "Send sends it to the program" : s.settings.askBeforeRun ? "Run shows the command first" : "Run runs it"
            }
            // an empty box is one line: iOS keeps the old height when a new line from a hardware
            // keyboard is cleared by JS (Shift+Return then Return)
            style={[styles.input, draft === "" && styles.inputEmpty]}
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
            <Icon name="run" size={18} color={colors.onAccent} />
          </Pressable>
        </View>
      </View>
      {keys ? (
        <ControlRow
          ids={rowWithPaste(s.settings.controlKeys, canUpload && running !== null)}
          sticky={sticky}
          disabled={!online}
          onSticky={setSticky}
          onKey={onRowKey}
          onOpen={onOpen}
          onPaste={canType || canUpload || running === null ? () => void onPaste() : undefined}
          systemPaste={
            canPasteHere
              ? {
                  onText: (text) => accept({ kind: "text", text }),
                  onImage: (image) => accept(nativeAttached({ image }, rules)),
                  onFile: (file: PastedFile) => accept(nativeAttached({ file }, rules)),
                }
              : undefined
          }
          onHideKeyboard={keyboardUp ? () => Keyboard.dismiss() : undefined}
          onShowKeyboard={keyboardUp || waiting ? undefined : () => input.current?.focus()}
        />
      ) : null}
      {completions}
    </KeyCaptureView>
  );
}

// The Pane board's command line: chrome ground, 8 / 12 pt padding, 8 pt between rows.
const useStyles = makeStyles((colors) => ({
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
  // the completions' strip reaches 6 pt past the 32 pt pills (a scroll view takes no touch outside it): a 44 pt touch
  chipStrip: { marginVertical: -6, flexGrow: 0 },
  chips: { gap: 6, paddingVertical: 6 },
  // a completion: a 32 pt pill, mono 13, outlined; the first marked in the accent (Return takes it)
  chip: { height: 32, paddingHorizontal: 12, borderRadius: 16, flexDirection: "row", alignItems: "center", gap: 6, maxWidth: 260 },
  suggestion: { borderWidth: 1, borderColor: colors.hairline, backgroundColor: colors.tints.chipFill },
  firstSuggestion: { borderWidth: 1, borderColor: `${colors.accent}55`, backgroundColor: colors.tints.accentBadge },
  chipText: { fontFamily: mono, color: colors.text, fontSize: 13, flexShrink: 1 },
  // a completion: what was typed bright, the rest dim, the source small after it
  typed: { color: colors.textBright },
  rest: { color: colors.textMuted },
  source: { fontFamily: ui(400), color: colors.textDim, fontSize: 10 },
  none: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, alignSelf: "center" },
  inputRow: { flexDirection: "row", gap: 8, alignItems: "flex-end" },
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
  // ⌃ while the keys show: the accent tint and rim
  roundOn: { borderColor: colors.accent, backgroundColor: colors.tints.accentBadge },
  field: {
    flex: 1,
    minWidth: 0,
    minHeight: TOUCH,
    flexDirection: "row",
    alignItems: "flex-end",
    paddingVertical: 3,
    paddingLeft: 16,
    paddingRight: 4,
    borderRadius: TOUCH / 2,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
  },
  // grows with the message, up to five lines, then scrolls
  input: { flex: 1, minWidth: 0, minHeight: 36, maxHeight: 108, paddingTop: 9, paddingBottom: 9, paddingHorizontal: 0, color: colors.textBright, fontFamily: mono, fontSize: 14, lineHeight: 18, textAlignVertical: "top" },
  inputEmpty: { height: 36 },
  // the board dims the run button to 40 % while the field is empty
  send: { width: 36, height: 36, borderRadius: 18, backgroundColor: `${colors.accent}66`, alignItems: "center", justifyContent: "center" },
  sendOn: { backgroundColor: colors.accent },
}));
