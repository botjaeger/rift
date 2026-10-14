// When and what to notify for a finished command (`done`). Pure: the app passes the facts in.
// These are the notifications the app posts itself, while it runs; a computer that holds this
// device's push token alerts through push instead (crates/rift-remote/src/push.rs, same words
// and the same rules: `pushRegistration`).

import { toBase64Url } from "./noise/bytes";
import type { RandomBytes } from "./noise/noise";
import { isNtfyUrl } from "./protocol/types";
import type { NotifyPolicy } from "./stores";

// --- ntfy (https://ntfy.sh, or a self-hosted server) ------------------------------------------

/** The ntfy server a new topic uses unless the user names another. */
export const NTFY_SERVER = "https://ntfy.sh";

/** The length of a topic `newNtfyTopic` makes (`rift-` and 22 characters). */
export const NTFY_TOPIC_LEN = 27;

/** A new random topic: `rift-` and 22 base64url characters (128 bits): the topic is the secret. */
export function newNtfyTopic(randomBytes: RandomBytes): string {
  return `rift-${toBase64Url(randomBytes(16))}`;
}

/** A topic as the app creates it (and the gateway accepts it). */
export function isNtfyTopic(topic: string): boolean {
  return /^[A-Za-z0-9_-]{1,64}$/.test(topic);
}

/**
 * The server as typed (`ntfy.example.com`, `https://ntfy.example.com:8443/`) as
 * `https://<host>[:port]`, or null when it is not one the gateway would post to (http, a path,
 * a user, a query).
 */
export function normalizeNtfyServer(input: string): string | null {
  let s = input.trim().replace(/\/+$/, "");
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(s)) s = `https://${s}`;
  s = s.replace(/^https:\/\//i, "https://");
  // checked with a topic as long as the app makes, so the whole URL stays valid
  return isNtfyUrl(`${s}/${"x".repeat(NTFY_TOPIC_LEN)}`) ? s.toLowerCase() : null;
}


/** The topic URL the gateway posts to, or null when ntfy is off or the settings are not usable. */
export function ntfyUrl(s: { ntfyServer: string; ntfyEnabled: boolean; ntfyTopic: string | null }): string | null {
  if (!s.ntfyEnabled || s.ntfyTopic === null) return null;
  const url = `${s.ntfyServer}/${s.ntfyTopic}`;
  return isNtfyUrl(url) ? url : null;
}

/**
 * The ntfy app's subscribe link, `ntfy://<host>[:port]/<topic>` (docs.ntfy.sh/subscribe/phone,
 * "Integrations"; documented for Android: ntfy for iPhone registers no link, so it fails there
 * and the app shows `ntfyManualSteps` instead).
 */
export function ntfySubscribeLink(server: string, topic: string): string {
  return `ntfy://${server.replace(/^https:\/\//, "")}/${topic}`;
}

/** How to subscribe by hand (ntfy for iPhone): the topic is on the clipboard by then. */
export function ntfyManualSteps(server: string): string {
  const own = server === NTFY_SERVER ? "" : ` turn on “Use another server” and enter ${server},`;
  return `The topic is copied. In the ntfy app, tap +, paste it as the topic,${own} then tap Subscribe.`;
}

/**
 * What to do when the subscribe link did not open: on Android ntfy is not installed (it handles
 * the link), so install it first; on iPhone ntfy has no link, so subscribe by hand (`copied`: the
 * topic is on the clipboard).
 */
export function ntfySubscribeFallback(platform: string, server: string): { copy: boolean; text: string } {
  if (platform === "android") {
    return { copy: false, text: "Install ntfy from Google Play or F-Droid first, then tap Subscribe in ntfy again." };
  }
  return { copy: true, text: ntfyManualSteps(server) };
}

/** After the server changed: the topic stays, but the ntfy app subscribes per server. */
export function ntfyServerChanged(server: string): string {
  return `Subscribe again in ntfy: the topic is the same, but it is now on ${server.replace(/^https:\/\//, "")}.`;
}

/** After "New topic". */
export const NTFY_ROTATED =
  "New topic made. Subscribe to it in ntfy; computers that get it stop posting to the old topic. " +
  "One that is offline may post there until it reconnects, and one unpaired while offline may keep posting there.";

/**
 * Settings › While the app is closed footer (the Settings board): what the ntfy server sees.
 * With "Show the command" on, the command line too.
 */
export function ntfyFooter(server: string, showCommand = false): string {
  const who = server === NTFY_SERVER ? "ntfy.sh" : server.replace(/^https:\/\//, "");
  return showCommand
    ? `${who} sees each alert's computer name, exit status, a link to the pane and, with “Show the command”, the command line. Never output.`
    : `${who} sees each alert's computer name, exit status and a link to the pane. Never output.`;
}

/** The default "Notify after": commands at least this long notify without being asked to. */
export const LONG_COMMAND_MS = 30_000;

export interface NotifyFacts {
  /** The app is in the foreground. */
  appActive: boolean;
  /** The pane on screen, if any. */
  viewing: { host: string; pane: number } | null;
  host: string;
  pane: number;
  durationMs: number;
  /** The command's exit status (null: the shell reported none). */
  exit: number | null;
  /** The user asked "Notify me when it finishes" for this command. */
  asked: boolean;
  /** Settings › Notifications. */
  policy: NotifyPolicy;
  /** Settings › Notifications › Notify after. */
  afterMs: number;
  /** The computer's Notifications switch is off. */
  muted: boolean;
}

/** A command that failed: an exit status other than 0 (none reported counts as not failed). */
const failed = (exit: number | null) => exit !== null && exit !== 0;

/**
 * Notify when the user cannot see the result (app in the background, or another screen) and
 * either the user asked for this one, or the computer is not muted and the command ran at
 * least `afterMs` and the policy covers it (all, or failures only).
 */
export function shouldNotify(f: NotifyFacts): boolean {
  const watching = f.appActive && f.viewing !== null && f.viewing.host === f.host && f.viewing.pane === f.pane;
  if (watching) return false;
  if (f.asked) return true;
  if (f.muted || f.policy === "off" || f.durationMs < f.afterMs) return false;
  return f.policy === "all" || failed(f.exit);
}

/** What `push` registers with a computer (`pushRegistration`). */
export interface PushRegistration {
  longMs: number | null;
  failuresOnly: boolean;
  /**
   * Registered with an ntfy topic and no Expo token: the computer then sends no unasked alert
   * about a pane this device follows (no ntfy banner for the pane on screen).
   */
  ntfyOnly?: boolean;
}

/**
 * What `push` registers with a computer, by the same rules: no `long_ms` alerts when the policy
 * is off or the computer is muted ("Notify me" still alerts), else from `afterMs` on, failures
 * only when the policy says so. A computer that cannot filter failures (`failuresSupported`
 * false: no `push_failures` feature) would alert for successes too, so it gets no `long_ms`.
 */
export function pushRegistration(
  policy: NotifyPolicy,
  afterMs: number,
  muted: boolean,
  failuresSupported = true,
): PushRegistration {
  const off = policy === "off" || muted || (policy === "failures" && !failuresSupported);
  return { longMs: off ? null : afterMs, failuresOnly: policy === "failures" };
}

/**
 * Whether a computer registered with `reg` alerts through push for a finished command by
 * itself, as the gateway decides (crates/rift-remote/src/push.rs): a "Notify me" it received
 * (`askSent`), or a command at least `longMs` long, failed when `failuresOnly`, except, for an
 * ntfy-only registration, one about the pane this device follows (`followed`: the gateway drops
 * that ntfy alert). The app then posts nothing of its own; otherwise it does.
 */
export function pushCovers(
  reg: PushRegistration,
  done: { askSent: boolean; durationMs: number; exit: number | null; followed?: boolean },
): boolean {
  if (done.askSent) return true;
  if (reg.ntfyOnly && done.followed) return false;
  return reg.longMs !== null && done.durationMs >= reg.longMs && (!reg.failuresOnly || failed(done.exit));
}

export interface NotificationText {
  title: string;
  body: string;
}

export function notificationText(
  hostName: string,
  exit: number | null,
  command: string,
  showCommand: boolean,
): NotificationText {
  const title =
    exit === null || exit === 0
      ? `A command finished on ${hostName}`
      : `A command failed on ${hostName} (exit ${exit})`;
  return { title, body: showCommand ? command : "" };
}

/** The parts of a notification response (expo-notifications) that `tapTarget` reads. */
export interface TapResponse {
  /** The alert's action button pressed ("yes", "no", "open"), or the system's default tap. */
  actionIdentifier?: string;
  notification: {
    request: { identifier: string; content: { data?: unknown } };
  };
}

/** The alert categories (the P6 Lock board): a question with Yes / No / Open, or Open only. */
export const ASK_CATEGORY = "rift-ask";
export const ASK_OPEN_CATEGORY = "rift-ask-open";

/**
 * The pane a tapped notification opens, once per notification: the launch response and the
 * response listener can both deliver the same tap, and a remounted root layout asks for the
 * launch response again. `handled` holds the identifiers already opened. Null when the tap was
 * handled already or carries no pane.
 */
export function tapTarget(
  r: TapResponse | null,
  handled: Set<string>,
): { host: string; pane: number; ask?: string; action?: "yes" | "no" | "open" } | null {
  if (!r) return null;
  const id = r.notification.request.identifier;
  if (handled.has(id)) return null;
  const data = r.notification.request.content.data as { host?: unknown; pane?: unknown; ask?: unknown } | undefined;
  if (typeof data?.host !== "string" || typeof data.pane !== "number") return null;
  handled.add(id);
  const out: { host: string; pane: number; ask?: string; action?: "yes" | "no" | "open" } = { host: data.host, pane: data.pane };
  // a "Needs you" alert carries its question: Yes and No answer it, a tap opens it
  if (typeof data.ask === "string") {
    out.ask = data.ask;
    const a = r.actionIdentifier;
    out.action = a === "yes" || a === "no" ? a : "open";
  }
  return out;
}

/**
 * Why there is no push token, in Settings' words. The system's own error (an Android build
 * without its Firebase setup throws a Java message naming FirebaseApp and a docs URL) stays in
 * the log.
 */
export function pushTokenProblem(detail: string): string {
  if (/firebase|fcm/i.test(detail)) return "this build is not set up for Android push.";
  return "the system gave no push token.";
}

/** Settings › Notifications' footer: where notifications come from, and what push carries. */
export function pushFooter(
  push: { state: "on" } | { state: "off" } | { state: "unavailable"; reason: string },
  ntfyOn = false,
): string {
  // ntfy replaces Expo push for the computers that support it (fleet `sendPush`)
  if (ntfyOn) return "While the app is closed, your computers send these through the ntfy app.";
  switch (push.state) {
    case "on":
      return "While the app is closed, your computers send these through Expo and Apple's or Google's push service: the computer's name, the exit status and, only with “Show the command”, the command line. Never output.";
    case "off":
      return "Notifications come from this app while it is open. Allow notifications to get them while it is closed too.";
    case "unavailable":
      return `Notifications come only while this app is open: ${push.reason} Turn on Alerts through ntfy to get them while it is closed.`;
  }
}
