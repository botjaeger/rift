// pnpm test
import test from 'node:test';
import assert from 'node:assert/strict';
import { dropSection, dropTestItems, escapePlaceholders, rewriteLinks, takeSection, transformPage } from './docs-transform.mjs';

const page = { from: 'docs/x.md', title: 'X', description: 'd', order: 1 };

test('links to published pages point at the site, with their anchor and a readable title', () => {
  assert.equal(rewriteLinks('see [keyboard-shortcuts.md](keyboard-shortcuts.md)'), 'see [Keyboard shortcuts](/docs/keyboard-shortcuts/)');
  assert.equal(rewriteLinks('[profiles](configuration.md#profiles)'), '[profiles](/docs/configuration/#profiles)');
  assert.equal(rewriteLinks('[list](docs/getting-started.md)'), '[list](/docs/getting-started/)');
  assert.equal(rewriteLinks('[phone](features.md#remote-access-phone-companion)'), '[phone](/docs/phone/)');
});

test('links to unpublished pages and the private repo become plain text; others stay', () => {
  assert.equal(rewriteLinks('[updates](features.md#updates)'), 'updates');
  assert.equal(rewriteLinks('[src](../src/app.rs)'), 'src');
  assert.equal(rewriteLinks('[repo](https://github.com/botjaeger/rift)'), 'repo');
  assert.equal(rewriteLinks('[rel](https://github.com/botjaeger/rift-releases)'), '[rel](https://github.com/botjaeger/rift-releases)');
  assert.equal(rewriteLinks('[top](#top)'), '[top](#top)');
});

test('pointers that only send the reader to an unpublished page are dropped whole', () => {
  assert.equal(rewriteLinks('Restored. See [durable\nsessions](features.md#durable-sessions) for recovery.\n'), 'Restored.\n');
  assert.equal(rewriteLinks('running shells (see\n[updates](features.md#updates)).'), 'running shells.');
  assert.equal(rewriteLinks('too. Details: [SSH profiles](features.md#ssh-profiles).'), 'too.');
  assert.equal(rewriteLinks('pane. Details:\n[A](features.md#a),\n[b](features.md#b).\n\n## Next'), 'pane.\n\n## Next');
  // Not a pointer: the words stay.
  assert.equal(rewriteLinks('Use [SSH profiles](features.md#ssh) to save hosts.'), 'Use SSH profiles to save hosts.');
});

test('test-only switches are dropped with their continuation lines; other bullets and code stay', () => {
  const md = '- `RIFT_NO_NOTIFY=1` off.\n- `RIFT_TEST_X` (tests): a\n  headless run.\n- `RIFT_CLI_DIR`: folder.\n\n```sh\n- not a bullet\n```';
  assert.equal(dropTestItems(md), '- `RIFT_NO_NOTIFY=1` off.\n- `RIFT_CLI_DIR`: folder.\n\n```sh\n- not a bullet\n```');
  assert.throws(() => transformPage('# T\nRun with `--script` here.\n', page), /test-only/);
});

test('placeholders in prose are escaped; code and real tags are left alone', () => {
  assert.equal(escapePlaceholders('"<device> ran <cmd>"'), '"&lt;device&gt; ran &lt;cmd&gt;"');
  assert.equal(escapePlaceholders('keep `<data dir>` and <br>'), 'keep `<data dir>` and <br>');
  assert.equal(escapePlaceholders('```\n# <name>\n```'), '```\n# <name>\n```');
});

test('sections are cut at the next heading of the same or higher level, never at # in code', () => {
  const md = '# T\n## A\na\n```toml\n# not a heading\n```\n### A.1\nx\n## B\nb';
  assert.equal(takeSection(md, '## A'), 'a\n```toml\n# not a heading\n```\n### A.1\nx');
  assert.equal(dropSection(md, '### A.1'), '# T\n## A\na\n```toml\n# not a heading\n```\n## B\nb');
  assert.throws(() => takeSection(md, '## Missing'), /not found/);
});

test('sample paths become the generic names', () => {
  const out = transformPage('# T\ncwd = "/Users/me/code/rift"\n', page);
  assert.match(out, /project1/);
  assert.doesNotMatch(out, /code\/rift"/);
});

test('private paths, the private repo and unpublished docs stop the sync', () => {
  assert.throws(() => transformPage('# T\nsee /Users/alice/work\n', page), /matches/);
  assert.throws(() => transformPage('# T\nclone https://github.com/botjaeger/rift.git\n', page), /matches/);
  assert.throws(() => transformPage('# T\nsee ui.md for keys\n', page), /does not publish/);
  assert.throws(() => transformPage('# T\nbuild `packaging/relay.Dockerfile`\n', page), /does not publish/);
  assert.throws(() => transformPage('# T\nan earlier name of the app (`old`)\n', page), /matches/);
});

test('a clean page gets front matter and loses its H1', () => {
  const out = transformPage('# Title\n\nBody text.\n', page);
  assert.equal(out, '---\ntitle: "X"\ndescription: "d"\norder: 1\nsource: "docs/x.md"\n---\nBody text.\n');
});
