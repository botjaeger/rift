import { usePathname, useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { AccessibilityInfo, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { ActivityItem } from "@/fleet";
import { primaryChoice } from "@/answer";
import { avatarFor, folderOf } from "@/avatar";
import { needsAuthBeforeRun } from "@/lock";
import {
  agoShort,
  clockOf,
  hostMarks,
  nowSections,
  quickChoices,
  restSummary,
  WORKING_CAP,
  type HostMark,
  type NeedsItem,
  type RunningRow,
} from "@/now";
import { useNowFilter } from "@/nowFilter";
import { authenticate, fleet, useApp } from "@/runtime";
import { restoreTarget } from "@/stores";
import { Avatar } from "@/ui/Avatar";
import { Button } from "@/ui/Button";
import { formatClock } from "@/ui/format";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { PromptText } from "@/ui/PromptText";
import { TabScreen, tabBottom, titleTop } from "@/ui/Screen";
import { useNow } from "@/ui/useNow";
import { colors, mono, tint, tints, ui } from "@/ui/theme";

/** The last pane is reopened once per app run, on the first launch of Now. */
let restored = false;

/** First run (the Empty board): how to connect, then Scan the code. */
function FirstRun() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const steps = [
    <Text key="1" style={styles.stepText}>
      Open Rift on your Mac or Linux computer.
    </Text>,
    <Text key="2" style={styles.stepText}>
      Settings › Remote access: turn on <Text style={styles.bold}>Allow remote access</Text>, then{" "}
      <Text style={styles.bold}>Pair a phone</Text>. A code appears.
    </Text>,
    <Text key="3" style={styles.stepText}>
      Scan it here. The code works once, for 2 minutes.
    </Text>,
  ];
  return (
    <View style={[styles.firstRun, { paddingTop: titleTop(insets.top) }]}>
      <Text style={styles.firstTitle} accessibilityRole="header">
        Now
      </Text>
      {/* scrolls on a short phone (320 × 568), where the steps would run under the buttons */}
      <ScrollView style={styles.flex} contentContainerStyle={styles.firstBody}>
        <View style={styles.intro}>
          <Text style={styles.h2} accessibilityRole="header">
            Connect to Rift on your computer
          </Text>
          <Text style={styles.lead}>Watch its panes, run commands and get told when they finish.</Text>
        </View>
        <View style={styles.steps} accessibilityRole="list">
          {steps.map((text, i) => (
            <View key={i} style={[styles.step, i > 0 && styles.stepDivider]} accessible>
              <View style={styles.stepNo}>
                <Text style={styles.stepNoText}>{i + 1}</Text>
              </View>
              {text}
            </View>
          ))}
        </View>
      </ScrollView>
      {/* above the floating tab bar */}
      <View style={[styles.firstActions, { paddingBottom: tabBottom(insets.bottom) }]}>
        <Button
          label="Scan the code"
          variant="primary"
          size="large"
          icon={<Icon name="scan" color={colors.bg} />}
          onPress={() => router.push("/scan")}
        />
        <Button label="Paste a pairing link" variant="link" onPress={() => router.push("/add")} />
      </View>
    </View>
  );
}

const openAnswer = (router: ReturnType<typeof useRouter>, n: NeedsItem, reply = false) =>
  router.push({ pathname: "/answer", params: { key: n.host, pane: String(n.pane), ...(reply ? { reply: "1" } : {}) } });

const openPane = (router: ReturnType<typeof useRouter>, host: string, pane: number) =>
  router.push({ pathname: "/host/[key]/pane/[id]", params: { key: host, id: String(pane) } });

/** Where a row's pane is: its computer (when Now shows several), its folder (else its tab), its branch. */
const placeOf = (hostName: string | null, cwd: string | null, tab: string, branch: string | null) =>
  [hostName, folderOf(cwd) ?? tab, branch].filter(Boolean).join(" · ");

/** A coding agent's menu (Needs you): who asks, the question, and its answers as buttons. */
function AgentCard({ n, since, mark }: { n: NeedsItem; since: string; mark: HostMark | null }) {
  const router = useRouter();
  const s = useApp();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // a second tap before the re-render that disables the buttons sends nothing
  const sending = useRef(false);
  const quick = n.operate ? quickChoices(n.waiting) : null;
  const primary = primaryChoice(n.waiting);
  const name = n.title ?? n.program ?? n.tab;
  const where = placeOf(mark ? n.hostName : null, n.cwd, n.tab, n.branch);
  const pick = async (index: number) => {
    if (sending.current) return;
    sending.current = true;
    setError(null);
    if (needsAuthBeforeRun(s.settings)) {
      const auth = await authenticate(`Answer ${n.program ?? "the program"} on ${n.hostName}`);
      if (!auth.ok) {
        sending.current = false;
        if (auth.message !== "Cancelled.") setError(auth.message);
        return;
      }
    }
    setBusy(true);
    const r = await fleet.answer(n.host, n.pane, n.waiting, { choice: index });
    setBusy(false);
    sending.current = false;
    if (!r.ok) setError(r.message);
    else AccessibilityInfo.announceForAccessibility("Answer sent");
  };
  return (
    <View style={styles.agentCard}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${name}, ${[n.program, n.hostName, n.tab].filter(Boolean).join(", ")}, ${since === "now" ? "just now" : `${since} ago`}: ${n.waiting.prompt}`}
        accessibilityHint="Opens the question"
        onPress={() => openAnswer(router, n)}
        style={({ pressed }) => [styles.agentBody, pressed && styles.pressed]}
      >
        <View style={styles.agentTop}>
          <Avatar avatar={avatarFor(n.cwd, n.tab, n.program)} mark={mark} />
          <View style={styles.flex}>
            <Text style={styles.title} numberOfLines={1}>
              {name}
            </Text>
            <Text style={styles.meta} numberOfLines={1}>
              {where ? `${where} · ${since}` : since}
            </Text>
          </View>
        </View>
        <PromptText text={n.waiting.prompt} style={styles.agentPrompt} monoSize={13} />
      </Pressable>
      {quick ? (
        <View style={styles.quick}>
          {quick.map((q) => (
            <Pressable
              key={q.index}
              accessibilityRole="button"
              accessibilityLabel={q.opens ? `${n.waiting.choices[q.index]}, opens the question` : n.waiting.choices[q.index]}
              accessibilityState={{ disabled: busy }}
              disabled={busy}
              hitSlop={{ top: 2, bottom: 2 }}
              onPress={() => (q.opens ? openAnswer(router, n, true) : void pick(q.index))}
              style={({ pressed }) => [styles.quickButton, q.index === primary ? styles.quickPrimary : styles.quickOutline, pressed && styles.pressed]}
            >
              {/* a third of a 320 pt phone cut "Yes, always" to "Yes, alw…": shrink to fit first */}
              <Text
                style={q.index === primary ? styles.quickPrimaryText : styles.quickText}
                numberOfLines={1}
                adjustsFontSizeToFit
                minimumFontScale={0.75}
              >
                {q.label}
              </Text>
            </Pressable>
          ))}
        </View>
      ) : null}
      {error ? <ErrorText>{error}</ErrorText> : null}
    </View>
  );
}

/** A password or other question (Needs you): one row that opens the answer. */
function AskRow({ n }: { n: NeedsItem }) {
  const router = useRouter();
  const meta = [n.command, n.hostName].filter(Boolean).join(" · ");
  const password = n.waiting.kind === "password";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${n.waiting.prompt}, ${meta}`}
      accessibilityHint="Opens the question"
      onPress={() => openAnswer(router, n)}
      style={({ pressed }) => [styles.askRow, pressed && styles.pressed]}
    >
      <View style={[styles.dot, { backgroundColor: colors.warning }]} />
      <View style={styles.askBody}>
        <Text style={password ? styles.askPromptMono : styles.askPrompt} numberOfLines={2}>
          {n.waiting.prompt}
        </Text>
        <Text style={styles.meta} numberOfLines={1}>
          {meta}
        </Text>
      </View>
      <Text style={styles.answerLink}>Answer</Text>
    </Pressable>
  );
}

/** A working row: the session's name (a coding agent's) or the command, how long, its latest line, where. */
function WorkingLine({ r, now, first, mark }: { r: RunningRow; now: number; first: boolean; mark: HostMark | null }) {
  const router = useRouter();
  const elapsed = r.started !== null ? formatClock(now - r.started) : "";
  const where = placeOf(mark ? r.hostName : null, r.cwd, r.tab, r.branch);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={[
        r.title ? `${r.title}, ${r.command} working` : `${r.command} running`,
        elapsed ? `for ${elapsed}` : null,
        r.hostName,
        r.tab,
        r.notifies ? "notifies when done" : null,
      ]
        .filter(Boolean)
        .join(", ")}
      accessibilityHint="Opens the pane"
      onPress={() => openPane(router, r.host, r.pane)}
      style={({ pressed }) => [styles.runRow, !first && styles.divider, pressed && styles.pressed]}
    >
      <Avatar avatar={avatarFor(r.cwd, r.tab, r.program)} mark={mark} />
      <View style={styles.runBody}>
        <View style={styles.runTop}>
          <Text style={r.title ? styles.titleFlex : styles.cmd} numberOfLines={1}>
            {r.title ?? (r.command || " ")}
          </Text>
          <Text style={styles.elapsed}>{elapsed}</Text>
        </View>
        {r.line ? (
          <Text style={styles.lastLine} numberOfLines={1}>
            {r.line}
          </Text>
        ) : null}
        <View style={styles.runMeta}>
          <Text style={[styles.meta, styles.flex]} numberOfLines={1}>
            {where}
          </Text>
          {r.notifies ? <Text style={styles.notifies}>Notifies when done</Text> : null}
        </View>
      </View>
    </Pressable>
  );
}

/** A coding agent at its own prompt (Idle): a compact row, its avatar greyed. */
function IdleLine({ r, first, mark }: { r: RunningRow; first: boolean; mark: HostMark | null }) {
  const router = useRouter();
  const name = r.title ?? r.program ?? r.tab;
  // the monogram already names the computer: the folder alone
  const where = folderOf(r.cwd) ?? r.tab;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${name}, idle, ${r.hostName}, ${r.tab}`}
      accessibilityHint="Opens the pane"
      onPress={() => openPane(router, r.host, r.pane)}
      style={({ pressed }) => [styles.idleRow, !first && styles.divider, pressed && styles.pressed]}
    >
      <Avatar avatar={avatarFor(r.cwd, r.tab, r.program)} size={28} mark={mark} muted />
      <Text style={styles.idleName} numberOfLines={1}>
        {name}
      </Text>
      <Text style={[styles.meta, styles.idlePlace]} numberOfLines={1}>
        {where}
      </Text>
    </Pressable>
  );
}

function FinishedLine({ a, first }: { a: ActivityItem; first: boolean }) {
  const router = useRouter();
  const failed = a.exit !== null && a.exit !== 0;
  const result = a.exit === null ? "finished" : failed ? `failed, exit ${a.exit}` : "succeeded";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${a.command || "Command"} ${result} on ${a.hostName}${a.tab ? `, ${a.tab}` : ""}, at ${clockOf(a.at)}`}
      accessibilityHint="Opens the pane at this command"
      onPress={() =>
        router.push({ pathname: "/host/[key]/pane/[id]", params: { key: a.host, id: String(a.pane), block: String(a.block) } })
      }
      style={({ pressed }) => [styles.doneRow, !first && styles.divider, pressed && styles.pressed]}
    >
      <Text numberOfLines={1} style={[styles.status, { color: failed ? colors.textBright : a.exit === 0 ? colors.success : colors.textMuted }]}>
        {failed ? `✕ ${a.exit}` : a.exit === 0 ? "✓" : "·"}
      </Text>
      <Text style={[styles.cmd, styles.flex]} numberOfLines={1}>
        {a.command || " "}
      </Text>
      <Text style={styles.meta}>{clockOf(a.at)}</Text>
    </Pressable>
  );
}

/** Stacked colour dots: the computers Now shows, or the sessions behind "N more working". */
function Dots({ colors: dots, ground }: { colors: string[]; ground: string }) {
  return (
    <View style={styles.dots}>
      {dots.map((c, i) => (
        <View key={i} style={[styles.stackDot, { backgroundColor: c, borderColor: ground }, i > 0 && styles.stacked]} />
      ))}
    </View>
  );
}

/** Now (the P1 board): what needs you, what works, what waits or finished, on every connected computer or one. */
export default function NowScreen() {
  const router = useRouter();
  const s = useApp();
  const filter = useNowFilter();
  // a computer unpaired since it was picked: all of them again
  const only = filter !== null && s.hosts.some((h) => h.record.key === filter) ? filter : null;
  const marks = useMemo(() => hostMarks(s.hosts), [s.hosts]);
  // the monograms only while Now mixes several computers
  const markOf = (key: string) => (only === null && s.hosts.length > 1 ? (marks.get(key) ?? null) : null);
  // not again on every tick of the running clocks below
  const { needs, running, idle, finished } = useMemo(
    () => nowSections(s.hosts, s.activity, s.asked, only),
    [s.hosts, s.activity, s.asked, only],
  );
  const [allWorking, setAllWorking] = useState(false);
  const [restOpen, setRestOpen] = useState(false);
  // seconds for the running clocks, else minutes ("1 min" since a question came)
  const now = useNow(running.some((r) => r.started !== null) ? 1000 : 30_000);

  // A cold start goes straight back to the pane open last, while its computer is still paired.
  // Only when Now is on screen then: a pane already on top (this restore, a tapped alert) is
  // not pushed a second time, whose Back would then only lead to the first.
  const path = usePathname();
  useEffect(() => {
    if (restored || !s.ready) return;
    restored = true;
    if (path !== "/") return;
    const target = restoreTarget(
      s.lastView,
      s.hosts.map((h) => h.record),
    );
    if (!target || target.pane === null) return;
    router.push({ pathname: "/host/[key]/pane/[id]", params: { key: target.host, id: String(target.pane), restored: "1" } });
  }, [s.ready, s.lastView, s.hosts, router, path]);

  if (s.ready && !s.loadError && s.hosts.length === 0) return <FirstRun />;

  const picked = only === null ? null : (s.hosts.find((h) => h.record.key === only) ?? null);
  const anyOpen = (picked ? [picked] : s.hosts).some((h) => h.conn.status === "open");
  const runButton = (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Run a command"
      hitSlop={{ top: 2, bottom: 2 }}
      onPress={() => router.push("/run")}
      style={({ pressed }) => [styles.runButton, pressed && styles.pressed]}
    >
      <Icon name="terminal" color={colors.bg} />
    </Pressable>
  );
  const pickerLabel = picked ? picked.record.name : `All ${s.hosts.length} computers`;
  const picker =
    s.hosts.length > 1 ? (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Showing ${pickerLabel}`}
        accessibilityHint="Chooses the computers Now shows"
        onPress={() => router.push("/pick-computer")}
        style={({ pressed }) => [styles.picker, pressed && styles.pressed]}
      >
        <Dots colors={(picked ? [picked] : s.hosts).map((h) => marks.get(h.record.key)!.color)} ground={colors.chrome} />
        <Text style={styles.pickerText} numberOfLines={1}>
          {pickerLabel}
        </Text>
        <View style={styles.chevronDown}>
          <Icon name="forward" size={12} color={colors.textMuted} />
        </View>
      </Pressable>
    ) : null;

  const working = allWorking ? running : running.slice(0, WORKING_CAP);
  const more = running.length - working.length;
  const rest = restSummary(idle.length, finished);
  const restLabel = rest
    ? [rest.idle ? `${rest.idle} idle` : null, rest.finished ? `${rest.finished} finished` : null].filter(Boolean).join(" · ")
    : "";

  return (
    <TabScreen title="Now" accessory={runButton} gap={18} large>
      {picker}
      {s.loadError ? (
        <View style={styles.loadError}>
          <ErrorText>Could not read this device&apos;s keys: {s.loadError}. Unlock the device, then try again.</ErrorText>
          <Button label="Try again" variant="primary" onPress={() => void fleet.retryInit()} />
        </View>
      ) : null}
      {needs.length === 0 && running.length === 0 && !rest ? (
        <Text style={styles.empty}>
          {anyOpen
            ? "Nothing needs you. Commands that run or finish on your computers show up here."
            : picked
              ? `Not connected to ${picked.record.name}. Computers shows how it is doing.`
              : // not "yet": a computer that dropped or unpaired this phone was connected before
                "Not connected to your computers. Computers shows how each one is doing."}
        </Text>
      ) : null}
      {needs.length > 0 ? (
        <View style={styles.section}>
          <Text style={[styles.sectionTitle, { color: colors.warning }]} accessibilityRole="header">
            Needs you · {needs.length}
          </Text>
          {needs.map((n) =>
            n.waiting.kind === "choice" && quickChoices(n.waiting) ? (
              <AgentCard
                key={n.key}
                n={n}
                since={agoShort(now - (fleet.waitingSince(n.host, n.pane, n.waiting) ?? now))}
                mark={markOf(n.host)}
              />
            ) : (
              <AskRow key={n.key} n={n} />
            ),
          )}
        </View>
      ) : null}
      {running.length > 0 ? (
        <View style={styles.section}>
          <Text style={[styles.sectionTitle, { color: colors.accent }]} accessibilityRole="header">
            Working · {running.length}
          </Text>
          <View style={styles.card}>
            {working.map((r, i) => (
              <WorkingLine key={r.key} r={r} now={now} first={i === 0} mark={markOf(r.host)} />
            ))}
            {running.length > WORKING_CAP ? (
              <Pressable
                accessibilityRole="button"
                accessibilityState={{ expanded: allWorking }}
                accessibilityLabel={more > 0 ? `${more} more working` : "Show fewer"}
                onPress={() => setAllWorking((v) => !v)}
                style={({ pressed }) => [styles.moreRow, styles.divider, pressed && styles.pressed]}
              >
                {more > 0 ? (
                  <Dots
                    colors={running.slice(WORKING_CAP, WORKING_CAP + 4).map((r) => tint(avatarFor(r.cwd, r.tab, r.program).color, 0x2e))}
                    ground={colors.surface}
                  />
                ) : null}
                <Text style={[styles.moreText, styles.flex]}>{more > 0 ? `${more} more working` : "Show fewer"}</Text>
                <View style={more > 0 ? null : styles.chevronUp}>
                  <Icon name="forward" size={14} color={colors.textMuted} />
                </View>
              </Pressable>
            ) : null}
          </View>
        </View>
      ) : null}
      {rest ? (
        <View style={styles.section}>
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ expanded: restOpen }}
            accessibilityLabel={[restLabel, rest.failed ? `${rest.failed} failed` : null].filter(Boolean).join(", ")}
            onPress={() => setRestOpen((v) => !v)}
            style={({ pressed }) => [styles.summary, pressed && styles.pressed]}
          >
            <View style={[styles.dot, { backgroundColor: rest.failed ? colors.error : colors.textDim }]} />
            <Text style={[styles.summaryText, styles.flex]} numberOfLines={1}>
              {restLabel}
              {rest.failed ? (
                <Text style={styles.summaryFailed}>
                  {restLabel ? " · " : ""}
                  {rest.failed} failed
                </Text>
              ) : null}
            </Text>
            <View style={restOpen ? styles.chevronDown : null}>
              <Icon name="forward" size={14} color={colors.textMuted} />
            </View>
          </Pressable>
          {restOpen && idle.length > 0 ? (
            <View style={styles.section}>
              <Text style={[styles.sectionTitle, { color: colors.textMuted }]} accessibilityRole="header">
                Idle · {idle.length}
              </Text>
              <View style={styles.card}>
                {idle.map((r, i) => (
                  <IdleLine key={r.key} r={r} first={i === 0} mark={markOf(r.host)} />
                ))}
              </View>
            </View>
          ) : null}
          {restOpen && finished.length > 0 ? (
            <View style={styles.section}>
              <Text style={[styles.sectionTitle, { color: colors.textMuted }]} accessibilityRole="header">
                Finished · {finished.length}
              </Text>
              <View style={styles.card}>
                {finished.map((a, i) => (
                  <FinishedLine key={a.id} a={a} first={i === 0} />
                ))}
              </View>
            </View>
          ) : null}
        </View>
      ) : null}
    </TabScreen>
  );
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  flex: { flex: 1, minWidth: 0 },
  loadError: { gap: 12, paddingHorizontal: 8 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20, paddingHorizontal: 8 },
  // the header's Run button: 40 pt round, accent
  runButton: { width: 40, height: 40, borderRadius: 20, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center" },
  // the computer picker under the title (several paired): a glass pill with each computer's dot
  picker: {
    alignSelf: "flex-start",
    marginTop: -6,
    marginLeft: 8,
    height: 32,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.glassRim,
    backgroundColor: colors.glass,
    paddingLeft: 6,
    paddingRight: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    maxWidth: "90%",
  },
  pickerText: { flexShrink: 1, fontFamily: ui(400), color: colors.text, fontSize: 13 },
  dots: { flexDirection: "row" },
  stackDot: { width: 18, height: 18, borderRadius: 9, borderWidth: 2 },
  stacked: { marginLeft: -6 },
  chevronDown: { transform: [{ rotate: "90deg" }] },
  chevronUp: { transform: [{ rotate: "-90deg" }] },
  section: { gap: 8 },
  sectionTitle: { fontFamily: ui(600), fontSize: 13, paddingHorizontal: 8 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  // Needs you: the agent's card
  agentCard: {
    gap: 10,
    padding: 14,
    borderRadius: 14,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: `${colors.warning}55`,
  },
  agentBody: { gap: 10 },
  agentTop: { flexDirection: "row", alignItems: "center", gap: 12 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  titleFlex: { flex: 1, minWidth: 0, fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  agentPrompt: { fontFamily: ui(400), color: colors.textBright, fontSize: 15, lineHeight: 21 },
  quick: { flexDirection: "row", gap: 8 },
  quickButton: { flex: 1, height: 40, borderRadius: 20, alignItems: "center", justifyContent: "center", paddingHorizontal: 6 },
  quickPrimary: { backgroundColor: colors.accent },
  quickOutline: { borderWidth: 1, borderColor: colors.borderStrong },
  quickPrimaryText: { fontFamily: ui(600), color: colors.bg, fontSize: 14 },
  quickText: { fontFamily: ui(400), color: colors.text, fontSize: 14 },
  // Needs you: a password or other question
  askRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 12,
    paddingHorizontal: 14,
    borderRadius: 14,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  askBody: { flex: 1, minWidth: 0, gap: 3 },
  askPromptMono: { fontFamily: mono, color: colors.textBright, fontSize: 13 },
  askPrompt: { fontFamily: ui(400), color: colors.textBright, fontSize: 14 },
  answerLink: { fontFamily: ui(400), color: colors.accent, fontSize: 13 },
  meta: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  // Running and Finished
  card: {
    borderRadius: 14,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  // the Now board's working rows: the pane's avatar, then its name or command, last line and place
  runRow: { flexDirection: "row", gap: 12, paddingVertical: 12, paddingHorizontal: 14 },
  runBody: { flex: 1, minWidth: 0, gap: 6 },
  runTop: { flexDirection: "row", alignItems: "baseline", gap: 8 },
  cmd: { flex: 1, minWidth: 0, fontFamily: mono, color: colors.textBright, fontSize: 13 },
  elapsed: { fontFamily: mono, color: colors.accent, fontSize: 12, fontVariant: ["tabular-nums"] },
  lastLine: { fontFamily: mono, color: colors.textMuted, fontSize: 11 },
  runMeta: { flexDirection: "row", gap: 8 },
  notifies: { fontFamily: ui(400), color: colors.accent, fontSize: 12 },
  // "4 more working", and the summary row under Working
  moreRow: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: 11, paddingHorizontal: 14, minHeight: 44 },
  moreText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    height: 44,
    paddingHorizontal: 14,
    borderRadius: 22,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  summaryText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  summaryFailed: { fontFamily: ui(500), color: colors.textBright },
  // Idle: a coding agent at its own prompt
  idleRow: { flexDirection: "row", alignItems: "center", gap: 12, paddingVertical: 10, paddingHorizontal: 14 },
  idleName: { flex: 1, minWidth: 0, fontFamily: ui(500), color: colors.text, fontSize: 14 },
  idlePlace: { flexShrink: 1, maxWidth: "35%" },
  doneRow: { flexDirection: "row", alignItems: "center", gap: 12, paddingVertical: 12, paddingHorizontal: 14 },
  status: { minWidth: 34, fontFamily: mono, fontSize: 12 },
  // first run
  firstRun: { flex: 1, backgroundColor: colors.chrome },
  firstTitle: {
    fontFamily: ui(600),
    color: colors.textBright,
    fontSize: 30,
    letterSpacing: -0.6,
    paddingHorizontal: 20,
    paddingBottom: 8,
  },
  firstBody: { paddingTop: 24, paddingBottom: 8, paddingHorizontal: 20, gap: 22 },
  intro: { gap: 8 },
  h2: { fontFamily: ui(600), color: colors.textBright, fontSize: 22, lineHeight: 27.5 },
  lead: { fontFamily: ui(400), color: colors.textMuted, fontSize: 15, lineHeight: 22.5 },
  steps: {
    backgroundColor: colors.surface,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.hairline,
    overflow: "hidden",
  },
  step: { flexDirection: "row", gap: 14, padding: 14 },
  stepDivider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  stepNo: {
    width: 24,
    height: 24,
    borderRadius: 12,
    backgroundColor: tints.accentBadge,
    alignItems: "center",
    justifyContent: "center",
  },
  stepNoText: { fontFamily: ui(600), color: colors.accent, fontSize: 13 },
  stepText: { fontFamily: ui(400), flex: 1, color: colors.text, fontSize: 14, lineHeight: 20.3 },
  bold: { fontFamily: ui(600), color: colors.textBright },
  firstActions: { paddingTop: 16, paddingHorizontal: 20, gap: 10 },
});
