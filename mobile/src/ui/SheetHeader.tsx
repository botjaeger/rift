import { useRouter } from "expo-router";
import { useCallback, useEffect, type ReactNode } from "react";
import { Platform, Pressable, Text, View } from "react-native";
import { Icon } from "./Icon";
import { mono, tint, ui, makeStyles, useColors } from "./theme";

/**
 * The one sheet header (the Standard board): the grabber, the title (a header for screen
 * readers), a subtitle under it and the round ✕, always. On the web, where a sheet is a plain
 * page, Escape closes it too. `onClose` defaults to Back (Portals when there is nothing behind).
 */
export function SheetHeader({
  title,
  subtitle,
  subtitleColor,
  titleMono,
  subtitleMono,
  accessory,
  onClose,
  closeLabel = "Close",
}: {
  title: string;
  /** Text, or Text runs (Changes' "+12 −3" in their colours). */
  subtitle?: ReactNode;
  subtitleColor?: string;
  /** The title is a command or a path (a block's command, a file's name). */
  titleMono?: boolean;
  /** The subtitle is a path (Change folder's folder). */
  subtitleMono?: boolean;
  /** Beside the ✕ (a diff's line counts). */
  accessory?: ReactNode;
  onClose?: () => void;
  /** What the ✕ says when it does not close (a question's ✕ folds it). */
  closeLabel?: string;
}) {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const close = useCallback(() => {
    if (onClose) onClose();
    else if (router.canGoBack()) router.back();
    else router.replace("/");
  }, [onClose, router]);
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      close();
    };
    // capture: react-native-web's text fields stop their key events from bubbling
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [close]);
  return (
    <View>
      {/* the boards' 36 × 5 grabber, drawn here on every platform: the system's (`sheetGrabberVisible`) is off, iOS 27 draws a wider one */}
      <View style={styles.grabber} importantForAccessibility="no" accessibilityElementsHidden />

      <View style={styles.row}>
        <View style={styles.titles}>
          <Text style={[styles.title, titleMono && styles.titleMono]} numberOfLines={1} ellipsizeMode={titleMono ? "head" : "tail"} accessibilityRole="header">
            {title}
          </Text>
          {subtitle ? (
            <Text
              style={[styles.subtitle, subtitleMono && styles.subtitleMono, subtitleColor ? { color: subtitleColor } : null]}
              numberOfLines={1}
              ellipsizeMode={subtitleMono ? "head" : "tail"}
            >
              {subtitle}
            </Text>
          ) : null}
        </View>
        {accessory}
        {/* the 36 pt ✕, 44 to the touch */}
        <Pressable accessibilityRole="button" accessibilityLabel={closeLabel} onPress={close} hitSlop={4} style={({ pressed }) => [styles.close, pressed && styles.pressed]}>
          {/* the boards' ✕: a 7 pt mark in the 36 pt circle */}
          <Icon name="close" size={10} color={colors.text} />
        </Pressable>
      </View>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  grabber: { alignSelf: "center", width: 36, height: 5, borderRadius: 3, marginTop: 8, backgroundColor: tint(colors.textBright, 0x33) },
  // the board: 10 pt over, 14 under, 18 pt in on the left, 14 on the right, 12 between
  row: { flexDirection: "row", alignItems: "center", gap: 12, paddingTop: 10, paddingBottom: 14, paddingLeft: 18, paddingRight: 14 },
  titles: { flex: 1, minWidth: 0 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 18, lineHeight: 22 },
  titleMono: { fontFamily: mono, fontSize: 17 },
  subtitle: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  subtitleMono: { fontFamily: mono, fontSize: 13 },
  close: { width: 36, height: 36, borderRadius: 18, backgroundColor: colors.glass, borderWidth: 1, borderColor: colors.glassRim, alignItems: "center", justifyContent: "center" },
  pressed: { opacity: 0.6 },
}));
