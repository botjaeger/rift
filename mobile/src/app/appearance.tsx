import { Pressable, Text, View } from "react-native";
import { computerDetail } from "@/appearance";
import { fleet, useApp } from "@/runtime";
import type { Appearance } from "@/stores";
import { BUILTIN_THEMES } from "@/themes.data";
import { Icon } from "@/ui/Icon";
import { CardBox, CHEVRON, ChoiceRow, Group, SettingsPage } from "@/ui/Rows";
import { SaveError } from "@/ui/SaveError";
import { makeStyles, paletteOf, tint, ui, useColors } from "@/ui/theme";

/** The Themes board's choice rows: 60 pt and 8 over and under. */
const THEME_ROW = 76;

/** Themes per row of the swatch grid (the Themes board). */
export const SWATCH_COLUMNS = 5;

/**
 * A theme's swatch (the Themes board): its ground, 40 pt tall, with its accent as a 10 pt dot in
 * the corner; a tap chooses it. The chosen one is ringed in the accent.
 */
function Swatch({ name, bg, accent, chosen, onPress }: { name: string; bg: string; accent: string; chosen: boolean; onPress: () => void }) {
  const styles = useStyles();
  const colors = useColors();
  return (
    <Pressable
      accessibilityRole="radio"
      accessibilityLabel={name}
      accessibilityState={{ checked: chosen }}
      aria-checked={chosen}
      onPress={onPress}
      style={({ pressed }) => [styles.swatch, { backgroundColor: bg }, chosen && { borderColor: colors.accent, borderWidth: 2 }, pressed && styles.pressed]}
    >
      <View style={[styles.swatchDot, { backgroundColor: accent }]} />
    </Pressable>
  );
}

/**
 * Settings › Appearance (the Themes board): the computer's theme (the default), the phone's
 * light or dark, or one of the desktop's builtin themes, picked from the swatch grid under the
 * choices. The choice stays on this phone.
 */
export default function AppearanceScreen() {
  const colors = useColors();
  const styles = useStyles();
  const s = useApp();
  const a = s.settings.appearance;
  const host = s.lastView ? s.hosts.find((h) => h.record.key === s.lastView!.host) : undefined;
  const set = (appearance: Appearance) => void fleet.updateSettings({ appearance });
  // "Choose a theme" starts on the theme shown now when it is a builtin one
  const chosen = a.mode === "theme" ? a.theme : BUILTIN_THEMES.some((t) => t.name === colors.name) ? colors.name : "Dark Default";
  return (
    <SettingsPage title="Appearance" notice={<SaveError />}>
      <Group title="Theme">
        <CardBox>
          <View accessibilityRole="radiogroup" accessibilityLabel="Theme">
            <ChoiceRow
              first
              label="Same as the computer"
              detail={computerDetail(host ? { name: host.record.name, theme: host.theme } : null, s.lastTheme)}
              minHeight={THEME_ROW}
              selected={a.mode === "computer"}
              onSelect={() => set({ mode: "computer" })}
            />
            <ChoiceRow
              label="Match this phone"
              detail="Light Default by day, Dark Default at night"
              minHeight={THEME_ROW}
              selected={a.mode === "phone"}
              onSelect={() => set({ mode: "phone" })}
            />
            <ChoiceRow
              label="Choose a theme"
              detail={`The same ${BUILTIN_THEMES.length} themes as the terminal`}
              minHeight={THEME_ROW}
              selected={a.mode === "theme"}
              trailing={
                <View style={styles.chevron} importantForAccessibility="no">
                  <Icon name="forward" size={CHEVRON} color={colors.textMuted} />
                </View>
              }
              onSelect={() => set({ mode: "theme", theme: chosen })}
            />
          </View>
        </CardBox>
      </Group>
      <View style={styles.grid} accessibilityRole="radiogroup" accessibilityLabel="Themes">
        {BUILTIN_THEMES.map((t) => {
          const p = paletteOf(t);
          return (
            <View key={t.name} style={styles.cell}>
              <Swatch name={t.name} bg={p.bg} accent={p.accent} chosen={a.mode === "theme" && a.theme === t.name} onPress={() => set({ mode: "theme", theme: t.name })} />
            </View>
          );
        })}
      </View>
      <Text style={styles.foot}>With several computers paired, a pane uses its own computer&apos;s theme; Portals and Settings use the last one opened.</Text>
    </SettingsPage>
  );
}

// The board: the grid 14 pt under the card and 4 pt further in, five columns 8 pt apart; the note 14 pt under it.
const useStyles = makeStyles((colors) => ({
  chevron: { width: 20, alignItems: "center" },
  grid: { flexDirection: "row", flexWrap: "wrap", marginTop: -4 },
  cell: { width: `${100 / SWATCH_COLUMNS}%`, paddingHorizontal: 4, paddingBottom: 8 },
  swatch: {
    height: 40,
    borderRadius: 10,
    borderWidth: 1,
    // the board's 20 % rim, on light and dark grounds alike
    borderColor: tint(colors.textBright, 0x33),
    alignItems: "flex-end",
    justifyContent: "flex-end",
    padding: 4,
  },
  swatchDot: { width: 10, height: 10, borderRadius: 5 },
  pressed: { opacity: 0.7 },
  foot: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 19, paddingHorizontal: 4, marginTop: -12 },
}));
