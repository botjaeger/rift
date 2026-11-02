import { useSyncExternalStore } from "react";
import { pending, type PendingView } from "@/pending";

/** What waits in the message box of the pane `key` (`draftKey`): its items and pasted text. */
export function usePending(key: string): PendingView {
  return useSyncExternalStore(pending.subscribe, () => pending.view(key));
}
