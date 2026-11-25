// Builds the user docs pages from the Rift repo this site lives in (site/ in it) into
// src/content/docs/. Usage: node scripts/sync-docs.mjs [path/to/rift]   (default: the repo above)
//
// `pnpm build` and `pnpm dev` run it first, so the pages always follow the repo's docs; the
// generated files are not committed. The transforms live in docs-transform.mjs. Every page is
// transformed and checked before any file is written, so a failed sync leaves the docs as they were.
// src/content/docs/phone.md is written by hand and is not touched here.
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { transformPage } from './docs-transform.mjs';
import { checkLinuxKeys, parseDefaults } from './os-keys.mjs';

const site = fileURLToPath(new URL('..', import.meta.url));
const rift = resolve(process.argv[2] ?? `${site}..`);
const out = `${site}src/content/docs/`;

const PAGES = [
  {
    slug: 'getting-started',
    from: 'docs/getting-started.md',
    title: 'Getting started',
    description: 'From the first launch to everyday use: blocks, tabs and panes, the palette, settings, SSH, automation and troubleshooting.',
    order: 1,
  },
  {
    slug: 'features',
    from: 'README.md',
    section: '## Features',
    title: 'Features and how to use them',
    description: 'Every Rift feature in one table, with the shortcut, menu or setting that reaches it.',
    order: 2,
  },
  {
    slug: 'configuration',
    from: 'docs/configuration.md',
    title: 'Configuration',
    description: 'Where Rift keeps its files and every config.toml key, plus themes, launch configurations, profiles, actions, secret masking, notifications and keybindings.',
    order: 3,
    // Notes for Rift's developers, not for people configuring it.
    drop: ['### Settings UI exposure policy'],
  },
  {
    slug: 'keyboard-shortcuts',
    from: 'docs/keyboard-shortcuts.md',
    title: 'Keyboard shortcuts',
    description: 'Every default shortcut with its action name for [keybindings], plus editor, vim, search, mouse and mark-mode keys.',
    order: 4,
  },
];

// The Windows/Linux shortcuts beside the macOS ones are the ones the app itself shows there.
const keymap = await readFile(`${rift}/crates/rift-core/src/keybindings.rs`, 'utf8');
const defaults = parseDefaults(keymap);

const results = [];
for (const page of PAGES) {
  results.push([page, transformPage(await readFile(`${rift}/${page.from}`, 'utf8'), page, defaults)]);
}

const shortcuts = results.find(([page]) => page.slug === 'keyboard-shortcuts')[1];
const { checked, bad } = checkLinuxKeys(shortcuts, keymap);
if (bad.length || checked < 40) {
  throw new Error(`keyboard-shortcuts.md: Linux shortcuts disagree with rift-core's defaults (${checked} checked):\n${bad.join('\n')}`);
}
for (const [page, text] of results) {
  await writeFile(`${out}${page.slug}.md`, text);
  console.log(`${page.slug}.md  ←  ${page.from}${page.section ? ` (${page.section})` : ''}`);
}
