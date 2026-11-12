import { useRouter } from "expo-router";
import type { ReactNode } from "react";
import {
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Icon } from "./Icon";
import { PageHeader, SectionTitle } from "./Screen";
import { mono, radius, ui, makeStyles, useColors } from "./theme";

/**
 * The one settings row (the Settings and Sheets boards): `regular` 68 pt (52 and 8 pt over and
 * under), with or without a line under the label; `compact` 44 pt only where a row asks for it
 * (Diagnostics' results). Content lists (Tabs, Panes) use ListRow's rows instead.
 */
export type RowDensity = "regular" | "compact";
/** A row's chevron size: the boards' small › (Material's chevron at 14 px draws about 3 × 6). */
export const CHEVRON = 8;
export const ROW_HEIGHT: Record<RowDensity, number> = { regular: 68, compact: 44 };
export const densityOf = (_detail: unknown, density?: RowDensity): RowDensity => density ?? "regular";

/**
 * The boards' switch: a 51 × 31 track (the accent when on, `switchOff` when off) and a round
 * 27 pt white knob 2 pt in. Drawn, not the system's (iOS 27 draws a wider capsule); it only shows
 * the state: the row around it is the control.
 */
export function Toggle({ on }: { on: boolean }) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <View style={[styles.track, { backgroundColor: on ? colors.accent : colors.switchOff }]} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
      <View style={[styles.knob, on ? styles.knobOn : styles.knobOff]} />
    </View>
  );
}

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
  const styles = useStyles();
  return (
    <Pressable
      style={[styles.row, styles[densityOf(detail)], !first && styles.divider, !first && styles.ruled, disabled && styles.disabled]}
      accessibilityRole="switch"
      accessibilityLabel={label}
      accessibilityHint={detail}
      accessibilityState={{ checked: value, disabled: !!disabled }}
      aria-checked={value}
      disabled={disabled}
      onPress={() => onChange(!value)}
    >
      <View style={styles.text}>
        <Text style={styles.label}>{label}</Text>
        {detail ? <Text style={styles.detail}>{detail}</Text> : null}
      </View>
      <Toggle on={value} />
    </Pressable>
  );
}

/** One choice of a group (a radio button): the whole row is the target, a check marks the chosen one. */
export function ChoiceRow({
  label,
  detail,
  selected,
  onSelect,
  first,
  leading,
  trailing,
  minHeight,
}: {
  label: string;
  detail?: string;
  selected: boolean;
  onSelect: () => void;
  first?: boolean;
  /** Drawn before the label (a theme's swatch). */
  leading?: ReactNode;
  /** Drawn in the check's place while not chosen (Choose a theme's ›). */
  trailing?: ReactNode;
  /** A taller row than the settings rows' 68 (Appearance's themes: 76). */
  minHeight?: number;
}) {
  const styles = useStyles();
  return (
    <Pressable
      style={({ pressed }) => [styles.row, styles[densityOf(detail)], !first && styles.divider, !first && styles.ruled, minHeight !== undefined && { minHeight: minHeight + (first ? 0 : 1) }, pressed && styles.rowPressed]}
      accessibilityRole="radio"
      accessibilityLabel={label}
      accessibilityHint={detail}
      accessibilityState={{ checked: selected }}
      aria-checked={selected}
      onPress={onSelect}
    >
      {leading}
      <View style={styles.text}>
        <Text style={styles.label}>{label}</Text>
        {detail ? <Text style={styles.detail}>{detail}</Text> : null}
      </View>
      {trailing && !selected ? (
        trailing
      ) : (
        <Text style={styles.check} importantForAccessibility="no">
          {selected ? "✓" : " "}
        </Text>
      )}
    </Pressable>
  );
}

/** A label/value row (Diagnostics): read as one; the value wraps under the label when space runs out. */
export function InfoRow({ label, children, first }: { label: string; children: ReactNode; first?: boolean }) {
  const styles = useStyles();
  return (
    <View style={[styles.row, styles.compact, styles.wrap, !first && styles.divider]} accessible>
      <Text style={styles.infoLabel}>{label}</Text>
      <View style={styles.infoValue}>{children}</View>
    </View>
  );
}

/** A plain row; `wrap` lets its children flow onto a second line at large text sizes. */
export function Row({ children, first, wrap, density = "regular" }: { children: ReactNode; first?: boolean; wrap?: boolean; density?: RowDensity }) {
  const styles = useStyles();
  return <View style={[styles.row, styles[density], wrap && styles.wrap, !first && styles.divider, !first && density === "regular" && styles.ruled]}>{children}</View>;
}

/**
 * A read-only row: the label, and its value on the right (Connection, System, Starts in). Read as
 * one. A `path` value is mono and keeps its last folders when cut.
 */
export function ValueRow({
  label,
  detail,
  value,
  first,
  path,
}: {
  label: string;
  detail?: string;
  value: string;
  first?: boolean;
  path?: boolean;
}) {
  const styles = useStyles();
  return (
    <View style={[styles.row, styles[densityOf(detail)], !first && styles.divider, !first && styles.ruled]} accessible accessibilityLabel={`${label}, ${value}`}>
      <View style={path ? styles.labelOnly : styles.text}>
        <Text style={styles.label}>{label}</Text>
        {detail ? <Text style={styles.detail}>{detail}</Text> : null}
      </View>
      <Text style={[styles.value, path && styles.pathValue]} numberOfLines={path ? 1 : undefined} ellipsizeMode={path ? "head" : undefined}>
        {value}
      </Text>
    </View>
  );
}

/**
 * A row that opens something: label, detail, a value and a chevron (which turns down while
 * `expanded`). `leading` sits before the label (a status dot, a plus, a program's tile); `accent`
 * draws the label in the accent (Pair a computer), `destructive` in the error red with no chevron
 * (Unpair this computer).
 */
export function NavRow({
  label,
  detail,
  value,
  mono: monoDetail,
  leading,
  accent,
  destructive,
  chevron = !destructive,
  expanded,
  disabled,
  onPress,
  hint,
  accessibilityLabel,
  first,
  density,
  path,
}: {
  label: string;
  detail?: string;
  value?: string;
  /** The value is a folder: cut at its start, so its last folders show (New tab's Folder). */
  path?: boolean;
  /** The detail is keys or a command (Choose keys' preview). */
  mono?: boolean;
  leading?: ReactNode;
  accent?: boolean;
  destructive?: boolean;
  chevron?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  onPress: () => void;
  hint?: string;
  accessibilityLabel?: string;
  first?: boolean;
  density?: RowDensity;
}) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? [label, value, detail].filter(Boolean).join(", ")}
      accessibilityHint={hint}
      accessibilityState={expanded === undefined ? { disabled: !!disabled } : { expanded, disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.row, styles[densityOf(detail, density)], !first && styles.divider, !first && densityOf(detail, density) === "regular" && styles.ruled, pressed && styles.rowPressed, disabled && styles.disabled]}
    >
      {leading}
      <View style={styles.text}>
        <Text style={[styles.label, accent && styles.accent, destructive && styles.destructive]}>{label}</Text>
        {detail ? <Text style={[styles.detail, monoDetail && styles.monoDetail]}>{detail}</Text> : null}
      </View>
      {value ? (
        <Text style={styles.value} numberOfLines={1} ellipsizeMode={path ? "head" : "tail"}>
          {value}
        </Text>
      ) : null}
      {chevron ? (
        // the boards' › : a 3 × 6 pt mark in a 14 pt column
        <View style={[styles.chevron, expanded && styles.turned]} importantForAccessibility="no">
          <Icon name="forward" size={CHEVRON} color={colors.textMuted} />
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
  const styles = useStyles();
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
            aria-checked={on}
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

/** A group of rows: a caption, the rows (a CardBox), a note under it. */
export function Group({ title, footer, children, style }: { title?: string; footer?: string; children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const styles = useStyles();
  return (
    <View style={[styles.group, style]}>
      {title ? <SectionTitle style={styles.caption}>{title}</SectionTitle> : null}
      {children}
      {footer ? <Text style={styles.footer}>{footer}</Text> : null}
    </View>
  );
}

/** The card of a Group: settings rows sit in cards, content rows edge to edge. */
export function CardBox({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const styles = useStyles();
  return <View style={[styles.card, style]}>{children}</View>;
}

/**
 * A Settings page (Settings, While the app is closed, a computer, Appearance): the page header
 * (‹ Back and the title, `subtitle` under it), then groups in a 16 pt gutter, 24 pt apart;
 * `notice` stays under the header.
 */
export function SettingsPage({ title, subtitle, notice, children }: { title: string; subtitle?: string; notice?: ReactNode; children: ReactNode }) {
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));
  // The header stays put (the Settings board); only the groups under it scroll.
  const page = (
    <View style={styles.fill}>
      <PageHeader title={title} subtitle={subtitle} back={back} />
      {notice ? <View style={styles.page}>{notice}</View> : null}
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

// The Settings board's rows: 16 pt in, 68 pt tall, 1 pt hairlines; 16 pt labels, 13 pt lines. The
// page is the bg under the header's chrome; a card ends 18 pt above the next caption, a footer 10
// pt under its card and 10 over the next caption's room.
const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  page: { maxWidth: 720, width: "100%", alignSelf: "center" },
  body: { paddingTop: 18, paddingHorizontal: 16, gap: 18 },
  group: { gap: 8 },
  caption: {},
  footer: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 19, paddingHorizontal: 4, paddingTop: 2, paddingBottom: 10 },
  card: { backgroundColor: colors.surface, borderRadius: radius.card, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  row: { flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 16, paddingVertical: 8 },
  regular: { minHeight: ROW_HEIGHT.regular },
  compact: { minHeight: ROW_HEIGHT.compact, paddingVertical: 6 },
  // the wrapped lines centred in the row's height (they pack to its top by default)
  wrap: { flexWrap: "wrap", alignContent: "center" },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  // a hairline over a row adds its point, as the boards draw it under the 68 pt row
  ruled: { minHeight: ROW_HEIGHT.regular + 1 },
  disabled: { opacity: 0.5 },
  rowPressed: { opacity: 0.7 },
  chevron: { width: 14, alignItems: "center" },
  turned: { transform: [{ rotate: "90deg" }] },
  check: { fontFamily: ui(600), color: colors.accent, fontSize: 16, minWidth: 20, textAlign: "center" },
  text: { flex: 1, gap: 2 },
  labelOnly: { gap: 2 },
  label: { fontFamily: ui(400), color: colors.text, fontSize: 16, lineHeight: 21 },
  accent: { color: colors.accent },
  destructive: { color: colors.error },
  detail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  monoDetail: { fontFamily: mono, fontSize: 12 },
  // a long value (a theme file's name) wraps or cuts beside its label, never over it
  value: { flexShrink: 1, maxWidth: "60%", textAlign: "right", fontFamily: ui(400), color: colors.textMuted, fontSize: 15, lineHeight: 20 },
  pathValue: { flex: 1, maxWidth: "100%", textAlign: "right", fontFamily: mono, fontSize: 13 },
  infoLabel: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15, minWidth: 84 },
  infoValue: { flexGrow: 1, flexShrink: 1, flexBasis: 160, alignItems: "flex-end" },
  segments: { flexDirection: "row", padding: 3, gap: 2, borderRadius: radius.button, backgroundColor: colors.chrome },
  segment: { flex: 1, height: 34, borderRadius: radius.button - 2, alignItems: "center", justifyContent: "center" },
  segmentOn: { backgroundColor: colors.segmentOn },
  segmentText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 14 },
  segmentTextOn: { color: colors.textBright },
  track: { width: 51, height: 31, borderRadius: 16 },
  knob: { position: "absolute", top: 2, width: 27, height: 27, borderRadius: 14, backgroundColor: "#ffffff" },
  knobOff: { left: 2 },
  knobOn: { right: 2 },
}));
