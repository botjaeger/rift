import * as Clipboard from "expo-clipboard";
import { memo, useEffect, useRef, useState } from "react";
import {
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type TextStyle,
} from "react-native";
import { KeyCaptureView, type KeyCaptureHandle, type KeyEvent } from "../../modules/rift-keys";
import {
  applySticky,
  keyEventToAction,
  stickyKey,
  terminalInput,
  textToActions,
  type InputAction,
  type InputBatcher,
} from "@/keyboard";
import { SubmitAuthHold } from "@/lock";
import { clampZoom, isDoubleTap, keptZoom, pinchZoom, touchDistance, zoomPercent, zoomStep } from "@/paneView";
import type { ScreenState } from "@/protocol/store";
import { RUN_BOLD, RUN_ITALIC, RUN_STRIKE, RUN_UNDERLINE, type ScreenRun } from "@/protocol/types";
import { authenticate, fleet, useApp } from "@/runtime";
import { CELL_RATIO, fitFont, LINE_RATIO, rowSegments, segmentColors, showsKeyRow } from "@/terminal";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { useOpenShell } from "./useOpenShell";
import { Icon } from "./Icon";
import { ErrorText, WarningText } from "./Notice";
import { colors, mono, monoBold, TOUCH, ui } from "./theme";

/** The terminal's default colours (a run's null fg / bg): terminal text on the pane's ground. */
const DEFAULTS = { fg: colors.fg, bg: colors.bg };
const PADDING = 8;
const PADDING_X = 6;

function decoration(flags: number): TextStyle["textDecorationLine"] {
  const u = (flags & RUN_UNDERLINE) !== 0;
  const s = (flags & RUN_STRIKE) !== 0;
  return u && s ? "underline line-through" : u ? "underline" : s ? "line-through" : undefined;
}

/** One screen row; re-renders only when its runs (kept by reference across diffs) or the cursor change. */
const Row = memo(function Row({
  runs,
  cursorCol,
  fontSize,
  width,
}: {
  runs: ScreenRun[];
  cursorCol: number | null;
  fontSize: number;
  width: number;
}) {
  const lineHeight = Math.round(fontSize * LINE_RATIO);
  const segments = rowSegments(runs, cursorCol);
  const blank = segments.every((s) => s.text.trim() === "");
  return (
    <Text
      style={[styles.row, { fontSize, lineHeight, height: lineHeight, width }]}
      numberOfLines={1}
      ellipsizeMode="clip"
      // blank rows are skipped by screen readers
      accessibilityElementsHidden={blank}
      importantForAccessibility={blank ? "no-hide-descendants" : "auto"}
    >
      {segments.length === 0
        ? " "
        : segments.map((s, i) => (
            <Text
              key={i}
              style={{
                ...segmentColors(s, DEFAULTS),
                // bold is a face of its own (Android takes no weight from a custom family)
                fontFamily: s.flags & RUN_BOLD ? monoBold : mono,
                fontStyle: s.flags & RUN_ITALIC ? "italic" : undefined,
                textDecorationLine: decoration(s.flags),
              }}
            >
              {s.text}
            </Text>
          ))}
    </Text>
  );
});

/**
 * The screen as monospace rows, fitted to the width (7 to 14 pt) and scaled by the pinch zoom
 * (60 to 200 %), scrolling sideways when wider.
 */
function Grid({ screen, zoom }: { screen: ScreenState; zoom: number }) {
  const [width, setWidth] = useState(0);
  const onLayout = (e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width);
  const fit = fitFont(width - PADDING_X * 2, screen.cols);
  const fontSize = Math.round(fit.fontSize * zoom * 10) / 10;
  const rowWidth = Math.ceil(screen.cols * fontSize * CELL_RATIO) + 2;
  const scroll = fit.scroll || rowWidth > width - PADDING_X * 2;
  const [cursorRow, cursorCol] = screen.cursor ?? [-1, -1];
  const rows = (
    <View>
      {screen.lines.map((runs, r) => (
        <Row key={r} runs={runs} cursorCol={r === cursorRow ? cursorCol : null} fontSize={fontSize} width={rowWidth} />
      ))}
    </View>
  );
  return (
    <View style={styles.gridWrap} onLayout={onLayout}>
      {width === 0 ? null : scroll ? (
        <ScrollView horizontal contentContainerStyle={styles.gridPad}>
          {rows}
        </ScrollView>
      ) : (
        <View style={styles.gridPad}>{rows}</View>
      )}
    </View>
  );
}

/**
 * The pane while a full-screen program runs (the FullScreen board): the gateway's screen (vim,
 * less, htop), a note, and on an operate device the keys and the keyboard for it. Typed text goes out as `text` (batched per frame, `keyboard.ts`), keys as
 * `keys`; the control-key row works as in the composer. A key that submits a line asks for
 * authentication like Run (lock.ts): at the pane's prompt every time, while a program runs once
 * per view while the app stays in the foreground.
 */
export function TerminalView({
  hostKey,
  paneId,
  screen,
  operate,
  canType,
  online,
  running,
  program,
  pinned,
  bare,
}: {
  hostKey: string;
  paneId: number;
  screen: ScreenState | null;
  operate: boolean;
  /** The gateway takes `text` (feature `text`). */
  canType: boolean;
  online: boolean;
  /** The pane runs a program (not at its prompt). */
  running: boolean;
  /** The full-screen program's name ("htop"), for the note. */
  program: string;
  /** Shown by the header's Terminal view switch, not by a full-screen program. */
  pinned?: boolean;
  /** The pane's command line is drawn under the screen (the Pane board): no note, keys or keyboard here. */
  bare?: boolean;
}) {
  const s = useApp();
  const capture = useRef<KeyCaptureHandle>(null);
  const [focused, setFocused] = useState(false);
  const [sticky, setSticky] = useState<Sticky>(NO_STICKY);
  const [error, setError] = useState<string | null>(null);
  const [atPrompt, setAtPrompt] = useState(false);
  const [hold] = useState(() => new SubmitAuthHold());
  // everything below reads the latest render's values from these refs
  const live = useRef({ settings: s.settings, online, running, hostKey, paneId });
  useEffect(() => {
    live.current = { settings: s.settings, online, running, hostKey, paneId };
  });

  // Input waits here for the next frame, the gateway's rate and any authentication before a
  // submit key (keyboard.ts), and goes out in order without waiting for answers; what is
  // queued goes out when the view closes.
  const batcher = useRef<InputBatcher | null>(null);
  useEffect(() => {
    const b = terminalInput({
      sendKeys: (keys) => fleet.sendKeys(live.current.hostKey, live.current.paneId, keys),
      sendText: (text) => fleet.sendText(live.current.hostKey, live.current.paneId, text),
      online: () => live.current.online,
      delay: () => fleet.inputDelay(live.current.hostKey),
      authorize: (keys) => {
        const { settings, running: busy } = live.current;
        if (!hold.needs(keys, settings, busy)) return null;
        return authenticate(`Send ${keys.join(" ")}`).then((auth) => {
          if (!auth.ok) {
            if (auth.message !== "Cancelled.") setError(auth.message);
            return false;
          }
          hold.grant(live.current.running);
          return true;
        });
      },
      report: (r) => {
        if (r === "ok") {
          setError(null);
          setAtPrompt(false);
        } else if (r === "busy") {
          setAtPrompt(true);
        } else {
          setError(r.error);
        }
      },
      schedule: (fn, delay) => {
        if (delay > 0) setTimeout(fn, delay);
        else requestAnimationFrame(fn);
      },
      now: () => Date.now(),
    });
    batcher.current = b;
    return () => {
      batcher.current = null;
      b.flush(true);
    };
  }, [hold]);
  const push = (...actions: InputAction[]) => batcher.current?.push(...actions);

  // The authentication holds until the view closes or the app leaves the foreground.
  useEffect(() => {
    const sub = AppState.addEventListener("change", (state) => {
      if (state !== "active") hold.end();
    });
    return () => {
      sub.remove();
      hold.end();
    };
  }, [hold]);

  const takeSticky = (): Sticky => {
    const cur = sticky;
    if (cur.ctrl || cur.alt || cur.shift) setSticky(NO_STICKY);
    return cur;
  };

  // One character typed with sticky ctrl / alt is that key (ctrl+r, ctrl+enter); text the
  // modifier cannot apply to goes out as typed, and the sticky is used up either way.
  const typed = (text: string) => {
    const mods = takeSticky();
    const key = [...text].length === 1 ? stickyKey(text, mods) : null;
    push(...(key ? [{ keys: [key] }] : textToActions(text)));
  };

  const onKey = (e: KeyEvent) => {
    const action = keyEventToAction(e);
    if (!action) return;
    if ("text" in action) typed(action.text);
    else {
      const mods = takeSticky();
      push({ keys: action.keys.map((k) => applySticky(k, mods)) });
    }
  };

  const onRowKey = (id: string) => push({ keys: [applySticky(id, takeSticky())] });
  // the row's Paste: the phone's clipboard, typed into the program as the keyboard would
  const onPaste = () =>
    void Clipboard.getStringAsync().then((text) => {
      if (text) push(...textToActions(text));
    });
  const onOpen = useOpenShell(hostKey, paneId, setError);

  const typing = operate && canType && !bare;

  // Pinch to zoom, double-tap to reset (the Pane board); kept for the app's run
  const [zoom, setZoomState] = useState(keptZoom.value);
  const setZoom = (z: number) => {
    const c = clampZoom(z);
    keptZoom.value = c;
    setZoomState(c);
  };
  const pinch = useRef<{ distance: number; zoom: number } | null>(null);
  const [pinching, setPinching] = useState(false);
  const tap = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const lastTap = useRef<number | null>(null);
  const onTouchStart = (e: GestureResponderEvent) => {
    const t = e.nativeEvent.touches;
    if (t.length === 2) {
      pinch.current = { distance: touchDistance(t[0]!, t[1]!), zoom };
      tap.current = null;
      setPinching(true);
    } else if (t.length === 1) tap.current = { x: t[0]!.pageX, y: t[0]!.pageY, moved: false };
  };
  const onTouchMove = (e: GestureResponderEvent) => {
    const t = e.nativeEvent.touches;
    if (t.length === 2 && pinch.current) setZoom(pinchZoom(pinch.current, touchDistance(t[0]!, t[1]!)));
    else if (t.length === 1 && tap.current && Math.hypot(t[0]!.pageX - tap.current.x, t[0]!.pageY - tap.current.y) > 10) tap.current.moved = true;
  };
  const onTouchEnd = (e: GestureResponderEvent) => {
    const left = e.nativeEvent.touches.length;
    if (pinch.current && left < 2) {
      pinch.current = null;
      lastTap.current = null;
      setPinching(false);
    } else if (left === 0 && tap.current && !tap.current.moved) {
      const now = Date.now();
      if (isDoubleTap(lastTap.current, now)) {
        lastTap.current = null;
        setZoom(1);
      } else lastTap.current = now;
    }
    if (left === 0) tap.current = null;
  };
  const zoomRow = (
    <View style={styles.zoomRow}>
      <Text style={styles.zoomHint}>Pinch to zoom · double-tap to reset</Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Smaller text"
        accessibilityState={{ disabled: zoom <= 0.6 }}
        onPress={() => setZoom(zoomStep(zoom, -1))}
        hitSlop={6}
        style={({ pressed }) => [styles.zoomButton, pressed && styles.pressed]}
      >
        <Text style={styles.zoomGlyph}>−</Text>
      </Pressable>
      <Text style={styles.zoomValue} accessibilityLabel={`Zoom ${zoomPercent(zoom)}`}>
        {zoomPercent(zoom)}
      </Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Larger text"
        accessibilityState={{ disabled: zoom >= 2 }}
        onPress={() => setZoom(zoomStep(zoom, 1))}
        hitSlop={6}
        style={({ pressed }) => [styles.zoomButton, pressed && styles.pressed]}
      >
        <Text style={styles.zoomGlyph}>+</Text>
      </Pressable>
    </View>
  );
  const label = screen
    ? `Terminal screen, ${screen.cols} columns by ${screen.rows} rows${screen.alt ? ", full-screen program" : ""}${screen.stale ? ", not up to date" : ""}`
    : "Terminal screen";

  const grid = screen ? (
    <Grid screen={screen} zoom={zoom} />
  ) : (
    <Text style={styles.empty} accessibilityLiveRegion="polite">
      {online ? "Waiting for the screen…" : "Waiting for the computer…"}
    </Text>
  );

  return (
    <View style={styles.fill} onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd} onTouchCancel={onTouchEnd}>
      {typing ? (
        // the label stays on the ScrollView: on the capture view TalkBack would read the
        // screen as one element instead of its rows
        <KeyCaptureView
          ref={capture}
          mode="terminal"
          onText={(e) => typed(e.text)}
          onKey={onKey}
          onFocusChange={setFocused}
          style={styles.fill}
        >
          <ScrollView style={styles.fill} accessibilityLabel={label} scrollEnabled={!pinching}>
            {zoomRow}
            {/* not one element for screen readers: they read the rows */}
            <Pressable onPress={() => capture.current?.focus()} accessible={false}>
              {grid}
            </Pressable>
          </ScrollView>
        </KeyCaptureView>
      ) : (
        <ScrollView style={styles.fill} accessibilityLabel={label} scrollEnabled={!pinching}>
          {zoomRow}
          {grid}
        </ScrollView>
      )}
      {bare ? null : (
        <Text style={styles.note}>
          {pinned
            ? "Terminal view: the pane's screen as it is. The terminal button shows blocks again."
            : operate
              ? `Keys go straight to ${program}. When it quits, the pane shows blocks again.`
              : `${program} is full screen. When it quits, the pane shows blocks again.`}
        </Text>
      )}
      {bare ? null : operate ? (
        <View style={styles.bar} accessibilityLabel="Keys">
          {atPrompt ? (
            <WarningText>This pane is at its prompt: typed text goes to programs only.</WarningText>
          ) : null}
          {error ? <ErrorText>{error}</ErrorText> : null}
          {!canType ? (
            <Text style={styles.oldRift}>This computer's Rift takes keys from the row only; update it to type here.</Text>
          ) : null}
          {/* a full-screen program needs its keys: the row shows here whatever Settings says */}
          {showsKeyRow(s.settings.showKeys, true) ? (
            <ControlRow
              ids={s.settings.controlKeys}
              sticky={sticky}
              disabled={!online}
              onSticky={setSticky}
              onKey={onRowKey}
              onOpen={onOpen}
              onPaste={canType ? onPaste : undefined}
              wrap
            />
          ) : null}
          {typing ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={focused ? "Hide keyboard" : "Show keyboard"}
              accessibilityState={{ disabled: !online, expanded: focused }}
              disabled={!online}
              onPress={() => (focused ? capture.current?.blur() : capture.current?.focus())}
              style={({ pressed }) => [styles.kbd, !online && styles.disabled, pressed && styles.pressed]}
            >
              <Icon name="keyboard" color={colors.text} />
              <Text style={styles.kbdLabel}>{focused ? "Hide keyboard" : "Show keyboard"}</Text>
            </Pressable>
          ) : null}
        </View>
      ) : (
        <Text style={styles.viewOnly}>View only: this device cannot type here.</Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  gridWrap: { flex: 1 },
  gridPad: { paddingVertical: PADDING, paddingHorizontal: PADDING_X },
  row: { fontFamily: mono, color: DEFAULTS.fg },
  empty: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, padding: 16 },
  // as the blocks view's view-only line
  viewOnly: {
    fontFamily: ui(400),
    color: colors.textDim,
    fontSize: 13,
    textAlign: "center",
    paddingVertical: 10,
    backgroundColor: colors.chrome,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.hairline,
  },
  // the FullScreen board: a note on a surface strip, then the keys on the chrome ground
  note: {
    fontFamily: ui(400),
    color: colors.textMuted,
    fontSize: 12,
    lineHeight: 17.4,
    paddingVertical: 8,
    paddingHorizontal: 16,
    backgroundColor: colors.surface,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  bar: {
    backgroundColor: colors.chrome,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
    paddingHorizontal: 12,
    paddingTop: 8,
    paddingBottom: 8,
    gap: 8,
  },
  // the Pane board's zoom row: the hint, − and + 32 pt, the percentage between
  zoomRow: { flexDirection: "row", alignItems: "center", gap: 8, paddingTop: 10, paddingHorizontal: 12, paddingBottom: 2 },
  zoomHint: { flex: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  zoomButton: { width: 32, height: 32, borderRadius: 16, borderWidth: 1, borderColor: colors.borderStrong, alignItems: "center", justifyContent: "center" },
  zoomGlyph: { fontFamily: ui(400), color: colors.text, fontSize: 16 },
  zoomValue: { fontFamily: mono, color: colors.text, fontSize: 12, minWidth: 44, textAlign: "center" },
  oldRift: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  kbd: {
    height: TOUCH,
    flexDirection: "row",
    gap: 8,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: TOUCH / 2,
    borderWidth: 1,
    borderColor: colors.borderStrong,
  },
  kbdLabel: { fontFamily: ui(400), color: colors.text, fontSize: 14 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
