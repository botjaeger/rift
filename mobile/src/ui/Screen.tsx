import type { ReactNode } from "react";
import { Platform, Pressable, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Icon, type IconName } from "./Icon";
import { TOUCH, ui, makeStyles, useColors } from "./theme";
import { toggleA11y } from "./toggle";

/** The mockups' top inset for a header (the large Portals title too): 54 pt on a 47 pt status bar. */
export const headerTop = (inset: number) => Math.max(inset, 47) + 7;
/** The bottom padding of a top-level page (Portals, the first-run page): the safe area and a gap. */
export const tabBottom = (inset: number) => inset + 24;

/**
 * An icon size from the boards: `material`, their Material Symbols size (Android and the web draw
 * those), `sf`, the SF Symbols size that matches it on iOS (SF draws larger at the same size).
 */
export const glyph = (material: number, sf: number): number => (Platform.OS === "ios" ? sf : material);

/** A round glass header button's size (back, +, ⚙, ⌕, ⋯): the boards' 38 pt. */
export const GLASS = 38;

/**
 * The glyph sizes in a glass button that the boards draw smaller than the 18 pt default: their +
 * is about 9 pt across, the gear about 13 (measured on the planned frames); their ⋯ is 19.
 */
export const GLASS_ICON: Partial<Record<IconName, number>> = { add: 12, settings: 16, more: 19 };

/** An uppercase group title (TABS, RUNNING COMMANDS): the Standard board's caption. */
export function SectionTitle({ children, style }: { children: string; style?: StyleProp<ViewStyle> }) {
  const styles = useStyles();
  return (
    <View style={style}>
      <Text style={styles.sectionTitle} accessibilityRole="header">
        {children}
      </Text>
    </View>
  );
}

/** A 44 pt icon button (header actions). */
export function IconButton({
  icon,
  label,
  onPress,
  color: colorProp,
  active,
  pressed: pressedState,
  hint,
  disabled,
  glass,
  badge,
  expanded,
}: {
  icon: IconName;
  label: string;
  onPress: () => void;
  color?: string;
  /** Drawn on the accent tint (a switch that is on, like Wrap long lines: off). */
  active?: boolean;
  /** Reported as a toggle button's state (aria-pressed). */
  pressed?: boolean;
  hint?: string;
  disabled?: boolean;
  /** The boards' round glass badge (38 pt, reaching 44 with its slop): back, close, header actions. */
  glass?: boolean;
  /** A count on the badge's corner (the panes that need you), in the caution colour. */
  badge?: number;
  /** It opens a menu: reported as expanded while that is open (the pane's ⋯). */
  expanded?: boolean;
}) {
  const colors = useColors();
  const color = colorProp ?? colors.text;
  const styles = useStyles();
  return (
    <Pressable
      {...(pressedState === undefined
        ? { accessibilityRole: "button" as const, accessibilityState: { disabled: !!disabled, ...(expanded === undefined ? null : { expanded }) } }
        : toggleA11y(pressedState, !!disabled))}
      accessibilityLabel={label}
      accessibilityHint={hint}
      disabled={disabled}
      onPress={onPress}
      hitSlop={glass ? 3 : undefined}
      style={({ pressed }) => [
        glass ? styles.glassButton : styles.iconButton,
        active && styles.iconButtonActive,
        pressed && styles.pressed,
        disabled && styles.disabled,
      ]}
    >
      <Icon name={icon} size={glass ? GLASS_ICON[icon] : undefined} color={active ? colors.accent : color} />
      {badge ? (
        <View style={styles.badge} importantForAccessibility="no">
          <Text style={styles.badgeText}>{badge > 99 ? "99+" : badge}</Text>
        </View>
      ) : null}
    </Pressable>
  );
}

/**
 * The one page header (the Standard board): ‹ Back (✕ only on a camera or a lock page), the
 * title, an optional subtitle under it, and at most two actions on the right, on the chrome
 * ground with a hairline under it. Portals keeps its large title: it is the root page.
 */
export function PageHeader({
  title,
  subtitle,
  subtitleColor: subtitleColorProp,
  subtitleLines = 1,
  dot,
  back,
  backLabel,
  backBadge,
  right,
}: {
  title?: string;
  subtitle?: string;
  subtitleColor?: string;
  /** Lines the subtitle may take; a one-line subtitle is cut once, at its end. */
  subtitleLines?: number;
  /** A status dot before the subtitle. */
  dot?: string;
  back?: () => void;
  backLabel?: string;
  /** A count on the back button (the panes elsewhere that need you). */
  backBadge?: number;
  /** At most two actions (IconButton glass, or one Button). */
  right?: ReactNode;
}) {
  const colors = useColors();
  const subtitleColor = subtitleColorProp ?? colors.textMuted;
  const styles = useStyles();
  const insets = useSafeAreaInsets();
  return (
    <View style={[styles.pageHeader, { paddingTop: headerTop(insets.top) }]}>
      {back ? (
        <IconButton
          glass
          icon="back"
          label={backLabel ?? "Back"}
          badge={backBadge}
          onPress={back}
        />
      ) : null}
      <View style={styles.headerText}>
        {title ? (
          <Text style={styles.pageTitle} numberOfLines={1} accessibilityRole="header">
            {title}
          </Text>
        ) : null}
        {subtitle ? (
          <View style={styles.subtitleRow} accessible accessibilityLiveRegion="polite">
            {dot ? <View style={[styles.dot, { backgroundColor: dot }]} /> : null}
            <Text style={[styles.headerSubtitle, { color: subtitleColor }]} numberOfLines={subtitleLines}>
              {subtitle}
            </Text>
          </View>
        ) : null}
      </View>
      {right ? <View style={styles.headerActions}>{right}</View> : null}
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  sectionTitle: {
    fontFamily: ui(500),
    color: colors.textMuted,
    fontSize: 12,
    lineHeight: 16,
    textTransform: "uppercase",
    letterSpacing: 0.5,
    paddingHorizontal: 4,
  },
  iconButton: { width: TOUCH, height: TOUCH, borderRadius: TOUCH / 2, alignItems: "center", justifyContent: "center" },
  iconButtonActive: { backgroundColor: `${colors.accent}22` },
  glassButton: {
    width: GLASS,
    height: GLASS,
    borderRadius: GLASS / 2,
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
    alignItems: "center",
    justifyContent: "center",
  },
  badge: {
    position: "absolute",
    top: -4,
    right: -4,
    minWidth: 18,
    height: 18,
    paddingHorizontal: 4,
    borderRadius: 9,
    backgroundColor: colors.warning,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: { fontFamily: ui(600), color: colors.onAccent, fontSize: 11 },
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  // the Standard board's header: 38 pt glass back 14 pt in, 12 pt before the title, a hairline under
  pageHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 14,
    paddingBottom: 12,
    backgroundColor: colors.chrome,
    borderBottomWidth: 1,
    borderBottomColor: colors.hairline,
  },
  headerText: { flex: 1, minWidth: 0 },
  headerActions: { flexDirection: "row", alignItems: "center", gap: 8 },
  pageTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 20, lineHeight: 24 },
  subtitleRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  headerSubtitle: { fontFamily: ui(400), fontSize: 13, lineHeight: 17, flexShrink: 1 },
}));
