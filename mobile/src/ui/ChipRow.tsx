import { useState } from "react";
import { Pressable, Text, View, type LayoutChangeEvent } from "react-native";
import { CHIP_GAP, DOT_GAP, DOT_SIZE, fitChips, isTimer, moreSpoken, type ChipItem } from "@/promptChips";
import { Icon, type IconName } from "./Icon";
import { mono, ui, makeStyles, useColors } from "./theme";

/** A chip's height (the command line redesign board's 28 pt pill); the touch target is 44 (hitSlop makes up the rest). */
export const CHIP_HEIGHT = 28;
const SLOP = (44 - CHIP_HEIGHT) / 2;

/** Everything a chip's width depends on: a new value measures again. */
const widthKey = (c: ChipItem) => `${c.id}|${c.icon}|${c.short.map((p) => p.text).join("\u0000")}`;

/** One chip's contents: its glyph and its parts, each in its own colour (the desktop's prompt chip). */
function ChipBody({ item }: { item: ChipItem }) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <>
      <Icon name={item.icon as IconName} size={13} color={colors[item.iconColor]} />
      {/* one Text, so a chip that is cut short ends in … as a whole instead of each part shrinking */}
      <Text style={styles.part} numberOfLines={1} ellipsizeMode="tail">
        {item.short.map((p, i) => (
          <Text key={i} style={{ color: colors[p.color] }}>
            {i > 0 ? " " : ""}
            {p.text}
          </Text>
        ))}
      </Text>
    </>
  );
}

/**
 * The context above the command line: one row that never wraps and never scrolls sideways.
 * The chips that fit (their real widths, measured from an invisible copy of the row) are drawn in
 * the pane's order; the rest fold into "+N" at the end, with a dot for each folded chip that needs
 * attention, which opens the "This pane" sheet listing all of them. A timer (time on tab, the clock)
 * is plain text after them ("· 2m"), still opening its card when it has one.
 */
export function ChipRow({
  items: all,
  onOpen,
  onMore,
}: {
  items: ChipItem[];
  /** A shown chip with a menu was tapped. */
  onOpen: (item: ChipItem) => void;
  /** "+N" was tapped. */
  onMore: () => void;
}) {
  const colors = useColors();
  const styles = useStyles();
  const [avail, setAvail] = useState(0);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [moreBase, setMoreBase] = useState(0);
  const [timerW, setTimerW] = useState(0);
  // the first timer is the line's text; any other one stays a chip
  const timer = all.find((c) => isTimer(c.id)) ?? null;
  const items = timer ? all.filter((c) => c !== timer) : all;
  const keys = items.map(widthKey);
  const room = avail - (timer ? timerW + CHIP_GAP : 0);
  const measured = avail > 0 && moreBase > 0 && (!timer || timerW > 0) && keys.every((k) => widths[k] !== undefined);
  const fit = measured ? fitChips(keys.map((k) => widths[k]!), items.map((c) => c.attention), room, moreBase) : null;
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
      {timer ? (
        <Pressable
          disabled={!timer.opens}
          accessibilityRole={timer.opens ? "button" : "text"}
          accessibilityLabel={timer.spoken}
          onPress={() => onOpen(timer)}
          // "· 7m" is ~25 pt wide: the reach grows to the right (nothing is there) to make 44
          hitSlop={{ top: SLOP, bottom: SLOP, left: 3, right: 20 }}
          onLayout={(e) => setTimerW(Math.ceil(e.nativeEvent.layout.width))}
          style={({ pressed }) => [styles.timer, pressed && styles.pressed]}
        >
          <Text style={styles.timerText} numberOfLines={1}>
            · {timer.short.map((p) => p.text).join(" ")}
          </Text>
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

const useStyles = makeStyles((colors) => ({
  // one row, never wrapping; whatever does not fit is folded into "+N" before it is drawn
  row: { height: CHIP_HEIGHT, flexDirection: "row", alignItems: "center", gap: CHIP_GAP, overflow: "hidden" },
  // hairline, 4 % fill, a pill, glyph and parts (the desktop's prompt chip)
  chip: {
    height: CHIP_HEIGHT,
    paddingHorizontal: 10,
    borderRadius: CHIP_HEIGHT / 2,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.tints.chipFill,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexShrink: 0,
  },
  more: { backgroundColor: colors.glass, gap: DOT_GAP },
  part: { fontFamily: mono, fontSize: 12, flexShrink: 1 },
  // the timer: plain words after the chips, never cut
  timer: { flexShrink: 0, height: CHIP_HEIGHT, justifyContent: "center" },
  timerText: { fontFamily: ui(400), fontSize: 12, color: colors.textMuted },
  dot: { width: DOT_SIZE, height: DOT_SIZE, borderRadius: DOT_SIZE / 2, backgroundColor: colors.warning },
  pressed: { opacity: 0.7 },
  probe: { position: "absolute", left: 0, top: 0, width: 4000, flexDirection: "row", gap: CHIP_GAP, opacity: 0 },
}));
