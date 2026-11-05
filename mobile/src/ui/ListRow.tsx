import type { ReactNode } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import type { TabKind } from "@/now";
import { Icon } from "./Icon";
import type { RowHandle } from "./ReorderRow";
import { KIND_COLOR, StatusMark } from "./StatusMark";
import { colors, mono, tints, ui } from "./theme";

/** A row's height: rows sit edge to edge, so this is also the drag's step. */
export const LIST_ROW_H = 64;

/**
 * One row of the Tabs and Panes pages (the "Feedback fixes, playable" board): the grip that drags
 * it, its title (with its marks), what it does, and on the other side its status mark and since
 * when; ✕ closes it. The row is one button for a screen reader, its moves are its actions; the
 * grip is only for a finger.
 */
export function ListRow({
  handle,
  movable,
  title,
  marks,
  line,
  kind,
  when,
  label,
  selected,
  lead,
  onPress,
  onClose,
  closeLabel,
}: {
  handle: RowHandle;
  /** Shows the grip (the whole list in its own order); else the grip's room stays empty. */
  movable: boolean;
  title: string;
  /** Beside the title: the pin, the pane count. */
  marks?: ReactNode;
  line: string;
  kind: TabKind;
  when: string;
  label: string;
  selected: boolean;
  /** Before the title (a pane's program tile). */
  lead?: ReactNode;
  onPress: () => void;
  onClose?: () => void;
  closeLabel?: string;
}) {
  const { grip, ...a11y } = handle;
  return (
    <View style={[styles.row, selected && styles.selected]}>
      {movable ? (
        grip(
          <View style={styles.grip} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
            <Icon name="grip" size={16} color={colors.textMuted} />
          </View>,
        )
      ) : (
        <View style={styles.noGrip} />
      )}
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ selected }}
        accessibilityLabel={label}
        accessibilityHint={a11y.accessibilityActions.length > 0 ? "Actions move it" : undefined}
        {...a11y}
        onPress={onPress}
        style={({ pressed }) => [styles.body, pressed && styles.pressed]}
      >
        {lead}
        <View style={styles.text}>
          <View style={styles.titleLine}>
            <Text style={styles.title} numberOfLines={1}>
              {title}
            </Text>
            {marks}
          </View>
          {/* textMuted on the selected row: textDim is 4.3:1 on the accent tint (palette.ts) */}
          <Text style={[styles.line, selected && { color: colors.textMuted }]} numberOfLines={1}>
            {line}
          </Text>
        </View>
        <View style={styles.side}>
          <StatusMark kind={kind} />
          {when ? (
            <Text style={[styles.when, { color: kind === "idle" ? colors.textMuted : KIND_COLOR[kind] }]} numberOfLines={1}>
              {when}
            </Text>
          ) : null}
        </View>
      </Pressable>
      {onClose ? (
        <Pressable accessibilityRole="button" accessibilityLabel={closeLabel} onPress={onClose} style={({ pressed }) => [styles.close, pressed && styles.pressed]}>
          <Icon name="close" size={14} color={colors.textMuted} />
        </Pressable>
      ) : (
        <View style={styles.noClose} />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { height: LIST_ROW_H, flexDirection: "row", alignItems: "center", borderBottomWidth: 1, borderBottomColor: colors.hairline, backgroundColor: colors.bg },
  selected: { backgroundColor: tints.running },
  pressed: { opacity: 0.7 },
  grip: { width: 40, height: LIST_ROW_H, alignItems: "center", justifyContent: "center" },
  noGrip: { width: 16 },
  body: { flex: 1, minWidth: 0, height: LIST_ROW_H, flexDirection: "row", alignItems: "center", gap: 12 },
  text: { flex: 1, minWidth: 0, gap: 4 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 6, minWidth: 0 },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  line: { fontFamily: mono, color: colors.textDim, fontSize: 12 },
  side: { minWidth: 36, alignItems: "flex-end", justifyContent: "center", gap: 5 },
  when: { fontFamily: ui(500), fontSize: 11, fontVariant: ["tabular-nums"] },
  close: { width: 44, height: 44, alignItems: "center", justifyContent: "center" },
  noClose: { width: 12 },
});
