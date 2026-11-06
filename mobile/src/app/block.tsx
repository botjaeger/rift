import { useActionScope } from "@/ui/useActionScope";
import * as Clipboard from "expo-clipboard";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { tildePath } from "@/chips";
import { gapLines, heldLines, MAX_EARLIER_LINES, nextRequest, unloadedLines } from "@/earlier";
import { canOperate, draftKey, hasFeature, wrapsLines } from "@/fleet";
import { agoText, outputLines, sheetChip, type ChipTone } from "@/paneView";
import { commandProblem } from "@/protocol/types";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, runningCommand } from "@/topology";
import { Output } from "@/ui/BlockView";
import { Button } from "@/ui/Button";
import { Icon } from "@/ui/Icon";
import { SelectableText } from "@/ui/SelectableText";
import { SheetHeader } from "@/ui/SheetHeader";
import { confirmDestructive } from "@/ui/confirm";
import { formatDuration } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { useNow } from "@/ui/useNow";
import { useSingleFlight } from "@/ui/useSingleFlight";
import { fromPalette, mono, radius, ui, makeStyles, useColors } from "@/ui/theme";

/** The status line's colour under the command (the board: running accent, failed red, succeeded green). */
const statusColor = fromPalette((colors): Record<ChipTone, string> => ({
  running: colors.accent,
  failed: colors.error,
  ok: colors.success,
  done: colors.textMuted,
}));

const lines = (n: number) => `${n.toLocaleString("en-US")} ${n === 1 ? "line" : "lines"}`;

/**
 * A block (the BlockSheet board; a tap on a block's card): its status and when, where it ran,
 * the command to copy, its whole output to read and copy, and Run again (Stop while it runs) or
 * Edit, which puts the command in the pane's command line.
 */
export default function BlockActionsScreen() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const {
    key,
    pane: paneParam,
    block: blockParam,
  } = useLocalSearchParams<{ key: string; pane: string; block: string }>();
  const paneId = Number(paneParam);
  const blockId = Number(blockParam);
  const scope = useActionScope(`${key}:${paneId}:${blockId}`);
  const s = useApp();
  const lock = useLock();
  const [error, setError] = useState<string | null>(null);

  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked && scope.leave()) router.back();
  }, [lock.locked, router, scope]);

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
  const wrap = wrapsLines(s, key, paneId);
  // the output the app holds: earlier lines loaded, then the tail (with what is missing between)
  const gap = block ? gapLines(block, earlier) : 0;
  const output =
    (heldLines(earlier) > 0 ? earlier!.text + (gap > 0 ? `[${gap} lines not loaded]\n` : "") : "") +
    (block?.output ?? "");
  const unloaded = block ? unloadedLines(block, earlier) : 0;
  const canLoad = hasFeature(host, "output");
  const loading = earlier?.loading ?? false;

  // Hidden while the app is in the app switcher, locked, or under a Face ID prompt (the sheet
  // sits above the lock cover). Kept laid out: a sheet sized to its content must not collapse.
  const hidden = lock.covered || lock.locked;

  const done = (message: string) => {
    AccessibilityInfo.announceForAccessibility(message);
    if (scope.leave()) router.back();
  };

  const copy = async (text: string, what: string) => {
    const current = scope.capture();
    try {
      await Clipboard.setStringAsync(text);
      if (current()) done(`${what} copied`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const edit = async () => {
    const current = scope.capture();
    // the command line holds other text: replacing it loses that, so ask
    const draft = s.drafts[draftKey(key, paneId)] ?? "";
    if (draft !== "" && draft !== command) {
      const ok = await confirmDestructive("Replace the text in the command line?", `It holds: ${draft}`, "Replace");
      if (!ok || !current()) return;
    }
    fleet.setDraft(key, paneId, command);
    done("Command put in the command line");
  };

  // a command on its way: a second tap before the sheet closes must not run it twice
  const once = useSingleFlight();
  const runAgain = () =>
    once(async () => {
      const current = scope.capture();
      if (s.settings.askBeforeRun) {
        // the same path as Run: the Confirm sheet, with this block's command instead of the draft
        router.push({ pathname: "/confirm", params: { key, pane: String(paneId), block: String(blockId), parent: "1" } });
        return;
      }
      const r = await fleet.run(key, paneId, command, { fromDraft: false });
      if (!current()) return;
      if (r.ok) done("Command sent");
      else setError(r.message);
    });

  const stop = async () => {
    const current = scope.capture();
    const r = await fleet.sendKeys(key, paneId, ["ctrl+c"]);
    if (!current()) return;
    if (r.ok) done("Control C sent");
    else setError(r.message);
  };

  const now = useNow(30_000);
  const chip = block ? sheetChip(block) : null;
  const failed = chip?.tone === "failed";
  // "1.2s · 6 min ago", "4 min ago", "started 2 min ago": when it ran, from what the app saw
  const at = block ? s.activity.find((a) => a.host === key && a.pane === paneId && a.block === blockId)?.at : undefined;
  const when = block
    ? running
      ? `started ${agoText(block.duration_ms)}`
      : [
          failed && at !== undefined ? null : formatDuration(block.duration_ms),
          at !== undefined ? agoText(now - at) : null,
          block.restored ? "restored" : null,
        ]
          .filter(Boolean)
          .join(" · ")
    : "";
  const place = block ? [block.cwd ? tildePath(block.cwd) : null, block.branch] : [];
  const disabled = !runnable || !connected || !atPrompt;
  const outputGround = colors.chrome;

  return (
    // The sheet's own ScrollView, as its root: iOS lays out a form sheet's first scroll view as the
    // sheet's content, so the output's box alone filled the sheet and covered the rest.
    <ScrollView
      style={[styles.sheet, hidden && styles.hidden]}
      contentContainerStyle={[styles.fill, { paddingBottom: Math.max(30, insets.bottom + 2) }]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      {/* the command as the title, its status in words under it */}
      <SheetHeader
        title={block ? command || "(no command)" : "Command"}
        titleMono={!!block && command !== ""}
        subtitle={chip ? [chip.text.replace(/^[✕✓●] ?/, ""), when].filter(Boolean).join(" · ") : "This block is gone."}
        subtitleColor={chip ? statusColor(colors)[chip.tone] : undefined}
        onClose={() => {
          if (scope.leave()) router.back();
        }}
      />
      {block ? (
        <View style={styles.body}>
          <View style={styles.section}>
            {place[0] || place[1] ? (
              <View style={styles.place} accessible accessibilityLabel={`Ran in ${place[0] ?? "an unknown folder"}${place[1] ? ` on ${place[1]}` : ""}`}>
                {place[0] ? (
                  <>
                    <Icon name="folder" size={13} color={colors.textMuted} />
                    <Text style={styles.path} numberOfLines={1} ellipsizeMode="head">
                      {place[0]}
                    </Text>
                  </>
                ) : null}
                {place[1] ? <Text style={styles.branch}>⎇ {place[1]}</Text> : null}
              </View>
            ) : null}
            <View style={styles.code}>
              <Text style={styles.prompt}>$</Text>
              <View style={styles.commandBox}>
                <SelectableText text={command || "(no command)"} style={styles.command} lineHeight={21} />
              </View>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Copy command"
                accessibilityState={{ disabled: command === "" }}
                disabled={command === ""}
                onPress={() => void copy(command, "Command")}
                hitSlop={6}
                style={({ pressed }) => [styles.copyIcon, pressed && styles.pressed]}
              >
                <Icon name="copy" size={17} color={colors.textMuted} />
              </Pressable>
            </View>
          </View>
          <View style={styles.section}>
            <View style={styles.outHead}>
              <Text style={styles.outTitle}>Output · {outputLines(output)}</Text>
              <Button
                label="Copy"
                accessibilityLabel="Copy output"
                size="small"
                disabled={output === ""}
                icon={<Icon name="copy" size={13} color={colors.text} />}
                onPress={() => void copy(output, "Output")}
              />
            </View>
            {canLoad && unloaded > 0 && nextRequest(block, earlier) ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={loading ? "Loading earlier output" : `Load earlier output, ${lines(unloaded)}`}
                accessibilityHint="Loads up to 500 lines at a time"
                accessibilityState={{ disabled: loading, busy: loading }}
                disabled={loading}
                onPress={() => void fleet.loadEarlier(blockId)}
                style={styles.note}
              >
                <Text style={styles.noteLink}>{loading ? "Loading earlier output…" : `Load earlier output (${lines(unloaded)})`}</Text>
              </Pressable>
            ) : !canLoad && block.dropped > 0 ? (
              <Text style={styles.noteText}>{lines(block.dropped)} earlier not shown</Text>
            ) : null}
            {earlier?.gone ? (
              <Text style={styles.noteText}>
                {earlier.from > 0 ? `${lines(earlier.from)} before these: ` : ""}These lines are gone from the computer's scrollback.
              </Text>
            ) : null}
            {heldLines(earlier) >= MAX_EARLIER_LINES && unloaded > 0 ? (
              <Text style={styles.noteText}>
                Showing {lines(heldLines(earlier))} of earlier output, the most kept per block; the rest stays on the computer.
              </Text>
            ) : null}
            {earlier?.error ? <ErrorText>{earlier.error}</ErrorText> : null}
            <ScrollView
              style={[styles.outBox, { borderColor: failed ? `${colors.error}55` : colors.hairline }]}
              contentContainerStyle={styles.outPad}
              nestedScrollEnabled
              accessibilityLabel="Output"
            >
              {output === "" ? (
                <Text style={styles.empty}>(no output)</Text>
              ) : (
                <Output text={output.endsWith("\n") ? output.slice(0, -1) : output} wrap={wrap} ground={outputGround} size={13} lineHeight={19} />
              )}
            </ScrollView>
          </View>
          {error ? <ErrorText>{error}</ErrorText> : null}
          {operate ? (
            <View style={styles.buttons}>
              {running ? (
                <Button
                  label="Stop"
                  variant="destructive"
                  accessibilityHint="Sends Control C"
                  disabled={!connected}
                  icon={<Icon name="stop" size={14} color={colors.error} />}
                  onPress={() => void stop()}
                  style={styles.flex}
                />
              ) : (
                <Button
                  label="Run again"
                  variant="primary"
                  accessibilityHint={!atPrompt ? "Unavailable while the pane is running a program" : !connected ? "Not connected" : undefined}
                  disabled={disabled}
                  icon={<Icon name="rerun" size={17} color={colors.onAccent} />}
                  onPress={() => void runAgain()}
                  style={styles.flex}
                />
              )}
              <Button
                label="Edit"
                accessibilityLabel="Edit, then run"
                accessibilityHint="Puts it in the command line"
                disabled={command === ""}
                icon={<Icon name="edit" size={17} color={colors.text} />}
                onPress={() => void edit()}
              />
            </View>
          ) : null}
        </View>
      ) : null}
    </ScrollView>
  );
}

const useStyles = makeStyles((colors) => ({
  sheet: { backgroundColor: colors.surface },
  fill: { backgroundColor: colors.surface },
  body: { paddingHorizontal: 16, gap: 14, maxWidth: 560, width: "100%", alignSelf: "center" },
  hidden: { opacity: 0 },
  pressed: { opacity: 0.7 },
  flex: { flex: 1 },
  section: { gap: 6 },
  place: { flexDirection: "row", alignItems: "center", gap: 6 },
  path: { flexShrink: 1, fontFamily: mono, color: colors.textMuted, fontSize: 12 },
  branch: { fontFamily: mono, color: colors.success, fontSize: 12 },
  code: { flexDirection: "row", alignItems: "flex-start", gap: 8, borderRadius: radius.field, backgroundColor: colors.chrome, paddingTop: 10, paddingBottom: 10, paddingLeft: 12, paddingRight: 6 },
  prompt: { fontFamily: mono, color: colors.accent, fontSize: 14, lineHeight: 21 },
  commandBox: { flex: 1, minWidth: 0 },
  command: { fontFamily: mono, color: colors.textBright, fontSize: 14 },
  copyIcon: { width: 32, height: 32, borderRadius: 8, alignItems: "center", justifyContent: "center", marginVertical: -4 },
  outHead: { flexDirection: "row", alignItems: "center", gap: 8 },
  outTitle: { flex: 1, fontFamily: ui(500), color: colors.textMuted, fontSize: 12 },
  note: { minHeight: 32, justifyContent: "center", alignSelf: "flex-start" },
  noteLink: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  noteText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  outBox: { maxHeight: 280, flexGrow: 0, borderRadius: radius.field, borderWidth: 1, backgroundColor: colors.chrome },
  outPad: { padding: 12 },
  // "(no output)" is words, not output
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 19 },
  buttons: { flexDirection: "row", gap: 8 },
}));
