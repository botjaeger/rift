import * as Clipboard from "expo-clipboard";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, ScrollView, Share, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { gapLines, heldLines } from "@/earlier";
import { canOperate, draftKey } from "@/fleet";
import { commandProblem } from "@/protocol/types";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, runningCommand } from "@/topology";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { confirmDestructive } from "@/ui/confirm";
import { colors, mono, radius } from "@/ui/theme";

/** A block's actions (the "⋯" on each block): copy, share, edit, run again, stop, notify. */
export default function BlockActionsScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const {
    key,
    pane: paneParam,
    block: blockParam,
  } = useLocalSearchParams<{ key: string; pane: string; block: string }>();
  const paneId = Number(paneParam);
  const blockId = Number(blockParam);
  const s = useApp();
  const lock = useLock();
  const [error, setError] = useState<string | null>(null);

  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);

  const host = s.hosts.find((h) => h.record.key === key);
  const pane = s.pane?.host === key && s.pane.paneId === paneId ? s.pane : null;
  const block = pane?.state?.blocks.find((b) => b.id === blockId) ?? null;
  const loc = host ? findPane(host.windows, paneId) : null;
  const operate = host ? canOperate(host) : false;
  const connected = host?.conn.status === "open";
  const atPrompt = runningCommand(loc?.pane ?? null, pane?.state ?? null) === null;
  const running = block?.state === "running";
  const command = block?.command ?? "";
  const runnable = command !== "" && commandProblem(command) === null;
  const earlier = pane?.earlier[blockId];
  // the output the app holds: earlier lines loaded, then the tail (with what is missing between)
  const gap = block ? gapLines(block, earlier) : 0;
  const output =
    (heldLines(earlier) > 0 ? earlier!.text + (gap > 0 ? `[${gap} lines not loaded]\n` : "") : "") +
    (block?.output ?? "");

  if (lock.covered || lock.locked) return <View style={styles.fill} />;

  const done = (message: string) => {
    AccessibilityInfo.announceForAccessibility(message);
    router.back();
  };

  const copy = async (text: string, what: string) => {
    try {
      await Clipboard.setStringAsync(text);
      done(`${what} copied`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const share = async () => {
    try {
      await Share.share({ message: output });
      router.back();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const edit = async () => {
    // the command line holds other text: replacing it loses that, so ask
    const draft = s.drafts[draftKey(key, paneId)] ?? "";
    if (draft !== "" && draft !== command) {
      const ok = await confirmDestructive(
        "Replace the text in the command line?",
        `It holds: ${draft}`,
        "Replace",
      );
      if (!ok) return;
    }
    fleet.setDraft(key, paneId, command);
    done("Command put in the command line");
  };

  const runAgain = async () => {
    if (s.settings.confirmEveryCommand) {
      // the same path as Run: the Confirm sheet, with this block's command instead of the draft
      router.replace({ pathname: "/confirm", params: { key, pane: String(paneId), block: String(blockId) } });
      return;
    }
    const r = await fleet.run(key, paneId, command, { fromDraft: false });
    if (r.ok) done("Command sent");
    else setError(r.message);
  };

  const stop = async () => {
    const r = await fleet.sendKeys(key, paneId, ["ctrl+c"]);
    if (r.ok) done("Control C sent");
    else setError(r.message);
  };

  const notify = () => {
    fleet.notifyWhenDone(key, paneId, command);
    done("You will be notified when it finishes");
  };

  return (
    <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 16 }]}>
      <View
        style={styles.command}
        accessible
        accessibilityRole="header"
        accessibilityLabel={`Command: ${command || "none"}`}
      >
        <Text style={styles.commandText} numberOfLines={3}>
          {block ? command || "(no command)" : "This block is gone."}
        </Text>
      </View>
      {block ? (
        <View style={styles.actions}>
          <Button label="Copy command" disabled={command === ""} onPress={() => void copy(command, "Command")} />
          <Button label="Copy output" disabled={output === ""} onPress={() => void copy(output, "Output")} />
          <Button label="Share output" disabled={output === ""} onPress={() => void share()} />
          {operate ? (
            <>
              <Button label="Edit command" disabled={command === ""} onPress={() => void edit()} />
              <Button
                label="Run again"
                disabled={!runnable || !connected || !atPrompt}
                accessibilityHint={
                  !atPrompt ? "Unavailable while the pane is running a program" : !connected ? "Not connected" : undefined
                }
                onPress={() => void runAgain()}
              />
              {running ? (
                <Button label="Stop (ctrl+c)" disabled={!connected} onPress={() => void stop()} />
              ) : null}
            </>
          ) : null}
          {running ? <Button label="Notify me when it finishes" onPress={notify} /> : null}
        </View>
      ) : null}
      {error ? <ErrorText>{error}</ErrorText> : null}
      <Button label="Cancel" onPress={() => router.back()} />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  content: { padding: 20, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
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
  actions: { gap: 8 },
});
