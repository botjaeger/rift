// Which lock-screen Live Activities to start, update and end (the P6 Lock board): one per
// "Notify me" wait (a run from this phone with "Notify me when my commands finish", or the
// running card's Notify when done), ended by its `done`. Pure; runtime.ts applies the plan.

import type { ActivityItem, AskedView, HostView } from "./fleet";
import { findPane, paneTitle, tabTitle } from "./topology";

/** The pane's screen in the app (the deep link a notification tap opens; push.rs `pane_link`). */
export const paneLink = (host: string, pane: number) => `rift://host/${encodeURIComponent(host)}/pane/${pane}`;

/**
 * An activity shown now: the wait it shows, its id (null while the start is in flight), the line
 * it shows, and, once its wait ended while the start was still in flight, the exit status to end
 * it with.
 */
export interface LiveShown {
  wait: AskedView;
  id: string | null;
  line: string;
  ended?: { exit: number | null; timed: boolean };
}

export interface LivePlan {
  start: { key: string; wait: AskedView; command: string; place: string; link: string; startedAt: number; line: string }[];
  update: { key: string; id: string; line: string }[];
  /** `timed`: it ends now (its `done` came); false when the time it ended is not known (no `done`). */
  end: { key: string; id: string | null; exit: number | null; line: string; timed: boolean }[];
}

export const liveKey = (a: AskedView) => `${a.host}\n${a.pane}\n${a.command}\n${a.at}`;

/**
 * The plan from the activities shown (`shown`, by `liveKey`) to the waits in `asked`: start one
 * for each new wait (its place and start time from the topology, else when asked), update the
 * line where the pane's last line changed, end the ones whose wait is gone with the exit status
 * of the newest finished command matching it.
 */
export function livePlan(
  shown: ReadonlyMap<string, LiveShown>,
  asked: readonly AskedView[],
  hosts: readonly HostView[],
  finished: readonly ActivityItem[],
): LivePlan {
  const plan: LivePlan = { start: [], update: [], end: [] };
  const live = new Set<string>();
  for (const a of asked) {
    const key = liveKey(a);
    live.add(key);
    const h = hosts.find((x) => x.record.key === a.host);
    const loc = h ? findPane(h.windows, a.pane) : null;
    const line = loc?.pane.running?.line ?? "";
    const cur = shown.get(key);
    if (!cur) {
      const started = loc?.pane.running?.started;
      plan.start.push({
        key,
        wait: a,
        command: a.command,
        // the pane's number too when its tab has more than one
        place: [h?.record.name, loc ? (loc.tab.panes.length > 1 ? paneTitle(loc) : tabTitle(loc.tab)) : null]
          .filter(Boolean)
          .join(" · "),
        link: paneLink(a.host, a.pane),
        startedAt: started != null ? started * 1000 : a.at,
        line,
      });
    } else if (cur.id !== null && line !== "" && line !== cur.line) plan.update.push({ key, id: cur.id, line });
  }
  for (const [key, cur] of shown) {
    if (live.has(key)) continue;
    const { host, pane, command } = cur.wait;
    const done = finished.find((f) => f.host === host && f.pane === pane && f.command === command);
    // one that finished while this device was away has no `done`: the pane's last command tells
    const last = findPane(hosts.find((x) => x.record.key === host)?.windows ?? [], pane)?.pane;
    const exit = done ? done.exit : last && !last.running && last.last_command === command ? last.last_exit : null;
    plan.end.push({ key, id: cur.id, exit, line: cur.line, timed: done !== undefined });
  }
  return plan;
}
