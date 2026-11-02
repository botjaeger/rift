import * as Clipboard from "expo-clipboard";
import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Platform, StyleSheet, Text, TextInput, View, type NativeSyntheticEvent, type TextInputSelectionChangeEventData, type TextStyle } from "react-native";
import { COPY_SETTLE_MS, selectedText, showsOwnCopied, textToCopy } from "@/selection";
import { colors, mono, ui } from "./theme";

/** How long our own "Copied" stays. */
const COPIED_MS = 1200;

/**
 * Block output you can select, and selecting copies it: a read-only field (`Text selectable`
 * only offers the system's Copy in a menu), whose selection is copied `COPY_SETTLE_MS` after the
 * handles stop. A tap puts a caret, which is an empty selection and copies nothing. The text read
 * is the field's own at that moment, so output that changed meanwhile copies what is shown.
 * `lines` limits the height (a running card's single line); the system menu still appears.
 */
export function SelectableText({ text, style, lineHeight, lines }: { text: string; style: TextStyle; lineHeight: number; lines?: number }) {
  const shown = useRef(text);
  useEffect(() => {
    shown.current = text;
  }, [text]);
  const copied = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const hide = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [notice, setNotice] = useState(false);
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      clearTimeout(hide.current);
    },
    [],
  );
  const onSelection = (e: NativeSyntheticEvent<TextInputSelectionChangeEventData>) => {
    const sel = e.nativeEvent.selection;
    clearTimeout(timer.current);
    if (sel.start === sel.end) {
      copied.current = null;
      return;
    }
    timer.current = setTimeout(() => {
      const t = textToCopy(selectedText(shown.current, sel), copied.current);
      if (t === null) return;
      copied.current = t;
      void Clipboard.setStringAsync(t).then(
        () => {
          AccessibilityInfo.announceForAccessibility("Copied");
          if (!showsOwnCopied(Platform.OS, Platform.Version)) return;
          setNotice(true);
          clearTimeout(hide.current);
          hide.current = setTimeout(() => setNotice(false), COPIED_MS);
        },
        () => AccessibilityInfo.announceForAccessibility("Could not copy"),
      );
    }, COPY_SETTLE_MS);
  };
  return (
    <View>
      <TextInput
        value={text}
        multiline
        // React Native's read-only on Android is a disabled view, which cannot select: there the field
        // stays enabled with no keyboard, and `value` undoes anything a hardware keyboard types
        readOnly={Platform.OS !== "android"}
        caretHidden={Platform.OS === "android"}
        scrollEnabled={false}
        showSoftInputOnFocus={false}
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
