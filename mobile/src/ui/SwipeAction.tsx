import type { ReactNode } from "react";
import { useState } from "react";
import { Animated, PanResponder, Pressable, StyleSheet, Text, View } from "react-native";
import { ACTION_W, settle } from "./swipe";
import { colors, ui } from "./theme";

/** The row's position and its drag, made once: no ref is read while rendering. */
function makeRow(onOpen: (open: boolean) => void) {
  const x = new Animated.Value(0);
  const state = { open: false };
  const go = (to: "open" | "closed") => {
    state.open = to === "open";
    onOpen(state.open);
    Animated.timing(x, { toValue: to === "open" ? -ACTION_W : 0, duration: 160, useNativeDriver: true }).start();
  };
  const pan = PanResponder.create({
    // only a mostly-sideways drag moves the row: scrolling the page stays a scroll
    onMoveShouldSetPanResponder: (_, g) => Math.abs(g.dx) > 12 && Math.abs(g.dx) > 2 * Math.abs(g.dy),
    onPanResponderMove: (_, g) => x.setValue(Math.min(0, Math.max(-ACTION_W, (state.open ? -ACTION_W : 0) + g.dx))),
    onPanResponderRelease: (_, g) => go(settle(state.open, g.dx)),
    onPanResponderTerminate: () => go(state.open ? "open" : "closed"),
  });
  return { x, pan, go };
}

/**
 * A row that slides left to show one destructive action behind it (a Portals card's Unpair). The
 * action is a real button even while the row covers it, so a screen reader reaches it without the
 * gesture. While it is open, a tap on the row closes it instead of pressing what is under the finger.
 */
export function SwipeAction({
  children,
  label,
  text,
  onAction,
  radius,
}: {
  children: ReactNode;
  /** The spoken name of the action ("Unpair studio-mac"). */
  label: string;
  /** What the action shows ("Unpair"). */
  text: string;
  onAction: () => void;
  /** The row's corner radius: the action's ground is clipped to it. */
  radius: number;
}) {
  const [open, setOpen] = useState(false);
  const [row] = useState(() => makeRow(setOpen));
  // the red ground shows only once the row has moved: a closed row has no red at its rounded corners
  const shown = row.x.interpolate({ inputRange: [-ACTION_W, -6, 0], outputRange: [1, 1, 0], extrapolate: "clamp" });
  return (
    <View style={[styles.wrap, { borderRadius: radius }]}>
      <Animated.View style={[styles.ground, { opacity: shown }]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={label}
          onPress={() => {
            row.go("closed");
            onAction();
          }}
          style={({ pressed }) => [styles.action, pressed && styles.pressed]}
        >
          <Text style={styles.actionText}>{text}</Text>
        </Pressable>
      </Animated.View>
      <Animated.View style={{ transform: [{ translateX: row.x }] }} {...row.pan.panHandlers}>
        {children}
        {open ? <Pressable accessible={false} importantForAccessibility="no" onPress={() => row.go("closed")} style={StyleSheet.absoluteFill} /> : null}
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { overflow: "hidden" },
  ground: { position: "absolute", right: 0, top: 0, bottom: 0, width: ACTION_W, backgroundColor: colors.error },
  action: { flex: 1, alignItems: "center", justifyContent: "center" },
  pressed: { opacity: 0.8 },
  // the error color is light: dark text keeps the contrast
  actionText: { fontFamily: ui(600), color: colors.bg, fontSize: 14 },
});
