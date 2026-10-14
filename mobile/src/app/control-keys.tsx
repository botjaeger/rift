import { useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Animated, PanResponder, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { DEFAULT_CONTROL_KEYS, addChoices, controlKey, moveKeyTo, toggleKey } from "@/controlKeys";
import { fleet, useApp } from "@/runtime";
import { PageHeader, SectionTitle } from "@/ui/Screen";
import { colors, mono, radius, TOUCH, ui } from "@/ui/theme";

const ROW = 48;

/**
 * One key of the row: the ≡ handle drags it to a new place (one row per 48 pt); screen readers
 * get Move up / Move down actions instead.
 */
function KeyRow({
  id,
  index,
  count,
  onMove,
  onRemove,
  onDragging,
}: {
  id: string;
  index: number;
  count: number;
  onMove: (to: number) => void;
  onRemove: () => void;
  onDragging: (on: boolean) => void;
}) {
  const k = controlKey(id)!;
  const what = k.name ?? k.spoken;
  const [dy] = useState(() => new Animated.Value(0));
  const [dragging, setDragging] = useState(false);
  // the latest index and callbacks, for the responder made once (a box, not a ref: only the
  // responder's callbacks read it)
  const [live] = useState(() => ({ index, onMove, onDragging }));
  useEffect(() => {
    Object.assign(live, { index, onMove, onDragging });
  });
  const [pan] = useState(() =>
    PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onMoveShouldSetPanResponder: () => true,
      onPanResponderTerminationRequest: () => false,
      onPanResponderGrant: () => {
        setDragging(true);
        live.onDragging(true);
      },
      onPanResponderMove: Animated.event([null, { dy }], { useNativeDriver: false }),
      onPanResponderRelease: (_e, g) => {
        const by = Math.round(g.dy / ROW);
        dy.setValue(0);
        setDragging(false);
        live.onDragging(false);
        if (by !== 0) live.onMove(live.index + by);
      },
      onPanResponderTerminate: () => {
        dy.setValue(0);
        setDragging(false);
        live.onDragging(false);
      },
    }),
  );
  return (
    <Animated.View
      style={[
        styles.keyRow,
        index > 0 && styles.divider,
        index === 0 && styles.firstRow,
        index === count - 1 && styles.lastRow,
        dragging && styles.dragging,
        { transform: [{ translateY: dy }] },
      ]}
      accessible
      accessibilityLabel={`${what}, ${k.label}, position ${index + 1} of ${count}`}
      accessibilityActions={[
        ...(index > 0 ? [{ name: "moveUp", label: "Move up" }] : []),
        ...(index < count - 1 ? [{ name: "moveDown", label: "Move down" }] : []),
        { name: "remove", label: `Remove ${what}` },
      ]}
      onAccessibilityAction={(e) => {
        if (e.nativeEvent.actionName === "moveUp") onMove(index - 1);
        else if (e.nativeEvent.actionName === "moveDown") onMove(index + 1);
        else if (e.nativeEvent.actionName === "remove") onRemove();
      }}
    >
      <View style={styles.handle} {...pan.panHandlers}>
        <Text style={styles.handleText}>≡</Text>
      </View>
      <Text style={styles.cap}>{k.label}</Text>
      <Text style={styles.what} numberOfLines={1}>
        {what}
      </Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Remove ${what}`}
        onPress={onRemove}
        style={({ pressed }) => [styles.remove, pressed && styles.pressed]}
      >
        <Text style={styles.removeText}>−</Text>
      </Pressable>
    </Animated.View>
  );
}

/** Settings › Keys (the ControlKeys board): the row above the command line, its order, and more keys. */
export default function ControlKeysScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const ids = s.settings.controlKeys;
  const [dragging, setDragging] = useState(false);
  const set = (controlKeys: string[]) => void fleet.updateSettings({ controlKeys });
  const isDefault = ids.length === DEFAULT_CONTROL_KEYS.length && ids.every((id, i) => id === DEFAULT_CONTROL_KEYS[i]);
  const known = ids.filter((id) => controlKey(id));

  return (
    <View style={styles.fill}>
      <PageHeader
        title="Keys"
        back={() => (router.canGoBack() ? router.back() : router.replace("/settings"))}
        backLabel="Back to settings"
        right={
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Reset"
            accessibilityHint="Back to ^C, esc, tab, ctrl and the up and down arrows"
            accessibilityState={{ disabled: isDefault }}
            disabled={isDefault}
            onPress={() => set([...DEFAULT_CONTROL_KEYS])}
            style={({ pressed }) => [styles.reset, isDefault && styles.disabled, pressed && styles.pressed]}
          >
            <Text style={styles.resetText}>Reset</Text>
          </Pressable>
        }
      />
      <ScrollView
        scrollEnabled={!dragging}
        contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}
      >
        <View style={styles.section}>
          <SectionTitle>Preview</SectionTitle>
          <View style={styles.preview} accessible accessibilityLabel={`The row: ${known.map((id) => controlKey(id)!.spoken).join(", ") || "no keys"}`}>
            {known.length === 0 ? <Text style={styles.empty}>No keys: the row is hidden.</Text> : null}
            {known.map((id) => (
              <View key={id} style={styles.previewKey}>
                <Text style={styles.previewText}>{controlKey(id)!.label}</Text>
              </View>
            ))}
          </View>
        </View>

        {known.length > 0 ? (
          <View style={styles.section}>
            <SectionTitle>In the row · drag to reorder</SectionTitle>
            <View style={styles.card}>
              {known.map((id, i) => (
                <KeyRow
                  key={id}
                  id={id}
                  index={i}
                  count={known.length}
                  onDragging={setDragging}
                  onMove={(to) => set(moveKeyTo(known, i, to))}
                  onRemove={() => set(toggleKey(known, id, false))}
                />
              ))}
            </View>
          </View>
        ) : null}

        <View style={styles.section}>
          <SectionTitle>Add</SectionTitle>
          <View style={styles.add}>
            {addChoices(known).map((c) => (
              <Pressable
                key={c.label}
                accessibilityRole="button"
                accessibilityLabel={`Add ${c.ids.length > 1 ? "F1 to F12" : (controlKey(c.ids[0]!)?.spoken ?? c.label)}`}
                onPress={() => set(c.ids.reduce((row, id) => toggleKey(row, id, true), known))}
                style={({ pressed }) => [styles.addChip, pressed && styles.pressed]}
              >
                {/* +tab / +pane already start with one */}
                <Text style={styles.addText}>{c.label.startsWith("+") ? c.label : `+ ${c.label}`}</Text>
              </Pressable>
            ))}
          </View>
        </View>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.chrome },
  content: { paddingTop: 12, paddingHorizontal: 12, gap: 22, maxWidth: 720, width: "100%", alignSelf: "center" },
  section: { gap: 6 },
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  reset: { minHeight: TOUCH, paddingHorizontal: 12, justifyContent: "center" },
  resetText: { fontFamily: ui(400), color: colors.accent, fontSize: 15 },
  preview: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
    padding: 10,
    marginTop: 2,
    borderRadius: radius.card,
    backgroundColor: colors.bg,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  previewKey: {
    minWidth: 40,
    height: 36,
    paddingHorizontal: 8,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
  },
  previewText: { color: colors.text, fontFamily: mono, fontSize: 12 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, paddingVertical: 8 },
  card: {
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  keyRow: {
    minHeight: ROW,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingLeft: 14,
    paddingRight: 6,
    backgroundColor: colors.surface,
  },
  // the card does not clip (a dragged row's shadow), so the end rows round themselves
  firstRow: { borderTopLeftRadius: radius.card - 1, borderTopRightRadius: radius.card - 1 },
  lastRow: { borderBottomLeftRadius: radius.card - 1, borderBottomRightRadius: radius.card - 1 },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  dragging: { zIndex: 1, elevation: 4, shadowColor: "#000", shadowOpacity: 0.4, shadowRadius: 8, shadowOffset: { width: 0, height: 4 } },
  handle: { width: 24, height: ROW, justifyContent: "center", marginLeft: -6, paddingLeft: 6 },
  handleText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 16 },
  cap: { width: 44, color: colors.textBright, fontFamily: mono, fontSize: 13 },
  what: { fontFamily: ui(400), flex: 1, color: colors.textMuted, fontSize: 13 },
  remove: { width: TOUCH, height: TOUCH, alignItems: "center", justifyContent: "center" },
  removeText: { fontFamily: ui(400), color: colors.warning, fontSize: 18 },
  add: { flexDirection: "row", flexWrap: "wrap", gap: 6, paddingHorizontal: 4 },
  addChip: {
    minWidth: TOUCH,
    height: 40,
    paddingHorizontal: 10,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.control,
    borderWidth: 1,
    borderStyle: "dashed",
    borderColor: colors.borderStrong,
  },
  addText: { color: colors.text, fontFamily: mono, fontSize: 12 },
});
