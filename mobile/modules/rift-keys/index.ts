// Hardware and soft-keyboard key capture for the terminal view and the composer's shortcuts
// (iOS: ios/KeyCaptureView.swift, Android: android/…/KeyCaptureView.kt, web: the .web.tsx).
export { KeyCaptureView } from "./src/KeyCaptureView";
export { clipboardFiles } from "./src/clipboardFile";
export { hasSystemPasteButton, SystemPasteButton, type PastedFile } from "./src/PasteButton";
export type { KeyCaptureHandle, KeyCaptureMode, KeyCaptureViewProps, KeyEvent, TextEvent } from "./src/types";
