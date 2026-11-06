import type { ReactNode } from "react";
import { Pressable, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Icon, type IconName } from "./Icon";
import { TOUCH, ui, makeStyles, useColors } from "./theme";
import { toggleA11y } from "./toggle";

/** The mockups' top inset for a page title: 62 pt on a 47 pt status bar, never under the island. */
export const titleTop = (inset: number) => Math.max(inset, 47) + 15;
/** The mockups' top inset for a navigation header: 54 pt on a 47 pt status bar. */
export const headerTop = (inset: number) => Math.max(inset, 47) + 7;
/** The bottom padding of a top-level page (Portals, the first-run page): the safe area and a gap. */
export const tabBottom = (inset: number) => inset + 24;

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
  grouped,
  badge,
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
  /** The boards' round glass badge (40 pt, reaching 44 with its slop): back, close, header actions. */
  glass?: boolean;
  /** One of a GlassGroup's 38 pt buttons. */
  grouped?: boolean;
  /** A count on the badge's corner (the panes that need you), in the caution colour. */
  badge?: number;
}) {
  const colors = useColors();
  const color = colorProp ?? colors.text;
  const styles = useStyles();
  return (
    <Pressable
      {...(pressedState === undefined
        ? { accessibilityRole: "button" as const, accessibilityState: { disabled: !!disabled } }
        : toggleA11y(pressedState, !!disabled))}
      accessibilityLabel={label}
      accessibilityHint={hint}
      disabled={disabled}
      onPress={onPress}
      hitSlop={glass ? 2 : grouped ? 3 : undefined}
      style={({ pressed }) => [
        glass ? styles.glassButton : grouped ? styles.groupButton : styles.iconButton,
        active && styles.iconButtonActive,
        pressed && styles.pressed,
        disabled && styles.disabled,
      ]}
    >
      <Icon name={icon} color={active ? colors.accent : color} />
      {badge ? (
        <View style={styles.badge} importantForAccessibility="no">
          <Text style={styles.badgeText}>{badge > 99 ? "99+" : badge}</Text>
        </View>
      ) : null}
    </Pressable>
  );
}

/** The boards' glass pill holding a header's two actions (search, more). */
export function GlassGroup({ children }: { children: ReactNode }) {
  const styles = useStyles();
  return <View style={styles.glassGroup}>{children}</View>;
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
  backIcon = "back",
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
  /** "close" only for a camera or a lock page. */
  backIcon?: "back" | "close";
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
          icon={backIcon}
          label={backLabel ?? (backIcon === "close" ? "Close" : "Back")}
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
    width: 40,
    height: 40,
    borderRadius: 20,
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
    alignItems: "center",
    justifyContent: "center",
  },
  groupButton: { width: 38, height: 38, borderRadius: 19, alignItems: "center", justifyContent: "center" },
  glassGroup: {
    height: 40,
    borderRadius: 20,
    paddingHorizontal: 1,
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
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
  // the Standard board's header: 40 pt glass back 14 pt in, 12 pt before the title, a hairline under
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
