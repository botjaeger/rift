import { useActionScope } from "@/ui/useActionScope";
import * as Clipboard from "expo-clipboard";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Text, View } from "react-native";
import { clipboardFiles, hasSystemPasteButton, SystemPasteButton } from "../../modules/rift-keys";
import { draftKey, hasFeature } from "@/fleet";
import { attachedItem, nativeAttached, pending, PENDING_FULL, readClipboard, routeRefusal, type AttachRules, type Attached } from "@/pending";
import { pickFile } from "@/pickFile";
import { pickLibraryPhoto, takePhoto } from "@/pickPhoto";
import { useApp, useLock } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { SheetFrame } from "@/ui/NewSheet";
import { ErrorText } from "@/ui/Notice";
import { Icon, type IconName } from "@/ui/Icon";
import { CardBox, NavRow } from "@/ui/Rows";
import { radius, TOUCH, ui, makeStyles, useColors } from "@/ui/theme";

/**
 * The Attach sheet (the Attach board), opened by the message box's +: Paste from clipboard,
 * Photo library, Take a photo, File. Text goes into the box, an image or file becomes a token in it
 * until Send (pending.ts); the sheet then closes back to the pane, which keeps its draft. Cancel
 * (Close, or the sheet's swipe) changes nothing. At a shell prompt an image or file is refused,
 * as a Paste is: there is no program to take it.
 */
export default function AttachSheet() {
  const styles = useStyles();
  const colors = useColors();
  const { key, pane: paneParam, program, running } = useLocalSearchParams<{ key: string; pane: string; program?: string; running?: string }>();
  const paneId = Number(paneParam);
  const router = useRouter();
  const s = useApp();
  const lock = useLock();
  const scope = useActionScope(`${key}:${paneId}`);
  const host = s.hosts.find((h) => h.record.key === key);
  const [error, setError] = useState<string | null>(null);
  // one at a time: a picker or a clipboard read is out
  const out = useRef(false);
  const [busy, setBusy] = useState(false);
  const box = draftKey(key, paneId);
  const inProgram = running === "1";
  // the tab it goes to ("To project1 on studio-mac"), else the program in it
  const loc = host ? findPane(host.windows, paneId) : null;
  const rules: AttachRules = { atPrompt: !inProgram, image: hasFeature(host, "upload"), file: hasFeature(host, "upload_files") };

  const close = () => {
    if (scope.leave()) router.back();
  };
  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) close();
  });

  // What a route gave: text for the box, an item to wait in it, or why not (the sheet stays).
  const deliver = (a: Attached) => {
    if (a.kind === "none") return;
    if (a.kind === "error") return setError(a.message);
    if (a.kind === "text") {
      if (inProgram && !hasFeature(host, "text")) return setError("Rift on this computer is too old to type into programs. Update it.");
      pending.giveText(box, a.text);
    } else if (!pending.add(box, a.item)) return setError(PENDING_FULL);
    // an item's token announces itself when the box places it
    if (a.kind === "text") AccessibilityInfo.announceForAccessibility("Pasted");
    close();
  };

  // runs `route` once at a time; a refusal that needs no picker comes first (`file`: the route's kind)
  const route = async (file: boolean, get: () => Promise<Attached>) => {
    if (out.current) return;
    setError(null);
    const why = routeRefusal(file, rules);
    if (why) return setError(why);
    out.current = true;
    setBusy(true);
    const current = scope.capture();
    try {
      const a = await get();
      if (current()) deliver(a);
    } finally {
      out.current = false;
      setBusy(false);
    }
  };
  const photo = (pick: typeof pickLibraryPhoto) => async (): Promise<Attached> => {
    const r = await pick();
    if (r === null) return { kind: "none" };
    return r.ok ? attachedItem({ ok: true, b64: r.b64 }, rules) : { kind: "error", message: r.message };
  };
  const paste = async () => {
    if (out.current) return;
    setError(null);
    out.current = true;
    setBusy(true);
    const current = scope.capture();
    try {
      const a = await readClipboard(Clipboard, clipboardFiles, rules);
      if (current()) deliver(a);
    } finally {
      out.current = false;
      setBusy(false);
    }
  };

  const name = host?.record.name ?? "";
  return (
    <SheetFrame title="Attach" subtitle={`To ${inProgram && program ? program : loc ? tabTitle(loc.tab) : "this pane"} on ${name || "a computer not found"}`} onClose={close}>
      <CardBox>
        {hasSystemPasteButton ? (
          // iOS's own paste control IS this row: tapping it is the consent, so no "Allow Paste" prompt
          <View style={styles.pasteRow}>
            <SystemPasteButton
              labelled
              ground={colors.bg}
              ink={colors.accent}
              onText={(text) => deliver(nativeAttached({ text }, rules))}
              onImage={(image) => deliver(nativeAttached({ image }, rules))}
              onFile={(file) => deliver(nativeAttached({ file }, rules))}
              style={styles.control}
            />
            <Text style={styles.pasteLine}>Paste from clipboard: text, an image or a file</Text>
          </View>
        ) : (
          <Row icon="paste" title="Paste from clipboard" line="Text, an image or a file" disabled={busy} onPress={() => void paste()} />
        )}
        <Row
          rule
          icon="photo"
          title="Photo library"
          line="Choose a photo to send"
          disabled={busy}
          onPress={() => void route(false, photo(pickLibraryPhoto))}
        />
        <Row rule icon="camera" title="Take a photo" line="Opens the camera" disabled={busy} onPress={() => void route(false, photo(takePhoto))} />
        <Row
          rule
          icon="file"
          title="File"
          line="Choose a file to send"
          disabled={busy}
          onPress={() =>
            void route(true, async () => {
              const r = await pickFile();
              return attachedItem(r.file, rules, r.name);
            })
          }
        />
      </CardBox>
      {error ? <ErrorText>{error}</ErrorText> : null}
    </SheetFrame>
  );
}

/** One way in: the list row with the route's icon in its tile. */
function Row({ icon, title, line, rule, disabled, onPress }: { icon: IconName; title: string; line: string; rule?: boolean; disabled: boolean; onPress: () => void }) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <NavRow
      first={!rule}
      label={title}
      detail={line}
      accessibilityLabel={`${title}. ${line}`}
      disabled={disabled}
      leading={
        <View style={styles.tile}>
          <Icon name={icon} size={16} color={colors.accent} />
        </View>
      }
      onPress={onPress}
    />
  );
}

const useStyles = makeStyles((colors) => ({
  tile: { width: 28, height: 28, borderRadius: radius.small + 2, backgroundColor: colors.faint, alignItems: "center", justifyContent: "center" },
  // the system control draws its own icon and word; the line under it says what it takes
  pasteRow: { gap: 4, paddingTop: 8, paddingBottom: 10, paddingHorizontal: 8 },
  control: { height: TOUCH, alignSelf: "stretch" },
  pasteLine: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16, textAlign: "center" },
}));
