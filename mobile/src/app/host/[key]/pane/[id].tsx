import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { Stack, useFocusEffect, useLocalSearchParams } from "expo-router";
import { useHeaderHeight } from "expo-router/react-navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Keyboard,
  KeyboardAvoidingView,
  Platform,
  StyleSheet,
  Text,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { canLoadOlder, MAX_BLOCKS } from "@/protocol/store";
import type { Block } from "@/protocol/types";
import { canOperate } from "@/fleet";
import { fleet, useApp } from "@/runtime";
import { findPane, paneTitle, runningCommand } from "@/topology";
import { BlockView } from "@/ui/BlockView";
import { Button } from "@/ui/Button";
import { Composer } from "@/ui/Composer";
import { ErrorText } from "@/ui/Notice";
import { StatusLine } from "@/ui/StatusLine";
import { colors } from "@/ui/theme";

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
          extraData={expanded}
          renderItem={({ item }) => <BlockView block={item} expanded={expanded.has(item.id)} onToggle={toggle} />}
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
});
