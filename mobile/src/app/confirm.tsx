import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Text, View } from "react-native";
import { tildePath } from "@/chips";
import { canOperate, draftKey } from "@/fleet";
import { needsAuthBeforeRun } from "@/lock";
import { chipCommand, isMenuCommand, type ChipAction } from "@/chips";
import { commandProblem, MAX_MENU_NAME } from "@/protocol/types";
import { authenticate, fleet, useApp, useLock } from "@/runtime";
import { findPane, paneTitle, runningCommand } from "@/topology";
import { Button } from "@/ui/Button";
import { ErrorText, WarningText } from "@/ui/Notice";
import { SheetButtons, SheetFrame } from "@/ui/SheetFrame";
import { mono, radius, ui, makeStyles, useColors } from "@/ui/theme";
import { useSingleFlight } from "@/ui/useSingleFlight";
import { useActionScope } from "@/ui/useActionScope";

/** A `run` pick's command is at most the gateway's 2 KiB. */
const MAX_COMMAND_CHARS = 2048;

/** A chip pick from the sheet's params: `cd` a folder, `checkout` or `create` a branch, `run` a version or container pick. */
function chipAction(kind: string | undefined, arg: string | undefined): ChipAction | null {
  // a version or container pick is the desktop's whole command, but only in the shapes its menus build
  if (kind === "run") return typeof arg === "string" && arg.length <= MAX_COMMAND_CHARS && isMenuCommand(arg) ? { kind, command: arg } : null;
  if (typeof arg !== "string" || arg === "" || arg.length > MAX_MENU_NAME || /[\n\r]/.test(arg)) return null;
  if (kind === "cd") return { kind: "cd", name: arg };
  if (kind === "checkout" || kind === "create") return { kind, branch: arg };
  return null;
}

/**
 * "Run on <host>?" (the Confirm board): the target and the command, shown before anything is
 * sent, only with Settings › Ask before running on.
 */
export default function ConfirmScreen() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const {
    key,
    pane: paneParam,
    block: blockParam,
    chip: chipParam,
    arg: argParam,
    parent,
  } = useLocalSearchParams<{ key: string; pane: string; block?: string; chip?: string; arg?: string; parent?: string }>();
  const paneId = Number(paneParam);
  const s = useApp();
  const lock = useLock();
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scope = useActionScope(`${key}:${paneId}:${blockParam}:${chipParam}:${argParam}:${parent}`);
  const close = () => {
    if (scope.leave()) router.back();
  };
  const finish = () => {
    if (!scope.leave()) return;
    if (parent === "1") router.dismissTo({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(paneId) } });
    else router.back();
  };

  // The sheet is presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    // Remove the parent sheet too: it must not remain above the lock cover.
    if (lock.locked) finish();
  });

  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const state = s.pane?.host === key && s.pane.paneId === paneId ? s.pane.state : null;
  // Run again names a block of the followed pane (never a command: a link could carry one);
  // otherwise the pane's draft runs
  const again =
    blockParam === undefined ? undefined : (state?.blocks.find((b) => b.id === Number(blockParam))?.command ?? "");
  // a prompt chip's pick: the command is built here from the folder or branch name (quoted),
  // never taken whole from the link
  const chip = chipAction(chipParam, argParam);
  const command = chip ? chipCommand(chip) : (again ?? s.drafts[draftKey(key, paneId)] ?? "");
  const running = runningCommand(loc?.pane ?? null, state);
  const connected = host?.conn.status === "open";
  const problem = commandProblem(command);
  // The permission can drop while the sheet is open, and a deep link can open it for a view-only host.
  const operate = host ? canOperate(host) : false;
  const canRun = !!host && operate && !!loc && connected && running === null && problem === null && !sending;
  const askFirst = needsAuthBeforeRun(s.settings);
  const hostName = host?.record.name ?? "this computer";
  // where it runs, in words: "In ~/code/project1 on studio-mac."
  const place = loc ? (loc.pane.cwd ? tildePath(loc.pane.cwd) : paneTitle(loc)) : null;
  const where = loc ? `In ${place} on ${hostName}.` : "This pane has closed.";

  const once = useSingleFlight();
  const run = () => once(async () => {
    const current = scope.capture();
    if (!current() || !canRun) return;
    setError(null);
    if (askFirst) {
      const auth = await authenticate(`Run on ${hostName}`);
      if (!current()) return;
      if (!auth.ok) {
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    setSending(true);
    const r = await fleet.run(key, paneId, command, chip ? { fromDraft: false, notify: false } : { fromDraft: again === undefined });
    setSending(false);
    if (!current()) return;
    if (r.ok) {
      AccessibilityInfo.announceForAccessibility(`Command sent to ${hostName}`);
      finish();
    } else {
      setError(r.message);
    }
  });

  return (
    // a title that asks; why it asks under it; where it runs under the command; Cancel / Run pinned to the foot
    <SheetFrame
      board="confirm"
      title="Run this?"
      subtitle="Ask before running is on"
      onClose={close}
      footer={
        <SheetButtons>
          <Button label="Cancel" onPress={close} style={styles.flex} />
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
            style={styles.flex}
          />
        </SheetButtons>
      }
    >
      <View style={styles.command} accessible accessibilityLabel={`Command: ${command}`}>
        <Text style={styles.commandText} selectable>
          <Text style={{ color: colors.success }}>❯ </Text>
          {command}
        </Text>
      </View>

      <Text style={styles.why}>{where}</Text>

      {running ? (
        <WarningText>
          This pane is running {running}. Commands only run at a prompt; the pane's command line sends text to the program.
        </WarningText>
      ) : null}
      {host && !operate ? <WarningText>This device may only view {hostName}.</WarningText> : null}
      {!connected && host ? <WarningText>Not connected to {hostName}.</WarningText> : null}
      {problem === "too_long" ? <WarningText>The command is too long (2 KiB at most).</WarningText> : null}
      {problem === "control" ? <WarningText>The command must be one line.</WarningText> : null}
      {error ? <ErrorText>{error}</ErrorText> : null}
    </SheetFrame>
  );
}

// The Run this? board: 16 pt in, 12 between, the command in a #050505 box 12 pt in.
const useStyles = makeStyles((colors) => ({
  flex: { flex: 1 },
  command: {
    backgroundColor: colors.bg,
    borderRadius: radius.field,
    borderWidth: 1,
    borderColor: colors.hairline,
    padding: 12,
  },
  commandText: { color: colors.textBright, fontFamily: mono, fontSize: 14, lineHeight: 20 },
  why: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18 },
}));
