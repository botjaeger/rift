import { useActionScope } from "@/ui/useActionScope";
import * as Clipboard from "expo-clipboard";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Pressable, ScrollView, Text, View } from "react-native";
import { tildePath } from "@/chips";
import { gapLines, heldLines, MAX_EARLIER_LINES, nextRequest, unloadedLines } from "@/earlier";
import { canOperate, draftKey, hasFeature, wrapsLines } from "@/fleet";
import { agoText, liveBlock, outputLines, sheetChip } from "@/paneView";
import { commandProblem } from "@/protocol/types";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, runningCommand } from "@/topology";
import { Output } from "@/ui/BlockView";
import { Button } from "@/ui/Button";
import { Icon } from "@/ui/Icon";
import { SelectableText } from "@/ui/SelectableText";
import { SheetButtons, SheetFrame } from "@/ui/SheetFrame";
import { confirmDestructive } from "@/ui/confirm";
import { formatDuration } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { useNow } from "@/ui/useNow";
import { useSingleFlight } from "@/ui/useSingleFlight";
import { mono, radius, ui, makeStyles, useColors } from "@/ui/theme";

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
  // a running block's clock ticks here between the gateway's updates, like its card's
  const now = useNow(1000, running);
  const [seen, setSeen] = useState({ ms: block?.duration_ms ?? -1, at: now });
  // a fresh duration from the gateway restarts the local count
  if (block && seen.ms !== block.duration_ms) setSeen({ ms: block.duration_ms, at: now });
  const live = block ? liveBlock(block, seen.at, now, loc?.pane.running?.started ?? null) : null;
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

  const chip = live ? sheetChip(live) : null;
  const place = block ? [block.cwd ? tildePath(block.cwd) : null, block.branch] : [];
  // the board's subtitle: "Failed with 1 · 2.4 s · ~/code/project1" ("started 2 min ago" while it runs)
  const subtitle = chip
    ? [
        chip.text.replace(/^[✕✓●] ?/, ""),
        live!.state === "running" ? `started ${agoText(live!.duration_ms)}` : spacedDuration(block!.duration_ms),
        place[0],
        block!.restored ? "restored" : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "This block is gone.";
  const disabled = !runnable || !connected || !atPrompt;

  const footer =
    block && operate ? (
      <SheetButtons>
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
          style={styles.flex}
        />
      </SheetButtons>
    ) : null;

  return (
    // the command as the title, its status, how long and where under it; Run again / Edit pinned to the foot
    <SheetFrame
      board="block"
      title={block ? command || "(no command)" : "Command"}
      subtitle={subtitle}
      onClose={() => {
        if (scope.leave()) router.back();
      }}
      footer={footer}
      bodyStyle={styles.body}
    >
      {block ? (
        <>
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
            <View style={styles.commandBox}>
              <SelectableText text={`$ ${command || "(no command)"}`} style={styles.command} lineHeight={18} />
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
          <ScrollView style={styles.outBox} contentContainerStyle={styles.outPad} nestedScrollEnabled accessibilityLabel="Output">
            {output === "" ? (
              <Text style={styles.empty}>(no output)</Text>
            ) : (
              <Output text={output.endsWith("\n") ? output.slice(0, -1) : output} wrap={wrap} ground={colors.bg} size={12} lineHeight={18} />
            )}
          </ScrollView>
          {error ? <ErrorText>{error}</ErrorText> : null}
        </>
      ) : null}
    </SheetFrame>
  );
}

/** "2.4 s", "120 ms", "3m 4s": the board's duration, a space before a lone unit. */
const spacedDuration = (ms: number) => formatDuration(ms).replace(/^([\d.]+)(m?s)$/, (_, n: string, unit: string) => `${n} ${unit}`);

// The Block board: a 10 pt gap down the body, the command and the output in #050505 boxes with a
// neutral hairline, mono 13/18 and 12/18, the output box 150 pt of lines (172 with its padding).
const useStyles = makeStyles((colors) => ({
  body: { gap: 10 },
  pressed: { opacity: 0.7 },
  flex: { flex: 1 },
  place: { flexDirection: "row", alignItems: "center", gap: 8 },
  path: { flexShrink: 1, fontFamily: mono, color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  branch: { fontFamily: mono, color: colors.success, fontSize: 12, lineHeight: 16 },
  code: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
    borderRadius: radius.field,
    borderWidth: 1,
    borderColor: colors.hairline,
    backgroundColor: colors.bg,
    paddingVertical: 10,
    paddingLeft: 12,
    paddingRight: 8,
  },
  commandBox: { flex: 1, minWidth: 0 },
  command: { fontFamily: mono, color: colors.textBright, fontSize: 13 },
  copyIcon: { width: 24, height: 24, alignItems: "center", justifyContent: "center", marginVertical: -3 },
  outHead: { flexDirection: "row", alignItems: "center", gap: 8 },
  outTitle: { flex: 1, fontFamily: ui(500), color: colors.textMuted, fontSize: 12, lineHeight: 16, textTransform: "uppercase" },
  note: { minHeight: 32, justifyContent: "center", alignSelf: "flex-start" },
  noteLink: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  noteText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  outBox: { height: 172, flexGrow: 0, borderRadius: radius.field, borderWidth: 1, borderColor: colors.hairline, backgroundColor: colors.bg },
  outPad: { paddingVertical: 10, paddingHorizontal: 12 },
  // "(no output)" is words, not output
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
}));
