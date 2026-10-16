// How a computer is being reached, in words: the Portals card's status line and the Connection
// sheet's rows (the RelayPhone board). Free of React Native imports (tested in node). Nothing
// here is invented: a latency, a wait or an error is shown only when the client measured it.

import type { ConnState, PathStatus } from "../protocol/client";
import { NO_ANSWER } from "../protocol/client";
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

/** "retrying in 8 s", "retrying now". */
export const retryText = (secs: number) => (secs === 0 ? "retrying now" : `retrying in ${secs} s`);

/** The status line under a computer's name: how it is connected, or what is being tried. */
export function hostLine(conn: ConnState, now: number, tabs: number | null): HostLine {
  const line = (text: string, tone: LineTone, more: Partial<HostLine> = {}): HostLine => ({
    text,
    tone,
    progress: false,
    retry: false,
    hint: null,
    ...more,
  });
  switch (conn.status) {
    case "open": {
      const via = conn.path?.kind === "relay" ? "through the relay" : "this network";
      return line(`Connected · ${via}${tabs === null ? "" : ` · ${tabsText(tabs)}`}`, "muted");
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
      return line(`Can’t reach it · ${retryText(retrySeconds(conn, now))}`, "bad", {
        retry: true,
        hint: hasRelay(conn)
          ? "Rift on your computer may be closed, asleep, or not connected to the relay. Your tabs come back when it answers."
          : "Rift on your computer may be closed or asleep. Your tabs come back when it answers.",
      });
    case "failed":
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

/** Why a path failed, in the board's words where the cause is known, else the error itself. */
export function failNote(p: PathStatus): string {
  const error = p.error ?? "";
  if (p.path.kind === "relay") {
    if (NOT_REGISTERED.test(error)) return "The relay answered, but your computer is not registered there";
    if (error === NO_ANSWER) return p.reached ? "The relay answered, but your computer is not registered there" : `No answer from the relay${seconds(p.waitedMs)}`;
    return error || "No answer from the relay";
  }
  if (error === NO_ANSWER) return `No answer on this network${seconds(p.waitedMs)}`;
  return error || "No answer on this network";
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

/** Whether the sheet offers "Try again now" (the same cases as the card's Try again). */
export const canRetry = (conn: ConnState, now: number): boolean => hostLine(conn, now, null).retry;

/** "0:03", "1:05", "12:00" (an elapsed wait). */
export function elapsedText(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** "a moment ago", "2 min ago", "3 h ago", "2 d ago". */
export function agoText(ms: number): string {
  const min = Math.floor(Math.max(0, ms) / 60_000);
  if (min < 1) return "a moment ago";
  if (min < 60) return `${min} min ago`;
  if (min < 24 * 60) return `${Math.floor(min / 60)} h ago`;
  return `${Math.floor(min / (24 * 60))} d ago`;
}
