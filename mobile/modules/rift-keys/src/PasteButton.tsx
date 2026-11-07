import { requireNativeView } from "expo";
import type { NativeSyntheticEvent, StyleProp, ViewStyle } from "react-native";
import { Platform } from "react-native";

/** What ios/RiftPasteButton.swift reads of a file (upload.ts `NativeFile`). */
export interface PastedFile {
  status: "ok" | "too_large" | "unreadable";
  data?: string;
  name?: string;
}

interface PasteButtonProps {
  onText: (text: string) => void;
  /** The image as base64 (PNG, or JPEG when large). */
  onImage: (b64: string) => void;
  /** A file that is neither: its name and bytes, or why it could not be read. */
  onFile: (file: PastedFile) => void;
  /** The control draws its icon and the system's word "Paste" (a row of the Attach sheet), not the word alone (a key). */
  labelled?: boolean;
  /** The theme's ground and ink for the control (`#rrggbb` or `#rrggbbaa`). */
  ground?: string;
  ink?: string;
  style?: StyleProp<ViewStyle>;
}

interface NativeProps {
  onPasteText: (e: NativeSyntheticEvent<{ text: string }>) => void;
  onPasteImage: (e: NativeSyntheticEvent<{ data: string }>) => void;
  onPasteFile: (e: NativeSyntheticEvent<PastedFile>) => void;
  labelled?: boolean;
  ground?: string;
  ink?: string;
  style?: StyleProp<ViewStyle>;
}

const NativeButton = Platform.OS === "ios" ? requireNativeView<NativeProps>("RiftPaste") : null;

/** iOS's own paste button: reads the clipboard with no prompt (ios/RiftPasteButton.swift). */
export const hasSystemPasteButton = NativeButton !== null;

export function SystemPasteButton({ onText, onImage, onFile, labelled, ground, ink, style }: PasteButtonProps) {
  if (!NativeButton) return null;
  return (
    <NativeButton
      onPasteText={(e) => onText(e.nativeEvent.text)}
      onPasteImage={(e) => onImage(e.nativeEvent.data)}
      onPasteFile={(e) => onFile(e.nativeEvent)}
      labelled={labelled}
      ground={ground}
      ink={ink}
      style={style}
    />
  );
}
