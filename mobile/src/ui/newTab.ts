import { canOperate, hasFeature, type HostView } from "@/fleet";
import { authenticate, fleet } from "@/runtime";
import { canStartTab } from "./StartTab";

/** The pane a new tab is opened from while Rift shows windows: the active tab's active pane, else any. */
function basePane(host: HostView): number | null {
  const tabs = host.windows.filter((w) => !w.background).flatMap((w) => w.tabs);
  const tab = tabs.find((t) => t.active) ?? tabs[0];
  return (tab?.panes.find((p) => p.active) ?? tab?.panes[0])?.id ?? null;
}

/** Whether "New tab" can work on `host` now: a shell while Rift is closed (`start`), else a tab beside a pane (`open`). */
export function canNewTab(host: HostView): boolean {
  if (host.conn.status !== "open" || !canOperate(host)) return false;
  return canStartTab(host) || (hasFeature(host, "open") && basePane(host) !== null);
}

/**
 * Opens a tab on `host`: with Rift closed, Face ID first and a shell the computer keeps (as
 * StartTab); else a tab in its window. `pane`: the new pane to go to; `message`: why not (null:
 * the user cancelled).
 */
export async function openNewTab(host: HostView): Promise<{ ok: true; pane: number | undefined } | { ok: false; message: string | null }> {
  const key = host.record.key;
  if (canStartTab(host)) {
    const auth = await authenticate(`Open a tab on ${host.record.name}`);
    if (!auth.ok) return { ok: false, message: auth.message === "Cancelled." ? null : auth.message };
    const r = await fleet.startShell(key);
    return r.ok ? { ok: true, pane: r.pane } : { ok: false, message: r.message };
  }
  const base = basePane(host);
  if (base === null) return { ok: false, message: `${host.record.name} has no tab to open one beside.` };
  const r = await fleet.openShell(key, base, "tab");
  return r.ok ? { ok: true, pane: r.pane } : { ok: false, message: r.message };
}
