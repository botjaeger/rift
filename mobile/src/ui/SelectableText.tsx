import * as Clipboard from "expo-clipboard";
import { useEffect, useId, useRef, useState } from "react";
import { AccessibilityInfo, Platform, StyleSheet, Text, TextInput, View, type TextInputSelectionChangeEvent, type TextStyle } from "react-native";
import * as haptics from "@/haptics";
import { COPY_SETTLE_MS, lineRange, selectedText, showsOwnCopied, textToCopy } from "@/selection";
import { useSelectionOwner, useSelectionScope } from "./SelectionScope";
import { colors, mono, ui } from "./theme";

/** How long our own "Copied" stays. */
const COPIED_MS = 1200;

type Range = { start: number; end: number };
// the field's handle (named: test/fonts.test.ts reads every `<TextInput` as an element)
type Field = TextInput;

/**
 * Block output you can select, and selecting copies it: a read-only field (`Text selectable`
 * only offers the system's Copy in a menu), whose selection is copied `COPY_SETTLE_MS` after the
 * handles stop, with a small vibration when it lands. A tap puts a caret, which is an empty
 * selection and copies nothing. The text read is the field's own at that moment, so output that
 * changed meanwhile copies what is shown. `lines` limits the height (a running card's single line).
 *
 * Inside a SelectionScope (the pane's output) the scope's bar says how much is selected and
 * offers Line, All, To prompt and ✕, and the system's edit menu stays hidden (it opened over the
 * bar and offered Copy again); elsewhere (a sheet) the field shows its own "Copied" and the menu.
 */
export function SelectableText({ text, style, lineHeight, lines }: { text: string; style: TextStyle; lineHeight: number; lines?: number }) {
  const shown = useRef(text);
  useEffect(() => {
    shown.current = text;
  }, [text]);
  const scope = useSelectionScope();
  const ownId = useId();
  const owner = useSelectionOwner() ?? ownId;
  const field = useRef<Field>(null);
  const wrap = useRef<View>(null);
  const copied = useRef<string | null>(null);
  const range = useRef<Range>({ start: 0, end: 0 });
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const hide = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [notice, setNotice] = useState(false);
  // a selection the bar set (Line, All); the field follows the finger otherwise
  const [forced, setForced] = useState<Range | undefined>(undefined);
  // the latest scope and owner for the timers and the bar's callbacks
  const live = useRef({ scope, owner });
  useEffect(() => {
    live.current = { scope, owner };
  });
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      clearTimeout(hide.current);
      live.current.scope?.clear(live.current.owner);
    },
    [],
  );
  // what the bar last said: a relocation after a scroll keeps it
  const lastCopied = useRef(false);
  const report = (sel: Range, isCopied: boolean) => {
    const { scope: sc, owner: own } = live.current;
    if (!sc) return;
    lastCopied.current = isCopied;
    wrap.current?.measureInWindow((_x, y, _w, h) => {
      sc.show({
        owner: own,
        selected: selectedText(shown.current, sel),
        copied: isCopied,
        top: y,
        bottom: y + h,
        line: () => select(lineRange(shown.current, range.current)),
        all: () => select({ start: 0, end: shown.current.length }),
        end,
        relocate: () => report(range.current, lastCopied.current),
      });
    });
  };
  const copy = (sel: Range) => {
    const t = textToCopy(selectedText(shown.current, sel), copied.current);
    if (t === null) {
      report(sel, copied.current !== null && copied.current === selectedText(shown.current, sel));
      return;
    }
    copied.current = t;
    void Clipboard.setStringAsync(t).then(
      () => {
        haptics.copied();
        AccessibilityInfo.announceForAccessibility("Copied");
        report(sel, true);
        if (live.current.scope || !showsOwnCopied(Platform.OS, Platform.Version)) return;
        setNotice(true);
        clearTimeout(hide.current);
        hide.current = setTimeout(() => setNotice(false), COPIED_MS);
      },
      () => {
        AccessibilityInfo.announceForAccessibility("Could not copy");
        report(sel, false);
      },
    );
  };
  const select = (sel: Range) => {
    clearTimeout(timer.current);
    range.current = sel;
    setForced(sel);
    copy(sel);
  };
  const end = () => {
    clearTimeout(timer.current);
    copied.current = null;
    range.current = { start: 0, end: 0 };
    setForced({ start: 0, end: 0 });
    field.current?.blur();
    live.current.scope?.clear(live.current.owner);
  };
  const onSelection = (e: TextInputSelectionChangeEvent) => {
    const sel = e.nativeEvent.selection;
    range.current = sel;
    clearTimeout(timer.current);
    if (forced && (forced.start !== sel.start || forced.end !== sel.end)) setForced(undefined);
    if (sel.start === sel.end) {
      copied.current = null;
      live.current.scope?.clear(live.current.owner);
      return;
    }
    timer.current = setTimeout(() => copy(sel), COPY_SETTLE_MS);
  };
  return (
    <View ref={wrap} collapsable={false}>
      <TextInput
        ref={field}
        value={text}
        multiline
        // React Native's read-only on Android is a disabled view, which cannot select: there the field
        // stays enabled with no keyboard, and `value` undoes anything a hardware keyboard types
        readOnly={Platform.OS !== "android"}
        caretHidden={Platform.OS === "android"}
        scrollEnabled={false}
        showSoftInputOnFocus={false}
        selection={forced}
        selectionColor={colors.accent}
        // iOS only: Android's selection lives in the edit menu's mode, and hiding the menu ends it at once
        contextMenuHidden={scope !== null && Platform.OS === "ios"}
        onSelectionChange={onSelection}
        style={[styles.field, style, { lineHeight }, lines ? { maxHeight: lineHeight * lines } : null]}
      />
      {notice ? (
        <View pointerEvents="none" style={styles.notice}>
          <Text style={styles.noticeText}>Copied</Text>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  // a field draws its own padding: a block's text sits flush like Text
  // the caller's style names the family; mono is the fallback
  field: { padding: 0, margin: 0, textAlignVertical: "top", fontFamily: mono },
  notice: { position: "absolute", top: 0, right: 0, paddingHorizontal: 8, paddingVertical: 2, borderRadius: 10, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.glassRim },
  noticeText: { fontFamily: ui(500), color: colors.textBright, fontSize: 12 },
});
