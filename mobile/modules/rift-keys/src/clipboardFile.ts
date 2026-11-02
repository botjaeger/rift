import { requireOptionalNativeModule } from "expo";

/** What android/…/RiftClipboardModule.kt reads of the clip (upload.ts `NativeFile`). */
export interface ClipFile {
  status: "none" | "ok" | "too_large" | "unreadable";
  data?: string;
  name?: string;
}

/** The clipboard's file on Android, where a clip holds a content URI; null elsewhere and on an older build. */
export const clipboardFiles = requireOptionalNativeModule<{ clipFile(): Promise<ClipFile> }>("RiftClipboard");
