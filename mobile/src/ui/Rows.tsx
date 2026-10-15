import { useRouter } from "expo-router";
import type { ReactNode } from "react";
import {
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Icon } from "./Icon";
import { colors, mono, TOUCH, ui } from "./theme";

/** The switch's off track, the board's white 16 % (on is the success green). */
export const SWITCH_OFF = "#ffffff29";
/** The segmented control's chosen segment (the Settings board), under `textBright`. */
export const SEGMENT_ON = "#3a3a3f";

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
          trackColor={{ true: colors.success, false: SWITCH_OFF }}
          ios_backgroundColor={SWITCH_OFF}
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
      style={({ pressed }) => [styles.row, styles.choice, !first && styles.divider, pressed && styles.rowPressed]}
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

/** A read-only row: the label, and its value on the right (Connection, System). Read as one. */
export function ValueRow({
  label,
  detail,
  value,
  first,
  mono: monoValue,
}: {
  label: string;
  detail?: string;
  value: string;
  first?: boolean;
  /** The value in the mono face at 14 pt (Version). */
  mono?: boolean;
}) {
  return (
    <View style={[styles.row, !first && styles.divider]} accessible accessibilityLabel={`${label}, ${value}`}>
      <View style={styles.text}>
        <Text style={styles.label}>{label}</Text>
        {detail ? <Text style={styles.detail}>{detail}</Text> : null}
      </View>
      <Text style={[styles.value, monoValue && styles.monoValue]}>{value}</Text>
    </View>
  );
}

/**
 * A row that opens something: label, detail, a value and a chevron (which turns down while
 * `expanded`). `leading` sits before the label (a status dot, a plus); `accent` draws the label
 * in the accent (Pair a computer).
 */
export function NavRow({
  label,
  detail,
  value,
  mono,
  leading,
  accent,
  chevron = true,
  expanded,
  onPress,
  hint,
  accessibilityLabel,
  first,
}: {
  label: string;
  detail?: string;
  value?: string;
  /** The detail in the mono face (Choose keys' preview). */
  mono?: boolean;
  leading?: ReactNode;
  accent?: boolean;
  chevron?: boolean;
  expanded?: boolean;
  onPress: () => void;
  hint?: string;
  accessibilityLabel?: string;
  first?: boolean;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? [label, value, detail].filter(Boolean).join(", ")}
      accessibilityHint={hint}
      accessibilityState={expanded === undefined ? undefined : { expanded }}
      onPress={onPress}
      style={({ pressed }) => [styles.row, !first && styles.divider, pressed && styles.rowPressed]}
    >
      {leading}
      <View style={styles.text}>
        <Text style={[styles.label, accent && styles.accent]}>{label}</Text>
        {detail ? <Text style={[styles.detail, mono && styles.monoDetail]}>{detail}</Text> : null}
      </View>
      {value ? <Text style={styles.value}>{value}</Text> : null}
      {chevron ? (
        <View style={expanded ? styles.turned : undefined} importantForAccessibility="no">
          <Icon name="forward" size={14} color={colors.textDim} />
        </View>
      ) : null}
    </Pressable>
  );
}

/** Nothing | Failures | Everything: a radio group of equal segments on the chrome ground. */
export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <View style={styles.segments} accessibilityRole="radiogroup" accessibilityLabel={label}>
      {options.map((o) => {
        const on = o.value === value;
        return (
          <Pressable
            key={o.value}
            accessibilityRole="radio"
            accessibilityLabel={o.label}
            accessibilityState={{ checked: on }}
            // 34 pt segments, 44 pt to the touch (the group's padding and the slop)
            hitSlop={{ top: 5, bottom: 5 }}
            onPress={() => onChange(o.value)}
            style={[styles.segment, on && styles.segmentOn]}
          >
            <Text style={[styles.segmentText, on && styles.segmentTextOn]}>{o.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

/**
 * The board's pill buttons: `primary` (accent fill), `outline`, `caution` (outlined in the
 * caution colour). `height` 50 / 46 on the ntfy page, 34 inside a row (44 pt to the touch).
 */
export function Pill({
  label,
  onPress,
  kind = "outline",
  height = 46,
  disabled,
  hint,
  style,
}: {
  label: string;
  onPress: () => void;
  kind?: "primary" | "outline" | "caution";
  height?: 50 | 46 | 34;
  disabled?: boolean;
  hint?: string;
  style?: StyleProp<ViewStyle>;
}) {
  const small = height === 34;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      accessibilityState={{ disabled: !!disabled, busy: disabled && small }}
      disabled={disabled}
      hitSlop={small ? { top: 5, bottom: 5 } : undefined}
      onPress={onPress}
      style={({ pressed }) => [
        styles.pill,
        { height, borderRadius: height / 2 },
        small && styles.pillSmall,
        kind === "primary" ? styles.pillPrimary : kind === "caution" ? styles.pillCaution : styles.pillOutline,
        pressed && styles.rowPressed,
        disabled && styles.disabled,
        style,
      ]}
    >
      <Text style={[styles.pillText, height === 50 && styles.pillTextLarge, kind === "primary" && styles.pillTextPrimary, kind === "caution" && styles.accentWarning]}>
        {label}
      </Text>
    </Pressable>
  );
}

/** A group on a Settings page: a caption, a card of rows, a note under it. */
export function Group({ title, footer, children, style }: { title?: string; footer?: string; children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={[styles.group, style]}>
      {title ? (
        <Text style={styles.caption} accessibilityRole="header">
          {title}
        </Text>
      ) : null}
      {children}
      {footer ? <Text style={styles.footer}>{footer}</Text> : null}
    </View>
  );
}

/** The card of a Group. */
export function CardBox({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return <View style={[styles.card, style]}>{children}</View>;
}

/**
 * A Settings page (Settings, While the app is closed, a computer): a 44 pt glass back badge and
 * a 22 pt title over groups in a 16 pt gutter, 24 pt apart.
 */
export function SettingsPage({ title, children }: { title: string; children: ReactNode }) {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));
  // The header stays put (the Settings board); only the groups under it scroll.
  const page = (
    <View style={styles.fill}>
      <View style={[styles.header, styles.page, { paddingTop: Math.max(insets.top, 47) + 9 }]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Back"
          onPress={back}
          style={({ pressed }) => [styles.back, pressed && styles.rowPressed]}
        >
          <Icon name="back" size={20} color={colors.text} />
        </Pressable>
        <Text style={styles.title} numberOfLines={1} accessibilityRole="header">
          {title}
        </Text>
      </View>
      <ScrollView
        style={styles.fill}
        contentContainerStyle={{ paddingBottom: insets.bottom + 40 }}
        contentInsetAdjustmentBehavior="never"
        automaticallyAdjustContentInsets={false}
        // iOS: the keyboard insets the page and scrolls a focused field above it
        automaticallyAdjustKeyboardInsets
        keyboardShouldPersistTaps="handled"
      >
        <View style={[styles.page, styles.body]}>{children}</View>
      </ScrollView>
    </View>
  );
  // Android 15 draws edge to edge: the padding leaves room to scroll a field above the keyboard
  return Platform.OS === "android" ? (
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      {page}
    </KeyboardAvoidingView>
  ) : (
    page
  );
}

// The Settings board's rows: 8 / 14 pt padding, 52 pt tall, 1 pt hairlines.
const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.chrome },
  page: { maxWidth: 720, width: "100%", alignSelf: "center" },
  header: { flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 12, paddingBottom: 8 },
  back: {
    width: TOUCH,
    height: TOUCH,
    borderRadius: 22,
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
    alignItems: "center",
    justifyContent: "center",
  },
  title: { flex: 1, marginLeft: 6, fontFamily: ui(600), color: colors.textBright, fontSize: 22 },
  body: { paddingTop: 8, paddingHorizontal: 16, gap: 24 },
  group: { gap: 6 },
  caption: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13, paddingHorizontal: 16 },
  footer: { fontFamily: ui(400), color: colors.textDim, fontSize: 12, lineHeight: 17, paddingHorizontal: 16 },
  card: { backgroundColor: colors.surface, borderRadius: 14, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  row: { minHeight: 52, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 14, paddingVertical: 8 },
  wrap: { flexWrap: "wrap" },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  choice: { minHeight: 46, paddingLeft: 28, backgroundColor: colors.bg },
  disabled: { opacity: 0.5 },
  rowPressed: { opacity: 0.7 },
  turned: { transform: [{ rotate: "90deg" }] },
  check: { fontFamily: ui(400), color: colors.accent, fontSize: 16, minWidth: 20, textAlign: "center" },
  text: { flex: 1, gap: 2 },
  label: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  accent: { color: colors.accent },
  accentWarning: { color: colors.warning },
  detail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  monoDetail: { fontFamily: mono },
  value: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15 },
  monoValue: { fontFamily: mono, fontSize: 14 },
  infoLabel: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, minWidth: 84 },
  infoValue: { flexGrow: 1, flexShrink: 1, flexBasis: 160, alignItems: "flex-end" },
  segments: { flexDirection: "row", padding: 3, gap: 2, borderRadius: 10, backgroundColor: colors.chrome },
  segment: { flex: 1, height: 34, borderRadius: 8, alignItems: "center", justifyContent: "center" },
  segmentOn: { backgroundColor: SEGMENT_ON },
  segmentText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 14 },
  segmentTextOn: { color: colors.textBright },
  pill: { alignItems: "center", justifyContent: "center", paddingHorizontal: 14 },
  pillSmall: { paddingHorizontal: 14 },
  pillPrimary: { backgroundColor: colors.accent },
  pillOutline: { borderWidth: 1, borderColor: colors.borderStrong },
  pillCaution: { borderWidth: 1, borderColor: `${colors.warning}66` },
  pillText: { fontFamily: ui(500), color: colors.text, fontSize: 15 },
  pillTextLarge: { fontFamily: ui(500), fontSize: 16 },
  pillTextPrimary: { fontFamily: ui(600), color: colors.bg },
});
