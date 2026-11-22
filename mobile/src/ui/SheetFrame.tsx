import { useState, type ReactNode } from "react";
import { Platform, ScrollView, View, type StyleProp, type ViewStyle } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useLock } from "@/runtime";
import { sheetUnder, useSheetHeight, type SheetBoard } from "./sheet";
import { SheetHeader } from "./SheetHeader";
import { makeStyles } from "./theme";

/**
 * The frame of every board sheet (New tab, New pane, Attach, Change folder, Block, Run this?,
 * What's new, Connection): the sheet header (grabber, title, subtitle, ✕), the content, which
 * scrolls when it is longer than the sheet, and `footer`, the sheet's actions, pinned to its foot
 * 24 pt above the screen's end. The root has the board's height (`board`): the sheet fits its
 * content (`fitToContents`), so a growing root would collapse it to nothing.
 */
export function SheetFrame({
  title,
  subtitle,
  subtitleColor,
  titleMono,
  subtitleMono,
  onClose,
  board,
  footer,
  bodyStyle,
  fill = false,
  fit = false,
  children,
}: {
  title: string;
  subtitle?: ReactNode;
  subtitleColor?: string;
  titleMono?: boolean;
  subtitleMono?: boolean;
  onClose: () => void;
  board: SheetBoard;
  footer?: ReactNode;
  /** The content's gutter and gaps, when the board's differ (What's new: 18 pt in). */
  bodyStyle?: StyleProp<ViewStyle>;
  /** The content fills the sheet and does not scroll: a child scrolls itself (Block's long
   * output), so the room above the footer is its bound instead of running on under it. */
  fill?: boolean;
  /** The sheet is as tall as its header, content and footer, up to the board's height (a
   * block's one line of output, a short list): no empty room between the content and the footer. */
  fit?: boolean;
  children: ReactNode;
}) {
  const styles = useStyles();
  const insets = useSafeAreaInsets();
  const height = useSheetHeight(board);
  const lock = useLock();
  // Hidden while the app is in the app switcher, locked, or under a Face ID prompt (the sheet
  // sits above the lock cover). Kept laid out: a sheet sized to its content must not collapse.
  const hidden = lock.covered || lock.locked;
  // `fit`: the parts' measured heights; the sheet keeps the board's height until all three are known
  // (a footer that came and went counts as none: its last height is not kept)
  const [parts, setParts] = useState<{ head: number; body: number; foot: number }>({ head: 0, body: 0, foot: 0 });
  const measure = (part: "head" | "body" | "foot", h: number) =>
    setParts((p) => (Math.abs(p[part] - h) < 0.5 ? p : { ...p, [part]: h }));
  const footH = footer ? parts.foot : 0;
  const natural = fit && parts.head > 0 && parts.body > 0 && (!footer || footH > 0) ? Math.ceil(parts.head + parts.body + footH) : null;
  const rootHeight = height === undefined ? undefined : natural !== null ? Math.min(height, natural) : height;
  // the boards' 24 pt to the screen's end: on iOS the room the system adds under the content
  // (the home indicator's inset) counts; Android draws the sheet under the navigation bar
  const foot = Platform.OS === "ios" ? Math.max(0, SHEET_FOOT - sheetUnder("ios", insets.bottom)) : SHEET_FOOT + (Platform.OS === "android" ? insets.bottom : 0);
  return (
    // Never flattened: react-native-screens resizes a form sheet's top-level scroll view to the
    // whole sheet (over the header), so the scroll view must sit inside this root, not beside it
    <View
      collapsable={false}
      style={[styles.fill, rootHeight === undefined ? styles.grow : { height: rootHeight }, hidden && styles.hidden]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <View onLayout={fit ? (e) => measure("head", e.nativeEvent.layout.height) : undefined}>
        <SheetHeader title={title} subtitle={subtitle} subtitleColor={subtitleColor} titleMono={titleMono} subtitleMono={subtitleMono} onClose={onClose} />
      </View>
      {fill ? (
        <View style={[styles.grow, styles.body, bodyStyle, { paddingBottom: footer ? 12 : foot }]}>{children}</View>
      ) : (
        <ScrollView
          style={styles.grow}
          contentContainerStyle={[styles.body, bodyStyle, { paddingBottom: footer ? 12 : foot }]}
          keyboardShouldPersistTaps="handled"
          onContentSizeChange={fit ? (_, h) => measure("body", h) : undefined}
        >
          {children}
        </ScrollView>
      )}
      {footer ? (
        <View style={[styles.pinned, { paddingBottom: foot }]} onLayout={fit ? (e) => measure("foot", e.nativeEvent.layout.height) : undefined}>
          {footer}
        </View>
      ) : null}
    </View>
  );
}

/** The boards' room under a sheet's pinned actions (or its content), to the screen's end. */
export const SHEET_FOOT = 24;

/** Two actions side by side in a sheet's foot, equal halves 8 pt apart (Cancel / Run, Run again / Edit). */
export function SheetButtons({ children }: { children: ReactNode }) {
  const styles = useStyles();
  return <View style={styles.buttons}>{children}</View>;
}

const useStyles = makeStyles((colors) => ({
  fill: { backgroundColor: colors.surface },
  grow: { flex: 1 },
  hidden: { opacity: 0 },
  body: { gap: 12, paddingHorizontal: 16, maxWidth: 560, width: "100%", alignSelf: "center" },
  pinned: { paddingTop: 12, paddingHorizontal: 16, maxWidth: 560, width: "100%", alignSelf: "center" },
  buttons: { flexDirection: "row", gap: 8 },
}));
