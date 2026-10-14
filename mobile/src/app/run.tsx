import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { AccessibilityInfo, Keyboard, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { canOperate, draftKey, hasFeature, type HostView } from "@/fleet";
import { agoShort } from "@/now";
import type { Completion, HistoryEntry } from "@/protocol/types";
import { commandProblem } from "@/protocol/types";
import { fleet, useApp } from "@/runtime";
import { tabTitle } from "@/topology";
import { confirmDestructive } from "@/ui/confirm";
import { ErrorText } from "@/ui/Notice";
import { SheetClose } from "@/ui/SheetClose";
import { useNow } from "@/ui/useNow";
import { plainPunctuation } from "@/ui/format";
import { colors, mono, tints, ui } from "@/ui/theme";

/** A pane commands can run in: on a connected computer this device may operate. */
interface Target {
  host: string;
  pane: number;
  /** "studio-mac › rift › pane 1" */
  label: string;
  /** What the pane runs or last ran: tabs often share a title (the same folder), this tells them apart. */
  detail: string | null;
}

function targets(hosts: readonly HostView[]): Target[] {
  const out: Target[] = [];
  for (const h of hosts) {
    if (h.conn.status !== "open" || !canOperate(h)) continue;
    for (const w of h.windows)
      for (const t of w.tabs)
        for (const [i, p] of t.panes.entries())
          out.push({
            host: h.record.key,
            pane: p.id,
            label: `${h.record.name} › ${tabTitle(t)} › pane ${i + 1}`,
            detail: p.job ? `running ${p.job}` : p.last_command,
          });
  }
  return out;
}

/** Wait this long after typing stops before asking the computer for completions. */
const COMPLETE_DELAY_MS = 150;

/**
 * Run anywhere (the P5 board): pick the pane, type with the desktop's completion and this pane's
 * history, run it there.
 */
export default function RunSheet() {
  const params = useLocalSearchParams<{ key?: string; pane?: string }>();
  const router = useRouter();
  const s = useApp();
  const all = useMemo(() => targets(s.hosts), [s.hosts]);
  const initial = useMemo(() => {
    const want = params.key && params.pane ? { host: params.key, pane: Number(params.pane) } : s.lastView;
    const found = want ? all.find((t) => t.host === want.host && t.pane === want.pane) : undefined;
    if (found) return found;
    // the active pane of the first computer's active tab
    for (const h of s.hosts)
      for (const w of h.windows)
        for (const t of w.tabs)
          if (t.active) {
            const p = t.panes.find((x) => x.active);
            const hit = p && all.find((x) => x.host === h.record.key && x.pane === p.id);
            if (hit) return hit;
          }
    return all[0] ?? null;
    // only when the sheet opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [target, setTarget] = useState<Target | null>(initial);
  const [picking, setPicking] = useState(false);
  const [line, setLine] = useState("");
  const [items, setItems] = useState<Completion[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  // a command on its way: a second Return sends nothing
  const sending = useRef(false);
  const host = target ? s.hosts.find((h) => h.record.key === target.host) : undefined;
  const completes = hasFeature(host, "complete") && !!host && canOperate(host);
  // "4 min" moves on
  const now = useNow(60_000);
  // an older computer completes nothing: no lists (not the last pane's)
  const shownItems = completes ? items : [];
  const shownHistory = completes ? history : [];

  // completions and history for what is typed, once typing pauses
  useEffect(() => {
    if (!target || !completes) return;
    let live = true;
    const t = setTimeout(() => {
      void fleet.complete(target.host, target.pane, line).then((r) => {
        if (!live) return;
        // none from another pane, nor stale ones when the computer could not answer
        setItems(r.ok ? r.msg.items : []);
        setHistory(r.ok ? r.msg.history.filter((h) => h.text !== line) : []);
      });
    }, COMPLETE_DELAY_MS);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [target, line, completes]);

  const run = async (command: string) => {
    if (!target || sending.current) return;
    const problem = commandProblem(command);
    if (problem) {
      if (problem !== "empty") setError(problem === "too_long" ? "The command is too long (2 KiB at most)." : "The command must be one line.");
      return;
    }
    setError(null);
    if (s.settings.askBeforeRun) {
      // the Confirm sheet runs the pane's draft: replacing other text there loses it, so ask
      const draft = s.drafts[draftKey(target.host, target.pane)] ?? "";
      if (draft !== "" && draft !== command) {
        sending.current = true;
        const ok = await confirmDestructive("Replace the text in the command line?", `It holds: ${draft}`, "Replace");
        sending.current = false;
        if (!ok) return;
      }
      fleet.setDraft(target.host, target.pane, command);
      Keyboard.dismiss();
      router.replace({ pathname: "/confirm", params: { key: target.host, pane: String(target.pane) } });
      return;
    }
    sending.current = true;
    const r = await fleet.run(target.host, target.pane, command, { fromDraft: false });
    sending.current = false;
    if (!r.ok) {
      setError(r.message);
      return;
    }
    AccessibilityInfo.announceForAccessibility("Command sent");
    Keyboard.dismiss();
    router.back();
    router.push({ pathname: "/host/[key]/pane/[id]", params: { key: target.host, id: String(target.pane) } });
  };

  return (
    <View style={styles.sheet} accessibilityViewIsModal accessibilityLabel="Run a command">
      <SheetClose />
      <View style={styles.top}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={target ? `Runs in ${target.label.replace(/ › /g, ", ")}` : "No pane to run in"}
          accessibilityHint="Picks another pane"
          accessibilityState={{ expanded: picking, disabled: all.length < 2 }}
          disabled={all.length < 2}
          onPress={() => setPicking((p) => !p)}
          hitSlop={{ top: 6, bottom: 6 }}
          style={({ pressed }) => [styles.picker, pressed && styles.pressed]}
        >
          <Text style={styles.pickerText} numberOfLines={1} ellipsizeMode="middle">
            {/* the ▾ only where it opens something: one pane leaves nothing to pick */}
            {target ? `${target.label}${all.length > 1 ? " ▾" : ""}` : "No computer you can run commands on"}
          </Text>
        </Pressable>
        <View style={styles.field}>
          <Text style={styles.mark} importantForAccessibility="no" accessibilityElementsHidden>
            ❯
          </Text>
          <TextInput
            value={line}
            onChangeText={(t) => {
              setError(null);
              setLine(plainPunctuation(t));
            }}
            autoFocus
            autoCapitalize="none"
            autoCorrect={false}
            spellCheck={false}
            returnKeyType="send"
            submitBehavior="submit"
            onSubmitEditing={() => void run(line)}
            editable={!!target}
            placeholder="Command"
            placeholderTextColor={colors.textMuted}
            accessibilityLabel="Command"
            // the field's accent border is its focus; the browser's own ring drew a second box inside it
            style={[styles.input, Platform.OS === "web" && styles.noRing]}
          />
        </View>
        {error ? <ErrorText>{error}</ErrorText> : null}
      </View>
      <ScrollView style={styles.lists} contentContainerStyle={styles.listsContent} keyboardShouldPersistTaps="handled">
        {picking ? (
          all.map((t) => (
            <Pressable
              key={`${t.host}:${t.pane}`}
              accessibilityRole="radio"
              accessibilityState={{ checked: t.host === target?.host && t.pane === target?.pane }}
              accessibilityLabel={[t.label.replace(/ › /g, ", "), t.detail].filter(Boolean).join(", ")}
              onPress={() => {
                // the last pane's completions and history are not this one's
                if (t.host !== target?.host || t.pane !== target?.pane) {
                  setItems([]);
                  setHistory([]);
                }
                setTarget(t);
                setPicking(false);
              }}
              style={({ pressed }) => [styles.item, t.host === target?.host && t.pane === target?.pane && styles.itemOn, pressed && styles.pressed]}
            >
              <View style={styles.flex}>
                {/* cut in the middle, as the picker above: at 320 pt the end cut every row to the same "…" */}
                <Text style={styles.itemText} numberOfLines={1} ellipsizeMode="middle">
                  {t.label}
                </Text>
                {t.detail ? (
                  <Text style={styles.itemDetail} numberOfLines={1}>
                    {t.detail}
                  </Text>
                ) : null}
              </View>
            </Pressable>
          ))
        ) : (
          <>
            {shownItems.length > 0 ? (
              <>
                <Text style={styles.heading} accessibilityRole="header">
                  Complete
                </Text>
                {shownItems.map((c, i) => (
                  <Pressable
                    key={`${i}:${c.kind}:${c.text}`}
                    accessibilityRole="button"
                    accessibilityLabel={`${c.text}, ${c.kind}`}
                    accessibilityHint="Puts it in the command line"
                    onPress={() => setLine(c.text)}
                    style={({ pressed }) => [styles.item, i === 0 && styles.itemOn, pressed && styles.pressed]}
                  >
                    <Text style={[styles.itemText, styles.flex]} numberOfLines={1}>
                      {c.text}
                    </Text>
                    <Text style={styles.itemMeta}>{c.kind}</Text>
                  </Pressable>
                ))}
              </>
            ) : null}
            {shownHistory.length > 0 ? (
              <>
                <Text style={[styles.heading, styles.headingGap]} accessibilityRole="header">
                  Ran here before
                </Text>
                {shownHistory.map((h, i) => (
                  <Pressable
                    key={`${i}:${h.text}`}
                    accessibilityRole="button"
                    accessibilityLabel={h.at !== null ? `${h.text}, ${agoShort(now - h.at * 1000)}` : h.text}
                    accessibilityHint="Puts it in the command line"
                    onPress={() => setLine(h.text)}
                    style={({ pressed }) => [styles.item, pressed && styles.pressed]}
                  >
                    <Text style={[styles.itemText, styles.flex]} numberOfLines={1}>
                      {h.text}
                    </Text>
                    {h.at !== null ? <Text style={styles.itemMeta}>{agoShort(now - h.at * 1000)}</Text> : null}
                  </Pressable>
                ))}
              </>
            ) : null}
            {!completes && target ? (
              <Text style={styles.note}>Update Rift on this computer for its completions and history here.</Text>
            ) : null}
          </>
        )}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  sheet: { flex: 1, backgroundColor: colors.surface },
  pressed: { opacity: 0.7 },
  flex: { flex: 1, minWidth: 0 },
  // 12 pt on the board, below the system grabber (24 pt) here
  top: { paddingTop: 24, paddingHorizontal: 16, paddingBottom: 8, gap: 10 },
  picker: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    height: 32,
    paddingHorizontal: 10,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    justifyContent: "center",
  },
  pickerText: { fontFamily: ui(400), color: colors.text, fontSize: 13 },
  field: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    height: 46,
    paddingHorizontal: 12,
    borderRadius: 10,
    backgroundColor: colors.bg,
    borderWidth: 1,
    borderColor: colors.accent,
  },
  mark: { fontFamily: mono, color: colors.success, fontSize: 14 },
  input: { flex: 1, minWidth: 0, height: 46, padding: 0, color: colors.textBright, fontFamily: mono, fontSize: 14 },
  // web only: Chrome draws `outline-style: auto` whatever its width ("none" is not in the native types)
  noRing: { outlineStyle: "none" as "solid" },
  lists: { flex: 1 },
  listsContent: { paddingHorizontal: 8, paddingBottom: 24 },
  heading: { fontFamily: ui(500), color: colors.textMuted, fontSize: 12, marginTop: 6, marginBottom: 6, marginHorizontal: 8 },
  headingGap: { marginTop: 10 },
  item: { minHeight: 44, flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 10, paddingVertical: 6, borderRadius: 8 },
  itemOn: { backgroundColor: tints.accentBadge },
  itemText: { fontFamily: mono, color: colors.textBright, fontSize: 13 },
  itemMeta: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  itemDetail: { fontFamily: mono, color: colors.textMuted, fontSize: 11, marginTop: 2 },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, padding: 8 },
});
