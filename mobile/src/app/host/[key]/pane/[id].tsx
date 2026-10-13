import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { Stack, useFocusEffect, useLocalSearchParams, useRouter } from "expo-router";
import { useHeaderHeight } from "expo-router/react-navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Keyboard,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { canLoadOlder, MAX_BLOCKS } from "@/protocol/store";
import type { Block } from "@/protocol/types";
import { canOperate, hasFeature } from "@/fleet";
import { fleet, useApp } from "@/runtime";
import { keptPick, showsTerminal, type ViewPick } from "@/terminal";
import { findPane, paneTitle, runningCommand } from "@/topology";
import { BlockView } from "@/ui/BlockView";
import { Button } from "@/ui/Button";
import { Composer } from "@/ui/Composer";
import { ErrorText } from "@/ui/Notice";
import { StatusLine } from "@/ui/StatusLine";
import { TerminalView } from "@/ui/TerminalView";
import { colors, radius, TOUCH } from "@/ui/theme";

/**
 * Item and container roles of the switch. React Native on iOS (Fabric) gives `tab` and `tablist`
 * no trait at all (accessibilityPropsConversions.h); `tabbar` on the container makes VoiceOver
 * read the items as tabs ("Blocks, selected, tab, 1 of 2"), and `button` gives them their trait.
 */
const SWITCH_ROLES =
  Platform.OS === "ios" ? ({ item: "button", list: "tabbar" } as const) : ({ item: "tab", list: "tablist" } as const);

/** The Blocks | Terminal switch (a gateway with `screen` only). */
function ViewSwitch({ terminal, onChange }: { terminal: boolean; onChange: (terminal: boolean) => void }) {
  const item = (label: string, value: boolean) => (
    <Pressable
      accessibilityRole={SWITCH_ROLES.item}
      accessibilityLabel={label}
      accessibilityState={{ selected: terminal === value }}
      onPress={() => onChange(value)}
      style={({ pressed }) => [styles.tab, terminal === value && styles.tabOn, pressed && styles.pressed]}
    >
      <Text style={[styles.tabLabel, terminal === value && styles.tabLabelOn]}>{label}</Text>
    </Pressable>
  );
  return (
    <View style={styles.tabs} accessibilityRole={SWITCH_ROLES.list}>
      {item("Blocks", false)}
      {item("Terminal", true)}
    </View>
  );
}

/** Within this many points of the end counts as "at the bottom" (stick to new output). */
const BOTTOM_SLOP = 48;

export default function PaneScreen() {
  const { key, id } = useLocalSearchParams<{ key: string; id: string }>();
  const paneId = Number(id);
  const s = useApp();
  const insets = useSafeAreaInsets();
  const headerHeight = useHeaderHeight();
  const list = useRef<FlashListRef<Block>>(null);
  const atBottom = useRef(true);
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set());
  // this screen, as the pane's holder (the same pane may be pushed twice)
  const [holder] = useState(() => ({}));
  // the keyboard covers the bottom inset: no gap between it and the composer
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

  // Subscribe once connected and whenever this screen regains focus (a no-op when already
  // followed): a pane pushed on top (a notification tap) follows its own pane, and closes it
  // when it goes back, so this one must follow again.
  useFocusEffect(
    useCallback(() => {
      if (Number.isSafeInteger(paneId) && status === "open") fleet.openPane(key, paneId, holder);
    }, [key, paneId, status, holder]),
  );
  useEffect(() => () => fleet.closePane(key, paneId, holder), [key, paneId, holder]);

  // Notifications skip the pane on screen.
  useFocusEffect(
    useCallback(() => {
      fleet.setViewing({ host: key, pane: paneId });
      return () => fleet.setViewing(null);
    }, [key, paneId]),
  );

  const pane = s.pane?.host === key && s.pane.paneId === paneId ? s.pane : null;
  const state = pane?.state ?? null;
  const blocks = state?.blocks ?? [];
  const loc = useMemo(() => (host ? findPane(host.windows, paneId) : null), [host, paneId]);
  const title = loc ? paneTitle(loc) : "Pane";
  const running = runningCommand(loc?.pane ?? null, state);

  // Blocks | Terminal. Unpicked, the view follows the screen: Terminal while a full-screen
  // program shows (screen `auto`), Blocks after it. Terminal picked asks for the screen always
  // (`on`) and stays; Blocks picked while a program shows lasts until that screen ends.
  const screenFeature = hasFeature(host, "screen");
  const screen = pane?.screen ?? null;
  const [picked, setPicked] = useState<ViewPick>(null);
  const kept = keptPick(picked, screen, pane?.screenExpired);
  if (kept !== picked) setPicked(kept);
  const showTerminal = showsTerminal(picked, screen, screenFeature);
  const screenMode = picked === "terminal" ? "on" : "auto";
  const following = pane !== null;
  useEffect(() => {
    if (screenFeature && status === "open" && following) fleet.setScreenMode(key, paneId, screenMode);
  }, [screenFeature, status, following, key, paneId, screenMode]);
  const pick = (terminal: boolean) => setPicked(terminal ? "terminal" : screen?.alt ? "blocks" : null);

  const router = useRouter();
  const onActions = useCallback(
    (blockId: number) =>
      router.push({ pathname: "/block", params: { key, pane: String(paneId), block: String(blockId) } }),
    [router, key, paneId],
  );
  const onLoadEarlier = useCallback((blockId: number) => void fleet.loadEarlier(blockId), []);
  const canLoadEarlier = hasFeature(host, "output");
  const earlier = pane?.earlier;
  const extraData = useMemo(() => ({ expanded, earlier, canLoadEarlier }), [expanded, earlier, canLoadEarlier]);

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

  // With no room left, older blocks would be trimmed straight back off, so none are fetched.
  const header = canLoadOlder(state) ? (
    <View style={styles.older}>
      <Button
        label={pane?.loadingOlder ? "Loading earlier blocks…" : "Load earlier blocks"}
        disabled={pane?.loadingOlder}
        onPress={() => fleet.loadOlder()}
      />
    </View>
  ) : state?.hasOlder ? (
    <Text style={styles.capped}>Showing the most recent {Math.min(state.blocks.length, MAX_BLOCKS)} blocks</Text>
  ) : null;

  const operate = host ? canOperate(host) : false;

  return (
    <KeyboardAvoidingView
      style={styles.fill}
      // Android draws edge to edge (SDK 35+), so the window no longer resizes for the keyboard
      behavior="padding"
      keyboardVerticalOffset={headerHeight}
    >
      <Stack.Screen options={{ title: host ? `${title} · ${host.record.name}` : title }} />
      {host ? <StatusLine conn={host.conn} banner /> : null}
      {host?.notice ? <ErrorText style={styles.notice}>{host.notice}</ErrorText> : null}
      {screenFeature ? <ViewSwitch terminal={showTerminal} onChange={pick} /> : null}
      {host && showTerminal ? (
        <View style={[styles.fill, { paddingBottom: keyboard ? 0 : insets.bottom }]}>
          <TerminalView
            hostKey={key}
            paneId={paneId}
            screen={screen}
            operate={operate}
            canType={hasFeature(host, "text")}
            online={status === "open"}
            running={running !== null}
            onShowBlocks={() => pick(false)}
          />
        </View>
      ) : (
        <>
          {!host ? (
            <Text style={styles.empty}>This computer is not paired.</Text>
          ) : !state ? (
            <Text style={styles.empty} accessibilityLiveRegion="polite">
              {status === "open" ? "Loading blocks…" : "Waiting for the computer…"}
            </Text>
          ) : blocks.length === 0 ? (
            <Text style={styles.empty}>No commands in this pane yet.</Text>
          ) : null}
          <View style={styles.fill}>
            {/* Not inverted: `blocks` is oldest first, so the reading order (and swiping through
                with a screen reader) ends at the newest block, which the list starts scrolled to. */}
            <FlashList
              ref={list}
              data={blocks}
              keyExtractor={(b) => String(b.id)}
              extraData={extraData}
              renderItem={({ item }) => (
                <BlockView
                  block={item}
                  expanded={expanded.has(item.id)}
                  onToggle={toggle}
                  onActions={onActions}
                  earlier={earlier?.[item.id]}
                  canLoadEarlier={canLoadEarlier}
                  onLoadEarlier={onLoadEarlier}
                />
              )}
              ListHeaderComponent={header}
              maintainVisibleContentPosition={{
                startRenderingFromBottom: true,
                autoscrollToBottomThreshold: 0.1,
                animateAutoScrollToBottom: false,
              }}
              onScroll={onScroll}
              scrollEventThrottle={100}
              onContentSizeChange={onContentSizeChange}
              contentContainerStyle={styles.listContent}
              accessibilityLabel="Command blocks, newest at the bottom"
            />
          </View>
          {host && operate ? (
            <View
              style={{
                paddingBottom: keyboard ? 0 : insets.bottom,
                backgroundColor: colors.chrome,
              }}
            >
              <Composer hostKey={key} paneId={paneId} running={running} online={status === "open"} />
            </View>
          ) : host ? (
            <Text style={[styles.viewOnly, { paddingBottom: insets.bottom + 10 }]}>
              View only: this device cannot run commands here.
            </Text>
          ) : null}
        </>
      )}
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  listContent: { paddingVertical: 8 },
  older: { paddingHorizontal: 12, paddingVertical: 8 },
  capped: {
    color: colors.textDim,
    fontSize: 13,
    textAlign: "center",
    paddingHorizontal: 12,
    paddingVertical: 12,
  },
  empty: { color: colors.textMuted, fontSize: 14, padding: 16 },
  notice: { paddingHorizontal: 16, paddingVertical: 6 },
  viewOnly: {
    color: colors.textDim,
    fontSize: 13,
    textAlign: "center",
    paddingTop: 10,
    backgroundColor: colors.chrome,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.hairline,
  },
  tabs: {
    flexDirection: "row",
    gap: 4,
    padding: 4,
    marginHorizontal: 12,
    marginVertical: 6,
    borderRadius: radius.card,
    backgroundColor: colors.chrome,
  },
  tab: { flex: 1, minHeight: TOUCH, alignItems: "center", justifyContent: "center", borderRadius: radius.control },
  tabOn: { backgroundColor: colors.surface },
  tabLabel: { color: colors.textMuted, fontSize: 14, fontWeight: "500" },
  tabLabelOn: { color: colors.textBright },
  pressed: { opacity: 0.7 },
});
