// What a pane's page says while it opens (the PaneLoading board): the status strip, the steps
// card, and whether the blocks held in memory are shown dimmed. Free of React Native imports
// (tested in node). The wording is the board's.

import type { ConnState } from "../protocol/client";
import { failureLabel } from "./connLabel";
import { agoText, elapsedText, failNote, retrySeconds, retryText } from "./connStatus";

/** After this long without the pane the card says "Taking longer than usual". */
export const SLOW_MS = 15_000;

export interface PaneStep {
  mark: "✓" | "…" | "" | "✕";
  tone: "done" | "now" | "todo" | "bad";
  label: string;
  note: string;
}

export interface PaneLoad {
  /** The full-width strip under the header; null when nothing needs saying (the pane is open and loaded). */
  strip: { text: string; tone: "busy" | "quiet" | "bad"; busy: boolean; elapsed: string } | null;
  /** The steps card over the skeleton; null when blocks are held or the pane is loaded. */
  card: {
    title: string;
    steps: PaneStep[];
    hint: string | null;
    actions: boolean;
    /** Details only: another try cannot help (the computer refused this device). */
    noRetry?: boolean;
  } | null;
  /** Show what is held in memory dimmed (the pane reconnects). */
  held: boolean;
}

export interface PaneLoadInput {
  conn: ConnState;
  now: number;
  /** Epoch ms this page began waiting (it opened, or Try again was pressed). */
  waitingSince: number;
  hostName: string;
  /** What the page waits for: the blocks, or the pane's screen (agent / terminal view). */
  view: "blocks" | "screen";
  /** The title of the card ("Opening “3.16.0 updates pull”"): see [openingTitle]. */
  opening: string;
  /** The computer's topology lists the pane. */
  paneKnown: boolean;
  /** The blocks (or the screen) are on this page. */
  loaded: boolean;
  /** Epoch ms what is held in memory was last current (the computer last seen); null: nothing is held. */
  heldSince: number | null;
}

/** "Opening “title”", or "Opening claude" for a coding agent's pane. */
export function openingTitle(title: string | null, agent: string | null): string {
  if (agent) return `Opening ${agent}`;
  return title ? `Opening “${title}”` : "Opening this pane";
}

const step = (tone: PaneStep["tone"], label: string, note = ""): PaneStep => ({
  tone,
  label,
  note,
  mark: tone === "done" ? "✓" : tone === "now" ? "…" : tone === "bad" ? "✕" : "",
});

const viaPhrase = (conn: ConnState) => (conn.path?.kind === "relay" ? "through the relay" : "on this network");

function latency(conn: ConnState): string {
  const ms = conn.paths.find((p) => p.state === "ok")?.latencyMs;
  return ms == null ? "" : ` · ${ms} ms`;
}

/** What the first step says while a path is being tried. */
function reachNote(conn: ConnState): string {
  const p = conn.path;
  if (!p) return "Connecting…";
  if (p.kind === "relay") {
    const status = conn.paths.find((x) => x.path.kind === "relay");
    if (status?.reached) return "Relay answered · waiting for your computer";
    return conn.paths.some((x) => x.path.kind === "lan" && x.state === "failed")
      ? "Not on this network · through the relay"
      : "Connecting through the relay";
  }
  return "Looking on this network";
}

/** Why the computer cannot be reached, in full: the client's failure, else what the paths reported. */
export function reachFailure(conn: ConnState): string {
  if (conn.status === "failed") return failureLabel(conn);
  const failed = conn.paths.filter((p) => p.state === "failed");
  // the relay is the last resort: its words say most
  const last = failed.at(-1);
  return last ? failNote(last) : (conn.error ?? "Could not reach the computer.");
}

export function paneLoad(i: PaneLoadInput): PaneLoad {
  const { conn } = i;
  const none: PaneLoad = { strip: null, card: null, held: false };
  const open = conn.status === "open";
  if (open && i.loaded) return none;
  const hasHeld = i.heldSince !== null;
  const what = i.view === "blocks" ? "blocks" : "the screen";
  const loadLabel = i.view === "blocks" ? "Load its blocks" : "Get the screen";
  const reach = `Reach ${i.hostName}`;
  const wait = Math.max(0, i.now - Math.max(i.waitingSince, conn.since ?? 0));
  const slow = wait >= SLOW_MS;
  const todo = [step("todo", "Open this pane"), step("todo", loadLabel)];
  const loadHint = i.view === "blocks" ? "Your blocks" : "Your screen";

  if (open) {
    const steps = [
      step("done", reach, `${conn.path?.kind === "relay" ? "Through the relay" : "On this network"}${latency(conn)}`),
      i.paneKnown ? step("done", "Open this pane") : step("now", "Open this pane", "Waiting for the computer’s tab list"),
      i.paneKnown ? step("now", loadLabel, i.view === "blocks" ? "Waiting for the computer to send them" : "Waiting for its next frame") : step("todo", loadLabel),
    ];
    return {
      strip: { text: `Connected ${viaPhrase(conn)}${latency(conn)}`, tone: "quiet", busy: false, elapsed: "" },
      card: slow
        ? { title: "Taking longer than usual", steps, hint: "The computer is connected but has not sent this pane yet. This keeps trying.", actions: true }
        : { title: i.opening, steps, hint: null, actions: false },
      held: false,
    };
  }

  const heldAgo = hasHeld ? agoText(i.now - i.heldSince!) : "";
  switch (conn.status) {
    case "connecting": {
      const via = conn.path?.kind === "relay" ? " through the relay" : conn.path ? " on this network" : "";
      if (hasHeld) {
        return {
          strip: { text: `Reconnecting · showing ${what} from ${heldAgo}`, tone: "busy", busy: true, elapsed: elapsedText(wait) },
          card: null,
          held: true,
        };
      }
      return {
        strip: { text: `${slow ? "Still connecting" : "Connecting"}${via}`, tone: "busy", busy: true, elapsed: elapsedText(wait) },
        card: slow
          ? {
              title: "Taking longer than usual",
              steps: [step("now", reach, reachNote(conn)), ...todo],
              hint: "Rift on your computer may be asleep, closed, or still joining the relay. This keeps trying.",
              actions: true,
            }
          : { title: i.opening, steps: [step("now", reach, reachNote(conn)), ...todo], hint: null, actions: false },
        held: false,
      };
    }
    case "closed":
    case "failed": {
      if (conn.status === "closed" && conn.retryAt === null && conn.retryInMs === null) break;
      const text =
        conn.status === "failed"
          ? "Can’t reach your computer"
          : `Can’t reach your computer · ${retryText(retrySeconds(conn, i.now))}`;
      const strip = { text, tone: "bad" as const, busy: false, elapsed: "" };
      if (hasHeld) return { strip, card: null, held: true };
      return {
        strip,
        card: {
          title: "Can’t open this pane",
          steps: [step("bad", reach, reachFailure(conn)), ...todo],
          hint: `Open Rift on your computer and check Settings › Remote access. ${loadHint} come${i.view === "blocks" ? "" : "s"} back when it answers.`,
          actions: true,
          noRetry: conn.failure === "auth",
        },
        held: false,
      };
    }
    case "offline": {
      const strip = { text: "Offline · this phone has no network", tone: "quiet" as const, busy: false, elapsed: "" };
      if (hasHeld) return { strip, card: null, held: true };
      return {
        strip,
        card: {
          title: "Can’t open this pane",
          steps: [step("now", reach, "Waiting for a network"), ...todo],
          hint: "Rift reconnects as soon as this phone is back online.",
          actions: false,
        },
        held: false,
      };
    }
  }
  // idle, or closed with nothing scheduled (the app was in the background): not trying right now
  const strip = { text: "Not connected", tone: "quiet" as const, busy: false, elapsed: "" };
  if (hasHeld) return { strip, card: null, held: true };
  return { strip, card: { title: i.opening, steps: [step("now", reach, "Not connected"), ...todo], hint: null, actions: false }, held: false };
}
