import { useRouter } from "expo-router";
import { useState } from "react";
import { KeyboardAvoidingView, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parsePairLink } from "@/pairing";
import { Button } from "@/ui/Button";
import { Field } from "@/ui/Field";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { PageHeader, SectionTitle } from "@/ui/Screen";
import { ui, makeStyles, useColors } from "@/ui/theme";

/** "Paste a pairing link" (no board of its own; styled as the Pair boards). */
export default function AddScreen() {
  const colors = useColors();
  const styles = useStyles();
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
    // padding on Android too: edge to edge (Android 15) the window no longer shrinks for the
    // keyboard, so Continue sat under it
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      <PageHeader title="Paste a pairing link" back={() => (router.canGoBack() ? router.back() : router.replace("/"))} />
      <ScrollView contentContainerStyle={styles.body} keyboardShouldPersistTaps="handled">
        <Text style={styles.text}>
          On your computer: Rift › Settings › Remote access › Pair a phone, then copy the link under the code. It works once, for 2 minutes.
        </Text>
        {/* the board's caption sits on the text's own edge, 14 pt over the field */}
        <SectionTitle style={styles.caption}>Pairing link</SectionTitle>
        <Field
          icon="paste"
          label="Pairing link"
          value={text}
          onChangeText={(t) => {
            setText(t);
            setError(null);
          }}
          onSubmitEditing={next}
          placeholder="rift-remote://pair?…"
          autoCapitalize="none"
          autoCorrect={false}
          spellCheck={false}
          keyboardType="url"
          returnKeyType="go"
          accessibilityHint="Paste the link that starts with rift-remote://pair"
        />
      </ScrollView>
      <View style={[styles.actions, { paddingBottom: Math.max(16, insets.bottom) }]}>
        {/* over Continue, not under the field: with the keyboard up the scroll view ended at the
            field and the error drew below its visible part */}
        {error ? <ErrorText>{error}</ErrorText> : null}
        {/* at full strength while empty (the board): a tap then says what the link looks like */}
        <Button label="Continue" variant="primary" onPress={next} />
        <Button
          label="Scan the code instead"
          variant="text"
          icon={<Icon name="scan" color={colors.accent} />}
          onPress={() => router.replace("/scan")}
        />
      </View>
    </KeyboardAvoidingView>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  body: { paddingTop: 20, paddingHorizontal: 16, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  text: { fontFamily: ui(400), color: colors.text, fontSize: 15, lineHeight: 22 },
  caption: { marginHorizontal: -4 },
  actions: { paddingTop: 16, paddingHorizontal: 16, gap: 14 },
}));
