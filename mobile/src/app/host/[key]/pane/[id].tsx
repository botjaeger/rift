import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { Stack, useFocusEffect, useIsFocused, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AccessibilityInfo,
  Keyboard,
  KeyboardAvoidingView,
  PanResponder,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { askingTitle, questionKey } from "@/answer";
import { chipItems } from "@/promptChips";
import { canClose, canOperate, draftKey, hasFeature, isAsked, wrapsLines } from "@/fleet";
import { foldBlocks, paneName, programName, swipeTarget, type PaneItem } from "@/now";
import { CLAUDE_COLOR, paneMark, runsAgent, showsViewSwitch, terminalViewOn, waitingOthers } from "@/paneView";
import { canLoadOlder, MAX_BLOCKS } from "@/protocol/store";
import type { Block } from "@/protocol/types";
import { fleet, useApp } from "@/runtime";
import { screenProgram, showsTerminal } from "@/terminal";
import { findPane, paneTitle, runningCommand, tabTitle } from "@/topology";
import { AskCard } from "@/ui/AskCard";
import { FailedCard, FoldRow, RunningCard } from "@/ui/BlockCards";
import { BlockView, sideways } from "@/ui/BlockView";
import { Composer } from "@/ui/Composer";
import { Button } from "@/ui/Button";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { BlockSkeleton, PaneCard, PaneStrip } from "@/ui/PaneLoading";
import { openingTitle, paneLoad } from "@/ui/paneLoad";
import { headerTop } from "@/ui/Screen";
import { TerminalView } from "@/ui/TerminalView";
import { toggleA11y } from "@/ui/toggle";
import { useClose } from "@/ui/useClose";
import { useNow } from "@/ui/useNow";
import { useSingleFlight } from "@/ui/useSingleFlight";
import { colors, mono, radius, TOUCH, tints, ui } from "@/ui/theme";

/** Within this many points of the end counts as "at the bottom" (stick to new output). */
const BOTTOM_SLOP = 48;
/** The wall clock, for effects (reads of it in render are flagged as impure). */
const wallClock = () => Date.now();
/** A horizontal drag this long (and mostly sideways) switches to the next or previous pane. */
const SWIPE_PX = 60;

/** Blocks whose command or output holds `query` (any case); all of them for an empty query. */
function matching(blocks: Block[], query: string): Block[] {
  const q = query.trim().toLowerCase();
  if (q === "") return blocks;
  return blocks.filter((b) => b.command.toLowerCase().includes(q) || b.output.toLowerCase().includes(q));
}

const itemKey = (it: PaneItem) => (it.kind === "fold" ? `fold:${it.ids[0]}` : String(it.block.id));

/**
 * A pane (the Pane board): the tab and its panes as two buttons that open their sheets, search and
 * the ⋯ menu; a swipe between the tab's panes; its blocks as cards with the earlier successes
 * folded; an agent's question docked at the bottom (the PaneAsking board); or, while a
 * full-screen program runs (vim, htop, less: the alternate screen) or the Terminal view is on,
 * the pane's screen (the FullScreen board).
 */
export default function PaneScreen() {
  const { key, id, block: blockParam, restored, toast: toastParam } = useLocalSearchParams<{
    key: string;
    id: string;
    block?: string;
    restored?: string;
    toast?: string;
  }>();
  const paneId = Number(id);
  const s = useApp();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const list = useRef<FlashListRef<PaneItem>>(null);
  const atBottom = useRef(true);
  const [menu, setMenu] = useState(false);
  // a close that brought us here says so (useClose), then the toast goes as any other
  const [toast, setToast] = useState(toastParam ?? "");
  const toastTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    if (toastParam) toastTimer.current = setTimeout(() => setToast(""), 1600);
    return () => clearTimeout(toastTimer.current);
  }, [toastParam]);
  // a line at the top that goes after a moment (and is announced), as the board's toasts
  const say = useCallback((text: string) => {
    clearTimeout(toastTimer.current);
    setToast(text);
    AccessibilityInfo.announceForAccessibility(text);
    toastTimer.current = setTimeout(() => setToast(""), 1600);
  }, []);
  // opened at a block (Now › Finished): unfolded, so the block is in the list
  const [unfolded, setUnfolded] = useState(() => blockParam !== undefined);
  const [search, setSearch] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // this screen, as the pane's holder (the same pane may be pushed twice)
  const [holder] = useState(() => ({}));
  // the keyboard covers the bottom inset: no gap between it and the command line
  const [keyboard, setKeyboard] = useState(false);
  useEffect(() => {
    const ios = Platform.OS === "ios";
    const show = Keyboard.addListener(ios ? "keyboardWillShow" : "keyboardDidShow", () => setKeyboard(true));
    const hide = Keyboard.addListener(ios ? "keyboardWillHide" : "keyboardDidHide", () => setKeyboard(false));
    // the list shrinks as the keyboard comes up: keep its end in sight once it has (the screen's
    // own ScrollView follows its end itself)
    const settled = Keyboard.addListener("keyboardDidShow", () => {
      if (atBottom.current) list.current?.scrollToEnd({ animated: false });
    });
    return () => {
      show.remove();
      hide.remove();
      settled.remove();
    };
  }, []);

  const host = s.hosts.find((h) => h.record.key === key);
  const status = host?.conn.status;
  const loc = useMemo(() => (host ? findPane(host.windows, paneId) : null), [host, paneId]);
  // the header's name: the tab's title, else the one last seen for this pane (never a placeholder)
  const cachedTitle = s.paneTitles[draftKey(key, paneId)] ?? null;
  const title = loc ? paneTitle(loc) : (cachedTitle ?? host?.record.name ?? "this pane");
  // the computer's tabs no longer hold it (none at all, too): nothing left to show or type into
  const closed = !!host && status === "open" && host.topology && !loc;

  // A cold start's pane that has closed meanwhile: back to Portals rather than a dead page. Only
  // once it is on top: dismissing to Portals would also close a sheet over it (What's New).
  const focused = useIsFocused();
  useEffect(() => {
    if (closed && restored === "1" && focused) router.dismissTo("/");
  }, [closed, restored, focused, router]);

  // Subscribe once connected and whenever this screen regains focus (a no-op when already
  // followed): a pane pushed on top (a notification tap) follows its own pane, and closes it
  // when it goes back, so this one must follow again.
  useFocusEffect(
    useCallback(() => {
      // a closed pane is not followed: the gateway refuses it, and its screen requests with it
      if (Number.isSafeInteger(paneId) && status === "open" && !closed) fleet.openPane(key, paneId, holder);
    }, [key, paneId, status, holder, closed]),
  );
  useEffect(() => () => fleet.closePane(key, paneId, holder), [key, paneId, holder]);

  // Notifications skip the pane on screen; a cold start comes back to it.
  useFocusEffect(
    useCallback(() => {
      fleet.setViewing({ host: key, pane: paneId });
      if (Number.isSafeInteger(paneId) && fleet.state.hosts.some((h) => h.record.key === key))
        void fleet.setLastView({ host: key, pane: paneId });
      // the pager's replace focuses the next pane's screen before this one goes
      return () => fleet.leaveViewing(key, paneId);
    }, [key, paneId]),
  );

  const pane = s.pane?.host === key && s.pane.paneId === paneId ? s.pane : null;
  const state = pane?.state ?? null;
  const blocks = useMemo(() => state?.blocks ?? [], [state]);
  const running = runningCommand(loc?.pane ?? null, state);

  // The view follows the pane: its screen while a full-screen program runs, blocks otherwise.
  const screenFeature = hasFeature(host, "screen");
  const screen = pane?.screen ?? null;
  // the header's Terminal view: the pane's screen whatever runs (gateway mode `on`). On by default
  // for an agent (Claude Code never uses the alternate screen), off at a shell; a tap overrides
  // the default while this screen is open.
  const [termChoice, setTermChoice] = useState<boolean | null>(null);
  const agent = !!loc && runsAgent(loc.pane);
  const termView = terminalViewOn(termChoice, agent);
  const pinned = termView && screenFeature;
  const fullScreen = pinned || showsTerminal(screen, screenFeature);
  const program = (loc && programName(loc.pane)) || screenProgram(running, loc?.pane.job ?? null);
  const following = pane !== null;
  useEffect(() => {
    if (screenFeature && status === "open" && following && !closed) fleet.setScreenMode(key, paneId, pinned ? "on" : "auto");
  }, [screenFeature, status, following, closed, key, paneId, pinned]);
  // the search belongs to the blocks: a full-screen program closes it
  const searching = fullScreen ? null : search;

  // While the pane opens: the status strip, the steps card, the blocks held in memory (dimmed)
  // the wait starts when the page opens, when Try again is pressed, and when the view switches
  // between blocks and the screen (that one has its own thing to wait for)
  const [waitingSince, setWaitingSince] = useState(() => Date.now());
  const shownScreen = useRef(fullScreen);
  useEffect(() => {
    if (shownScreen.current === fullScreen) return;
    shownScreen.current = fullScreen;
    setWaitingSince(wallClock());
  }, [fullScreen]);
  const loaded = fullScreen ? screen !== null : state !== null;
  const waitingForPane = !closed && !(status === "open" && loaded);
  const now = useNow(1000, waitingForPane && focused);
  const hostName = host?.record.name ?? "this computer";
  const load = host && !closed ? paneLoad({
    conn: host.conn,
    now,
    waitingSince,
    hostName,
    view: fullScreen ? "screen" : "blocks",
    opening: openingTitle(loc ? tabTitle(loc.tab) : cachedTitle, agent && program ? program : null),
    paneKnown: !!loc,
    loaded,
    heldSince: loaded ? (host.lastSeen ?? waitingSince) : null,
  }) : null;
  const details = () => router.push({ pathname: "/connection", params: { key } });
  const tryAgain = () => {
    setWaitingSince(Date.now());
    if (status === "open") fleet.refreshPane(key, paneId);
    else fleet.retry(key);
  };

  const wrap = wrapsLines(s, key, paneId);
  const operate = host ? canOperate(host) : false;
  const closeable = canClose(host);
  const onClose = useClose(key, paneId, setError, say);
  const onActions = useCallback(
    (blockId: number) => router.push({ pathname: "/block", params: { key, pane: String(paneId), block: String(blockId) } }),
    [router, key, paneId],
  );
  const onBell = useCallback((b: Block, on: boolean) => fleet.setNotify(key, paneId, b.command, on), [key, paneId]);
  const onStop = useCallback(() => {
    setError(null);
    void fleet.sendKeys(key, paneId, ["ctrl+c"]).then((r) => {
      if (r.ok) say("⌃C sent");
      else setError(r.message);
    });
  }, [key, paneId, say]);
  // a Run again on its way: a second tap (a double tap) must not run the command twice
  const once = useSingleFlight();
  const onRunAgain = useCallback(
    (b: Block) => {
      if (!operate) return setError("View only: this device cannot run commands here.");
      setError(null);
      if (fleet.state.settings.askBeforeRun) {
        // the same path as Run: the Confirm sheet, with this block's command instead of the draft
        router.push({ pathname: "/confirm", params: { key, pane: String(paneId), block: String(b.id) } });
        return;
      }
      void once(async () => {
        const r = await fleet.run(key, paneId, b.command, { fromDraft: false });
        if (r.ok) say(`Sent to ${loc ? tabTitle(loc.tab) : "the pane"}`);
        else setError(r.message);
      });
    },
    [router, key, paneId, operate, once, say, loc],
  );
  const asked = s.asked;
  const extraData = useMemo(() => ({ wrap, asked }), [wrap, asked]);
  const items = useMemo(
    () => (searching === null ? foldBlocks(blocks, unfolded) : foldBlocks(matching(blocks, searching), true)),
    [blocks, searching, unfolded],
  );
  const shownBlocks = items.filter((i) => i.kind === "block").length;

  // A Finished row of Now opens the pane at its block: unfold it, then scroll it into view.
  const target = blockParam !== undefined ? Number(blockParam) : null;
  const reached = useRef<number | null>(null);
  useEffect(() => {
    if (target === null || reached.current === target || !state) return;
    const index = items.findIndex((i) => i.kind === "block" && i.block.id === target);
    if (index === -1) return;
    reached.current = target;
    atBottom.current = false;
    // once now, and again after the rows above it measured themselves
    for (const ms of [50, 400])
      setTimeout(() => list.current?.scrollToIndex({ index, viewPosition: 0.2, animated: false }), ms);
  }, [target, items, state]);

  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    atBottom.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - BOTTOM_SLOP;
  }, []);

  const onContentSizeChange = useCallback(() => {
    if (atBottom.current) list.current?.scrollToEnd({ animated: false });
  }, []);

  // the tab's panes, and a swipe across the blocks between them
  const tab = loc?.tab ?? null;
  // another pane takes this one's place: Back still leads where it led
  const goPane = useCallback(
    (next: number | null) => next !== null && router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(next) } }),
    [router, key],
  );
  const swipe = useMemo(
    () =>
      PanResponder.create({
        // not while a sideways-scrolling block (Wrap long lines off) has the touch: that drag scrolls it
        onMoveShouldSetPanResponder: (_, g) => !sideways.touching && Math.abs(g.dx) > 24 && Math.abs(g.dx) > 2 * Math.abs(g.dy),
        onPanResponderRelease: (_, g) => {
          if (!tab || Math.abs(g.dx) < SWIPE_PX) return;
          goPane(swipeTarget(tab, paneId, g.dx < 0 ? 1 : -1));
        },
      }),
    [tab, paneId, goPane],
  );
  // a screen reader swipes through elements, so the swipe is also two actions on the pane button
  const pagerActions = tab
    ? [
        ...(swipeTarget(tab, paneId, 1) !== null ? [{ name: "next", label: "Next pane" }] : []),
        ...(swipeTarget(tab, paneId, -1) !== null ? [{ name: "previous", label: "Previous pane" }] : []),
      ]
    : [];

  const recent = useMemo(() => {
    const out: string[] = [];
    for (let i = blocks.length - 1; i >= 0 && out.length < 6; i--) {
      const c = blocks[i]!.command.trim();
      if (c && !out.includes(c)) out.push(c);
    }
    return out;
  }, [blocks]);

  // With no room left, older blocks would be trimmed straight back off, so none are fetched.
  const header =
    searching !== null ? null : canLoadOlder(state) ? (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={pane?.loadingOlder ? "Loading earlier blocks" : "Earlier blocks"}
        accessibilityHint="Loads older commands of this pane"
        accessibilityState={{ disabled: !!pane?.loadingOlder, busy: !!pane?.loadingOlder }}
        disabled={pane?.loadingOlder}
        onPress={() => fleet.loadOlder()}
        hitSlop={{ top: 4, bottom: 4 }}
        style={({ pressed }) => [styles.older, pressed && styles.pressed]}
      >
        <Text style={styles.olderText}>{pane?.loadingOlder ? "Loading earlier blocks…" : "Earlier blocks"}</Text>
      </Pressable>
    ) : state?.hasOlder ? (
      <Text style={styles.capped}>Showing the most recent {Math.min(state.blocks.length, MAX_BLOCKS)} blocks</Text>
    ) : null;

  // the back badge's count: this computer's other panes waiting for an answer
  const waitingElsewhere =
    host?.windows.reduce((n, w) => n + w.tabs.reduce((m, t) => m + t.panes.filter((p) => p.waiting && p.id !== paneId).length, 0), 0) ?? 0;
  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));
  const lastBlock = blocks.at(-1);
  const notifOn = host?.record.notifications !== false;
  const headerPad = headerTop(insets.top) + 2;
  const tabNumber = loc ? loc.window.tabs.indexOf(loc.tab) + 1 : 0;
  const tabCount = loc ? loc.window.tabs.length : 0;
  const paneNumber = loc ? loc.paneIndex + 1 : 0;
  const others = tab ? waitingOthers(tab, paneId) : 0;
  const mark = loc ? paneMark(loc.pane) : null;
  const pickName = loc ? paneName(loc.pane, loc.paneIndex) : "";
  const waiting = loc?.pane.waiting;
  // an agent's menu is answered on the pane, from a card at the bottom
  const asking = !!waiting && waiting.kind === "choice" && waiting.choices.length > 0 && status === "open" && operate && !closed;
  // the Terminal view at a prompt, or a question to answer: the screen without its own keys, the
  // pane's command line (or the question) under it
  const bareScreen = fullScreen && (asking || (pinned && !screen?.alt));
  const bottomInset = keyboard ? 0 : Math.max(0, insets.bottom - 12);
  const composer =
    closed || !host ? null : operate ? (
      // the board's 30 pt under the command line: the home indicator's inset, less the
      // command line's own 8 pt
      <View style={{ paddingBottom: bottomInset, backgroundColor: colors.chrome }}>
        <Composer
          hostKey={key}
          paneId={paneId}
          running={running}
          program={program}
          online={status === "open"}
          chips={chipItems(loc?.pane ?? null, state, {
            git: hasFeature(host, "git"),
            menus: hasFeature(host, "chips"),
            promptChips: hasFeature(host, "prompt_chips"),
          })}
          complete={hasFeature(host, "complete") && operate}
          recent={recent}
          label={title}
          canType={hasFeature(host, "text")}
          waiting={status !== "open"}
        />
      </View>
    ) : (
      <Text style={[styles.viewOnly, { paddingBottom: insets.bottom + 10 }]}>View only: this device cannot run commands here.</Text>
    );
  const footer =
    asking && waiting ? (
      <AskCard key={questionKey(waiting)} hostKey={key} paneId={paneId} waiting={waiting} program={program} bottomInset={bottomInset} onSay={say} />
    ) : (
      composer
    );

  return (
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      <Stack.Screen options={{ headerShown: false }} />
      <View style={[styles.header, { paddingTop: headerPad }]}>
        <View style={styles.headerRow}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={"Back from " + hostName + (waitingElsewhere ? ", " + waitingElsewhere + " " + (waitingElsewhere === 1 ? "needs" : "need") + " you" : "")}
            onPress={back}
            style={({ pressed }) => [styles.glass, pressed && styles.pressed]}
          >
            <Icon name="back" size={20} color={colors.text} />
            {waitingElsewhere ? (
              <View style={styles.badge} importantForAccessibility="no">
                <Text style={styles.badgeText}>{waitingElsewhere > 99 ? "99+" : waitingElsewhere}</Text>
              </View>
            ) : null}
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={loc ? tabTitle(loc.tab) + ", tab " + tabNumber + " of " + tabCount + " on " + hostName + ". Switch tab" : (cachedTitle ?? hostName)}
            disabled={!loc || closed}
            onPress={() => router.push({ pathname: "/tabs", params: { key, pane: String(paneId) } })}
            style={({ pressed }) => [styles.titleButton, pressed && styles.pressed]}
          >
            <View style={styles.titleLine}>
              <Text style={styles.title} numberOfLines={1}>
                {loc ? tabTitle(loc.tab) : (cachedTitle ?? hostName)}
              </Text>
              {loc && !closed ? <Icon name="chevronDown" size={16} color={colors.textMuted} /> : null}
            </View>
            {host && (loc || cachedTitle) ? (
              <Text style={styles.subtitle} numberOfLines={1}>
                {loc ? hostName + " · tab " + tabNumber + " of " + tabCount : hostName}
              </Text>
            ) : null}
          </Pressable>
          {!host || closed ? null : (
            <View style={styles.actions}>
              {fullScreen ? null : (
                <Pressable
                  {...toggleA11y(searching !== null)}
                  accessibilityLabel="Search output"
                  onPress={() => setSearch(searching === null ? "" : null)}
                  style={({ pressed }) => [styles.action, pressed && styles.pressed]}
                >
                  <Icon name="search" size={19} color={searching !== null ? colors.accent : colors.text} />
                </Pressable>
              )}
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="More"
                accessibilityState={{ expanded: menu }}
                onPress={() => setMenu((m) => !m)}
                style={({ pressed }) => [styles.action, pressed && styles.pressed]}
              >
                <Icon name="more" size={19} color={colors.text} />
              </Pressable>
            </View>
          )}
        </View>
        {loc && tab && !closed ? (
          <View style={styles.pickRow}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={
                "Pane " + paneNumber + " of " + tab.panes.length + ": " + pickName + (others ? ", " + others + " other" + (others === 1 ? "" : "s") + " need you" : "") + ". Switch pane"
              }
              accessibilityActions={pagerActions}
              onAccessibilityAction={(e) => goPane(swipeTarget(tab, paneId, e.nativeEvent.actionName === "next" ? 1 : -1))}
              hitSlop={{ top: 4, bottom: 4 }}
              onPress={() => router.push({ pathname: "/panes", params: { key, pane: String(paneId) } })}
              style={({ pressed }) => [styles.picker, pressed && styles.pressed]}
            >
              <Text style={[styles.pickMark, { color: mark?.claude ? CLAUDE_COLOR : colors.accent }]}>{mark?.mark}</Text>
              <Text style={styles.pickName} numberOfLines={1}>
                {pickName}
              </Text>
              {/* textMuted: the mockup's textDim is 4.3:1 on the picker's glass (palette.ts) */}
              <Text style={styles.pickPos}>
                · {paneNumber}/{tab.panes.length}
              </Text>
              <View style={styles.flex} />
              {others > 0 ? (
                <View style={styles.pickWait}>
                  <View style={styles.pickDot} />
                  <Text style={styles.pickWaitText}>{others}</Text>
                </View>
              ) : null}
              <Icon name="chevronDown" size={14} color={colors.textMuted} />
            </Pressable>
            {showsViewSwitch(screenFeature, agent) ? (
              <View style={styles.segments} accessibilityRole="tablist" accessibilityLabel="View">
                {(["Blocks", "Terminal"] as const).map((name) => {
                  const on = (name === "Terminal") === pinned;
                  return (
                    <Pressable
                      key={name}
                      accessibilityRole="tab"
                      accessibilityLabel={name}
                      accessibilityHint={name === "Blocks" ? "Shows the pane's commands as blocks" : "Shows the pane's screen as the program draws it"}
                      accessibilityState={{ selected: on }}
                      hitSlop={{ top: 4, bottom: 4 }}
                      onPress={() => setTermChoice(name === "Terminal")}
                      style={({ pressed }) => [styles.segment, on && styles.segmentOn, pressed && styles.pressed]}
                    >
                      <Text style={[styles.segmentText, on && styles.segmentTextOn]}>{name}</Text>
                    </Pressable>
                  );
                })}
              </View>
            ) : null}
          </View>
        ) : null}
      </View>
      {load?.strip ? <PaneStrip strip={load.strip} onPress={details} /> : null}
      {waiting && status === "open" && !asking ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={askingTitle(waiting, program) + ": " + waiting.prompt}
          accessibilityHint="Opens the question"
          onPress={() => router.push({ pathname: "/answer", params: { key, pane: String(paneId), from: "pane" } })}
          style={({ pressed }) => [styles.asking, pressed && styles.pressed]}
        >
          <View style={styles.askingDot} />
          <Text style={styles.askingText} numberOfLines={1}>
            {askingTitle(waiting, program)}: {waiting.prompt}
          </Text>
          <Text style={styles.askingLink}>Answer</Text>
        </Pressable>
      ) : null}
      {host?.notice ? <ErrorText style={styles.notice}>{host.notice}</ErrorText> : null}
      {error ? <ErrorText style={styles.notice}>{error}</ErrorText> : null}
      {host && fullScreen && !closed ? (
        <>
          <View style={[styles.fill, { paddingBottom: bareScreen ? 0 : bottomInset }, load?.held && styles.held]} {...swipe.panHandlers}>
            <TerminalView
              hostKey={key}
              paneId={paneId}
              screen={screen}
              operate={operate}
              canType={hasFeature(host, "text")}
              online={status === "open"}
              running={running !== null}
              bare={bareScreen}
              canFit={hasFeature(host, "fit")}
            />
            {load?.card ? <PaneCard card={load.card} onRetry={tryAgain} onDetails={details} /> : null}
          </View>
          {bareScreen ? footer : null}
        </>
      ) : (
        <>
          {searching !== null ? (
            <View style={styles.search}>
              <TextInput
                value={searching}
                onChangeText={setSearch}
                autoFocus
                autoCapitalize="none"
                autoCorrect={false}
                placeholder="Search commands and output"
                placeholderTextColor={colors.textMuted}
                returnKeyType="search"
                accessibilityLabel="Search output"
                style={styles.searchInput}
              />
              <Text style={styles.searchCount} accessibilityLiveRegion="polite">
                {searching.trim() === "" ? "" : shownBlocks + " of " + blocks.length}
              </Text>
            </View>
          ) : null}
          {!host ? (
            <Text style={styles.empty}>This computer is not paired.</Text>
          ) : closed ? (
            <View style={styles.closed}>
              <Text style={styles.closedText} accessibilityLiveRegion="polite">
                This pane has closed.
              </Text>
              <Button label="Go to Portals" onPress={() => router.dismissTo("/")} />
            </View>
          ) : !state ? null : blocks.length === 0 ? (
            <Text style={styles.empty}>No commands in this pane yet.</Text>
          ) : searching !== null && searching.trim() !== "" && shownBlocks === 0 ? (
            <Text style={styles.empty}>No block holds “{searching.trim()}”.</Text>
          ) : null}
          <View style={[styles.fill, load?.held && styles.held]} {...swipe.panHandlers}>
            {!state && host && !closed ? <BlockSkeleton /> : null}
            {/* Not inverted: oldest first, so the reading order (and swiping through with a
                screen reader) ends at the newest block, which the list starts scrolled to. */}
            <FlashList
              ref={list}
              data={closed ? [] : items}
              keyExtractor={itemKey}
              getItemType={(it) => (it.kind === "fold" ? "fold" : it.block.state === "running" ? "running" : it.block.exit ? "failed" : "block")}
              extraData={extraData}
              renderItem={({ item }) =>
                item.kind === "fold" ? (
                  <View style={styles.cardGap}>
                    <FoldRow count={item.count} onShow={() => setUnfolded(true)} />
                  </View>
                ) : item.block.state === "running" && item.block === lastBlock ? (
                  <View style={styles.cardGap}>
                    <RunningCard
                      block={item.block}
                      onActions={onActions}
                      notify={isAsked(s, key, paneId, item.block.command)}
                      onBell={onBell}
                      onStop={operate ? onStop : undefined}
                    />
                  </View>
                ) : item.block.state === "done" && item.block.exit !== null && item.block.exit !== 0 ? (
                  <View style={styles.cardGap}>
                    {/* keyed: a recycled card must not keep another block's state */}
                    <FailedCard key={item.block.id} block={item.block} onActions={onActions} onRunAgain={operate ? onRunAgain : undefined} />
                  </View>
                ) : (
                  <View style={styles.cardGap}>
                    <BlockView key={item.block.id} block={item.block} onActions={onActions} wrap={wrap} />
                  </View>
                )
              }
              ListHeaderComponent={closed ? null : header}
              // opened at a block: no sticking to the bottom, which would scroll past it
              maintainVisibleContentPosition={
                target === null
                  ? { startRenderingFromBottom: true, autoscrollToBottomThreshold: 0.1, animateAutoScrollToBottom: false }
                  : {}
              }
              onScroll={onScroll}
              scrollEventThrottle={100}
              onContentSizeChange={onContentSizeChange}
              contentContainerStyle={styles.listContent}
              accessibilityLabel="Command blocks, newest at the bottom"
            />
            {load?.card ? <PaneCard card={load.card} onRetry={tryAgain} onDetails={details} /> : null}
          </View>
          {footer}
        </>
      )}
      {menu ? (
        <>
          <Pressable accessibilityRole="button" accessibilityLabel="Close menu" onPress={() => setMenu(false)} style={StyleSheet.absoluteFill} />
          <View accessibilityRole="menu" accessibilityLabel="Pane options" style={[styles.menu, { top: headerPad + 50 }]}>
            {fullScreen ? (
              // wrapping is the blocks'; the screen is the program's own
              hasFeature(host, "fit") ? (
                <Text style={[styles.menuNote, styles.menuRule]}>The pane takes this phone's width here, so programs wrap their own lines.</Text>
              ) : null
            ) : (
              <MenuSwitch
                icon="wrapOn"
                title="Wrap long lines"
                hint="Off: wide output scrolls sideways"
                on={wrap}
                onPress={() => {
                  fleet.setPaneWrap(key, paneId, !wrap);
                  say(wrap ? "Long lines scroll sideways" : "Wrapping long lines");
                }}
              />
            )}
            <MenuSwitch
              icon="bell"
              title="Notifications"
              hint="Finished, failed and questions here"
              on={notifOn}
              last={!closeable}
              onPress={() => {
                void fleet.setHostNotifications(key, !notifOn);
                say(notifOn ? "Notifications off for this computer" : "Notifications on for this computer");
              }}
            />
            {closeable ? (
              <>
                <MenuAction title="Close pane" onPress={() => { setMenu(false); onClose(paneId, "pane"); }} />
                <MenuAction title="Close tab" last onPress={() => { setMenu(false); onClose(paneId, "tab"); }} />
              </>
            ) : null}
          </View>
        </>
      ) : null}
      {toast ? (
        <View pointerEvents="none" style={[styles.toastWrap, { top: headerPad + 94 }]}>
          <Text style={styles.toast} accessibilityLiveRegion="polite">
            {toast}
          </Text>
        </View>
      ) : null}
    </KeyboardAvoidingView>
  );
}

/** One of the ⋯ menu's actions: a destructive word. */
function MenuAction({ title, last, onPress }: { title: string; last?: boolean; onPress: () => void }) {
  return (
    <Pressable
      accessibilityRole="menuitem"
      accessibilityLabel={title}
      onPress={onPress}
      style={({ pressed }) => [styles.menuAction, !last && styles.menuRule, pressed && styles.pressed]}
    >
      <Text style={styles.menuDestructive}>{title}</Text>
    </Pressable>
  );
}

/** One of the ⋯ menu's switches: an icon, the label and what it does, the 40 x 24 switch. */
function MenuSwitch({
  icon,
  title,
  hint,
  on,
  last,
  onPress,
}: {
  icon: "wrapOn" | "bell";
  title: string;
  hint: string;
  on: boolean;
  last?: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="switch"
      accessibilityLabel={title}
      accessibilityHint={hint}
      accessibilityState={{ checked: on }}
      onPress={onPress}
      style={({ pressed }) => [styles.menuRow, !last && styles.menuRule, pressed && styles.pressed]}
    >
      <Icon name={icon} size={20} color={colors.text} />
      <View style={styles.menuText}>
        <Text style={styles.menuTitle}>{title}</Text>
        <Text style={styles.menuHint}>{hint}</Text>
      </View>
      <View style={[styles.track, { backgroundColor: on ? colors.accent : colors.borderStrong }]}>
        <View style={[styles.knob, { left: on ? 18 : 2 }]} />
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  flex: { flex: 1 },
  pressed: { opacity: 0.7 },
  // the Pane board's header: chrome, 12 pt gutter, 10 pt between its two rows, a hairline under
  header: { backgroundColor: colors.chrome, paddingHorizontal: 12, paddingBottom: 10, gap: 10, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  headerRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  glass: {
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
    alignItems: "center",
    justifyContent: "center",
  },
  badge: {
    position: "absolute",
    top: -3,
    right: -3,
    minWidth: 18,
    height: 18,
    paddingHorizontal: 4,
    borderRadius: 9,
    backgroundColor: colors.warning,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: { fontFamily: ui(600), color: colors.bg, fontSize: 11 },
  titleButton: { flex: 1, minWidth: 0, minHeight: 44, justifyContent: "center", paddingHorizontal: 6 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 4 },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  subtitle: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  actions: { height: 44, flexDirection: "row", borderRadius: 22, backgroundColor: colors.glass, borderWidth: 1, borderColor: colors.glassRim },
  action: { width: 44, height: 42, alignItems: "center", justifyContent: "center" },
  pickRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  picker: {
    flex: 1,
    minWidth: 0,
    height: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingLeft: 12,
    paddingRight: 10,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: colors.glassRim,
    backgroundColor: "#ffffff0f",
  },
  pickMark: { fontFamily: mono, fontSize: 13 },
  pickName: { flexShrink: 1, fontFamily: ui(500), color: colors.textBright, fontSize: 13 },
  pickPos: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  pickWait: { flexDirection: "row", alignItems: "center", gap: 5 },
  pickDot: { width: 7, height: 7, borderRadius: 4, backgroundColor: colors.warning },
  pickWaitText: { fontFamily: ui(400), color: colors.warning, fontSize: 12 },
  // the Blocks | Terminal switch: two segments in a 36 pt capsule, the chosen one on the accent tint
  segments: { height: 36, flexDirection: "row", padding: 2, borderRadius: 18, borderWidth: 1, borderColor: colors.glassRim },
  segment: { minWidth: 68, paddingHorizontal: 10, borderRadius: 16, alignItems: "center", justifyContent: "center" },
  segmentOn: { backgroundColor: tints.accentBadge },
  segmentText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  segmentTextOn: { color: colors.accent },
  // blocks held in memory while the pane reconnects (PaneLoading board)
  held: { opacity: 0.55 },
  // the ⋯ menu: 280 wide under the button, 14 pt radius, #2c2c30 with a soft shadow
  menu: {
    position: "absolute",
    right: 12,
    width: 280,
    borderRadius: 14,
    backgroundColor: "#2b2b2b",
    borderWidth: 1,
    borderColor: colors.glassRim,
    overflow: "hidden",
    shadowColor: "#000000",
    shadowOpacity: 0.6,
    shadowRadius: 20,
    shadowOffset: { width: 0, height: 16 },
    elevation: 16,
  },
  menuRow: { minHeight: 56, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 14 },
  menuRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  menuNote: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17, paddingHorizontal: 14, paddingVertical: 12 },
  menuAction: { minHeight: 48, justifyContent: "center", paddingHorizontal: 14 },
  menuDestructive: { fontFamily: ui(400), color: colors.warning, fontSize: 15 },
  menuText: { flex: 1, gap: 2, paddingVertical: 8 },
  menuTitle: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  menuHint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  track: { width: 40, height: 24, borderRadius: 12 },
  knob: { position: "absolute", top: 2, width: 20, height: 20, borderRadius: 10, backgroundColor: colors.textBright },
  toastWrap: { position: "absolute", left: 0, right: 0, alignItems: "center" },
  toast: {
    fontFamily: ui(500),
    color: colors.bg,
    fontSize: 14,
    paddingVertical: 10,
    paddingHorizontal: 16,
    borderRadius: 20,
    overflow: "hidden",
    backgroundColor: colors.textBright,
  },
  cardGap: { paddingHorizontal: 12, paddingVertical: 4 },
  // a question the card does not answer (a password, input, a view-only device): one tap to the answer page
  asking: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 44,
    paddingHorizontal: 16,
    backgroundColor: colors.surface,
    borderBottomWidth: 1,
    borderBottomColor: `${colors.warning}55`,
  },
  askingDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.warning },
  askingText: { flex: 1, fontFamily: ui(400), color: colors.textBright, fontSize: 13 },
  askingLink: { fontFamily: ui(400), color: colors.accent, fontSize: 13 },
  listContent: { paddingTop: 6, paddingBottom: 4 },
  older: {
    alignSelf: "center",
    minHeight: 36,
    marginTop: 8,
    marginBottom: 6,
    paddingHorizontal: 12,
    justifyContent: "center",
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  olderText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  capped: { fontFamily: ui(400), color: colors.textDim, fontSize: 12, textAlign: "center", padding: 12 },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, padding: 16 },
  closed: { padding: 16, gap: 12, alignItems: "flex-start" },
  closedText: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  notice: { paddingHorizontal: 16, paddingVertical: 6 },
  search: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    backgroundColor: colors.chrome,
    borderBottomWidth: 1,
    borderBottomColor: colors.hairline,
  },
  searchInput: {
    flex: 1,
    height: TOUCH - 4,
    paddingHorizontal: 10,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontFamily: mono,
    fontSize: 13,
  },
  searchCount: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, minWidth: 48, textAlign: "right" },
  viewOnly: {
    fontFamily: ui(400),
    color: colors.textDim,
    fontSize: 13,
    textAlign: "center",
    paddingTop: 10,
    backgroundColor: colors.chrome,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
});
