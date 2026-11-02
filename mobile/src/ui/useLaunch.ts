import { useActionScope } from "@/ui/useActionScope";
import { useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { AccessibilityInfo } from "react-native";
import type { HostView } from "@/fleet";
import { launchAsks, launchProgram, launchVisit, type Opened, type Program } from "@/launch";
import type { OpenKind } from "@/protocol/types";
import { authenticate, fleet, useApp, useLock } from "@/runtime";
import { openNewTab } from "./newTab";

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * The New tab and New pane sheets' work: a row picked (`pick`) asks first when the program is
 * typed and Settings › Ask before running is on (`ask`, then `go`), Face ID as any Run, opens the
 * tab (or the pane beside `pane`), types the program into it, and goes to the new pane. `from`
 * is where the sheet was opened: "home" (the Portals card, one sheet to close) or a pane's own
 * sheet (Tabs, Panes: two, and the new pane takes the open one's place).
 */
export function useLaunch({ host, kind, pane, from }: { host: HostView | undefined; kind: OpenKind; pane: number | null; from: string | undefined }) {
  const router = useRouter();
  const s = useApp();
  const lock = useLock();
  const hostKey = host?.record.key ?? "";
  const scope = useActionScope(launchVisit(hostKey, kind, pane));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [ask, setAsk] = useState<Program | null>(null);
  // the pane that opened although its program did not start: the sheet offers to go there
  const [opened, setOpened] = useState<number | null>(null);
  // one at a time: a second tap while the first is out would open two
  const out = useRef(false);
  const home = from === "home";

  const close = () => {
    if (scope.leave()) router.back();
  };
  // Presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) close();
  });

  const finish = (target: number | undefined) => {
    if (!scope.leave()) return;
    router.dismiss(home ? 1 : 2);
    if (target === undefined) return;
    const route = { pathname: "/host/[key]/pane/[id]", params: { key: hostKey, id: String(target) } } as const;
    // from a pane's own sheet the new pane takes that pane's place, so Back still leads where it led
    if (home) router.push(route);
    else router.replace(route);
  };

  const go = async (p: Program) => {
    if (!host || out.current) return;
    out.current = true;
    setBusy(true);
    setError(null);
    setOpened(null);
    setAsk(null);
    const current = scope.capture();
    try {
      if (launchAsks(p, s.settings).auth) {
        const auth = await authenticate(`Run ${p.command} on ${host.record.name}`);
        if (!current()) return;
        if (!auth.ok) {
          if (auth.message !== "Cancelled.") setError(auth.message);
          return;
        }
      }
      const open = async (): Promise<Opened> => {
        if (kind === "tab") return openNewTab(host, current);
        if (pane === null) return { ok: false, message: "This pane has closed." };
        const r = await fleet.openShell(hostKey, pane, "pane");
        return r.ok ? { ok: true, pane: r.pane } : { ok: false, message: r.message };
      };
      const r = await launchProgram(p, {
        open,
        run: (target, command) => fleet.run(hostKey, target, command, { notify: false, fromDraft: false }),
        wait: sleep,
        current,
      });
      if (!current()) return;
      if (r.ok) {
        AccessibilityInfo.announceForAccessibility(kind === "tab" ? "New tab opened" : "New pane opened");
        return finish(r.pane);
      }
      if (r.opened !== undefined) setOpened(r.opened);
      if (r.message) setError(r.message);
    } finally {
      out.current = false;
      setBusy(false);
    }
  };

  const pick = (p: Program) => {
    if (out.current) return;
    if (launchAsks(p, s.settings).confirm) {
      setError(null);
      setOpened(null);
      return setAsk(p);
    }
    void go(p);
  };

  return { error, busy, ask, setAsk, opened, pick, go, close, finish };
}
