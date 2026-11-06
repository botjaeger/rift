import * as SplashScreen from "expo-splash-screen";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Animated, StyleSheet, Text, View } from "react-native";
import { SPLASH, splashFadeAt, splashFrame, splashSteps } from "@/splash";
import { monoMedium, makeStyles } from "./theme";

/**
 * The opening animation over the app's first screen (src/splash.ts has its timeline). The native
 * splash is the same ground with nothing on it, so it hands over without a jump: the mark `>▌`
 * waits at an empty prompt, "rift" is typed after it, and the splash fades to the app. It never
 * takes a touch.
 */
export function Splash({ onDone }: { onDone: () => void }) {
  const styles = useStyles();
  const [frame, setFrame] = useState(() => splashFrame(0));
  const [fade] = useState(() => new Animated.Value(1));

  useEffect(() => {
    let live = true;
    const timers: ReturnType<typeof setTimeout>[] = [];
    let animation: Animated.CompositeAnimation | null = null;
    void SplashScreen.hideAsync();
    void AccessibilityInfo.isReduceMotionEnabled()
      .catch(() => false)
      .then((reduce) => {
        if (!live) return;
        setFrame(splashFrame(0, reduce));
        for (const at of splashSteps(reduce)) timers.push(setTimeout(() => setFrame(splashFrame(at)), at));
        timers.push(
          setTimeout(() => {
            animation = Animated.timing(fade, { toValue: 0, duration: SPLASH.fade, useNativeDriver: true });
            animation.start(() => onDone());
          }, splashFadeAt(reduce)),
        );
      });
    return () => {
      live = false;
      timers.forEach(clearTimeout);
      animation?.stop();
    };
  }, [fade, onDone]);

  return (
    <Animated.View
      pointerEvents="none"
      style={[StyleSheet.absoluteFill, styles.root, { opacity: fade }]}
      accessible
      accessibilityLabel="Rift"
      testID="splash"
    >
      <View style={styles.prompt}>
        <View style={styles.chevron}>
          <View style={[styles.arm, styles.armTop]} />
          <View style={[styles.arm, styles.armBottom]} />
        </View>
        <Text style={styles.word}>{SPLASH.word.slice(0, frame.typed)}</Text>
        <View style={[styles.cursor, { opacity: frame.cursor ? 1 : 0 }]} />
      </View>
    </Animated.View>
  );
}

/** Rift's mark `>▌` at rest (the lock screen): the chevron and the cursor, no word. */
export function Mark() {
  const styles = useStyles();
  return (
    <View style={styles.prompt} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <View style={styles.chevron}>
        <View style={[styles.arm, styles.armTop]} />
        <View style={[styles.arm, styles.armBottom]} />
      </View>
      <View style={styles.cursor} />
    </View>
  );
}

// The chevron is the mark's path M6 6 L22 20 L6 34 in a 30 × 40 box, stroke 6 with round caps:
// two arms of length √(16² + 14²) ≈ 21.3, each a pill turned ±41.2° about its midpoint.
const ARM = Math.hypot(16, 14);
const STROKE = 6;
const ANGLE = `${(Math.atan2(14, 16) * 180) / Math.PI}deg`;

const useStyles = makeStyles((colors) => ({
  root: { backgroundColor: colors.chrome, alignItems: "center", justifyContent: "center" },
  prompt: { flexDirection: "row", alignItems: "center", gap: 12 },
  chevron: { width: 30, height: 40 },
  arm: {
    position: "absolute",
    width: ARM + STROKE,
    height: STROKE,
    borderRadius: STROKE / 2,
    backgroundColor: colors.accent,
    left: 14 - (ARM + STROKE) / 2,
  },
  armTop: { top: 13 - STROKE / 2, transform: [{ rotate: ANGLE }] },
  armBottom: { top: 27 - STROKE / 2, transform: [{ rotate: `-${ANGLE}` }] },
  word: { fontFamily: monoMedium, fontSize: 34, lineHeight: 40, color: colors.textBright },
  cursor: { width: 18, height: 36, borderRadius: 2, backgroundColor: colors.fg },
}));
