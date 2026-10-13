// Lookups over a host's topology and pane state, shared by the screens. Pure.

import type { PaneState } from "./protocol/store";
import type { Pane, Tab, Window } from "./protocol/types";

export interface PaneLocation {
  window: Window;
  windowIndex: number;
  tab: Tab;
  pane: Pane;
  paneIndex: number;
}

export function findPane(windows: Window[], paneId: number): PaneLocation | null {
  for (const [windowIndex, window] of windows.entries())
    for (const tab of window.tabs)
      for (const [paneIndex, pane] of tab.panes.entries())
        if (pane.id === paneId) return { window, windowIndex, tab, pane, paneIndex };
  return null;
}

export const tabTitle = (tab: Tab) => tab.title || `Tab ${tab.index + 1}`;

/** "rift › pane 2" (the pane number only when the tab is split). */
export function paneTitle(loc: PaneLocation): string {
  return loc.tab.panes.length > 1 ? `${tabTitle(loc.tab)} › pane ${loc.paneIndex + 1}` : tabTitle(loc.tab);
}

/** What the pane is running, or null when it is at a prompt. */
export function runningCommand(pane: Pane | null, state: PaneState | null): string | null {
  const last = state?.blocks.at(-1);
  if (last?.state === "running") return last.command || pane?.job || "a program";
  if (state && last?.state === "done") return null;
  return pane?.job ?? null;
}

/** The newest branch the pane's blocks report, if any. */
export function paneBranch(state: PaneState | null): string | null {
  const blocks = state?.blocks ?? [];
  for (let i = blocks.length - 1; i >= 0; i--) if (blocks[i]!.branch) return blocks[i]!.branch;
  return null;
}
