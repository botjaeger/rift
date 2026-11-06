import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { Stack, useFocusEffect, useIsFocused, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  AccessibilityInfo,
  BackHandler,
  Keyboard,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
  useWindowDimensions,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { askingTitle, questionKey } from "@/answer";
import { chipItems } from "@/promptChips";
import { canClose, canOperate, draftKey, hasFeature, isAsked, wrapsLines } from "@/fleet";
import { foldBlocks, paneKind, paneName, programName, swipeTarget, tabKind, tabPane, type PaneItem } from "@/now";
import { bareScreen as screenIsBare, CLAUDE_COLOR, paneMark, runsAgent, screenView, waitingOthers } from "@/paneView";
import { canLoadOlder, MAX_BLOCKS } from "@/protocol/store";
import type { Block } from "@/protocol/types";
import { fleet, useApp, useScreen } from "@/runtime";
import { pending } from "@/pending";
import { atEnd, screenProgram, showsTerminal } from "@/terminal";
import { findPane, paneTitle, runningCommand, tabTitle } from "@/topology";
import { AskCard } from "@/ui/AskCard";
import { FailedCard, FoldRow, RunningCard } from "@/ui/BlockCards";
import { BlockView } from "@/ui/BlockView";
import { SelectionItem, SelectionScope, type SelectionScopeHandle } from "@/ui/SelectionScope";
import { Composer } from "@/ui/Composer";
import { ComputerMenu } from "@/ui/ComputerMenu";
import { hostLine } from "@/ui/connStatus";
import { hostDot } from "@/ui/hostDot";
import { StatusMark } from "@/ui/StatusMark";
import { usePending } from "@/ui/usePending";
import { Button } from "@/ui/Button";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { useHostNotifications } from "@/ui/useHostNotifications";
import { BlockSkeleton, PaneCard, PaneStrip } from "@/ui/PaneLoading";
import { openingTitle, paneLoad } from "@/ui/paneLoad";
import { canNewPane, canNewTab } from "@/ui/newTab";
import { Field } from "@/ui/Field";
import { GlassGroup, headerTop, IconButton } from "@/ui/Screen";
import { TabBar } from "@/ui/TabBar";
import { TerminalView } from "@/ui/TerminalView";
import { TextSizeMenu } from "@/ui/TextSizeMenu";
import { useClose } from "@/ui/useClose";
import { useNow } from "@/ui/useNow";
import { useSingleFlight } from "@/ui/useSingleFlight";
import { avatarInk } from "@/ui/palette";
import { mono, radius, ui, makeStyles, useColors } from "@/ui/theme";

/** Within this many points of the end counts as "at the bottom" (stick to new output). */
const BOTTOM_SLOP = 48;
/** The wall clock, for effects (reads of it in render are flagged as impure). */
const wallClock = () => Date.now();

/** Blocks whose command or output holds `query` (any case); all of them for an empty query. */
function matching(blocks: Block[], query: string): Block[] {
  const q = query.trim().toLowerCase();
  if (q === "") return blocks;
  return blocks.filter((b) => b.command.toLowerCase().includes(q) || b.output.toLowerCase().includes(q));
}

const itemKey = (it: PaneItem) => (it.kind === "fold" ? `fold:${it.ids[0]}` : String(it.block.id));

/**
 * A pane (the Pane board): the tab and its panes as two buttons that open their sheets, search and
 * the ⋯ menu; the pane picker between the tab's panes; its blocks as cards with the earlier successes
 * folded; an agent's question docked at the bottom (the PaneAsking board); or, while a
 * full-screen program runs (vim, htop, less: the alternate screen) or the pane runs an agent or a program no block holds,
 * the pane's screen (the FullScreen board).
 */
export default function PaneScreen() {
  const colors = useColors();
  const styles = useStyles();
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
  // the computer menu (the header's name)
  const [computers, setComputers] = useState(false);
  // the selection bar while text is selected in the output (SelectionScope) or on the screen (TerminalView)
  const [selBar, setSelBar] = useState<ReactNode | null>(null);
  const [barH, setBarH] = useState(0);
  // Android's Back closes the open menu, not the pane under it
  useEffect(() => {
    if (!menu && !computers) return;
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      setMenu(false);
      setComputers(false);
      return true;
    });
    return () => sub.remove();
  }, [menu, computers]);
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
  const alerts = useHostNotifications(key, (message) => {
    setError(message);
    if (message) setMenu(false);
  }, (on) => say(on ? "Notifications on for this computer" : "Notifications off for this computer"));
  // this screen, as the pane's holder (the same pane may be pushed twice)
  const [holder] = useState(() => ({}));
  // the keyboard covers the bottom inset: no gap between it and the command line
  const [keyboard, setKeyboard] = useState(false);
  useEffect(() => {
    const ios = Platform.OS === "ios";
    const show = Keyboard.addListener(ios ? "keyboardWillShow" : "keyboardDidShow", () => setKeyboard(true));
    const hide = Keyboard.addListener(ios ? "keyboardWillHide" : "keyboardDidHide", () => setKeyboard(false));
    // the list shrinks as the keyboard comes up (and grows as it goes): keep its end in sight once
    // it has (the screen's own ScrollView follows its end itself)
    const toEnd = () => {
      if (atBottom.current) list.current?.scrollToEnd({ animated: false });
    };
    const settled = Keyboard.addListener("keyboardDidShow", toEnd);
    const closed = Keyboard.addListener("keyboardDidHide", toEnd);
    return () => {
      show.remove();
      hide.remove();
      settled.remove();
      closed.remove();
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
  // every frame, read here only: the app state takes rows quietly (fleet.screenTick)
  const screen = useScreen(key, paneId);
  // TUI or blocks by one rule, no switch (screenView): the alternate screen, an agent (on either
  // screen: Claude Code picks its renderer per launch) or a program no block holds show the
  // screen (gateway mode `on` unless only the alternate screen asks); everything else, blocks.
  const agent = !!loc && runsAgent(loc.pane);
  const view = screenFeature && loc
    ? screenView(loc.pane, !!screen?.alt, state === null || state.blocks.at(-1)?.state === "running", state !== null && state.blocks.length === 0)
    : { tui: showsTerminal(screen, screenFeature), pinned: false };
  const pinned = view.pinned;
  const fullScreen = view.tui;
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
  // ticks while the pane loads or the computer menu (its "trying again in" lines) is open
  const now = useNow(1000, (waitingForPane || computers) && focused);
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
  // Cancel stops the retrying; both ways lead to Portals
  const leave = () => {
    if (load?.card?.leave === "Cancel") fleet.cancelRetry(key);
    router.dismissTo("/");
  };
  const tryAgain = () => {
    setWaitingSince(Date.now());
    if (status === "open") fleet.refreshPane(key, paneId);
    else fleet.retry(key);
  };

  const wrap = wrapsLines(s, key, paneId);
  const operate = host ? canOperate(host) : false;
  const closeable = canClose(host);
  const canSplit = !!host && !closed && canNewPane(host, paneId);
  const canOpenTab = !!host && !closed && canNewTab(host);
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
  // the Search output query, marked in the blocks it shows (undefined: no search)
  const found = searching?.trim() || undefined;
  const extraData = useMemo(() => ({ wrap, asked, found }), [wrap, asked, found]);
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

  // the output's selection scope: it measures the selected field again after a scroll
  const selectionScope = useRef<SelectionScopeHandle>(null);
  // Judged when a drag or fling ends, not on every scroll event: the list shrinking under the
  // keyboard also reports a scroll, with the end now out of reach, and that read as "scrolled up".
  const onScrollEnd = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    atBottom.current = atEnd(contentOffset.y, contentSize.height, layoutMeasurement.height, BOTTOM_SLOP);
    // the selected field moved: a tap on it (or its handles) must still keep the selection
    selectionScope.current?.refresh();
  }, []);

  const onContentSizeChange = useCallback(() => {
    if (atBottom.current) list.current?.scrollToEnd({ animated: false });
  }, []);

  // the tab's panes: the picker below the header switches between them
  const tab = loc?.tab ?? null;
  // another pane takes this one's place: Back still leads where it led
  const goPane = useCallback(
    (next: number | null) => next !== null && router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(next) } }),
    [router, key],
  );
  // a screen reader's actions on the pane picker: the next and the previous pane
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
  // on its side the screen needs every row: one header line, no picker
  const { width: winWidth, height: winHeight } = useWindowDimensions();
  const compact = fullScreen && winWidth > winHeight;
  const headerPad = compact ? Math.max(insets.top, 6) : headerTop(insets.top) + 2;
  const tabNumber = loc ? loc.window.tabs.indexOf(loc.tab) + 1 : 0;
  const tabCount = loc ? loc.window.tabs.length : 0;
  const paneNumber = loc ? loc.paneIndex + 1 : 0;
  const others = tab ? waitingOthers(tab, paneId) : 0;
  const mark = loc ? paneMark(loc.pane) : null;
  const pickName = loc ? paneName(loc.pane, loc.paneIndex) : "";
  const waiting = loc?.pane.waiting;
  // an agent's menu is answered on the pane, from a card at the bottom
  const asking = !!waiting && waiting.kind === "choice" && waiting.choices.length > 0 && status === "open" && operate && !closed;
  // a question to answer, a coding agent on either screen, or the TUI of a program no block holds: the
  // screen without its own keys, the pane's message input (or the question) under it
  const box = draftKey(key, paneId);
  // a screen with its own keys (vim, htop) shows the message box while something pasted is in it
  const [pastedPane, setPastedPane] = useState<number | null>(null);
  useEffect(() => {
    if (pastedPane === null) return;
    // a turn later: the box takes what was handed to it and fills the draft in two steps
    const check = () => setTimeout(() => {
      const v = pending.view(box);
      if (v.items.length === 0 && v.text === "" && !v.walking && !fleet.store.peek().drafts[box]) setPastedPane(null);
    }, 0);
    const offDrafts = fleet.store.subscribe(check);
    const offBox = pending.subscribe(check);
    return () => {
      offDrafts();
      offBox();
    };
  }, [pastedPane, box]);
  const held = usePending(box);
  const bareScreen = screenIsBare({ fullScreen, asking, agent, pinned, alt: !!screen?.alt }) || (fullScreen && (pastedPane === paneId || held.items.length > 0));
  const bottomInset = keyboard ? 0 : Math.max(0, insets.bottom - 12);
  // the tab bar over the command line: the window's tabs either side, a swipe goes to them
  const siblings = loc?.window.tabs ?? [];
  const prevTab = loc ? (siblings[siblings.indexOf(loc.tab) - 1] ?? null) : null;
  const nextTab = loc ? (siblings[siblings.indexOf(loc.tab) + 1] ?? null) : null;
  // stable across renders: the bar's swipe gesture is rebuilt when these change, and a pane
  // streaming output re-renders many times a second (a rebuilt gesture drops a swipe in progress)
  const prevPane = prevTab ? tabPane(prevTab) : null;
  const nextPane = nextTab ? tabPane(nextTab) : null;
  // the next tab slides in from the right, the previous one from the left (the pane screen's
  // animation reads `dir`, _layout.tsx)
  const goPaneOf = useCallback(
    (to: number | null, dir: "next" | "prev") => {
      if (to !== null) router.replace({ pathname: "/host/[key]/pane/[id]", params: { key, id: String(to), dir } });
    },
    [router, key],
  );
  const onPrevTab = useCallback(() => goPaneOf(prevPane, "prev"), [goPaneOf, prevPane]);
  const onNextTab = useCallback(() => goPaneOf(nextPane, "next"), [goPaneOf, nextPane]);
  const openSwitcher = useCallback(() => router.push({ pathname: "/tabs", params: { key, pane: String(paneId) } }), [router, key, paneId]);
  // the tab row under the header: on its side the screen needs every row
  const tabBar =
    loc && tab && !compact && !closed ? (
      <TabBar
        title={tabTitle(loc.tab)}
        kind={tabKind(loc.tab)}
        number={tabNumber}
        count={tabCount}
        prev={prevTab ? tabTitle(prevTab) : null}
        next={nextTab ? tabTitle(nextTab) : null}
        onPrev={onPrevTab}
        onNext={onNextTab}
        onOpen={openSwitcher}
      />
    ) : null;
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
          canUpload={hasFeature(host, "upload")}
          canUploadFiles={hasFeature(host, "upload_files")}
          canPaste={hasFeature(host, "paste")}
          agent={agent}
          compact={compact}
          waiting={status !== "open"}
        />
      </View>
    ) : (
      <Text style={[styles.viewOnly, { paddingBottom: insets.bottom + 10 }]}>View only: this device cannot run commands here.</Text>
    );
  const footer =
    asking && waiting ? (
      <AskCard key={questionKey(waiting)} hostKey={key} paneId={paneId} waiting={waiting} program={program} where={loc ? tabTitle(loc.tab) : undefined} bottomInset={bottomInset} onSay={say} />
    ) : (
      composer
    );
  // while text is selected its bar is docked where the controls are: drawn over them, which stay
  // in place, hidden, so the output does not move under the finger. A question card stays: it is
  // the pane's answer, not its controls.
  const docked = selBar !== null && !(asking && waiting);
  // controls shorter than the bar (no chips row): the place grows to the bar, all of it in reach
  const dockedFooter = (
    <View style={docked ? { minHeight: barH + bottomInset } : null}>
      {/* over a question card, which stays, the bar goes above it */}
      {selBar !== null && !docked ? <View style={styles.overCard}>{selBar}</View> : null}
      <View style={docked ? styles.underBar : null} pointerEvents={docked ? "none" : "auto"} accessibilityElementsHidden={docked} importantForAccessibility={docked ? "no-hide-descendants" : "auto"}>
        {footer}
      </View>
      {docked ? (
        <View style={[styles.dock, { bottom: bottomInset }]} onLayout={(e) => setBarH(e.nativeEvent.layout.height)}>
          {selBar}
        </View>
      ) : null}
    </View>
  );

  return (
    <KeyboardAvoidingView style={styles.fill} behavior="padding">
      {/* No Back swipe while text is selected: dragging a handle sideways would take the pane
          away (iOS starts its Back pan over any touch the app handles itself) */}
      <Stack.Screen options={{ headerShown: false, gestureEnabled: selBar === null }} />
      <View style={[styles.header, { paddingTop: headerPad }]}>
        <View style={styles.headerRow}>
          <IconButton
            glass
            icon="back"
            label={"Back from " + hostName + (waitingElsewhere ? ", " + waitingElsewhere + " " + (waitingElsewhere === 1 ? "needs" : "need") + " you" : "")}
            badge={waitingElsewhere || undefined}
            onPress={back}
          />
          {/* the computer: its name opens the computer menu; the tab's name is on the tab row */}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={host ? `${hostName}, ${hostLine(host.conn, now, null).text}. Computers` : hostName}
            accessibilityState={{ expanded: computers }}
            disabled={!host}
            onPress={() => {
              // the keys strip rides the keyboard above the page: a menu under it loses its last rows
              Keyboard.dismiss();
              setMenu(false);
              setComputers((c) => !c);
            }}
            style={({ pressed }) => [styles.titleButton, pressed && styles.pressed]}
          >
            <View style={styles.titleLine}>
              {host ? <View style={[styles.hostDot, { backgroundColor: colors[hostDot(host)] }]} /> : null}
              <Text style={styles.title} numberOfLines={1}>
                {hostName}
              </Text>
              {host ? <Icon name="chevronDown" size={16} color={colors.textMuted} /> : null}
            </View>
          </Pressable>
          {!host || closed ? null : (
            <GlassGroup>
              {fullScreen ? null : (
                <IconButton grouped icon="search" label="Search output" pressed={searching !== null} active={searching !== null} onPress={() => setSearch(searching === null ? "" : null)} />
              )}
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="More"
                accessibilityState={{ expanded: menu }}
                hitSlop={3}
                onPress={() => {
                  Keyboard.dismiss();
                  setComputers(false);
                  setMenu((m) => !m);
                }}
                style={({ pressed }) => [styles.action, pressed && styles.pressed]}
              >
                <Icon name="more" size={19} color={colors.text} />
              </Pressable>
            </GlassGroup>
          )}
        </View>
      </View>
      {tabBar}
      {loc && tab && !closed && !compact && tab.panes.length > 1 ? (
        <View style={styles.paneRow}>
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
              <Text style={[styles.pickMark, { color: mark?.claude ? avatarInk(CLAUDE_COLOR, colors) : colors.accent }]}>{mark?.mark}</Text>
              <Text style={styles.pickName} numberOfLines={1}>
                {pickName}
              </Text>
              {loc ? <StatusMark kind={paneKind(loc.pane)} /> : null}
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
          </View>
        </View>
      ) : null}
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
          <View style={[styles.fill, { paddingBottom: bareScreen ? 0 : bottomInset }, load?.held && styles.held]}>
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
              canScroll={hasFeature(host, "scroll")}
              canUpload={hasFeature(host, "upload") && operate}
              canUploadFiles={hasFeature(host, "upload_files") && operate}
              onPasted={() => setPastedPane(paneId)}
              onBar={setSelBar}
              canPaste={hasFeature(host, "paste") && operate}
              canMouse={hasFeature(host, "mouse") && operate}
              keepHeight={asking}
              covered={menu || computers}
            />
            {load?.card ? <PaneCard card={load.card} onRetry={tryAgain} onDetails={details} onLeave={leave} /> : null}
          </View>
          {bareScreen ? dockedFooter : null}
        </>
      ) : (
        <>
          {searching !== null ? (
            <View style={styles.search}>
              <Field
                icon="search"
                label="Search output"
                value={searching}
                onChangeText={setSearch}
                autoFocus
                autoCapitalize="none"
                autoCorrect={false}
                placeholder="Search commands and output"
                returnKeyType="search"
                trailing={
                  searching.trim() === "" ? null : (
                    <Text style={styles.searchCount} accessibilityLiveRegion="polite">
                      {`${shownBlocks} of ${blocks.length} ${blocks.length === 1 ? "block" : "blocks"}`}
                    </Text>
                  )
                }
              />
            </View>
          ) : null}
          {!host ? (
            // a tapped alert opens this page on a cold start before the paired computers are read
            s.ready ? <Text style={styles.empty}>This computer is not paired.</Text> : null
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
          <View style={[styles.fill, load?.held && styles.held]}>
            <SelectionScope ref={selectionScope} hostKey={key} paneId={paneId} canPrompt={operate && !closed} onBar={setSelBar}>
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
                    <SelectionItem owner={`block:${item.block.id}`}>
                      <View style={styles.cardGap}>
                        <RunningCard
                          block={item.block}
                          onActions={onActions}
                          notify={isAsked(s, key, paneId, item.block.command)}
                          onBell={onBell}
                          onStop={operate ? onStop : undefined}
                          mark={found}
                        />
                      </View>
                    </SelectionItem>
                  ) : item.block.state === "done" && item.block.exit !== null && item.block.exit !== 0 ? (
                    <SelectionItem owner={`block:${item.block.id}`}>
                      <View style={styles.cardGap}>
                        {/* keyed: a recycled card must not keep another block's state */}
                        <FailedCard key={item.block.id} block={item.block} onActions={onActions} onRunAgain={operate ? onRunAgain : undefined} mark={found} />
                      </View>
                    </SelectionItem>
                  ) : (
                    <SelectionItem owner={`block:${item.block.id}`}>
                      <View style={styles.cardGap}>
                        <BlockView key={item.block.id} block={item.block} onActions={onActions} wrap={wrap} mark={found} />
                      </View>
                    </SelectionItem>
                  )
                }
                ListHeaderComponent={closed ? null : header}
                // opened at a block: no sticking to the bottom, which would scroll past it
                maintainVisibleContentPosition={
                  target === null
                    ? { startRenderingFromBottom: true, autoscrollToBottomThreshold: 0.1, animateAutoScrollToBottom: false }
                    : {}
                }
                onScrollBeginDrag={() => {
                  atBottom.current = false;
                }}
                onScrollEndDrag={onScrollEnd}
                // the keyboard follows a drag down the list (the only way to put it away at a prompt)
                keyboardDismissMode="interactive"
                onMomentumScrollEnd={onScrollEnd}
                onContentSizeChange={onContentSizeChange}
                contentContainerStyle={styles.listContent}
                accessibilityLabel="Command blocks, newest at the bottom"
              />
            </SelectionScope>
            {load?.card ? <PaneCard card={load.card} onRetry={tryAgain} onDetails={details} onLeave={leave} /> : null}
          </View>
          {dockedFooter}
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
              disabled={alerts.saving}
              last={!fullScreen}
              onPress={() => alerts.change(!notifOn)}
            />
            {/* over a program's screen: the zoom a pinch changes, from here too (no Wrap there) */}
            {fullScreen ? <TextSizeMenu last /> : null}
            {/* grouped: settings above, then new things, then what closes (in red), a gap between */}
            {canSplit || canOpenTab ? <View style={styles.menuGap} /> : null}
            {/* the New pane / New tab sheets (the program to run), as the Tabs page's header */}
            {canSplit ? (
              <MenuItem icon="newPane" title="Split pane" hint="Another program beside this one" last={!canOpenTab} onPress={() => { setMenu(false); router.push({ pathname: "/new-pane", params: { key, pane: String(paneId) } }); }} />
            ) : null}
            {canOpenTab ? (
              <MenuItem icon="add" title="New tab" hint={`On ${hostName}`} last onPress={() => { setMenu(false); router.push({ pathname: "/new-tab", params: { key } }); }} />
            ) : null}
            {closeable ? (
              <>
                <View style={styles.menuGap} />
                <MenuAction title="Close pane" onPress={() => { setMenu(false); onClose(paneId, "pane"); }} />
                <MenuAction title="Close tab" last onPress={() => { setMenu(false); onClose(paneId, "tab"); }} />
              </>
            ) : null}
          </View>
        </>
      ) : null}
      {computers && host ? (
        <>
          <Pressable accessibilityRole="button" accessibilityLabel="Close menu" onPress={() => setComputers(false)} style={StyleSheet.absoluteFill} />
          <ComputerMenu
            hosts={s.hosts}
            current={key}
            now={now}
            top={headerPad + 50}
            onOpen={(h, p) => {
              setComputers(false);
              router.replace({ pathname: "/host/[key]/pane/[id]", params: { key: h, id: String(p) } });
            }}
            onConnection={() => {
              setComputers(false);
              details();
            }}
            onDismiss={() => setComputers(false)}
            onPortals={() => {
              setComputers(false);
              router.dismissTo("/");
            }}
          />
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
  const styles = useStyles();
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

/** One of the ⋯ menu's actions that opens a page: an icon, the label and what it does. */
function MenuItem({ icon, title, hint, last, onPress }: { icon: "newPane" | "add"; title: string; hint: string; last?: boolean; onPress: () => void }) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <Pressable accessibilityRole="menuitem" accessibilityLabel={title} accessibilityHint={hint} onPress={onPress} style={({ pressed }) => [styles.menuRow, !last && styles.menuRule, pressed && styles.pressed]}>
      <Icon name={icon} size={20} color={colors.text} />
      <View style={styles.menuText}>
        <Text style={styles.menuTitle}>{title}</Text>
        <Text style={styles.menuHint}>{hint}</Text>
      </View>
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
  disabled,
  onPress,
}: {
  icon: "wrapOn" | "bell";
  title: string;
  hint: string;
  on: boolean;
  last?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  const colors = useColors();
  const styles = useStyles();
  return (
    <Pressable
      accessibilityRole="switch"
      accessibilityLabel={title}
      accessibilityHint={hint}
      accessibilityState={{ checked: on, disabled: !!disabled }}
      aria-checked={on}
      disabled={disabled}
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

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  flex: { flex: 1 },
  pressed: { opacity: 0.7 },
  // the Pane board's header: chrome, 12 pt gutter, 10 pt between its two rows, a hairline under
  header: { backgroundColor: colors.chrome, paddingHorizontal: 12, paddingBottom: 10, gap: 10, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  headerRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  titleButton: { flex: 1, minWidth: 0, minHeight: 44, justifyContent: "center", paddingHorizontal: 6 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 7 },
  hostDot: { width: 8, height: 8, borderRadius: 4 },
  // the pane picker under the tab row, for a tab of several panes
  // the selection bar over the pane's controls (hidden in place under it)
  underBar: { opacity: 0 },
  dock: { position: "absolute", left: 0, right: 0, backgroundColor: colors.chrome },
  overCard: { backgroundColor: colors.chrome },
  paneRow: { backgroundColor: colors.chrome, paddingHorizontal: 12, paddingVertical: 8, borderBottomWidth: 1, borderBottomColor: colors.hairline },
  title: { flexShrink: 1, fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  // the ⋯ in the glass pill, as a GlassGroup's 38 pt button (it reports its menu as expanded)
  action: { width: 38, height: 38, borderRadius: 19, alignItems: "center", justifyContent: "center" },
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
    backgroundColor: colors.faint,
  },
  pickMark: { fontFamily: mono, fontSize: 13 },
  pickName: { flexShrink: 1, fontFamily: ui(500), color: colors.textBright, fontSize: 13 },
  pickPos: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13 },
  pickWait: { flexDirection: "row", alignItems: "center", gap: 5 },
  pickDot: { width: 7, height: 7, borderRadius: 4, backgroundColor: colors.warning },
  pickWaitText: { fontFamily: ui(400), color: colors.warning, fontSize: 12 },
  // blocks held in memory while the pane reconnects (PaneLoading board)
  held: { opacity: 0.55 },
  // the ⋯ menu: 280 wide under the button, 14 pt radius, #2c2c30 with a soft shadow
  menu: {
    position: "absolute",
    right: 12,
    width: 280,
    borderRadius: 14,
    backgroundColor: colors.menu,
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
  // between the menu's groups: a band of the page's ground, as iOS menus part theirs
  menuGap: { height: 6, backgroundColor: colors.bg, borderTopWidth: 1, borderBottomWidth: 1, borderColor: colors.hairline },
  menuNote: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17, paddingHorizontal: 14, paddingVertical: 12 },
  // the text column of the rows with icons (14 + the 20 pt icon + 12)
  menuAction: { minHeight: 48, justifyContent: "center", paddingLeft: 46, paddingRight: 14 },
  // Close pane, Close tab: the destructive red (orange is for "needs you" only)
  menuDestructive: { fontFamily: ui(400), color: colors.error, fontSize: 15 },
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
  // the field across the whole width (the Standard)
  search: {
    paddingHorizontal: 12,
    paddingVertical: 8,
    backgroundColor: colors.chrome,
    borderBottomWidth: 1,
    borderBottomColor: colors.hairline,
  },
  searchCount: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
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
}));
