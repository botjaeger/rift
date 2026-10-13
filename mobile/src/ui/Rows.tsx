import type { ReactNode } from "react";
import { Pressable, StyleSheet, Switch, Text, View } from "react-native";
import { colors, TOUCH } from "./theme";

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
        />
      </View>
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

const styles = StyleSheet.create({
  row: { minHeight: TOUCH + 8, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 14, paddingVertical: 8 },
  wrap: { flexWrap: "wrap" },
  divider: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.hairline },
  disabled: { opacity: 0.6 },
  text: { flex: 1, gap: 2 },
  label: { color: colors.text, fontSize: 15 },
  detail: { color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  infoLabel: { color: colors.textMuted, fontSize: 14, minWidth: 84 },
  infoValue: { flexGrow: 1, flexShrink: 1, flexBasis: 160, alignItems: "flex-end" },
});
