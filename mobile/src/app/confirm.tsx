import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { canOperate, draftKey } from "@/fleet";
import { needsAuthBeforeRun } from "@/lock";
import { commandProblem } from "@/protocol/types";
import { authenticate, fleet, useApp, useLock } from "@/runtime";
import { findPane, paneBranch, paneTitle, runningCommand } from "@/topology";
import { Button } from "@/ui/Button";
import { ErrorText, WarningText } from "@/ui/Notice";
import { InfoRow, SwitchRow } from "@/ui/Rows";
import { Section } from "@/ui/Screen";
import { colors, mono, radius } from "@/ui/theme";

/** "Run on <host>?": the explicit target and command, shown before anything is sent. */
export default function ConfirmScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const {
    key,
    pane: paneParam,
    block: blockParam,
  } = useLocalSearchParams<{ key: string; pane: string; block?: string }>();
  const paneId = Number(paneParam);
  const s = useApp();
  const lock = useLock();
  const [notify, setNotify] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The sheet is presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);

  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const state = s.pane?.host === key && s.pane.paneId === paneId ? s.pane.state : null;
  // Run again names a block of the followed pane (never a command: a link could carry one);
  // otherwise the pane's draft runs
  const again =
    blockParam === undefined ? undefined : (state?.blocks.find((b) => b.id === Number(blockParam))?.command ?? "");
  const command = again ?? s.drafts[draftKey(key, paneId)] ?? "";
  const running = runningCommand(loc?.pane ?? null, state);
  const branch = paneBranch(state);
  const connected = host?.conn.status === "open";
  const problem = commandProblem(command);
  // The permission can drop while the sheet is open, and a deep link can open it for a view-only host.
  const operate = host ? canOperate(host) : false;
  const canRun = !!host && operate && !!loc && connected && running === null && problem === null && !sending;
  const askFirst = needsAuthBeforeRun(s.settings);
  const hostName = host?.record.name ?? "this computer";

  const run = async () => {
    if (!canRun) return;
    setError(null);
    if (askFirst) {
      const auth = await authenticate(`Run on ${hostName}`);
      if (!auth.ok) {
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    setSending(true);
    const r = await fleet.run(key, paneId, command, { notify, fromDraft: again === undefined });
    setSending(false);
    if (r.ok) {
      AccessibilityInfo.announceForAccessibility(`Command sent to ${hostName}`);
      router.back();
    } else {
      setError(r.message);
    }
  };

  // Hidden while the app is in the app switcher or locked (the sheet sits above the lock cover).
  if (lock.covered || lock.locked) return <View style={styles.fill} />;

  return (
    <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 16 }]}>
      <Text style={styles.title} accessibilityRole="header">
        Run on {hostName}?
      </Text>
      <Section>
        <InfoRow label="Host" first>
          <Text style={styles.value}>{hostName}</Text>
        </InfoRow>
        <InfoRow label="Pane">
          <Text style={styles.value}>{loc ? paneTitle(loc) : "Pane closed"}</Text>
        </InfoRow>
        <InfoRow label="Directory">
          <Text style={[styles.value, styles.mono]}>{loc?.pane.cwd ?? "unknown"}</Text>
          {branch ? <Text style={[styles.value, styles.branch]}>{branch}</Text> : null}
        </InfoRow>
      </Section>

      <View style={styles.command} accessible accessibilityLabel={`Command: ${command}`}>
        <Text style={styles.commandText} selectable>
          {command}
        </Text>
      </View>

      {running ? (
        <WarningText>
          This pane is running {running}. Commands only run at a prompt; use the keys row to talk to the program.
        </WarningText>
      ) : null}
      {host && !operate ? <WarningText>This device may only view {hostName}.</WarningText> : null}
      {!connected && host ? <WarningText>Not connected to {hostName}.</WarningText> : null}
      {problem === "too_long" ? <WarningText>The command is too long (2 KiB at most).</WarningText> : null}
      {problem === "control" ? <WarningText>The command must be one line.</WarningText> : null}

      <Section>
        <SwitchRow first label="Notify me when it finishes" value={notify} onChange={setNotify} />
      </Section>

      {error ? <ErrorText>{error}</ErrorText> : null}
      <View style={styles.buttons}>
        <Button label="Cancel" onPress={() => router.back()} style={styles.button} />
        <Button
          label={sending ? "Running…" : "Run"}
          variant="primary"
          disabled={!canRun}
          accessibilityHint={
            host && !operate
              ? "Unavailable: this device may only view this computer"
              : running
                ? "Unavailable while the pane is running a program"
                : askFirst
                  ? "Asks for Face ID, a fingerprint or the passcode first"
                  : undefined
          }
          onPress={() => void run()}
          style={styles.button}
        />
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  content: { padding: 20, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  title: { color: colors.textBright, fontSize: 20, fontWeight: "600" },
  value: { color: colors.text, fontSize: 14, textAlign: "right" },
  mono: { fontFamily: mono, fontSize: 13 },
  branch: { color: colors.success, fontSize: 12 },
  command: {
    backgroundColor: colors.bg,
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
    borderLeftWidth: 2,
    borderLeftColor: colors.accent,
    padding: 12,
  },
  commandText: { color: colors.textBright, fontFamily: mono, fontSize: 14, lineHeight: 20 },
  // Side by side; stacked when large text makes them too wide.
  buttons: { flexDirection: "row", flexWrap: "wrap", gap: 10 },
  button: { flexGrow: 1, flexBasis: 140 },
});
