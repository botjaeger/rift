import { useIsFocused } from "expo-router";
import { useCallback, useEffect, useRef, type ReactNode } from "react";
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

/**
 * A tab's page (Now, Computers, Settings): a large title on the chrome ground, then the
 * content in a 12 pt gutter, over room for the floating tab bar.
 */
export function TabScreen({
  title,
  accessory,
  gap = 24,
  large,
  children,
}: {
  title: string;
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
      contentContainerStyle={{ paddingTop: titleTop(insets.top) - (large ? 4 : 0), paddingBottom: tabBottom(insets.bottom) }}
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
  // iOS keeps the bare scroll view: the system tab bar insets (and edge-blurs) only a scroll view
  // that is the page's own; wrapped, the title slid up under the status bar
  if (!android) return page;
  return (
    // the padding leaves room to scroll the last field above the keyboard
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      {page}
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
      style={({ pressed }) => [styles.iconButton, active && styles.iconButtonActive, pressed && styles.pressed, disabled && styles.disabled]}
    >
      <Icon name={icon} color={active ? colors.accent : color} />
    </Pressable>
  );
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
        <IconButton icon={backIcon} label={backLabel ?? (backIcon === "close" ? "Close" : "Back")} onPress={back} />
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
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  pageHeader: { flexDirection: "row", alignItems: "center", gap: 4, paddingHorizontal: 8, paddingBottom: 6 },
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
