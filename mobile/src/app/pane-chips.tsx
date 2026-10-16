import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { hasFeature } from "@/fleet";
import { chipItems, type ChipItem } from "@/promptChips";
import { useApp, useLock } from "@/runtime";
import { findPane, paneTitle } from "@/topology";
import { Icon, type IconName } from "@/ui/Icon";
import { SheetClose } from "@/ui/SheetClose";
import { colors, mono, radius, tints, ui } from "@/ui/theme";

/**
 * "This pane": every prompt chip of the pane, those the row above the command line folded into
 * "+N" too. A row shows the chip's glyph, name and full value; one with a menu opens it.
 */
export default function PaneChipsScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { key, pane: paneParam } = useLocalSearchParams<{ key: string; pane: string }>();
  const paneId = Number(paneParam);
  const s = useApp();
  const lock = useLock();

  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);

  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const state = s.pane?.host === key && s.pane.paneId === paneId ? s.pane.state : null;
  const items = chipItems(loc?.pane ?? null, state, {
    git: hasFeature(host, "git"),
    menus: hasFeature(host, "chips"),
    promptChips: hasFeature(host, "prompt_chips"),
  });
  const hidden = lock.covered || lock.locked;
  const open = (c: ChipItem) => c.opens && router.replace({ pathname: "/chips", params: { key, pane: String(paneId), menu: c.opens } });

  return (
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      contentContainerStyle={[styles.content, { paddingBottom: Math.max(36, insets.bottom + 2) }]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <SheetClose />
      <View style={styles.head}>
        <Text style={styles.title} accessibilityRole="header">
          This pane
        </Text>
        {loc ? <Text style={styles.subtitle}>{paneTitle(loc)}</Text> : null}
      </View>
      {items.length === 0 ? (
        <Text style={styles.empty}>{loc ? "No prompt chips to show here" : "This pane is closed"}</Text>
      ) : (
        <View style={styles.card}>
          {items.map((c, i) => {
            const body = (
              <>
                <View style={styles.glyph}>
                  <Icon name={c.icon as IconName} size={14} color={c.iconColor} />
                </View>
                <Text style={styles.name} numberOfLines={1}>
                  {c.name}
                </Text>
                <View style={styles.parts}>
                  {c.full.map((p, j) => (
                    <Text key={j} style={[styles.part, { color: p.color }]}>
                      {p.text}
                    </Text>
                  ))}
                </View>
                {c.attention ? <View style={styles.dot} /> : null}
                {c.opens ? <Icon name="forward" size={14} color={colors.textMuted} /> : null}
              </>
            );
            return c.opens ? (
              <Pressable
                key={c.id}
                accessibilityRole="button"
                accessibilityLabel={c.spoken}
                onPress={() => open(c)}
                style={({ pressed }) => [styles.row, i > 0 && styles.divider, pressed && styles.pressed]}
              >
                {body}
              </Pressable>
            ) : (
              <View key={c.id} accessible accessibilityRole="text" accessibilityLabel={c.spoken} style={[styles.row, i > 0 && styles.divider]}>
                {body}
              </View>
            );
          })}
        </View>
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  content: { paddingTop: 24, paddingHorizontal: 12, gap: 14 },
  hidden: { opacity: 0 },
  head: { paddingHorizontal: 6, gap: 4 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 20 },
  subtitle: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingHorizontal: 6, paddingVertical: 12 },
  card: { backgroundColor: colors.bg, borderRadius: radius.card, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  row: { minHeight: 52, flexDirection: "row", alignItems: "center", gap: 12, paddingVertical: 8, paddingHorizontal: 14 },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  pressed: { opacity: 0.6 },
  // the 28 pt glyph box of the board: 4 % fill, hairline, radius 6
  glyph: {
    width: 28,
    height: 28,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: tints.chipFill,
    alignItems: "center",
    justifyContent: "center",
  },
  name: { width: 96, fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
  parts: { flex: 1, minWidth: 0, flexDirection: "row", flexWrap: "wrap", columnGap: 6 },
  part: { fontFamily: mono, fontSize: 13 },
  dot: { width: 6, height: 6, borderRadius: 3, backgroundColor: colors.warning },
});
