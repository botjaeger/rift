import { useRouter } from "expo-router";
import { useState } from "react";
import { KeyboardAvoidingView, Platform, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { BLOCK_SLOTS, DEFAULT_CONTROL_KEYS, addChoices, canAddKey, controlKey, keyName, moveKeyTo, toggleKey } from "@/controlKeys";
import { fleet, useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { ControlRow, KeyCap, NO_STICKY } from "@/ui/ControlRow";
import { Field } from "@/ui/Field";
import { Icon } from "@/ui/Icon";
import { ReorderRow, useReorder, type RowHandle } from "@/ui/ReorderRow";
import { SaveError } from "@/ui/SaveError";
import { PageHeader, SectionTitle } from "@/ui/Screen";
import { monoMedium, radius, TOUCH, ui, makeStyles, useColors } from "@/ui/theme";

/** A list row's height: the distance one row's drag moves it. */
const ROW = 52;

/**
 * One key of the block: the grip drags it to a new place (ReorderRow); screen readers get Move up
 * / Move down and Remove actions instead. Remove is the red − at its end.
 */
function KeyRow({ id, index, count, handle, onRemove }: { id: string; index: number; count: number; handle: RowHandle; onRemove: () => void }) {
  const colors = useColors();
  const styles = useStyles();
  const k = controlKey(id)!;
  const what = keyName(k);
  const { grip, accessibilityActions, onAccessibilityAction } = handle;
  return (
    <View style={[styles.keyRow, index > 0 && styles.divider]}>
      {grip(
        <View style={styles.handle} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
          <Icon name="grip" size={16} weight="medium" color={colors.textMuted} />
        </View>,
      )}
      <View
        style={styles.keyBody}
        accessible
        accessibilityLabel={`${what}, ${k.label}, position ${index + 1} of ${count}`}
        accessibilityActions={[...accessibilityActions, { name: "remove", label: `Remove ${what}` }]}
        onAccessibilityAction={(e) => {
          if (e.nativeEvent.actionName === "remove") onRemove();
          else onAccessibilityAction(e);
        }}
      >
        <KeyCap def={k} color={k.danger ? colors.error : colors.text} ground={colors.bg} style={styles.cap} />
        <Text style={styles.what} numberOfLines={1}>
          {what}
          {k.icon && k.danger ? <Text style={styles.chord}> {k.label}</Text> : null}
        </Text>
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Remove ${what}`}
        onPress={onRemove}
        style={({ pressed }) => [styles.remove, pressed && styles.pressed]}
      >
        <Icon name="remove" size={22} weight="medium" color={colors.error} />
      </Pressable>
    </View>
  );
}

/**
 * Settings › Keys (the New Settings board): a preview of the key block and arrow pad, the block's
 * keys in their order (drag ≡ to reorder, − in red removes), and Add a key with a search. The
 * keyboard key is always the block's last and the arrows are always on the pad: neither is listed.
 */
export default function ControlKeysScreen() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const ids = s.settings.controlKeys;
  const reorder = useReorder();
  const [query, setQuery] = useState("");
  const set = (controlKeys: string[]) => void fleet.updateSettings({ controlKeys });
  const isDefault = ids.length === DEFAULT_CONTROL_KEYS.length && ids.every((id, i) => id === DEFAULT_CONTROL_KEYS[i]);
  const known = ids.filter((id) => controlKey(id));
  const full = !canAddKey(known);
  const choices = addChoices(known, query);

  // Android 15 draws edge to edge: the padding leaves room to scroll Search keys above the keyboard
  return (
    <KeyboardAvoidingView style={styles.fill} behavior="padding" enabled={Platform.OS === "android"}>
      <PageHeader
        title="Keys"
        // the keyboard key is one of the block's places
        subtitle={`${known.length + 1} of ${BLOCK_SLOTS} in the key block`}
        back={() => (router.canGoBack() ? router.back() : router.replace("/settings"))}
        // reached from Settings and from the command line's ⌨ held
        backLabel="Back"
        right={
          <Button
            label="Reset"
            variant="text"
            size="small"
            accessibilityHint="Back to Escape, Tab, Control, Return and Paste"
            disabled={isDefault}
            onPress={() => set([...DEFAULT_CONTROL_KEYS])}
          />
        }
      />
      <SaveError />
      <ScrollView
        scrollEnabled={reorder.scrollEnabled}
        keyboardShouldPersistTaps="handled"
        // iOS: the keyboard insets the page and scrolls Search keys (at its foot) above it
        automaticallyAdjustKeyboardInsets
        contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}
      >
        <View style={styles.section}>
          <SectionTitle>Preview</SectionTitle>
          <View style={styles.preview}>
            <ControlRow
              ids={known}
              sticky={NO_STICKY}
              disabled={false}
              onSticky={() => undefined}
              onKey={() => undefined}
              onOpen={() => undefined}
              onPaste={() => undefined}
              onShowKeyboard={() => undefined}
              tipBelow
            />
          </View>
          <Text style={styles.previewHint}>Hold a key to see what it sends.</Text>
        </View>

        {known.length > 0 ? (
          <View style={styles.section}>
            <SectionTitle>In the key block · drag ≡ to reorder</SectionTitle>
            <View style={styles.card} accessibilityRole="list" accessibilityLabel="Keys in the key block">
              {known.map((id, i) => (
                <ReorderRow
                  key={id}
                  grip
                  index={i}
                  count={known.length}
                  rowH={ROW}
                  enabled={known.length > 1}
                  drag={reorder.drag}
                  setDrag={reorder.setDrag}
                  onMove={(from, to) => set(moveKeyTo(known, from, to))}
                >
                  {(handle) => <KeyRow id={id} index={i} count={known.length} handle={handle} onRemove={() => set(toggleKey(known, id, false))} />}
                </ReorderRow>
              ))}
            </View>
          </View>
        ) : null}

        <View style={styles.section}>
          <SectionTitle>{full ? `Add a key · the block holds ${BLOCK_SLOTS}: remove one first` : "Add a key"}</SectionTitle>
          <Field icon="search" value={query} onChangeText={setQuery} label="Search keys" placeholder="Search keys" autoCapitalize="none" autoCorrect={false} returnKeyType="search" />
          <View style={styles.add}>
            {choices.length === 0 ? <Text style={styles.empty}>No key matches “{query.trim()}”.</Text> : null}
            {choices.map((c) => {
              const k = controlKey(c.ids[0]!)!;
              // the key as the block shows it, then its word ("⌃D End input")
              const word = k.short ?? keyName(k);
              return (
                <Pressable
                  key={c.label}
                  accessibilityRole="button"
                  accessibilityLabel={`Add ${k.spoken}`}
                  accessibilityState={{ disabled: full }}
                  disabled={full}
                  onPress={() => set(toggleKey(known, k.id, true))}
                  hitSlop={4}
                  style={({ pressed }) => [styles.addChip, full && styles.disabled, pressed && styles.pressed]}
                >
                  {/* the key as the block draws it: its icon, or its label in mono (⇧Tab leads with the shift icon) */}
                  {k.icon && !k.paste ? (
                    <Icon name={k.icon} size={14} weight="medium" color={colors.textBright} />
                  ) : k.label !== word ? (
                    <>
                      {k.lead ? <Icon name={k.lead} size={12} weight="medium" color={colors.textBright} /> : null}
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
    </KeyboardAvoidingView>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.chrome },
  content: { paddingTop: 12, paddingHorizontal: 12, gap: 22, maxWidth: 720, width: "100%", alignSelf: "center" },
  section: { gap: 6 },
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  previewHint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, paddingHorizontal: 4 },
  // the board's preview: the keys as the pane draws them, on the chrome ground
  preview: {
    zIndex: 2,
    paddingVertical: 10,
    paddingHorizontal: 12,
    marginTop: 2,
    borderRadius: radius.card,
    backgroundColor: colors.chrome,
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
    paddingLeft: 10,
    paddingRight: 4,
    borderRadius: radius.card - 1,
    backgroundColor: colors.surface,
  },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline, borderTopLeftRadius: 0, borderTopRightRadius: 0 },
  handle: { width: 28, height: ROW, alignItems: "center", justifyContent: "center" },
  keyBody: { flex: 1, minWidth: 0, flexDirection: "row", alignItems: "center", gap: 12 },
  // the board's list key: 44 x 34
  cap: { width: 44, height: 34 },
  what: { fontFamily: ui(400), flex: 1, color: colors.text, fontSize: 15 },
  chord: { fontFamily: monoMedium, color: colors.textMuted, fontSize: 13 },
  remove: { width: TOUCH, height: TOUCH, alignItems: "center", justifyContent: "center" },
  add: { flexDirection: "row", flexWrap: "wrap", gap: 8, paddingTop: 6 },
  // the board's Add chips: 36 pt, 8 pt corners, the key in mono then its word
  addChip: {
    height: 36,
    flexDirection: "row",
    gap: 6,
    paddingHorizontal: 12,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 8,
    borderWidth: 1,
    borderColor: colors.glassRim,
    backgroundColor: colors.surface,
  },
  addKey: { color: colors.textBright, fontFamily: monoMedium, fontSize: 13 },
  addText: { color: colors.text, fontFamily: ui(400), fontSize: 13 },
}));
