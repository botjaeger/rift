// When and what to notify for a finished command (`done`). Pure: the app passes the facts in.
// These are the notifications the app posts itself, while it runs; a computer that holds this
// device's push token alerts through push instead (crates/rift-remote/src/push.rs, same words).

/** Commands at least this long notify without being asked to. */
export const LONG_COMMAND_MS = 30_000;

export interface NotifyFacts {
  /** The app is in the foreground. */
  appActive: boolean;
  /** The pane on screen, if any. */
  viewing: { host: string; pane: number } | null;
  host: string;
  pane: number;
  durationMs: number;
  /** The user asked "Notify me when it finishes" for this command. */
  asked: boolean;
  /** Settings › Notifications › Command finished or failed. */
  enabled: boolean;
}

/**
 * Notify when the user cannot see the result (app in the background, or another screen) and
 * either the command ran long with notifications enabled, or the user asked for this one.
 */
export function shouldNotify(f: NotifyFacts): boolean {
  const watching = f.appActive && f.viewing !== null && f.viewing.host === f.host && f.viewing.pane === f.pane;
  if (watching) return false;
  return f.asked || (f.enabled && f.durationMs >= LONG_COMMAND_MS);
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
  notification: {
    request: { identifier: string; content: { data?: unknown } };
  };
}

/**
 * The pane a tapped notification opens, once per notification: the launch response and the
 * response listener can both deliver the same tap, and a remounted root layout asks for the
 * launch response again. `handled` holds the identifiers already opened. Null when the tap was
 * handled already or carries no pane.
 */
export function tapTarget(r: TapResponse | null, handled: Set<string>): { host: string; pane: number } | null {
  if (!r) return null;
  const id = r.notification.request.identifier;
  if (handled.has(id)) return null;
  const data = r.notification.request.content.data as { host?: unknown; pane?: unknown } | undefined;
  if (typeof data?.host !== "string" || typeof data.pane !== "number") return null;
  handled.add(id);
  return { host: data.host, pane: data.pane };
}

/** Settings › Notifications' footer: where notifications come from, and what push carries. */
export function pushFooter(push: { state: "on" } | { state: "off" } | { state: "unavailable"; reason: string }): string {
  switch (push.state) {
    case "on":
      return "While the app is closed, your computers send these through Expo and Apple's or Google's push service: the computer's name, the exit status and, only with the switch above, the command line. Never output.";
    case "off":
      return "Notifications come from this app while it is open. Allow notifications to get them while it is closed too.";
    case "unavailable":
      return `Notifications come from this app while it is open. ${push.reason}`;
  }
}
