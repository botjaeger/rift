import { base } from '../base.mjs';
import { osKeyTables } from './os-keys.mjs';

// Pure Markdown transforms behind scripts/sync-docs.mjs, kept apart so they can be tested
// (scripts/docs-transform.test.mjs). transformPage() turns one Rift doc into the site's copy, or
// throws when the result would carry a private path, a link to the private repo, or a reference
// to a page the site does not publish.

const LINKS = {
  'keyboard-shortcuts.md': '/docs/keyboard-shortcuts/',
  'configuration.md': '/docs/configuration/',
  'getting-started.md': '/docs/getting-started/',
  'features.md#remote-access-phone-companion': '/docs/phone/',
};
const TITLES = {
  'keyboard-shortcuts.md': 'Keyboard shortcuts',
  'configuration.md': 'Configuration',
  'getting-started.md': 'Getting started',
};

// Sample paths in the source docs, swapped for the site's generic sample names.
const SAMPLES = [
  ['/Users/me/code/rift', '/Users/me/code/project1'],
  ['/Users/me/code/site', '/Users/me/code/project2'],
  // The example script name in the Tab-completion row.
  [/`scripts\/`, then `[^`]+`/, '`scripts/`, then `deploy.sh`'],
];

// Plain-text pointers to docs the site does not publish. The UNPUBLISHED and REPO_PATHS guards
// fail the sync when a new one appears, so this list stays complete.
const PROSE = [
  [' (see features.md, Completions)', ''],
  ['\n(keys and states: ui.md › config.toml editor).', '.'],
  [' (docs/remote-protocol.md, The\ndesktop\'s theme)', ''],
  ['\n  so it survives a restart; see features.md, Pane focus links)', '\n  so it survives a restart)'],
  // Files in the private source repo.
  [' (app in `mobile/`)', ''],
  [' or\n`packaging/relay.Dockerfile`)', ')'],
  [
    '`rift-terminal relay` or\n                                  # packaging/relay.Dockerfile hosts one;\n                                  # scripts/relay-local.sh runs a temporary one',
    '`rift-terminal relay` hosts one',
  ],
  ['Behavior details: features · every screen:\nUI · first steps:', 'First steps:'],
  // The app's earlier names stay off the site.
  [/ When the `rift` config or data directory does not exist, an existing one from an earlier name of the app \([^)]*\) stays active; move it to `rift` when you want the new paths\./, ''],
];

const UNPUBLISHED = /\b(features|ui|remote-protocol|development|mobile-threat-model|performance)\.md\b|Behavior details: features/;
const REPO_PATHS = /(^|[\s`(#])(packaging|mobile|crates)\/|scripts\/relay/;
const PRIVATE_REPO = /github\.com\/botjaeger\/rift(?!-releases)/;
const FORBIDDEN = [/\/Users\/(?!me\b)/, PRIVATE_REPO, /screenshot\.png/, /earlier name of the app/];
const HTML_TAGS = new Set(['a', 'br', 'div', 'span', 'kbd', 'img', 'p', 'sub', 'sup', 'details', 'summary', 'code', 'pre']);

const level = (line) => /^(#{1,6}) /.exec(line)?.[1].length ?? 0;

// Splits markdown into lines and marks the ones inside fenced code blocks.
export function scan(md) {
  let fence = false;
  return md.split('\n').map((text) => {
    const isFence = /^\s*(```|~~~)/.test(text);
    const inCode = fence || isFence;
    if (isFence) fence = !fence;
    return { text, inCode };
  });
}

// The lines of the section under `heading`: [start, end) with start at the heading itself.
function sectionRange(lines, heading) {
  const start = lines.findIndex((l) => !l.inCode && l.text === heading);
  if (start < 0) throw new Error(`section "${heading}" not found`);
  const depth = level(heading);
  const end = lines.findIndex((l, i) => i > start && !l.inCode && level(l.text) > 0 && level(l.text) <= depth);
  return [start, end < 0 ? lines.length : end];
}

export function takeSection(md, heading) {
  const lines = scan(md);
  const [start, end] = sectionRange(lines, heading);
  return lines.slice(start + 1, end).map((l) => l.text).join('\n');
}

export function dropSection(md, heading) {
  const lines = scan(md);
  const [start, end] = sectionRange(lines, heading);
  return [...lines.slice(0, start), ...lines.slice(end)].map((l) => l.text).join('\n');
}

// A link to a page the site does not publish is marked ⟦like this⟧ while pointers to it are cut.
const OPEN = '\u0000';
const CLOSE = '\u0001';
const GONE = `${OPEN}[^${CLOSE}]*${CLOSE}`;
// "See X for more.", "(see X)" and "Details: X, Y." only send the reader to that page: drop them whole.
const POINTERS = [
  new RegExp(`\\s*\\(see\\s+${GONE}\\)`, 'g'),
  new RegExp(`\\s*\\bSee\\s+${GONE}[^.\\n]*(?:\\n[^.\\n]+)*\\.`, 'g'),
  new RegExp(`\\s*\\bDetails:\\s*${GONE}(?:,\\s*${GONE})*\\.`, 'g'),
];

export function rewriteLinks(md) {
  const out = md.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (all, text, target) => {
    if (target.startsWith('#')) return all;
    const bare = target.replace(/^docs\//, '');
    const [file, anchor] = bare.split('#');
    const mapped = LINKS[bare] ?? LINKS[file];
    if (mapped) return `[${TITLES[text] ?? text}](${base}${mapped}${LINKS[bare] || !anchor ? '' : `#${anchor}`})`;
    if (/^https?:\/\//.test(target) && !PRIVATE_REPO.test(target)) return all;
    return OPEN + text + CLOSE; // a page the site does not publish
  });
  // Anywhere else the link text reads fine on its own.
  return POINTERS.reduce((s, re) => s.replace(re, ''), out).replace(new RegExp(`${OPEN}([^${CLOSE}]*)${CLOSE}`, 'g'), '$1');
}

// Placeholders such as <name> or <data dir> in prose would be read as HTML tags and vanish.
export function escapePlaceholders(md) {
  return scan(md)
    .map(({ text, inCode }) =>
      inCode
        ? text
        : text
            .split(/(`[^`]*`)/)
            .map((part, i) =>
              i % 2
                ? part
                : part.replace(/<\/?([a-z][a-z0-9 _-]*)>/gi, (tag, name) =>
                    HTML_TAGS.has(name.toLowerCase()) ? tag : tag.replace('<', '&lt;').replace('>', '&gt;'),
                  ),
            )
            .join(''),
    )
    .join('\n');
}

export function check(md, from) {
  const bad = FORBIDDEN.find((re) => re.test(md));
  if (bad) throw new Error(`${from}: matches ${bad}; fix the source or extend SAMPLES`);
  const ref = md.split('\n').find((line) => UNPUBLISHED.test(line.replace(/`[^`]*`/g, '')) || REPO_PATHS.test(line));
  if (ref) throw new Error(`${from}: refers to a doc the site does not publish; extend PROSE:\n${ref}`);
  const test = md.split('\n').find((line) => TEST_ONLY.test(line));
  if (test) throw new Error(`${from}: a test-only switch outside a bullet dropTestItems can remove:\n${test}`);
}

// Test-harness switches (scripted runs, test keys) are for Rift's own test suite, not for readers.
const TEST_ONLY = /--script|\(tests?\b|RIFT_TEST_|_TEST_KEY/;

// Drops each top-level bullet (with its indented continuation lines) that TEST_ONLY matches.
export function dropTestItems(md) {
  const groups = []; // each: { item, lines }, item = a top-level bullet and its continuation lines
  for (const { text, inCode } of scan(md)) {
    const last = groups[groups.length - 1];
    if (!inCode && /^- /.test(text)) groups.push({ item: true, lines: [text] });
    else if (!inCode && last?.item && /^ {2}\S/.test(text)) last.lines.push(text);
    else groups.push({ item: false, lines: [text] });
  }
  return groups
    .filter((g) => !(g.item && TEST_ONLY.test(g.lines.join('\n'))))
    .flatMap((g) => g.lines)
    .join('\n');
}

// page: { from, title, description, order, section?, drop? }; defaults: the app's key defaults
// (os-keys parseDefaults; optional). Returns the file's full text.
export function transformPage(source, page, defaults) {
  let md = page.section ? takeSection(source, page.section) : source.replace(/^# .*\n+/, ''); // the layout prints the title
  for (const heading of page.drop ?? []) md = dropSection(md, heading);
  md = dropTestItems(md);
  md = osKeyTables(escapePlaceholders(rewriteLinks(md)), defaults);
  for (const [from, to] of [...SAMPLES, ...PROSE]) md = typeof from === 'string' ? md.split(from).join(to) : md.replace(from, to);
  check(md, page.from);
  const front = [
    '---',
    `title: ${JSON.stringify(page.title)}`,
    `description: ${JSON.stringify(page.description)}`,
    `order: ${page.order}`,
    `source: ${JSON.stringify(page.from)}`,
    '---',
    '',
  ].join('\n');
  return front + md.trim() + '\n';
}
