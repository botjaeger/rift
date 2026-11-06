import { useActionScope } from "@/ui/useActionScope";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parsePairLink } from "@/pairing";
import { addressHost, relayHost } from "@/protocol/relay";
import { fleet, useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { Icon } from "@/ui/Icon";
import { ErrorText, WarningText } from "@/ui/Notice";
import { PageHeader, SectionTitle } from "@/ui/Screen";
import { mono, radius, ui, makeStyles, useColors } from "@/ui/theme";

type Phase = { t: "ready" } | { t: "pairing" } | { t: "failed"; message: string };

/** The computer's key as people compare it: its first 16 characters in fours ("R45t cRB3 9vYh rDdE"). */
function keyGroups(key: string): string {
  return (key.slice(0, 16).match(/.{1,4}/g) ?? []).join(" ");
}

/** One label / value line of the details card. */
function Detail({ label, children, first, monoValue }: { label: string; children: string; first?: boolean; monoValue?: boolean }) {
  const styles = useStyles();
  return (
    <View style={[styles.detail, !first && styles.divider]} accessible accessibilityLabel={`${label}: ${children}`}>
      <Text style={styles.dt}>{label}</Text>
      <Text style={[styles.dd, monoValue && styles.mono]}>{children}</Text>
    </View>
  );
}

/**
 * "Pair with <computer>?" (the PairConfirm board), for a link from the scanner, a paste or a
 * deep link: nothing is sent until Pair.
 */
export default function PairScreen() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const params = useLocalSearchParams<{ link?: string }>();
  const scope = useActionScope(params.link);
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
  // is recreated) goes to Portals, where the computer's card is, instead of asking again.
  const spent = link !== null && fleet.pairedWith(link);
  useEffect(() => {
    if (!spent || !link) return;
    router.dismissTo("/");
  }, [spent, link?.key, router]); // eslint-disable-line react-hooks/exhaustive-deps

  const cancel = () => {
    if (!scope.leave()) return;
    cancelRef.current?.();
    if (router.canGoBack()) router.back();
    else router.replace("/");
  };

  if (!link) {
    return (
      <View style={styles.fill}>
        <PageHeader title="Not a pairing code" back={cancel} />
        <View style={styles.body}>
          <Text style={styles.text}>This link is not a valid Rift pairing link. Create a new code on your computer and try again.</Text>
        </View>
        <View style={[styles.actions, { paddingBottom: Math.max(40, insets.bottom + 6) }]}>
          <Button label="Scan again" variant="primary" style={styles.flex} onPress={() => router.replace("/scan")} />
        </View>
      </View>
    );
  }

  const already = s.hosts.some((h) => h.record.key === link.key);

  const pair = async () => {
    const current = scope.capture();
    setPhase({ t: "pairing" });
    const attempt = fleet.pair(link);
    cancelRef.current = attempt.cancel;
    const r = await attempt.result;
    if (!current()) return;
    cancelRef.current = null;
    if (r.ok) {
      // Portals: the new computer's card, its tabs in the redesigned rows (a deep link can make
      // this the only screen, so dismiss to the root rather than go back)
      router.dismissTo("/");
    } else if (r.message !== "Cancelled.") {
      setPhase({ t: "failed", message: r.message });
    }
  };

  const addresses = link.addresses.map((a) => `${addressHost(a)} · LAN`);
  return (
    <View style={styles.fill}>
      <PageHeader title={`Pair with ${link.name}?`} back={cancel} />
      <ScrollView contentContainerStyle={styles.body}>
        <View style={styles.intro}>
          <View style={styles.tile}>
            <Icon name="computer" size={26} color={colors.textMuted} />
          </View>
          <Text style={[styles.text, styles.flex]}>
            {link.permission === "operate"
              ? "This phone will see its tabs and panes and can run commands there."
              : "This phone will see its tabs and panes. It cannot run commands or send keys."}
          </Text>
        </View>
        <SectionTitle>From the code</SectionTitle>
        <View style={styles.card}>
          <Detail label="Computer" first>
            {link.name}
          </Detail>
          {addresses.map((a, i) => (
            <Detail key={a} label={i === 0 ? "Address" : ""}>
              {a}
            </Detail>
          ))}
          {link.relay ? <Detail label="Relay">{relayHost(link.relay)}</Detail> : null}
          <Detail label="Access">{link.permission === "operate" ? "Can run commands" : "View only"}</Detail>
          <Detail label="Key" monoValue>
            {keyGroups(link.key)}
          </Detail>
        </View>
        <Text style={styles.note}>Check the key matches the one on your computer’s screen.</Text>
        {link.relay ? (
          <Text style={styles.note}>Away from this network the phone connects through the relay, which forwards only encrypted traffic.</Text>
        ) : null}
        {already ? <WarningText>This computer is already paired. Pairing again replaces it.</WarningText> : null}
        {phase.t === "failed" ? <ErrorText>{phase.message}</ErrorText> : null}
        {phase.t === "pairing" ? (
          <View style={styles.progress} accessible accessibilityLiveRegion="polite" accessibilityLabel={`Connecting to ${link.name}`}>
            <ActivityIndicator color={colors.accent} />
            <Text style={styles.text}>Connecting to {link.name}…</Text>
          </View>
        ) : null}
      </ScrollView>
      <View style={[styles.actions, { paddingBottom: Math.max(40, insets.bottom + 6) }]}>
        <Button label="Cancel" style={styles.cancel} onPress={cancel} />
        <Button
          label={phase.t === "failed" ? "Try again" : "Pair"}
          variant="primary"
          style={styles.pair}
          disabled={phase.t === "pairing" || !s.ready || s.loadError !== null}
          onPress={() => void pair()}
        />
      </View>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.chrome },
  flex: { flex: 1 },
  body: { paddingTop: 20, paddingHorizontal: 16, gap: 18, maxWidth: 560, width: "100%", alignSelf: "center" },
  tile: {
    width: 56,
    height: 56,
    borderRadius: radius.card,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
    alignItems: "center",
    justifyContent: "center",
  },
  intro: { flexDirection: "row", alignItems: "center", gap: 14 },
  text: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15, lineHeight: 22 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 19 },
  card: {
    backgroundColor: colors.surface,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  // the standard's value row (the Pair confirmation board): the label left, its value right
  detail: { flexDirection: "row", alignItems: "center", minHeight: 48, paddingHorizontal: 14, gap: 12 },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  dt: { fontFamily: ui(400), paddingVertical: 12, color: colors.text, fontSize: 15 },
  dd: { fontFamily: ui(400), flex: 1, paddingVertical: 12, color: colors.textMuted, fontSize: 14, textAlign: "right" },
  mono: { fontFamily: mono, fontSize: 12 },
  progress: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 10 },
  actions: { flexDirection: "row", gap: 10, paddingTop: 16, paddingHorizontal: 16 },
  cancel: { flex: 1 },
  pair: { flex: 2 },
}));
