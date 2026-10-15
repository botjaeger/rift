import { useRouter } from "expo-router";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { tabBottom, titleTop } from "./Screen";
import { colors, tints, ui } from "./theme";

/** First run (the Empty board): how to connect, then Scan the code. */
export function FirstRun() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const steps = [
    <Text key="1" style={styles.stepText}>
      Open Rift on your Mac or Linux computer.
    </Text>,
    <Text key="2" style={styles.stepText}>
      Settings › Remote access: turn on <Text style={styles.bold}>Allow remote access</Text>, then{" "}
      <Text style={styles.bold}>Pair a phone</Text>. A code appears.
    </Text>,
    <Text key="3" style={styles.stepText}>
      Scan it here. The code works once, for 2 minutes.
    </Text>,
  ];
  return (
    <View style={[styles.firstRun, { paddingTop: titleTop(insets.top) }]}>
      <Text style={styles.firstTitle} accessibilityRole="header">
        Computers
      </Text>
      {/* scrolls on a short phone (320 × 568), where the steps would run under the buttons */}
      <ScrollView style={styles.flex} contentContainerStyle={styles.firstBody}>
        <View style={styles.intro}>
          <Text style={styles.h2} accessibilityRole="header">
            Connect to Rift on your computer
          </Text>
          <Text style={styles.lead}>Watch its panes, run commands and get told when they finish.</Text>
        </View>
        <View style={styles.steps} accessibilityRole="list">
          {steps.map((text, i) => (
            <View key={i} style={[styles.step, i > 0 && styles.stepDivider]} accessible>
              <View style={styles.stepNo}>
                <Text style={styles.stepNoText}>{i + 1}</Text>
              </View>
              {text}
            </View>
          ))}
        </View>
      </ScrollView>
      {/* above the home indicator */}
      <View style={[styles.firstActions, { paddingBottom: tabBottom(insets.bottom) }]}>
        <Button
          label="Scan the code"
          variant="primary"
          size="large"
          icon={<Icon name="scan" color={colors.bg} />}
          onPress={() => router.push("/scan")}
        />
        <Button label="Paste a pairing link" variant="link" onPress={() => router.push("/add")} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, minWidth: 0 },
  firstRun: { flex: 1, backgroundColor: colors.chrome },
  firstTitle: {
    fontFamily: ui(600),
    color: colors.textBright,
    fontSize: 32,
    letterSpacing: -0.5,
    paddingHorizontal: 20,
    paddingBottom: 8,
  },
  firstBody: { paddingTop: 24, paddingBottom: 8, paddingHorizontal: 20, gap: 22 },
  intro: { gap: 8 },
  h2: { fontFamily: ui(600), color: colors.textBright, fontSize: 22, lineHeight: 27.5 },
  lead: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15, lineHeight: 22.5 },
  steps: {
    backgroundColor: colors.surface,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  step: { flexDirection: "row", gap: 14, padding: 14 },
  stepDivider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  stepNo: {
    width: 24,
    height: 24,
    borderRadius: 12,
    backgroundColor: tints.accentBadge,
    alignItems: "center",
    justifyContent: "center",
  },
  stepNoText: { fontFamily: ui(600), color: colors.accent, fontSize: 13 },
  stepText: { fontFamily: ui(400), flex: 1, color: colors.text, fontSize: 14, lineHeight: 20.3 },
  bold: { fontFamily: ui(600), color: colors.textBright },
  firstActions: { paddingTop: 16, paddingHorizontal: 20, gap: 10 },
});
