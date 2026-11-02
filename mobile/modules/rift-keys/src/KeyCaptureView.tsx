import { requireNativeView } from "expo";
import { useImperativeHandle, useRef, type ReactNode, type Ref } from "react";
import type { NativeSyntheticEvent, StyleProp, ViewStyle } from "react-native";
import type { KeyCaptureMode, KeyCaptureViewProps, KeyEvent, TextEvent } from "./types";

interface NativeProps {
  mode: KeyCaptureMode;
  captureTab?: boolean;
  onText?: (e: NativeSyntheticEvent<TextEvent>) => void;
  onKey?: (e: NativeSyntheticEvent<KeyEvent>) => void;
  onFocusChange?: (e: NativeSyntheticEvent<{ focused: boolean }>) => void;
  onPaste?: () => void;
  style?: StyleProp<ViewStyle>;
  children?: ReactNode;
  accessibilityLabel?: string;
  accessibilityHint?: string;
}

/** The native view's functions (ios/RiftKeysModule.swift, android/…/RiftKeysModule.kt). */
interface NativeHandle {
  focus(): Promise<void>;
  blur(): Promise<void>;
}

const NativeView = requireNativeView<NativeProps & { ref?: Ref<NativeHandle> }>("RiftKeys");

export function KeyCaptureView({ mode, onText, onKey, onFocusChange, onPaste, ref, ...rest }: KeyCaptureViewProps) {
  const native = useRef<NativeHandle>(null);
  useImperativeHandle(
    ref,
    () => ({
      focus: () => void native.current?.focus().catch(() => undefined),
      blur: () => void native.current?.blur().catch(() => undefined),
    }),
    [],
  );
  return (
    <NativeView
      ref={native}
      mode={mode}
      onText={onText ? (e) => onText(e.nativeEvent) : undefined}
      onKey={onKey ? (e) => onKey(e.nativeEvent) : undefined}
      onFocusChange={onFocusChange ? (e) => onFocusChange(e.nativeEvent.focused) : undefined}
      onPaste={onPaste}
      {...rest}
    />
  );
}
