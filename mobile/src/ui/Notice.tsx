import type { ReactNode } from "react";
import { StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { colors, ui } from "./theme";

/**
 * An error or refusal message, announced at once. The red token is too dark for text on the
 * app's grounds (palette.ts), so the text is `text` and the red is a bar beside it.
 */
export function ErrorText({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={[styles.row, style]} accessible accessibilityRole="alert" accessibilityLiveRegion="assertive">
      <View style={[styles.bar, { backgroundColor: colors.error }]} />
      <Text style={styles.text}>{children}</Text>
    </View>
  );
}

/** A caution (view only, pairing replaces a host): `warning` text, announced politely. */
export function WarningText({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={[styles.row, style]} accessible accessibilityLiveRegion="polite">
      <View style={[styles.bar, { backgroundColor: colors.warning }]} />
      <Text style={[styles.text, { color: colors.warning }]}>{children}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", gap: 8, alignItems: "stretch" },
  bar: { width: 3, borderRadius: 1.5 },
  text: { fontFamily: ui(400), flex: 1, color: colors.text, fontSize: 14, lineHeight: 20 },
});
