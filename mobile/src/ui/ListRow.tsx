import type { ReactNode } from "react";
import { Pressable, Text, View } from "react-native";
import type { TabKind } from "@/now";
import { Icon } from "./Icon";
import type { RowHandle } from "./ReorderRow";
import { glyph } from "./Screen";
import { KIND_COLOR, StatusMark } from "./StatusMark";
import { monoMedium, ui, makeStyles, useColors } from "./theme";

/**
 * A content row's height (the New Lists board: 62 pt and its 1 pt hairline): rows sit edge to
 * edge, so this is also the drag's step.
 */
export const LIST_ROW_H = 63;

/**
 * A row's status mark and since when (the New Lists and New Portals boards): the 8 pt dot, then
 * the time in JetBrains Mono 500 11 in the state's colour ("4:30", "1m", "✕ 101").
 */
export function RowMark({ kind, when }: { kind: TabKind; when: string }) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <View style={styles.side}>
      <StatusMark kind={kind} />
      {when ? (
        <Text style={[styles.when, { color: kind === "idle" ? colors.textMuted : colors[KIND_COLOR[kind]] }]} numberOfLines={1}>
          {when}
        </Text>
      ) : null}
    </View>
  );
}

/**
 * One row of the Tabs and Panes pages (the New Lists board): the grip that drags it, its title
 * (with its marks), what it does in Inter, and on the other side its status mark and since when;
 * ✕ closes it. The row is one button for a screen reader, its moves are its actions; the grip is
 * only for a finger.
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
  /** Shows the grip (the whole list in its own order); else the row starts with its title. */
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
  const colors = useColors();
  const styles = useStyles();
  const { grip, ...a11y } = handle;
  return (
    <View style={[styles.row, selected && styles.selected]}>
      {movable
        ? grip(
            <View style={styles.grip} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
              <Icon name="grip" size={16} color={colors.textMuted} />
            </View>,
          )
        : null}
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
          <Text style={styles.line} numberOfLines={1}>
            {line}
          </Text>
        </View>
        <RowMark kind={kind} when={when} />
      </Pressable>
      {onClose ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={closeLabel}
          onPress={onClose}
          // the board's 32 pt button, 44 to the touch
          hitSlop={6}
          style={({ pressed }) => [styles.close, pressed && styles.pressed]}
        >
          <Icon name="close" size={glyph(14, 9)} color={colors.textMuted} />
        </Pressable>
      ) : null}
    </View>
  );
}

// The New Lists board's row: 16 pt in, 12 at the end and between its parts, 62 pt and a hairline.
const useStyles = makeStyles((colors) => ({
  row: { height: LIST_ROW_H, flexDirection: "row", alignItems: "center", gap: 12, paddingLeft: 16, paddingRight: 12, borderBottomWidth: 1, borderBottomColor: colors.hairline, backgroundColor: colors.bg },
  selected: { backgroundColor: colors.tints.selected },
  pressed: { opacity: 0.7 },
  // the 16 pt grip, 40 wide to the touch (its slack overlaps the gutter and the gap)
  grip: { width: 40, height: LIST_ROW_H - 1, marginHorizontal: -12, alignItems: "center", justifyContent: "center" },
  body: { flex: 1, minWidth: 0, height: LIST_ROW_H - 1, flexDirection: "row", alignItems: "center", gap: 12 },
  text: { flex: 1, minWidth: 0, gap: 3 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 6, minWidth: 0 },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15, lineHeight: 20 },
  // textMuted: textDim is 4.3:1 on the selected row's accent tint (palette.ts)
  line: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  side: { flexShrink: 0, minWidth: 34, alignItems: "flex-end", justifyContent: "center", gap: 3, alignSelf: "center" },
  when: { fontFamily: monoMedium, fontSize: 11, lineHeight: 14, fontVariant: ["tabular-nums"] },
  close: { width: 32, height: 32, alignItems: "center", justifyContent: "center" },
}));
