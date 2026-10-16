import { useRouter } from "expo-router";
import { useRef } from "react";
import { AccessibilityInfo } from "react-native";
import { afterClose, closedText, closeConfirm, closesCurrent, runningCount } from "@/closing";
import type { CloseKind } from "@/protocol/types";
import { fleet } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { confirmDestructive } from "./confirm";

/**
 * Close a pane or tab of a computer (feature `close`): asks first when a command runs in it,
 * closes, and says so (`onDone`, else the screen reader). When the pane on screen (`current`) is
 * among what closed, the screen moves to a pane beside it, else to Computers; `afterSheet` also
 * dismisses the sheet the close was started from. Refusals go to `onError`.
 */
export function useClose(
  hostKey: string,
  current: number,
  onError: (message: string | null) => void,
  onDone?: (text: string) => void,
  afterSheet?: boolean,
): (paneId: number, kind: CloseKind) => void {
  const router = useRouter();
  // one at a time: a second tap while the first is out would close two
  const busy = useRef(false);
  return (paneId, kind) => {
    if (busy.current) return;
    const windows = fleet.state.hosts.find((h) => h.record.key === hostKey)?.windows ?? [];
    const loc = findPane(windows, paneId);
    if (!loc) return;
    const title = kind === "tab" ? tabTitle(loc.tab) : `${tabTitle(loc.tab)} · pane ${loc.paneIndex + 1}`;
    const ask = closeConfirm(kind, title, runningCount(loc.tab, paneId, kind));
    busy.current = true;
    void (async () => {
      try {
        if (ask && !(await confirmDestructive(ask.title, ask.message, ask.action))) return;
        onError(null);
        const r = await fleet.closeShell(hostKey, paneId, kind);
        if (!r.ok) return onError(r.message);
        const text = closedText(kind, title);
        if (!closesCurrent(loc.tab, current, paneId, kind)) {
          if (onDone) onDone(text);
          else AccessibilityInfo.announceForAccessibility(text);
          return;
        }
        AccessibilityInfo.announceForAccessibility(text);
        const next = afterClose(windows, paneId, kind);
        if (afterSheet) router.dismiss();
        if (next === null) router.dismissTo("/");
        else
          router.replace({
            pathname: "/host/[key]/pane/[id]",
            params: { key: hostKey, id: String(next), toast: text },
          });
      } finally {
        busy.current = false;
      }
    })();
  };
}
