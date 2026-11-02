import { useActionScope } from "@/ui/useActionScope";
import * as Clipboard from "expo-clipboard";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Pressable, StyleSheet, Text, View } from "react-native";
import { clipboardFiles, hasSystemPasteButton, SystemPasteButton } from "../../modules/rift-keys";
import { draftKey, hasFeature } from "@/fleet";
import { attachedItem, nativeAttached, pending, PENDING_FULL, readClipboard, routeRefusal, type AttachRules, type Attached } from "@/pending";
import { pickFile } from "@/pickFile";
import { pickLibraryPhoto, takePhoto } from "@/pickPhoto";
import { useApp, useLock } from "@/runtime";
import { Group, SheetFrame } from "@/ui/NewSheet";
import { ErrorText } from "@/ui/Notice";
import { Icon, type IconName } from "@/ui/Icon";
import { colors, TOUCH, ui } from "@/ui/theme";

/**
 * The Attach sheet (the Attach board), opened by the message box's +: Paste from clipboard,
 * Photo library, Take a photo, File. Text goes into the box, an image or file becomes a token in it
 * until Send (pending.ts); the sheet then closes back to the pane, which keeps its draft. Cancel
 * (Close, or the sheet's swipe) changes nothing. At a shell prompt an image or file is refused,
 * as a Paste is: there is no program to take it.
 */
export default function AttachSheet() {
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
  const online = host?.conn.status === "open";
  return (
    <SheetFrame title="Attach" subtitle={`To ${inProgram && program ? program : "this pane"} · ${name || "computer not found"}`} online={!!online} onClose={close}>
      <Group>
        {hasSystemPasteButton ? (
          // iOS's own paste control IS this row: tapping it is the consent, so no "Allow Paste" prompt
          <View style={styles.pasteRow}>
            <SystemPasteButton
              labelled
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
      </Group>
      {error ? <ErrorText>{error}</ErrorText> : null}
    </SheetFrame>
  );
}

function Row({ icon, title, line, rule, disabled, onPress }: { icon: IconName; title: string; line: string; rule?: boolean; disabled: boolean; onPress: () => void }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${title}. ${line}`}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.row, rule && styles.rule, disabled && styles.disabled, pressed && styles.pressed]}
    >
      <Icon name={icon} size={20} color={colors.accent} />
      <View style={styles.text}>
        <Text style={styles.title}>{title}</Text>
        <Text style={styles.line}>{line}</Text>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  row: { flexDirection: "row", alignItems: "center", gap: 14, minHeight: 60, paddingVertical: 8, paddingHorizontal: 16 },
  rule: { borderTopWidth: 1, borderTopColor: colors.hairline },
  text: { flex: 1, minWidth: 0, gap: 2 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  line: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  // the system control draws its own icon and word; the line under it says what it takes
  pasteRow: { gap: 4, paddingTop: 8, paddingBottom: 10, paddingHorizontal: 8 },
  control: { height: TOUCH, alignSelf: "stretch" },
  pasteLine: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16, textAlign: "center" },
});
