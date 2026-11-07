import { useState, type ReactNode } from "react";
import { Pressable, TextInput, View, type StyleProp, type TextInputProps, type ViewStyle } from "react-native";
import { Icon, type IconName } from "./Icon";
import { radius, ui, makeStyles, useColors } from "./theme";

/** The field's height: the boards' 44 pt box and its 1 pt rim on each side. */
export const FIELD_HEIGHT = 46;

/**
 * The one text field (the Standard board): 46 pt, radius 10, on the surface, a leading icon,
 * words in Inter (the placeholder included), its rim in the accent while it has focus, and a
 * round clear mark while it holds text. Search tabs, Search folders, Search output and the
 * pairing link use it; the command line is its own control. Every other TextInput prop passes
 * through.
 */
export function Field({
  icon,
  value,
  onChangeText,
  label,
  trailing,
  style,
  onFocus,
  onBlur,
  ...input
}: Omit<TextInputProps, "style" | "value" | "onChangeText" | "accessibilityLabel"> & {
  icon: IconName;
  value: string;
  onChangeText: (text: string) => void;
  /** What a screen reader calls the field ("Search tabs"). */
  label: string;
  /** After the text, before the clear button (Search output's "2 of 9"). */
  trailing?: ReactNode;
  style?: StyleProp<ViewStyle>;
}) {
  const colors = useColors();
  const styles = useStyles();
  const [focused, setFocused] = useState(false);
  return (
    <View style={[styles.field, focused && styles.focused, style]}>
      <Icon name={icon} size={16} color={colors.textMuted} />
      <TextInput
        keyboardAppearance={colors.dark ? "dark" : "light"}
        placeholderTextColor={colors.placeholder}
        {...input}
        onFocus={(e) => {
          setFocused(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocused(false);
          onBlur?.(e);
        }}
        value={value}
        onChangeText={onChangeText}
        accessibilityLabel={label}
        style={styles.input}
      />
      {trailing}
      {value !== "" ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Clear"
          onPress={() => onChangeText("")}
          // the 28 pt mark, 44 to the touch
          hitSlop={8}
          style={({ pressed }) => [styles.clear, pressed && styles.pressed]}
        >
          <View style={styles.clearDot}>
            <Icon name="close" size={10} color={colors.surface} />
          </View>
        </Pressable>
      ) : null}
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  // the boards: 12 pt before the icon, 10 between, 8 after the clear mark
  field: {
    height: FIELD_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingLeft: 12,
    paddingRight: 8,
    borderRadius: radius.field,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.surface,
  },
  focused: { borderColor: colors.accent },
  input: { flex: 1, minWidth: 0, height: FIELD_HEIGHT - 2, paddingVertical: 0, fontFamily: ui(400), color: colors.textBright, fontSize: 16 },
  // the 16 pt round mark in a 28 pt button at the field's end
  clear: { width: 28, height: 28, alignItems: "center", justifyContent: "center" },
  clearDot: { width: 16, height: 16, borderRadius: 8, backgroundColor: colors.textMuted, alignItems: "center", justifyContent: "center" },
  pressed: { opacity: 0.6 },
}));
