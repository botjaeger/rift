import { useRouter } from "expo-router";
import { useEffect, useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { hostMarks, nowSections } from "@/now";
import { setNowFilter, useNowFilter } from "@/nowFilter";
import { useApp, useLock } from "@/runtime";
import { SheetClose } from "@/ui/SheetClose";
import { colors, ui } from "@/ui/theme";

/** "Show sessions from" (Now's computer picker): every computer, or one; each with what it is doing. */
export default function PickComputerScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const lock = useLock();
  const only = useNowFilter();
  const marks = useMemo(() => hostMarks(s.hosts), [s.hosts]);

  // The sheet is presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);

  const status = (key: string | null) => {
    const host = key === null ? null : s.hosts.find((h) => h.record.key === key);
    if (host && host.conn.status !== "open") return "Not connected";
    const n = nowSections(s.hosts, [], s.asked, key);
    return [`${n.running.length} working`, n.needs.length ? `${n.needs.length} needs you` : null].filter(Boolean).join(" · ");
  };
  const pick = (key: string | null) => {
    setNowFilter(key);
    router.back();
  };
  const options = [
    { key: null, name: `All ${s.hosts.length} computers`, dots: s.hosts.map((h) => marks.get(h.record.key)!.color) },
    ...s.hosts.map((h) => ({ key: h.record.key, name: h.record.name, dots: [marks.get(h.record.key)!.color] })),
  ];

  return (
    <View style={[styles.body, { paddingBottom: Math.max(36, insets.bottom + 2) }]}>
      <SheetClose />
      <Text style={styles.title} accessibilityRole="header">
        Show sessions from
      </Text>
      <View accessibilityRole="radiogroup">
        {options.map((o) => {
          const selected = o.key === only;
          return (
            <Pressable
              key={o.key ?? "all"}
              accessibilityRole="radio"
              accessibilityState={{ checked: selected }}
              accessibilityLabel={`${o.name}, ${status(o.key)}`}
              onPress={() => pick(o.key)}
              style={({ pressed }) => [styles.option, selected && styles.selected, pressed && styles.pressed]}
            >
              <View style={styles.dots}>
                {o.dots.map((c, i) => (
                  <View key={i} style={[styles.dot, { backgroundColor: c }, i > 0 && styles.stacked]} />
                ))}
              </View>
              <View style={styles.text}>
                <Text style={styles.name} numberOfLines={1}>
                  {o.name}
                </Text>
                <Text style={styles.meta} numberOfLines={1}>
                  {status(o.key)}
                </Text>
              </View>
              {selected ? <Text style={styles.check}>✓</Text> : null}
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  body: { backgroundColor: colors.surface, paddingTop: 20, paddingHorizontal: 14, gap: 12 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 17, paddingHorizontal: 4 },
  option: { flexDirection: "row", alignItems: "center", gap: 12, padding: 12, borderRadius: 12, minHeight: 56 },
  selected: { backgroundColor: colors.glass },
  pressed: { opacity: 0.75 },
  dots: { flexDirection: "row", minWidth: 18 },
  dot: { width: 18, height: 18, borderRadius: 9, borderWidth: 2, borderColor: colors.surface },
  stacked: { marginLeft: -6 },
  text: { flex: 1, minWidth: 0, gap: 2 },
  name: { fontFamily: ui(500), color: colors.textBright, fontSize: 15 },
  meta: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  check: { fontFamily: ui(600), color: colors.accent, fontSize: 16 },
});
