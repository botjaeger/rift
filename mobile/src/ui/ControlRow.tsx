import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { controlKey, type ControlKey } from "@/controlKeys";
import type { OpenKind } from "@/protocol/types";
import { Icon, type IconName } from "./Icon";
import { colors, ui } from "./theme";
import { toggleA11y } from "./toggle";

/** Sticky modifiers from the row, applied to the next key or letter. */
export interface Sticky {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

export const NO_STICKY: Sticky = { ctrl: false, alt: false, shift: false };

/** A sticky key's modifier ("ctrl", "alt", "shift"). */
const modOf = (id: string): keyof Sticky => (id === "alt" ? "alt" : id === "shift" ? "shift" : "ctrl");

/**
 * The boards' round key: 36 pt (40 under a full-screen program), its icon, or its label in
 * Inter; a word ("Esc", "Paste") makes a pill. The accent ground marks a sticky key that is on.
 */
export function KeyCap({
  def,
  color = colors.text,
  tall,
  active,
  ground = colors.surface,
}: {
  def: Pick<ControlKey, "label" | "icon">;
  color?: string;
  tall?: boolean;
  active?: boolean;
  /** The key's ground (Settings › Keys draws its rows' keys on the page). */
  ground?: string;
}) {
  const fg = active ? colors.bg : color;
  const word = !def.icon && [...def.label].length > 2;
  return (
    <View
      style={[
        styles.key,
        { backgroundColor: ground },
        tall && styles.keyTall,
        word && styles.keyWord,
        active && styles.keyActive,
      ]}
    >
      {def.icon ? (
        <Icon name={def.icon} size={17} color={fg} />
      ) : (
        <Text style={[word ? styles.wordLabel : styles.glyphLabel, { color: fg }]}>{def.label}</Text>
      )}
    </View>
  );
}

function Key({
  def,
  spoken,
  onPress,
  active,
  disabled,
  color,
  tall,
  hint,
}: {
  def: Pick<ControlKey, "label" | "icon">;
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
      // the modifiers stay on for the next key: toggle buttons, announced as on or off
      {...(active === undefined ? { accessibilityRole: "button" as const, accessibilityState: { disabled } } : toggleA11y(active, disabled))}
      accessibilityLabel={spoken}
      accessibilityHint={hint}
      disabled={disabled}
      onPress={onPress}
      // 36 / 40 pt keys (the boards) reach 44 pt with the slop
      hitSlop={{ top: tall ? 2 : 4, bottom: tall ? 2 : 4 }}
      style={({ pressed }) => [disabled && styles.disabled, pressed && styles.pressed]}
    >
      <KeyCap def={def} color={color} tall={tall} active={active} />
    </Pressable>
  );
}

const icon = (name: IconName, label: string) => ({ icon: name, label });

/**
 * The control-key row (Settings › Keys): `onKey` gets a key's protocol name, the sticky ctrl /
 * alt / shift toggle `sticky`, the new tab / new pane keys `onOpen` (disabled without it: the
 * computer's Rift cannot open them), Paste `onPaste` (disabled without it: the computer's Rift
 * takes no text). Above the command line it is one line that scrolls, led by Hide keyboard
 * (`onHideKeyboard`, while the keyboard is up) and ending in Edit keys (`onEdit`, Settings ›
 * Keys); under a full-screen program it wraps (`wrap`) with taller keys. Nothing when the row
 * is empty and there is no edit key.
 */
export function ControlRow({
  ids,
  sticky,
  disabled,
  onSticky,
  onKey,
  onOpen,
  onPaste,
  onEdit,
  onHideKeyboard,
  wrap,
}: {
  ids: readonly string[];
  sticky: Sticky;
  disabled: boolean;
  onSticky: (next: Sticky) => void;
  onKey: (key: string) => void;
  onOpen?: (kind: OpenKind) => void;
  onPaste?: () => void;
  onEdit?: () => void;
  onHideKeyboard?: () => void;
  wrap?: boolean;
}) {
  if (ids.length === 0 && !onEdit) return null;
  const tall = !!wrap;
  const keys = ids.map((id) => {
    const def: ControlKey | undefined = controlKey(id);
    if (!def) return null;
    // Interrupt in the caution colour (red is below 4.5:1 on a surface, palette.ts)
    const color = def.danger ? colors.warning : colors.text;
    if (def.sticky) {
      const mod = modOf(id);
      return (
        <Key
          key={id}
          def={def}
          spoken={def.spoken}
          color={color}
          tall={tall}
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
          def={def}
          spoken={def.spoken}
          hint={onOpen ? undefined : "Update Rift on the computer to open tabs and panes from here"}
          color={color}
          tall={tall}
          disabled={disabled || !onOpen}
          onPress={() => onOpen?.(kind)}
        />
      );
    }
    if (def.paste) {
      return (
        <Key
          key={id}
          def={def}
          spoken={def.spoken}
          hint={onPaste ? "Types it into the program" : "Update Rift on the computer to paste here"}
          color={color}
          tall={tall}
          disabled={disabled || !onPaste}
          onPress={() => onPaste?.()}
        />
      );
    }
    return <Key key={id} def={def} spoken={def.spoken} color={color} tall={tall} disabled={disabled} onPress={() => onKey(id)} />;
  });
  const hide = onHideKeyboard ? (
    <Key key="hide" def={icon("hideKeyboard", "⌨")} spoken="Hide keyboard" color={colors.textMuted} tall={tall} disabled={false} onPress={onHideKeyboard} />
  ) : null;
  const edit = onEdit ? (
    <Key
      key="edit"
      def={icon("editKeys", "⌨")}
      spoken="Edit keys"
      hint="Settings › Keys: choose and order these keys"
      color={colors.textMuted}
      tall={tall}
      disabled={false}
      onPress={onEdit}
    />
  ) : null;
  if (wrap) {
    return (
      <View style={[styles.keys, styles.wrap]} accessibilityRole="toolbar" accessibilityLabel="Keys">
        {hide}
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
      {hide}
      {keys}
      {edit}
    </ScrollView>
  );
}

// The Keys and Pane boards' keys: 36 pt circles (40 under a full-screen program), a word key a
// pill 14 pt in, on the surface with a hairline; the accent ground for a modifier that is on.
const styles = StyleSheet.create({
  keys: { gap: 6 },
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  key: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  keyTall: { width: 40, height: 40, borderRadius: 20 },
  keyWord: { width: undefined, minWidth: 36, paddingHorizontal: 14 },
  keyActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  glyphLabel: { fontFamily: ui(400), fontSize: 16 },
  wordLabel: { fontFamily: ui(500), fontSize: 14 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
