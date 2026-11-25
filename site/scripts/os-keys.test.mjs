import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkLinuxKeys, linuxChord, linuxShortcut, osKeyTables, parseDefaults } from './os-keys.mjs';

test('glyphs become Linux names; keys and plain text stay', () => {
  assert.equal(linuxChord('⇧⌘C'), 'Shift+Super+C');
  assert.equal(linuxChord('⌃⇧1 … ⌃⇧9'), 'Ctrl+Shift+1 … Ctrl+Shift+9');
  assert.equal(linuxChord('⌥⌘← → ↑ ↓'), 'Alt+Super+← → ↑ ↓');
  assert.equal(linuxChord('⌘, ⌘- ⌘⌫ ⇧⏎'), 'Super+, Super+- Super+Backspace Shift+Enter');
  assert.equal(linuxChord('⌘-click / ⇧-click'), 'Super-click / Shift-click');
  assert.equal(linuxChord('⌘ + wheel'), 'Super + wheel');
  assert.equal(linuxChord('PageUp / PageDown'), 'PageUp / PageDown');
  assert.equal(linuxChord('⌫ / ⌦ (Delete)'), 'Backspace / Delete');
});

test('one action with several chords shows the one without ⌘, as the app does off macOS', () => {
  assert.equal(linuxShortcut('⌘B / ⌃⇧B', 'bookmark_toggle'), 'Ctrl+Shift+B');
  assert.equal(linuxShortcut('⇧⌘} / ⌃Tab', 'tab_next'), 'Ctrl+Tab');
  assert.equal(linuxShortcut('⌘= / ⇧⌘+', 'font_increase'), 'Super+=');
  assert.equal(linuxShortcut('⌘↑', 'select_block_prev'), 'Super+↑');
  // Two actions, or fixed keys: every part stays.
  assert.equal(linuxShortcut('⌥↑ / ⌥↓', 'bookmark_closest_prev / _next'), 'Alt+↑ / Alt+↓');
  assert.equal(linuxShortcut('⌘-click / ⇧-click', '—'), 'Super-click / Shift-click');
  assert.equal(linuxShortcut('⌥⌫ / ⌥⌦', null), 'Alt+Backspace / Alt+Delete');
});

test('key tables get both versions in their first column; other tables and code stay', () => {
  const md = [
    '| Shortcut | Action name | Effect |',
    '|---|---|---|',
    '| ⌘B / ⌃⇧B | `bookmark_toggle` | Bookmark |',
    '| — | `pipe_block_output` | Send |',
    '',
    '| Key | Effect |',
    '|---|---|',
    '| `u` / ⌃R | Undo / redo |',
    '',
    '| Gesture | Effect |',
    '|---|---|',
    '| ⌘-click | Open |',
  ].join('\n');
  const out = osKeyTables(md).split('\n');
  assert.equal(out[2], '| <span class="os-mac">⌘B / ⌃⇧B</span><span class="os-linux">Ctrl+Shift+B</span> | `bookmark_toggle` | Bookmark |');
  assert.equal(out[3], '| — | `pipe_block_output` | Send |');
  assert.equal(out[7], '| <span class="os-mac">`u` / ⌃R</span><span class="os-linux">`u` / Ctrl+R</span> | Undo / redo |');
  assert.equal(out[11], '| ⌘-click | Open |');
});

test("the Linux shortcuts are checked against the app's own defaults", () => {
  // keybindings.rs's shape: the shared table, the Windows/Linux `_pc` one, and a test elsewhere
  // whose ("name", "text") pairs are not keys
  const shared = '("bookmark_toggle", "ctrl+shift+b"),\n("bookmark_toggle_alt", "cmd+b"),\n("select_block_prev", "cmd+up"),\n("hide_app", "cmd+h"),\n("bookmark_closest_prev", "alt+up"),\n("bookmark_closest_next", "alt+down"),';
  const pc = '("select_block_prev_pc", "ctrl+up"),';
  const tables = (s) =>
    `fn shared_defaults() -> Vec<(&'static str, &'static str)> {\n    vec![\n${s}\n    ]\n}\n\n` +
    `fn pc_defaults() -> Vec<(&'static str, &'static str)> {\n    vec![\n${pc}\n    ]\n}\n\n` +
    'fn guard() {\n    let allow = [("hide_app", "hiding an app is a macOS concept")];\n}\n';
  const rust = tables(shared);
  const defaults = parseDefaults(rust);
  // the test's pair is no key
  assert.deepEqual(defaults.get('hide_app'), ['cmd+h']);
  const page = osKeyTables(
    ['| Shortcut | Action name | Effect |', '|---|---|---|', '| ⌘B / ⌃⇧B | `bookmark_toggle` | x |', '| ⌘↑ | `select_block_prev` | x |', '| ⌥↑ / ⌥↓ | `bookmark_closest_prev` / `_next` | x |'].join('\n'),
    defaults,
  );
  // a ⌘-only action shows its own Windows/Linux key, not ⌘ as Super
  assert.match(page, /<span class="os-linux">Ctrl\+↑<\/span> \| `select_block_prev`/);
  assert.deepEqual(checkLinuxKeys(page, rust), { checked: 4, bad: [] });
  // The app changes its Linux chord: the check names the row.
  const moved = rust.replace('"ctrl+shift+b"', '"ctrl+alt+b"');
  assert.deepEqual(checkLinuxKeys(page, moved).bad, ['bookmark_toggle: the page shows "Ctrl+Shift+B", the app shows "Ctrl+Alt+B"']);
  assert.deepEqual(checkLinuxKeys(page, rust.replace(/\("select_block_prev(_pc)?"[^\n]*/g, '')).bad, ["select_block_prev: not in the app's defaults"]);
});
