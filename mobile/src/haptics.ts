// The phone's small vibrations, one per moment, so every screen means the same thing by them:
// a hold that starts a selection, a handle stepping to the next word, a copy that landed, a row
// lifted to be moved. The system's own settings (iOS System Haptics, Android touch feedback)
// decide whether they are felt; a phone without a vibrator, or the web, feels nothing and nothing fails.

import * as Haptics from "expo-haptics";
import { Platform } from "react-native";

const quietly = (p: Promise<void>) => void p.catch(() => {});
const felt = () => Platform.OS === "ios" || Platform.OS === "android";

/** A hold started a selection. */
export function selectStart(): void {
  if (felt()) quietly(Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light));
}

/** A dragged selection handle stepped onto another word. */
export function selectStep(): void {
  if (felt()) quietly(Haptics.selectionAsync());
}

/** What was selected is on the clipboard. */
export function copied(): void {
  if (felt()) quietly(Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success));
}

/** A row was lifted to be moved. */
export function dragLift(): void {
  if (felt()) quietly(Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium));
}
