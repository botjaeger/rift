import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, AppState, Pressable, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { hasSystemPasteButton, SystemPasteButton, type PastedFile } from "../../modules/rift-keys";
import { ARROW_KEYS, blockColumns, controlKey, KeyRepeat, keyTipTitle, PressGate, type ControlKey } from "@/controlKeys";
import type { OpenKind } from "@/protocol/types";
import { Icon, type IconName } from "./Icon";
import { monoMedium, ui, makeStyles, useColors } from "./theme";
import { toggleA11y } from "./toggle";

/** The key block's key height: two rows and their 6 pt gap are the 88 pt of the arrow pad. */
export const KEY_H = 41;
/** The arrow pad: 132 x 88 pt, three columns and two rows 4 pt apart (the Keys redesign board). */
export const PAD_W = 132;
export const BLOCK_H = 88;

/** Sticky modifiers from the block, applied to the next key or letter. */
export interface Sticky {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

export const NO_STICKY: Sticky = { ctrl: false, alt: false, shift: false };

/** A sticky key's modifier ("ctrl", "alt", "shift"). */
const modOf = (id: string): keyof Sticky => (id === "alt" ? "alt" : id === "shift" ? "shift" : "ctrl");

/**
 * A key's face (the Keys redesign board): the surface ground, a glass rim and 8 pt corners; an
 * 18 pt icon from the one set (arrows, ⌃ ⌥ ⇧, Return, the keyboard), its label in JetBrains Mono
 * 13 ("Esc", "⌃D", "F5"), or a word in Inter 12 (Paste). A modifier before a word (⇧Tab) is its
 * icon at 13 pt. Colour means a state only: a held modifier is lit, the accent with dark ink.
 * `style` sizes it (the block's cells fill their place; Settings › Keys' list draws 44 x 34).
 */
export function KeyCap({
  def,
  color: colorProp,
  active,
  ground: groundProp,
  style,
}: {
  def: Pick<ControlKey, "label" | "icon" | "lead" | "paste">;
  color?: string;
  active?: boolean;
  /** The key's ground (Settings › Keys draws its list's keys on the page). */
  ground?: string;
  style?: StyleProp<ViewStyle>;
}) {
  const colors = useColors();
  const styles = useStyles();
  const color = colorProp ?? colors.text;
  const ground = groundProp ?? colors.surface;
  const fg = active ? colors.onAccent : color;
  // the lead icon stands for the label's first character (the ⇧ of ⇧Tab)
  const text = def.lead ? [...def.label].slice(1).join("") : def.label;
  return (
    <View style={[styles.key, { backgroundColor: ground }, style, active && styles.keyActive]}>
      {def.paste ? (
        <Text style={[styles.word, { color: fg }]}>{def.label}</Text>
      ) : def.icon ? (
        <Icon name={def.icon} size={18} weight="medium" color={fg} />
      ) : (
        <>
          {def.lead ? <Icon name={def.lead} size={13} weight="medium" color={fg} /> : null}
          <Text style={[styles.label, { color: fg }]} numberOfLines={1}>
            {text}
          </Text>
        </>
      )}
    </View>
  );
}

/** What a long press on a key shows: its name and what it sends. */
export interface Tip {
  title: string;
  text: string;
}

/** Hold a key this long (milliseconds) and the tip shows instead of the key being sent. */
export const TIP_DELAY_MS = 450;

export const tipOf = (k: ControlKey): Tip => ({ title: keyTipTitle(k), text: k.tip });
const HIDE_TIP: Tip = { title: "Hide keyboard", text: "Closes the keyboard; tap the command field to bring it back" };
const SHOW_TIP: Tip = { title: "Show keyboard", text: "Opens the keyboard for this pane" };

/** The tip card: the key's name in bold, what it sends under it. Not touchable; the block positions it above itself. */
export function KeyTip({ tip, below }: { tip: Tip; below?: boolean }) {
  const styles = useStyles();
  return (
    <View pointerEvents="none" style={[styles.tip, below && styles.tipBelow]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <Text style={styles.tipTitle}>{tip.title}</Text>
      <Text style={styles.tipText}>{tip.text}</Text>
    </View>
  );
}

/**
 * A key of the block: a tap sends it; holding it for 450 ms shows its tip (`onTip`) and lifting the
 * finger hides the tip without sending anything. The tip is also the screen reader's hint. A held
 * modifier (`active` given) is a toggle, announced as selected while it is lit.
 */
export function Key({
  def,
  spoken,
  onPress,
  active,
  disabled,
  color,
  hint,
  tip,
  onTip,
}: {
  def: Pick<ControlKey, "label" | "icon" | "lead" | "paste">;
  spoken: string;
  onPress: () => void;
  active?: boolean;
  disabled: boolean;
  color?: string;
  hint?: string;
  tip?: Tip;
  onTip?: (tip: Tip | null) => void;
}) {
  const styles = useStyles();
  const [gate] = useState(() => new PressGate());
  const toggle = active === undefined ? null : toggleA11y(active, disabled);
  return (
    <Pressable
      // the modifiers stay on for the next key: toggle buttons, announced as selected while lit
      {...(toggle
        ? { ...toggle, accessibilityState: { ...toggle.accessibilityState, selected: active } }
        : { accessibilityRole: "button" as const, accessibilityState: { disabled } })}
      accessibilityLabel={spoken}
      accessibilityHint={hint ?? tip?.text}
      disabled={disabled}
      onPressIn={() => gate.begin()}
      delayLongPress={TIP_DELAY_MS}
      onLongPress={
        tip && onTip
          ? () => {
              gate.longPress();
              onTip(tip);
            }
          : undefined
      }
      onPressOut={() => onTip?.(null)}
      onPress={() => {
        if (gate.release() === "send") onPress();
      }}
      style={({ pressed }) => [styles.cell, disabled && styles.disabled, pressed && styles.pressed]}
    >
      <KeyCap def={def} color={color} active={active} style={styles.fill} />
    </Pressable>
  );
}

/**
 * An arrow of the pad: it goes when the finger goes down and repeats while held (controlKeys.ts
 * `KeyRepeat`); lifting, a cancelled touch, disabling it, leaving the view or the app leaving the
 * foreground stops it. A screen reader's activation sends it once.
 */
function ArrowKey({ id, disabled, onKey }: { id: string; disabled: boolean; onKey: (id: string) => void }) {
  const colors = useColors();
  const styles = useStyles();
  const def = controlKey(id)!;
  const [repeat] = useState(() => new KeyRepeat());
  // the latest handler: a repeat calls the render's after a held modifier was used up
  const send = useRef(onKey);
  useEffect(() => {
    send.current = onKey;
  });
  useEffect(() => {
    if (disabled) repeat.stop();
  }, [disabled, repeat]);
  useEffect(() => {
    const sub = AppState.addEventListener("change", (state) => {
      if (state !== "active") repeat.stop();
    });
    return () => {
      sub.remove();
      repeat.stop();
    };
  }, [repeat]);
  const fire = () => send.current(id);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={def.spoken}
      accessibilityHint={`${def.tip}. Hold to repeat`}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPressIn={() => repeat.start(fire)}
      onPressOut={() => repeat.stop()}
      onPress={() => repeat.tap(fire)}
      style={({ pressed }) => [styles.arrow, disabled && styles.disabled, pressed && styles.pressed]}
    >
      <KeyCap def={def} color={colors.text} style={styles.fill} />
    </Pressable>
  );
}

const icon = (name: IconName, label: string) => ({ icon: name, label });

/**
 * The keys (the Keys redesign board), one strip for the command line and a full-screen program
 * alike: the key block (Settings › Keys, `ids`, in two rows; the keyboard key always last) beside
 * the arrow pad (↑ above ← ↓ →; a held arrow repeats), 88 pt high, so the screen above keeps its
 * size. `onKey` gets a key's protocol name, the sticky ctrl / alt / shift toggle `sticky` (lit
 * until the next key), the new tab / new pane keys `onOpen` (disabled without it: the computer's
 * Rift cannot open them), Paste `onPaste` (disabled without it: the computer's Rift takes no
 * text). The keyboard key hides the keyboard (`onHideKeyboard`) or opens it (`onShowKeyboard`).
 */
export function ControlRow({
  ids,
  sticky,
  disabled,
  onSticky,
  onKey,
  onOpen,
  onPaste,
  systemPaste,
  onHideKeyboard,
  onShowKeyboard,
  tipBelow,
}: {
  ids: readonly string[];
  sticky: Sticky;
  disabled: boolean;
  onSticky: (next: Sticky) => void;
  onKey: (key: string) => void;
  onOpen?: (kind: OpenKind) => void;
  onPaste?: () => void;
  /** Where iOS's own paste button takes the key's place: it reads with no prompt. */
  systemPaste?: { onText: (text: string) => void; onImage: (b64: string) => void; onFile: (file: PastedFile) => void };
  onHideKeyboard?: () => void;
  /** The keyboard is down: the keyboard key opens it. */
  onShowKeyboard?: () => void;
  /** The tip card under the keys (Settings › Keys, under its header). */
  tipBelow?: boolean;
}) {
  const colors = useColors();
  const styles = useStyles();
  const [tip, setTip] = useState<Tip | null>(null);
  const keys = ids.flatMap((id) => {
    const def: ControlKey | undefined = controlKey(id);
    // the arrows are the pad's
    if (!def || ARROW_KEYS.includes(id)) return [];
    // Interrupt ends a program: the destructive red (palette.ts holds it to 4.5:1 on a surface)
    const color = def.danger ? colors.error : colors.text;
    if (def.sticky) {
      const mod = modOf(id);
      return [
        <Key
          key={id}
          tip={tipOf(def)}
          onTip={setTip}
          def={def}
          spoken={def.spoken}
          color={color}
          disabled={disabled}
          active={sticky[mod]}
          onPress={() => {
            const on = !sticky[mod];
            onSticky({ ...sticky, [mod]: on });
            AccessibilityInfo.announceForAccessibility(on ? `${def.short ?? def.spoken} held for the next key` : `${def.short ?? def.spoken} let go`);
          }}
        />,
      ];
    }
    if (def.open) {
      const kind = def.open;
      return [
        <Key
          key={id}
          tip={tipOf(def)}
          onTip={setTip}
          def={def}
          spoken={def.spoken}
          hint={onOpen ? undefined : "Update Rift on the computer to open tabs and panes from here"}
          color={color}
          disabled={disabled || !onOpen}
          onPress={() => onOpen?.(kind)}
        />,
      ];
    }
    if (def.paste) {
      if (hasSystemPasteButton && systemPaste && onPaste && !disabled) {
        return [
          <View key={id} style={styles.cell}>
            <SystemPasteButton {...systemPaste} ground={colors.surface} ink={colors.text} style={[styles.key, styles.fill, { backgroundColor: colors.surface }]} />
          </View>,
        ];
      }
      return [
        <Key
          key={id}
          tip={tipOf(def)}
          onTip={setTip}
          def={def}
          spoken={def.spoken}
          hint={onPaste ? "Pastes the clipboard into the program" : "Update Rift on the computer to paste here"}
          color={color}
          disabled={disabled || !onPaste}
          onPress={() => onPaste?.()}
        />,
      ];
    }
    return [<Key key={id} def={def} spoken={def.spoken} tip={tipOf(def)} onTip={setTip} color={color} disabled={disabled} onPress={() => onKey(id)} />];
  });
  // the keyboard key, always the block's last
  keys.push(
    onHideKeyboard ? (
      <Key key="keyboard" def={icon("hideKeyboard", "⌨")} spoken="Hide keyboard" tip={HIDE_TIP} onTip={setTip} disabled={false} onPress={onHideKeyboard} />
    ) : (
      <Key
        key="keyboard"
        def={icon("keyboard", "⌨")}
        spoken="Show keyboard"
        hint={onShowKeyboard ? undefined : "This computer's Rift takes keys only; update it to type here"}
        tip={SHOW_TIP}
        onTip={setTip}
        disabled={disabled || !onShowKeyboard}
        onPress={() => onShowKeyboard?.()}
      />
    ),
  );
  // two rows, as many columns as the keys need (three for the default six); a short last row keeps the columns
  const cols = blockColumns(keys.length);
  const rows = [keys.slice(0, cols), keys.slice(cols)].map((row, r) => (
    <View key={r} style={styles.row}>
      {row}
      {Array.from({ length: cols - row.length }, (_, i) => (
        <View key={`gap${i}`} style={styles.cell} />
      ))}
    </View>
  ));
  const arrow = (id: string) => <ArrowKey key={id} id={id} disabled={disabled} onKey={onKey} />;
  return (
    <View style={styles.host}>
      {tip ? <KeyTip tip={tip} below={tipBelow} /> : null}
      <View style={styles.block}>
        <View style={styles.keys} accessibilityRole="toolbar" accessibilityLabel="Keys">
          {rows}
        </View>
        <View style={styles.pad} accessibilityRole="toolbar" accessibilityLabel="Arrow keys">
          <View style={styles.padRow}>
            <View style={styles.arrowGap} />
            {arrow("up")}
            <View style={styles.arrowGap} />
          </View>
          <View style={styles.padRow}>
            {arrow("left")}
            {arrow("down")}
            {arrow("right")}
          </View>
        </View>
      </View>
    </View>
  );
}

// The Keys redesign board: the key block (3 columns, 41 pt keys 6 pt apart) and, 10 pt beside it,
// the 132 x 88 arrow pad (4 pt gaps); every key 8 pt corners on the surface with a glass rim, a lit
// modifier the accent with dark ink.
const useStyles = makeStyles((colors) => ({
  host: { zIndex: 2 },
  block: { flexDirection: "row", gap: 10, height: BLOCK_H },
  keys: { flex: 1, minWidth: 0, gap: 6 },
  row: { flex: 1, flexDirection: "row", gap: 6 },
  cell: { flex: 1, minWidth: 0, height: KEY_H },
  pad: { width: PAD_W, height: BLOCK_H, gap: 4 },
  padRow: { flex: 1, flexDirection: "row", gap: 4 },
  arrow: { flex: 1 },
  arrowGap: { flex: 1 },
  fill: { flex: 1, width: "100%" },
  // the tip floats above the keys, over what is there: a card on the raised ground
  tip: {
    position: "absolute",
    left: 0,
    bottom: "100%",
    marginBottom: 8,
    maxWidth: 320,
    paddingVertical: 8,
    paddingHorizontal: 12,
    gap: 2,
    borderRadius: 12,
    backgroundColor: colors.menu,
    borderWidth: 1,
    borderColor: colors.glassRim,
    elevation: 12,
    zIndex: 10,
  },
  // where there is no room above (Settings › Keys, under its header): under the keys
  tipBelow: { bottom: undefined, top: "100%", marginBottom: 0, marginTop: 8 },
  tipTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 14 },
  tipText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17 },
  key: {
    borderRadius: 8,
    flexDirection: "row",
    gap: 3,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: colors.glassRim,
  },
  // a held modifier: lit, the accent ground and rim, its icon in the accent's ink
  keyActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  label: { fontFamily: monoMedium, fontSize: 13, letterSpacing: 0.2 },
  word: { fontFamily: ui(500), fontSize: 12 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
}));
