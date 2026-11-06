// How a computer is being reached, in words: the Portals card's status line and the Connection
// sheet's rows (the RelayPhone board). Free of React Native imports (tested in node). Nothing
// here is invented: a latency, a wait or an error is shown only when the client measured it.

import type { ConnState, PathStatus } from "../protocol/client";
import { MAX_RETRIES, NO_ANSWER } from "../protocol/client";
import { relayHost } from "../protocol/relay";
import { failureLabel } from "./connLabel";

export type LineTone = "muted" | "busy" | "bad" | "dim";

export interface HostLine {
  text: string;
  tone: LineTone;
  /** A thin progress bar under the line while a path is being tried. */
  progress: boolean;
  /** Offer "Try again". */
  retry: boolean;
  /** The paragraph under the card (why, and what happens next). */
  hint: string | null;
}

export const tabsText = (n: number) => `${n} ${n === 1 ? "tab" : "tabs"}`;

const hasRelay = (conn: Pick<ConnState, "paths">) => conn.paths.some((p) => p.path.kind === "relay");

/** Seconds until the next attempt, from the moment it was scheduled; 0 once it is due. */
export function retrySeconds(conn: Pick<ConnState, "retryAt" | "retryInMs">, now: number): number {
  if (conn.retryAt !== null) return Math.max(0, Math.ceil((conn.retryAt - now) / 1000));
  return Math.max(0, Math.ceil((conn.retryInMs ?? 0) / 1000));
}

/** "retry 2 of 5 in 8 s", "retry 2 of 5 now": the reconnect round that is waiting, of [MAX_RETRIES]. */
export function retryRound(conn: Pick<ConnState, "attempt" | "retryAt" | "retryInMs">, now: number): string {
  const secs = retrySeconds(conn, now);
  return `retry ${Math.min(conn.attempt + 1, MAX_RETRIES)} of ${MAX_RETRIES} ${secs === 0 ? "now" : `in ${secs} s`}`;
}

/** Why retrying ended: "stopped after 5 retries", or just "stopped" when it was cancelled sooner. */
export const stoppedText = (conn: Pick<ConnState, "attempt">) =>
  conn.attempt >= MAX_RETRIES ? `stopped after ${MAX_RETRIES} retries` : "stopped";

/** What an update promised when no `back_in_s` came with it. */
const UPDATE_BACK_IN_S = 60;

/** The computer said Rift is updating, and the client is reconnecting to it (connecting, or waiting to retry). */
export function updating(conn: Pick<ConnState, "closing" | "status" | "retryAt" | "retryInMs">): boolean {
  if (conn.closing?.reason !== "update") return false;
  return conn.status === "connecting" || (conn.status === "closed" && (conn.retryAt !== null || conn.retryInMs !== null));
}

/**
 * When an updating computer is back, from what it promised: "back in about a minute", "back in
 * about 3 minutes", "back in a moment", then "taking a little longer than expected" once the
 * promised time is over (never a countdown that sits at 0).
 */
export function backInText(closing: NonNullable<ConnState["closing"]>, now: number): string {
  const left = closing.at + (closing.backInS ?? UPDATE_BACK_IN_S) * 1000 - now;
  if (left <= 0) return "taking a little longer than expected";
  if (left <= 30_000) return "back in a moment";
  if (left <= 90_000) return "back in about a minute";
  return `back in about ${Math.round(left / 60_000)} minutes`;
}

/** The hint while Rift updates on the computer. */
export const UPDATE_HINT = "Nothing is wrong. Your tabs come back by themselves.";
/** The hint once Rift quit on the computer. */
export const QUIT_HINT = "Open Rift on the computer, then tap Try again.";

/**
 * The status line under a computer's name: where it is ("This network · 30 tabs", "Over the
 * relay"), or what is being tried. `seenAt`: epoch ms it was last connected (null: not known),
 * for a computer that stopped answering ("Can't reach it · last seen 2 h ago").
 */
export function hostLine(conn: ConnState, now: number, tabs: number | null, seenAt: number | null = null): HostLine {
  const line = (text: string, tone: LineTone, more: Partial<HostLine> = {}): HostLine => ({
    text,
    tone,
    progress: false,
    retry: false,
    hint: null,
    ...more,
  });
  if (updating(conn) && conn.closing) {
    // expected: Rift restarts on the computer; no retry counter, nothing alarming
    return line(`Updating Rift · ${backInText(conn.closing, now)}`, "busy", {
      progress: conn.status === "connecting",
      retry: true,
      hint: UPDATE_HINT,
    });
  }
  switch (conn.status) {
    case "open": {
      const via = conn.path?.kind === "relay" ? "Over the relay" : "This network";
      return line(`${via}${tabs === null ? "" : ` · ${tabsText(tabs)}`}`, "muted");
    }
    case "connecting": {
      if (conn.path?.kind === "relay") {
        const lanFailed = conn.paths.some((p) => p.path.kind === "lan" && p.state === "failed");
        return line(lanFailed ? "Not on this network · trying the relay…" : "Connecting through the relay…", "busy", { progress: true });
      }
      return line(conn.path ? "Looking on this network…" : "Connecting…", "busy", { progress: true });
    }
    case "closed":
      if (conn.retryAt === null && conn.retryInMs === null) return line("Not connected", "dim");
      return line(`Can’t reach it · ${retryRound(conn, now)}`, "bad", {
        retry: true,
        hint: hasRelay(conn)
          ? "Rift on your computer may be closed, asleep, or not connected to the relay. Your tabs come back when it answers."
          : "Rift on your computer may be closed or asleep. Your tabs come back when it answers.",
      });
    case "failed":
      if (conn.failure === "gave_up") {
        return line(`Can’t reach it · ${seenAt === null ? stoppedText(conn) : `last seen ${agoText(now - seenAt)}`}`, "bad", {
          retry: true,
          hint: "Check that your computer is on and Rift is running there. Rift tries again every 30 seconds while the app is open, when you tap Try again, when this phone is back online, and when you open the app.",
        });
      }
      if (conn.failure === "host_quit") return line("Rift quit on this computer", "dim", { retry: true, hint: QUIT_HINT });
      return line("Can’t connect to it", "bad", { retry: conn.failure !== "auth", hint: failureLabel(conn) });
    case "offline":
      return line("Offline · this phone has no network", "dim", { hint: "Rift reconnects as soon as this phone is back online." });
    case "idle":
      return line("Not connected", "dim");
  }
}

export type PathTone = "ok" | "busy" | "bad" | "off";

export interface PathRow {
  /** Stable key: the path's address. */
  key: string;
  title: string;
  /** The LAN address, or the relay's host. */
  addr: string;
  /** ✓ in use, … connecting, ✕ failed, – not used. */
  mark: "✓" | "…" | "✕" | "–";
  tone: PathTone;
  note: string;
}

const MARK: Record<PathTone, PathRow["mark"]> = { ok: "✓", busy: "…", bad: "✕", off: "–" };

const seconds = (ms: number | null) => (ms === null ? "" : ` (${Math.max(1, Math.ceil(ms / 1000))} s)`);

/** The relay says a phone's gateway is not there with a 404 ("gateway offline") or by closing the pair. */
const NOT_REGISTERED = /\b404\b|gateway offline|did not answer/i;

/** A raw error as a note's sentence: its first letter capitalised ("the computer's key …" → "The …"). */
const sentence = (t: string): string => (t ? t.charAt(0).toUpperCase() + t.slice(1) : t);

/** Why a path failed, in the board's words where the cause is known, else the error itself. */
export function failNote(p: PathStatus): string {
  const error = p.error ?? "";
  if (p.path.kind === "relay") {
    if (NOT_REGISTERED.test(error)) return "The relay answered, but your computer is not registered there";
    if (error === NO_ANSWER) return p.reached ? "The relay answered, but your computer is not registered there" : `No answer from the relay${seconds(p.waitedMs)}`;
    return sentence(error) || "No answer from the relay";
  }
  if (error === NO_ANSWER) return `No answer on this network${seconds(p.waitedMs)}`;
  return sentence(error) || "No answer on this network";
}

function row(p: PathStatus, tone: PathTone, note: string): PathRow {
  const lan = p.path.kind === "lan";
  const addr = p.path.kind === "lan" ? p.path.address : relayHost(p.path.url);
  return { key: addr, title: lan ? "This network" : "Relay", addr, mark: MARK[tone], tone, note };
}

/** One row per path, in trying order, as the Connection sheet lists them. */
export function pathRows(conn: ConnState): PathRow[] {
  const open = conn.status === "open";
  const searching = conn.status === "connecting";
  return conn.paths.map((p) => {
    const lan = p.path.kind === "lan";
    if (conn.status === "offline") return row(p, "off", "Waiting for a network");
    if (open) {
      if (p.state === "ok") return row(p, "ok", p.latencyMs === null ? "In use" : `In use · ${p.latencyMs} ms`);
      return row(p, "off", lan ? "Not on the same network" : "Not needed on the same network");
    }
    switch (p.state) {
      case "trying":
        return row(p, "busy", !lan && p.reached ? "Relay up · your computer has not joined" : "Connecting…");
      case "failed": {
        // a LAN address that did not answer is the usual case away from home: dim once the round is over
        if (lan && p.error === NO_ANSWER && !searching) return row(p, "off", "Not on the same network");
        return row(p, "bad", failNote(p));
      }
      case "ok": // the connection dropped: nothing is in use
      case "idle":
        return row(p, "off", "Not tried yet");
    }
  });
}


/** "paired 3 Nov" (this year) or "paired 3 Nov 2025": what tells two pairings of one computer apart. */
export function pairedText(pairedAt: string, now: number): string {
  const d = new Date(pairedAt);
  if (Number.isNaN(d.getTime())) return "";
  const month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][d.getMonth()]!;
  const year = d.getFullYear() === new Date(now).getFullYear() ? "" : ` ${d.getFullYear()}`;
  return `paired ${d.getDate()} ${month}${year}`;
}

/** When a computer was last connected: this run's moment, else the one stored at its last `welcome`. */
export const lastSeenAt = (host: { lastSeen: number | null; record: { seenAt?: number } }): number | null =>
  host.lastSeen ?? host.record.seenAt ?? null;

/** "a moment ago", "2 min ago", "3 h ago", "2 d ago". */
export function agoText(ms: number): string {
  const min = Math.floor(Math.max(0, ms) / 60_000);
  if (min < 1) return "a moment ago";
  if (min < 60) return `${min} min ago`;
  if (min < 24 * 60) return `${Math.floor(min / 60)} h ago`;
  return `${Math.floor(min / (24 * 60))} d ago`;
}
