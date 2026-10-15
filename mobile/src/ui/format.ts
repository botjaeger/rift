/** `252000` → `4m 12s`; under a minute `8.1s`. */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m ${sec}s`;
}

/** A running block's clock: `252000` → `4:12`, `3723000` → `1:02:03`. */
export function formatClock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

/** A "Notify after" choice as people say it: `30000` → `30 seconds`, `300000` → `5 minutes`. */
export function formatAfter(ms: number): string {
  if (ms < 60_000) return `${Math.round(ms / 1000)} seconds`;
  const m = Math.round(ms / 60_000);
  return m === 1 ? "1 minute" : `${m} minutes`;
}

/** Last `max` lines of `text` and how many were hidden (a trailing newline is not a line). */
export function tailLines(text: string, max: number): { shown: string; hidden: number } {
  const body = text.endsWith("\n") ? text.slice(0, -1) : text;
  if (body === "") return { shown: "", hidden: 0 };
  let idx = body.length;
  for (let n = 0; n < max; n++) {
    idx = body.lastIndexOf("\n", idx - 1);
    if (idx === -1) return { shown: body, hidden: 0 };
  }
  let hidden = 0;
  for (let i = 0; i < idx + 1; i++) if (body.charCodeAt(i) === 10) hidden++;
  return { shown: body.slice(idx + 1), hidden };
}

/**
 * A command line as typed, with the keyboard's smart punctuation undone: iOS (Smart
 * Punctuation, on by default and not switched off by autoCorrect) turns `--` into an em dash and
 * quotes into curly ones, which no shell reads as flags or quotes.
 */
export function plainPunctuation(text: string): string {
  return text.replace(/\u2014/g, "--").replace(/[\u201c\u201d]/g, '"').replace(/[\u2018\u2019]/g, "'");
}

/** The one character (UTF-16 unit) typed into `next` compared with `prev`, or null for any other edit. */
export function insertedChar(prev: string, next: string): string | null {
  if (next.length !== prev.length + 1) return null;
  let i = 0;
  while (i < prev.length && prev[i] === next[i]) i++;
  if (next.slice(i + 1) !== prev.slice(i)) return null;
  return next[i]!;
}

/** A gateway's `os` as people write it (`macos` → `macOS`); anything else as sent. */
export function osName(os: string): string {
  const names: Record<string, string> = {
    macos: "macOS",
    linux: "Linux",
    windows: "Windows",
  };
  return names[os] ?? os;
}

/**
 * The Settings › About line for this build: its version, then which JavaScript it runs —
 * the bundle it shipped with, or an over-the-air update by its number (`update`, from
 * release.ts), else by when it was published.
 */
export function appVersionText(
  version: string,
  updates: { enabled: boolean; embedded: boolean; createdAt: Date | null; update: number },
): string {
  if (!updates.enabled) return `${version} · updates off`;
  if (updates.embedded || !updates.createdAt) return `${version} · as installed`;
  if (updates.update > 0) return `${version} · update ${updates.update}`;
  const when = updates.createdAt.toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
  return `${version} · update of ${when}`;
}

/** Settings › About's update line, from the last check (ota.ts OtaStatus). */
export function updateStatusText(
  check: { state: "idle" | "checking" | "downloaded" | "none" | "failed"; at: number | null },
  pending: boolean,
): string {
  if (pending) return "An update is ready and applies when the app restarts.";
  const when = check.at === null ? "" : ` · ${new Date(check.at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`;
  switch (check.state) {
    case "checking":
      return "Checking for updates…";
    case "none":
      return `Up to date${when}`;
    case "downloaded":
      return "Downloading the update…";
    case "failed":
      return `Could not check for updates${when}. Check now tries again.`;
    default:
      return "Not checked yet";
  }
}
