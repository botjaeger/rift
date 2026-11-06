import { Text, View } from "react-native";
import type { Avatar as AvatarModel } from "@/avatar";
import { CLAUDE_COLOR, programMark } from "@/paneView";
import { avatarInk } from "./palette";
import { monoBold, monoMedium, tint, ui, makeStyles, useColors } from "./theme";

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
  const colors = useColors();
  const styles = useStyles();
  const color = muted ? colors.textMuted : avatarInk(avatar.color, colors);
  return (
    <View style={styles.wrap} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <View style={[styles.avatar, { width: size, height: size, borderRadius: size / 2, backgroundColor: muted ? colors.faint : tint(avatar.color, 0x2e) }]}>
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

const useStyles = makeStyles((colors) => ({
  wrap: { flexShrink: 0 },
  avatar: { alignItems: "center", justifyContent: "center" },
  glyph: { fontFamily: ui(600) },
  prompt: { fontFamily: monoBold, letterSpacing: -0.5 },
  tile: { alignItems: "center", justifyContent: "center", flexShrink: 0 },
  // the agent colour, held to 3:1 on its tint over a light page too
  tileGlyph: { fontFamily: ui(500), fontSize: 20, color: avatarInk(CLAUDE_COLOR, colors) },
  tilePrompt: { fontFamily: monoMedium, fontSize: 14, color: colors.accent },
}));


/**
 * A tab's rounded-square tile (the Computers and Tabs boards): ✳ in the agent colour on a
 * 12 % tint of it for a coding agent, `>_` in the accent on its tint for a shell. Decorative: the
 * row around it is labelled.
 */
export function Tile({ agent, program = null, size = 40, radius = 12, promptSize = 14 }: { agent: boolean; program?: string | null; size?: number; radius?: number; promptSize?: number }) {
  const colors = useColors();
  const styles = useStyles();
  // an agent the app has no mark for (a session title says it is one) keeps the ✳
  const mark = programMark(program);
  const claude = agent && (mark.claude || mark.mark === ">_");
  return (
    <View
      style={[
        styles.tile,
        { width: size, height: size, borderRadius: radius, backgroundColor: claude ? `${CLAUDE_COLOR}1f` : colors.tints.accentBadge },
      ]}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      <Text
        style={claude ? styles.tileGlyph : [styles.tilePrompt, { fontSize: promptSize }]}
        allowFontScaling={false}
      >
        {claude ? "\u2733\uFE0E" : mark.mark}
      </Text>
    </View>
  );
}

