# UI Surfaces

Every visual element in Shift, top to bottom. Layer order for compositing:
0 chrome → 1 terminal + editor → 2 completion/search → 3 settings → 4 palette
→ 5 context menu and hints → 6 toasts and confirm dialogs. Visual tokens and
metrics: [DESIGN.md](../DESIGN.md).

## Window chrome

- Custom title-bar-free window; traffic lights (macOS) close / minimize / maximize.
- The whole chrome strip doubles as a window-drag region on Linux.
- The window reopens at its last position and size (maximized/fullscreen too)
  when `restore_session` is on.
- Mouse pointer: a hand over anything clickable (tabs, buttons, dialog
  buttons, menu and palette rows, settings controls, ⌘-hovered links),
  resize arrows over split handles, the I-beam over terminal text.

## Menu bar (macOS)

Native menus: **Shift** (About, Settings…, Keyboard Shortcuts, Services,
Hide/Hide Others/Show All, Quit) · **File** (New Tab, New Window, Split Pane
Right/Down, Reopen Closed Tab, Save Layout as Launch Configuration, Close
Pane/Other Tabs/Window) · **Edit** (Undo, Redo, Cut, Copy, Paste, Select All,
Find…, Filter Block Output…, Clear Blocks) · **View** (Command Palette,
Workflows, Command History, File Explorer, Code Review, Bigger/Smaller/Actual
Size, Maximize Pane, Toggle Full Screen) · **Blocks** (select/copy/reinput/bookmark/scroll actions) ·
**Window** (Minimize, Zoom, tab and pane navigation, Synchronize Inputs,
Bring All to Front) · **Help** (What's New, Keyboard Shortcuts).

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
- No palette hint on the right (⌘P still opens it).
- Panel buttons (Warp layout), accent while their panel is open: a sidebar
  icon right after the traffic lights toggles the file explorer; at the far
  right, `± +added −removed` for the active pane's repository (reloaded on
  `cd`, tab or pane switch and after each command) toggles code review — a
  bare `±` outside a repository or with no changes. Left of it, an inbox
  icon opens the notifications popover; an accent dot marks unread
  entries. The vertical-tab layout's top strip has the same buttons.
- A tab with synchronized inputs on shows a sync marker.

## Vertical tab rail

`tab_layout = "vertical"` (Settings → Appearance → *Tabs: Side*) replaces the
tab strip with a 220 px rail on the left: one 54 px row per tab with shell,
title, directory, git branch, pane count, sync state, activity dot and tab
color. Click selects, × closes; ⌘T adds a tab; drag a row up or down to
reorder. When the tabs do not fit, the rail keeps the active tab visible: a
`+N above` line at the top and a `+N more · palette lists all` line at the
bottom stand in for the hidden tabs, each marked with the most urgent alert
among them (waiting, then failed, bell, notification, finished; else the
activity dot). Clicking one selects the nearest hidden tab.

## File explorer (⇧⌘E)

A 260 px panel left of the panes (right of the vertical rail when it is on)
showing the active pane's project: the enclosing git working tree, else the
pane's directory. It follows the pane when `cd` leaves the project.

- Header: project name, ⊟ collapse all, ↻ refresh.
- Rows: folders first, then files, case-insensitive; `.git`, `.DS_Store` and
  git-ignored entries hidden; at most 2 000 entries per folder — a bigger
  folder shows the first 2 000 in this order, then `… more entries not
  shown`. A folder that cannot be read shows the error in its place (e.g.
  `Permission denied`). Folders show
  ▸ / ▾ and a dot when they hold changes; changed files are tinted by status
  with a letter at the right (`M` modified, `A` added, `U` untracked, `D`
  deleted).
- Click a folder to expand it, a file to open it in the viewer; ⌥-click
  inserts the quoted path into the command line.
- Keys once clicked: ↑ ↓ move, ← collapses or goes to the parent, → expands,
  ⏎ opens (⌥⏎ inserts the path), Esc returns focus to the terminal. Any other
  key goes to the terminal.
- Listings and git colors reload when a command finishes, after a review
  action, and on ↻.

## File viewer

Opens over the pane area from the explorer or the review panel's ↗: the
path, a `read-only` chip, **Insert path** and ✕. Line numbers and
highlighting for keywords, strings, comments and numbers (C-family and
`#`-comment languages by extension; Markdown and text stay plain). Binary
files and files over 4 MB show a notice instead. Wheel, arrows, Space, Page
Up/Down and Home/End scroll; Esc or ✕ closes. While it is open, typing is
swallowed (app shortcuts still work) and the terminal keeps running
underneath.

⌘F (the `search` shortcut) opens a find bar under the header for the file
instead of the terminal search: case-insensitive plain text, the counter
shows `n/total` or `No matches`, Enter / ⇧Enter step to the next / previous
match (wrapping), Esc closes the bar (a second Esc closes the viewer).

## Code review (⇧⌘G)

A 460 px panel right of the panes listing the repository's changes. The
header shows the branch and total `+added −removed`, and ↻ reloads.

- **Staged** (HEAD → index) and **Changes** (index → working tree, untracked
  files included) sections, each collapsible, with **Unstage all** / **Stage
  all**.
- File rows: status letter, name and folder, `+a −d`, then ↗ open in the
  viewer, ↺ revert (unstaged only) and + stage / − unstage. Click a row to
  expand its diff.
- Hunks: the `@@` header with **Stage** / **Revert** (unstaged) or
  **Unstage** (staged), then the lines with old and new line numbers, added
  lines on a green wash and removed lines on a red one. A binary file shows a
  note; a diff past 4 000 lines is cut with a note, and a hunk the cut runs
  through shows `Too long · use file actions` instead of its buttons.
- Revert asks first ("Revert this change?" / "Revert file?") and discards
  only unstaged changes; reverting an untracked file deletes it.
- A hunk action re-reads the diff first and checks the hunk's line ranges and
  content against the same diff it applies; if the hunk no longer matches,
  nothing is written and the panel shows why above the list.
- Stage, unstage and revert run in order on a background writer thread, so
  a slow repository never blocks input.
- The panel reloads (off the UI thread) when it opens, when a command
  finishes, after each action, and on ↻.

Both panels hide when the window is too narrow to leave 360 px for the panes
(review first). While the review panel is hidden this way, its toolbar button
is not lit, and ⇧⌘G / the button show a "Window too narrow" toast instead of
toggling; widening the window brings the panel back.

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
- **Boot placeholder**: `Starting zsh…` (shell name from config) shown on the
  input line (bottom, or top with `input_position = "top"`) until the shell's
  first prompt, with the terminal cursor hidden. A shell without integration
  never marks a prompt; the placeholder leaves and its cursor shows 1.5 s after
  its first byte.

## Split panes

- Vertical / horizontal splits, arbitrary nesting (binary tree).
- Active pane: accent border + glow; inactive panes dim.
- 10 px drag handles between panes with grip indicator; ⌃⌘arrows resize from
  the keyboard (±5% per press).
- Zoom (⇧⌘⏎): the active pane temporarily takes the whole tab rect and renders
  like a single pane (full editor, no gutters). Navigating or splitting unzooms.
- Compact editor (input line only, no context row) inside splits.
- The tab shows a `N panes` pill, and the tab strip's right edge shows a
  `⌃⇧D split` pill (chords follow the keymap).

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
- **History** (⌃R from the editor) — imported shell history (zsh, bash or
  fish, from the configured shell's history file) + per-pane local history; Enter runs, or insert into the editor.
- **Workflows** (⌃⇧R) — Warp-compatible YAML workflows; selecting one inserts
  its command with Tab-cyclable placeholders.
- **Session switcher** (⇧⌘O) — tabs, panes of split tabs (directory and
  running job), tabs of other Shift windows, detached session-service shells,
  and the 40 newest finished blocks. Enter jumps there.

Long rows are cut with `…` inside the dialog.

## Notifications

**Inbox popover** — dropped from the inbox button, or ⇧⌘A, 420 px wide and
at most 620 px tall.

- Header `Notifications`, `Mark all as read` (while anything is unread) and
  ✕; chips `All tabs (N)`, `Unread (N)` and `Errors (N)` filter the list
  (Tab / ⇧Tab cycle them).
- Rows, newest first: a badge — the agent's (Claude Code orange `*`, Codex
  `>_`, OpenCode `oc`, Gemini `G`) with a kind dot, or a tinted kind disc
  (✓ finished, ✕ failed, `!` waiting on you, dot for a program
  notification, bell) — an accent dot while unread, the agent name and
  `⎇ branch`, the title (bold, wrapped, at most four lines, then `…`), its
  detail (`Completed in 1.2s.`, `Failed with exit 1 after 3s.`,
  `Waiting for your input.`, a notification body) and the age on the right
  (`Just now`, `4 minutes ago`).
- Click a row, or ↑/↓ then ⏎, to jump to its pane and block (marks it
  read). The wheel scrolls the list. Esc, ✕, a right-click, a click outside
  or a plain key closes it.
- An entry turns read when you open it, when you look at its pane, or with
  `Mark all as read`.
- `No notifications`, `Nothing unread` or `No errors` when the list is
  empty.

**Toasts** — while Shift is focused, an alert from a pane you are not
looking at appears as a 360 px card under the inbox button: badge, a kind
stripe on the left edge, `agent · branch`, the title (two lines) and detail.
At most two show, newest on top. Each stays `toast_secs` (default 8);
hovering keeps it. A click jumps to the pane; ✕ dismisses. Toasts hide while
the popover or Settings is open.

**Tab markers** — a tab with unread entries shows a dot in the newest one's
kind color (green finished, red failed, yellow waiting, accent notification)
in place of the activity dot.

## Hint mode (⇧⌘H)

Every visible URL, existing file path and block header gets a short letter
label drawn over it. Typing a label opens the link or path, or selects the
block; ⇧ + label copies it (a block copies its output). Esc cancels.

## Dialogs and toasts

- **Confirm dialogs** — warning icon, title, the affected commands (at most a
  few, then `and N more…`), Cancel and a confirm button. Used for closing
  running commands, pasting multi-line text into a shell without bracketed
  paste ("Paste and run?"), and installing shell integration on an SSH host.
  Enter confirms, Esc cancels. An Enter that is auto-repeated, or that
  arrives within 300 ms of the dialog opening (a held or typed-ahead key),
  is ignored.
- **Toasts** — short notices at the bottom (a keybinding moved from another
  action, the session service could not start, config.toml was not applied,
  a copy, paste, open or save failed, …).
- **What's New** — release notes from `CHANGELOG.md`, newest first: title
  with the current version and date, then each release's summary and its
  New / Improved / Fixed items (section titles in accent / success / warning).
  Opens from Help → What's New or the palette (`whats_new`), and once on the
  first launch after an update (never on a first install). The body scrolls
  with the wheel or trackpad, arrows, Space, Page Up/Down and Home/End; Esc,
  Enter, ✕, **Got it** or a click outside the card closes it.

## Search (⌃⇧F / ⌘F)

Bar overlay; live yellow highlight of all matches, orange for the current one.
Enter/⇧Enter or ⌘G/⇧⌘G cycle matches (viewport jumps to each), Esc closes.
Two toggle chips at the right: `Aa` match case (⌥C) and `.*` regular
expression (⌥R); an invalid regex shows `invalid regex` in red instead of the
counter. The counter shows the current match's position among all matches
(`n/total`); only the newest 500 matches are kept for navigation (older
ones are counted, not visited), so with 600 matches it runs 101/600 …
600/600. A query longer
than the field scrolls so its end stays visible. With the file viewer open,
⌘F finds in the file instead (see File viewer).

With a block filter active (⌥⇧F), the same bar edits the filter: only that
block's output lines containing the text stay visible.

## Block context menu (right-click or ⌃M)

Copy command · Copy output · Copy both · Copy working directory ·
Copy git branch · Re-run command · Bookmark/Remove bookmark ·
Collapse/Expand · Filter output…/Clear output filter · Send output to
command… · Scroll to top of block · Scroll to bottom of block.
Disabled items are dimmed (e.g. Copy output on an output-less block).

## Settings overlay (⌘,)

Header (`❯ Settings`), a six-entry sidebar with a per-page icon (arrow keys navigate pages;
PageUp/PageDown scroll the body) and a scrollable body footed with
`~/.config/shift/config.toml`. Keyboard: Tab / ⇧Tab move a focus ring over
the page's controls, Space or Enter presses the focused control, ← / →
adjust a focused slider, and Esc drops the focus (the next Esc closes
Settings). The arrow keys change page unless a slider is focused; Home /
End jump to the first / last page. Every control writes that file live (the
hot-reload watcher applies changes < 300 ms, including external edits); while
config.toml is invalid, controls do not save and a toast says to fix it first; the
wheel scrolls the page body (24 px per line, clamped to the content) and
switching pages resets the scroll. ✕ closes; clicks elsewhere in the overlay
are swallowed rather than reaching the terminal.

- **General** — *Shell and session behavior*.
  - Shell: `Shell path` (read-only: `shell` override, else `$SHELL`),
    `Scrollback` stepper (lines, ±1000, shown as `10,000`; described
    *Maximum lines kept in the buffer*).
  - Session: `Restore tabs on launch` toggle, `New tabs start in` segmented
    `Previous | Home | Custom` (a custom path is set in config.toml),
    `Warn before closing running commands` toggle,
    `Save block output for restore` toggle (*Output can contain secrets;
    turning this off deletes it*; `save_transcripts`),
    `Keep shells running when Shift quits` toggle (*Session service;
    relaunch reattaches (new shells)*; `durable_sessions`).
  - Automation: `Control socket` segmented `Off | Navigate | Full`
    (`control_api`).
  - Terminal: `Mouse reporting to programs (Shift selects)` toggle,
    `Ask before pasting lines that would run` toggle, and on macOS
    `Option key sends Meta`.
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
- **Features** — *Input and blocks*.
  - Input: `Vim keybindings` toggle, `Autosuggestions (ghost text)` toggle,
    `Completions menu` toggle, `Autocomplete quotes and brackets` toggle,
    `Input position` segmented `Bottom | Top`, `Prompt` segmented
    `Shift | Shell (PS1)`, `Expand aliases as you type`, `Underline unknown
    commands`, `Suggest corrections for failed commands` toggles.
  - Blocks: `Copy on select` toggle.
- **Notifications** (bell icon) — *Toasts, desktop notifications and sounds;
  everything also lands in the inbox*.
  - Alerts: `Notifications` master toggle, `In-window toasts` toggle,
    `Toast duration` stepper (s, 2–60).
  - Notify me when: `A long command finishes` toggle, `Long command
    threshold` stepper (s, ±5), `A command fails`, `An agent finishes its
    task`, `A program or agent needs attention`, `A password prompt appears`
    toggles.
  - Sounds: `Play notification sounds` toggle; `Sound`, `Finished`, `Failed`,
    `Needs attention` selects — a click steps to the next sound (`Default`,
    `None`, then every sound file found) and plays it; the per-event ones
    start at `Same as Sound`.
  - Terminal bell: `Bell` segmented `Visual | Audible | Both | None`.
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
