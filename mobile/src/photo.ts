// A photo for the Attach sheet, from the library or the camera: the rules, with the picker passed
// in (pickPhoto.ts is the phone's, a fake in tests).

import { photoAttachment } from "./pending";
import { MAX_UPLOAD_BYTES } from "./protocol/types";
import type { ClipboardImage } from "./upload";

/** A picked photo's JPEG quality: well within the upload limit, sharp enough to read. */
export const PICKED_QUALITY = 0.8;

export const CAMERA_DENIED = "Rift cannot use the camera. Turn it on in the phone's Settings under Rift, then try again.";
export const PHOTO_UNREADABLE = "The phone could not open that photo. Try another.";

/** The pieces of expo-image-picker a pick uses. */
export interface PhotoPicker {
  /** Asks for the camera (iOS refuses a launch without it); absent for the library, which needs no permission. */
  camera?: () => Promise<{ granted: boolean }>;
  launch(): Promise<{ canceled: boolean; assets?: { base64?: string | null }[] | null }>;
}

/** The photo the user picks or takes; null when they cancel, else the photo or why not (as a message). */
export async function pickedPhoto(picker: PhotoPicker): Promise<{ ok: true; b64: string } | { ok: false; message: string } | null> {
  if (picker.camera) {
    const granted = await picker.camera().then(
      (r) => r.granted,
      () => false,
    );
    if (!granted) return { ok: false, message: CAMERA_DENIED };
  }
  let r;
  try {
    r = await picker.launch();
  } catch {
    return { ok: false, message: PHOTO_UNREADABLE };
  }
  if (r.canceled) return null;
  const data = r.assets?.[0]?.base64 ?? null;
  if (data === null) return { ok: false, message: PHOTO_UNREADABLE };
  const photo: ClipboardImage = photoAttachment(data);
  if (photo?.ok) return { ok: true, b64: photo.b64 };
  return { ok: false, message: photo?.reason === "too_large" ? `The photo is larger than ${MAX_UPLOAD_BYTES >> 20} MB.` : PHOTO_UNREADABLE };
}
