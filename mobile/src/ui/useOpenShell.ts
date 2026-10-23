import { useActionScope } from "@/ui/useActionScope";
import { useRouter } from "expo-router";
import { useRef } from "react";
import { AccessibilityInfo } from "react-native";
import { hasFeature } from "@/fleet";
import type { OpenKind } from "@/protocol/types";
import { fleet, useApp } from "@/runtime";

/**
 * The key row's new tab / new pane keys for `paneId` (feature `open`): opens it on the computer,
 * then goes to the new pane in place of this one. Undefined while the computer's Rift cannot
 * open them (the keys show disabled); refusals go to `onError`.
 */
export function useOpenShell(
  hostKey: string,
  paneId: number,
  onError: (message: string | null) => void,
): ((kind: OpenKind) => void) | undefined {
  const router = useRouter();
  const scope = useActionScope(`${hostKey}:${paneId}`);
  const s = useApp();
  // one at a time: a second tap while the first is out would open two
  const busy = useRef(false);
  const host = s.hosts.find((h) => h.record.key === hostKey);
  if (!hasFeature(host, "open")) return undefined;
  return (kind) => {
    if (busy.current) return;
    const current = scope.capture();
    busy.current = true;
    onError(null);
    void fleet.openShell(hostKey, paneId, kind).then((r) => {
      busy.current = false;
      if (!current()) return;
      if (!r.ok) return onError(r.message);
      AccessibilityInfo.announceForAccessibility(kind === "tab" ? "New tab opened" : "New pane opened");
      if (r.pane !== undefined)
        router.replace({ pathname: "/host/[key]/pane/[id]", params: { key: hostKey, id: String(r.pane) } });
    });
  };
}
