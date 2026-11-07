// What a pane's page says while it opens (the PaneLoading board): the status strip, the steps
// card, and whether the blocks held in memory are shown dimmed. Free of React Native imports
// (tested in node). The wording is the board's.

import { MAX_RETRIES, type ConnState } from "../protocol/client";
import { failureLabel } from "./connLabel";
import { agoText, backInText, failNote, QUIT_HINT, retryRound, stoppedText, UPDATE_HINT, updating } from "./connStatus";
import { formatClock } from "./format";

/** After this long without the pane the card says "Taking longer than usual". */
export const SLOW_MS = 15_000;

export interface PaneStep {
  /** The step's glyph (the New Blocks board): done ✓, current ◌, to do ○, failed ✕. */
  mark: "✓" | "◌" | "○" | "✕";
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
    /** The way out to Portals: "Cancel" stops the retrying first, "Back to Portals" only leaves. */
    leave?: "Cancel" | "Back to Portals";
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

/** "Opening project1" (the tab's title, as the New Blocks board), or "Opening claude" for a coding agent's pane. */
export function openingTitle(title: string | null, agent: string | null): string {
  if (agent) return `Opening ${agent}`;
  return title ? `Opening ${title}` : "Opening this pane";
}

const step = (tone: PaneStep["tone"], label: string, note = ""): PaneStep => ({
  tone,
  label,
  note,
  mark: tone === "done" ? "✓" : tone === "now" ? "◌" : tone === "bad" ? "✕" : "○",
});

const viaPhrase = (conn: ConnState) => (conn.path?.kind === "relay" ? "through the relay" : "on this network");

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

/** A reason as a sentence: a capital first letter and one full stop ("connection error" → "Connection error."). */
export function asSentence(reason: string): string {
  const text = reason.trim().replace(/[.\s]+$/, "");
  return text === "" ? "" : `${text[0]!.toUpperCase()}${text.slice(1)}.`;
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
    // the New Blocks board's wording: reached, the blocks loading, the screen still to come
    const loading = i.view === "blocks" ? "Loading the pane’s blocks…" : "Loading the screen…";
    const steps = [
      step("done", `Reached ${i.hostName} ${viaPhrase(conn)}`),
      ...(i.paneKnown
        ? [step("now", loading), ...(i.view === "blocks" ? [step("todo", "The screen")] : [])]
        : [step("now", "Open this pane", "Waiting for the computer’s tab list"), step("todo", i.view === "blocks" ? "The pane’s blocks" : "The screen")]),
    ];
    return {
      // connected: no strip (the board), the card says what is left
      strip: null,
      card: slow
        ? { title: "Taking longer than usual", steps, hint: "The computer is connected but has not sent this pane yet. This keeps trying.", actions: true }
        : { title: i.opening, steps, hint: null, actions: false },
      held: false,
    };
  }

  const heldAgo = hasHeld ? agoText(i.now - i.heldSince!) : "";
  if (updating(conn) && conn.closing) {
    // Rift restarts on the computer: expected, so no retry counter and no "Can’t reach"
    const strip = { text: `${i.hostName} is updating Rift`, tone: "busy" as const, busy: true, elapsed: "" };
    if (hasHeld) return { strip, card: null, held: true };
    const back = backInText(conn.closing, i.now);
    return {
      strip,
      card: {
        title: `${i.hostName} is updating Rift`,
        steps: [step("now", reach, `${back[0]!.toUpperCase()}${back.slice(1)}`), ...todo],
        hint: UPDATE_HINT,
        actions: true,
        noRetry: true,
        leave: "Cancel",
      },
      held: false,
    };
  }
  switch (conn.status) {
    case "connecting": {
      const via = conn.path?.kind === "relay" ? " through the relay" : conn.path ? " on this network" : "";
      if (hasHeld) {
        return {
          strip: { text: `Reconnecting · showing ${what} from ${heldAgo}`, tone: "busy", busy: true, elapsed: formatClock(wait) },
          card: null,
          held: true,
        };
      }
      return {
        strip: { text: `${slow ? "Still connecting" : "Connecting"}${via}`, tone: "busy", busy: true, elapsed: formatClock(wait) },
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
      const retrying = conn.status === "closed";
      const gaveUp = conn.status === "failed" && conn.failure === "gave_up";
      const quit = conn.status === "failed" && conn.failure === "host_quit";
      const text = retrying
        ? `Can’t reach your computer · ${retryRound(conn, i.now)}`
        : gaveUp
          ? `Can’t reach your computer · ${stoppedText(conn)}`
          : quit
            ? `Rift quit on ${i.hostName}`
            : "Can’t reach your computer";
      const strip = { text, tone: quit ? ("quiet" as const) : ("bad" as const), busy: false, elapsed: "" };
      if (hasHeld) return { strip, card: null, held: true };
      if (quit) {
        return {
          strip,
          card: {
            title: `Rift quit on ${i.hostName}`,
            steps: [step("bad", reach, "Rift quit on this computer")],
            hint: QUIT_HINT,
            actions: true,
            leave: "Back to Portals",
          },
          held: false,
        };
      }
      if (retrying) {
        const round = retryRound(conn, i.now);
        return {
          strip,
          card: {
            title: `Reconnecting to ${i.hostName}`,
            steps: [step("now", reach, `${round[0]!.toUpperCase()}${round.slice(1)}`), ...todo],
            hint: [asSentence(reachFailure(conn)), `Rift retries ${MAX_RETRIES} times, then stops and waits for you.`].filter(Boolean).join(" "),
            actions: true,
            noRetry: true,
            leave: "Cancel",
          },
          held: false,
        };
      }
      if (gaveUp) {
        return {
          strip,
          card: {
            title: `Can’t reach ${i.hostName}`,
            steps: [step("bad", reach, conn.attempt >= MAX_RETRIES ? `No answer after ${MAX_RETRIES} retries` : "Stopped")],
            hint: `Rift stopped trying. Check that ${i.hostName} is on and Rift is running there, then try again.`,
            actions: true,
            leave: "Back to Portals",
          },
          held: false,
        };
      }
      return {
        strip,
        card: {
          title: "Can’t open this pane",
          steps: [step("bad", reach, reachFailure(conn)), ...todo],
          hint: `Open Rift on your computer and check Settings › Remote access. ${loadHint} come${i.view === "blocks" ? "" : "s"} back when it answers.`,
          actions: true,
          noRetry: conn.failure === "auth",
          leave: "Back to Portals",
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
