import * as Clipboard from "expo-clipboard";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Pressable, Share, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { tildePath } from "@/chips";
import { gapLines, heldLines } from "@/earlier";
import { canOperate, draftKey } from "@/fleet";
import { commandProblem } from "@/protocol/types";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, runningCommand } from "@/topology";
import { confirmDestructive } from "@/ui/confirm";
import { formatDuration } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { colors, mono, monoMedium, radius, ui } from "@/ui/theme";

/** One row of the actions card: a mono glyph, then the label. */
function Action({
  icon,
  label,
  onPress,
  first,
  accent,
  disabled,
  hint,
}: {
  icon: string;
  label: string;
  onPress: () => void;
  first?: boolean;
  accent?: boolean;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.action, !first && styles.divider, pressed && styles.pressed, disabled && styles.disabled]}
    >
      <Text style={styles.icon} importantForAccessibility="no" accessibilityElementsHidden>
        {icon}
      </Text>
      <Text style={[styles.label, accent && { color: colors.accent }]}>{label}</Text>
    </Pressable>
  );
}

/**
 * A block's actions (the BlockSheet board; "···" on a block, or a long press): Run again,
 * Edit, then run, Copy command, Copy output, Share output, and Stop while it runs.
 */
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

  // Hidden while the app is in the app switcher, locked, or under a Face ID prompt (the sheet
  // sits above the lock cover). Kept laid out: a sheet sized to its content must not collapse.
  const hidden = lock.covered || lock.locked;

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
      const ok = await confirmDestructive("Replace the text in the command line?", `It holds: ${draft}`, "Replace");
      if (!ok) return;
    }
    fleet.setDraft(key, paneId, command);
    done("Command put in the command line");
  };

  const runAgain = async () => {
    if (s.settings.askBeforeRun) {
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

  // "Failed · exit 101 · 8.4s · ~/zzz/rift · main"
  const meta = block
    ? [
        running
          ? "Running"
          : block.exit === null
            ? "Finished"
            : block.exit === 0
              ? "Succeeded"
              : `Failed · exit ${block.exit}`,
        running ? null : formatDuration(block.duration_ms),
        block.cwd ? tildePath(block.cwd) : null,
        block.branch,
        block.restored ? "restored" : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";

  const actions: { icon: string; label: string; onPress: () => void; disabled?: boolean; hint?: string; accent?: boolean }[] = [];
  if (block && operate) {
    actions.push({
      icon: "↻",
      label: "Run again",
      accent: true,
      disabled: !runnable || !connected || !atPrompt,
      hint: !atPrompt ? "Unavailable while the pane is running a program" : !connected ? "Not connected" : undefined,
      onPress: () => void runAgain(),
    });
    actions.push({ icon: "✎", label: "Edit, then run", disabled: command === "", hint: "Puts it in the command line", onPress: () => void edit() });
  }
  if (block) {
    actions.push({ icon: "⧉", label: "Copy command", disabled: command === "", onPress: () => void copy(command, "Command") });
    actions.push({ icon: "≡", label: "Copy output", disabled: output === "", onPress: () => void copy(output, "Output") });
    actions.push({ icon: "↗", label: "Share output", disabled: output === "", onPress: () => void share() });
  }
  if (block && operate && running) actions.push({ icon: "^C", label: "Stop", hint: "Sends Control C", disabled: !connected, onPress: () => void stop() });

  return (
    <View
      style={[styles.fill, styles.content, { paddingBottom: Math.max(36, insets.bottom + 2) }, hidden && styles.hidden]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <View style={styles.head} accessible accessibilityRole="header" accessibilityLabel={`Command: ${command || "none"}. ${meta}`}>
        <Text style={styles.command} numberOfLines={3}>
          {block ? command || "(no command)" : "This block is gone."}
        </Text>
        {meta ? <Text style={styles.meta}>{meta}</Text> : null}
      </View>
      {actions.length > 0 ? (
        <View style={styles.card}>
          {actions.map((a, i) => (
            <Action key={a.label} first={i === 0} {...a} />
          ))}
        </View>
      ) : null}
      {error ? <ErrorText>{error}</ErrorText> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { backgroundColor: colors.surface },
  hidden: { opacity: 0 },
  content: { paddingTop: 24, paddingHorizontal: 12, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  head: { paddingHorizontal: 6, gap: 4 },
  command: { color: colors.textBright, fontFamily: monoMedium, fontSize: 15 },
  meta: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  card: {
    backgroundColor: colors.bg,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  action: { minHeight: 52, flexDirection: "row", alignItems: "center", gap: 14, paddingHorizontal: 14 },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  icon: { width: 20, textAlign: "center", fontFamily: mono, color: colors.textMuted },
  label: { fontFamily: ui(400), color: colors.text, fontSize: 15 },
});
