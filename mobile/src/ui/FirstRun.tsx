import { useRouter } from "expo-router";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { headerTop } from "./Screen";
import { ui, makeStyles, useColors } from "./theme";

const STEPS = [
  "Open Rift on your Mac or Linux computer.",
  "Go to Settings › Remote access and turn it on.",
  "Choose Pair a phone and scan the code it shows.",
];

/**
 * First run (the New Portals board, no computer yet): the large title on the header's chrome,
 * then how to connect as a plain list of steps, then Scan the code and Paste a pairing link.
 */
export function FirstRun() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  return (
    <View style={styles.firstRun}>
      <View style={[styles.header, { paddingTop: headerTop(insets.top) }]}>
        <Text style={styles.firstTitle} accessibilityRole="header">
          Portals
        </Text>
      </View>
      {/* scrolls on a short phone (320 × 568), where the steps would run under the buttons */}
      <ScrollView style={styles.flex} contentContainerStyle={styles.firstBody}>
        <Text style={styles.h2} accessibilityRole="header">
          Connect your computer
        </Text>
        <View style={styles.steps} accessibilityRole="list">
          {STEPS.map((text, i) => (
            <View key={i} style={styles.step} accessible>
              <View style={styles.stepNo}>
                <Text style={styles.stepNoText}>{i + 1}</Text>
              </View>
              <Text style={styles.stepText}>{text}</Text>
            </View>
          ))}
        </View>
      </ScrollView>
      {/* the board's 28 pt under Paste a pairing link, clear of the home indicator */}
      <View style={[styles.firstActions, { paddingBottom: Math.max(28, insets.bottom - 6) }]}>
        <Button label="Scan the code" variant="primary" icon={<Icon name="scan" color={colors.onAccent} />} onPress={() => router.push("/scan")} />
        <Button label="Paste a pairing link" variant="text" onPress={() => router.push("/add")} />
      </View>
    </View>
  );
}

// The board: a hairline under the header, the #050505 page, 20 pt in, 22 pt between the parts.
const useStyles = makeStyles((colors) => ({
  flex: { flex: 1, minWidth: 0 },
  firstRun: { flex: 1, backgroundColor: colors.bg },
  header: { paddingHorizontal: 14, paddingBottom: 12, backgroundColor: colors.chrome, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  firstTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 30, lineHeight: 36, letterSpacing: -0.5 },
  firstBody: { paddingTop: 24, paddingHorizontal: 20, gap: 22 },
  h2: { fontFamily: ui(600), color: colors.textBright, fontSize: 22, lineHeight: 28 },
  steps: { gap: 22 },
  step: { flexDirection: "row", gap: 14 },
  stepNo: { width: 28, height: 28, borderRadius: 14, backgroundColor: colors.tints.accentBadge, alignItems: "center", justifyContent: "center" },
  stepNoText: { fontFamily: ui(600), color: colors.accent, fontSize: 14 },
  stepText: { fontFamily: ui(400), flex: 1, color: colors.text, fontSize: 16, lineHeight: 23 },
  firstActions: { paddingTop: 22, paddingHorizontal: 20, gap: 22 },
}));
