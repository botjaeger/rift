// The words of the Settings pages (the Settings board), free of React Native imports (tested in node).

import type { ConnState } from "../protocol/client";
import { doneText } from "../notifyText";
import { updateStatusText } from "./format";

/** "30 s", "1 min", "5 min": the "Only if it ran longer than" value. */
export const shortAfter = (ms: number) => (ms < 60_000 ? `${Math.round(ms / 1000)} s` : `${Math.round(ms / 60_000)} min`);

/** Where a computer is, in words: "This network" or "Over the relay" while connected, else what stops it. */
export function placeWord(conn: Pick<ConnState, "status" | "path">): string {
  switch (conn.status) {
    case "open":
      return conn.path?.kind === "relay" ? "Over the relay" : "This network";
    case "connecting":
      return "Connecting…";
    case "offline":
      return "Offline";
    case "idle":
      return "Not connected";
    case "closed":
    case "failed":
      return "Can’t reach it";
  }
}

/** A computer's line on Settings: "This network · can run commands"; one it cannot reach says only that. */
export function hostMeta(host: { conn: Pick<ConnState, "status" | "path">; viewOnly: boolean }): string {
  const where = placeWord(host.conn);
  return host.conn.status === "open" ? `${where} · ${host.viewOnly ? "view only" : "can run commands"}` : where;
}

/** "Show the command"'s detail: the example alert for the current value, as the app posts it. */
export function notifyExample(hostName: string, showCommand: boolean): string {
  const t = doneText({ host: hostName, tab: null, command: "npm test", exit: 1, durationMs: 8400, showCommand });
  return `“${t.title}” · ${t.subtitle}`;
}

/** The "Updates" line: its text and whether it is the green "just checked" line. */
export function updateLine(
  check: { state: "idle" | "checking" | "downloaded" | "none" | "failed"; at: number | null },
  pending: boolean,
  now: number,
): { text: string; fresh: boolean } {
  if (pending) return { text: updateStatusText(check, pending), fresh: false };
  if (check.state === "checking") return { text: "Checking…", fresh: false };
  if (check.state === "none" && check.at !== null && now - check.at < 60_000) return { text: "Up to date · checked just now", fresh: true };
  return { text: updateStatusText(check, pending), fresh: false };
}

/** The unpair page's note: what the second tap does, and what is still to do on the computer. */
export function unpairFooter(armed: boolean, name: string, connected: boolean): string {
  if (!armed) return "To pair again you need a new code from Rift on the computer.";
  return connected
    ? `${name} forgets this phone. Pairing again needs a new code.`
    : `This phone forgets ${name}. It is not connected now, so remove this phone on the computer too (Settings › Remote access).`;
}

/** The ntfy row's value on Settings. */
export const ntfySummary = (on: boolean) => (on ? "On" : "Off");
