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
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { askingTitle } from "@/answer";
import { promptChips } from "@/chips";
import { canOperate, hasFeature, isAsked, wrapsLines } from "@/fleet";
import { foldBlocks, pagerModel, programName, swipeTarget, tabPane, tabState, type PaneItem } from "@/now";
import { canLoadOlder, MAX_BLOCKS } from "@/protocol/store";
import type { Block } from "@/protocol/types";
import { fleet, useApp } from "@/runtime";
import { screenProgram, showsTerminal } from "@/terminal";
import { findPane, paneTitle, runningCommand, tabTitle } from "@/topology";
import { FailedCard, FoldRow, RunningCard } from "@/ui/BlockCards";
import { BlockView, sideways } from "@/ui/BlockView";
import { Composer } from "@/ui/Composer";
import { describeConn } from "@/ui/connLabel";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { GlassGroup, headerTop, IconButton } from "@/ui/Screen";
import { TerminalView } from "@/ui/TerminalView";
import { useSingleFlight } from "@/ui/useSingleFlight";
import { colors, mono, radius, TOUCH, ui } from "@/ui/theme";

/** Within this many points of the end counts as "at the bottom" (stick to new output). */
const BOTTOM_SLOP = 48;
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
 * A pane (the P3 Pane board): the window's tabs as a strip, a swipe between the tab's panes,
 * its blocks with the earlier successes folded and the failed and running commands as cards;
 * or, while a full-screen program runs (vim, htop, less: the alternate screen), that program's
 * screen (the FullScreen board) until it quits.
 */
export default function PaneScreen() {
  const { key, id, block: blockParam, restored } = useLocalSearchParams<{ key: string; id: string; block?: string; restored?: string }>();
  const paneId = Number(id);
  const s = useApp();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const list = useRef<FlashListRef<PaneItem>>(null);
  const atBottom = useRef(true);
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set());
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
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);

  const host = s.hosts.find((h) => h.record.key === key);
  const status = host?.conn.status;
  const loc = useMemo(() => (host ? findPane(host.windows, paneId) : null), [host, paneId]);
  const title = loc ? paneTitle(loc) : "Pane";
  // the computer's tabs no longer hold it: nothing left to show or type into
  const closed = !!host && status === "open" && host.windows.length > 0 && !loc;

  // A cold start's pane that has closed meanwhile: back to Now rather than a dead page. Only
  // once it is on top: dismissing to Now would also close a sheet over it (What's New).
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
  // the header's Terminal view: the pane's screen whatever runs (gateway mode `on`)
  const [termView, setTermView] = useState(false);
  const pinned = termView && screenFeature;
  const fullScreen = pinned || showsTerminal(screen, screenFeature);
  const program = (loc && programName(loc.pane)) || screenProgram(running, loc?.pane.job ?? null);
  const following = pane !== null;
  useEffect(() => {
    if (screenFeature && status === "open" && following && !closed) fleet.setScreenMode(key, paneId, pinned ? "on" : "auto");
  }, [screenFeature, status, following, closed, key, paneId, pinned]);
  // the search belongs to the blocks: a full-screen program closes it
  const searching = fullScreen ? null : search;

  const wrap = wrapsLines(s, key, paneId);
  const operate = host ? canOperate(host) : false;
  const onActions = useCallback(
    (blockId: number) => router.push({ pathname: "/block", params: { key, pane: String(paneId), block: String(blockId) } }),
    [router, key, paneId],
  );
  const onBell = useCallback((b: Block, on: boolean) => fleet.setNotify(key, paneId, b.command, on), [key, paneId]);
  const onStop = useCallback(() => {
    setError(null);
    void fleet.sendKeys(key, paneId, ["ctrl+c"]).then((r) => {
      if (r.ok) AccessibilityInfo.announceForAccessibility("Control C sent");
      else setError(r.message);
    });
  }, [key, paneId]);
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
        if (r.ok) AccessibilityInfo.announceForAccessibility("Command sent");
        else setError(r.message);
      });
    },
    [router, key, paneId, operate, once],
  );
  const onLoadEarlier = useCallback((blockId: number) => void fleet.loadEarlier(blockId), []);
  const canLoadEarlier = hasFeature(host, "output");
  const earlier = pane?.earlier;
  const asked = s.asked;
  const extraData = useMemo(() => ({ expanded, earlier, canLoadEarlier, wrap, asked }), [expanded, earlier, canLoadEarlier, wrap, asked]);
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

  const toggle = useCallback(
    (blockId: number) =>
      setExpanded((prev) => {
        const next = new Set(prev);
        if (!next.delete(blockId)) next.add(blockId);
        return next;
      }),
    [],
  );

  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    atBottom.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - BOTTOM_SLOP;
  }, []);

  const onContentSizeChange = useCallback(() => {
    if (atBottom.current) list.current?.scrollToEnd({ animated: false });
  }, []);

  // the tab's panes, and a swipe across the blocks between them
  const tab = loc?.tab ?? null;
  const pager = tab ? pagerModel(tab, paneId) : null;
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
  // a screen reader swipes through elements, so the pager is also two actions
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

  const conn = host ? describeConn(host.conn) : null;
  // the back badge's count: this computer's other panes waiting for an answer
  const waitingElsewhere =
    host?.windows.reduce((n, w) => n + w.tabs.reduce((m, t) => m + t.panes.filter((p) => p.waiting && p.id !== paneId).length, 0), 0) ?? 0;
  const hostName = host?.record.name ?? "this computer";
  const back = () => (router.canGoBack() ? router.back() : router.replace("/"));
  const lastBlock = blocks.at(-1);
  // a line under the tabs: the pager, a full-screen program, or why the computer is not connected
  const strip = !host ? null : status !== "open" ? (
    <Text style={[styles.pagerText, { color: colors.warning }]} accessibilityLiveRegion="polite">
      {conn!.label}
    </Text>
  ) : fullScreen && !(pinned && !screen?.alt) ? (
    <Text style={[styles.pagerText, { color: colors.warning }]} accessibilityLiveRegion="polite">
      {program} is full screen · showing its screen{pager ? ` · pane ${pager.index + 1} of ${pager.count}` : ""}
    </Text>
  ) : pager ? (
    <View
      style={styles.pagerRow}
      accessible
      accessibilityLabel={pager.text}
      accessibilityActions={pagerActions}
      onAccessibilityAction={(e) => goPane(tab ? swipeTarget(tab, paneId, e.nativeEvent.actionName === "next" ? 1 : -1) : null)}
    >
      <Text style={styles.pagerText}>{pager.text}</Text>
      <View style={styles.dots}>
        {Array.from({ length: pager.count }, (_, i) => (
          <View key={i} style={[styles.pageDot, i === pager.index && styles.pageDotOn]} />
        ))}
      </View>
    </View>
  ) : null;

  return (
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      <Stack.Screen options={{ headerShown: false }} />
      <View style={[styles.header, { paddingTop: headerTop(insets.top) - 2 }, !strip && styles.headerRule]}>
        <View style={styles.headerRow}>
          <IconButton
            glass
            icon="back"
            label={`Back from ${hostName}${waitingElsewhere ? `, ${waitingElsewhere} ${waitingElsewhere === 1 ? "needs" : "need"} you` : ""}`}
            badge={waitingElsewhere}
            onPress={back}
          />
          <View style={styles.titlePill}>
            <Text style={styles.hostName} numberOfLines={1} accessibilityRole="header">
              {host ? hostName : "Pane"}
            </Text>
          </View>
          <View style={styles.flex} />
          {!host || closed ? null : (
            <GlassGroup>
              {fullScreen ? null : (
                <>
                  <IconButton
                    grouped
                    icon={wrap ? "wrapOn" : "wrapOff"}
                    label={`Wrap long lines: ${wrap ? "on" : "off"}`}
                    hint="For this pane; Settings › Output sets it for all"
                    active={!wrap}
                    onPress={() => fleet.setPaneWrap(key, paneId, !wrap)}
                  />
                  <IconButton
                    grouped
                    icon="search"
                    label="Search output"
                    pressed={searching !== null}
                    onPress={() => setSearch(searching === null ? "" : null)}
                  />
                </>
              )}
              {screenFeature ? (
                <IconButton
                  grouped
                  icon="terminal"
                  label="Terminal view"
                  hint="Shows the pane's screen instead of its blocks"
                  pressed={pinned}
                  active={pinned}
                  onPress={() => setTermView((v) => !v)}
                />
              ) : null}
            </GlassGroup>
          )}
        </View>
        {loc ? (
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            style={styles.tabStrip}
            contentContainerStyle={styles.tabs}
            accessibilityRole="tablist"
            accessibilityLabel="Tabs"
          >
            {loc.window.tabs.map((t, i) => {
              const on = t === loc.tab;
              const failed = tabState(t) === "failed";
              const go = tabPane(t);
              return (
                <Pressable
                  key={`${i}:${t.index}`}
                  accessibilityRole="tab"
                  accessibilityState={{ selected: on }}
                  accessibilityLabel={[tabTitle(t), t.panes.length > 1 ? `${t.panes.length} panes` : null, failed ? "a command failed" : null]
                    .filter(Boolean)
                    .join(", ")}
                  onPress={() => !on && goPane(go)}
                  hitSlop={{ top: 2, bottom: 6 }}
                  style={[styles.tab, on && styles.tabOn]}
                >
                  <Text style={[styles.tabText, on && styles.tabTextOn]} numberOfLines={1}>
                    {tabTitle(t)}
                    {on && t.panes.length > 1 ? <Text style={styles.tabCount}> {t.panes.length}</Text> : null}
                    {failed ? <Text style={styles.tabFailed}> ●</Text> : null}
                  </Text>
                </Pressable>
              );
            })}
          </ScrollView>
        ) : null}
      </View>
      {strip ? <View style={styles.pager}>{strip}</View> : null}
      {loc?.pane.waiting && status === "open" ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${askingTitle(loc.pane.waiting, program)}: ${loc.pane.waiting.prompt}`}
          accessibilityHint="Opens the question"
          onPress={() => router.push({ pathname: "/answer", params: { key, pane: String(paneId), from: "pane" } })}
          style={({ pressed }) => [styles.asking, pressed && styles.pressed]}
        >
          <View style={styles.askingDot} />
          <Text style={styles.askingText} numberOfLines={1}>
            {askingTitle(loc.pane.waiting, program)}: {loc.pane.waiting.prompt}
          </Text>
          <Text style={styles.askingLink}>Answer</Text>
        </Pressable>
      ) : null}
      {host?.notice ? <ErrorText style={styles.notice}>{host.notice}</ErrorText> : null}
      {error ? <ErrorText style={styles.notice}>{error}</ErrorText> : null}
      {host && fullScreen && !closed ? (
        <View style={[styles.fill, { paddingBottom: keyboard ? 0 : Math.max(0, insets.bottom - 12) }]} {...swipe.panHandlers}>
          <TerminalView
            hostKey={key}
            paneId={paneId}
            screen={screen}
            operate={operate}
            canType={hasFeature(host, "text")}
            online={status === "open"}
            running={running !== null}
            program={program}
            pinned={pinned}
          />
        </View>
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
                {searching.trim() === "" ? "" : `${shownBlocks} of ${blocks.length}`}
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
              <Button label="Go to Now" onPress={() => router.dismissTo("/")} />
            </View>
          ) : !state ? (
            <Text style={styles.empty} accessibilityLiveRegion="polite">
              {status === "open" ? "Loading blocks…" : "Waiting for the computer…"}
            </Text>
          ) : blocks.length === 0 ? (
            <Text style={styles.empty}>No commands in this pane yet.</Text>
          ) : searching !== null && searching.trim() !== "" && shownBlocks === 0 ? (
            <Text style={styles.empty}>No block holds “{searching.trim()}”.</Text>
          ) : null}
          <View style={styles.fill} {...swipe.panHandlers}>
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
                    {/* keyed: a recycled card must not keep another block's "All N lines" */}
                    <FailedCard key={item.block.id} block={item.block} onActions={onActions} onRunAgain={operate ? onRunAgain : undefined} />
                  </View>
                ) : (
                  <BlockView
                    block={item.block}
                    expanded={expanded.has(item.block.id)}
                    onToggle={toggle}
                    onActions={onActions}
                    earlier={earlier?.[item.block.id]}
                    canLoadEarlier={canLoadEarlier}
                    onLoadEarlier={onLoadEarlier}
                    wrap={wrap}
                    notify={isAsked(s, key, paneId, item.block.command)}
                    onBell={onBell}
                  />
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
          </View>
          {closed ? null : host && operate ? (
            // the board's 30 pt under the command line: the home indicator's inset, less the
            // command line's own 8 pt
            <View style={{ paddingBottom: keyboard ? 0 : Math.max(0, insets.bottom - 12), backgroundColor: colors.chrome }}>
              <Composer
                hostKey={key}
                paneId={paneId}
                running={running}
                online={status === "open"}
                chips={promptChips(loc?.pane ?? null, state, hasFeature(host, "git"))}
                chipMenus={hasFeature(host, "chips")}
                complete={hasFeature(host, "complete") && operate}
                recent={recent}
                label={title}
                canType={hasFeature(host, "text")}
              />
            </View>
          ) : host ? (
            <Text style={[styles.viewOnly, { paddingBottom: insets.bottom + 10 }]}>View only: this device cannot run commands here.</Text>
          ) : null}
        </>
      )}
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  pressed: { opacity: 0.7 },
  // the P3 Pane board's header: back, the computer, search; the tab strip under it
  flex: { flex: 1 },
  // the Pane board's glass header: back badge, the computer in a pill, the actions' pill
  header: { backgroundColor: colors.chrome, paddingHorizontal: 12 },
  headerRule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  headerRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  titlePill: {
    flexShrink: 1,
    height: 40,
    paddingHorizontal: 16,
    justifyContent: "center",
    borderRadius: 20,
    backgroundColor: colors.glass,
    borderWidth: 1,
    borderColor: colors.glassRim,
  },
  hostName: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  // the strip reaches 6 pt below the 36 pt tabs (iOS takes no touch outside it): a 44 pt touch
  tabStrip: { marginBottom: -6 },
  tabs: { gap: 4, paddingTop: 6, paddingBottom: 6 },
  tab: { height: 36, paddingHorizontal: 12, justifyContent: "center", borderBottomWidth: 2, borderBottomColor: "transparent" },
  tabOn: { borderBottomColor: colors.accent },
  tabText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
  tabTextOn: { fontFamily: ui(600), color: colors.textBright },
  tabCount: { fontFamily: ui(400), color: colors.textMuted },
  tabFailed: { color: colors.error },
  pager: {
    paddingVertical: 8,
    alignItems: "center",
    backgroundColor: colors.chrome,
    borderBottomWidth: 1,
    borderBottomColor: colors.hairline,
  },
  pagerRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  pagerText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  dots: { flexDirection: "row", gap: 4, marginLeft: 6 },
  pageDot: { width: 6, height: 6, borderRadius: 3, backgroundColor: "#ffffff38" },
  pageDotOn: { width: 14, backgroundColor: colors.accent },
  cardGap: { paddingVertical: 2 },
  // the pane waits for the user: its question, one tap from the answer
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
  listContent: { paddingTop: 6, paddingBottom: 6 },
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
