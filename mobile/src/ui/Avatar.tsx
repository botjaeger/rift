import { StyleSheet, Text, View } from "react-native";
import type { Avatar as AvatarModel } from "@/avatar";
import { monoBold, tint, ui } from "./theme";

/**
 * A pane's round mark (the Computers, Host and Now boards): the prompt mark `>_`, or Claude
 * Code's ✳, in the avatar's colour on an 18 % tint of it. Decorative: the row around it is
 * labelled.
 */
export function Avatar({ avatar, size = 36 }: { avatar: AvatarModel; size?: number }) {
  return (
    <View
      style={[styles.avatar, { width: size, height: size, borderRadius: size / 2, backgroundColor: tint(avatar.color, 0x2e) }]}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      <Text
        style={[avatar.glyph ? styles.glyph : styles.prompt, { color: avatar.color, fontSize: size * (avatar.glyph ? 0.42 : 0.36) }]}
        allowFontScaling={false}
      >
        {avatar.glyph ?? ">_"}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  avatar: { alignItems: "center", justifyContent: "center", flexShrink: 0 },
  glyph: { fontFamily: ui(600) },
  prompt: { fontFamily: monoBold, letterSpacing: -0.5 },
});
