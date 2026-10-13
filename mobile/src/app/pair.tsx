import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from "react-native";
import { parsePairLink } from "@/pairing";
import { relayHost } from "@/protocol/relay";
import { fleet, useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { ErrorText, WarningText } from "@/ui/Notice";
import { InfoRow } from "@/ui/Rows";
import { Section } from "@/ui/Screen";
import { colors, mono } from "@/ui/theme";

type Phase = { t: "ready" } | { t: "pairing" } | { t: "failed"; message: string };

/**
 * Confirms a pairing link (from the scanner, a paste or a deep link) before anything is sent:
 * the device pairs only on the Pair tap.
 */
export default function PairScreen() {
  const router = useRouter();
  const params = useLocalSearchParams<{ link?: string }>();
  const s = useApp();
  const link = typeof params.link === "string" ? parsePairLink(params.link) : null;
  // The phase belongs to the link it was reached with: a new link opened while this screen
  // shows (a second scan or deep link) starts over at "ready" instead of the old error.
  const [state, setState] = useState<{ link?: string; phase: Phase }>({ phase: { t: "ready" } });
  const phase: Phase = state.link === params.link ? state.phase : { t: "ready" };
  const setPhase = (p: Phase) => setState({ link: params.link, phase: p });
  const cancelRef = useRef<(() => void) | null>(null);

  // Leaving the screen, or a new link arriving, stops a pairing attempt still in progress.
  useEffect(
    () => () => {
      cancelRef.current?.();
      cancelRef.current = null;
    },
    [params.link],
  );

  // A link this app already paired with (Android re-delivers the launching link when the activity
  // is recreated) opens its host instead of asking again.
  const spent = link !== null && fleet.pairedWith(link);
  useEffect(() => {
    if (!spent || !link) return;
    router.dismissTo("/");
    router.push({ pathname: "/host/[key]", params: { key: link.key } });
  }, [spent, link?.key, router]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!link) {
    return (
      <View style={[styles.fill, styles.content]}>
        <Text style={styles.title} accessibilityRole="header">
          Not a pairing code
        </Text>
        <Text style={styles.text}>This link is not a valid Rift pairing link. Create a new code on your computer and try again.</Text>
        <Button label="Scan again" variant="primary" onPress={() => router.replace("/scan")} />
      </View>
    );
  }

  const already = s.hosts.some((h) => h.record.key === link.key);

  const pair = async () => {
    setPhase({ t: "pairing" });
    const attempt = fleet.pair(link);
    cancelRef.current = attempt.cancel;
    const r = await attempt.result;
    cancelRef.current = null;
    if (r.ok) {
      // A deep link can make this the only screen: return to the tabs first so the host
      // screen has a way back.
      router.dismissTo("/");
      router.push({ pathname: "/host/[key]", params: { key: r.host.key } });
    } else if (r.message !== "Cancelled.") {
      setPhase({ t: "failed", message: r.message });
    }
  };

  return (
    <ScrollView style={styles.fill} contentContainerStyle={styles.content}>
      <Stack.Screen options={{ title: `Pair with ${link.name}` }} />
      <Text style={styles.title} accessibilityRole="header">
        Pair with {link.name}?
      </Text>
      <Text style={styles.text}>
        {link.permission === "operate"
          ? "This device will see this computer's terminal sessions and can run commands and send keys in them."
          : "This device will see this computer's terminal sessions. It cannot run commands or send keys."}
      </Text>
      <Section>
        <InfoRow label="Computer" first>
          <Text style={styles.value}>{link.name}</Text>
        </InfoRow>
        <InfoRow label="Access">
          <Text style={[styles.value, link.permission === "view" && { color: colors.warning }]}>
            {link.permission === "operate" ? "Operate" : "View only"}
          </Text>
        </InfoRow>
        <InfoRow label="Addresses">
          {link.addresses.map((a) => (
            <Text key={a} style={[styles.value, styles.mono]}>
              {a}
            </Text>
          ))}
        </InfoRow>
        {link.relay ? (
          <InfoRow label="Relay">
            <Text style={[styles.value, styles.mono]}>{relayHost(link.relay)}</Text>
          </InfoRow>
        ) : null}
      </Section>
      {link.relay ? (
        <Text style={styles.text}>
          Away from this network the device connects through the relay, which forwards only encrypted traffic.
        </Text>
      ) : null}
      {already ? <WarningText>This computer is already paired. Pairing again replaces it.</WarningText> : null}
      {phase.t === "failed" ? <ErrorText>{phase.message}</ErrorText> : null}
      {phase.t === "pairing" ? (
        <View style={styles.progress} accessible accessibilityLiveRegion="polite" accessibilityLabel={`Connecting to ${link.name}`}>
          <ActivityIndicator color={colors.accent} />
          <Text style={styles.text}>Connecting to {link.name}…</Text>
        </View>
      ) : null}
      <Button
        label={phase.t === "failed" ? "Try again" : "Pair"}
        variant="primary"
        disabled={phase.t === "pairing" || !s.ready || s.loadError !== null}
        onPress={() => void pair()}
      />
      <Button
        label="Cancel"
        onPress={() => {
          cancelRef.current?.();
          if (router.canGoBack()) router.back();
          else router.replace("/");
        }}
      />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 16, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  title: { color: colors.textBright, fontSize: 22, fontWeight: "600" },
  text: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  value: { color: colors.text, fontSize: 14, textAlign: "right" },
  mono: { fontFamily: mono, fontSize: 13 },
  progress: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 10 },
});
