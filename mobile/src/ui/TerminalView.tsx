import * as Clipboard from "expo-clipboard";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
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
import {
  clampZoom,
  FIT_DEBOUNCE_MS,
  FIT_REFRESH_MS,
  fitFontSize,
  fitSize,
  isDoubleTap,
  keptZoom,
  pinchZoom,
  touchDistance,
  zoomPercent,
  zoomStep,
} from "@/paneView";
import type { ScreenState } from "@/protocol/store";
import { RUN_BOLD, RUN_ITALIC, RUN_STRIKE, RUN_UNDERLINE, type ScreenRun } from "@/protocol/types";
import { authenticate, fleet, useApp } from "@/runtime";
import { useActionScope } from "./useActionScope";
import { DRAG_SLOP, DragVelocity, dragLines, dragScrollsPane, Fling, ScrollCoalescer } from "@/scroll";
import { clipboardImage, ImagePaste } from "@/upload";
import { rowWithPaste } from "@/controlKeys";
import { CELL_RATIO, fitFont, LINE_RATIO, rowPieces, rowSegments, screenDefaults, segmentColors, showsKeyRow, type Piece } from "@/terminal";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { ScreenSkeleton } from "./PaneLoading";
import { useOpenShell } from "./useOpenShell";
import { ErrorText, WarningText } from "./Notice";
import { colors, mono, monoBold, ui } from "./theme";

/** The terminal's default colours (a run's null fg / bg): terminal text on the pane's ground. */
const DEFAULTS = { fg: colors.fg, bg: colors.bg };
const PADDING = 8;
const PADDING_X = 6;

function decoration(flags: number): TextStyle["textDecorationLine"] {
  const u = (flags & RUN_UNDERLINE) !== 0;
  const s = (flags & RUN_STRIKE) !== 0;
  return u && s ? "underline line-through" : u ? "underline" : s ? "line-through" : undefined;
}

/** Within this many points of the end counts as "at the bottom": the screen follows its end. */
const BOTTOM_SLOP = 24;

/** One piece of a row on the cell grid: at its column, exactly its cells wide, a wide character centred in its two cells. */
function PieceView({
  piece,
  cell,
  fontSize,
  lineHeight,
  defaults,
}: {
  piece: Piece;
  cell: number;
  fontSize: number;
  lineHeight: number;
  defaults: { fg: string; bg: string };
}) {
  const { color, backgroundColor } = segmentColors(piece, defaults);
  const left = piece.col * cell;
  const width = piece.cells * cell;
  const text: TextStyle = {
    color,
    fontSize,
    lineHeight,
    height: lineHeight,
    // bold is a face of its own (Android takes no weight from a custom family)
    ...(piece.flags & RUN_BOLD ? { fontFamily: monoBold } : null),
    fontStyle: piece.flags & RUN_ITALIC ? "italic" : undefined,
    textDecorationLine: decoration(piece.flags),
    includeFontPadding: false,
  };
  return (
    <View style={{ position: "absolute", left, top: 0, width, height: lineHeight, backgroundColor }}>
      <Text
        style={[styles.cell, text, piece.wide ? { width, textAlign: "center" } : { width: width + 4 }]}
        numberOfLines={1}
        ellipsizeMode="clip"
        allowFontScaling={false}
      >
        {piece.text}
      </Text>
    </View>
  );
}

/**
 * One screen row on the cell grid (terminal.ts `rowPieces`): every piece sits at `col * cell`, so a
 * wide character (emoji, CJK) takes exactly two cells and the columns after it stay aligned
 * whatever the font does. Re-renders only when its runs (kept by reference across diffs), the
 * cursor, its rule or the size change.
 */
const Row = memo(function Row({
  runs,
  cursorCol,
  fontSize,
  width,
  rule,
  fg,
  bg,
}: {
  runs: ScreenRun[];
  cursorCol: number | null;
  fontSize: number;
  width: number;
  /** `rrggbb` of the 2 px rule drawn at the row's left edge (a block's rule), or null. */
  rule: string | null;
  fg: string;
  bg: string;
}) {
  const lineHeight = Math.round(fontSize * LINE_RATIO);
  const cell = fontSize * CELL_RATIO;
  const segments = rowSegments(runs, cursorCol);
  const spoken = segments.map((x) => x.text).join("");
  const blank = spoken.trim() === "";
  const defaults = { fg, bg };
  return (
    <View
      style={{ height: lineHeight, width }}
      // one element per row for screen readers; blank rows are skipped
      accessible={!blank}
      accessibilityLabel={blank ? undefined : spoken}
      accessibilityElementsHidden={blank}
      importantForAccessibility={blank ? "no-hide-descendants" : "yes"}
    >
      {rule ? <View style={[styles.rule, { height: lineHeight, backgroundColor: `#${rule}` }]} /> : null}
      {rowPieces(segments).map((piece) => (
        <PieceView key={piece.col} piece={piece} cell={cell} fontSize={fontSize} lineHeight={lineHeight} defaults={defaults} />
      ))}
    </View>
  );
});

/**
 * The screen as monospace rows, fitted to the width (7 to 14 pt) and scaled by the pinch zoom
 * (60 to 200 %), scrolling sideways when wider. `fitted`: the computer sizes the program to the
 * view (feature `fit`), so the font is the zoomed base size, not shrunk to the program's columns.
 */
function Grid({
  screen,
  zoom,
  fitted,
  onRowHeight,
}: {
  screen: ScreenState;
  zoom: number;
  fitted: boolean;
  /** Told each row's height in points (a drag's lines are counted in it). */
  onRowHeight?: (height: number) => void;
}) {
  const [width, setWidth] = useState(0);
  const onLayout = (e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width);
  const fit = fitted ? { fontSize: fitFontSize(1), scroll: false } : fitFont(width - PADDING_X * 2, screen.cols);
  const fontSize = fitted ? fitFontSize(zoom) : Math.round(fit.fontSize * zoom * 10) / 10;
  const rowHeight = Math.round(fontSize * LINE_RATIO);
  useEffect(() => {
    onRowHeight?.(rowHeight);
  }, [onRowHeight, rowHeight]);
  const rowWidth = Math.ceil(screen.cols * fontSize * CELL_RATIO) + 2;
  const defaults = screenDefaults(screen, DEFAULTS);
  const scroll = fit.scroll || rowWidth > width - PADDING_X * 2;
  const [cursorRow, cursorCol] = screen.cursor ?? [-1, -1];
  const rows = (
    <View>
      {screen.lines.map((runs, r) => (
        <Row
          key={r}
          runs={runs}
          cursorCol={r === cursorRow ? cursorCol : null}
          fontSize={fontSize}
          width={rowWidth}
          rule={screen.rules.get(r) ?? null}
          fg={defaults.fg}
          bg={defaults.bg}
        />
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
 * less, htop), and on an operate device the keys and the keyboard for it. Typed text goes out as `text` (batched per frame, `keyboard.ts`), keys as
 * `keys`; the control-key row works as in the composer. A key that submits a line asks for
 * authentication like Run (lock.ts): at the pane's prompt every time, while a program runs once
 * per view while the app stays in the foreground. On a gateway with `fit` (and a device that may
 * operate) the view asks the computer to size the program to its own width and height at the zoomed
 * font, re-sending every 10 s and releasing when it closes.
 */
export function TerminalView({
  hostKey,
  paneId,
  screen,
  operate,
  canType,
  online,
  running,
  bare,
  canFit,
  canScroll,
  canUpload,
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
  /** The pane's command line is drawn under the screen (the Pane board): no keys or keyboard here. */
  bare?: boolean;
  /** The gateway sizes programs to the view (feature `fit`). */
  canFit?: boolean;
  /** A drag scrolls the pane on the computer (feature `scroll`). */
  canScroll?: boolean;
  /** Paste sends the clipboard's image (feature `upload`). */
  canUpload?: boolean;
}) {
  const s = useApp();
  const scope = useActionScope(`${hostKey}:${paneId}`);
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
        const current = scope.capture();
        const { settings, running: busy } = live.current;
        if (!hold.needs(keys, settings, busy)) return null;
        return authenticate(`Send ${keys.join(" ")}`).then((auth) => {
          if (!current()) return false;
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
  }, [hold, scope]);
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
  // the row's Paste: an image on the clipboard (a screenshot) goes to the computer, which pastes
  // its saved file's path into the program (Claude Code and Codex attach it); text is typed into
  // the program as the keyboard would. One image at a time: a Paste while one goes out says so.
  const [sendingImage, setSendingImage] = useState(false);
  const [images] = useState(() => new ImagePaste(setSendingImage));
  const onPaste = async () => {
    const current = scope.capture();
    if (canUpload) {
      const out = await images.paste(
        () => clipboardImage(Clipboard),
        (b64) => fleet.uploadImage(hostKey, paneId, b64),
      );
      if (!current()) return;
      if (out.kind === "sending") return setError("An image is still going to the computer.");
      if (out.kind === "error") return setError(out.message);
      if (out.kind === "sent") return setError(null);
    }
    if (!canType) return;
    const text = await Clipboard.getStringAsync();
    if (current() && text) push(...textToActions(text));
  };
  const onOpen = useOpenShell(hostKey, paneId, setError);

  const typing = operate && canType && !bare;

  // The screen follows its end: when the view shrinks (the keyboard opens) or the screen grows,
  // the last rows stay in sight unless the user scrolled up (judged when a drag or fling ends, not
  // on every scroll event: the view shrinking under the keyboard also reports a scroll).
  const scroller = useRef<ScrollView>(null);
  const stick = useRef(true);
  const toEnd = () => {
    if (!stick.current) return;
    scroller.current?.scrollToEnd({ animated: false });
    // again once the layout (and the keyboard's animation) has settled
    setTimeout(() => {
      if (stick.current) scroller.current?.scrollToEnd({ animated: false });
    }, 350);
  };
  const onScroll = (e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    stick.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - BOTTOM_SLOP;
    trackLocal(e);
  };

  // A vertical drag scrolls the pane on the computer (feature `scroll`: Claude Code, vim, less,
  // a shell's scrollback), its lines counted in rows and coalesced (scroll.ts); while the view
  // itself scrolls (zoomed in), it does first, the computer's pane past its edges.
  const local = useRef({ y: 0, viewH: 0, contentH: 0 });
  const [overflow, setOverflow] = useState(false);
  const measureLocal = () => {
    const { viewH, contentH } = local.current;
    setOverflow(contentH - viewH > 1);
  };
  const trackLocal = (e: NativeSyntheticEvent<NativeScrollEvent>) => {
    local.current.y = e.nativeEvent.contentOffset.y;
  };
  const rowHeight = useRef(16);
  const onRowHeight = useCallback((h: number) => {
    rowHeight.current = h;
  }, []);
  const remote = useRef<ScrollCoalescer | null>(null);
  // a flick carries on after the finger lifts (scroll.ts Fling)
  const velocity = useRef(new DragVelocity());
  const fling = useRef<Fling | null>(null);
  useEffect(() => {
    const timers = {
      now: () => Date.now(),
      schedule: (fn: () => void, ms: number) => setTimeout(fn, ms),
      unschedule: (h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>),
    };
    const c = new ScrollCoalescer({
      send: (lines) =>
        void fleet.sendScroll(live.current.hostKey, live.current.paneId, lines).then((r) => {
          // the rate only slows it (an older desktop allows fewer): sent again, slower
          if (!r.ok && r.code === "rate") c.refused(lines);
          // any other refusal says why once
          else if (!r.ok) setError(r.message);
        }),
      ...timers,
    });
    const f = new Fling({ add: (lines) => c.add(lines), done: () => c.flush(), ...timers });
    remote.current = c;
    fling.current = f;
    return () => {
      remote.current = null;
      fling.current = null;
      f.stop();
      c.cancel();
    };
  }, []);
  const drag = useRef<{ x: number; y: number; lastY: number; vertical: boolean | null } | null>(null);
  const dragMove = (x: number, y: number) => {
    const d = drag.current;
    if (!d) return;
    if (d.vertical === null) {
      if (Math.hypot(x - d.x, y - d.y) <= DRAG_SLOP) return;
      d.vertical = Math.abs(y - d.y) > Math.abs(x - d.x);
      d.lastY = y;
      return;
    }
    const dy = y - d.lastY;
    d.lastY = y;
    if (!d.vertical || !canScroll || !online || dy === 0) return;
    const { y: at, viewH, contentH } = local.current;
    const edges = { overflow: contentH - viewH > 1, atTop: at <= 1, atBottom: at + viewH >= contentH - 1 };
    if (dragScrollsPane(dy, edges)) {
      const lines = dragLines(dy, rowHeight.current);
      velocity.current.add(lines, Date.now());
      remote.current?.add(lines);
    }
  };

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
    // a touch stops a fling where it is
    fling.current?.stop();
    velocity.current.clear();
    if (t.length === 2) {
      pinch.current = { distance: touchDistance(t[0]!, t[1]!), zoom };
      tap.current = null;
      drag.current = null;
      remote.current?.cancel();
      setPinching(true);
    } else if (t.length === 1) {
      tap.current = { x: t[0]!.pageX, y: t[0]!.pageY, moved: false };
      drag.current = { x: t[0]!.pageX, y: t[0]!.pageY, lastY: t[0]!.pageY, vertical: null };
    }
  };
  const onTouchMove = (e: GestureResponderEvent) => {
    const t = e.nativeEvent.touches;
    if (t.length === 2 && pinch.current) setZoom(pinchZoom(pinch.current, touchDistance(t[0]!, t[1]!)));
    else if (t.length === 1) {
      if (tap.current && Math.hypot(t[0]!.pageX - tap.current.x, t[0]!.pageY - tap.current.y) > DRAG_SLOP) tap.current.moved = true;
      dragMove(t[0]!.pageX, t[0]!.pageY);
    }
  };
  const onTouchEnd = (e: GestureResponderEvent) => {
    const left = e.nativeEvent.touches.length;
    if (left === 0) {
      const flicked = drag.current?.vertical === true && !pinch.current;
      drag.current = null;
      // a flick goes on (and flushes when it stops); otherwise what waits goes out now
      if (!(flicked && fling.current?.start(velocity.current.speed(Date.now())))) remote.current?.flush();
      velocity.current.clear();
    }
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
  // The scrolling area's size, less the zoom row, is what a fitted program may fill.
  const [area, setArea] = useState({ width: 0, height: 0 });
  const [zoomHeight, setZoomHeight] = useState(0);
  const onArea = (e: LayoutChangeEvent) => {
    const { width, height } = e.nativeEvent.layout;
    setArea((a) => (a.width === width && a.height === height ? a : { width, height }));
    local.current.viewH = height;
    measureLocal();
    toEnd();
  };
  const onContent = (_w: number, h: number) => {
    local.current.contentH = h;
    measureLocal();
    toEnd();
  };
  // with the computer's pane taking the drag, the view moves only when it has more to show
  const localScroll = !pinching && (!canScroll || overflow);
  const fitting = !!canFit && operate && online;
  const size = fitting ? fitSize(area.width - PADDING_X * 2, area.height - zoomHeight - PADDING * 2, fitFontSize(zoom)) : null;
  const cols = size?.cols;
  const rows = size?.rows;
  // sent once the size has settled, and again while held: the computer drops a fit not refreshed
  useEffect(() => {
    if (cols === undefined || rows === undefined) return;
    const send = () => void fleet.sendFit(hostKey, paneId, { cols, rows });
    let refresh: ReturnType<typeof setInterval> | undefined;
    const settle = setTimeout(() => {
      send();
      refresh = setInterval(send, FIT_REFRESH_MS);
    }, FIT_DEBOUNCE_MS);
    return () => {
      clearTimeout(settle);
      clearInterval(refresh);
    };
  }, [cols, rows, hostKey, paneId]);
  // released when the view closes, the pane changes or the computer goes away
  const held = cols !== undefined;
  useEffect(() => {
    if (!held) return;
    return () => void fleet.sendFit(hostKey, paneId, null);
  }, [held, hostKey, paneId]);
  const zoomRow = (
    <View style={styles.zoomRow} onLayout={(e) => setZoomHeight(e.nativeEvent.layout.height)}>
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
    <Grid screen={screen} zoom={zoom} fitted={held} onRowHeight={onRowHeight} />
  ) : (
    // the pane's steps card (PaneLoading) says what is awaited; these are the rows to come
    <ScreenSkeleton />
  );

  return (
    <View style={[styles.fill, { backgroundColor: screenDefaults(screen, DEFAULTS).bg }]} onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd} onTouchCancel={onTouchEnd}>
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
          <ScrollView ref={scroller} style={styles.fill} accessibilityLabel={label} scrollEnabled={localScroll} bounces={!canScroll} onLayout={onArea} onScroll={trackLocal} scrollEventThrottle={16} onScrollEndDrag={onScroll} onMomentumScrollEnd={onScroll} onContentSizeChange={onContent}>
            {screen ? zoomRow : null}
            {/* not one element for screen readers: they read the rows */}
            <Pressable onPress={() => capture.current?.focus()} accessible={false}>
              {grid}
            </Pressable>
          </ScrollView>
        </KeyCaptureView>
      ) : (
        <ScrollView ref={scroller} style={styles.fill} accessibilityLabel={label} scrollEnabled={localScroll} bounces={!canScroll} onLayout={onArea} onScroll={trackLocal} scrollEventThrottle={16} onScrollEndDrag={onScroll} onMomentumScrollEnd={onScroll} onContentSizeChange={onContent}>
          {screen ? zoomRow : null}
          {grid}
        </ScrollView>
      )}
      {/* without the key bar (the message input below, or view only), a refused scroll or paste says why here */}
      {error && (bare || !operate) ? <ErrorText style={styles.bareError}>{error}</ErrorText> : null}
      {bare ? null : operate ? (
        <View style={styles.bar} accessibilityLabel="Keys">
          {atPrompt ? (
            <WarningText>This pane is at its prompt: typed text goes to programs only.</WarningText>
          ) : null}
          {error ? <ErrorText>{error}</ErrorText> : null}
          {sendingImage ? (
            <Text style={styles.oldRift} accessibilityLiveRegion="polite">
              Sending the image to the computer…
            </Text>
          ) : null}
          {!canType ? (
            <Text style={styles.oldRift}>This computer's Rift takes keys from the row only; update it to type here.</Text>
          ) : null}
          {/* a full-screen program needs its keys: the row shows here whatever Settings says */}
          {showsKeyRow(s.settings.showKeys, true) ? (
            <ControlRow
              // a screenshot for the program needs Paste: on the row whatever Settings › Keys holds
              ids={rowWithPaste(s.settings.controlKeys, !!canUpload)}
              sticky={sticky}
              disabled={!online}
              onSticky={setSticky}
              onKey={onRowKey}
              onOpen={onOpen}
              onPaste={canType || canUpload ? () => void onPaste() : undefined}
              // the last key: hide the keyboard, or open it (a tap on the screen opens it too)
              onHideKeyboard={typing && focused ? () => capture.current?.blur() : undefined}
              onShowKeyboard={typing && !focused ? () => capture.current?.focus() : undefined}
              wrap
            />
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
  cell: { fontFamily: mono, color: DEFAULTS.fg },
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
  // the FullScreen board: the keys on the chrome ground
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
  bareError: { paddingHorizontal: 12, paddingVertical: 6, backgroundColor: colors.chrome },
  // a block's rule: 2 px in the 6 pt gutter left of the row
  rule: { position: "absolute", left: -4, top: 0, width: 2 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
