import type { ReactNode } from "react";
import { Pressable, Text, type StyleProp, type ViewStyle } from "react-native";
import { radius, TOUCH, ui, makeStyles } from "./theme";
import { toggleA11y } from "./toggle";

/**
 * The Standard board's four buttons: `primary` (accent fill, the one action a page or sheet asks
 * for), `secondary` (outlined), `text` (accent words only: Paste a pairing link) and `destructive`
 * (outlined in the error red: Close tab, Unpair). Orange is never a button's colour: it means
 * "needs you".
 */
export type ButtonVariant = "primary" | "secondary" | "text" | "destructive";

/** regular: 48 pt (pages, sheets, cards); small: 34 pt with a 44 pt hit area (Copy error, Run again). */
export type ButtonSize = "regular" | "small";

/** A regular button's height and a small one's (the boards: 48 and the cards' 34 pt pills). */
export const BUTTON_HEIGHT = { regular: 48, small: 34 } as const;

/** The hit slop that takes a small button's 34 pt to the 44 pt touch target. */
export const SMALL_SLOP = (TOUCH - BUTTON_HEIGHT.small) / 2;

interface Props {
  label: string;
  onPress: () => void;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
  disabled?: boolean;
  /** What a screen reader says, when it says more than the label ("Stop, sends control C"). */
  accessibilityLabel?: string;
  accessibilityHint?: string;
  /** A toggle (Notify when done): its state is spoken, and on draws the accent tint. */
  pressed?: boolean;
  /** Lines the label may take; given, a longer label shrinks to fit (an answer's choices). */
  numberOfLines?: number;
  style?: StyleProp<ViewStyle>;
}

export function Button({
  label,
  onPress,
  variant = "secondary",
  size = "regular",
  icon,
  disabled,
  accessibilityLabel,
  accessibilityHint,
  pressed: toggle,
  numberOfLines,
  style,
}: Props) {
  const styles = useStyles();
  const small = size === "small";
  return (
    <Pressable
      {...(toggle === undefined
        ? { accessibilityRole: "button" as const, accessibilityState: { disabled: !!disabled } }
        : toggleA11y(toggle, !!disabled))}
      accessibilityLabel={accessibilityLabel ?? label}
      accessibilityHint={accessibilityHint}
      disabled={disabled}
      onPress={onPress}
      hitSlop={small ? { top: SMALL_SLOP, bottom: SMALL_SLOP } : undefined}
      style={({ pressed }) => [
        styles.base,
        small ? styles.small : styles.regular,
        styles[variant],
        toggle && styles.on,
        pressed && styles.pressed,
        disabled && styles.disabled,
        style,
      ]}
    >
      {icon}
      <Text
        style={[styles.label, small && styles.smallLabel, styles[`${variant}Label`]]}
        numberOfLines={numberOfLines}
        adjustsFontSizeToFit={numberOfLines !== undefined}
        minimumFontScale={0.75}
      >
        {label}
      </Text>
    </Pressable>
  );
}

const useStyles = makeStyles((colors) => ({
  // the boards: 48 pt, 18 pt in, Inter 600 16; the cards' pills 34 pt, 12 pt in, 600 13; 6 pt to an icon
  base: { flexDirection: "row", gap: 6, alignItems: "center", justifyContent: "center", borderRadius: radius.button },
  regular: { minHeight: BUTTON_HEIGHT.regular, paddingHorizontal: 18 },
  small: { minHeight: BUTTON_HEIGHT.small, paddingHorizontal: 12 },
  primary: { backgroundColor: colors.accent },
  secondary: { borderWidth: 1, borderColor: colors.rim },
  text: {},
  destructive: { borderWidth: 1, borderColor: `${colors.error}66` },
  on: { backgroundColor: colors.tints.accentBadge, borderColor: `${colors.accent}33` },
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  label: { fontFamily: ui(600), color: colors.text, fontSize: 16, textAlign: "center", flexShrink: 1 },
  smallLabel: { fontFamily: ui(600), fontSize: 13 },
  primaryLabel: { color: colors.onAccent },
  secondaryLabel: {},
  textLabel: { color: colors.accent },
  destructiveLabel: { color: colors.error },
}));
