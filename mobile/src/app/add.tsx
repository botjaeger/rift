import { useRouter } from "expo-router";
import { useState } from "react";
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parsePairLink } from "@/pairing";
import { Button } from "@/ui/Button";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { PageHeader } from "@/ui/Screen";
import { colors, mono, radius, ui } from "@/ui/theme";

/** "Paste a pairing link" (no board of its own; styled as the Pair boards). */
export default function AddScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);

  const next = () => {
    const link = text.trim();
    if (!parsePairLink(link)) {
      setError("Not a Rift pairing link. It starts with rift-remote://pair?");
      return;
    }
    setError(null);
    router.replace({ pathname: "/pair", params: { link } });
  };

  return (
    <KeyboardAvoidingView style={styles.fill} behavior={Platform.OS === "ios" ? "padding" : undefined}>
      <PageHeader title="Paste a pairing link" back={() => (router.canGoBack() ? router.back() : router.replace("/"))} />
      <ScrollView contentContainerStyle={styles.body} keyboardShouldPersistTaps="handled">
        <Text style={styles.text}>
          On your computer: Rift › Settings › Remote access › Pair a phone, then copy the link under the code (or run{" "}
          <Text style={styles.code}>rift-terminal remote pair</Text>). It works once, for 2 minutes.
        </Text>
        <View style={styles.field}>
          <Text style={styles.label}>Pairing link</Text>
          <TextInput
            value={text}
            onChangeText={(t) => {
              setText(t);
              setError(null);
            }}
            onSubmitEditing={next}
            placeholder="rift-remote://pair?a=…"
            placeholderTextColor={colors.textMuted}
            autoCapitalize="none"
            autoCorrect={false}
            spellCheck={false}
            keyboardType="url"
            returnKeyType="go"
            accessibilityLabel="Pairing link"
            accessibilityHint="Paste the link that starts with rift-remote://pair"
            style={styles.input}
          />
        </View>
        {error ? <ErrorText>{error}</ErrorText> : null}
      </ScrollView>
      <View style={[styles.actions, { paddingBottom: Math.max(40, insets.bottom + 6) }]}>
        <Button label="Continue" variant="primary" size="large" onPress={next} disabled={text.trim() === ""} />
        <Button
          label="Scan the code instead"
          variant="link"
          icon={<Icon name="scan" color={colors.accent} />}
          onPress={() => router.replace("/scan")}
        />
      </View>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.chrome },
  body: { paddingTop: 20, paddingHorizontal: 20, gap: 22, maxWidth: 560, width: "100%", alignSelf: "center" },
  text: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15, lineHeight: 22.5 },
  code: { fontFamily: mono, color: colors.text },
  field: { gap: 4 },
  label: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  input: {
    height: 44,
    paddingHorizontal: 10,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontFamily: mono,
    fontSize: 14,
  },
  actions: { paddingTop: 16, paddingHorizontal: 20, gap: 10 },
});
