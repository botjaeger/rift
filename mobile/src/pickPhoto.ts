// The phone's photo pickers for the Attach sheet (rules and messages: photo.ts).

import * as ImagePicker from "expo-image-picker";
import { pickedPhoto, PICKED_QUALITY } from "./photo";

const options = { mediaTypes: ["images"] as ImagePicker.MediaType[], base64: true, quality: PICKED_QUALITY };

/** From the phone's library. */
export const pickLibraryPhoto = () => pickedPhoto({ launch: () => ImagePicker.launchImageLibraryAsync(options) });

/** With the phone's camera, after its permission. */
export const takePhoto = () =>
  pickedPhoto({ camera: () => ImagePicker.requestCameraPermissionsAsync(), launch: () => ImagePicker.launchCameraAsync(options) });
