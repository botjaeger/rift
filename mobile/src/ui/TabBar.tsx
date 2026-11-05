import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { Directions, Gesture, GestureDetector } from "react-native-gesture-handler";
import { Icon } from "./Icon";
import { colors, mono, ui } from "./theme";

interface Props {
  title: string;
  /** 1-based place of the tab among the window's tabs, and how many there are. */
  number: number;
  count: number;
  /** The neighbours' names (null: none on that side). */
  prev: string | null;
  next: string | null;
  /** The tab's panes and which one this is (0-based). */
  panes: number;
  pane: number;
  onPrev: () => void;
  onNext: () => void;
  /** Opens the Switcher. */
  onOpen: () => void;
}

/**
 * The tab bar over the command line (the Pane board): this tab's name with its place and a dot per
 * pane, the neighbours' names at the sides. A swipe left or right goes to the next or previous tab,
 * a tap on a side name goes there, a tap in the middle opens the Switcher. A screen reader gets
 * Next tab / Previous tab as actions.
 */
export function TabBar({ title, number, count, prev, next, panes, pane, onPrev, onNext, onOpen }: Props) {
  // flings only: a vertical drag still scrolls the output above, a tap still taps
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
  return (
    <GestureDetector gesture={swipe}>
      <View style={styles.bar}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={prev ? `Previous tab: ${prev}` : undefined}
          accessibilityElementsHidden={prev === null}
          importantForAccessibility={prev === null ? "no-hide-descendants" : "auto"}
          disabled={prev === null}
          onPress={onPrev}
          style={({ pressed }) => [styles.side, pressed && styles.pressed]}
        >
          <Text style={styles.sideText} numberOfLines={1}>
            {prev ? `‹ ${prev}` : ""}
          </Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${title}, tab ${number} of ${count}${panes > 1 ? `, pane ${pane + 1} of ${panes}` : ""}. Open the switcher`}
          accessibilityActions={actions}
          onAccessibilityAction={(e) => {
            if (e.nativeEvent.actionName === "next" && next !== null) onNext();
            else if (e.nativeEvent.actionName === "previous" && prev !== null) onPrev();
          }}
          onPress={onOpen}
          hitSlop={{ top: 4, bottom: 4 }}
          style={({ pressed }) => [styles.middle, pressed && styles.pressed]}
        >
          <View style={styles.titleLine}>
            <Text style={styles.title} numberOfLines={1}>
              {title}
            </Text>
            <Icon name="chevronDown" size={12} color={colors.textMuted} />
          </View>
          <View style={styles.meta}>
            <Text style={styles.metaText}>
              tab {number} of {count}
            </Text>
            {panes > 1 ? (
              <>
                <View style={styles.dots}>
                  {Array.from({ length: Math.min(panes, 6) }, (_, i) => (
                    <View key={i} style={[styles.dot, i === pane && styles.dotOn]} />
                  ))}
                </View>
                <Text style={styles.metaText}>{panes} panes</Text>
              </>
            ) : null}
          </View>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={next ? `Next tab: ${next}` : undefined}
          accessibilityElementsHidden={next === null}
          importantForAccessibility={next === null ? "no-hide-descendants" : "auto"}
          disabled={next === null}
          onPress={onNext}
          style={({ pressed }) => [styles.side, styles.sideRight, pressed && styles.pressed]}
        >
          <Text style={[styles.sideText, styles.right]} numberOfLines={1}>
            {next ? `${next} ›` : ""}
          </Text>
        </Pressable>
      </View>
    </GestureDetector>
  );
}

const styles = StyleSheet.create({
  bar: { height: 48, flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 6 },
  side: { width: 72, height: 44, justifyContent: "center" },
  sideRight: { alignItems: "flex-end" },
  sideText: { fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  right: { textAlign: "right" },
  middle: { flex: 1, minWidth: 0, alignItems: "center", gap: 3 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 6, maxWidth: "100%" },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 16 },
  meta: { flexDirection: "row", alignItems: "center", gap: 6 },
  metaText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 11 },
  dots: { flexDirection: "row", gap: 3 },
  dot: { width: 5, height: 5, borderRadius: 3, backgroundColor: colors.borderStrong },
  dotOn: { width: 12, backgroundColor: colors.accent },
  pressed: { opacity: 0.7 },
});
