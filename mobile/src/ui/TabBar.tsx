import { useMemo } from "react";
import { Pressable, Text, View } from "react-native";
import { Directions, Gesture, GestureDetector } from "react-native-gesture-handler";
import type { TabKind } from "@/now";
import type { ColorName } from "./palette";
import { Icon } from "./Icon";
import { glyph } from "./Screen";
import { KIND_COLOR } from "./StatusMark";
import { ui, makeStyles, useColors } from "./theme";

/** The tab row's ‹ › buttons: plain grey chevrons on a 32 pt square (44 to the touch with their slop), so Back stays the header's only round ‹ (user, 2026-11-07). */
export const TAB_ARROW = 32;

interface Props {
  title: string;
  /** The tab's status, as its mark on the desktop's tab bar. */
  kind: TabKind;
  /** The dot's colour instead of the status's: grey while the pane opens, dim once it closed. */
  dot?: ColorName;
  /** 1-based place of the tab among the window's tabs, and how many there are. */
  number: number;
  count: number;
  /** The neighbours' names (null: none on that side). */
  prev: string | null;
  next: string | null;
  onPrev: () => void;
  onNext: () => void;
  /** Opens the Tabs page. */
  onOpen: () => void;
}

/**
 * The tab row inside the pane's header block (the New Blocks board): as wide at one tab as at a
 * hundred. ‹ and ›, plain grey chevrons fixed at the row's two ends, go to the neighbouring tabs
 * (dimmed where there is none); the middle names this tab, its status dot before the name and its place under it, and
 * opens the Tabs page. A fling left or right does what › and ‹ do; a screen reader gets Next tab
 * / Previous tab as actions. The header around it draws the ground and the one hairline.
 */
export function TabBar({ title, kind, dot, number, count, prev, next, onPrev, onNext, onOpen }: Props) {
  const colors = useColors();
  const styles = useStyles();
  // flings only: a tap still taps
  const swipe = useMemo(
    () =>
      Gesture.Race(
        Gesture.Fling()
          .runOnJS(true)
          .direction(Directions.LEFT)
          .enabled(next !== null)
          .onEnd(onNext),
        Gesture.Fling()
          .runOnJS(true)
          .direction(Directions.RIGHT)
          .enabled(prev !== null)
          .onEnd(onPrev),
      ),
    [next, prev, onNext, onPrev],
  );
  const actions = [...(next !== null ? [{ name: "next", label: "Next tab" }] : []), ...(prev !== null ? [{ name: "previous", label: "Previous tab" }] : [])];
  const arrow = (to: string | null, label: string, icon: "chevronLeft" | "chevronRight", go: () => void) => (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={to ? `${label}: ${to}` : `No ${label.toLowerCase()}`}
      accessibilityState={{ disabled: to === null }}
      disabled={to === null}
      onPress={go}
      hitSlop={6}
      style={({ pressed }) => [styles.arrow, pressed && styles.pressed]}
    >
      <Icon name={icon} size={glyph(20, 13)} color={to === null ? colors.borderStrong : colors.textMuted} />
    </Pressable>
  );
  return (
    <GestureDetector gesture={swipe}>
      <View style={styles.bar}>
        {arrow(prev, "Previous tab", "chevronLeft", onPrev)}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${title}, tab ${number} of ${count}. Open the tabs`}
          accessibilityActions={actions}
          onAccessibilityAction={(e) => {
            if (e.nativeEvent.actionName === "next" && next !== null) onNext();
            else if (e.nativeEvent.actionName === "previous" && prev !== null) onPrev();
          }}
          onPress={onOpen}
          hitSlop={{ top: 6, bottom: 6 }}
          style={({ pressed }) => [styles.middle, pressed && styles.pressed]}
        >
          <View style={styles.titleLine}>
            <View style={[styles.dot, { backgroundColor: colors[dot ?? KIND_COLOR[kind]] }]} />
            <Text style={styles.title} numberOfLines={1}>
              {title}
            </Text>
            <Icon name="chevronDown" size={glyph(12, 8)} color={colors.textMuted} />
          </View>
          <Text style={styles.metaText}>
            tab {number} of {count}
          </Text>
        </Pressable>
        {arrow(next, "Next tab", "chevronRight", onNext)}
      </View>
    </GestureDetector>
  );
}

const useStyles = makeStyles((colors) => ({
  bar: { flexDirection: "row", alignItems: "center", gap: 8 },
  arrow: { width: TAB_ARROW, height: TAB_ARROW, alignItems: "center", justifyContent: "center" },
  middle: { flex: 1, minWidth: 0, minHeight: 34, alignItems: "center", justifyContent: "center" },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 6, maxWidth: "100%" },
  dot: { width: 7, height: 7, borderRadius: 4 },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 14, lineHeight: 18 },
  metaText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 11, lineHeight: 14 },
  pressed: { opacity: 0.7 },
}));
