// Closing a pane or a tab from the phone (feature `close`): when to offer it, when to ask first,
// and where the screen goes afterwards. Pure.

import type { CloseKind, Pane, Tab, Window } from "./protocol/types";
import { findPane } from "./topology";

/** Whether the pane is running a command (a program, not the prompt). */
export const paneBusy = (p: Pane) => !!(p.running || p.job);

/** How many commands closing ends: the pane's, or every busy pane of its tab. */
export function runningCount(tab: Tab, paneId: number, kind: CloseKind): number {
  const panes = kind === "tab" ? tab.panes : tab.panes.filter((p) => p.id === paneId);
  return panes.filter(paneBusy).length;
}

/**
 * The question asked before closing something with a running command (none: close at once).
 * `title` is the pane's or tab's name.
 */
export function closeConfirm(
  kind: CloseKind,
  title: string,
  running: number,
): { title: string; message: string; action: string } | null {
  if (running <= 0) return null;
  return {
    title: `Close “${title}”?`,
    message: `${running} running ${running === 1 ? "command ends" : "commands end"} on your computer.`,
    action: kind === "tab" ? "Close tab" : "Close pane",
  };
}

/** What the toast says once it is closed. */
export const closedText = (kind: CloseKind, title: string) => `${kind === "tab" ? "Tab" : "Pane"} “${title}” closed`;

/**
 * The pane to show once `paneId` (kind pane) or its tab (kind tab) is closed: a pane beside it in
 * its tab, else the neighbouring tab's active pane, else another tab of the computer; null when
 * nothing is left (the screen goes to Computers). `windows` is the topology before the close.
 */
export function afterClose(windows: Window[], paneId: number, kind: CloseKind): number | null {
  const loc = findPane(windows, paneId);
  if (!loc) return null;
  if (kind === "pane" && loc.tab.panes.length > 1) {
    const beside = loc.tab.panes[loc.paneIndex + 1] ?? loc.tab.panes[loc.paneIndex - 1];
    if (beside) return beside.id;
  }
  const tabs = loc.window.tabs;
  const at = tabs.indexOf(loc.tab);
  const near = tabs[at + 1] ?? tabs[at - 1];
  const others = near ? [near] : windows.flatMap((w) => w.tabs).filter((t) => t !== loc.tab);
  const tab = others[0];
  return tab ? ((tab.panes.find((p) => p.active) ?? tab.panes[0])?.id ?? null) : null;
}

/** Whether a close ends the pane on screen: the pane itself, or its tab. */
export const closesCurrent = (tab: Tab, current: number, paneId: number, kind: CloseKind) =>
  kind === "tab" ? tab.panes.some((p) => p.id === current) : paneId === current;
