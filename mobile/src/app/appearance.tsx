import { View } from "react-native";
import { computerDetail } from "@/appearance";
import { fleet, useApp } from "@/runtime";
import type { Appearance } from "@/stores";
import { BUILTIN_THEMES } from "@/themes.data";
import { CardBox, ChoiceRow, Group, SettingsPage } from "@/ui/Rows";
import { SaveError } from "@/ui/SaveError";
import { makeStyles, paletteOf, useColors } from "@/ui/theme";

/** A theme's swatch: its ground with its accent as a dot (the Themes board). */
function Swatch({ bg, accent }: { bg: string; accent: string }) {
  const styles = useStyles();
  return (
    <View style={[styles.swatch, { backgroundColor: bg }]} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
      <View style={[styles.swatchDot, { backgroundColor: accent }]} />
    </View>
  );
}

/**
 * Settings › Appearance (the Themes board): the computer's theme (the default), the phone's
 * light or dark, or one of the desktop's builtin themes. The choice stays on this phone.
 */
export default function AppearanceScreen() {
  const colors = useColors();
  const s = useApp();
  const a = s.settings.appearance;
  const host = s.lastView ? s.hosts.find((h) => h.record.key === s.lastView!.host) : undefined;
  const set = (appearance: Appearance) => void fleet.updateSettings({ appearance });
  // "Choose a theme" starts on the theme shown now when it is a builtin one
  const chosen = a.mode === "theme" ? a.theme : BUILTIN_THEMES.some((t) => t.name === colors.name) ? colors.name : "Dark Default";
  return (
    <SettingsPage title="Appearance" notice={<SaveError />}>
      <Group title="Theme" footer="A choice here stays on this phone and never changes a computer.">
        <CardBox>
          <View accessibilityRole="radiogroup" accessibilityLabel="Theme">
            <ChoiceRow
              first
              label="Same as the computer"
              detail={computerDetail(host ? { name: host.record.name, theme: host.theme } : null, s.lastTheme)}
              selected={a.mode === "computer"}
              onSelect={() => set({ mode: "computer" })}
            />
            <ChoiceRow
              label="Match this phone"
              detail="Light Default in light mode, Dark Default in dark mode"
              selected={a.mode === "phone"}
              onSelect={() => set({ mode: "phone" })}
            />
            <ChoiceRow
              label="Choose a theme"
              detail={`The same ${BUILTIN_THEMES.length} themes as the terminal, below`}
              selected={a.mode === "theme"}
              onSelect={() => set({ mode: "theme", theme: chosen })}
            />
          </View>
        </CardBox>
      </Group>
      <Group title="Themes" footer="With several computers paired, a pane uses its own computer's theme; Portals and Settings use the last one opened.">
        <CardBox>
          <View accessibilityRole="radiogroup" accessibilityLabel="Themes">
            {BUILTIN_THEMES.map((t, i) => {
              const p = paletteOf(t);
              return (
                <ChoiceRow
                  key={t.name}
                  first={i === 0}
                  label={t.name}
                  detail={t.dark ? "Dark" : "Light"}
                  leading={<Swatch bg={p.bg} accent={p.accent} />}
                  selected={a.mode === "theme" && a.theme === t.name}
                  onSelect={() => set({ mode: "theme", theme: t.name })}
                />
              );
            })}
          </View>
        </CardBox>
      </Group>
    </SettingsPage>
  );
}

const useStyles = makeStyles((colors) => ({
  swatch: {
    width: 28,
    height: 28,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  swatchDot: { width: 10, height: 10, borderRadius: 5 },
}));
