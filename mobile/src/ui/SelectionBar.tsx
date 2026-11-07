import { useEffect } from "react";
import { BackHandler, Pressable, Text, View } from "react-native";
import { countLabel, selectionCount } from "@/selection";
import { Icon } from "./Icon";
import { glyph } from "./Screen";
import { ui, makeStyles, useColors } from "./theme";

/**
 * The bar a selection shows (the "Redesign: selection" board), one for block output and a program's
 * screen: Word, Line, All, To prompt and ✕, always in the same places. All is the whole block or the
 * whole screen (`of`). It is docked over the pane's controls (the message box, the key row), which
 * stay in place under it, hidden, so it never covers the selected text, a handle or the cursor, and
 * nothing moves when it comes.
 *
 * Its first line says how much is selected and, once that is on the clipboard, "✓ Copied" (kept in
 * view while the selection stays copied; the caller announces each copy once, so this line is not
 * a live region), else `note`. To prompt is disabled, with `promptOff` as its screen reader hint, when the
 * selection cannot go into a command line here. Android's Back ends the selection, not the pane.
 */
export function SelectionBar({
  selected,
  copied,
  of,
  note,
  onWord,
  onLine,
  onAll,
  onPrompt,
  promptOff,
  onCancel,
}: {
  selected: string;
  copied: boolean;
  /** What All selects: the block's text or the program's screen. */
  of: "block" | "screen";
  note?: string;
  onWord: () => void;
  onLine: () => void;
  onAll: () => void;
  /** Puts the selection in the command line; omitted, To prompt is disabled and `promptOff` says why. */
  onPrompt?: () => void;
  promptOff?: string;
  onCancel: () => void;
}) {
  const colors = useColors();
  const styles = useStyles();
  useEffect(() => {
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      onCancel();
      return true;
    });
    return () => sub.remove();
  }, [onCancel]);
  const key = (label: string, hint: string, onPress: (() => void) | undefined, prompt = false) => (
    <Pressable
      key={label}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={onPress ? hint : promptOff}
      accessibilityState={{ disabled: !onPress }}
      disabled={!onPress}
      onPress={onPress}
      hitSlop={4}
      style={({ pressed }) => [styles.key, prompt && styles.promptKey, !onPress && styles.disabled, pressed && styles.pressed]}
    >
      <Text style={[styles.keyText, prompt && styles.promptText]} numberOfLines={1} adjustsFontSizeToFit minimumFontScale={0.75}>
        {label}
      </Text>
    </Pressable>
  );
  return (
    <View style={styles.bar} accessibilityRole="toolbar" accessibilityLabel="Selection">
      <View style={styles.info}>
        <Text style={styles.count}>{countLabel(selectionCount(selected))}</Text>
        {copied ? (
          <Text style={styles.copied}>✓ Copied</Text>
        ) : note ? (
          <Text style={styles.note}>{note}</Text>
        ) : null}
      </View>
      <View style={styles.keys}>
        {key("Word", "Select whole words", onWord)}
        {key("Line", "Select whole lines", onLine)}
        {key("All", of === "block" ? "Select all of this block" : "Select the whole screen", onAll)}
        {key("To prompt", "Put the selection in the command line", onPrompt, true)}
        <Pressable accessibilityRole="button" accessibilityLabel="End the selection" onPress={onCancel} hitSlop={4} style={({ pressed }) => [styles.key, styles.close, pressed && styles.pressed]}>
          {/* the board's 16 pt Material ✕; iOS's symbol draws larger at a size */}
          <Icon name="close" size={glyph(16, 11)} color={colors.text} />
        </Pressable>
      </View>
      <Text style={styles.foot}>{`All selects the whole ${of}.`}</Text>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  // in the flow, at the bottom: the controls' place
  bar: { marginHorizontal: 10, marginTop: 6, marginBottom: 8, gap: 8, paddingHorizontal: 10, paddingTop: 10, paddingBottom: 8, borderRadius: 16, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.borderStrong },
  info: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 8, paddingHorizontal: 4 },
  count: { fontFamily: ui(500), color: colors.text, fontSize: 13, lineHeight: 18 },
  copied: { fontFamily: ui(500), color: colors.success, fontSize: 13, lineHeight: 18 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  keys: { flexDirection: "row", gap: 6 },
  key: { flex: 1, height: 40, borderRadius: 10, alignItems: "center", justifyContent: "center", backgroundColor: colors.tints.chipFill, borderWidth: 1, borderColor: colors.hairline },
  promptKey: { flex: 1.4, backgroundColor: colors.tints.accentBadge, borderColor: `${colors.accent}55` },
  close: { flex: 0, width: 40, backgroundColor: "transparent" },
  keyText: { fontFamily: ui(500), color: colors.text, fontSize: 14 },
  promptText: { color: colors.textBright },
  foot: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16, paddingHorizontal: 4 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
}));
