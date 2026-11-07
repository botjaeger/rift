import { useEffect, useState } from "react";
import { Pressable, Text, View, type AccessibilityActionEvent } from "react-native";
import { keptZoom, onKeptZoom, setKeptZoom, ZOOM_MAX, ZOOM_MIN, zoomPercent, zoomStep } from "@/paneView";
import { Icon } from "./Icon";
import { makeStyles, TOUCH, ui, useColors } from "./theme";

/**
 * The ⋯ menu's Text size over a full-screen program (the New Screen board: "Text size · Pinch the
 * screen, or here"): tapping the row shows − / the size / + and Reset under it. They change the
 * zoom a pinch changes, within its bounds, kept for the app's run the same way (`setKeptZoom`), so
 * the screen behind the menu follows at once. For a screen reader the size is one adjustable
 * control (swipe up or down); Reset is its own button.
 */
export function TextSizeMenu({ last }: { last?: boolean }) {
  const colors = useColors();
  const styles = useStyles();
  const [open, setOpen] = useState(false);
  const [zoom, setZoom] = useState(keptZoom.value);
  // a pinch while the menu is open (or another view) moves the size shown here too
  useEffect(() => onKeptZoom(setZoom), []);
  const change = (z: number) => setZoom(setKeptZoom(z));
  const percent = zoomPercent(zoom);
  const onAction = (e: AccessibilityActionEvent) => {
    if (e.nativeEvent.actionName === "increment") change(zoomStep(zoom, 1));
    else if (e.nativeEvent.actionName === "decrement") change(zoomStep(zoom, -1));
  };
  return (
    <View style={!last && styles.rule}>
      <Pressable
        accessibilityRole="menuitem"
        accessibilityLabel="Text size"
        accessibilityHint="Pinch the screen, or here"
        accessibilityValue={{ text: percent }}
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen((o) => !o)}
        style={({ pressed }) => [styles.row, pressed && styles.pressed]}
      >
        <Icon name="textSize" size={20} color={colors.text} />
        <View style={styles.text}>
          <Text style={styles.title}>Text size</Text>
          <Text style={styles.hint}>Pinch the screen, or here</Text>
        </View>
      </Pressable>
      {open ? (
        <View style={styles.controls}>
          <View
            accessible
            accessibilityRole="adjustable"
            accessibilityLabel="Text size"
            accessibilityValue={{ min: Math.round(ZOOM_MIN * 100), max: Math.round(ZOOM_MAX * 100), now: Math.round(zoom * 100), text: percent }}
            accessibilityActions={[{ name: "increment" }, { name: "decrement" }]}
            onAccessibilityAction={onAction}
            style={styles.stepper}
          >
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Smaller text"
              disabled={zoom <= ZOOM_MIN}
              accessibilityState={{ disabled: zoom <= ZOOM_MIN }}
              onPress={() => change(zoomStep(zoom, -1))}
              hitSlop={6}
              style={({ pressed }) => [styles.step, zoom <= ZOOM_MIN && styles.off, pressed && styles.pressed]}
            >
              <Text style={styles.glyph}>−</Text>
            </Pressable>
            <Text style={styles.size}>{percent}</Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Larger text"
              disabled={zoom >= ZOOM_MAX}
              accessibilityState={{ disabled: zoom >= ZOOM_MAX }}
              onPress={() => change(zoomStep(zoom, 1))}
              hitSlop={6}
              style={({ pressed }) => [styles.step, zoom >= ZOOM_MAX && styles.off, pressed && styles.pressed]}
            >
              <Text style={styles.glyph}>+</Text>
            </Pressable>
          </View>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Reset text size"
            accessibilityHint="Back to 100%"
            disabled={zoom === 1}
            accessibilityState={{ disabled: zoom === 1 }}
            onPress={() => change(1)}
            style={({ pressed }) => [styles.reset, zoom === 1 && styles.off, pressed && styles.pressed]}
          >
            <Text style={styles.resetText}>Reset</Text>
          </Pressable>
        </View>
      ) : null}
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  // the menu's rows (the pane route's menuRow, menuText, menuTitle, menuHint)
  row: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 14, paddingVertical: 6 },
  rule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  pressed: { opacity: 0.7 },
  off: { opacity: 0.4 },
  text: { flex: 1 },
  title: { fontFamily: ui(400), color: colors.text, fontSize: 15, lineHeight: 20 },
  hint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  // under the row, lined up with its title: − size + then Reset
  controls: { flexDirection: "row", alignItems: "center", gap: 10, paddingLeft: 44, paddingRight: 14, paddingBottom: 10 },
  stepper: { flexDirection: "row", alignItems: "center", gap: 8 },
  // the terminal view's zoom buttons (32 pt round, the hit slop makes 44)
  step: { width: 32, height: 32, borderRadius: 16, borderWidth: 1, borderColor: colors.borderStrong, alignItems: "center", justifyContent: "center" },
  glyph: { fontFamily: ui(400), color: colors.textBright, fontSize: 16 },
  size: { fontFamily: ui(500), color: colors.textBright, fontSize: 14, minWidth: 44, textAlign: "center", fontVariant: ["tabular-nums"] },
  reset: { minHeight: TOUCH, justifyContent: "center", paddingHorizontal: 12, marginLeft: "auto" },
  resetText: { fontFamily: ui(500), color: colors.textBright, fontSize: 14 },
}));
