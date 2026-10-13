// Web fallback of the key capture view. Terminal mode: a visually hidden textarea holds the
// focus; `keydown` reports named keys and ctrl/alt combinations, `beforeinput` and composition
// report typed text (soft keyboards send "Unidentified" keydowns, so their text arrives there).
// Shortcuts mode: a capturing `keydown` listener on the wrapper takes ctrl/alt combinations
// that are keys, Escape, F-keys and (`captureTab`) Tab / Shift+Tab from the field inside it.

import { useEffect, useImperativeHandle, useRef } from "react";
import { Pressable, View } from "react-native";
import { keyEventToAction } from "../../../src/keyboard";
import type { KeyCaptureViewProps, KeyEvent } from "./types";

const MODIFIERS = new Set(["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "Fn", "OS", "Hyper", "Super"]);

function toEvent(e: KeyboardEvent): KeyEvent {
  return {
    key: e.key,
    code: e.code,
    // the browser's key is already what the layout composed (AltGr+Q "@", Option+2 "™")
    char: e.key,
    altGraph: e.getModifierState("AltGraph"),
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    meta: e.metaKey,
  };
}

/** A key the terminal takes on keydown (the rest is text, delivered by beforeinput). */
function terminalKey(e: KeyboardEvent): boolean {
  if (e.metaKey || e.isComposing || MODIFIERS.has(e.key)) return false;
  if (e.key === "Unidentified" || e.key === "Dead" || e.key === "Process") return false;
  return e.key.length > 1 || e.ctrlKey || e.altKey;
}

/**
 * A key the composer's shortcuts take from its field: ctrl/alt combinations that are keys
 * (characters composed with AltGr / Option are typed into the field), Escape, F-keys, and Tab
 * when `captureTab`.
 */
function shortcutKey(e: KeyboardEvent, captureTab: boolean): boolean {
  if (e.metaKey || e.isComposing || MODIFIERS.has(e.key)) return false;
  if (e.key === "Tab") return captureTab;
  if (e.key === "Escape" || /^F([1-9]|1[0-2])$/.test(e.key)) return true;
  if (!(e.ctrlKey || e.altKey) || [...e.key].length !== 1) return false;
  const action = keyEventToAction(toEvent(e));
  return action !== null && "keys" in action;
}

export function KeyCaptureView({
  mode,
  onText,
  onKey,
  onFocusChange,
  captureTab = false,
  style,
  children,
  accessibilityLabel,
  accessibilityHint,
  ref,
}: KeyCaptureViewProps) {
  const input = useRef<HTMLTextAreaElement>(null);
  const wrapper = useRef<View>(null);
  const handlers = useRef({ onText, onKey, onFocusChange, captureTab });
  useEffect(() => {
    handlers.current = { onText, onKey, onFocusChange, captureTab };
  });

  useImperativeHandle(
    ref,
    () => ({
      focus: () => input.current?.focus(),
      blur: () => input.current?.blur(),
    }),
    [],
  );

  useEffect(() => {
    if (mode !== "shortcuts") return;
    const node = wrapper.current as unknown as HTMLElement | null;
    if (!node?.addEventListener) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (!shortcutKey(e, handlers.current.captureTab)) return;
      e.preventDefault();
      e.stopPropagation();
      handlers.current.onKey?.(toEvent(e));
    };
    node.addEventListener("keydown", onKeyDown, true);
    return () => node.removeEventListener("keydown", onKeyDown, true);
  }, [mode]);

  useEffect(() => {
    const el = input.current;
    if (mode !== "terminal" || !el) return;
    const clear = () => {
      el.value = "";
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (!terminalKey(e)) return;
      e.preventDefault();
      handlers.current.onKey?.(toEvent(e));
    };
    const onBeforeInput = (e: InputEvent) => {
      if (e.isComposing || e.inputType === "insertCompositionText") return;
      e.preventDefault();
      const key = (k: string) => handlers.current.onKey?.({ key: k, code: "", ctrl: false, alt: false, shift: false, meta: false });
      if (e.inputType === "insertText" && e.data) handlers.current.onText?.({ text: e.data });
      else if (e.inputType === "insertLineBreak" || e.inputType === "insertParagraph") key("Enter");
      else if (e.inputType === "deleteContentBackward") key("Backspace");
      else if (e.inputType === "deleteContentForward") key("Delete");
    };
    const onCompositionEnd = (e: CompositionEvent) => {
      if (e.data) handlers.current.onText?.({ text: e.data });
      clear();
    };
    const onPaste = (e: ClipboardEvent) => {
      e.preventDefault();
      const text = e.clipboardData?.getData("text/plain");
      if (text) handlers.current.onText?.({ text });
    };
    const onFocus = () => handlers.current.onFocusChange?.(true);
    const onBlur = () => handlers.current.onFocusChange?.(false);
    el.addEventListener("keydown", onKeyDown);
    el.addEventListener("beforeinput", onBeforeInput);
    el.addEventListener("input", clear);
    el.addEventListener("compositionend", onCompositionEnd);
    el.addEventListener("paste", onPaste);
    el.addEventListener("focus", onFocus);
    el.addEventListener("blur", onBlur);
    return () => {
      el.removeEventListener("keydown", onKeyDown);
      el.removeEventListener("beforeinput", onBeforeInput);
      el.removeEventListener("input", clear);
      el.removeEventListener("compositionend", onCompositionEnd);
      el.removeEventListener("paste", onPaste);
      el.removeEventListener("focus", onFocus);
      el.removeEventListener("blur", onBlur);
    };
  }, [mode]);

  if (mode === "shortcuts") {
    return (
      <View ref={wrapper} style={style}>
        {children}
      </View>
    );
  }
  return (
    <Pressable
      style={style}
      onPress={() => input.current?.focus()}
      accessibilityLabel={accessibilityLabel}
      accessibilityHint={accessibilityHint}
    >
      {children}
      <textarea
        ref={input}
        aria-label={accessibilityLabel ?? "Terminal input"}
        autoCapitalize="off"
        autoCorrect="off"
        autoComplete="off"
        spellCheck={false}
        style={hidden}
      />
    </Pressable>
  );
}

// Off screen but focusable (display: none would refuse the focus).
const hidden = {
  position: "absolute",
  left: 0,
  bottom: 0,
  width: 1,
  height: 1,
  opacity: 0,
  border: 0,
  padding: 0,
  resize: "none",
  pointerEvents: "none",
} as const;
