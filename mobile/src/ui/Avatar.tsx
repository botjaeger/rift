import { Text, View } from "react-native";
import type { Avatar as AvatarModel } from "@/avatar";
import { CLAUDE_COLOR, programMark, type AgentLogo as Logo } from "@/paneView";
import { AgentLogo, type LogoSize } from "./AgentLogo";
import { avatarInk } from "./palette";
import { monoBold, monoMedium, tint, makeStyles, useColors } from "./theme";

/**
 * A pane's round mark (the Computers and Host boards): the prompt mark `>_`, or Claude Code's
 * logo, in the avatar's colour on an 18 % tint of it; muted, in grey. Decorative: the row around
 * it is labelled.
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
        {avatar.logo ? (
          <AgentLogo logo={avatar.logo} size={size >= 36 ? 20 : 16} tint={muted ? colors.textMuted : undefined} />
        ) : (
          <Text style={[styles.prompt, { color, fontSize: size * 0.36 }]} allowFontScaling={false}>
            {">_"}
          </Text>
        )}
      </View>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  wrap: { flexShrink: 0 },
  avatar: { alignItems: "center", justifyContent: "center" },
  prompt: { fontFamily: monoBold, letterSpacing: -0.5 },
  tile: { alignItems: "center", justifyContent: "center", flexShrink: 0 },
  tilePrompt: { fontFamily: monoMedium, fontSize: 12, color: colors.accent },
}));

/**
 * A tab's rounded-square tile (the Portals, Panes and Sheets boards): a coding agent's logo (20
 * pt in a list's 36 pt tile, 16 pt in a sheet's 28 pt one), Claude Code's on a 12 % tint of its
 * orange; `>_` in the accent on its tint for a shell. `plain`: a sheet's grey tile, whatever the
 * program. Decorative: the row around it is labelled.
 */
export function Tile({
  agent,
  program = null,
  size = 36,
  radius = 10,
  promptSize = 12,
  logoSize = 20,
  plain,
}: {
  agent: boolean;
  program?: string | null;
  size?: number;
  radius?: number;
  promptSize?: number;
  logoSize?: LogoSize;
  plain?: boolean;
}) {
  const colors = useColors();
  const styles = useStyles();
  // an agent the app has no logo for (a session title says it is one) keeps Claude Code's
  const logo: Logo | null = programMark(program).logo ?? (agent ? "claude" : null);
  const ground = plain ? colors.faint : logo === "claude" ? `${CLAUDE_COLOR}1f` : colors.tints.accentBadge;
  return (
    <View
      style={[styles.tile, { width: size, height: size, borderRadius: radius, backgroundColor: ground }]}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {logo ? (
        <AgentLogo logo={logo} size={logoSize} />
      ) : (
        <Text style={[styles.tilePrompt, { fontSize: promptSize }]} allowFontScaling={false}>
          {">_"}
        </Text>
      )}
    </View>
  );
}
