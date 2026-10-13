import { useRouter } from "expo-router";
import { useState } from "react";
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, TextInput } from "react-native";
import { parsePairLink } from "@/pairing";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { colors, mono, radius, TOUCH } from "@/ui/theme";

export default function AddScreen() {
  const router = useRouter();
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
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text style={styles.lead}>
          On your computer, open Rift Settings › Remote access › Pair a phone, or run{" "}
          <Text style={styles.code}>rift-terminal remote pair</Text>. The code is valid for 2 minutes and works once.
        </Text>
        <Button label="Scan the QR code" variant="primary" onPress={() => router.push("/scan")} />

        <Text style={styles.label}>Or paste the pairing link</Text>
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
        {error ? <ErrorText>{error}</ErrorText> : null}
        <Button label="Continue" onPress={next} disabled={text.trim() === ""} />
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 16, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  lead: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  code: { fontFamily: mono, color: colors.text },
  label: { color: colors.textMuted, fontSize: 13, fontWeight: "500", marginTop: 12 },
  input: {
    minHeight: TOUCH,
    paddingHorizontal: 12,
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.surface,
    color: colors.textBright,
    fontFamily: mono,
    fontSize: 14,
  },
});
