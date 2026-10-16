import { useEffect, useState } from "react";
import { AccessibilityInfo, Animated, Easing, StyleSheet, View, type LayoutChangeEvent } from "react-native";
import { colors } from "./theme";

/**
 * The thin bar under a computer's status line while a path is tried (RelayPhone board): a 45 %
 * segment sweeping along a 3 pt track; still, at 45 %, with Reduce Motion. Decorative: the
 * status line says the same in words.
 */
export function ProgressBar() {
  const [width, setWidth] = useState(0);
  const [still, setStill] = useState(false);
  const [x] = useState(() => new Animated.Value(0));
  useEffect(() => {
    void AccessibilityInfo.isReduceMotionEnabled()
      .then(setStill)
      .catch(() => undefined);
  }, []);
  useEffect(() => {
    if (still || width === 0) return;
    const loop = Animated.loop(Animated.timing(x, { toValue: 1, duration: 1400, easing: Easing.inOut(Easing.ease), useNativeDriver: true }));
    loop.start();
    return () => loop.stop();
  }, [still, width, x]);
  const bar = width * 0.45;
  return (
    <View
      style={styles.track}
      onLayout={(e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width)}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      <Animated.View
        style={[
          styles.bar,
          { width: bar },
          !still && { transform: [{ translateX: x.interpolate({ inputRange: [0, 1], outputRange: [-bar, width] }) }] },
        ]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  track: { height: 3, borderRadius: 2, backgroundColor: colors.glass, overflow: "hidden" },
  bar: { height: 3, backgroundColor: colors.accent },
});
