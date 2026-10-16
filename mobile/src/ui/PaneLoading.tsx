import { useEffect, useState } from "react";
import { AccessibilityInfo, ActivityIndicator, Animated, Easing, Pressable, StyleSheet, Text, View } from "react-native";
import { Button } from "./Button";
import { Icon } from "./Icon";
import type { PaneLoad, PaneStep } from "./paneLoad";
import { colors, mono, tint, ui } from "./theme";

/** The card's ground (the board's #262626, a step above the page). */
const CARD = "#262626";

const STRIP_FG = { busy: colors.accent, quiet: colors.textMuted, bad: colors.error } as const;
const STRIP_BG = { busy: tint(colors.accent, 0x14), quiet: "transparent", bad: tint(colors.error, 0x14) } as const;

/**
 * The pane's status strip (PaneLoading board): full width, its text wraps and is never cut, it
 * shows the wait while connecting, and a tap opens the Connection sheet. It is not a live
 * region (its wait and countdown change each second); the card under it is, politely.
 */
export function PaneStrip({ strip, onPress }: { strip: NonNullable<PaneLoad["strip"]>; onPress: () => void }) {
  const fg = STRIP_FG[strip.tone];
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${strip.text}${strip.elapsed ? `, ${strip.elapsed}` : ""}. Connection details`}
      onPress={onPress}
      style={({ pressed }) => [styles.strip, { backgroundColor: STRIP_BG[strip.tone] }, pressed && styles.pressed]}
    >
      {strip.busy ? <ActivityIndicator size="small" color={fg} /> : <View style={[styles.stripDot, { backgroundColor: fg }]} />}
      <Text style={[styles.stripText, { color: fg }]}>{strip.text}</Text>
      {strip.elapsed ? <Text style={[styles.elapsed, { color: fg }]}>{strip.elapsed}</Text> : null}
      <Icon name="forward" size={12} color={fg} />
    </Pressable>
  );
}

const STEP_COLORS: Record<PaneStep["tone"], { fg: string; bg: string; label: string; note: string }> = {
  done: { fg: colors.success, bg: tint(colors.success, 0x26), label: colors.text, note: colors.textMuted },
  now: { fg: colors.accent, bg: tint(colors.accent, 0x26), label: colors.textBright, note: colors.accent },
  // the board's #ffffff66 label is 3:1 on the card; the muted token keeps AA
  todo: { fg: colors.textMuted, bg: "#ffffff0f", label: colors.textMuted, note: colors.textMuted },
  bad: { fg: colors.error, bg: tint(colors.error, 0x26), label: colors.textBright, note: colors.error },
};

/**
 * The steps card centred over the skeleton: Reach the computer, Open this pane, Load its
 * blocks (or Get the screen), the current step saying what it waits for. After a while, or
 * when the computer cannot be reached, it says why in full with Try again and Details.
 */
export function PaneCard({
  card,
  onRetry,
  onDetails,
}: {
  card: NonNullable<PaneLoad["card"]>;
  onRetry: () => void;
  onDetails: () => void;
}) {
  return (
    <View style={styles.cardLayer} pointerEvents="box-none">
      <View style={styles.card} accessibilityLiveRegion="polite" accessibilityLabel={card.title}>
        <Text style={styles.cardTitle} accessibilityRole="header">
          {card.title}
        </Text>
        <View style={styles.steps}>
          {card.steps.map((st) => {
            const c = STEP_COLORS[st.tone];
            return (
              <View key={st.label} style={styles.step} accessible accessibilityLabel={`${st.label}${st.note ? `. ${st.note}` : ""}`}>
                <View style={[styles.stepMark, { backgroundColor: c.bg }]}>
                  <Text style={[styles.stepMarkText, { color: c.fg }]}>{st.mark}</Text>
                </View>
                <View style={styles.stepText}>
                  <Text style={[styles.stepLabel, { color: c.label }]}>{st.label}</Text>
                  {st.note ? <Text style={[styles.stepNote, { color: c.note }]}>{st.note}</Text> : null}
                </View>
              </View>
            );
          })}
        </View>
        {card.hint ? <Text style={styles.hint}>{card.hint}</Text> : null}
        {card.actions ? (
          <View style={styles.actions}>
            {card.noRetry ? null : <Button label="Try again" variant="primary" onPress={onRetry} style={styles.action} />}
            <Button label="Details" onPress={onDetails} style={styles.action} />
          </View>
        ) : null}
      </View>
    </View>
  );
}

/** The board's pulse: .45 to .9 and back, still with Reduce Motion. */
function usePulse(): Animated.Value {
  const [v] = useState(() => new Animated.Value(0.6));
  const [still, setStill] = useState(false);
  useEffect(() => {
    void AccessibilityInfo.isReduceMotionEnabled()
      .then(setStill)
      .catch(() => undefined);
  }, []);
  useEffect(() => {
    if (still) return;
    const loop = Animated.loop(
      Animated.sequence([
        Animated.timing(v, { toValue: 0.9, duration: 800, easing: Easing.inOut(Easing.ease), useNativeDriver: true }),
        Animated.timing(v, { toValue: 0.45, duration: 800, easing: Easing.inOut(Easing.ease), useNativeDriver: true }),
      ]),
    );
    loop.start();
    return () => loop.stop();
  }, [still, v]);
  return v;
}

const BLOCKS = [
  { a: "52%", b: "86%", c: "40%" },
  { a: "34%", b: "70%", c: "62%" },
  { a: "60%", b: "92%", c: "28%" },
  { a: "44%", b: "58%", c: "74%" },
] as const;

/** Blocks that are not there yet: four cards of pulsing bars, at the bottom as the real ones sit. */
export function BlockSkeleton() {
  const opacity = usePulse();
  return (
    <View style={styles.blocks} pointerEvents="none" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {BLOCKS.map((k, i) => (
        <View key={i} style={styles.block}>
          <View style={styles.blockHead}>
            <Animated.View style={[styles.bar, { width: k.a, height: 12, opacity }]} />
            <Animated.View style={[styles.bar, { width: 40, height: 12, opacity }]} />
          </View>
          <Animated.View style={[styles.bar, { width: k.b, height: 10, opacity }]} />
          <Animated.View style={[styles.bar, { width: k.c, height: 10, opacity }]} />
        </View>
      ))}
    </View>
  );
}

const ROWS = ["72%", "40%", "88%", "64%", "30%", "80%", "55%", "92%", "46%", "70%", "38%", "84%", "60%", "25%", "78%", "50%", "90%", "42%", "66%", "34%", "82%", "58%", "48%", "74%", "36%"] as const;

/** The terminal's rows that are not there yet. */
export function ScreenSkeleton() {
  const opacity = usePulse();
  return (
    <View style={styles.rows} pointerEvents="none" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {ROWS.map((w, i) => (
        <Animated.View key={i} style={[styles.bar, { width: w, height: 9, opacity }]} />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.7 },
  strip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
    paddingHorizontal: 16,
    paddingVertical: 9,
    borderBottomWidth: 1,
    borderBottomColor: colors.hairline,
  },
  stripDot: { width: 8, height: 8, borderRadius: 4 },
  stripText: { flex: 1, fontFamily: ui(400), fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: mono, fontSize: 12, opacity: 0.8 },
  cardLayer: { ...StyleSheet.absoluteFill, justifyContent: "center", paddingHorizontal: 16 },
  card: {
    gap: 12,
    padding: 16,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.glassRim,
    backgroundColor: CARD,
    shadowColor: "#000000",
    shadowOpacity: 0.5,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 12 },
    elevation: 12,
  },
  cardTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  steps: { gap: 10 },
  step: { flexDirection: "row", alignItems: "flex-start", gap: 10 },
  stepMark: { width: 20, height: 20, borderRadius: 10, alignItems: "center", justifyContent: "center" },
  stepMarkText: { fontFamily: ui(500), fontSize: 11 },
  stepText: { flex: 1, minWidth: 0, gap: 2 },
  stepLabel: { fontFamily: ui(400), fontSize: 14 },
  stepNote: { fontFamily: ui(400), fontSize: 12, lineHeight: 16 },
  hint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  actions: { flexDirection: "row", gap: 8 },
  action: { flex: 1, borderRadius: 22 },
  blocks: { ...StyleSheet.absoluteFill, justifyContent: "flex-end", gap: 10, paddingHorizontal: 12, paddingTop: 12, paddingBottom: 10 },
  block: { gap: 8, padding: 12, borderRadius: 12, borderWidth: 1, borderColor: "#ffffff0f", backgroundColor: "#232326" },
  blockHead: { flexDirection: "row", justifyContent: "space-between" },
  bar: { borderRadius: 6, backgroundColor: "#ffffff0f" },
  rows: { gap: 9, padding: 12 },
});
