// Windows and Linux versions of the macOS shortcuts in the docs' key tables, for the macOS /
// Windows and Linux toggle on the keyboard shortcuts page. The sync writes both into the Markdown;
// CSS shows one (macOS without JavaScript). (The class names keep their first name: os-linux.)
//
// The rule is the app's own (Keymap::preferred_for in rift-core): an action shows its first chord
// without ⌘ off macOS (a ⌘-only action has its own `_pc` Windows/Linux key), else its first; ⌘ is
// the Super / Win key there.

const MODS = [['⌃', 'Ctrl'], ['⌥', 'Alt'], ['⇧', 'Shift'], ['⌘', 'Super']];
const KEYS = { '⏎': 'Enter', '⌫': 'Backspace', '⌦': 'Delete' };
const GLYPH = /[⌃⌥⇧⌘⏎⌫⌦]/;

// "⇧⌘C" → "Shift+Super+C"; text that is not a chord stays as it is.
export function linuxChord(text) {
  return text
    .replace(/([⌃⌥⇧⌘]+)(-?)([^\s/`]?)/g, (_, mods, dash, key) => {
      const names = MODS.filter(([g]) => mods.includes(g)).map(([, n]) => n);
      if (!key) return names.join('+') + (dash ? '+-' : ''); // "⌘-" is the minus key; "⌘ + wheel" has none
      return names.join('+') + (dash || '+') + (KEYS[key] ?? key);
    })
    .replace(/[⏎⌫⌦]/g, (k) => KEYS[k])
    .replace(/\b(\w+) \(\1\)/g, '$1'); // "⌦ (Delete)" must not read "Delete (Delete)"
}

// Every action's default chords, in the app's order, from crates/rift-core/src/keybindings.rs: the
// table all platforms share, then the Windows/Linux-only one (`_pc`); `_alt` / `_pc` name the same
// action. Only those two tables are read (the file's tests hold other ("name", "text") pairs).
export function parseDefaults(rustSource) {
  const body = (fn) => {
    const m = new RegExp(`fn ${fn}\\(\\)[^{]*\\{([\\s\\S]*?)\\n\\}`).exec(rustSource);
    if (!m) throw new Error(`keybindings.rs: no fn ${fn}()`);
    return m[1];
  };
  const chords = new Map();
  for (const fn of ['shared_defaults', 'pc_defaults']) {
    for (const [, action, chord] of body(fn).matchAll(/\("([a-z0-9_]+)", "([^"]+)"\)/g)) {
      const base = action.replace(/_(alt|pc)$/, '');
      chords.set(base, [...(chords.get(base) ?? []), chord]);
    }
  }
  return chords;
}

const NAMES = { ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', cmd: 'Super' };
const KEY = { up: '↑', down: '↓', left: '←', right: '→', enter: 'Enter', backspace: 'Backspace', delete: 'Delete', plus: '+', pageup: 'PageUp', pagedown: 'PageDown', escape: 'Esc' };

// "ctrl+shift+p" → "Ctrl+Shift+P", as the docs write a chord.
export function shownChord(chord) {
  const parts = chord.split('+');
  const key = parts.pop();
  const mods = Object.keys(NAMES).filter((m) => parts.includes(m)).map((m) => NAMES[m]);
  return [...mods, KEY[key] ?? (key.length === 1 ? key.toUpperCase() : key[0].toUpperCase() + key.slice(1))].join('+');
}

// The chord the app shows for an action off macOS (Keymap::preferred_for): its first without ⌘,
// else its first.
export function offMacChord(all) {
  return all.find((c) => !c.split('+').includes('cmd')) ?? all[0];
}

// A shortcut cell, given its action name cell (null when the table has none) and the app's
// defaults (`parseDefaults`; optional). A single action the defaults know shows the chord the app
// shows off macOS (a ⌘-only action has its own Windows/Linux key). Otherwise alternatives for one
// action ("⌘B / ⌃⇧B" → bookmark_toggle) collapse to the one without ⌘; pairs for two actions
// ("⌥↑ / ⌥↓" → `_prev` / `_next`) and fixed keys keep every part. Inside `code` nothing changes.
export function linuxShortcut(cell, action, defaults) {
  if (cell.includes('`')) return cell.split(/(`[^`]*`)/).map((p, i) => (i % 2 ? p : linuxChord(p))).join('');
  const single = action != null && action !== '—' && !action.includes(' / ');
  if (single && defaults?.has(action)) return shownChord(offMacChord(defaults.get(action)));
  const parts = cell.split(' / ');
  const picked = single && parts.length > 1 ? (parts.find((p) => !p.includes('⌘')) ?? parts[0]) : cell;
  return linuxChord(picked);
}

// In every Markdown table whose first column is "Shortcut" or "Key", each first cell with a key glyph
// becomes <span class="os-mac">…</span><span class="os-linux">…</span> (`defaults`: as for
// `linuxShortcut`).
export function osKeyTables(md, defaults) {
  let table = null; // { hasAction } while inside a key table
  return md
    .split('\n')
    .map((line) => {
      const head = /^\| (Shortcut|Key) \| (.*) \|$/.exec(line);
      if (head) {
        table = { hasAction: head[2].startsWith('Action name') };
        return line;
      }
      if (!table || !line.startsWith('|')) {
        table = null;
        return line;
      }
      if (/^\|[-| ]+\|$/.test(line)) return line;
      const cells = line.replace(/^\| /, '').replace(/ \|$/, '').split(' | ');
      if (!GLYPH.test(cells[0])) return line;
      const action = table.hasAction ? (cells[1] ?? '').replace(/`/g, '') : null;
      cells[0] = `<span class="os-mac">${cells[0]}</span><span class="os-linux">${linuxShortcut(cells[0], action, defaults)}</span>`;
      return `| ${cells.join(' | ')} |`;
    })
    .join('\n');
}

// Checks every Linux shortcut in a synced page against the app's own default table (`defaults()` in
// crates/rift-core/src/keybindings.rs): the shown chord must be the one Keymap::preferred picks off
// macOS. Rows for ranges (tab_select_1..9, resize_pane_*, navigate_left/right/…) are skipped.
export function checkLinuxKeys(md, rustSource) {
  const chords = parseDefaults(rustSource);
  let checked = 0;
  const bad = [];
  for (const [, linux, actionCell] of md.matchAll(/<span class="os-linux">([^<]*)<\/span> \| (`[^|]+) \|/g)) {
    const names = actionCell.replace(/`/g, '').split(' / ');
    if (names.some((n) => /\.\.|\*|\//.test(n))) continue;
    const full = names.map((n) => (n.startsWith('_') ? names[0].replace(/_[a-z]+$/, '') + n : n));
    const keys = full.length > 1 ? linux.split(' / ') : [linux];
    full.forEach((name, i) => {
      const all = chords.get(name);
      if (!all) return bad.push(`${name}: not in the app's defaults`);
      const want = shownChord(offMacChord(all));
      checked++;
      if (keys[i] !== want) bad.push(`${name}: the page shows "${keys[i]}", the app shows "${want}"`);
    });
  }
  return { checked, bad };
}
