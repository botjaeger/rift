import { Pressable, ScrollView, StyleSheet, Text } from "react-native";
import { controlKey, type ControlKey } from "@/controlKeys";
import { colors, mono, radius, TOUCH } from "./theme";

/** Sticky modifiers from the row, applied to the next key or letter. */
export interface Sticky {
  ctrl: boolean;
  alt: boolean;
}

export const NO_STICKY: Sticky = { ctrl: false, alt: false };

function Key({
  def,
  onPress,
  active,
  disabled,
}: {
  def: ControlKey;
  onPress: () => void;
  active?: boolean;
  disabled: boolean;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={def.spoken}
      accessibilityState={{ disabled, ...(active === undefined ? {} : { selected: active }) }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.key,
        def.danger && styles.keyDanger,
        active && styles.keyActive,
        disabled && styles.disabled,
        pressed && styles.pressed,
      ]}
    >
      <Text style={[styles.keyLabel, active && { color: colors.bg }]}>{def.label}</Text>
    </Pressable>
  );
}

/**
 * The control-key row (Settings › Control keys): `onKey` gets a key's protocol name, the
 * sticky ctrl / alt toggle `sticky`. Nothing when the row is empty.
 */
export function ControlRow({
  ids,
  sticky,
  disabled,
  onSticky,
  onKey,
}: {
  ids: readonly string[];
  sticky: Sticky;
  disabled: boolean;
  onSticky: (next: Sticky) => void;
  onKey: (key: string) => void;
}) {
  if (ids.length === 0) return null;
  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.keys}
      keyboardShouldPersistTaps="always"
    >
      {ids.map((id) => {
        const def = controlKey(id);
        if (!def) return null;
        if (def.sticky) {
          const mod = id === "alt" ? "alt" : "ctrl";
          return (
            <Key
              key={id}
              def={def}
              disabled={disabled}
              active={sticky[mod]}
              onPress={() => onSticky({ ...sticky, [mod]: !sticky[mod] })}
            />
          );
        }
        return <Key key={id} def={def} disabled={disabled} onPress={() => onKey(id)} />;
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  // Eight keys share the row (≥ 44 pt each) and fit a 390 pt phone; more keys, or narrower
  // screens, scroll the row.
  keys: { gap: 3, flexGrow: 1 },
  key: {
    flexGrow: 1,
    minWidth: TOUCH,
    minHeight: TOUCH,
    paddingHorizontal: 4,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.control,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  // Interrupt is marked by a red border: red text on a surface is below 4.5:1 (palette.ts).
  keyDanger: { borderColor: colors.error },
  keyActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  keyLabel: { color: colors.text, fontFamily: mono, fontSize: 14 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
