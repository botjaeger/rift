import { useIsFocused } from "expo-router";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  Keyboard,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Icon, type IconName } from "./Icon";
import { colors, radius, TOUCH, ui } from "./theme";
import { toggleA11y } from "./toggle";

/** The mockups' top inset for a page title: 62 pt on a 47 pt status bar, never under the island. */
export const titleTop = (inset: number) => Math.max(inset, 47) + 15;
/** The mockups' top inset for a navigation header: 54 pt on a 47 pt status bar. */
export const headerTop = (inset: number) => Math.max(inset, 47) + 7;
/** Room left under a tab page for the floating tab bar (62 pt, 26 pt above the edge). */
export const TAB_BAR_ROOM = 120;
/**
 * The bottom padding of a plain tab page (the first-run page), from its bottom safe-area inset.
 * On iOS the system tab bar is part of that inset already (83 pt on an iPhone 16 Pro), so only a
 * gap is added; Android and the web draw the floating bar over the page and leave TAB_BAR_ROOM.
 */
export const tabBottom = (inset: number) => inset + (Platform.OS === "ios" ? 24 : TAB_BAR_ROOM);

const android = Platform.OS === "android";
/** Room under a page's content for a floating overlay (a 44 pt bar and a gap). */
const OVERLAY_ROOM = 56;

/**
 * A tab's page (Now, Computers, Settings): a large title on the chrome ground, then the
 * content in a 12 pt gutter, over room for the floating tab bar.
 */
export function TabScreen({
  title,
  accessory,
  gap = 24,
  large,
  overlay,
  children,
}: {
  title: string;
  /** Floats over the page above the tab bar (Computers' search bar); the page leaves room for it. */
  overlay?: ReactNode;
  accessory?: ReactNode;
  /** The phone-first boards' 30 pt title (Now, Computers) with 10 pt under the header. */
  large?: boolean;
  /** Space between the content's children (Settings 24, Now 18, Computers' cards 10). */
  gap?: number;
  children: ReactNode;
}) {
  const insets = useSafeAreaInsets();
  const scroll = useRef<ScrollView>(null);
  const offset = useRef(0);
  // the keyboard's top edge (window coordinates) while it is up
  const keyboardTop = useRef<number | null>(null);
  const focused = useIsFocused();
  // iOS: the overlay rides above the keyboard while it is up (Android's KeyboardAvoidingView
  // already lifts the page's bottom edge)
  const [keyboardH, setKeyboardH] = useState(0);
  const hasOverlay = !!overlay;
  useEffect(() => {
    if (android || !hasOverlay) return;
    const show = Keyboard.addListener("keyboardWillShow", (e) => setKeyboardH(e.endCoordinates.height));
    const hide = Keyboard.addListener("keyboardWillHide", () => setKeyboardH(0));
    return () => {
      show.remove();
      hide.remove();
    };
  }, [hasOverlay]);
  const overlayBottom = keyboardH > 0 ? keyboardH + 8 : tabBottom(insets.bottom) - 12;
  // Android 15 draws edge to edge: the window keeps its size for the keyboard and nothing
  // scrolls a focused field low on the page (Settings › This phone's name) out from under it
  const revealFocusedField = useCallback(() => {
    const input = TextInput.State.currentlyFocusedInput();
    const top = keyboardTop.current;
    if (!focused || !input || top === null) return;
    input.measureInWindow((_x, y, _w, h) => {
      const under = y + h + 12 - top;
      if (under > 0) scroll.current?.scrollTo({ y: offset.current + under, animated: true });
    });
  }, [focused]);
  useEffect(() => {
    if (!android) return;
    let settle: ReturnType<typeof setTimeout> | undefined;
    const show = Keyboard.addListener("keyboardDidShow", (e) => {
      keyboardTop.current = e.endCoordinates.screenY;
      revealFocusedField();
      // and once the padding's layout has landed: a scroll started before it can stop at the old
      // end of the page, leaving the field flush under the keyboard (seen 2 of 6 runs)
      clearTimeout(settle);
      settle = setTimeout(revealFocusedField, 300);
    });
    const hide = Keyboard.addListener("keyboardDidHide", () => {
      keyboardTop.current = null;
      clearTimeout(settle);
    });
    return () => {
      show.remove();
      hide.remove();
      clearTimeout(settle);
    };
  }, [revealFocusedField]);
  const page = (
    <ScrollView
      ref={scroll}
      onScroll={(e) => {
        offset.current = e.nativeEvent.contentOffset.y;
      }}
      scrollEventThrottle={32}
      // again once the padding has shrunk the view: before that the scroll stops short at the
      // old end of the page
      onLayout={revealFocusedField}
      style={styles.fill}
      // the large header's row is its 40 pt button, 58 pt down on the boards
      // the page pads itself by the safe area: the tab layout turns off the system's automatic
      // content inset (disableAutomaticContentInsets), which a keyboard anywhere in the app undid
      // (React Native clamps the offset to the prop inset, the title slid under the status bar)
      contentContainerStyle={{
        paddingTop: titleTop(insets.top) - (large ? 4 : 0),
        paddingBottom: tabBottom(insets.bottom) + (overlay ? OVERLAY_ROOM : 0),
      }}
      contentInsetAdjustmentBehavior="never"
      automaticallyAdjustContentInsets={false}
      // iOS: the keyboard insets the page and scrolls a focused field above it
      automaticallyAdjustKeyboardInsets
      keyboardShouldPersistTaps="handled"
    >
      <View style={styles.page}>
        <View
          style={[
            styles.header,
            accessory ? styles.headerWithAccessory : null,
            styles.headerGap,
            large ? styles.headerLarge : null,
          ]}
        >
          <Text style={[styles.title, large && styles.titleLarge]} accessibilityRole="header">
            {title}
          </Text>
          {accessory}
        </View>
        <View style={[styles.content, { gap }]}>{children}</View>
      </View>
    </ScrollView>
  );
  // iOS keeps the scroll view the page's first view: the system tab bar insets (and edge-blurs)
  // only a scroll view that is the page's own; wrapped, the title slid up under the status bar
  if (!android)
    return overlay ? (
      <View style={styles.fill}>
        {page}
        <View pointerEvents="box-none" style={[styles.overlay, { bottom: overlayBottom }]}>
          {overlay}
        </View>
      </View>
    ) : (
      page
    );
  return (
    // the padding leaves room to scroll the last field above the keyboard
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      {page}
      {overlay ? (
        <View pointerEvents="box-none" style={[styles.overlay, { bottom: overlayBottom }]}>
          {overlay}
        </View>
      ) : null}
      {/* the status bar is see-through (edge to edge): content scrolled up under it hid behind the clock */}
      <View pointerEvents="none" style={[styles.statusScrim, { height: insets.top }]} />
    </KeyboardAvoidingView>
  );
}

/** An uppercase group title (TABS, RUNNING COMMANDS). */
export function SectionTitle({ children, style }: { children: string; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={style}>
      <Text style={styles.sectionTitle} accessibilityRole="header">
        {children}
      </Text>
    </View>
  );
}

/** A rounded card on the page (surface, hairline border). */
export function Card({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return <View style={[styles.group, style]}>{children}</View>;
}

/** A titled card of rows with an optional note under it (Settings, Pair). */
export function Section({ title, footer, children }: { title?: string; footer?: string; children: ReactNode }) {
  return (
    <View style={styles.section}>
      {title ? <SectionTitle>{title}</SectionTitle> : null}
      <View style={styles.group}>{children}</View>
      {footer ? <Text style={styles.footer}>{footer}</Text> : null}
    </View>
  );
}

/** A 44 pt icon button (header actions). */
export function IconButton({
  icon,
  label,
  onPress,
  color = colors.text,
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

/** The boards' glass pill holding a page's header actions (wrap, search, terminal view). */
export function GlassGroup({ children }: { children: ReactNode }) {
  return <View style={styles.glassGroup}>{children}</View>;
}

/**
 * A pushed page's header (Host, Pair, Keys, the pane): back (or close), a title with an
 * optional status line under it, and actions on the right.
 */
export function PageHeader({
  title,
  titleSize = 17,
  subtitle,
  subtitleColor = colors.textMuted,
  subtitleLines = 2,
  dot,
  back,
  backIcon = "back",
  backLabel,
  right,
  chrome,
}: {
  title?: string;
  titleSize?: number;
  subtitle?: string;
  subtitleColor?: string;
  /** Lines the subtitle may take; a one-line subtitle is cut in the middle. */
  subtitleLines?: number;
  /** A status dot before the subtitle. */
  dot?: string;
  back?: () => void;
  backIcon?: "back" | "close";
  backLabel?: string;
  right?: ReactNode;
  /** The pane's header: on the chrome ground with a hairline under it. */
  chrome?: boolean;
}) {
  const insets = useSafeAreaInsets();
  return (
    <View style={[styles.pageHeader, chrome && styles.pageHeaderChrome, { paddingTop: headerTop(insets.top) }]}>
      {back ? (
        <IconButton glass icon={backIcon} label={backLabel ?? (backIcon === "close" ? "Close" : "Back")} onPress={back} />
      ) : null}
      <View style={styles.headerText}>
        {title ? (
          <Text style={[styles.pageTitle, { fontSize: titleSize }]} numberOfLines={1} accessibilityRole="header">
            {title}
          </Text>
        ) : null}
        {subtitle ? (
          <View style={styles.subtitleRow} accessible accessibilityLiveRegion="polite">
            {dot ? <View style={[styles.dot, { backgroundColor: dot }]} /> : null}
            <Text
              style={[styles.headerSubtitle, { color: subtitleColor }]}
              numberOfLines={subtitleLines}
              ellipsizeMode={subtitleLines === 1 ? "middle" : "tail"}
            >
              {subtitle}
            </Text>
          </View>
        ) : null}
      </View>
      {right}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.chrome },
  overlay: { position: "absolute", left: 24, right: 24 },
  statusScrim: { position: "absolute", top: 0, left: 0, right: 0, backgroundColor: colors.chrome },
  page: { maxWidth: 720, width: "100%", alignSelf: "center" },
  header: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 12, paddingLeft: 20, paddingRight: 20 },
  headerWithAccessory: { paddingRight: 16, paddingBottom: 4 },
  headerGap: { paddingBottom: 14 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 28, letterSpacing: -0.56, flexShrink: 1 },
  titleLarge: { fontFamily: ui(600), fontSize: 30, letterSpacing: -0.6 },
  headerLarge: { paddingBottom: 10, paddingRight: 16 },
  content: { paddingHorizontal: 12 },
  section: { gap: 6 },
  sectionTitle: {
    fontFamily: ui(500),
    color: colors.textDim,
    fontSize: 12,
    textTransform: "uppercase",
    letterSpacing: 0.5,
    paddingHorizontal: 8,
  },
  group: {
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  footer: { fontFamily: ui(400), color: colors.textDim, fontSize: 12, lineHeight: 17, marginTop: 2, marginHorizontal: 8 },
  iconButton: { width: TOUCH, height: TOUCH, borderRadius: radius.control, alignItems: "center", justifyContent: "center" },
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
  badgeText: { fontFamily: ui(600), color: colors.bg, fontSize: 11 },
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  // the boards' header: a 40 pt glass back badge 12 pt in, 10 pt before the title
  pageHeader: { flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 12, paddingBottom: 6 },
  pageHeaderChrome: {
    gap: 2,
    paddingLeft: 8,
    paddingRight: 4,
    backgroundColor: colors.chrome,
    borderBottomWidth: 1,
    borderBottomColor: colors.hairline,
  },
  headerText: { flex: 1, minWidth: 0, gap: 1 },
  pageTitle: { fontFamily: ui(600), color: colors.textBright },
  subtitleRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { width: 6, height: 6, borderRadius: 3 },
  headerSubtitle: { fontFamily: ui(400), fontSize: 12, flexShrink: 1 },
});
