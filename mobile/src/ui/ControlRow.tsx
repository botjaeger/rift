import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { controlKey, type ControlKey } from "@/controlKeys";
import type { OpenKind } from "@/protocol/types";
import { colors, mono, radius } from "./theme";
import { toggleA11y } from "./toggle";

/** Sticky modifiers from the row, applied to the next key or letter. */
export interface Sticky {
  ctrl: boolean;
  alt: boolean;
}

export const NO_STICKY: Sticky = { ctrl: false, alt: false };

function Key({
  label,
  spoken,
  onPress,
  active,
  disabled,
  color = colors.text,
  tall,
  hint,
}: {
  label: string;
  spoken: string;
  onPress: () => void;
  active?: boolean;
  disabled: boolean;
  color?: string;
  tall: boolean;
  hint?: string;
}) {
  return (
    <Pressable
      // ctrl and alt stay on for the next key: toggle buttons, announced as on or off
      {...(active === undefined ? { accessibilityRole: "button" as const, accessibilityState: { disabled } } : toggleA11y(active, disabled))}
      accessibilityLabel={spoken}
      accessibilityHint={hint}
      disabled={disabled}
      onPress={onPress}
      // 36 / 40 pt keys (the boards) reach 44 pt with the slop
      hitSlop={{ top: tall ? 2 : 4, bottom: tall ? 2 : 4 }}
      style={({ pressed }) => [
        styles.key,
        tall && styles.keyTall,
        active && styles.keyActive,
        disabled && styles.disabled,
        pressed && styles.pressed,
      ]}
    >
      <Text style={[styles.keyLabel, { color: active ? colors.bg : color }]}>{label}</Text>
    </Pressable>
  );
}

/**
 * The control-key row (Settings › Keys): `onKey` gets a key's protocol name, the sticky ctrl /
 * alt toggle `sticky`, the new tab / new pane keys `onOpen` (disabled without it: the computer's
 * Rift cannot open them). Above the command line it is one line that scrolls, ending in "⌨"
 * (`onEdit`, Settings › Keys); under a full-screen program it wraps (`wrap`) with taller keys.
 * Nothing when the row is empty and there is no edit key.
 */
export function ControlRow({
  ids,
  sticky,
  disabled,
  onSticky,
  onKey,
  onOpen,
  onEdit,
  wrap,
}: {
  ids: readonly string[];
  sticky: Sticky;
  disabled: boolean;
  onSticky: (next: Sticky) => void;
  onKey: (key: string) => void;
  onOpen?: (kind: OpenKind) => void;
  onEdit?: () => void;
  wrap?: boolean;
}) {
  if (ids.length === 0 && !onEdit) return null;
  const keys = ids.map((id) => {
    const def: ControlKey | undefined = controlKey(id);
    if (!def) return null;
    // Interrupt in the caution colour (red text is below 4.5:1 on a surface, palette.ts)
    const color = def.danger ? colors.warning : colors.text;
    if (def.sticky) {
      const mod = id === "alt" ? "alt" : "ctrl";
      return (
        <Key
          key={id}
          label={def.label}
          spoken={def.spoken}
          color={color}
          tall={!!wrap}
          disabled={disabled}
          active={sticky[mod]}
          onPress={() => onSticky({ ...sticky, [mod]: !sticky[mod] })}
        />
      );
    }
    if (def.open) {
      const kind = def.open;
      return (
        <Key
          key={id}
          label={def.label}
          spoken={def.spoken}
          hint={onOpen ? undefined : "Update Rift on the computer to open tabs and panes from here"}
          color={color}
          tall={!!wrap}
          disabled={disabled || !onOpen}
          onPress={() => onOpen?.(kind)}
        />
      );
    }
    return (
      <Key key={id} label={def.label} spoken={def.spoken} color={color} tall={!!wrap} disabled={disabled} onPress={() => onKey(id)} />
    );
  });
  const edit = onEdit ? (
    <Key
      key="edit"
      label="⌨"
      spoken="Edit keys"
      hint="Settings › Keys: choose and order these keys"
      color={colors.textMuted}
      tall={!!wrap}
      disabled={false}
      onPress={onEdit}
    />
  ) : null;
  if (wrap) {
    return (
      <View style={[styles.keys, styles.wrap]} accessibilityRole="toolbar" accessibilityLabel="Keys">
        {keys}
        {edit}
      </View>
    );
  }
  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.keys}
      keyboardShouldPersistTaps="always"
      accessibilityRole="toolbar"
      accessibilityLabel="Keys"
    >
      {keys}
      {edit}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  keys: { gap: 6 },
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  key: {
    minWidth: 44,
    height: 36,
    paddingHorizontal: 10,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.control,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  keyTall: { height: 40 },
  keyActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  keyLabel: { fontFamily: mono, fontSize: 12 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
