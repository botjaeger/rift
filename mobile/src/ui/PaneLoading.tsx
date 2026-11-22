import { useEffect, useState } from "react";
import { AccessibilityInfo, ActivityIndicator, Animated, Easing, Pressable, StyleSheet, Text, View } from "react-native";
import { Button } from "./Button";
import { Icon } from "./Icon";
import type { PaneLoad, PaneStep } from "./paneLoad";
import { fromPalette, mono, radius, tint, ui, makeStyles, useColors } from "./theme";

const stripFg = fromPalette((colors) => ({ busy: colors.accent, quiet: colors.textMuted, bad: colors.error }));
const stripBg = fromPalette((colors) => ({ busy: tint(colors.accent, 0x14), quiet: "transparent", bad: tint(colors.error, 0x14) }));

/**
 * The pane's status strip (PaneLoading board): full width, its text wraps and is never cut, it
 * shows the wait while connecting, and a tap opens the Connection sheet. It is not a live
 * region (its wait and countdown change each second); the card under it is, politely.
 */
export function PaneStrip({ strip, onPress, onRetry }: { strip: NonNullable<PaneLoad["strip"]>; onPress: () => void; onRetry?: () => void }) {
  const colors = useColors();
  const styles = useStyles();
  const fg = stripFg(colors)[strip.tone];
  const details = (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${strip.text}${strip.elapsed ? `, ${strip.elapsed}` : ""}. Connection details`}
      onPress={onPress}
      style={({ pressed }) => [styles.stripDetails, pressed && styles.pressed]}
    >
      {strip.busy ? <ActivityIndicator size="small" color={fg} /> : <View style={[styles.stripDot, { backgroundColor: fg }]} />}
      <Text style={[styles.stripText, { color: fg }]}>{strip.text}</Text>
      {strip.elapsed ? <Text style={[styles.elapsed, { color: fg }]}>{strip.elapsed}</Text> : null}
      {strip.retry && onRetry ? null : <Icon name="forward" size={12} color={fg} />}
    </Pressable>
  );
  // Try again beside the details, not inside them: a button in a button is one element to a screen reader
  return (
    <View style={[styles.strip, { backgroundColor: stripBg(colors)[strip.tone] }]}>
      {details}
      {strip.retry && onRetry ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Try again"
          hitSlop={{ top: 6, bottom: 6, left: 4, right: 4 }}
          onPress={onRetry}
          style={({ pressed }) => [styles.stripRetry, { borderColor: fg }, pressed && styles.pressed]}
        >
          <Text style={[styles.stripRetryText, { color: fg }]}>Try again</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

// a step's glyph colour (✓ ◌ ○ ✕, decorative: the label says it) and its note's
const stepColors = fromPalette((colors): Record<PaneStep["tone"], { fg: string; note: string }> => ({
  done: { fg: colors.success, note: colors.textMuted },
  now: { fg: colors.accent, note: colors.accent },
  todo: { fg: colors.textDim, note: colors.textMuted },
  bad: { fg: colors.error, note: colors.error },
}));

/**
 * The steps card centred over the skeleton (the New Blocks board): "Opening project1", then
 * Reached the computer ✓, the blocks loading ◌, the screen to come ○, and Details. A step
 * that waits on something else says what under it. After a while, or when the computer cannot
 * be reached, it says why in full with Try again and Details.
 */
export function PaneCard({
  card,
  onRetry,
  onDetails,
  onLeave,
}: {
  card: NonNullable<PaneLoad["card"]>;
  onRetry: () => void;
  onDetails: () => void;
  /** Cancel or Back to Portals (`card.leave`). */
  onLeave: () => void;
}) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <View style={styles.cardLayer} pointerEvents="box-none">
      <View style={styles.card} accessibilityLiveRegion="polite" accessibilityLabel={card.title}>
        <Text style={styles.cardTitle} accessibilityRole="header">
          {card.title}
        </Text>
        {card.steps.map((st) => {
          const c = stepColors(colors)[st.tone];
          return (
            <View key={st.label} style={styles.step} accessible accessibilityLabel={`${st.label}${st.note ? `. ${st.note}` : ""}`}>
              <Text style={[styles.stepMark, { color: c.fg }]}>{st.mark}</Text>
              <View style={styles.stepText}>
                <Text style={styles.stepLabel}>{st.label}</Text>
                {st.note ? <Text style={[styles.stepNote, { color: c.note }]}>{st.note}</Text> : null}
              </View>
            </View>
          );
        })}
        {card.hint ? <Text style={styles.hint}>{card.hint}</Text> : null}
        {/* while it only waits: Details, the Connection sheet */}
        {!card.actions ? (
          <View style={styles.details}>
            <Button label="Details" size="small" onPress={onDetails} />
          </View>
        ) : null}
        {card.actions && card.leave && !card.noRetry ? (
          <>
            <View style={styles.actions}>
              <Button label="Try again" variant="primary" onPress={onRetry} style={styles.action} />
              <Button label={card.leave} onPress={onLeave} style={styles.action} />
            </View>
            <Button label="Details" variant="text" size="small" onPress={onDetails} />
          </>
        ) : card.actions ? (
          <View style={styles.actions}>
            {card.leave ? <Button label={card.leave} onPress={onLeave} style={styles.action} /> : null}
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
  const styles = useStyles();
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
  const styles = useStyles();
  const opacity = usePulse();
  return (
    <View style={styles.rows} pointerEvents="none" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {ROWS.map((w, i) => (
        <Animated.View key={i} style={[styles.bar, { width: w, height: 9, opacity }]} />
      ))}
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
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
  // the strip's tappable words (dot, text, wait, ›): the row's room but Try again's
  stripDetails: { flex: 1, minHeight: 26, flexDirection: "row", alignItems: "center", gap: 8 },
  stripDot: { width: 8, height: 8, borderRadius: 4 },
  stripText: { flex: 1, fontFamily: ui(400), fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: mono, fontSize: 12, opacity: 0.8 },
  // the strip's Try again: an outlined pill in the strip's colour, 32 pt (44 to the touch)
  stripRetry: { height: 32, paddingHorizontal: 14, borderRadius: 16, borderWidth: 1, justifyContent: "center", backgroundColor: colors.surface },
  stripRetryText: { fontFamily: ui(600), fontSize: 13 },
  cardLayer: { ...StyleSheet.absoluteFill, justifyContent: "center", paddingHorizontal: 16 },
  // the New Blocks board's card: the surface, a hairline rim, 14 pt corners, no shadow
  card: {
    gap: 12,
    padding: 16,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
  },
  cardTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 16, lineHeight: 21 },
  // a step: its 18 pt glyph, 10 pt, the label (14/20); a note under the label when it waits on something
  step: { flexDirection: "row", alignItems: "flex-start", gap: 10 },
  stepMark: { width: 18, fontFamily: ui(400), fontSize: 16, lineHeight: 22, textAlign: "center" },
  stepText: { flex: 1, minWidth: 0, gap: 2 },
  stepLabel: { fontFamily: ui(400), color: colors.text, fontSize: 14, lineHeight: 20 },
  stepNote: { fontFamily: ui(400), fontSize: 12, lineHeight: 16 },
  details: { flexDirection: "row", paddingTop: 4 },
  hint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  actions: { flexDirection: "row", gap: 8 },
  action: { flex: 1 },
  blocks: { ...StyleSheet.absoluteFill, justifyContent: "flex-end", gap: 10, paddingHorizontal: 12, paddingTop: 12, paddingBottom: 10 },
  block: { gap: 8, padding: 12, borderRadius: radius.card, borderWidth: 1, borderColor: colors.faint, backgroundColor: colors.skeleton },
  blockHead: { flexDirection: "row", justifyContent: "space-between" },
  bar: { borderRadius: 6, backgroundColor: colors.faint },
  rows: { gap: 9, padding: 12 },
}));
