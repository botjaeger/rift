import assert from 'node:assert/strict';
import { test } from 'node:test';
import { groupRows, inline, parseRows } from '../src/lib/feature-rows.ts';

test('inline markdown: escaped, code kept literal, emphasis, key chords as kbd', () => {
  assert.equal(inline('runs `cmd < output` <b>'), 'runs <code>cmd &lt; output</code> &lt;b&gt;');
  assert.equal(inline('**Bold** and *Menu item*'), 'Bold and <em>Menu item</em>');
  assert.equal(inline('⇧⌘C / ⌥⇧⌘C, ⌃⇧1…9'), '<span class="kbd">⇧⌘C</span> / <span class="kbd">⌥⇧⌘C</span>, <span class="kbd">⌃⇧1…9</span>');
  // Not chords: a modifier joined to a word, and anything inside code.
  assert.equal(inline('⌘-click, ⌥⌘arrows, `⌘P`'), '⌘-click, ⌥⌘arrows, <code>⌘P</code>');
});

test('table rows parse to name and how; header and other lines are skipped', () => {
  const rows = parseRows('intro\n\n| Feature | How to use it |\n|---|---|\n| **Blocks** — header | ⌘↑ |\n| Clear | `clear` |\n\nafter');
  assert.deepEqual(rows.map((r) => r.plain), ['Blocks: header', 'Clear']);
  assert.equal(rows[1].how, '<code>clear</code>');
  assert.throws(() => parseRows('| a | b | c |'), /2 cells/);
});

test('every row lands in exactly one group, and every prefix is used', () => {
  const rows = parseRows('| Session restore | x |\n| Session switcher | y |\n| Window — size | z |');
  const [a, b] = groupRows(rows, [{ prefixes: ['Session restore', 'Window'] }, { prefixes: ['Session switcher'] }]);
  assert.deepEqual([a.rows.length, b.rows.length], [2, 1]);
  assert.throws(() => groupRows(rows, [{ prefixes: ['Session restore'] }]), /matches 0 groups/);
  assert.throws(() => groupRows(rows, [{ prefixes: ['Session'] }, { prefixes: ['Session restore', 'Window'] }]), /matches 2 groups/);
  assert.throws(() => groupRows(rows, [{ prefixes: ['Session restore', 'Session switcher', 'Window', 'Gone'] }]), /matches no README row/);
  // A prefix only matches whole words: "Window" is not "Windows".
  assert.throws(() => groupRows(parseRows('| Windows | x |'), [{ prefixes: ['Window'] }]), /matches 0 groups/);
});
