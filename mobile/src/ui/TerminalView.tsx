import * as Clipboard from "expo-clipboard";
import { useFocusEffect } from "expo-router";
import { memo, useCallback, useEffect, useRef, useState, type RefObject } from "react";
import {
  AccessibilityInfo,
  AppState,
  Keyboard,
  Platform,
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
import { clipboardFiles, KeyCaptureView, type KeyCaptureHandle, type KeyEvent, type PastedFile } from "../../modules/rift-keys";
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
import { draftKey } from "@/fleet";
import { nativeAttached, pending, PENDING_FULL, readClipboard, type AttachRules, type Attached } from "@/pending";
import { programSend } from "@/paneView";
import { barStaysLow, promptText, textToCopy } from "@/selection";
import { SelectionBar } from "./SelectionBar";
import * as haptics from "@/haptics";
import { fullScreenRow, rowWithPaste } from "@/controlKeys";
import { CELL_RATIO, textRoom, cellAt, cursorScrollY, drawnText, fitArea, fitFont, lastTextRow, lineAt, ordered, screenSpan, selectionText, wordAt, wordStep, type Cell, LINE_RATIO, rowPieces, rowSegments, screenDefaults, segmentColors, showsKeyRow, type Piece } from "@/terminal";
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

/** A selection handle: a 2 pt stem the row's height with a 12 pt knob (above the start, below the end), and how close a finger must come to take it. */
const STEM = 2;
const KNOB = 12;
const HANDLE_REACH = 44;
/** The loupe over a dragged handle: its size, how much larger it draws the row, and its gap above the finger's row. */
const LOUPE_W = 150;
const LOUPE_SCALE = 1.6;
const LOUPE_GAP = 28;
/** Hold this long, still, and a touch starts a selection. */
const SELECT_HOLD_MS = 450;
/** How long the zoom controls stay after a pinch. */
const ZOOM_SHOWN_MS = 2500;

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
        style={[styles.cell, text, piece.wide ? { width, textAlign: "center" } : { width: textRoom(width), maxWidth: textRoom(width) }]}
        numberOfLines={1}
        ellipsizeMode="clip"
        allowFontScaling={false}
      >
        {drawnText(piece.text)}
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

/** Where a selection's handles take a finger, in the rows' coordinates: the centre of each knob. */
export function handleSpots(a: Cell, b: Cell, cell: number, rowHeight: number) {
  const [from, to] = ordered(a, b);
  return {
    from,
    to,
    start: { x: from.col * cell, y: from.row * rowHeight - KNOB / 2 },
    end: { x: (to.col + 1) * cell, y: (to.row + 1) * rowHeight + KNOB / 2 },
  };
}

/**
 * The selected cells' ground and a handle at each end (a stem the row's height, the start's knob
 * above it, the end's below): drawn only, the view's touches move them.
 */
function SelectionMarks({ a, b, cols, cell, rowHeight }: { a: Cell; b: Cell; cols: number; cell: number; rowHeight: number }) {
  const { from, to, start, end } = handleSpots(a, b, cell, rowHeight);
  const bands = [];
  for (let r = from.row; r <= to.row; r++) {
    const left = r === from.row ? from.col : 0;
    const right = r === to.row ? to.col : cols - 1;
    bands.push(<View key={r} style={[styles.band, { top: r * rowHeight, height: rowHeight, left: left * cell, width: (right - left + 1) * cell }]} />);
  }
  return (
    <View pointerEvents="none" style={StyleSheet.absoluteFill} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {bands}
      <View style={[styles.stem, { left: start.x - STEM / 2, top: from.row * rowHeight, height: rowHeight }]} />
      <View style={[styles.knob, { left: start.x - KNOB / 2, top: start.y - KNOB / 2 }]} />
      <View style={[styles.stem, { left: end.x - STEM / 2, top: to.row * rowHeight, height: rowHeight }]} />
      <View style={[styles.knob, { left: end.x - KNOB / 2, top: end.y - KNOB / 2 }]} />
    </View>
  );
}

/**
 * The loupe over a dragged handle: the finger's row drawn larger, centred on the handle's cell with
 * a caret there, above the row (below it on the first rows) so the finger never hides what it picks.
 */
function Loupe({ row, edge, screen, fontSize, rowHeight, cell, fg, bg }: { row: number; edge: number; screen: ScreenState; fontSize: number; rowHeight: number; cell: number; fg: string; bg: string }) {
  const big = fontSize * LOUPE_SCALE;
  const bigRow = Math.round(big * LINE_RATIO);
  const height = bigRow + 12;
  // `edge`: the column boundary the handle stands on
  const x = edge * cell;
  const left = Math.max(0, Math.min(x - LOUPE_W / 2, screen.cols * cell - LOUPE_W));
  const centre = x - left;
  const above = row * rowHeight - LOUPE_GAP - height;
  const top = above >= 0 ? above : (row + 1) * rowHeight + LOUPE_GAP;
  const width = Math.ceil(screen.cols * big * CELL_RATIO) + 2;
  return (
    <View pointerEvents="none" accessibilityElementsHidden importantForAccessibility="no-hide-descendants" style={[styles.loupe, { left, top, height, backgroundColor: bg }]}>
      <View style={{ position: "absolute", top: 6, left: centre - edge * big * CELL_RATIO, width }}>
        <Row runs={screen.lines[row] ?? []} cursorCol={null} fontSize={big} width={width} rule={null} fg={fg} bg={bg} />
      </View>
      <View style={[styles.loupeCaret, { left: centre - 1, top: 6, height: bigRow }]} />
    </View>
  );
}

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
  rowsRef,
  selection,
  loupe,
}: {
  screen: ScreenState;
  zoom: number;
  fitted: boolean;
  /** Told each row's height and each cell's width in points (a drag's lines, a touch's cell). */
  onRowHeight?: (height: number, cell: number) => void;
  /** The rows' view, to turn a touch into a cell. */
  rowsRef?: RefObject<View | null>;
  /** The cells selected, in any order; drawn over the rows with a handle at each end. */
  selection?: { a: Cell; b: Cell } | null;
  /** A dragged handle's row and the column boundary it stands on: the loupe shows them. */
  loupe?: { row: number; edge: number } | null;
}) {
  const [width, setWidth] = useState(0);
  const onLayout = (e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width);
  const fit = fitted ? { fontSize: fitFontSize(1), scroll: false } : fitFont(width - PADDING_X * 2, screen.cols);
  const fontSize = fitted ? fitFontSize(zoom) : Math.round(fit.fontSize * zoom * 10) / 10;
  const rowHeight = Math.round(fontSize * LINE_RATIO);
  useEffect(() => {
    onRowHeight?.(rowHeight, fontSize * CELL_RATIO);
  }, [onRowHeight, rowHeight, fontSize]);
  const rowWidth = Math.ceil(screen.cols * fontSize * CELL_RATIO) + 2;
  const defaults = screenDefaults(screen, DEFAULTS);
  const scroll = fit.scroll || rowWidth > width - PADDING_X * 2;
  const [cursorRow, cursorCol] = screen.cursor ?? [-1, -1];
  const rows = (
    <View ref={rowsRef} collapsable={false}>
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
      {selection ? <SelectionMarks a={selection.a} b={selection.b} cols={screen.cols} cell={fontSize * CELL_RATIO} rowHeight={rowHeight} /> : null}
      {selection && loupe ? <Loupe row={loupe.row} edge={loupe.edge} screen={screen} fontSize={fontSize} rowHeight={rowHeight} cell={fontSize * CELL_RATIO} fg={defaults.fg} bg={defaults.bg} /> : null}
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
  canUploadFiles,
  canPaste,
  canMouse,
  keepHeight,
  onPasted,
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
  /** Paste takes the clipboard's image (feature `upload`). */
  canUpload?: boolean;
  /** Paste takes the clipboard's file (feature `upload_files`). */
  canUploadFiles?: boolean;
  /** Several lines can go as one paste (feature `paste`). */
  canPaste?: boolean;
  /** A tap goes to a program that tracks the mouse as a click (feature `mouse`). */
  canMouse?: boolean;
  /** Something docked over the view (a question card): the program keeps its height, as under the keyboard. */
  keepHeight?: boolean;
  /** Something pasted waits in the pane's message box: show it. */
  onPasted?: () => void;
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
  // the row's Paste, a hardware keyboard's and iOS's own paste button: nothing pasted reaches the program
  // before Send. This screen has no message box of its own (vim, htop): what was pasted goes to the
  // pane's, which shows under the screen until it is sent or emptied.
  const box = draftKey(hostKey, paneId);
  const rules: AttachRules = { atPrompt: !running, image: !!canUpload, file: !!canUploadFiles };
  const deliver = (a: Attached) => {
    if (a.kind === "none") return;
    if (a.kind === "error") return setError(a.message);
    if (a.kind === "text") {
      if (!canType && running) return;
      // refused here while no box is open to say why
      const p = running ? programSend(a.text, canType, canPaste) : null;
      if (p && !p.ok) {
        if (p.message) setError(p.message);
        return;
      }
      pending.giveText(box, a.text);
    } else if (!pending.add(box, a.item)) return setError(PENDING_FULL);
    setError(null);
    onPasted?.();
  };
  const onPaste = async () => {
    const current = scope.capture();
    const a = await readClipboard(Clipboard, clipboardFiles, rules);
    if (current()) deliver(a);
  };
  // what iOS's own paste button read, with no prompt: no reading here
  const systemPaste = {
    onText: (text: string) => deliver(nativeAttached({ text }, rules)),
    onImage: (image: string) => deliver(nativeAttached({ image }, rules)),
    onFile: (file: PastedFile) => deliver(nativeAttached({ file }, rules)),
  };
  const onOpen = useOpenShell(hostKey, paneId, setError);

  const typing = operate && canType && !bare;

  // The screen follows its end: when the view shrinks (the keyboard opens) or the screen grows,
  // the last rows stay in sight unless the user scrolled up (judged when a drag or fling ends, not
  // on every scroll event: the view shrinking under the keyboard also reports a scroll).
  const scroller = useRef<ScrollView>(null);
  const stick = useRef(true);
  const toEnd = () => {
    const go = () => {
      if (!stick.current) return;
      // under a question card: the program kept its height, so its last text, not its blank rows
      if (keepHeight && screen) {
        const y = (lastTextRow(screen.lines) + 1) * rowHeight.current + PADDING - local.current.viewH;
        scroller.current?.scrollTo({ y: Math.max(0, y), animated: false });
      } else {
        // the keyboard is up over a tall screen: the cursor stays in sight, not the blank rows at the end
        const y = keepHeight ? null : cursorScrollY(screen?.cursor?.[0], rowHeight.current, PADDING, local.current.viewH);
        if (y !== null) scroller.current?.scrollTo({ y, animated: false });
        else scroller.current?.scrollToEnd({ animated: false });
      }
    };
    go();
    // again once the layout (and the keyboard's animation) has settled
    setTimeout(go, 350);
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
  const cellWidth = useRef(16 / LINE_RATIO * CELL_RATIO);
  const onRowHeight = useCallback((h: number, cell: number) => {
    rowHeight.current = h;
    cellWidth.current = cell;
  }, []);
  const remote = useRef<ScrollCoalescer | null>(null);
  const scrollGeneration = useRef(0);
  // a flick carries on after the finger lifts (scroll.ts Fling)
  const velocity = useRef(new DragVelocity());
  const fling = useRef<Fling | null>(null);
  const drag = useRef<{ x: number; y: number; lastY: number; vertical: boolean | null } | null>(null);
  const focusOnTap = useRef(true);
  useFocusEffect(useCallback(() => {
    if (!online || !canScroll) return;
    let active = true;
    const timers = {
      now: () => Date.now(),
      schedule: (fn: () => void, ms: number) => setTimeout(fn, ms),
      unschedule: (h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>),
    };
    const c = new ScrollCoalescer({
      send: (lines) => {
        const generation = scrollGeneration.current;
        void fleet.sendScroll(hostKey, paneId, lines).then((r) => {
          if (!active || generation !== scrollGeneration.current) return;
          // the rate only slows it (an older desktop allows fewer): sent again, slower
          if (!r.ok && r.code === "rate") c.refused(lines);
          // any other refusal says why once
          else if (!r.ok) setError(r.message);
        });
      },
      ...timers,
    });
    const f = new Fling({ add: (lines) => c.add(lines), done: () => c.flush(), ...timers });
    remote.current = c;
    fling.current = f;
    const stop = () => {
      scrollGeneration.current++;
      f.stop();
      c.cancel();
      velocity.current.clear();
      drag.current = null;
    };
    const sub = AppState.addEventListener("change", (state) => {
      if (state !== "active") stop();
    });
    return () => {
      active = false;
      sub.remove();
      remote.current = null;
      fling.current = null;
      stop();
    };
  }, [hostKey, paneId, online, canScroll]));
  // Hold a finger still on the screen to select: the screen stands still while a selection is up
  // (what the program draws meanwhile waits), a handle follows the finger. Selecting copies: the hold
  // copies the word, lifting a handle copies the range, All copies the screen; a tap copies nothing.
  const [frozen, setFrozen] = useState<ScreenState | null>(null);
  const [selection, setSelection] = useState<{ a: Cell; b: Cell } | null>(null);
  const [copied, setCopied] = useState(false);
  // what the selection last put on the clipboard (null: nothing since it began)
  const [copiedText, setCopiedText] = useState<string | null>(null);
  const selecting = selection !== null;
  // the dragged handle, for the loupe (null: no handle is moving)
  const [loupe, setLoupe] = useState<{ row: number; edge: number } | null>(null);
  // a program on the computer copied to this phone: the same word for it
  const programCopy = s.hosts.find((h) => h.record.key === hostKey)?.copiedAt;
  const seenCopy = useRef(programCopy);
  useEffect(() => {
    if (programCopy === seenCopy.current) return;
    seenCopy.current = programCopy;
    setCopied(true);
    AccessibilityInfo.announceForAccessibility("Copied from the computer");
    const t = setTimeout(() => setCopied(false), 1200);
    return () => clearTimeout(t);
  }, [programCopy]);
  const rowsView = useRef<View | null>(null);
  const metrics = useRef({ x: 0, y: 0 });
  const holdTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const movingHandle = useRef<"a" | "b" | null>(null);
  // where the moving end last was: touches come faster than renders, and each word step ticks once
  const lastEnd = useRef<Cell | null>(null);
  const shown = frozen ?? screen;
  const cancelHold = () => {
    clearTimeout(holdTimer.current);
    holdTimer.current = undefined;
  };
  const endSelection = useCallback(() => {
    setSelection(null);
    setFrozen(null);
    setCopiedText(null);
    setLoupe(null);
    movingHandle.current = null;
  }, []);
  useEffect(() => () => clearTimeout(holdTimer.current), []);
  const cellSize = () => cellWidth.current;
  const cellOf = (pageX: number, pageY: number, s: ScreenState): Cell =>
    cellAt(pageX - metrics.current.x, pageY - metrics.current.y, cellSize(), rowHeight.current, s.cols, s.lines.length);
  // where the rows are on screen, then what the touch means there
  const atRows = (then: () => void) => {
    const view = rowsView.current;
    if (!view) return;
    view.measure((_x, _y, _w, _h, pageX, pageY) => {
      metrics.current = { x: pageX, y: pageY };
      then();
    });
  };
  // where the hold selected: a finger that stays within the slop of it keeps the whole word
  const holdAt = useRef<{ x: number; y: number } | null>(null);
  const startSelection = (pageX: number, pageY: number) => {
    const s = screen;
    if (!s) return;
    atRows(() => {
      const [a, b] = wordAt(s.lines, cellOf(pageX, pageY, s));
      focusOnTap.current = false;
      tap.current = null;
      drag.current = null;
      setFrozen(s);
      setSelection({ a, b });
      movingHandle.current = "b";
      lastEnd.current = b;
      holdAt.current = { x: pageX, y: pageY };
      setCopied(false);
      haptics.selectStart();
      // the hold copies the word
      copyRange(s, a, b, null);
    });
  };
  // Selecting copies. Nothing is written for a blank range or text already on the clipboard.
  const copyRange = (from: ScreenState, a: Cell, b: Cell, already: string | null) => {
    const text = textToCopy(selectionText(from.lines, a, b), already);
    if (text === null) return;
    void Clipboard.setStringAsync(text).then(
      () => {
        setCopiedText(text);
        haptics.copied();
        AccessibilityInfo.announceForAccessibility("Selected and copied");
      },
      () => AccessibilityInfo.announceForAccessibility("Could not copy"),
    );
  };
  // the bar's Word, Line and Screen widen the selection to whole words, whole rows or every cell
  const widen = (to: "word" | "line" | "screen") => {
    if (!shown || !selection) return;
    const [from, end] = ordered(selection.a, selection.b);
    const [a, b] =
      to === "screen"
        ? screenSpan(shown.lines.length, shown.cols)
        : to === "line"
          ? [{ row: from.row, col: 0 }, lineAt(shown.lines, end)[1]]
          : [wordAt(shown.lines, from)[0], wordAt(shown.lines, end)[1]];
    setSelection({ a, b });
    copyRange(shown, a, b, copiedText);
  };
  // To prompt: the selection goes into this pane's command line (promptText: nothing in it runs by itself);
  // offered only while that line is drawn under the screen (`bare`), where the text shows
  const toPrompt = () => {
    if (!shown || !selection) return;
    const text = promptText(selectionText(shown.lines, selection.a, selection.b));
    if (text === "") return;
    const draft = fleet.store.peek().drafts[draftKey(hostKey, paneId)] ?? "";
    fleet.setDraft(hostKey, paneId, draft === "" || /\s$/.test(draft) ? draft + text : `${draft} ${text}`);
    AccessibilityInfo.announceForAccessibility("Added to the command line");
    endSelection();
  };
  // a screen reader has no hold: it copies the whole screen
  const copyScreen = () => {
    if (!screen) return;
    const last = { row: screen.lines.length - 1, col: screen.cols - 1 };
    void Clipboard.setStringAsync(selectionText(screen.lines, { row: 0, col: 0 }, last)).then(
      () => AccessibilityInfo.announceForAccessibility("Copied"),
      () => AccessibilityInfo.announceForAccessibility("Could not copy"),
    );
  };
  const a11yCopy = {
    accessibilityActions: [{ name: "copy", label: "Copy the screen" }],
    onAccessibilityAction: () => copyScreen(),
  };
  const dragMove = (x: number, y: number) => {
    const d = drag.current;
    if (!d) return;
    if (d.vertical === null) {
      if (Math.hypot(x - d.x, y - d.y) <= DRAG_SLOP) return;
      focusOnTap.current = false;
      d.vertical = Math.abs(y - d.y) > Math.abs(x - d.x);
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
  // the controls float over the screen while a pinch changes it, and always for a screen reader
  // (it has no pinch): they never take rows from the program
  const [zoomShown, setZoomShown] = useState(false);
  const [readerOn, setReaderOn] = useState(false);
  const zoomTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    void AccessibilityInfo.isScreenReaderEnabled().then(setReaderOn);
    const sub = AccessibilityInfo.addEventListener("screenReaderChanged", setReaderOn);
    return () => {
      sub.remove();
      clearTimeout(zoomTimer.current);
    };
  }, []);
  const setZoom = (z: number) => {
    const c = clampZoom(z);
    keptZoom.value = c;
    setZoomState(c);
    setZoomShown(true);
    clearTimeout(zoomTimer.current);
    zoomTimer.current = setTimeout(() => setZoomShown(false), ZOOM_SHOWN_MS);
  };
  const pinch = useRef<{ distance: number; zoom: number } | null>(null);
  const [pinching, setPinching] = useState(false);
  const tap = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const lastTap = useRef<number | null>(null);
  const onTouchStart = (e: GestureResponderEvent) => {
    const t = e.nativeEvent.touches;
    cancelHold();
    if (selecting) {
      // a second finger or a touch away from both handles ends it; neither zooms nor scrolls
      focusOnTap.current = false;
      if (t.length !== 1 || !shown) return endSelection();
      const { pageX, pageY } = t[0]!;
      const { from, to, start, end } = handleSpots(selection.a, selection.b, cellSize(), rowHeight.current);
      const dist = (p: { x: number; y: number }) => Math.hypot(pageX - metrics.current.x - p.x, pageY - metrics.current.y - p.y);
      const toFirst = dist(start);
      const toLast = dist(end);
      if (Math.min(toFirst, toLast) > HANDLE_REACH) return endSelection();
      // the end the finger took keeps moving, the other stays
      setSelection({ a: toFirst <= toLast ? to : from, b: toFirst <= toLast ? from : to });
      lastEnd.current = toFirst <= toLast ? from : to;
      movingHandle.current = "b";
      return;
    }
    // a touch stops a fling where it is; what a plain drag left waiting still goes out
    const flinging = !!fling.current?.running;
    focusOnTap.current = !flinging;
    if (flinging) {
      scrollGeneration.current++;
      fling.current?.stop();
      remote.current?.cancel();
    }
    velocity.current.clear();
    velocity.current.add(0, Date.now());
    if (t.length === 2) {
      pinch.current = { distance: touchDistance(t[0]!, t[1]!), zoom };
      tap.current = null;
      drag.current = null;
      remote.current?.cancel();
      setPinching(true);
    } else if (t.length === 1) {
      const { pageX, pageY } = t[0]!;
      tap.current = { x: pageX, y: pageY, moved: false };
      drag.current = { x: pageX, y: pageY, lastY: pageY, vertical: null };
      holdTimer.current = setTimeout(() => {
        holdTimer.current = undefined;
        if (tap.current && !tap.current.moved) startSelection(pageX, pageY);
      }, SELECT_HOLD_MS);
    }
  };
  const onTouchMove = (e: GestureResponderEvent) => {
    const t = e.nativeEvent.touches;
    if (selecting) {
      if (movingHandle.current && t.length === 1 && shown) {
        const at = holdAt.current;
        if (at && Math.hypot(t[0]!.pageX - at.x, t[0]!.pageY - at.y) <= DRAG_SLOP) return;
        holdAt.current = null;
        const b = cellOf(t[0]!.pageX, t[0]!.pageY, shown);
        if (wordStep(shown.lines, lastEnd.current ?? selection.b, b)) haptics.selectStep();
        lastEnd.current = b;
        // the moving end is the selection's start when it lies before the other end
        setLoupe({ row: b.row, edge: ordered(selection.a, b)[0] === b ? b.col : b.col + 1 });
        setSelection((sel) => (sel ? { a: sel.a, b } : sel));
      }
      return;
    }
    if (t.length === 2 && pinch.current) setZoom(pinchZoom(pinch.current, touchDistance(t[0]!, t[1]!)));
    else if (t.length === 1) {
      if (tap.current && Math.hypot(t[0]!.pageX - tap.current.x, t[0]!.pageY - tap.current.y) > DRAG_SLOP) {
        tap.current.moved = true;
        cancelHold();
      }
      dragMove(t[0]!.pageX, t[0]!.pageY);
    }
  };
  const onTouchEnd = (e: GestureResponderEvent) => {
    const left = e.nativeEvent.touches.length;
    cancelHold();
    if (selecting) {
      // lifting a handle copies the range it left (a drag copies nothing on the way)
      if (left === 0 && movingHandle.current && shown) copyRange(shown, selection.a, selection.b, copiedText);
      setLoupe(null);
      lastEnd.current = null;
      movingHandle.current = null;
      holdAt.current = null;
      return;
    }
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
      } else {
        lastTap.current = now;
        // a program that tracks the mouse takes the tap as a click; any other says no, silently.
        // A touch that only caught a flick clicks nothing
        if (canMouse && screen && focusOnTap.current) {
          const { x, y } = tap.current;
          atRows(() => {
            const c = cellOf(x, y, screen);
            void fleet.sendMouse(hostKey, paneId, c.col, c.row);
          });
        }
      }
    }
    if (left === 0) tap.current = null;
  };
  const onTouchCancel = () => {
    cancelHold();
    movingHandle.current = null;
    scrollGeneration.current++;
    fling.current?.stop();
    remote.current?.cancel();
    velocity.current.clear();
    drag.current = null;
    pinch.current = null;
    tap.current = null;
    lastTap.current = null;
    setPinching(false);
  };
  // The scrolling area's size, less the zoom row, is what a fitted program may fill.
  const [area, setArea] = useState({ width: 0, height: 0 });
  // the keyboard only covers the program: its last rows scroll into sight, its size stays
  const keyboardUp = useRef(false);
  useEffect(() => {
    const ios = Platform.OS === "ios";
    const show = Keyboard.addListener(ios ? "keyboardWillShow" : "keyboardDidShow", () => {
      keyboardUp.current = true;
    });
    const hide = Keyboard.addListener("keyboardDidHide", () => {
      keyboardUp.current = false;
      setArea((a) => ({ ...a }));
    });
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);
  const onArea = (e: LayoutChangeEvent) => {
    const { width, height } = e.nativeEvent.layout;
    setArea((a) => fitArea(a, { width, height }, keyboardUp.current || !!keepHeight || screen?.ageMs != null));
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
  const size = fitting ? fitSize(area.width - PADDING_X * 2, area.height - PADDING * 2, fitFontSize(zoom)) : null;
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
    <View style={styles.zoomRow}>
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

  const selectedNow = selection && shown ? selectionText(shown.lines, selection.a, selection.b) : "";
  // a desktop in use keeps its size: then the phone fits what comes to its width
  const fittedHere = held && (shown?.cols ?? 0) <= (cols ?? 0);
  const grid = shown ? (
    <Grid screen={shown} zoom={zoom} fitted={fittedHere} onRowHeight={onRowHeight} rowsRef={rowsView} selection={selection} loupe={loupe} />
  ) : (
    // the pane's steps card (PaneLoading) says what is awaited; these are the rows to come
    <ScreenSkeleton />
  );

  return (
    <View style={[styles.fill, { backgroundColor: screenDefaults(screen, DEFAULTS).bg }]}>
      {/* above the screen, not over its first row; the program keeps its size meanwhile (onArea) */}
      {screen?.ageMs != null ? (
        <View style={styles.age} accessibilityLiveRegion="polite">
          <Text style={styles.ageText}>{`Updated ${Math.round(screen.ageMs / 1000)} s ago`}</Text>
        </View>
      ) : null}
      {/* the floating controls belong to the screen: over it, never under the key bar */}
      <View style={styles.fill}>
      <View style={styles.fill} onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd} onTouchCancel={onTouchCancel}>
        {typing ? (
          // the label stays on the ScrollView: on the capture view TalkBack would read the
          // screen as one element instead of its rows
          <KeyCaptureView
            ref={capture}
            mode="terminal"
            onText={(e) => typed(e.text)}
            onKey={onKey}
            onFocusChange={setFocused}
            onPaste={() => void onPaste()}
            style={styles.fill}
          >
            <ScrollView ref={scroller} style={styles.fill} accessibilityLabel={label} {...a11yCopy} scrollEnabled={localScroll && !selecting} bounces={!canScroll} onLayout={onArea} onScroll={trackLocal} scrollEventThrottle={16} onScrollEndDrag={onScroll} onMomentumScrollEnd={onScroll} onContentSizeChange={onContent}>
              {/* not one element for screen readers: they read the rows */}
              <Pressable onPress={() => { if (focusOnTap.current) capture.current?.focus(); }} accessible={false}>
                {grid}
              </Pressable>
            </ScrollView>
          </KeyCaptureView>
        ) : (
          <ScrollView ref={scroller} style={styles.fill} accessibilityLabel={label} {...a11yCopy} scrollEnabled={localScroll && !selecting} bounces={!canScroll} onLayout={onArea} onScroll={trackLocal} scrollEventThrottle={16} onScrollEndDrag={onScroll} onMomentumScrollEnd={onScroll} onContentSizeChange={onContent}>
            {grid}
          </ScrollView>
        )}
      </View>
      {screen && (zoomShown || readerOn) && !selecting ? zoomRow : null}
      {selecting ? (
        <SelectionBar
          selected={selectedNow}
          copied={copiedText !== null && copiedText === selectedNow}
          note={loupe ? "lift to copy" : "screen held"}
          // at the bottom, in thumb reach, unless the selection's last row is where the bar would be
          low={barStaysLow(Math.max(selection.a.row, selection.b.row), shown!.lines.length, area.height)}
          keys={[
            { label: "Word", hint: "Select whole words", onPress: () => widen("word") },
            { label: "Line", hint: "Select whole lines", onPress: () => widen("line") },
            { label: "Screen", hint: "Select the whole screen", onPress: () => widen("screen") },
            ...(operate && bare ? [{ label: "To prompt", hint: "Put the selection in the command line", onPress: toPrompt, sep: true }] : []),
          ]}
          onCancel={endSelection}
        />
      ) : copied ? (
        <View style={styles.copiedBar} pointerEvents="none">
          <Text style={styles.selectKeyText}>Copied</Text>
        </View>
      ) : null}
      </View>
      {/* without the key bar (the message input below, or view only), a refused scroll or paste says why here */}
      {error && (bare || !operate) ? <ErrorText style={styles.bareError}>{error}</ErrorText> : null}
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
              // a screenshot for the program needs Paste: on the row whatever Settings › Keys holds
              ids={rowWithPaste(fullScreenRow(s.settings.controlKeys), !!canUpload)}
              sticky={sticky}
              disabled={!online}
              onSticky={setSticky}
              onKey={onRowKey}
              onOpen={onOpen}
              onPaste={canType || canUpload ? () => void onPaste() : undefined}
              systemPaste={canUpload ? systemPaste : undefined}
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
  zoomRow: { position: "absolute", top: 8, right: 8, flexDirection: "row", alignItems: "center", gap: 8, padding: 6, borderRadius: 22, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.glassRim, elevation: 8 },
  zoomButton: { width: 32, height: 32, borderRadius: 16, borderWidth: 1, borderColor: colors.borderStrong, alignItems: "center", justifyContent: "center" },
  zoomGlyph: { fontFamily: ui(400), color: colors.text, fontSize: 16 },
  zoomValue: { fontFamily: mono, color: colors.text, fontSize: 12, minWidth: 44, textAlign: "center" },
  oldRift: { fontFamily: ui(400), color: colors.textDim, fontSize: 12 },
  // a selection's ground and its handles; the bar floats over the top of the screen
  age: { paddingHorizontal: 12, paddingVertical: 4, backgroundColor: colors.chrome },
  ageText: { fontFamily: ui(500), color: colors.textMuted, fontSize: 12 },
  band: { position: "absolute", backgroundColor: `${colors.accent}55` },
  stem: { position: "absolute", width: STEM, borderRadius: STEM / 2, backgroundColor: colors.accent },
  knob: { position: "absolute", width: KNOB, height: KNOB, borderRadius: KNOB / 2, backgroundColor: colors.accent, borderWidth: 2, borderColor: colors.bg },
  loupe: { position: "absolute", width: LOUPE_W, overflow: "hidden", borderRadius: 22, borderWidth: 1, borderColor: colors.borderStrong, elevation: 10, shadowColor: "#000", shadowOpacity: 0.8, shadowRadius: 14, shadowOffset: { width: 0, height: 8 } },
  loupeCaret: { position: "absolute", width: 2, borderRadius: 1, backgroundColor: colors.accent },
  selectKeyText: { fontFamily: ui(500), color: colors.text, fontSize: 14 },
  copiedBar: { position: "absolute", top: 8, alignSelf: "center", paddingHorizontal: 14, height: 36, justifyContent: "center", borderRadius: 18, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.glassRim, elevation: 8 },
  bareError: { paddingHorizontal: 12, paddingVertical: 6, backgroundColor: colors.chrome },
  // a block's rule: 2 px in the 6 pt gutter left of the row
  rule: { position: "absolute", left: -4, top: 0, width: 2 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
