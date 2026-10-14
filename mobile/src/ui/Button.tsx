import type { ReactNode } from "react";
import { Pressable, StyleSheet, Text, type StyleProp, type ViewStyle } from "react-native";
import { colors, radius, TOUCH, ui } from "./theme";

interface Props {
  label: string;
  onPress: () => void;
  /** primary: accent fill; secondary: outlined; link: accent text only (Paste a pairing link). */
  variant?: "primary" | "secondary" | "link";
  /** large: 50 pt (page actions); medium: 44 pt; small: 40 pt (inside a card). */
  size?: "large" | "medium" | "small";
  icon?: ReactNode;
  disabled?: boolean;
  accessibilityHint?: string;
  style?: StyleProp<ViewStyle>;
}

export function Button({ label, onPress, variant = "secondary", size = "medium", icon, disabled, accessibilityHint, style }: Props) {
  const primary = variant === "primary";
  const link = variant === "link";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={accessibilityHint}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.base,
        styles[size],
        primary ? styles.primary : link ? null : styles.secondary,
        pressed && styles.pressed,
        disabled && styles.disabled,
        style,
      ]}
    >
      {icon}
      <Text style={[styles.label, styles[`${size}Label`], primary && styles.primaryLabel, primary && size === "large" && styles.largePrimaryLabel, link && styles.linkLabel]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  base: {
    paddingHorizontal: 16,
    flexDirection: "row",
    gap: 8,
    alignItems: "center",
    justifyContent: "center",
  },
  large: { minHeight: 50, borderRadius: radius.button },
  medium: { minHeight: TOUCH, borderRadius: radius.button },
  small: { minHeight: 40, paddingHorizontal: 10, borderRadius: radius.small },
  primary: { backgroundColor: colors.accent },
  secondary: { borderWidth: 1, borderColor: colors.borderStrong },
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  label: { color: colors.text, textAlign: "center" },
  largeLabel: { fontFamily: ui(400), fontSize: 15 },
  mediumLabel: { fontFamily: ui(400), fontSize: 15 },
  smallLabel: { fontFamily: ui(400), fontSize: 14 },
  primaryLabel: { fontFamily: ui(600), color: colors.bg },
  largePrimaryLabel: { fontFamily: ui(400), fontSize: 16 },
  linkLabel: { color: colors.accent },
});
