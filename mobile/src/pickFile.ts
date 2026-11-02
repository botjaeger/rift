// Picking a file for an `upload` (the Attach sheet's File): the system's document picker, read
// only after its name and stated size pass (upload.ts `pickedAttachment`).

import * as DocumentPicker from "expo-document-picker";
import { dataUrlBase64, pickedAttachment, type ClipboardImage } from "./upload";

/** The file at `uri` in base64. */
async function readBase64(uri: string): Promise<string> {
  const blob = await (await fetch(uri)).blob();
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("unreadable"));
    reader.onload = () => resolve(dataUrlBase64(String(reader.result)));
    reader.readAsDataURL(blob);
  });
}

/** The file the user picks, to send with `fleet.uploadImage`, and its name for the chip; `file` null when they cancel. */
export async function pickFile(): Promise<{ file: ClipboardImage; name?: string }> {
  let name: string | undefined;
  const file = await pickedAttachment(async () => {
    const r = await DocumentPicker.getDocumentAsync({ type: "*/*", copyToCacheDirectory: true, multiple: false });
    name = r.assets?.[0]?.name;
    return r;
  }, readBase64);
  return { file, name };
}
