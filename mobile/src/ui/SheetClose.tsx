import { useRouter } from "expo-router";
import { useCallback, useEffect } from "react";
import { Platform, StyleSheet, View } from "react-native";
import { IconButton } from "./Screen";

/**
 * A form sheet's way out on the web (Run, a block's actions, the chip menus): there the sheet is
 * a plain page with no grabber to swipe down, so it gets a Close button, and Escape closes it.
 * iOS and Android close a sheet by its swipe and Back; nothing is drawn there.
 */
export function SheetClose({ onClose }: { onClose?: () => void } = {}) {
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
  if (Platform.OS !== "web") return null;
  return (
    <View style={styles.row}>
      <IconButton glass icon="close" label="Close" onPress={close} />
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", justifyContent: "flex-end", paddingTop: 8, paddingHorizontal: 8, marginBottom: -16 },
});
