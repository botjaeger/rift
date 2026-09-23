# UI Surfaces

Every visual element in Shift, top to bottom. Layer order for compositing:
0 chrome → 1 terminal + editor → 2 completion/search → 3 settings → 4 palette.

## Window chrome

- Custom title-bar-free window; traffic lights (macOS) close / minimize / maximize.
- The whole chrome strip doubles as a window-drag region on Linux.
- The window reopens at its last position and size (maximized/fullscreen too)
  when `restore_session` is on.

## Menu bar (macOS)

Native menus: **Shift** (About, Settings…, Keyboard Shortcuts, Services,
Hide/Hide Others/Show All, Quit) · **File** (New Tab, New Window, Split Pane
Right/Down, Reopen Closed Tab, Save Layout as Launch Configuration, Close
Pane/Other Tabs/Window) · **Edit** (Undo, Redo, Cut, Copy, Paste, Select All,
Find…, Filter Block Output…, Clear Blocks) · **View** (Command Palette,
Workflows, Command History, Bigger/Smaller/Actual Size, Maximize Pane, Toggle
Full Screen) · **Blocks** (select/copy/reinput/bookmark/scroll actions) ·
**Window** (Minimize, Zoom, tab and pane navigation, Synchronize Inputs,
Bring All to Front) · **Help** (Keyboard Shortcuts).

Shortcut labels come from the live keymap (`[keybindings]` overrides show up
after the reload); only ⌘ chords become menu key equivalents so ctrl chords
stay with the terminal. Choosing an item replays its chord through the same
key routing as the keyboard (overlays, editor vs raw mode). Quit goes through
the running-command warning. Linux has no native menu bar (the palette lists
every action).

## Tab bar

- One tab per shell session group. Label = shell name (`zsh`) + tab title
  (OSC 0/2 title, else cwd basename; `· N panes` suffix when split).
- Activity pulse dot on background tabs that produced output since last viewed.
- `×` closes the tab (all its panes), `+` spawns a new tab.
- Right side: `⌘P palette` hint.

## Blocks (terminal area)

Each command becomes a block via shell-integration OSC 133 markers:

- **Header row** (synthesized, replaces the shell's prompt echo):
  - collapse chevron `▾`/`▸` — click toggles collapse
  - command text (bright)
  - context: `~/path git:(branch)` — cwd and branch *at the time the command ran*
    (dim; collapsed blocks show an italic `N lines hidden` here instead)
  - right side: status badge pill (`✓ 0` green / `✕ N` red / amber `running`
    with a spinning ring /
    `—` unknown), duration (live elapsed while running), copy-output button
- **Left border**, 2 px, colored by state: accent = running, green = exit 0,
  red = exit ≠ 0, muted = no exit info. Consecutive blocks' borders are 2 px
  apart (design 01 block spacing).
- **Block wash**: only the selected block gets an accent tint + 3 px accent
  border; failed blocks are not tinted (exit state lives on the border and
  badge, design 01).
- **Selected block**: selected via click on the header or ⌘↑/⌘↓; target of
  ⇧⌘C / ⌥⇧⌘C / ⌘I / ⌘B / ⌃M; Esc or clicking again deselects.
- **Sticky command header**: when a block's output scrolls but its header would
  leave the viewport, the header pins to the top edge; clicking it jumps to the
  block.
- **Bookmark dots** on the right edge (position proportional to scrollback
  position); click jumps to the bookmarked block.
- **Header overflow**: a long command is cut with `…` before the badge group;
  the cwd/branch context only shows when it fits. Multi-line commands show
  `⏎` between lines.
- **Links**: http(s) URLs get a subtle underline; OSC 8 hyperlinks (`ls
  --hyperlink`, compilers, `delta`) are underlined too. Hold ⌘ and the pointer
  turns into a hand over a URL, a hyperlink or an existing file path
  (absolute, `~/…`, or relative to the pane's cwd; `file:line:col` suffixes
  are accepted) — ⌘-click opens it with the system opener. Only http, https,
  file, mailto and ftp targets open.
- **Cell styles**: bold, italic (embedded JetBrains Mono Italic/Bold Italic),
  dim, inverse, hidden, strikethrough, and underline variants (single, double,
  curly, dotted, dashed; SGR 58 colored).

## Command editor (input zone)

App-owned line editor shown while the shell is at a prompt (raw keys pass
through to the PTY otherwise — vim, password prompts, shells without
integration). Sits at the bottom of the pane, or at the top with
`input_position = "top"`.

- **Prompt context** (design 01, above the input): `⇕ ~/zzz/shift on ⎇ main
  ↑2 ↓1 •` — cwd in the accent, branch in green, ahead/behind arrows and the
  dirty dot muted. Hidden in compact (split-pane) mode and in shell-prompt
  mode.
- **Prompt glyph** `❯` — green normally, red after a non-zero exit, muted in
  an unfocused split pane. In
  shell-prompt mode (`prompt = "shell"`) the shell's own PS1 stays on the grid
  above and the editor shows a muted `›` instead.
- **Live syntax highlighting**: command / flags / strings / operators.
- **Multi-line commands**: ⇧⏎ / ⌥⏎ add a line; the zone grows 20 px per extra
  line and each continuation row carries a dim `…` in the prompt column.
- **Soft wrap**: a line longer than the editor wraps onto more rows (wide
  CJK/emoji characters count two cells); the zone grows with it, and cursor,
  selection, clicks and error underlines follow the wrapped rows.
- **IME**: composition text (Korean, Japanese, Chinese, dead keys) shows
  underlined at the cursor and the OS candidate window follows the cursor, in
  the editor and in raw mode.
- **Drag and drop**: files dropped on the window are pasted as shell-quoted
  paths.
- **Cursor**: `cursor_style` = block (8×18 px), bar (2 px) or underline
  (2 px), 1.1 s blink; exact shaped-text positioning, translucent when a glyph
  sits under it. Vim NORMAL always draws a solid, non-blinking block.
- **Vim mode pill** at the right edge of the input line when `vim_mode` is on:
  `NORMAL` (warning color) or `INSERT` (muted).
- **Ghost text**: inline dimmed completion preview after the cursor;
  Tab or ⌃→ (word-at-a-time) accepts.
- **Completion dropdown**: git subcommands with descriptions, path completions
  (with `~/` expansion), history matches. ↑/↓ navigate, Tab accepts, Esc closes.
  Floats under the input line (design 04) and flips above the whole editor
  zone when the editor sits at the window bottom.
- **Workflow placeholders**: `{{placeholder}}` spans highlight and Tab-cycle;
  typing replaces the active span.
- **Boot placeholder**: `Starting zsh…` (shell name from config) shown until
  the shell's first output byte.

## Split panes

- Vertical / horizontal splits, arbitrary nesting (binary tree).
- Active pane: accent border + glow; inactive panes dim.
- 10 px drag handles between panes with grip indicator; ⌃⌘arrows resize from
  the keyboard (±5% per press).
- Zoom (⇧⌘⏎): the active pane temporarily takes the whole tab rect and renders
  like a single pane (full editor, no gutters). Navigating or splitting unzooms.
- Compact editor (input line only, no context row) inside splits.
- The tab shows a `N panes` pill, and the tab strip's right edge swaps the
  palette hint for a `⌃⇧D split` pill (chords follow the keymap).

## Command palette (⌘P)

Fuzzy-find modal (nucleo matcher) with match-character highlighting and
shortcut chips. Modes:

- **Commands** — every app action (blocks, scrolling, tabs, panes, font,
  search, history, workflows and settings) with the resolved custom shortcut;
  keyboard selection scrolls through long result sets. Also includes `Toggle
  vim keybindings`, one
  `Launch configuration: <name>` row per file in
  `~/.config/shift/launch_configs/` (re-scanned each time the palette opens),
  `Theme: <name>` rows, plus recent commands.
- **History** (⌃R from the editor) — imported zsh history + per-pane local
  history; Enter runs, or insert into the editor.
- **Workflows** (⌃⇧R) — Warp-compatible YAML workflows; selecting one inserts
  its command with Tab-cyclable placeholders.

## Search (⌃⇧F / ⌘F)

Bar overlay; live yellow highlight of all matches, orange for the current one.
Enter/⇧Enter or ⌘G/⇧⌘G cycle matches (viewport jumps to each), Esc closes.
Two toggle chips at the right: `Aa` match case (⌥C) and `.*` regular
expression (⌥R); an invalid regex shows `invalid regex` in red instead of the
counter.

## Block context menu (right-click or ⌃M)

Copy command · Copy output · Copy both · Copy working directory ·
Copy git branch · Re-run command · Bookmark/Remove bookmark ·
Collapse/Expand · Scroll to top of block · Scroll to bottom of block.
Disabled items are dimmed (e.g. Copy output on an output-less block).

## Settings overlay (⌘,)

Header (`❯ Settings`), a five-entry sidebar with a per-page icon (arrow keys navigate pages;
PageUp/PageDown scroll the body) and a scrollable body footed with
`~/.config/shift/config.toml`. Every control writes that file live (the
hot-reload watcher applies changes < 300 ms, including external edits); the
wheel scrolls the page body (24 px per line, clamped to the content) and
switching pages resets the scroll. ✕ closes; clicks elsewhere in the overlay
are swallowed rather than reaching the terminal.

- **General** — *Shell and session behavior*.
  - Shell: `Shell path` (read-only: `shell` override, else `$SHELL`),
    `Scrollback` stepper (lines, ±1000, shown as `10,000`; described
    *Maximum lines kept in the buffer*).
  - Session: `Restore tabs on launch` toggle, `New tabs start in` segmented
    `Previous | Home | Custom` (a custom path is set in config.toml),
    `Warn before closing running commands` toggle.
  - Terminal: `Mouse reporting to programs (Shift selects)` toggle, and on
    macOS `Option key sends Meta`.
- **Appearance** — *Fonts, cursor, window and theme*.
  - Font: `Font family` select (cycles the installed monospace families),
    `Font size` stepper (px), `Line height` slider.
  - Cursor: `Cursor style` segmented `Block | Bar | Underline`,
    `Cursor blink` toggle (*Pulse the cursor when the editor is focused*).
  - Window & theme: `Active theme` select (background swatch ringed in the
    theme's accent + display name), `Sync with OS appearance` toggle,
    `Light theme (OS sync)` select (same swatch style), `Window opacity`
    slider (%, value boxed right of the track),
    `Window padding` stepper (px, ±2), `Font ligatures` toggle (*Render -> >=
    != as combined glyphs*).
- **Features** — *Input, blocks, notifications*.
  - Input: `Vim keybindings` toggle, `Autosuggestions (ghost text)` toggle,
    `Completions menu` toggle, `Autocomplete quotes and brackets` toggle,
    `Input position` segmented `Bottom | Top`, `Prompt` segmented
    `Shift | Shell (PS1)`, `Expand aliases as you type`, `Underline unknown
    commands`, `Suggest corrections for failed commands` toggles.
  - Blocks & notifications: `Copy on select` toggle, `Notify when a long
    command finishes` toggle, `Notify after` stepper (s, ±5), `Bell` segmented
    `Visual | Audible | Both | None`.
- **Keybindings** — *Click a shortcut to record a new one · type to filter*:
  a filter row, then every bindable action (including ones that ship unbound)
  with its first chord as chips; overridden rows get accent-edged chips and a
  ↺ reset button, unbound rows say `unbound`. Typing filters by action name or
  shortcut (⌫ edits, Esc clears). Clicking a row records: the next chord with
  ⌘/⌃/⌥ (or a function/navigation key) becomes the shortcut, Esc cancels,
  ⌫ unbinds; a chord taken by another action moves over with a toast naming
  it. ⌃⌘K jumps straight here.
- **Themes** — *Built-in themes plus `~/.config/shift/themes` (TOML or Warp
  YAML)*: a row per available theme with four swatches (background, accent,
  success, error), its display name and an `active` / `light` tag; clicking a
  row sets `theme`.

## Scrollback

- Trackpad/wheel scrolling with fractional-delta accumulation (no dropped
  ticks). The wheel scrolls the pane under the pointer; programs that track
  the mouse get wheel reports, other full-screen apps get arrow keys
  (alternate-scroll mode, app-cursor aware).
- PageUp/PageDown page, Home/End jump to top/bottom (at the prompt; keys pass
  through to full-screen apps).
- `clear` performs a Warp-style soft clear: the viewport empties, old blocks
  stay reachable by scrolling up. ⌘K is the destructive variant (blocks +
  scrollback wiped).
