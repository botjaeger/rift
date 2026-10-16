// The app's release notes (mobile/CHANGELOG.md, embedded as changelog.data.ts): parsed for
// What's New after an update and for Settings › About › What's New.

/** One release: a native build (`update` 0) or over-the-air update N to it. */
export interface Release {
  version: string;
  update: number;
  /** The heading's date, as written (YYYY-MM-DD). */
  date: string;
  /** The paragraph under the heading, if any. */
  summary: string | null;
  groups: { title: string; items: { title: string; text: string }[] }[];
}

/** `0.1.0 · update 2`, or `0.1.0` for the build itself (the heading's and Settings' wording). */
export const releaseName = (r: { version: string; update: number }) =>
  r.update > 0 ? `${r.version} · update ${r.update}` : r.version;

/** The key What's New remembers: which release this JavaScript is (`0.1.0-u2`; no `+`, which a link reads as a space). */
export const releaseKey = (version: string, update: number) => `${version}-u${update}`;

const HEADING = /^## (\S+)(?: · update (\d+))? — (\S+)\s*$/;
const ITEM = /^- \*\*(.+?)\*\* — (.*)$/;

/** The releases in `md`, newest first; text it does not recognise is skipped. */
export function parseChangelog(md: string): Release[] {
  const out: Release[] = [];
  let cur: Release | null = null;
  let item: { title: string; text: string } | null = null;
  let summary: string[] = [];
  const endSummary = () => {
    if (cur && summary.length > 0 && cur.summary === null) cur.summary = summary.join(" ");
    summary = [];
  };
  for (const raw of md.split("\n")) {
    const line = raw.trimEnd();
    const h = HEADING.exec(line);
    if (h) {
      endSummary();
      cur = { version: h[1]!, update: h[2] ? Number(h[2]) : 0, date: h[3]!, summary: null, groups: [] };
      out.push(cur);
      item = null;
      continue;
    }
    // any other `## ` heading (`## Unreleased`: notes for the next release) is not shown
    if (line.startsWith("## ")) {
      endSummary();
      cur = null;
      item = null;
      continue;
    }
    if (!cur) continue;
    if (line.startsWith("### ")) {
      endSummary();
      cur.groups.push({ title: line.slice(4).trim(), items: [] });
      item = null;
      continue;
    }
    const group = cur.groups[cur.groups.length - 1];
    const it = ITEM.exec(line);
    if (it && group) {
      item = { title: it[1]!, text: it[2]!.trim() };
      group.items.push(item);
    } else if (line.startsWith("  ") && item) {
      item.text = `${item.text} ${line.trim()}`;
    } else if (line.trim() === "") {
      item = null;
      if (!group) endSummary();
    } else if (!group) {
      summary.push(line.trim());
    }
  }
  endSummary();
  for (const r of out) {
    if (r.summary !== null) r.summary = plain(r.summary);
    for (const g of r.groups) for (const it of g.items) it.text = plain(it.text);
  }
  return out;
}

/** `*Restart now*` → `Restart now`: the file's italics (the names the UI shows) read as plain text. */
export const plain = (s: string) => s.replace(/\*([^*\s](?:[^*]*[^*\s])?)\*/g, "$1");

/**
 * What's New after an update: the releases newer than the one last seen, newest first, up to and
 * including `current`. Empty when nothing changed, on a first launch (nothing seen yet), or when
 * `current` is not in the notes (a development build).
 */
export function releasesSince(all: readonly Release[], seen: string | null, current: string): Release[] {
  if (seen === null || seen === current) return [];
  const from = all.findIndex((r) => releaseKey(r.version, r.update) === current);
  if (from < 0) return [];
  const to = all.findIndex((r) => releaseKey(r.version, r.update) === seen);
  // the one seen is not listed (or is newer: a downgrade): only the current release
  return all.slice(from, to > from ? to : from + 1);
}
