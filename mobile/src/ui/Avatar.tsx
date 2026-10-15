import { StyleSheet, Text, View } from "react-native";
import type { Avatar as AvatarModel } from "@/avatar";
import { colors, monoBold, monoMedium, tint, tints, ui } from "./theme";

/**
 * A pane's round mark (the Computers and Host boards): the prompt mark `>_`, or Claude Code's
 * ✳, in the avatar's colour on an 18 % tint of it; muted, in grey. Decorative: the row around it
 * is labelled.
 */
export function Avatar({
  avatar,
  size = 36,
  muted,
}: {
  avatar: AvatarModel;
  size?: number;
  muted?: boolean;
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
    </View>
  );
}

/** The coding agents' brand colour (the Computers mockup's ✳ tile). */
export const AGENT_COLOR = "#d97757";

const styles = StyleSheet.create({
  wrap: { flexShrink: 0 },
  avatar: { alignItems: "center", justifyContent: "center" },
  glyph: { fontFamily: ui(600) },
  prompt: { fontFamily: monoBold, letterSpacing: -0.5 },
  tile: { alignItems: "center", justifyContent: "center", flexShrink: 0 },
  tileGlyph: { fontFamily: ui(500), fontSize: 20, color: AGENT_COLOR },
  tilePrompt: { fontFamily: monoMedium, fontSize: 14, color: colors.accent },
});


/**
 * A tab's rounded-square tile (the Computers and Tabs boards): ✳ in the agent colour on a
 * 12 % tint of it for a coding agent, `>_` in the accent on its tint for a shell. Decorative: the
 * row around it is labelled.
 */
export function Tile({ agent, size = 40, radius = 12, promptSize = 14 }: { agent: boolean; size?: number; radius?: number; promptSize?: number }) {
  return (
    <View
      style={[
        styles.tile,
        { width: size, height: size, borderRadius: radius, backgroundColor: agent ? `${AGENT_COLOR}1f` : tints.accentBadge },
      ]}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      <Text
        style={agent ? styles.tileGlyph : [styles.tilePrompt, { fontSize: promptSize }]}
        allowFontScaling={false}
      >
        {agent ? "\u2733\uFE0E" : ">_"}
      </Text>
    </View>
  );
}

