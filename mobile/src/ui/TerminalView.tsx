import { memo, useEffect, useRef, useState } from "react";
import {
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
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
import type { ScreenState } from "@/protocol/store";
import { RUN_BOLD, RUN_ITALIC, RUN_STRIKE, RUN_UNDERLINE, type ScreenRun } from "@/protocol/types";
import { authenticate, fleet, useApp } from "@/runtime";
import { CELL_RATIO, fitFont, LINE_RATIO, rowSegments, segmentColors } from "@/terminal";
import { Button } from "./Button";
import { ControlRow, NO_STICKY, type Sticky } from "./ControlRow";
import { ErrorText, WarningText } from "./Notice";
import { colors, mono, radius, TOUCH } from "./theme";

/** The terminal's default colours (a run's null fg / bg): the app's text on its background. */
const DEFAULTS = { fg: colors.text, bg: colors.bg };
const PADDING = 8;

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
                fontWeight: s.flags & RUN_BOLD ? "700" : undefined,
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

/** The screen as monospace rows, fitted to the width (7 to 14 pt), scrolling sideways when wider. */
function Grid({ screen }: { screen: ScreenState }) {
  const [width, setWidth] = useState(0);
  const onLayout = (e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width);
  const { fontSize, scroll } = fitFont(width - PADDING * 2, screen.cols);
  const rowWidth = Math.ceil(screen.cols * fontSize * CELL_RATIO) + 2;
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
 * The pane's Terminal view: the gateway's screen (vim, less, htop) and, on an operate device,
 * the keyboard for it. Typed text goes out as `text` (batched per frame, `keyboard.ts`), keys as
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
  onShowBlocks,
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
  /** Back to the Blocks view (its command line). */
  onShowBlocks: () => void;
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
    if (cur.ctrl || cur.alt) setSticky(NO_STICKY);
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

  const typing = operate && canType;
  const label = screen
    ? `Terminal screen, ${screen.cols} columns by ${screen.rows} rows${screen.alt ? ", full-screen program" : ""}${screen.stale ? ", not up to date" : ""}`
    : "Terminal screen";

  const grid = screen ? (
    <Grid screen={screen} />
  ) : (
    <Text style={styles.empty} accessibilityLiveRegion="polite">
      {online ? "Waiting for the screen…" : "Waiting for the computer…"}
    </Text>
  );

  return (
    <View style={styles.fill}>
      <Text style={styles.caption} accessibilityRole="header">
        {screen
          ? `${screen.cols} × ${screen.rows}${screen.alt ? " · full-screen program" : ""}${screen.stale ? " · not up to date" : ""}`
          : "Terminal"}
      </Text>
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
          <ScrollView style={styles.fill} accessibilityLabel={label}>
            {/* not one element for screen readers: they read the rows */}
            <Pressable onPress={() => capture.current?.focus()} accessible={false}>
              {grid}
            </Pressable>
          </ScrollView>
        </KeyCaptureView>
      ) : (
        <ScrollView style={styles.fill} accessibilityLabel={label}>
          {grid}
        </ScrollView>
      )}
      {operate ? (
        <View style={styles.bar}>
          {atPrompt ? (
            <View style={styles.prompt}>
              <WarningText style={styles.flex}>
                This pane is at its prompt: typed text goes to programs only. Type commands in the command line.
              </WarningText>
              <Button label="Command line" onPress={onShowBlocks} />
            </View>
          ) : null}
          {error ? <ErrorText>{error}</ErrorText> : null}
          {!canType ? (
            <Text style={styles.note}>This computer's Rift takes keys from the row only; update it to type here.</Text>
          ) : null}
          <View style={styles.keysRow}>
            {typing ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={focused ? "Hide keyboard" : "Show keyboard"}
                accessibilityState={{ disabled: !online, expanded: focused }}
                disabled={!online}
                onPress={() => (focused ? capture.current?.blur() : capture.current?.focus())}
                style={({ pressed }) => [
                  styles.kbd,
                  focused && styles.kbdOn,
                  !online && styles.disabled,
                  pressed && styles.pressed,
                ]}
              >
                <Text style={[styles.kbdLabel, focused && { color: colors.bg }]}>⌨</Text>
              </Pressable>
            ) : null}
            <View style={styles.flex}>
              <ControlRow
                ids={s.settings.controlKeys}
                sticky={sticky}
                disabled={!online}
                onSticky={setSticky}
                onKey={onRowKey}
              />
            </View>
          </View>
        </View>
      ) : (
        <Text style={styles.viewOnly}>View only: this device cannot type here.</Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  flex: { flex: 1 },
  caption: { color: colors.textDim, fontSize: 12, paddingHorizontal: 12, paddingTop: 6 },
  gridWrap: { flex: 1 },
  gridPad: { padding: PADDING },
  row: { fontFamily: mono, color: DEFAULTS.fg },
  empty: { color: colors.textMuted, fontSize: 14, padding: 16 },
  // as the blocks view's view-only line
  viewOnly: {
    color: colors.textDim,
    fontSize: 13,
    textAlign: "center",
    paddingVertical: 10,
    backgroundColor: colors.chrome,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.hairline,
  },
  bar: {
    backgroundColor: colors.chrome,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.hairline,
    paddingHorizontal: 6,
    paddingTop: 6,
    paddingBottom: 8,
    gap: 6,
  },
  prompt: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 8 },
  note: { color: colors.textMuted, fontSize: 12, paddingHorizontal: 4 },
  keysRow: { flexDirection: "row", gap: 3, alignItems: "center" },
  kbd: {
    minWidth: TOUCH,
    minHeight: TOUCH,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.control,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  kbdOn: { backgroundColor: colors.accent, borderColor: colors.accent },
  kbdLabel: { color: colors.text, fontSize: 18 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
