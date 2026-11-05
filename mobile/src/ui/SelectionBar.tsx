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
 * The bar a selection shows (the SelectBlock and SelectTerminal boards), the same for block output
 * and a program's screen: how much is selected, "Copied" once it is (not on Android 13 and later,
 * whose system says it), else `note`; then the keys and ✕. Next to the selection (`top`), or at
 * the top or the bottom of the area it floats over (`low`), never over the selected text.
 */
export function SelectionBar({
  selected,
  copied,
  note,
  keys,
  onCancel,
  low,
  top,
  onHeight,
}: {
  selected: string;
  copied: boolean;
  note?: string;
  keys: SelectionKey[];
  onCancel: () => void;
  low: boolean;
  /** A place of its own (points from the area's top), next to the selection; `low` otherwise. */
  top?: number;
  onHeight?: (h: number) => void;
}) {
  return (
    <View
      style={[styles.bar, top !== undefined ? { top } : low ? styles.low : styles.high]}
      onLayout={onHeight ? (e) => onHeight(e.nativeEvent.layout.height) : undefined}
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
  bar: { position: "absolute", left: 8, right: 8, gap: 6, padding: 6, borderRadius: 24, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.glassRim, elevation: 8, shadowColor: "#000", shadowOpacity: 0.6, shadowRadius: 14, shadowOffset: { width: 0, height: 6 } },
  low: { bottom: 8 },
  high: { top: 8 },
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
