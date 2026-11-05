import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { hasSystemPasteButton, SystemPasteButton, type PastedFile } from "../../modules/rift-keys";
import { controlKey, keyTipTitle, PressGate, type ControlKey } from "@/controlKeys";
import type { OpenKind } from "@/protocol/types";
import { Icon, type IconName } from "./Icon";
import { colors, monoMedium, tints, TOUCH, ui } from "./theme";
import { toggleA11y } from "./toggle";

/** A key's width (the Keys board: 52 x 44 pt). */
export const KEY_W = 52;

/** Sticky modifiers from the row, applied to the next key or letter. */
export interface Sticky {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

export const NO_STICKY: Sticky = { ctrl: false, alt: false, shift: false };

/** A sticky key's modifier ("ctrl", "alt", "shift"). */
const modOf = (id: string): keyof Sticky => (id === "alt" ? "alt" : id === "shift" ? "shift" : "ctrl");

/**
 * The Keys board's key (KeysNormalized): 52 x 44 pt with 12 pt corners, the surface ground and a
 * glass rim; an 18 pt icon from the one set (arrows, ⌃ ⌥ ⇧, Paste, New pane), or its label in
 * JetBrains Mono 13 ("Esc", "⌃D", "F5"), a word as wide as it needs. A modifier before a word
 * (⇧Tab) is its icon at 13 pt. Colour means a state only: the accent tint marks a held modifier.
 */
export function KeyCap({
  def,
  color = colors.text,
  active,
  ground = colors.surface,
}: {
  def: Pick<ControlKey, "label" | "icon" | "lead">;
  color?: string;
  active?: boolean;
  /** The key's ground (Settings › Keys draws its rows' keys on the page). */
  ground?: string;
}) {
  const fg = active ? colors.accent : color;
  // the lead icon stands for the label's first character (the ⇧ of ⇧Tab)
  const text = def.lead ? [...def.label].slice(1).join("") : def.label;
  const word = !def.icon && [...def.label].length > 2;
  return (
    <View style={[styles.key, { backgroundColor: ground }, word && styles.keyWord, active && styles.keyActive]}>
      {def.icon ? (
        <Icon name={def.icon} size={18} weight="medium" color={fg} />
      ) : (
        <>
          {def.lead ? <Icon name={def.lead} size={13} weight="medium" color={fg} /> : null}
          <Text style={[styles.label, { color: fg }]}>{text}</Text>
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

/** The tip card: the key's name in bold, what it sends under it. Not touchable; the row positions it above itself. */
export function KeyTip({ tip, below }: { tip: Tip; below?: boolean }) {
  return (
    <View pointerEvents="none" style={[styles.tip, below && styles.tipBelow]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <Text style={styles.tipTitle}>{tip.title}</Text>
      <Text style={styles.tipText}>{tip.text}</Text>
    </View>
  );
}

/**
 * A key: a tap sends it; holding it for 450 ms shows its tip (`onTip`) and lifting the finger hides
 * the tip without sending anything. The tip is also the screen reader's hint.
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
  def: Pick<ControlKey, "label" | "icon" | "lead">;
  spoken: string;
  onPress: () => void;
  active?: boolean;
  disabled: boolean;
  color?: string;
  hint?: string;
  tip?: Tip;
  onTip?: (tip: Tip | null) => void;
}) {
  const [gate] = useState(() => new PressGate());
  return (
    <Pressable
      // the modifiers stay on for the next key: toggle buttons, announced as on or off
      {...(active === undefined ? { accessibilityRole: "button" as const, accessibilityState: { disabled } } : toggleA11y(active, disabled))}
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
      style={({ pressed }) => [styles.slot, disabled && styles.disabled, pressed && styles.pressed]}
    >
      <KeyCap def={def} color={color} active={active} />
    </Pressable>
  );
}

const icon = (name: IconName, label: string) => ({ icon: name, label });

/**
 * The control-key row (Settings › Keys): `onKey` gets a key's protocol name, the sticky ctrl /
 * alt / shift toggle `sticky`, the new tab / new pane keys `onOpen` (disabled without it: the
 * computer's Rift cannot open them), Paste `onPaste` (disabled without it: the computer's Rift
 * takes no text). Above the command line it is one line that scrolls, led by Hide keyboard
 * (`onHideKeyboard`, while the keyboard is up) and ending in Edit keys (`onEdit`, Settings ›
 * Keys); under a full-screen program it wraps (`wrap`). Its keys are 52 pt wide in 44 pt
 * touch targets, so the row scrolls sideways; that scroll never moves the screen (one
 * direction only, no bounce). Nothing when the row is empty and there is no edit key.
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
  onEdit,
  onHideKeyboard,
  onShowKeyboard,
  wrap,
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
  onEdit?: () => void;
  onHideKeyboard?: () => void;
  /** The keyboard is down: the last key opens it (the terminal view; the command line has its own field). */
  onShowKeyboard?: () => void;
  wrap?: boolean;
}) {
  const [tip, setTip] = useState<Tip | null>(null);
  if (ids.length === 0 && !onEdit && !onHideKeyboard && !onShowKeyboard) return null;
  const keys = ids.map((id) => {
    const def: ControlKey | undefined = controlKey(id);
    if (!def) return null;
    // Interrupt in the caution colour (red is below 4.5:1 on a surface, palette.ts)
    const color = def.danger ? colors.warning : colors.text;
    if (def.sticky) {
      const mod = modOf(id);
      return (
        <Key
          key={id}
          tip={tipOf(def)}
          onTip={setTip}
          def={def}
          spoken={def.spoken}
          color={color}
          disabled={disabled}
          active={sticky[mod]}
          onPress={() => onSticky({ ...sticky, [mod]: !sticky[mod] })}
        />
      );
    }
    if (def.open) {
      const kind = def.open;
      return (
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
        />
      );
    }
    if (def.paste) {
      if (hasSystemPasteButton && systemPaste && onPaste && !disabled) {
        return <SystemPasteButton key={id} {...systemPaste} style={[styles.key, styles.slot, { backgroundColor: colors.surface }]} />;
      }
      return (
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
        />
      );
    }
    return (
      <Key key={id} def={def} spoken={def.spoken} tip={tipOf(def)} onTip={setTip} color={color} disabled={disabled} onPress={() => onKey(id)} />
    );
  });
  // the last key, outside the scrolling keys so it never scrolls away: muted
  const hide = onHideKeyboard ? (
    <Key key="hide" def={icon("hideKeyboard", "⌨")} spoken="Hide keyboard" tip={HIDE_TIP} onTip={setTip} color={colors.textMuted} disabled={false} onPress={onHideKeyboard} />
  ) : onShowKeyboard ? (
    <Key key="show" def={icon("keyboard", "⌨")} spoken="Show keyboard" tip={SHOW_TIP} onTip={setTip} color={colors.textMuted} disabled={disabled} onPress={onShowKeyboard} />
  ) : null;
  const edit = onEdit ? (
    <Key
      key="edit"
      def={icon("editKeys", "⌨")}
      spoken="Edit keys"
      hint="Settings › Keys: choose and order these keys"
      color={colors.textMuted}
      disabled={false}
      onPress={onEdit}
      tip={{ title: "Edit keys", text: "Settings › Keys: choose and order these keys" }}
      onTip={setTip}
    />
  ) : null;
  const card = tip ? <KeyTip tip={tip} /> : null;
  if (wrap) {
    return (
      <View style={styles.host}>
        {card}
        <View style={[styles.keys, styles.wrap]} accessibilityRole="toolbar" accessibilityLabel="Keys">
          {keys}
          {edit}
          {hide}
        </View>
      </View>
    );
  }
  return (
    <View style={styles.host}>
      {card}
      <View style={styles.line}>
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={styles.scroll}
          contentContainerStyle={styles.keys}
          // sideways only: a swipe on the keys never drags the page or the keyboard with it
          directionalLockEnabled
          alwaysBounceVertical={false}
          bounces={false}
          overScrollMode="never"
          nestedScrollEnabled
          keyboardShouldPersistTaps="always"
          accessibilityRole="toolbar"
          accessibilityLabel="Keys"
        >
          {keys}
          {edit}
        </ScrollView>
        {hide}
      </View>
    </View>
  );
}

// The Keys board's keys (KeysNormalized): 52 x 44 with a 12 pt radius, the key itself the 44 pt slot (the row takes
// 4 pt back above and below, as the boards do), a word key as wide as its word, on the surface
// with a glass rim; the accent tint and rim for a modifier that is on.
const styles = StyleSheet.create({
  host: { zIndex: 2 },
  line: { flexDirection: "row", alignItems: "center", gap: 6, marginVertical: -4 },
  scroll: { flex: 1, flexGrow: 1 },
  // the tip floats above the row, over what is there: a card on the raised ground
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
    backgroundColor: "#2b2b2b",
    borderWidth: 1,
    borderColor: colors.glassRim,
    elevation: 12,
    zIndex: 10,
  },
  // where there is no room above (Settings › Keys, under its header): under the keys
  tipBelow: { bottom: undefined, top: "100%", marginBottom: 0, marginTop: 8 },
  tipTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 14 },
  tipText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17 },
  keys: { gap: 6 },
  wrap: { flexDirection: "row", flexWrap: "wrap", marginVertical: -4 },
  // the 44 x 44 touch target around a key
  slot: { minWidth: TOUCH, minHeight: TOUCH, alignItems: "center", justifyContent: "center" },
  key: {
    width: KEY_W,
    height: TOUCH,
    borderRadius: 12,
    flexDirection: "row",
    gap: 3,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: colors.glassRim,
  },
  keyWord: { width: undefined, minWidth: KEY_W, paddingHorizontal: 12 },
  // a held modifier: the accent tint and rim, its icon in the accent
  keyActive: { backgroundColor: tints.accentBadge, borderColor: colors.accent },
  label: { fontFamily: monoMedium, fontSize: 13, letterSpacing: 0.2 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
