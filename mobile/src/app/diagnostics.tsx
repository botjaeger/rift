import { Stack } from "expo-router";
import { useState } from "react";
import { ScrollView, StyleSheet, Text } from "react-native";
import { runCryptoBench, type BenchResult } from "@/bench";
import { randomBytes } from "@/runtime";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { InfoRow } from "@/ui/Rows";
import { Section } from "@/ui/Screen";
import { colors, mono } from "@/ui/theme";

/**
 * Hidden diagnostics (rift://diagnostics): how fast this device's JavaScript engine runs the
 * transport. A snapshot is at most 1 MiB; the gateway's frames are this size or smaller.
 */
export default function DiagnosticsScreen() {
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
    <ScrollView style={styles.fill} contentContainerStyle={styles.content}>
      <Stack.Screen options={{ title: "Diagnostics" }} />
      <Text style={styles.lead}>
        Runs one Noise handshake and seals then opens 1 MiB in 65 000-byte chunks, as the gateway sends a full snapshot.
      </Text>
      <Button label={busy ? "Running…" : "Run crypto benchmark"} variant="primary" disabled={busy} onPress={run} />
      {error ? <ErrorText>{error}</ErrorText> : null}
      {runs.map((r, n) => (
        <Section key={n} title={`Run ${n + 1} · ${r.engine}`}>
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
        </Section>
      ))}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 16, gap: 14 },
  lead: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  value: { color: colors.textBright, fontFamily: mono, fontSize: 14 },
});
