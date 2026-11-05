import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import { countLabel, selectionCount, showsOwnCopied } from "@/selection";
import { Icon } from "./Icon";
import { colors, mono, ui } from "./theme";

/** One of the bar's words; `sep` draws a rule before it (To prompt stands apart from the ways to widen). */
export interface SelectionKey {
  label: string;
  hint: string;
  onPress: () => void;
  sep?: boolean;
}

/**
 * The bar a selection shows (the "Feedback fixes, playable" board), the same for block output and a
 * program's screen, and in the same place: docked over the pane's controls (the message box, the key
 * row), which stay in place under it, hidden, so it never covers the selected text, a handle or the
 * cursor, and nothing moves when it comes. How much is selected, "Copied" once it is (not on Android 13 and later,
 * whose system says it), else `note`; then the keys and ✕.
 */
export function SelectionBar({
  selected,
  copied,
  note,
  keys,
  onCancel,
}: {
  selected: string;
  copied: boolean;
  note?: string;
  keys: SelectionKey[];
  onCancel: () => void;
}) {
  return (
    <View
      style={styles.bar}
      accessibilityRole="toolbar"
      accessibilityLabel="Selection"
    >
      <View style={styles.info}>
        <Text style={styles.count}>{countLabel(selectionCount(selected))}</Text>
        {copied && showsOwnCopied(Platform.OS, Platform.Version) ? (
          <Text style={styles.copied} accessibilityLiveRegion="polite">
            Copied
          </Text>
        ) : note ? (
          <Text style={styles.note}>{note}</Text>
        ) : null}
      </View>
      <View style={styles.keys}>
        {keys.map((k) => (
          <View key={k.label} style={styles.keyWrap}>
            {k.sep ? <View style={styles.sep} /> : null}
            <Pressable accessibilityRole="button" accessibilityLabel={k.label} accessibilityHint={k.hint} onPress={k.onPress} hitSlop={4} style={({ pressed }) => [styles.key, pressed && styles.pressed]}>
              <Text style={styles.keyText} numberOfLines={1}>
                {k.label}
              </Text>
            </Pressable>
          </View>
        ))}
        <View style={styles.fill} />
        <Pressable accessibilityRole="button" accessibilityLabel="Cancel the selection" onPress={onCancel} hitSlop={4} style={({ pressed }) => [styles.key, pressed && styles.pressed]}>
          <Icon name="close" size={16} color={colors.textMuted} />
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  // in the flow, at the bottom: the controls' place
  bar: { marginHorizontal: 10, marginTop: 6, marginBottom: 8, gap: 6, padding: 6, borderRadius: 24, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.glassRim },
  info: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, paddingTop: 4 },
  count: { fontFamily: mono, color: colors.text, fontSize: 12 },
  copied: { fontFamily: ui(500), color: colors.success, fontSize: 12 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  keys: { flexDirection: "row", alignItems: "center" },
  keyWrap: { flexDirection: "row", alignItems: "center" },
  sep: { width: 1, height: 20, backgroundColor: colors.borderStrong, marginHorizontal: 2 },
  key: { minWidth: 40, height: 40, paddingHorizontal: 12, borderRadius: 20, alignItems: "center", justifyContent: "center" },
  keyText: { fontFamily: ui(500), color: colors.text, fontSize: 14 },
  pressed: { opacity: 0.7 },
});
