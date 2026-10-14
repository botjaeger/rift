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
