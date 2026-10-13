import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { CONTROL_KEYS, DEFAULT_CONTROL_KEYS, controlKey, moveKey, toggleKey } from "@/controlKeys";
import { fleet, useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { Row, SwitchRow } from "@/ui/Rows";
import { Section } from "@/ui/Screen";
import { colors, mono, radius, TOUCH } from "@/ui/theme";

function MoveButton({
  label,
  spoken,
  disabled,
  onPress,
}: {
  label: string;
  spoken: string;
  disabled: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={spoken}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.move, disabled && styles.disabled, pressed && styles.pressed]}
    >
      <Text style={styles.moveLabel}>{label}</Text>
    </Pressable>
  );
}

/** Settings › Control keys: which keys the row holds and in what order. */
export default function ControlKeysScreen() {
  const insets = useSafeAreaInsets();
  const s = useApp();
  const ids = s.settings.controlKeys;
  const set = (controlKeys: string[]) => void fleet.updateSettings({ controlKeys });
  const unused = CONTROL_KEYS.filter((k) => !ids.includes(k.id));
  const isDefault = ids.length === DEFAULT_CONTROL_KEYS.length && ids.every((id, i) => id === DEFAULT_CONTROL_KEYS[i]);

  return (
    <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}>
      <Section
        title="In the row"
        footer="Left to right. ctrl and alt apply to the next key you press or letter you type."
      >
        {ids.length === 0 ? (
          <Row first>
            <Text style={styles.empty}>No keys: the row is hidden.</Text>
          </Row>
        ) : null}
        {ids.map((id, i) => {
          const k = controlKey(id);
          if (!k) return null;
          return (
            <Row key={id} first={i === 0}>
              <Text style={styles.cap} importantForAccessibility="no">
                {k.label}
              </Text>
              <Text style={styles.name} accessibilityLabel={`${k.spoken}, position ${i + 1} of ${ids.length}`}>
                {k.spoken}
              </Text>
              <MoveButton
                label="↑"
                spoken={`Move ${k.spoken} up, earlier in the row`}
                disabled={i === 0}
                onPress={() => set(moveKey(ids, i, -1))}
              />
              <MoveButton
                label="↓"
                spoken={`Move ${k.spoken} down, later in the row`}
                disabled={i === ids.length - 1}
                onPress={() => set(moveKey(ids, i, 1))}
              />
              <MoveButton
                label="✕"
                spoken={`Remove ${k.spoken}`}
                disabled={false}
                onPress={() => set(toggleKey(ids, id, false))}
              />
            </Row>
          );
        })}
      </Section>

      <Section title="More keys" footer="Turn a key on to add it at the end of the row.">
        {unused.map((k, i) => (
          <SwitchRow
            key={k.id}
            first={i === 0}
            // a key whose cap already says it (F5) is named once
            label={k.label === k.spoken ? k.label : `${k.label} · ${k.spoken}`}
            value={false}
            onChange={() => set(toggleKey(ids, k.id, true))}
          />
        ))}
      </Section>

      <View style={styles.reset}>
        <Button
          label="Reset to the default row"
          disabled={isDefault}
          accessibilityHint="^C, esc, tab, ctrl and the arrows"
          onPress={() => set([...DEFAULT_CONTROL_KEYS])}
        />
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 16, gap: 20, maxWidth: 720, width: "100%", alignSelf: "center" },
  cap: {
    minWidth: 52,
    paddingHorizontal: 6,
    paddingVertical: 4,
    textAlign: "center",
    color: colors.text,
    fontFamily: mono,
    fontSize: 14,
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  name: { flex: 1, color: colors.text, fontSize: 15 },
  empty: { flex: 1, color: colors.textMuted, fontSize: 14 },
  move: { minWidth: TOUCH, minHeight: TOUCH, alignItems: "center", justifyContent: "center" },
  moveLabel: { color: colors.accent, fontSize: 18 },
  disabled: { opacity: 0.35 },
  pressed: { opacity: 0.6 },
  reset: { paddingHorizontal: 4 },
});
