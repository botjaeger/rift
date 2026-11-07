import { useRouter } from "expo-router";
import { useState } from "react";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { runCryptoBench, type BenchResult } from "@/bench";
import { randomBytes } from "@/runtime";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { CardBox, Group, InfoRow, Row } from "@/ui/Rows";
import { PageHeader } from "@/ui/Screen";
import { mono, ui, makeStyles } from "@/ui/theme";

/**
 * Hidden diagnostics (rift://diagnostics): how fast this device's JavaScript engine runs the
 * transport. A snapshot is at most 1 MiB; the gateway's frames are this size or smaller. The
 * app's own page header: the system bar it had named Back after the page under it.
 */
export default function DiagnosticsScreen() {
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [runs, setRuns] = useState<BenchResult[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = () => {
    setBusy(true);
    setError(null);
    // Let the button show its busy state before the synchronous work starts.
    setTimeout(() => {
      try {
        setRuns((r) => [...r, runCryptoBench(randomBytes, () => performance.now())]);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    }, 50);
  };

  return (
    <View style={styles.fill}>
      <PageHeader title="Diagnostics" back={() => (router.canGoBack() ? router.back() : router.replace("/"))} />
      <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}>
        {/* the Diagnostics board: what it measures as a row under Encryption, then Run */}
        <Group title="Encryption">
          <CardBox>
            <Row first>
              <View style={styles.text}>
                <Text style={styles.label}>Crypto benchmark</Text>
                <Text style={styles.lead}>One handshake, then 1 MiB in 65 000-byte chunks</Text>
              </View>
            </Row>
          </CardBox>
        </Group>
        <Button label={busy ? "Running…" : "Run"} accessibilityLabel="Run the crypto benchmark" disabled={busy} onPress={run} />
        {error ? <ErrorText>{error}</ErrorText> : null}
        {runs.map((r, n) => (
          <Group key={n} title={`Run ${n + 1} · ${r.engine}`}>
            <CardBox>
              <InfoRow label="Handshake" first>
                <Text style={styles.value} accessibilityLabel={`handshake ${r.handshakeMs.toFixed(1)} ms`}>{r.handshakeMs.toFixed(1)} ms</Text>
              </InfoRow>
              <InfoRow label="Seal 1 MiB">
                <Text style={styles.value} accessibilityLabel={`seal ${r.sealMs.toFixed(0)} ms`}>{r.sealMs.toFixed(0)} ms</Text>
              </InfoRow>
              <InfoRow label="Open 1 MiB">
                <Text style={styles.value} accessibilityLabel={`open ${r.openMs.toFixed(0)} ms`}>{r.openMs.toFixed(0)} ms</Text>
              </InfoRow>
              <InfoRow label="Throughput">
                <Text style={styles.value} accessibilityLabel={`throughput ${r.mibPerSecond.toFixed(1)} MiB per second`}>
                  {r.mibPerSecond.toFixed(1)} MiB/s
                </Text>
              </InfoRow>
            </CardBox>
          </Group>
        ))}
      </ScrollView>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  // the Diagnostics board: the caption 18 under the header, Run 14 under the card
  content: { paddingTop: 18, paddingHorizontal: 16, gap: 14, maxWidth: 720, width: "100%", alignSelf: "center" },
  text: { flex: 1, gap: 2 },
  label: { fontFamily: ui(400), color: colors.text, fontSize: 16, lineHeight: 21 },
  lead: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  // measured numbers: tabular, as output
  value: { color: colors.textBright, fontFamily: mono, fontSize: 14 },
}));
