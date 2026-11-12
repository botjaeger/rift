import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { bytesText, PROBE_INTERVAL_MS, rates, rateText, summarize, verdict, type Rates, type SpeedSnapshot, type Summary } from "@/protocol/speed";
import { fleet, useApp } from "@/runtime";
import { CardBox, Group, InfoRow } from "@/ui/Rows";
import { PageHeader } from "@/ui/Screen";
import { mono, ui, makeStyles, useColors } from "@/ui/theme";

const ms = (v: number) => `${Math.round(v)} ms`;

/**
 * "Connection speed": what the connection to one computer measures, live (opened from the
 * Connection sheet). While it is open the app pings every second; the delays from typing and
 * scrolling to the next screen update are timed as you use the pane, and shown here.
 */
export default function SpeedScreen() {
  const { key } = useLocalSearchParams<{ key: string }>();
  const styles = useStyles();
  const colors = useColors();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  const close = () => (router.canGoBack() ? router.back() : router.replace("/"));

  useEffect(() => {
    if (!key) return;
    fleet.probe(key, true);
    return () => fleet.probe(key, false);
  }, [key]);
  // a reconnect makes a new client: probe that one too
  const status = host?.conn.status;
  useEffect(() => {
    if (key && status === "open") fleet.probe(key, true);
  }, [key, status]);

  // a snapshot a second, and the rates since the one before (none across a reconnect); half a
  // probe after each ping, so this page's own render never delays a pong (and its round trip)
  const [view, setView] = useState<{ snap: SpeedSnapshot | null; rate: Rates | null }>(() => ({
    snap: key ? fleet.speed(key) : null,
    rate: null,
  }));
  useEffect(() => {
    if (!key) return;
    const read = () => {
      const next = fleet.speed(key);
      setView((v) => ({
        snap: next,
        rate: next && v.snap?.since === next.since ? (rates(v.snap, next) ?? v.rate) : null,
      }));
    };
    let t: ReturnType<typeof setInterval> | undefined;
    const start = setTimeout(() => {
      read();
      t = setInterval(read, PROBE_INTERVAL_MS);
    }, PROBE_INTERVAL_MS / 2);
    return () => {
      clearTimeout(start);
      clearInterval(t);
    };
  }, [key]);
  const { snap, rate } = view;

  const rtt = snap ? summarize(snap.rtt) : null;
  const echo = snap ? summarize(snap.echo) : null;
  const scroll = snap ? summarize(snap.scroll) : null;
  const path = host?.conn.path;
  const via = !path ? "—" : path.kind === "relay" ? "Relay (outside this network)" : "This network";
  const judged = rtt ? verdict(rtt.median) : null;
  const toneColor = { ok: colors.success, warn: colors.warning, bad: colors.error };

  const series = (title: string, sum: Summary | null, empty: string, lead: string) => (
    <Group title={title} footer={lead}>
      <CardBox>
        {sum ? (
          <>
            <InfoRow label="Last" first>
              <Text style={styles.value}>{ms(sum.last)}</Text>
            </InfoRow>
            <InfoRow label="Median">
              <Text style={styles.value}>{ms(sum.median)}</Text>
            </InfoRow>
            <InfoRow label="Slowest 5 %">
              <Text style={styles.value}>{ms(sum.p95)}</Text>
            </InfoRow>
            <InfoRow label="Range">
              <Text style={styles.value}>
                {ms(sum.min)} – {ms(sum.max)}
              </Text>
            </InfoRow>
            <InfoRow label="Jitter">
              <Text style={styles.value}>{ms(sum.jitter)}</Text>
            </InfoRow>
            <InfoRow label="Samples">
              <Text style={styles.value}>{sum.count}</Text>
            </InfoRow>
          </>
        ) : (
          <InfoRow label={empty} first>
            <Text style={styles.value}>—</Text>
          </InfoRow>
        )}
      </CardBox>
    </Group>
  );

  return (
    <View style={styles.fill}>
      <PageHeader
        title="Connection speed"
        subtitle={host ? `${host.record.name} · ${status === "open" ? via : "not connected"}` : undefined}
        back={close}
      />
      <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}>
        {!snap ? (
          <Text style={styles.lead}>Not connected to this computer. The numbers start when it connects.</Text>
        ) : (
          <>
            <View style={styles.hero} accessible accessibilityLabel={rtt ? `Round trip ${ms(rtt.median)}, ${judged!.word}` : "Measuring"}>
              <Text style={styles.heroValue}>{rtt ? ms(rtt.median) : "…"}</Text>
              <Text style={[styles.heroWord, { color: judged ? toneColor[judged.tone] : colors.textMuted }]}>
                {judged ? `${judged.word} · median round trip` : "Measuring the round trip"}
              </Text>
            </View>
            {series("Round trip", rtt, "Waiting for the first ping", "A ping every second while this page is open.")}
            <Group title="Traffic" footer="Encrypted bytes on the socket, since this connection began.">
              <CardBox>
                <InfoRow label="Down" first>
                  <Text style={styles.value}>{rate ? rateText(rate.inPerSec) : "…"}</Text>
                </InfoRow>
                <InfoRow label="Up">
                  <Text style={styles.value}>{rate ? rateText(rate.outPerSec) : "…"}</Text>
                </InfoRow>
                <InfoRow label="Screen updates">
                  <Text style={styles.value}>{rate ? `${rate.framesPerSec.toFixed(1)} /s` : "…"}</Text>
                </InfoRow>
                <InfoRow label="Received">
                  <Text style={styles.value}>{bytesText(snap.bytesIn)}</Text>
                </InfoRow>
                <InfoRow label="Sent">
                  <Text style={styles.value}>{bytesText(snap.bytesOut)}</Text>
                </InfoRow>
              </CardBox>
            </Group>
            {series(
              "Typing to screen",
              echo,
              "Type or send in a pane",
              "From what you type, send or tap in a pane to its next screen update, timed after a second without input.",
            )}
            {series(
              "Scrolling to screen",
              scroll,
              "Scroll a full-screen program",
              "From a scroll to the next screen update, timed after a second without input.",
            )}
          </>
        )}
      </ScrollView>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { paddingTop: 18, paddingHorizontal: 16, gap: 14, maxWidth: 720, width: "100%", alignSelf: "center" },
  hero: { alignItems: "center", paddingVertical: 12, gap: 4 },
  heroValue: { color: colors.textBright, fontFamily: mono, fontSize: 34, lineHeight: 40 },
  heroWord: { fontFamily: ui(500), fontSize: 14, lineHeight: 18 },
  lead: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 19 },
  // measured numbers: tabular, as output
  value: { color: colors.textBright, fontFamily: mono, fontSize: 14 },
}));
