import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { controlKey, keyTipTitle, MAX_ROW_KEYS, PressGate, type ControlKey } from "@/controlKeys";
import type { OpenKind } from "@/protocol/types";
import { Icon, type IconName } from "./Icon";
import { colors, ui } from "./theme";
import { toggleA11y } from "./toggle";

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
 * The boards' round key: 36 pt (40 under a full-screen program), its icon, or its label in
 * Inter; a word ("Esc", "Paste") makes a pill. The accent ground marks a sticky key that is on.
 */
export function KeyCap({
  def,
  color = colors.text,
  tall,
  active,
  ground = colors.surface,
  fill,
}: {
  def: Pick<ControlKey, "label" | "icon">;
  color?: string;
  tall?: boolean;
  active?: boolean;
  /** The key's ground (Settings › Keys draws its rows' keys on the page). */
  ground?: string;
  /** An equal share of the row's width (the row above the command line fits its keys). */
  fill?: boolean;
}) {
  const fg = active ? colors.bg : color;
  const word = !def.icon && [...def.label].length > 2;
  return (
    <View
      style={[
        styles.key,
        { backgroundColor: ground },
        tall && styles.keyTall,
        word && styles.keyWord,
        fill && styles.keyFill,
        active && styles.keyActive,
      ]}
    >
      {def.icon ? (
        <Icon name={def.icon} size={17} color={fg} />
      ) : (
        <Text style={[word ? styles.wordLabel : styles.glyphLabel, { color: fg }]}>{def.label}</Text>
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
  tall,
  hint,
  tip,
  onTip,
  fill,
}: {
  def: Pick<ControlKey, "label" | "icon">;
  spoken: string;
  onPress: () => void;
  active?: boolean;
  disabled: boolean;
  color?: string;
  tall: boolean;
  hint?: string;
  tip?: Tip;
  onTip?: (tip: Tip | null) => void;
  fill?: boolean;
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
      // 36 / 40 pt keys (the boards) reach 44 pt with the slop
      hitSlop={{ top: tall ? 2 : 4, bottom: tall ? 2 : 4 }}
      style={({ pressed }) => [fill && styles.fillSlot, disabled && styles.disabled, pressed && styles.pressed]}
    >
      <KeyCap def={def} color={color} tall={tall} active={active} fill={fill} />
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
 * Keys); under a full-screen program it wraps (`wrap`) with taller keys. Nothing when the row
 * is empty and there is no edit key.
 */
export function ControlRow({
  ids,
  sticky,
  disabled,
  onSticky,
  onKey,
  onOpen,
  onPaste,
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
  onEdit?: () => void;
  onHideKeyboard?: () => void;
  /** The keyboard is down: the last key opens it (the terminal view; the command line has its own field). */
  onShowKeyboard?: () => void;
  wrap?: boolean;
}) {
  const [tip, setTip] = useState<Tip | null>(null);
  if (ids.length === 0 && !onEdit && !onHideKeyboard && !onShowKeyboard) return null;
  const tall = !!wrap;
  // a row the width can hold shares it out evenly; a longer (customised) one scrolls; under a
  // full-screen program the 40 pt keys wrap onto a second line instead
  const fill = !wrap && ids.length <= MAX_ROW_KEYS;
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
          fill={fill}
          def={def}
          spoken={def.spoken}
          color={color}
          tall={tall}
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
          fill={fill}
          def={def}
          spoken={def.spoken}
          hint={onOpen ? undefined : "Update Rift on the computer to open tabs and panes from here"}
          color={color}
          tall={tall}
          disabled={disabled || !onOpen}
          onPress={() => onOpen?.(kind)}
        />
      );
    }
    if (def.paste) {
      return (
        <Key
          key={id}
          tip={tipOf(def)}
          onTip={setTip}
          fill={fill}
          def={def}
          spoken={def.spoken}
          hint={onPaste ? "Types it into the program" : "Update Rift on the computer to paste here"}
          color={color}
          tall={tall}
          disabled={disabled || !onPaste}
          onPress={() => onPaste?.()}
        />
      );
    }
    return (
      <Key key={id} def={def} spoken={def.spoken} tip={tipOf(def)} onTip={setTip} fill={fill} color={color} tall={tall} disabled={disabled} onPress={() => onKey(id)} />
    );
  });
  // the last key, outside the scrolling keys so it never scrolls away: 36 pt, muted
  const hide = onHideKeyboard ? (
    <Key key="hide" def={icon("hideKeyboard", "⌨")} spoken="Hide keyboard" tip={HIDE_TIP} onTip={setTip} color={colors.textMuted} tall={false} disabled={false} onPress={onHideKeyboard} />
  ) : onShowKeyboard ? (
    <Key key="show" def={icon("keyboard", "⌨")} spoken="Show keyboard" tip={SHOW_TIP} onTip={setTip} color={colors.textMuted} tall={false} disabled={disabled} onPress={onShowKeyboard} />
  ) : null;
  const edit = onEdit ? (
    <Key
      key="edit"
      def={icon("editKeys", "⌨")}
      spoken="Edit keys"
      hint="Settings › Keys: choose and order these keys"
      color={colors.textMuted}
      tall={tall}
      disabled={false}
      onPress={onEdit}
      tip={{ title: "Edit keys", text: "Settings › Keys: choose and order these keys" }}
      onTip={setTip}
    />
  ) : null;
  const card = tip ? <KeyTip tip={tip} /> : null;
  if (fill) {
    return (
      <View style={styles.host}>
        {card}
        <View style={styles.line} accessibilityRole="toolbar" accessibilityLabel="Keys">
          {keys}
          {edit}
          {hide}
        </View>
      </View>
    );
  }
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

// The Keys and Pane boards' keys: 36 pt circles (40 under a full-screen program), a word key a
// pill 14 pt in, on the surface with a hairline; the accent ground for a modifier that is on.
const styles = StyleSheet.create({
  host: { zIndex: 2 },
  line: { flexDirection: "row", alignItems: "center", gap: 6 },
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
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  key: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  keyTall: { width: 40, height: 40, borderRadius: 20 },
  keyWord: { width: undefined, minWidth: 36, paddingHorizontal: 14 },
  // an even share of the row: a rounded key as wide as its slot
  fillSlot: { flex: 1, minWidth: 0 },
  keyFill: { width: undefined, alignSelf: "stretch", paddingHorizontal: 0, borderRadius: 10 },
  keyActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  glyphLabel: { fontFamily: ui(400), fontSize: 16 },
  wordLabel: { fontFamily: ui(500), fontSize: 14 },
  disabled: { opacity: 0.45 },
  pressed: { opacity: 0.7 },
});
