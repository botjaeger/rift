import { Pressable, Text, View } from "react-native";
import { radius, ui, makeStyles } from "./theme";
import { toggleA11y } from "./toggle";

/**
 * The one filter (the Standard board): a 32 pt pill, the chosen one filled white, the others
 * outlined, a count after the word. A count that is only a count, not a filter, is a summary
 * line's text instead. A toggle for screen readers ("Working 2", selected).
 */
export function FilterChip({
  label,
  count,
  selected,
  onPress,
  dot,
}: {
  label: string;
  count?: number;
  selected: boolean;
  onPress: () => void;
  /** A status dot before the word (the needs-you colour). */
  dot?: string;
}) {
  const styles = useStyles();
  const spoken = count === undefined ? label : `${label} ${count}`;
  return (
    <Pressable
      {...toggleA11y(selected)}
      accessibilityLabel={spoken}
      // 32 pt, 44 to the touch
      hitSlop={{ top: 6, bottom: 6 }}
      onPress={onPress}
      style={({ pressed }) => [styles.chip, selected ? styles.on : styles.off, pressed && styles.pressed]}
    >
      {dot ? <View style={[styles.dot, { backgroundColor: dot }]} /> : null}
      <Text style={[styles.text, selected && styles.textOn]}>
        {label}
        {count === undefined ? null : <Text style={styles.count}>{` ${count}`}</Text>}
      </Text>
    </Pressable>
  );
}

const useStyles = makeStyles((colors) => ({
  chip: { height: 32, flexDirection: "row", alignItems: "center", gap: 6, paddingHorizontal: 12, borderRadius: radius.chip },
  on: { backgroundColor: colors.textBright },
  off: { borderWidth: 1, borderColor: colors.borderStrong },
  pressed: { opacity: 0.75 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  text: { fontFamily: ui(500), color: colors.text, fontSize: 13 },
  textOn: { fontFamily: ui(600), color: colors.bg },
  count: { opacity: 0.7 },
}));
