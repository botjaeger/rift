import type { ReactNode, Ref } from "react";
import type { StyleProp, ViewStyle } from "react-native";

/**
 * `terminal`: the view takes focus itself and reports every key and all typed text (the soft
 * keyboard included). `shortcuts`: the view wraps a text field and takes from it only hardware
 * ctrl/alt combinations that are keys (not characters composed with AltGr / Option, such as
 * "@" or "|"), Escape, F-keys and, with `captureTab`, Tab / Shift+Tab; everything else reaches
 * the field. Cmd / Meta combinations are always left to the system.
 */
export type KeyCaptureMode = "terminal" | "shortcuts";

export interface TextEvent {
  /** Text the keyboard inserted (may hold line breaks, tabs or other control characters). */
  text: string;
}

export interface KeyEvent {
  /** W3C KeyboardEvent.key: "ArrowUp", "Escape", "F5", "Enter", or the character ("c", "C", "["). */
  key: string;
  /** W3C KeyboardEvent.code where known ("KeyC"); may be empty. */
  code: string;
  /**
   * With ctrl / alt: the character the layout composes with every modifier held (AltGr+Q "@",
   * Option+2 "™"); empty when none.
   */
  char?: string;
  /** AltGr is held (Android right Alt, the web's AltGraph state). */
  altGraph?: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export interface KeyCaptureHandle {
  /** Takes the focus (terminal mode: shows the soft keyboard). */
  focus(): void;
  /** Gives the focus up (hides the soft keyboard). */
  blur(): void;
}

export interface KeyCaptureViewProps {
  mode: KeyCaptureMode;
  onText?: (e: TextEvent) => void;
  onKey?: (e: KeyEvent) => void;
  /** Terminal mode: the system asked to paste (⌘V, the keyboard's Paste key). */
  onPaste?: () => void;
  /** Terminal mode: the view took or lost the focus. */
  onFocusChange?: (focused: boolean) => void;
  /** Shortcuts mode: take Tab / Shift+Tab from the field (else they move the focus). */
  captureTab?: boolean;
  style?: StyleProp<ViewStyle>;
  children?: ReactNode;
  accessibilityLabel?: string;
  accessibilityHint?: string;
  ref?: Ref<KeyCaptureHandle>;
}
