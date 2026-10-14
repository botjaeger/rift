// Which lock-screen Live Activities to start, update and end (the P6 Lock board): one per
// "Notify me" wait (a run from this phone with "Notify me when my commands finish", or the
// running card's Notify when done), ended by its `done`. Pure; runtime.ts applies the plan.

import type { ActivityItem, AskedView, HostView } from "./fleet";
import { findPane, tabTitle } from "./topology";

/**
 * An activity shown now: the wait it shows, its id (null while the start is in flight), the line
 * it shows, and, once its wait ended while the start was still in flight, the exit status to end
 * it with.
 */
export interface LiveShown {
  wait: AskedView;
  id: string | null;
  line: string;
  ended?: { exit: number | null };
}

export interface LivePlan {
  start: { key: string; wait: AskedView; command: string; place: string; startedAt: number; line: string }[];
  update: { key: string; id: string; line: string }[];
  end: { key: string; id: string | null; exit: number | null; line: string }[];
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
        place: [h?.record.name, loc ? tabTitle(loc.tab) : null].filter(Boolean).join(" · "),
        startedAt: started != null ? started * 1000 : a.at,
        line,
      });
    } else if (cur.id !== null && line !== "" && line !== cur.line) plan.update.push({ key, id: cur.id, line });
  }
  for (const [key, cur] of shown) {
    if (live.has(key)) continue;
    const { host, pane, command } = cur.wait;
    const done = finished.find((f) => f.host === host && f.pane === pane && f.command === command);
    plan.end.push({ key, id: cur.id, exit: done ? done.exit : null, line: cur.line });
  }
  return plan;
}
