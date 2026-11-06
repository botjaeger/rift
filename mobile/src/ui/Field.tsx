import type { ReactNode } from "react";
import { Pressable, TextInput, View, type StyleProp, type TextInputProps, type ViewStyle } from "react-native";
import { Icon, type IconName } from "./Icon";
import { radius, TOUCH, ui, makeStyles, useColors } from "./theme";

/**
 * The one text field (the Standard board): 44 pt, radius 10, on the surface, a leading icon,
 * words in Inter (the placeholder included) and a clear button while it holds text. Search tabs,
 * Search folders, Search output and the pairing link use it; the command line is its own control.
 * Every other TextInput prop passes through.
 */
export function Field({
  icon,
  value,
  onChangeText,
  label,
  trailing,
  style,
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
  return (
    <View style={[styles.field, style]}>
      <Icon name={icon} size={16} color={colors.textMuted} />
      <TextInput
        keyboardAppearance={colors.dark ? "dark" : "light"}
        placeholderTextColor={colors.textMuted}
        {...input}
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
  field: {
    height: TOUCH,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingLeft: 12,
    borderRadius: radius.field,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.surface,
  },
  input: { flex: 1, minWidth: 0, height: TOUCH - 2, paddingVertical: 0, paddingRight: 12, fontFamily: ui(400), color: colors.textBright, fontSize: 16 },
  // the 16 pt round clear mark in a 44 pt target, at the field's end
  clear: { width: 40, height: TOUCH - 2, alignItems: "center", justifyContent: "center", marginLeft: -12 },
  clearDot: { width: 16, height: 16, borderRadius: 8, backgroundColor: colors.textMuted, alignItems: "center", justifyContent: "center" },
  pressed: { opacity: 0.6 },
}));
