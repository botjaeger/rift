import { canOperate, hasFeature, type HostView } from "@/fleet";
import { findPane } from "@/topology";
import { authenticate, fleet } from "@/runtime";
import { canStartTab } from "./StartTab";

/** The pane a new tab is opened from while Rift shows windows: the active tab's active pane, else any. */
export function basePane(host: HostView): number | null {
  const tabs = host.windows.filter((w) => !w.background).flatMap((w) => w.tabs);
  const tab = tabs.find((t) => t.active) ?? tabs[0];
  return (tab?.panes.find((p) => p.active) ?? tab?.panes[0])?.id ?? null;
}

/** Whether "New tab" can work on `host` now: a shell while Rift is closed (`start`), else a tab beside a pane (`open`). */
export function canNewTab(host: HostView): boolean {
  if (host.conn.status !== "open" || !canOperate(host)) return false;
  return canStartTab(host) || (hasFeature(host, "open") && basePane(host) !== null);
}

/** Whether "New pane" can work beside `pane` on `host` now (feature `open`). */
export function canNewPane(host: HostView, pane: number): boolean {
  const loc = findPane(host.windows, pane);
  return host.conn.status === "open" && canOperate(host) && hasFeature(host, "open") && !!loc && !loc.window.background;
}

/** The folder a new tab starts in: the one `basePane` is in; null when it is not known. */
export function newTabCwd(host: HostView): string | null {
  const base = basePane(host);
  return (base === null ? null : findPane(host.windows, base)?.pane.cwd) ?? null;
}

/**
 * Opens a tab on `host`: with Rift closed, Face ID first and a shell the computer keeps (as
 * StartTab); else a tab in its window. `pane`: the new pane to go to; `message`: why not (null:
 * the user cancelled).
 */
export async function openNewTab(host: HostView, current: () => boolean = () => true): Promise<{ ok: true; pane: number | undefined } | { ok: false; message: string | null }> {
  const key = host.record.key;
  if (canStartTab(host)) {
    const auth = await authenticate(`Open a tab on ${host.record.name}`);
    if (!current()) return { ok: false, message: null };
    if (!auth.ok) return { ok: false, message: auth.message === "Cancelled." ? null : auth.message };
    const r = await fleet.startShell(key);
    return r.ok ? { ok: true, pane: r.pane } : { ok: false, message: r.message };
  }
  const base = basePane(host);
  if (base === null) return { ok: false, message: `${host.record.name} has no tab to open one beside.` };
  const r = await fleet.openShell(key, base, "tab");
  return r.ok ? { ok: true, pane: r.pane } : { ok: false, message: r.message };
}
