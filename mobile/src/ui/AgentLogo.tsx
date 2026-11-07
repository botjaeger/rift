import { Image } from "react-native";
import type { AgentLogo as Logo } from "@/paneView";
import claude16 from "../../assets/agents/claude-16.png";
import claude20 from "../../assets/agents/claude-20.png";
import codex16 from "../../assets/agents/codex-16.png";
import codex20 from "../../assets/agents/codex-20.png";
import opencode16 from "../../assets/agents/opencode-16.png";
import opencode20 from "../../assets/agents/opencode-20.png";
import { useColors } from "./theme";

/** The two sizes the boards draw a logo at: 16 pt in a sheet's 28 pt tile, 20 pt in a list's 36 pt one. */
export type LogoSize = 16 | 20;

const SOURCES = {
  claude: { 16: claude16, 20: claude20 },
  codex: { 16: codex16, 20: codex20 },
  opencode: { 16: opencode16, 20: opencode20 },
} as const;

/**
 * A coding agent's logo (Lobe Icons 1.102.0, MIT: mobile/assets/agents/LICENSE), rendered to
 * PNG at 1x, 2x and 3x for each size: Claude Code and Codex in their own colours, OpenCode's
 * one-colour mark in the accent (`tint` overrides: a muted avatar). Decorative: the row
 * around it is labelled.
 */
export function AgentLogo({ logo, size, tint }: { logo: Logo; size: LogoSize; tint?: string }) {
  const colors = useColors();
  const color = tint ?? (logo === "opencode" ? colors.accent : undefined);
  return (
    <Image
      source={SOURCES[logo][size]}
      style={[{ width: size, height: size }, color ? { tintColor: color } : null]}
      accessibilityElementsHidden
      importantForAccessibility="no"
    />
  );
}
