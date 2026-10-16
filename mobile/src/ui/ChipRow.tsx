import { useState } from "react";
import { Pressable, StyleSheet, Text, View, type LayoutChangeEvent } from "react-native";
import { CHIP_GAP, DOT_GAP, DOT_SIZE, fitChips, moreSpoken, type ChipItem } from "@/promptChips";
import { Icon, type IconName } from "./Icon";
import { colors, mono, radius, tints } from "./theme";

/** A chip's height; the touch target is 44 (hitSlop makes up the rest). */
export const CHIP_HEIGHT = 32;
const SLOP = (44 - CHIP_HEIGHT) / 2;

/** Everything a chip's width depends on: a new value measures again. */
const widthKey = (c: ChipItem) => `${c.id}|${c.icon}|${c.short.map((p) => p.text).join("\u0000")}`;

/** One chip's contents: its glyph and its parts, each in its own colour (the desktop's prompt chip). */
function ChipBody({ item }: { item: ChipItem }) {
  return (
    <>
      <Icon name={item.icon as IconName} size={13} color={item.iconColor} />
      {/* one Text, so a chip that is cut short ends in … as a whole instead of each part shrinking */}
      <Text style={styles.part} numberOfLines={1} ellipsizeMode="tail">
        {item.short.map((p, i) => (
          <Text key={i} style={{ color: p.color }}>
            {i > 0 ? " " : ""}
            {p.text}
          </Text>
        ))}
      </Text>
    </>
  );
}

/**
 * The prompt chips above the command line: one row that never wraps and never scrolls sideways.
 * The chips that fit (their real widths, measured from an invisible copy of the row) are drawn in
 * the pane's order; the rest fold into "+N" at the end, with a dot for each folded chip that needs
 * attention, which opens the "This pane" sheet listing all of them.
 */
export function ChipRow({
  items,
  onOpen,
  onMore,
}: {
  items: ChipItem[];
  /** A shown chip with a menu was tapped. */
  onOpen: (item: ChipItem) => void;
  /** "+N" was tapped. */
  onMore: () => void;
}) {
  const [avail, setAvail] = useState(0);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [moreBase, setMoreBase] = useState(0);
  const keys = items.map(widthKey);
  const measured = avail > 0 && moreBase > 0 && keys.every((k) => widths[k] !== undefined);
  const fit = measured ? fitChips(keys.map((k) => widths[k]!), items.map((c) => c.attention), avail, moreBase) : null;
  const shown = fit ? items.slice(0, fit.count) : [];
  const hidden = fit ? items.slice(fit.count) : [];
  const dots = hidden.filter((c) => c.attention).length;

  const learn = (k: string) => (e: LayoutChangeEvent) => {
    const w = Math.ceil(e.nativeEvent.layout.width);
    setWidths((prev) => (prev[k] === w ? prev : { ...prev, [k]: w }));
  };

  const chip = (c: ChipItem, maxWidth?: number) => {
    const style = [styles.chip, maxWidth !== undefined && { maxWidth }];
    if (!c.opens)
      return (
        <View key={c.id} accessible accessibilityRole="text" accessibilityLabel={c.spoken} style={style}>
          <ChipBody item={c} />
        </View>
      );
    return (
      <Pressable
        key={c.id}
        accessibilityRole="button"
        accessibilityLabel={c.spoken}
        onPress={() => onOpen(c)}
        hitSlop={{ top: SLOP, bottom: SLOP }}
        style={({ pressed }) => [style, pressed && styles.pressed]}
      >
        <ChipBody item={c} />
      </Pressable>
    );
  };

  return (
    <View
      style={styles.row}
      onLayout={(e) => setAvail(Math.floor(e.nativeEvent.layout.width))}
      accessibilityRole="toolbar"
      accessibilityLabel="Prompt chips"
    >
      {shown.map((c, i) => chip(c, i === shown.length - 1 ? (fit?.clamp ?? undefined) : undefined))}
      {hidden.length > 0 ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={moreSpoken(hidden)}
          onPress={onMore}
          hitSlop={{ top: SLOP, bottom: SLOP }}
          style={({ pressed }) => [styles.chip, styles.more, pressed && styles.pressed]}
        >
          <Text style={[styles.part, { color: colors.text }]}>+{hidden.length}</Text>
          {Array.from({ length: dots }, (_, i) => (
            <View key={i} style={styles.dot} />
          ))}
        </Pressable>
      ) : null}
      {/* an invisible copy at natural widths: what each chip and "+N" really measure */}
      <View style={styles.probe} pointerEvents="none" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        {items.map((c, i) => (
          <View key={keys[i]} onLayout={learn(keys[i]!)} style={styles.chip}>
            <ChipBody item={c} />
          </View>
        ))}
        <View
          style={[styles.chip, styles.more]}
          onLayout={(e) => setMoreBase(Math.ceil(e.nativeEvent.layout.width))}
        >
          <Text style={[styles.part, { color: colors.text }]}>+{items.length}</Text>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  // one row, never wrapping; whatever does not fit is folded into "+N" before it is drawn
  row: { height: CHIP_HEIGHT, flexDirection: "row", alignItems: "center", gap: CHIP_GAP, overflow: "hidden" },
  // hairline, 4 % fill, radius 6, glyph and parts (the desktop's prompt chip)
  chip: {
    height: CHIP_HEIGHT,
    paddingHorizontal: 10,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: tints.chipFill,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexShrink: 0,
  },
  more: { marginLeft: "auto", backgroundColor: colors.glass, gap: DOT_GAP },
  part: { fontFamily: mono, fontSize: 13, flexShrink: 1 },
  dot: { width: DOT_SIZE, height: DOT_SIZE, borderRadius: DOT_SIZE / 2, backgroundColor: colors.warning },
  pressed: { opacity: 0.7 },
  probe: { position: "absolute", left: 0, top: 0, width: 4000, flexDirection: "row", gap: CHIP_GAP, opacity: 0 },
});
