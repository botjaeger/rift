import { useActionScope } from "@/ui/useActionScope";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { branchRows, chipCommand, dirRows, splitPath, tildePath, type ChipAction } from "@/chips";
import { canOperate } from "@/fleet";
import { containerRows, dockerTitle, elapsedBody, isPrUrl, versionRows, type ChipOpen } from "@/promptChips";
import type { BranchesMsg, ChangedFile, ChangesMsg, ChipMsg, DirsMsg } from "@/protocol/types";
import { fleet, useApp, useLock } from "@/runtime";
import { findPane, paneTitle } from "@/topology";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { SheetClose } from "@/ui/SheetClose";
import { colors, mono, monoMedium, radius, ui } from "@/ui/theme";

type Menu = ChipOpen;
type Answer = DirsMsg | BranchesMsg | ChangesMsg | ChipMsg;
const MENUS: readonly Menu[] = ["dirs", "branches", "changes", "pr", "python", "php", "docker", "elapsed"];

const STATUS: Record<string, { word: string; color: string }> = {
  M: { word: "modified", color: colors.warning },
  A: { word: "added", color: colors.success },
  D: { word: "deleted", color: colors.textBright },
  R: { word: "renamed", color: colors.accent },
  C: { word: "copied", color: colors.accent },
  T: { word: "type changed", color: colors.warning },
  "?": { word: "untracked", color: colors.textMuted },
  U: { word: "conflicted", color: colors.textBright },
};

/** One row of a menu card (the FolderSheet / BranchSheet boards). */
function Row({
  icon,
  label,
  spoken,
  first,
  onPress,
  disabled,
  accent,
  danger,
  mark,
  detail,
  health,
  dimLabel,
}: {
  icon: React.ReactNode;
  label: string;
  spoken: string;
  first: boolean;
  onPress: () => void;
  disabled?: boolean;
  accent?: boolean;
  /** A destructive pick (Reset): the label in the error colour. */
  danger?: boolean;
  /** The current branch: not a choice, but drawn at full strength. */
  mark?: string;
  /** Muted text after the label: a version's source, a container's ports and uptime. */
  detail?: string;
  /** A container's health check verdict, in its colour. */
  health?: { word: string; color: string } | null;
  /** The label is dimmed (a stopped container). */
  dimLabel?: boolean;
}) {
  const dim = disabled && !mark;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={spoken}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.row, !first && styles.divider, pressed && styles.pressed, dim && styles.disabled]}
    >
      <View style={styles.icon}>{icon}</View>
      <Text style={[styles.label, accent ? styles.accentLabel : null, danger ? styles.dangerLabel : null, dimLabel && styles.dimLabel]} numberOfLines={1}>
        {label}
      </Text>
      {detail ? (
        <Text style={styles.detail} numberOfLines={1}>
          {detail}
        </Text>
      ) : null}
      {health ? <Text style={[styles.detail, { color: health.color }]}>{health.word}</Text> : null}
      {mark ? <Text style={styles.mark}>{mark}</Text> : null}
    </Pressable>
  );
}

/**
 * A prompt chip's menu (the FolderSheet, BranchSheet and ChangesSheet boards): what the desktop's
 * chip opens. A folder or branch runs `cd` / `git checkout` in the pane through Run (the Confirm
 * sheet first with Ask before running on); a changed file opens its diff.
 */
export default function ChipsScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { key, pane: paneParam, menu: menuParam } = useLocalSearchParams<{ key: string; pane: string; menu: string }>();
  const menu: Menu = MENUS.find((m) => m === menuParam) ?? "dirs";
  const paneId = Number(paneParam);
  const scope = useActionScope(`${key}:${paneId}:${menu}`);
  const s = useApp();
  const lock = useLock();
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [sending, setSending] = useState(false);

  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked && scope.leave()) router.back();
  }, [lock.locked, router, scope]);

  useEffect(() => {
    let live = true;
    const asked =
      menu === "dirs" || menu === "branches" || menu === "changes" ? fleet.chipMenu(key, paneId, menu) : fleet.chipDetail(key, paneId, menu);
    void asked.then((r) => {
      if (!live) return;
      if (r.ok) setAnswer(r.msg);
      else setError(r.message);
    });
    return () => {
      live = false;
    };
  }, [key, paneId, menu]);

  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const where = loc ? paneTitle(loc) : "this pane";
  const operate = host ? canOperate(host) : false;
  const hidden = lock.covered || lock.locked;

  const act = async (action: ChipAction) => {
    const current = scope.capture();
    if (s.settings.askBeforeRun) {
      // the same path as Run: the Confirm sheet, which builds the command from the action
      router.push({
        pathname: "/confirm",
        params: {
          key,
          pane: String(paneId),
          parent: "1",
          chip: action.kind,
          arg: action.kind === "cd" ? action.name : action.kind === "run" ? action.command : action.branch,
        },
      });
      return;
    }
    setSending(true);
    setError(null);
    // a cd or checkout finishes at once (a version switch too, logs follow forever): no "Notify me" for it
    const r = await fleet.run(key, paneId, chipCommand(action), { fromDraft: false, notify: false });
    setSending(false);
    if (!current()) return;
    if (r.ok) {
      AccessibilityInfo.announceForAccessibility(`Ran ${chipCommand(action)}`);
      if (scope.leave()) router.back();
    } else setError(r.message);
  };

  const detail = answer?.t === "chip" ? answer : null;
  const title =
    menu === "dirs"
      ? answer?.t === "dirs" && answer.cwd
        ? tildePath(answer.cwd)
        : "Folder"
      : menu === "branches"
        ? answer?.t === "branches" && answer.current
          ? answer.current
          : "Branch"
        : menu === "changes"
          ? "Changes"
          : detail?.kind === "pr"
            ? `PR #${detail.number} · ${detail.state}`
            : detail?.kind === "docker"
              ? dockerTitle(detail.project, detail.containers, detail.error)
              : menu === "pr"
                ? "Pull request"
                : menu === "python"
                  ? "Python"
                  : menu === "php"
                    ? "PHP"
                    : menu === "docker"
                      ? "Docker"
                      : "Time on this tab";
  const changes = answer?.t === "changes" ? answer : null;
  const added = changes?.files.reduce((n, f) => n + f.added, 0) ?? 0;
  const removed = changes?.files.reduce((n, f) => n + f.removed, 0) ?? 0;
  const subtitle =
    menu === "dirs"
      ? `Change folder · runs cd in ${where}`
      : menu === "branches"
        ? `Change branch · runs git checkout in ${where}`
        : menu === "python" || menu === "php"
          ? `Pick a version for this shell · runs rift-use in ${where}`
          : menu === "docker"
            ? `Tap a service to follow its logs · runs docker compose logs in ${where}`
            : menu === "pr"
              ? "Opens in your browser."
              : detail?.kind === "elapsed"
                ? elapsedBody(detail.line)
                : changes
                  ? changes.branch === null
                    ? "Not a git repository"
                    : null
                  : null;

  const reset = async () => {
    const current = scope.capture();
    setSending(true);
    setError(null);
    const r = await fleet.resetElapsed(key, paneId);
    setSending(false);
    if (!current()) return;
    if (r.ok) {
      AccessibilityInfo.announceForAccessibility("Time on this tab reset");
      if (scope.leave()) router.back();
    } else setError(r.message);
  };

  /** The menus of the pull request, interpreter, Docker and Time on tab chips (feature `prompt_chips`). */
  const detailList = (a: ChipMsg) => {
    if (a.kind === "pr") {
      return (
        <View style={styles.card}>
          <Row
            first
            icon={<Icon name="pullRequest" size={14} color={colors.textMuted} />}
            label="Open on GitHub"
            spoken={`Open pull request ${a.number} in the browser`}
            disabled={!isPrUrl(a.url)}
            onPress={() => void Linking.openURL(a.url)}
          />
        </View>
      );
    }
    if (a.kind === "python" || a.kind === "php") {
      const rows = versionRows(a.versions, "");
      if (rows.length === 0)
        return <Text style={styles.empty}>{a.kind === "php" ? "No PHP installs found (Homebrew, phpenv, asdf, mise)" : "No Python installs found (pyenv, Homebrew, asdf, mise)"}</Text>;
      return (
        <View style={styles.card}>
          {rows.map((r, i) => (
            <Row
              key={`${r.label}\n${r.source}`}
              first={i === 0}
              icon={<Icon name="code" size={14} color={colors.textMuted} />}
              label={r.label}
              detail={r.source}
              mark={r.current ? "✓ current" : undefined}
              spoken={r.current ? `${r.label}, current version` : `Switch to ${a.kind === "php" ? "PHP" : "Python"} ${r.label} from ${r.source}`}
              disabled={!operate || sending || r.current || r.command === null}
              onPress={() => r.command && void act({ kind: "run", command: r.command })}
            />
          ))}
        </View>
      );
    }
    if (a.kind === "docker") {
      if (a.error) return <Text style={styles.empty}>{a.error}</Text>;
      const rows = containerRows(a.containers);
      if (rows.length === 0) return <Text style={styles.empty}>No containers: docker compose up</Text>;
      return (
        <View style={styles.card}>
          {rows.map((r, i) => (
            <Row
              key={r.service}
              first={i === 0}
              icon={<Icon name="docker" size={14} color={r.running ? colors.accent : colors.textMuted} />}
              label={r.service}
              dimLabel={!r.running}
              detail={r.detail}
              health={r.health}
              spoken={`${r.service}, ${r.running ? "running" : "stopped"}, ${r.detail}${r.health ? `, ${r.health.word}` : ""}: follow its logs`}
              disabled={!operate || sending || r.command === null}
              onPress={() => r.command && void act({ kind: "run", command: r.command })}
            />
          ))}
        </View>
      );
    }
    return (
      <View style={styles.card}>
        <Row
          first
          icon={<Icon name="rerun" size={14} color={colors.error} />}
          label="Reset"
          danger
          spoken="Reset the time on this tab to zero"
          disabled={!operate || sending}
          onPress={() => void reset()}
        />
      </View>
    );
  };

  const list = () => {
    if (!answer) return null;
    if (answer.t === "chip") return detailList(answer);
    if (answer.t === "dirs") {
      const rows = dirRows(answer.parent, answer.dirs, query);
      if (rows.length === 0) return <Text style={styles.empty}>{query ? "No matches" : "No folders here"}</Text>;
      return (
        <View style={styles.card}>
          {rows.map((d, i) => (
            <Row
              key={d}
              first={i === 0}
              icon={d === ".." ? <Text style={styles.glyph}>↑</Text> : <Icon name="folder" size={14} color={colors.textMuted} />}
              label={d === ".." ? ".. (Parent folder)" : d}
              spoken={d === ".." ? "cd .. (parent folder)" : `cd into ${d}`}
              disabled={!operate || sending}
              onPress={() => void act({ kind: "cd", name: d })}
            />
          ))}
        </View>
      );
    }
    if (answer.t === "branches") {
      if (answer.current === null && answer.branches.length === 0) return <Text style={styles.empty}>Not a git repository</Text>;
      const rows = branchRows(answer.branches, query);
      if (rows.length === 0) return <Text style={styles.empty}>No matches</Text>;
      return (
        <View style={styles.card}>
          {rows.map((b, i) => {
            const current = !b.create && b.name === answer.current;
            return (
              <Row
                key={`${b.create ? "+" : ""}${b.name}`}
                first={i === 0}
                icon={
                  b.create ? <Text style={[styles.glyph, { color: colors.accent }]}>+</Text> : <Icon name="branch" size={14} color={colors.textMuted} />
                }
                label={b.create ? `Create branch “${b.name}”` : b.name}
                accent={b.create}
                mark={current ? "✓ current" : undefined}
                spoken={
                  b.create ? `Create branch ${b.name}: git checkout -b ${b.name}` : current ? `${b.name}, current branch` : `git checkout ${b.name}`
                }
                disabled={!operate || sending || current}
                onPress={() => void act(b.create ? { kind: "create", branch: b.name } : { kind: "checkout", branch: b.name })}
              />
            );
          })}
        </View>
      );
    }
    if (answer.branch === null) return null;
    if (answer.files.length === 0) return <Text style={styles.empty}>No changes — working tree clean</Text>;
    const group = (title: string, files: ChangedFile[]) =>
      files.length === 0 ? null : (
        <View style={styles.group} key={title}>
          <Text style={styles.groupTitle} accessibilityRole="header">
            {title} {files.length}
          </Text>
          <View style={styles.card}>
            {files.map((f, i) => {
              const { name, folder } = splitPath(f.path);
              const st = STATUS[f.status] ?? STATUS.M!;
              return (
                <Pressable
                  key={`${f.staged}:${f.path}`}
                  accessibilityRole="button"
                  accessibilityLabel={`${name}${folder ? ` in ${folder}` : ""}, ${st.word}, ${f.added} added, ${f.removed} removed${f.staged ? ", staged" : ""}: show diff`}
                  onPress={() =>
                    router.push({
                      pathname: "/diff",
                      params: {
                        key,
                        pane: String(paneId),
                        path: f.path,
                        staged: f.staged ? "1" : "0",
                        status: f.status,
                        added: String(f.added),
                        removed: String(f.removed),
                      },
                    })
                  }
                  style={({ pressed }) => [styles.file, i > 0 && styles.divider, pressed && styles.pressed]}
                >
                  <Text style={[styles.status, { color: st.color }]}>{f.status}</Text>
                  <View style={styles.fileText}>
                    <Text style={styles.fileName} numberOfLines={1}>
                      {name}
                    </Text>
                    {folder ? (
                      <Text style={styles.folder} numberOfLines={1}>
                        {folder}
                      </Text>
                    ) : null}
                  </View>
                  <Text style={styles.counts}>
                    <Text style={{ color: colors.success }}>+{f.added}</Text> <Text style={{ color: colors.textBright }}>−{f.removed}</Text>
                  </Text>
                  <Icon name="forward" size={14} color={colors.textMuted} />
                </Pressable>
              );
            })}
          </View>
        </View>
      );
    return (
      <>
        {group("Staged", answer.files.filter((f) => f.staged))}
        {group("Changes", answer.files.filter((f) => !f.staged))}
      </>
    );
  };

  return (
    // One scroll view for the whole sheet: a native sheet takes its first scroll view over as
    // the sheet's own, so the header must scroll with the list rather than sit above it.
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      contentContainerStyle={[styles.content, { paddingBottom: Math.max(36, insets.bottom + 2) }]}
      keyboardShouldPersistTaps="handled"
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <SheetClose onClose={() => { if (scope.leave()) router.back(); }} />
      <View style={styles.head}>
        <Text
          style={menu === "dirs" || menu === "branches" ? styles.monoTitle : styles.title}
          accessibilityRole="header"
          numberOfLines={1}
          ellipsizeMode={menu === "dirs" ? "head" : "tail"}
        >
          {title}
        </Text>
        {subtitle ? <Text style={styles.subtitle}>{subtitle}</Text> : null}
        {changes && changes.branch !== null ? (
          <Text style={styles.subtitle}>
            {changes.branch} · {changes.files.length} {changes.files.length === 1 ? "file" : "files"} ·{" "}
            <Text style={{ color: colors.success }}>+{added}</Text> <Text style={{ color: colors.textBright }}>−{removed}</Text>
          </Text>
        ) : null}
      </View>
      {(menu === "dirs" || menu === "branches") && answer ? (
        <TextInput
          value={query}
          onChangeText={setQuery}
          autoCapitalize="none"
          autoCorrect={false}
          spellCheck={false}
          placeholder={menu === "dirs" ? "Search folders" : "Search or name a new branch"}
          placeholderTextColor={colors.textMuted}
          accessibilityLabel={menu === "dirs" ? "Search folders" : "Search branches"}
          style={styles.search}
        />
      ) : null}
      {!operate && host && menu !== "changes" && menu !== "pr" ? <Text style={styles.note}>View only: this device cannot run commands here.</Text> : null}
      {error ? <ErrorText>{error}</ErrorText> : null}
      {!answer && !error ? (
        <View style={styles.loading} accessible accessibilityLabel="Loading">
          <ActivityIndicator color={colors.accent} />
        </View>
      ) : null}
      {list()}
      {answer && "more" in answer && answer.more ? <Text style={styles.note}>Some were left out: the list holds 500 at most.</Text> : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  content: { paddingTop: 24, paddingHorizontal: 12, gap: 14 },
  hidden: { opacity: 0 },
  head: { paddingHorizontal: 6, gap: 4 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  monoTitle: { fontFamily: monoMedium, color: colors.textBright, fontSize: 15 },
  subtitle: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  search: {
    fontFamily: mono,
    height: 40,
    paddingHorizontal: 10,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontSize: 14,
  },
  note: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, paddingHorizontal: 6 },
  loading: { paddingVertical: 24 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, paddingHorizontal: 6, paddingVertical: 12 },
  card: { backgroundColor: colors.bg, borderRadius: radius.card, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  row: { minHeight: 48, flexDirection: "row", alignItems: "center", gap: 14, paddingHorizontal: 14 },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  pressed: { opacity: 0.6 },
  disabled: { opacity: 0.45 },
  icon: { width: 20, alignItems: "center" },
  glyph: { fontFamily: mono, color: colors.textMuted, fontSize: 14 },
  label: { fontFamily: mono, flex: 1, color: colors.text, fontSize: 14 },
  accentLabel: { fontFamily: ui(400), color: colors.accent },
  dangerLabel: { fontFamily: ui(400), color: colors.error },
  dimLabel: { color: colors.textMuted },
  detail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, flexShrink: 1 },
  mark: { fontFamily: ui(400), color: colors.success, fontSize: 12 },
  group: { gap: 6 },
  groupTitle: {
    fontFamily: ui(500),
    color: colors.textMuted,
    fontSize: 12,
    letterSpacing: 0.5,
    textTransform: "uppercase",
    paddingHorizontal: 6,
  },
  file: { minHeight: 56, flexDirection: "row", alignItems: "center", gap: 12, paddingVertical: 8, paddingLeft: 14, paddingRight: 10 },
  status: { fontFamily: mono, width: 14, fontSize: 13 },
  fileText: { flex: 1, minWidth: 0, gap: 2 },
  fileName: { fontFamily: mono, color: colors.textBright, fontSize: 13 },
  folder: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  counts: { fontFamily: mono, fontSize: 12 },
});
