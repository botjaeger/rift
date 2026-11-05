import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { Directions, Gesture, GestureDetector } from "react-native-gesture-handler";
import type { TabKind } from "@/now";
import { Icon } from "./Icon";
import { StatusMark } from "./StatusMark";
import { colors, ui } from "./theme";

interface Props {
  title: string;
  /** The tab's status, as its mark on the desktop's tab bar. */
  kind: TabKind;
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
 * The tab row under the pane's header (the "Feedback fixes, playable" board): as wide at one tab
 * as at a hundred. ‹ and › go to the neighbouring tabs (dimmed where there is none), the middle
 * names this tab with its status and place and opens the Tabs page. A fling left or right does
 * what › and ‹ do; a screen reader gets Next tab / Previous tab as actions.
 */
export function TabBar({ title, kind, number, count, prev, next, onPrev, onNext, onOpen }: Props) {
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
      style={({ pressed }) => [styles.arrow, pressed && styles.pressed]}
    >
      <Icon name={icon} size={18} color={to === null ? colors.borderStrong : colors.text} />
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
          hitSlop={{ top: 4, bottom: 4 }}
          style={({ pressed }) => [styles.middle, pressed && styles.pressed]}
        >
          <View style={styles.titleLine}>
            <Text style={styles.title} numberOfLines={1}>
              {title}
            </Text>
            <StatusMark kind={kind} />
            <Icon name="chevronDown" size={12} color={colors.textMuted} />
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

const styles = StyleSheet.create({
  bar: { height: 52, flexDirection: "row", alignItems: "center", gap: 2, paddingHorizontal: 4, backgroundColor: colors.chrome, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  arrow: { width: 44, height: 44, alignItems: "center", justifyContent: "center" },
  middle: { flex: 1, minWidth: 0, alignItems: "center", gap: 2 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 7, maxWidth: "100%" },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  metaText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 11 },
  pressed: { opacity: 0.7 },
});
