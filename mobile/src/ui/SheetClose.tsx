import { useRouter } from "expo-router";
import { useEffect } from "react";
import { Platform, StyleSheet, View } from "react-native";
import { IconButton } from "./Screen";

/**
 * A form sheet's way out on the web (Run, a block's actions, the chip menus): there the sheet is
 * a plain page with no grabber to swipe down, so it gets a Close button, and Escape closes it.
 * iOS and Android close a sheet by its swipe and Back; nothing is drawn there.
 */
export function SheetClose() {
  const router = useRouter();
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      if (router.canGoBack()) router.back();
      else router.replace("/");
    };
    // capture: react-native-web's text fields stop their key events from bubbling
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [router]);
  if (Platform.OS !== "web") return null;
  return (
    <View style={styles.row}>
      <IconButton icon="close" label="Close" onPress={() => (router.canGoBack() ? router.back() : router.replace("/"))} />
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", justifyContent: "flex-end", paddingTop: 8, paddingHorizontal: 8, marginBottom: -16 },
});
