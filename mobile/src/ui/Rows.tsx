import type { ReactNode } from "react";
import { Platform, Pressable, StyleSheet, Switch, Text, View } from "react-native";
import { colors, TOUCH, ui } from "./theme";

/**
 * A settings row with a switch; the whole row is one accessible switch (label, detail as the
 * hint, on/off and disabled state) and one touch target: a tap anywhere toggles it, and the
 * switch itself only shows the state (an accessible parent View swallowed the switch's own
 * touches on iOS). At large text sizes the label wraps; the switch stays.
 */
export function SwitchRow({
  label,
  detail,
  value,
  onChange,
  first,
  disabled,
}: {
  label: string;
  detail?: string;
  value: boolean;
  onChange: (v: boolean) => void;
  first?: boolean;
  disabled?: boolean;
}) {
  return (
    <Pressable
      style={[styles.row, !first && styles.divider, disabled && styles.disabled]}
      accessibilityRole="switch"
      accessibilityLabel={label}
      accessibilityHint={detail}
      accessibilityState={{ checked: value, disabled: !!disabled }}
      disabled={disabled}
      onPress={() => onChange(!value)}
    >
      <View style={styles.text}>
        <Text style={styles.label}>{label}</Text>
        {detail ? <Text style={styles.detail}>{detail}</Text> : null}
      </View>
      <View pointerEvents="none" importantForAccessibility="no-hide-descendants">
        <Switch
          value={value}
          disabled={disabled}
          trackColor={{ true: colors.accent, false: colors.hairline }}
          thumbColor={colors.textBright}
          {...webSwitch}
        />
      </View>
    </Pressable>
  );
}

// react-native-web draws an on switch's thumb in its own teal (`activeThumbColor`, a prop the
// native types do not have) instead of `thumbColor`.
const webSwitch: object = Platform.OS === "web" ? { activeThumbColor: colors.textBright } : {};

/** One choice of a group (a radio button): the whole row is the target, a check marks the chosen one. */
export function ChoiceRow({
  label,
  detail,
  selected,
  onSelect,
  first,
}: {
  label: string;
  detail?: string;
  selected: boolean;
  onSelect: () => void;
  first?: boolean;
}) {
  return (
    <Pressable
      style={({ pressed }) => [styles.row, !first && styles.divider, pressed && styles.pressed]}
      accessibilityRole="radio"
      accessibilityLabel={label}
      accessibilityHint={detail}
      accessibilityState={{ checked: selected }}
      onPress={onSelect}
    >
      <View style={styles.text}>
        <Text style={styles.label}>{label}</Text>
        {detail ? <Text style={styles.detail}>{detail}</Text> : null}
      </View>
      <Text style={styles.check} importantForAccessibility="no">
        {selected ? "✓" : " "}
      </Text>
    </Pressable>
  );
}

/** A label/value row (Confirm sheet, pairing): read as one; the value wraps under the label when space runs out. */
export function InfoRow({ label, children, first }: { label: string; children: ReactNode; first?: boolean }) {
  return (
    <View style={[styles.row, styles.wrap, !first && styles.divider]} accessible>
      <Text style={styles.infoLabel}>{label}</Text>
      <View style={styles.infoValue}>{children}</View>
    </View>
  );
}

/** A plain row; `wrap` lets its children flow onto a second line at large text sizes. */
export function Row({ children, first, wrap }: { children: ReactNode; first?: boolean; wrap?: boolean }) {
  return <View style={[styles.row, wrap && styles.wrap, !first && styles.divider]}>{children}</View>;
}

// The mockups' rows: 12 / 14 pt padding, 60 pt tall with a detail line, 1 pt hairlines.
const styles = StyleSheet.create({
  row: { minHeight: TOUCH + 4, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 14, paddingVertical: 12 },
  wrap: { flexWrap: "wrap" },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  disabled: { opacity: 0.6 },
  pressed: { opacity: 0.6 },
  check: { fontFamily: ui(600), color: colors.accent, fontSize: 17, minWidth: 20, textAlign: "center" },
  text: { flex: 1, gap: 3 },
  label: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  detail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17 },
  infoLabel: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, minWidth: 84 },
  infoValue: { flexGrow: 1, flexShrink: 1, flexBasis: 160, alignItems: "flex-end" },
});
