import { useEffect, useState } from "react";
import { AccessibilityInfo, Animated, Easing, View, type StyleProp, type ViewStyle } from "react-native";
import type { TabKind } from "@/now";
import type { ColorName } from "./palette";
import { makeStyles, useColors } from "./theme";

/** A state's colour, as the desktop's tab marks: needs you warning, failed error, working accent, idle success. */
export const KIND_COLOR: Record<TabKind, ColorName> = {
  needs: "warning",
  working: "accent",
  failed: "error",
  idle: "success",
};

/** An arc turning once a second (still under Reduce Motion): something runs. */
export function Spinner({ size = 14, style }: { size?: number; style?: StyleProp<ViewStyle> }) {
  const styles = useStyles();
  const [turn] = useState(() => new Animated.Value(0));
  // made once: a new interpolation on each render re-attaches the native animation
  const [rotate] = useState(() => turn.interpolate({ inputRange: [0, 1], outputRange: ["0deg", "360deg"] }));
  const [still, setStill] = useState(false);
  useEffect(() => {
    let live = true;
    void AccessibilityInfo.isReduceMotionEnabled().then((r) => live && setStill(r));
    return () => {
      live = false;
    };
  }, []);
  useEffect(() => {
    if (still) return;
    const loop = Animated.loop(Animated.timing(turn, { toValue: 1, duration: 1000, easing: Easing.linear, useNativeDriver: true }));
    loop.start();
    return () => loop.stop();
  }, [still, turn]);
  const box = { width: size, height: size, borderRadius: size / 2, borderWidth: size > 12 ? 2.5 : 2 };
  return (
    <Animated.View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[styles.arc, box, style, { transform: [{ rotate }] }]}
    />
  );
}

/**
 * A tab's or pane's status, as the desktop's tab bar draws it: the spinner while it works, else a
 * dot in its state's colour, ringed while it needs you. Decorative: rows say the state in words.
 */
export function StatusMark({ kind, size = 8 }: { kind: TabKind; size?: number }) {
  const colors = useColors();
  if (kind === "working") return <Spinner size={size + 5} />;
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[
        { width: size, height: size, borderRadius: size / 2, backgroundColor: colors[KIND_COLOR[kind]] },
        kind === "needs" && { borderWidth: 0, shadowColor: colors.warning, shadowOpacity: 0.6, shadowRadius: 3, shadowOffset: { width: 0, height: 0 } },
      ]}
    />
  );
}

const useStyles = makeStyles((colors) => ({
  arc: { borderColor: colors.accent, borderLeftColor: "transparent" },
}));
