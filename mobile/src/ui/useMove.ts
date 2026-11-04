import { AccessibilityInfo } from "react-native";
import type { MoveKind } from "@/protocol/types";
import { movedText } from "@/reorder";
import { fleet } from "@/runtime";

/**
 * Reorder a computer's tabs or a tab's panes (feature `move`): sends the move, then says where the
 * row went (`onDone`, else the screen reader). The new order comes back in the topology. Refusals
 * (a pinned tab leaving its zone, another window's place) go to `onError`.
 */
export function useMove(
  hostKey: string,
  onError: (message: string | null) => void,
  onDone?: (text: string) => void,
): (pane: number, kind: MoveKind, to: number, name: string, place: number, count: number) => void {
  return (pane, kind, to, name, place, count) => {
    onError(null);
    void fleet.moveShell(hostKey, pane, kind, to).then((r) => {
      if (!r.ok) return onError(r.message);
      const text = movedText(name, place, count);
      if (onDone) onDone(text);
      else AccessibilityInfo.announceForAccessibility(text);
    });
  };
}
