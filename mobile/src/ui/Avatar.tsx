import { StyleSheet, Text, View } from "react-native";
import type { Avatar as AvatarModel } from "@/avatar";
import type { HostMark } from "@/now";
import { colors, monoBold, tint, ui } from "./theme";

/**
 * A pane's round mark (the Computers, Host and Now boards): the prompt mark `>_`, or Claude
 * Code's ✳, in the avatar's colour on an 18 % tint of it; muted, in grey (Now's idle rows).
 * With `mark` (Now, several computers), the computer's monogram sits on its lower right in the
 * computer's colour, rimmed by `ground`. Decorative: the row around it is labelled.
 */
export function Avatar({
  avatar,
  size = 36,
  mark,
  muted,
  ground = colors.surface,
}: {
  avatar: AvatarModel;
  size?: number;
  mark?: HostMark | null;
  muted?: boolean;
  ground?: string;
}) {
  const color = muted ? colors.textMuted : avatar.color;
  return (
    <View style={styles.wrap} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <View style={[styles.avatar, { width: size, height: size, borderRadius: size / 2, backgroundColor: muted ? "#ffffff0f" : tint(avatar.color, 0x2e) }]}>
        <Text
          style={[avatar.glyph ? styles.glyph : styles.prompt, { color, fontSize: size * (avatar.glyph ? 0.42 : 0.36) }]}
          allowFontScaling={false}
        >
          {avatar.glyph ?? ">_"}
        </Text>
      </View>
      {mark ? (
        <View style={[styles.mark, { backgroundColor: mark.color, borderColor: ground }]}>
          <Text style={styles.markText} allowFontScaling={false}>
            {mark.short}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { flexShrink: 0 },
  avatar: { alignItems: "center", justifyContent: "center" },
  glyph: { fontFamily: ui(600) },
  prompt: { fontFamily: monoBold, letterSpacing: -0.5 },
  // the computer's monogram: 16 pt high, a 2 pt rim of the row's ground
  mark: {
    position: "absolute",
    right: -4,
    bottom: -3,
    minWidth: 18,
    height: 16,
    paddingHorizontal: 3,
    borderRadius: 8,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
  markText: { fontFamily: ui(600), fontSize: 8.5, letterSpacing: 0.2, color: colors.bg },
});
