import { useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Animated, PanResponder, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { DEFAULT_CONTROL_KEYS, MAX_ROW_KEYS, addChoices, canAddKey, controlKey, keyName, moveKeyTo, toggleKey } from "@/controlKeys";
import { fleet, useApp } from "@/runtime";
import { Key, KeyCap, KeyTip, tipOf, type Tip } from "@/ui/ControlRow";
import { Icon } from "@/ui/Icon";
import { SaveError } from "@/ui/SaveError";
import { PageHeader, SectionTitle } from "@/ui/Screen";
import { colors, monoMedium, radius, TOUCH, ui } from "@/ui/theme";

const ROW = 52;

/**
 * One key of the row: the grip handle drags it to a new place (one row per 48 pt); screen readers
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
  const what = keyName(k);
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
        <Icon name="grip" size={16} weight="medium" color={colors.textMuted} />
      </View>
      <KeyCap def={k} color={k.danger ? colors.warning : colors.text} ground={colors.bg} />
      <Text style={styles.what} numberOfLines={1}>
        {what}
        {k.icon && k.danger ? <Text style={styles.chord}> {k.label}</Text> : null}
      </Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Remove ${what}`}
        onPress={onRemove}
        style={({ pressed }) => [styles.remove, pressed && styles.pressed]}
      >
        <Icon name="remove" size={20} weight="medium" color={colors.warning} />
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
  const [tip, setTip] = useState<Tip | null>(null);
  const set = (controlKeys: string[]) => void fleet.updateSettings({ controlKeys });
  const isDefault = ids.length === DEFAULT_CONTROL_KEYS.length && ids.every((id, i) => id === DEFAULT_CONTROL_KEYS[i]);
  const known = ids.filter((id) => controlKey(id));

  return (
    <View style={styles.fill}>
      <PageHeader
        title="Keys"
        back={() => (router.canGoBack() ? router.back() : router.replace("/settings"))}
        // reached from Settings and from the pane's ⌨ key alike
        backLabel="Back"
        right={
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Reset"
            accessibilityHint="Back to Escape, Tab, Interrupt, Control and the arrows"
            accessibilityState={{ disabled: isDefault }}
            disabled={isDefault}
            onPress={() => set([...DEFAULT_CONTROL_KEYS])}
            style={({ pressed }) => [styles.reset, isDefault && styles.disabled, pressed && styles.pressed]}
          >
            <Text style={styles.resetText}>Reset</Text>
          </Pressable>
        }
      />
      <SaveError />
      <ScrollView
        scrollEnabled={!dragging}
        contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}
      >
        <View style={styles.section}>
          <SectionTitle>Preview</SectionTitle>
          <View style={styles.previewHost}>
            {tip ? <KeyTip tip={tip} below /> : null}
            <View style={styles.preview}>
              {known.length === 0 ? <Text style={styles.empty}>No keys: the row is hidden while the keyboard is down.</Text> : null}
              {known.map((id) => {
                const k = controlKey(id)!;
                return (
                  <Key
                    key={id}
                    def={k}
                    spoken={k.spoken}
                    tip={tipOf(k)}
                    onTip={setTip}
                    color={k.danger ? colors.warning : colors.text}
                    disabled={false}
                    onPress={() => undefined}
                  />
                );
              })}
              {/* the last key of the row, while the keyboard is up */}
              <Key
                def={{ icon: "hideKeyboard", label: "⌨" }}
                spoken="Hide keyboard"
                tip={{ title: "Hide keyboard", text: "Closes the keyboard; tap the command field to bring it back" }}
                onTip={setTip}
                color={colors.textMuted}
                disabled={false}
                onPress={() => undefined}
              />
            </View>
            <Text style={styles.previewHint}>Hold a key to see what it sends.</Text>
          </View>
        </View>

        {known.length > 0 ? (
          <View style={styles.section}>
            <SectionTitle>{`In the row · ${known.length} of ${MAX_ROW_KEYS} · drag to reorder`}</SectionTitle>
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
          <SectionTitle>{canAddKey(known) ? "Add" : `Add · the row holds ${MAX_ROW_KEYS} keys: remove one first`}</SectionTitle>
          <View style={styles.add}>
            {addChoices(known).map((c) => {
              const k = c.ids.length === 1 ? controlKey(c.ids[0]!) : undefined;
              // the key as the row shows it, then its word ("⌃D End input"); the F-keys as one chip
              const word = k ? (k.short ?? keyName(k)) : c.label;
              const full = !canAddKey(known);
              return (
                <Pressable
                  key={c.label}
                  accessibilityRole="button"
                  accessibilityLabel={`Add ${k?.spoken ?? c.label}`}
                  accessibilityState={{ disabled: full }}
                  disabled={full}
                  onPress={() => set(c.ids.reduce((row, id) => toggleKey(row, id, true), known))}
                  hitSlop={4}
                  style={({ pressed }) => [styles.addChip, full && styles.disabled, pressed && styles.pressed]}
                >
                  {/* the key as the row draws it: its icon, or its label in mono (⇧Tab leads with the shift icon) */}
                  {k?.icon ? (
                    <Icon name={k.icon} size={14} weight="medium" color={colors.text} />
                  ) : k && k.label !== word ? (
                    <>
                      {k.lead ? <Icon name={k.lead} size={12} weight="medium" color={colors.text} /> : null}
                      <Text style={styles.addKey}>{k.lead ? [...k.label].slice(1).join("") : k.label}</Text>
                    </>
                  ) : null}
                  <Text style={styles.addText}>{word}</Text>
                </Pressable>
              );
            })}
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
  // the board's Reset: a glass pill
  reset: { height: 40, paddingHorizontal: 16, borderRadius: 20, backgroundColor: colors.glass, justifyContent: "center" },
  resetText: { fontFamily: ui(400), color: colors.accent, fontSize: 15 },
  previewHost: { gap: 6, zIndex: 2 },
  previewHint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, paddingHorizontal: 4 },
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
  what: { fontFamily: ui(400), flex: 1, color: colors.textBright, fontSize: 14 },
  chord: { fontFamily: monoMedium, color: colors.textMuted, fontSize: 13 },
  remove: { width: TOUCH, height: TOUCH, alignItems: "center", justifyContent: "center" },
  add: { flexDirection: "row", flexWrap: "wrap", gap: 6, paddingHorizontal: 4 },
  // the board's Add chips: dashed 36 pt pills, the key then its word
  addChip: {
    height: 36,
    flexDirection: "row",
    gap: 6,
    paddingLeft: 10,
    paddingRight: 12,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 18,
    borderWidth: 1,
    borderStyle: "dashed",
    borderColor: colors.borderStrong,
  },
  addKey: { color: colors.text, fontFamily: monoMedium, fontSize: 13 },
  addText: { color: colors.text, fontFamily: ui(400), fontSize: 13 },
});
