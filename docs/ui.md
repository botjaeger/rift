# UI Surfaces

Every visual element in Rift, top to bottom. Layer order for compositing:
0 chrome → 1 terminal + editor → 2 completion, history, search, file viewer →
3 settings page, maximized review → 4 palette, settings select lists →
5 context and chip menus, hints, notifications popover → 6 toasts, confirm
dialogs, What's New, Edit prompt → 7 tooltips. Visual
tokens, metrics, radii and the icon set: [DESIGN.md](../DESIGN.md). The
design source for the current look is the *Rift Terminal Redesign* canvas
(Claude Design artifact: start session, tabs, panes, moving panes, reading
pane, history, autocomplete, palette, settings, controls, menus, panels).

## Window chrome

- Custom title-bar-free window; traffic lights close / minimize / maximize,
  vertically centred on the tab strip (native buttons moved there on macOS,
  drawn on Linux).
- The empty part of the chrome strip drags the window, and a double-click
  there zooms (maximizes) it. Tabs and buttons never do: on macOS the content
  view never acts as titlebar (`mouseDownCanMoveWindow` is NO) and the native
  titlebar is trimmed to the traffic lights, so a double-click on a tab only
  renames it.
- The window reopens at its last position and size (maximized/fullscreen too)
  when `restore_session` is on.
- Mouse pointer: a hand over anything clickable (tabs, buttons, dialog
  buttons, menu and palette rows, settings controls, ⌘-hovered links),
  resize arrows over split handles, the I-beam over terminal text.
- Hover highlights (tabs, welcome rows, pane controls, settings rows) repaint
  the moment the pointer crosses into or out of a control, not on the next
  frame something else draws.

## Menu bar (macOS)

Native menus: **Rift** (About Rift — Settings › About, Check for Updates…, Settings…, Keyboard Shortcuts, Services,
Hide/Hide Others/Show All, Quit) · **File** (New Tab, New Window, Split Pane
Right/Down, Reopen Closed Tab, Save Layout as Launch Configuration, Close
Pane/Other Tabs/Window) · **Edit** (Undo, Redo, Cut, Copy, Paste, Select All,
Find…, Filter Block Output…, Clear Blocks) · **View** (Command Palette,
Command History, File Explorer, Code Review, Bigger/Smaller/Actual
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

- One tab per shell session group: the tab title (OSC 0/2 title, else cwd
  basename) and, when split, a pane count chip (`3`) at its right end. As in Warp, the strip
  is 34 px tall and every tab 200 px wide (a longer title is cut short);
  tabs that do not fit shrink evenly to 120 px, then scroll.
- The active tab joins the pane below (same ground, no rule) with a 2 px
  accent top edge (its tab color when it has one); a hovered tab lifts. A
  colored tab shows its color as the 2 px top edge when inactive too.
- `×` shows on the active and the hovered tab and closes it (all its
  panes); on the others its place shows the tab's alert badge or an accent
  dot for output since last viewed. `+` spawns a new tab.
- Drag a tab: past 4 px it lifts and follows the pointer, its neighbours
  reorder as it passes their midpoints, and its slot shows an accent caret.
  Esc while dragging puts it back where it was pressed.
- When the tabs no longer fit they shrink to 120 px, then scroll around the
  active one behind `‹ N` / `N ›` chips.
- Panel buttons (Warp layout), accent while their panel is open: a sidebar
  icon right after the traffic lights toggles the file explorer; at the far
  right, `+added −removed` for the active pane's repository (reloaded on
  `cd`, tab or pane switch and after each command) toggles code review — a
  bare `±` outside a repository or with no changes. Left of it, an inbox
  (bell) icon opens the notifications popover; an accent dot marks unread
  entries. The vertical-tab layout's top strip has the same buttons.
- **Update Rift** — while a newer release is available (and not dismissed
  with *Later*), a 22 px pill left of the inbox: 1 px `warning` outline,
  `warning` text (12 px medium), no fill, a hover lift. Tooltip `vX.Y.Z
  available`; a click opens the update dialog (see *Dialogs and toasts*).
- **Remote · <device>** — while a phone is connected through remote access,
  the same pill in `accent`, left of *Update Rift* (or of the inbox):
  `Remote · <device name>` or `Remote · N devices` (label cut at 200 px).
  Tooltip `Connected: <names>` (`<name> via relay` for a relayed phone); a
  click opens Settings › Remote access.
- A tab with synchronized inputs on shows the sync icon in the accent.
- A tab where a command runs (in any of its panes) shows a spinner before
  its title, in place of the sync icon; it turns in background tabs too. A
  coding agent waiting at its own prompt (Claude Code's `✳` title, Gemini
  CLI's `◇ Ready`) is not working, so it shows none (Warp).
  A working agent (`◐` or a braille spinner leading its title) keeps the
  spinner even when its start was lost (reattached after a restart), and
  under a custom tab name.
- A program that owns the screen (Claude Code, vim) sits on the bottom
  padding: the part of a row left over goes above its first row.
- The Settings tab (⌘,) sits after the shell tabs with a settings icon; see
  *Settings tab*.

## Vertical tab rail

`tab_layout = "vertical"` (Settings → Appearance → *Tabs: Side*) replaces the
tab strip with a 220 px rail on the left: one 54 px row per tab with the
title (after a spinner while a command runs, else a sync icon when synced), directory, git branch, pane count, activity
dot and tab color; the active row sits on the pane ground with a 2 px accent
left edge. Click selects, × closes; ⌘T adds a tab; drag a row up or down to
reorder. When the tabs do not fit, the rail keeps the active tab visible: a
`+N above` line at the top and a `+N more · palette lists all` line at the
bottom stand in for the hidden tabs, each marked with the most urgent alert
among them (waiting, then failed, bell, notification, finished; else the
activity dot). Clicking one selects the nearest hidden tab.

## File explorer (⇧⌘E)

A 260 px panel left of the panes (right of the vertical rail when it is on)
showing the active pane's project: the enclosing git working tree, else the
pane's directory. It follows the pane when `cd` leaves the project.

- Header: project name in uppercase, collapse all (chevron), refresh. An
  empty folder says `Empty folder`; before the shell has reported its
  directory the panel says `Waiting for the shell…`.
- Rows carry a folder or file icon (a changed file's icon takes its status
  color).
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
  ⏎ opens a file or toggles a folder (⌥⏎ inserts the path), Esc returns focus to the terminal. Any other
  key goes to the terminal.
- Listings and git colors reload when a command finishes, after a review
  action, and on ↻.

## File viewer

Opens from the explorer or the review panel's ↗ as its own pane: the
leftmost column of the tab, next to the explorer (two panes become three; the tab's pill
counts it). Opening another file reuses that pane. It takes focus like any
pane (click, ⌥⌘arrows) and ⌘W, Esc or ✕ closes it. Its header is the
pane's title bar (same height, drags the pane): the path centred (it moves
to the left edge when the pane is too narrow to centre it), then, for
Markdown files, a **Rendered | Raw** switch, ⋮ (Insert path, Copy path, Find
in file, Show raw/rendered, Close) and ✕.

Markdown files (`.md`, `.markdown`, `.mdx`…) open **Rendered**: headings
(H1/H2 with a rule), paragraphs wrapped to the pane, bullet, numbered and
task lists, quotes, fenced and indented code blocks (a wide one scrolls sideways on its own),
rules and tables (columns sized to their content and wrapped to fit), with
bold, italic, strikethrough, inline code and links styled. **Raw** shows the
source with line numbers; switching views returns to the top, and ⌘F
switches to Raw because find works on the source. In Raw and for every
other file: line numbers and
highlighting for keywords, strings, comments and numbers (C-family and
`#`-comment languages by extension; Markdown and text stay plain). Binary
files and files over 4 MB show a notice instead. Wheel, arrows, Space, Page
Up/Down and Home/End scroll; a trackpad swipe, ⇧ + wheel or ← → scroll
sideways (the source view as a whole, its line numbers staying put; in
Rendered, the code block under the pointer). Esc or ✕ closes. While it is open, typing is
swallowed (app shortcuts still work) and the terminal keeps running
underneath.

⌘F (the `search` shortcut) opens a find bar under the header for the file
instead of the terminal search: case-insensitive plain text, the counter
shows `n/total` or `No matches`, Enter / ⇧Enter step to the next / previous
match (wrapping), Esc closes the bar (a second Esc closes the viewer).

## Code review (⇧⌘G)

A 460 px panel right of the panes listing the repository's changes. The
header shows the branch and total `+added −removed`, ⤢ maximizes and ↻
reloads. The panel is opaque (`bg`) even with a translucent window. The title
reads `CODE REVIEW`. An error notice (a failed action) shows as a red strip
under the header. With nothing to list the body says `Loading changes…`,
`Not a git repository` or `No changes — working tree clean`.

- **Staged** (HEAD → index) and **Changes** (index → working tree, untracked
  files included) sections, each collapsible and titled in uppercase with the
  file count (`STAGED  3`, `CHANGES  5`), with **Unstage all** / **Stage
  all**.
- Each file is a solid card (radius 6, hairline border): a `surface` header
  with status letter, name and folder, a `+a −d` pill, then icon buttons: ↗
  open in the viewer, ↺ revert (unstaged only) and + stage / − unstage.
  Click the header to expand its diff on a darker `bg_chrome` reading pane.
- The diff reads like Warp's: one column of new-side line numbers (removed
  lines have none), syntax-colored code, added lines on a green wash and
  removed lines on a red one with a 3 px bar at the card edge. A removed line
  paired with the added line it resembles (inserted lines between them are
  skipped) highlights the changed characters on a stronger wash.
- Long lines are not cut: a file's code scrolls sideways (trackpad, ⇧ +
  wheel, or ← → while maximized) as far as its widest shown line, under a
  fixed line-number gutter.
- Hovering a line of a hunk shows the hunk's icon-only toolbar over the gutter
  (Warp): `+` (tooltip "Stage hunk") and `↺` in the error color ("Discard the
  hunk") for unstaged hunks, or `−` ("Unstage hunk") for staged ones, then a
  copy icon ("Copy hunk": the hunk as a unified diff; toast "Hunk copied"). The
  accessibility tree lists every hunk's actions, hovered or not.
- Each hunk opens with a band that counts the **unmodified lines** hidden
  above it (⌃ above the first hunk, ⌃⌄ between hunks); a band under the last
  hunk counts the lines below (⌄). Clicking a band opens a gap of up to 40 lines
  whole, a longer one 20 lines at a time. With nothing hidden the band shows
  the `@@` header. Added, deleted, binary and over-1 MiB files have no
  bands. A binary file shows a note; a diff past 4 000 lines is cut with a
  note, and a hunk the cut runs through shows `Too long · use file actions`
  on its band and no toolbar.
- Revert asks first ("Revert this change?" / "Revert file?") and discards
  only unstaged changes; reverting an untracked file deletes it.
- A hunk action re-reads the diff first and checks the hunk's line ranges and
  content against the same diff it applies; if the hunk no longer matches,
  nothing is written and the panel shows why above the list.
- Stage, unstage and revert run in order on a background writer thread, so
  a slow repository never blocks input.
- The panel reloads (off the UI thread) when it opens, when a command
  finishes, after each action, and on ↻.
- **Maximized** (⤢): the panel covers the panes (their layout and shells stay
  as they are) below the tab bar; with at least 760 px a 300 px file list on
  the right shows each file's name, `staged` mark, folder and counts, and a
  click opens that file's card and scrolls to it. Keys go to the panel: Esc
  restores it, ↑ ↓ Page Up/Down Space Home End scroll, ← → scroll the file
  under the pointer sideways; app shortcuts still work and typing is
  swallowed. ↗ (open in the viewer) restores it too; ⇧⌘G closes the panel
  (it reopens beside the panes).

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
    (dim; collapsed blocks show an italic `N lines hidden` here instead; a
    block with an output filter active shows a warning-tinted pill `filter "q"
    · shown/total lines` instead)
  - right side: status badge pill (`✓ 0` green / `✕ N` red / amber `running`
    with a spinning ring /
    `—` unknown), duration (live elapsed while running), copy-output button.
    The duration is a right-aligned column (at least six cells), so pills
    and times line up from header to header; a pill is never taller than
    the row. A block restored from a saved transcript shows `restored · 1.2s`
    as its duration. Hovering the duration shows the full time (`Took 1 min
    18.432 s · finished 03:48:05 pm, October 11, 2026`; `Running for …`
    while it runs; `Took … · restored from a previous session` for a
    restored block). The copy button and the ⋮ menu button (three stacked
    dots, the block's context menu dropping down from it) show only while the
    pointer is on the header or the block is selected; their place stays
    reserved. After a copy the copy button is a green check for 1.5 s and a
    toast says "Copied output" ("Nothing to copy…" for a block without
    output). On the sticky header both buttons act on the block; only the
    rest of it scrolls back to the block
- **Left border**, 2 px, colored by state: accent = running, green = exit 0,
  red = exit ≠ 0, muted = no exit info. Consecutive blocks' borders are 2 px
  apart (design 01 block spacing).
- **Block wash**: only the selected block gets an accent tint + 3 px accent
  border, from its header (sticky header too) through its output; a selected
  failed block takes the error red instead. Unselected failed blocks are not
  tinted (exit state lives on the border and badge, design 01).
- **Selected block**: selected via a click on the header or the output, or ⌘↑/⌘↓; target of
  ⇧⌘C / ⌥⇧⌘C / ⌘I / ⌘B / ⌃M; Esc or clicking again deselects.
- **Sticky command header**: when a block's output scrolls but its header would
  leave the viewport, the header pins to the top edge; clicking it jumps to the
  block. Not over a command that is still running while the view follows its
  output: the program owns the whole PTY height and redraws its top row
  (vttest, progress UIs). Scrolled up, or once the command ends, it pins again.
- **Reverse screen** (DECSCNM, `CSI ? 5 h`, used by vttest and `tput flash`):
  the pane's default foreground and background trade places until the program
  resets it (or a terminal reset).
- **Bookmark dots** on the right edge (position proportional to scrollback
  position); click jumps to the bookmarked block.
- **Header overflow**: a long command is cut with `…` before the badge group;
  the cwd/branch context only shows when it fits. Multi-line commands show
  `⏎` between lines.
- **Links**: http(s) URLs and OSC 8 hyperlinks (`ls --hyperlink`, compilers,
  `delta`) draw in the accent color, as in Warp; under the pointer they turn
  lighter and underlined. A URL soft-wrapped onto the next rows is one link.
  Links work on the alternate screen too (Claude Code, Codex, vim). The
  pointer turns into a hand over a link, and resting on one shows a tooltip
  with its target and `⌘ Click`. A plain click shows that tooltip at once
  (`link_click = "prompt"`, Settings › Terminal › *Clicking a link: Show it*)
  or opens the link (`"open"`, *Open it*); a drag across it selects text. Over
  a program that tracks the mouse, a left click on a link is Rift's, not the
  program's (⇧ still selects).
  Hold ⌘ and the hand also shows over an existing file path (absolute, `~/…`,
  or relative to the pane's cwd; `file:line:col` suffixes are accepted) —
  ⌘-click opens any of them with the system opener. Only http, https, file,
  mailto and ftp targets open.
- **Cell styles**: bold, italic (embedded JetBrains Mono Italic/Bold Italic),
  dim, inverse, hidden, strikethrough, blinking (SGR 5: hidden and shown every
  600 ms), and underline variants (single, double, curly, dotted, dashed; SGR
  58 colored).
- **Block elements** (U+2580–U+259F: `█ ▀ ▄ ▌ ▐`, eighths, quadrants such as
  `▛ ▜ ▘ ▝`, shades `░ ▒ ▓`) are drawn as quads filling the whole cell and
  line height, not from the font, so block art (Claude Code's logo) has no
  gaps, as in Warp.
- **Line sizes**: double-width lines (DECDWL, `ESC # 6`) draw each character
  across two cells; double-height lines (DECDHL, `ESC # 3` and `ESC # 4`) draw
  the top and bottom halves of twice-size text. Double clicks and mouse reports
  map to the character drawn under the pointer.
- **Emoji presentation**: an emoji followed by VS16 (U+FE0F) draws from the
  color emoji font (Apple Color Emoji, Noto Color Emoji) across two cells; a
  character followed by VS15 (U+FE0E) draws its monochrome text glyph (Apple
  Symbols, DejaVu Sans) in one cell. Without a selector each emoji takes its
  Unicode default: ⌚ 😀 in color, ⏱ ✔ ☀ as text glyphs (from the terminal
  font, else Apple Symbols, STIX Two Math or Menlo; Noto Sans Symbols 2 or
  Noto Sans Math on Linux), as in Warp and iTerm2.

## Command editor (input zone)

App-owned line editor shown while the shell is at a prompt (raw keys pass
through to the PTY otherwise — vim, password prompts, shells without
integration). Sits at the bottom of the pane, or at the top with
`input_position = "top"`.

- **Prompt context** (above the input, like Warp): the `prompt_chips`, each
  a hairline chip (radius 4) with its Nerd Font glyph — by default a folder
  glyph and the cwd, then (in a repository) a git-branch glyph, the branch and
  the ahead/behind arrows and dirty dot, muted, then (on a dirty tree) a diff glyph and
  `files • +added −removed` in success/error colors; a click on it opens the
  code review panel, another closes it. Resting on a chip shows its hover
  card (300 px, `surface`): the chip's name, then what it shows and what a
  click does, wrapped in `text_dim`. Chip menus scroll by pixels
  (trackpad-smooth), the part-scrolled rows clipped to the list. Opt-in chips draw the same way: the PR chip (the
  GitHub mark and `PR #42` together in `warning` while open, else the
  state's color — `text_dim` draft, ANSI magenta merged, `error` closed — with a muted `draft` / `merged` /
  `closed`), Python (Python logo, `warning`), PHP (php logo, accent
  mixed half with ANSI magenta), Docker (whale, accent; project name,
  then `running/total` in `success` or a muted `down` / `off`), and the User, Host, Date and Time
  chips (12-hour `03:48 pm`, 24-hour `15:48`), which show one value and do
  nothing on click. The cwd,
  branch, PHP, Python and Docker chips open a 360 px searchable menu above
  the chip (layer 5): rows with an icon, the label, a muted detail (a
  version's source) and a `success` check on the current one. The Docker
  menu is a 520 px table under a muted `Container · Ports · Uptime` header
  (11.5 px medium), each column as wide as its widest cell: service name
  (dim when stopped), published ports (`3306→3306`, comma-separated, `/udp`
  only for UDP, `—` when none, `text_dim`), uptime (`5 hours`, or the whole
  status when stopped, muted) then a dot and `healthy` / `starting` /
  `unhealthy` in `success` / `warning` / `error` for services with a health
  check. An empty list says why (no installs found, Docker not
  running, `docker compose up`). The search field reads `Search directories…`,
  `Search branches…`, `Search versions…` (PHP, Python) or `Search
  containers…`. A PHP or Python row runs its `rift-use …` command in the
  pane; a Docker row (no check mark) runs `docker compose --project-directory
  <dir> logs --follow --tail 200 <service>`. Chips with nothing to show are left out. A
  narrow pane wraps the chips that do not fit onto more rows (Warp), and the
  input moves down with them; only a cwd wider than a whole row is cut, keeping
  its tail. In every pane of a split; hidden in shell-prompt mode.
- **Edit prompt** (Warp; layer 6, 600 px card over a 40 % black backdrop,
  `surface`, radius 8): `Rift prompt` with `Restore default`, a palette of
  the chips not in the prompt (sample values; the Python, PHP and Docker
  chips read `python`, `php`, `docker`), then the prompt itself in a
  dashed box, each chip with ×. Every chip there has the same hover card
  as in the prompt (none while one is dragged). A click on a palette chip appends it;
  dragging a chip (from the palette or the box) shows an accent insertion
  bar and drops it there, and a box chip dropped outside the box is
  removed. An empty palette says `Every chip is in your prompt.`, an empty
  box `Drag chips here, or click one above`. `Cancel` (Esc) and `Save changes` (Enter; enabled once something
  changed) close it. Screen readers get one button per chip and action.
- **Chip pickers** (Warp): the cwd chip ("Change directory") and the branch
  chip ("Change git branch") open a 360 px searchable list over the chip
  (under it when there is no room above; layer 5, `surface`, radius 8). The
  directory list holds `.. (Parent Directory)` and the sub-directories (a
  query starting with `.` shows hidden ones); the branch list holds the local
  branches, the checked-out one first with a ✓, and for a new name `Create
  branch <name>`. Typing filters (case-insensitive), ↑↓ / Page Up/Down
  select, Enter or a click runs `cd ./<dir>`, `cd ..`, `git checkout
  <branch>` or `git checkout -b <name>` as a command in the pane (its block
  shows the result); Esc, a right click or a click outside closes it. A pane
  running a command gets a toast instead.
- **Prompt glyph**: the logo's chevron, drawn (not a font glyph) — the logo's blue (`accent`)
  normally, red after a non-zero exit, muted in an unfocused split pane. In
  shell-prompt mode (`prompt = "shell"`) the shell's own PS1 stays on the grid
  above and the editor shows a smaller muted chevron instead.
- **Live syntax highlighting**: the command in `syntax.command`, flags in `syntax.flag`, strings in `syntax.string`, operators muted, other arguments in `fg`; block headers color the command line the same way. A command is known when it is an alias, function or builtin of your shell or an executable on its `$PATH` (as your rc files set it).
- **Unknown commands** (`Underline unknown commands`): a first command word the
  shell does not know gets a red zig-zag underline, except the word still being
  typed at the end of the line.
- **Corrections** (`Suggest corrections for failed commands`): after a failed
  command the empty editor shows `Did you mean`, the corrected command in a
  hint chip and `→ accept`.
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
- **Cursor**: `cursor_style` = block (8×17 px), bar (2 px) or underline
  (2 px), 1.1 s blink; exact shaped-text positioning, translucent when a glyph
  sits under it. Vim NORMAL always draws a solid, non-blinking block.
- **Vim mode pill** at the right edge of the input line when `vim_mode` is on:
  `NORMAL` (warning color) or `INSERT` (muted).
- **Ghost text**: inline dimmed completion preview continuing the line at
  the cursor (Warp), followed by a small `→` key chip; → at the end of the
  line or Tab accepts it all, ⌃→ one word at a time.
- **Completion panel**: git subcommands with descriptions, path completions
  (with `~/` expansion), then — when there are word candidates — the
  history lines that start with what you typed (alone they stay ghost text).
  Docked to the editor across the full pane width (Warp's completion pane):
  directly above the editor when it sits at the bottom, below it with
  `input_position = "top"`. The output moves out from under it, as in Warp:
  the newest lines stay in view right above it (below it with the input at
  the top), and the program keeps its rows. The header holds source
  tabs with counts (`ALL 7 · PATH 4 · HISTORY 3`; just the source when there
  is one): ⇧⇥ or a click switches the source. Rows show an icon (folder,
  file, history, branch, terminal), the name with the typed part in the
  accent, and the description at the right; a click accepts a row (a history
  row replaces the whole line). The footer shows `↑ ↓ navigate`, `⇥ accept`,
  `→ accept ghost`, `esc dismiss`. Square corners, hairline edge, up to 8
  rows.
- **Welcome list**: a fresh session (no command run yet, single pane with
  room) shows *New session* with the shell, directory and branch, then five
  rows with their shortcuts: Split pane, Command palette, Command history,
  File explorer, Settings. Clicking a row does it; the list goes away after
  the first command.
- **Boot placeholder**: `Starting zsh…` (shell name from config) shown on the
  chip row of the input zone (bottom, or top with `input_position = "top"`;
  the input line in shell-prompt mode), as in Warp, under the input divider
  already in place, until the shell's
  first prompt, with the terminal cursor hidden. A shell without integration
  never marks a prompt; the placeholder leaves and its cursor shows 1.5 s after
  its first byte.

## Split panes

- Vertical / horizontal splits, arbitrary nesting (binary tree).
- Panes are flush and square-cornered, separated by a 1 px divider (Warp
  style).
- Active pane: its title bar carries a 2 px accent top edge; inactive panes
  recede under a dim.
- Each pane has a 28 px title bar (Warp pane header): a terminal icon, the
  program's title (OSC 0/2), else the pane's directory, centered (after a
  spinner while a command runs); while the
  pane is focused or hovered, split (a new pane to its right), ⋮ (the pane
  menu) and × (close) show at the right. Clicking the bar focuses the pane.
- Pane menu (⋮, or right-click in the pane): Copy, Paste, Select all, Split
  pane right / down, Maximize pane and Move to new tab (split tabs), Clear
  blocks, Close pane.
- Drag a title bar to move the pane: past 4 px it fades in place and a chip
  with its title follows the pointer. Over another pane, the half nearest the
  pointer lights up with what the drop does (`Split right` …) and the pane
  lands on that side; the middle third of a pane of the same tab says `Swap`
  and the two trade places. Over another tab the tab lights up: release to
  join it (split right of its focused pane), or hold 600 ms and the tab
  opens so the pane can land beside any of its panes. Over the empty tab
  strip (or the vertical rail) a slot reads `New tab`. Esc stops the drag
  and leaves the pane where it was (back on its own tab if a hovered tab
  opened). The viewer pane's header drags the same
  way (into another tab, not a tab of its own). A tab left without shells
  closes.
- Drag a divider (grabbable 4 px either side) to resize; ⌃⌘arrows resize from
  the keyboard (±5% per press).
- Zoom (⇧⌘⏎): the active pane temporarily takes the whole tab rect and renders
  like a single pane. Navigating or splitting unzooms.
- Every pane keeps the full editor with its prompt context row.
- The tab shows its pane count chip.

## History pane (⌃R)

⌃R in the command line opens the history pane, docked above the command line
across the pane like the completion panel; the output moves up out from
under it, so the newest lines stay in view. The command line is its search
field: what you type narrows the rows (any case, anywhere in the command).

- Header: `HISTORY`, scope chips **This pane** · **All shells** · **Failed**
  (⌃R again or a click switches), `N of M`, and ×.
- Rows, newest first, one per distinct command: a status dot (green exit 0,
  red failed, grey unknown), the command with the match in the accent, the
  directory it ran in, and how long ago (`Just now`, `4 min ago`,
  `yesterday`; imported lines say `imported`). Sources: this window's
  finished commands, the pane's own typed history, then the shell's history
  file (zsh, bash or fish).
- Footer hints: `↑↓ navigate · ⏎ run · ⇥ edit first · ⌘⏎ run in new split · ⌃R
  scope · esc close`. An empty list says `No commands yet`, or `No matching
  commands` when a query filters everything out.
- Keys: ↑ ↓ (PageUp/PageDown) move, ⏎ runs, ⇥ puts it on the command line to
  edit first, ⌘⏎ runs it in a new split beside the pane, Esc closes. A click
  puts a row on the command line; a double-click runs it.
- ⏎ with no matching row runs the typed line; ⇥ with none closes the pane
  (completions show). A click outside it, focusing another pane, the viewer,
  the palette, find or Settings close it, and so does a program taking over
  the pane.

## Command palette (⌘P)

Fuzzy-find modal (nucleo matcher), 640 px, radius 8, over a dimmed window:
the query with a magnifier, the result count (`N results`, left of the `esc`
cap, hidden when the query leaves no room) and `esc`, mode chips (**Actions** `>` ·
**History** `#` · **Sessions** `@`), grouped rows with
icons, match highlighting and shortcut key caps, and a footer `↑ ↓ navigate
· ⏎ run · ⇥ next mode`. Typing a mode's prefix into an empty query, ⇥, or a
click on a chip switches mode. Modes:

- **Actions** — every app action (blocks, scrolling, tabs, panes, font,
  search, history and settings) with the resolved custom shortcut;
  keyboard selection scrolls through long result sets. Also includes `Toggle
  vim keybindings`, one
  `Launch configuration: <name>` row per file in
  `~/.config/rift/launch_configs/` (re-scanned each time the palette opens),
  `Theme: <name>` rows, plus recent commands.
- **History** — imported shell history (zsh, bash or fish, from the
  configured shell's history file) + per-pane local history; Enter runs, or
  insert into the editor. (⌃R in the command line opens the history pane.)
- **Session switcher** (⇧⌘O) — tabs, panes of split tabs (directory and
  running job), tabs of other Rift windows, detached session-service shells,
  and the 40 newest finished blocks. Enter jumps there.

- **Send output to command…** — a fourth mode, opened from the block menu or
  the action `Send block output to a command`, with no mode chips. The
  placeholder reads `Command that reads the output on stdin…`; an
  `OUTPUT (STDIN)` preview lists the selected block's lines under the row that
  says what will run, and the footer reads `⏎ run with output on stdin`.

Other Actions rows: `SSH: <name>` and `Install Rift shell integration on
<name>` per SSH profile, `Toggle vertical tabs`, `Clear saved block
transcripts` and `Notifications: finished commands and alerts`. A query with
no match shows a `No matches` band.

Long rows are cut with `…` inside the dialog.

## Notifications

**Inbox popover** — dropped from the inbox button, or ⇧⌘A, 420 px wide and
at most 620 px tall.

- Header `Notifications`, `Mark all as read` (while anything is unread) and
  ✕; chips `All tabs (N)`, `Unread (N)` and `Errors (N)` filter the list
  (Tab / ⇧Tab cycle them).
- Rows, newest first: a badge — the agent's (the Claude mark on orange for
  Claude Code, Codex `>_`, OpenCode `oc`, Gemini `G`) with a kind dot, or a
  tinted kind disc (✓ finished, ✕ failed, `!` waiting on you, a bell for a
  program notification, an outlined bell for a terminal bell) — an accent
  dot while unread, a meta line
  with the agent name, a Nerd Font branch glyph and the branch, and the age,
  the title (bold, wrapped, at most four lines, then `…`; `(empty command)`
  when it is blank), its detail (`Completed in 1.2s.`, `Failed with exit 1
  after 3s.`, `Waiting for your input.`, a notification body; wrapped to two
  lines in the popover, one in a toast). The age (`Just now`,
  `4 minutes ago`) sits right-aligned on the meta line, or on the title's
  first line when there is no agent or branch; a long title wraps clear of
  it.
- Click a row, or ↑/↓ then ⏎, to jump to its pane and block (marks it
  read). The wheel scrolls the list. Esc, ✕, a right-click, a click outside
  or a plain key closes it.
- An entry turns read when you open it, when you look at its pane, or with
  `Mark all as read`.
- `No notifications`, `Nothing unread` or `No errors` when the list is
  empty.

**Toasts** — while Rift is focused, an alert from a pane you are not
looking at appears as a 360 px card under the inbox button: the badge,
centred on the card's height, then `agent · branch`, the title (two lines)
and detail.
At most two show, newest on top. Each stays `toast_secs` (default 8);
hovering keeps it. A click jumps to the pane; ✕ dismisses. Toasts hide while
the popover or Settings is open.

**Tab markers** — a tab with unread entries shows a dot in the newest one's
kind color (green finished, red failed, yellow waiting, accent notification,
grey `text_dim` bell) in place of the activity dot.

## Hint mode (⇧⌘H)

Every visible URL, existing file path and block header gets a short letter
label drawn over it. Typing a label opens the link or path, or selects the
block; ⇧ + label copies it (a block copies its output). Esc cancels.

## Dialogs and toasts

- **Confirm dialogs** — a 460 px card (radius 12): title, description and
  note, the affected lines in an inset list (at most three, then `and N
  more…`), and a footer bar with Cancel `esc` and a confirm button `⏎` (red
  for discarding work). Running processes list one row each with the
  program's badge (a coding agent's brand disc, such as the Claude mark,
  else a terminal mark), the command, and the agent's name; pasted lines
  and files are plain code lines. Used for closing
  running commands, pasting multi-line text into a shell without bracketed
  paste ("Paste and run?"), installing shell integration on an SSH host, and
  updating ("Update to Rift vX.Y.Z?": what the restart keeps — tabs,
  running shells, windows that cannot be reopened — with *Later* `esc` and
  *Update* `⏎`, or *Open download page* for an install that cannot update
  itself; Later hides the chip for that version).
  Enter confirms, Esc cancels. An Enter that is auto-repeated, or that
  arrives within 300 ms of the dialog opening (a held or typed-ahead key),
  is ignored.
- **Toasts** — short notices at the bottom (a keybinding moved from another
  action, the session service could not start, config.toml was not applied,
  a copy, paste, open or save failed, …).
- **Tooltips** — resting the pointer on a control for 0.6 s shows what it
  does and its shortcut from the live keymap: the tab bar's +, ✕, explorer,
  review and inbox buttons, a block's chevron and copy button, a split
  pane's split, ⋮ and ✕, the sticky header ("Scroll to this command"), a
  block's duration (the full time), the history pane's scope chips
  and ✕, Settings' ↺, "Edit prompt" and "Open config.toml" (with its path),
  the explorer's and review panel's buttons, the file viewer's ✕ and ⋮, and
  on Linux the drawn traffic lights ("Close window", "Minimize", "Zoom"). A
  prompt chip shows its hover card. A tab shows its full
  title and directory, an explorer row its full path. Moving straight to the
  next control shows its tooltip at once; a click, key or scroll hides it
  until the pointer moves to another control. No tooltips while a menu,
  dialog, palette, What's New, a tab rename or a drag is open.
- **What's New** — release notes from `CHANGELOG.md`, newest first: title
  with the current version and date, then each release's summary and its
  New / Improved / Fixed items (section titles in accent / success / warning).
  The notes' italics (`*Relay*`) show as plain text.
  Opens from Help → What's New, the palette (`whats_new`) or Settings ›
  General › Updates › What's New, and once on the
  first launch after an update (never on a first install). The body scrolls
  with the wheel or trackpad, arrows, Space, Page Up/Down and Home/End; Esc,
  Enter, ✕, **Got it** or a click outside the card closes it.

## Search (⌃⇧F / ⌘F)

Bar overlay; live yellow highlight of all matches, orange for the current one.
Enter/⇧Enter or ⌘G/⇧⌘G cycle matches (viewport jumps to each), Esc closes.
Two toggle chips at the right: `Aa` match case (⌥C) and `.*` regular
expression (⌥R); an invalid regex shows `invalid regex` in red instead of the
counter. The counter shows the current match's position among all matches
(`n/total`; `search` while the query is empty, `0 results` when nothing
matches); only the newest 500 matches are kept for navigation (older
ones are counted, not visited), so with 600 matches it runs 101/600 …
600/600. Text is matched as printed: wide glyphs (日本) and combining marks
(a decomposed `é`, e + U+0301) are found, a typed `é` finds both spellings
(text and query are compared canonically decomposed; so does the block
filter), and a match highlights whole cells; hidden (SGR 8) text is not searchable. A query longer
than the field scrolls so its end stays visible. With the file viewer open,
⌘F finds in the file instead (see File viewer).

With a block filter active (⌥⇧F), the same bar edits the filter: only that
block's output lines containing the text stay visible. The bar is 340 px wide
(400 px in find mode, which adds the toggle chips), labelled `filter`, with
the hint `Enter keep · Esc clear` in place of the counter.

## Block context menu (right-click or ⌃M)

Copy command · Copy output · Copy both · Copy working directory ·
Copy git branch · Re-run command · Filter output…/Clear output filter · Send
output to command… · Bookmark/Remove bookmark · Collapse/Expand · Scroll to
top of block · Scroll to bottom of block.
Every menu (block, tab, pane, file viewer) is a 240 px list with an icon
column and the row's shortcut from the live keymap at the right. Only Close
pane and the three Close tab rows are red. Disabled items are dimmed (e.g.
Copy output on an output-less block). In a short window the rows shrink
(down to half height) so every item stays reachable.

**Tab menu** (right-click a tab): Rename tab · Duplicate tab · Synchronize
inputs in panes / Stop synchronizing inputs (disabled for a tab with one
pane) · color swatches Red, Green, Yellow, Blue, Magenta, Cyan · No color ·
Close tab · Close other tabs · Close tabs to the right.

**File viewer menu** (⋮): Insert path · Copy path · Find in file · Show
raw / Show rendered (Markdown files only) · Close.

## Settings tab (⌘,)

Settings opens as a tab after the shell tabs (settings icon); the shells keep
running and the tab strip stays live. ⌘, again goes back to the shell tab;
clicking any tab switches; the tab's ×, ⌘W or Esc closes it. Other app
shortcuts (new tab, next tab, palette…) work from it and leave it.

A 232 px sidebar: a search field, the nine pages with icons (arrow keys
navigate pages; PageUp/PageDown scroll the body) and *Open config.toml* at
the foot (opens the file beside the page, in the Settings tab; ✕ closes it; for 3 s after config.toml is
written or reloaded, a green check sits at its end, a green dot on its icon
in the icon-only sidebar). When the settings area is
under 700 px wide the sidebar shrinks to 56 px of icons, with no search
field or labels. The body (at most 720 px wide, centred right of the sidebar like Warp's)
has the page title and description, group labels, and flat rows: label (and
description) left, control right; it runs to the bottom edge.

- **Search**: typing anywhere outside Keybindings fills the search field and
  lists the matching settings of General, Appearance, Input, Terminal, Files
  and Notifications (by name or description; not Keybindings or Themes), each
  with its `Page › Group`, under the title `Results for “q”` and the subtitle
  `Settings on every page whose name or description match.`; no match shows
  `No settings match`. ⌫ edits, Esc clears it (the next Esc closes the tab).
- **Changed settings**: a setting that differs from its default shows an
  accent dot after its name and ↺ left of its control, which resets it.
- **Selects** open a list (check on the current value, theme swatches); ↑ ↓
  ⏎ pick, Esc or a click outside closes it. A list shows at most 12 rows and
  scrolls, with an `a–b of n` note at the top right.
- Keyboard: Tab / ⇧Tab move a focus ring over the page's controls (the
  page scrolls to bring a control below or above the fold into view; Open
  config.toml comes last), Space or Enter presses the focused control (a
  pressed ↺ hands the focus to its row's control), ← / → adjust a focused slider, and Esc
  drops the focus. The arrow keys change page unless a slider is focused;
  Home / End jump to the first / last page. Every control writes that file
  live (the hot-reload watcher applies changes < 300 ms, including external
  edits); while config.toml is invalid, controls do not save and a toast says
  to fix it first; the wheel scrolls the page body (24 px per line, clamped
  to the content) and switching pages resets the scroll. Clicks elsewhere in
  the page are swallowed rather than reaching the terminal.

- **General** — *Shell and session behavior*.
  - Shell: `Shell path` (read-only: `shell` override, else `$SHELL`),
    `Scrollback` stepper (lines, ±1000 within 1,000–1,000,000, shown as
    `10,000`; described *Maximum lines kept in the buffer*).
  - Session: `Restore tabs after a crash` toggle, `New tabs start in` segmented
    `Previous | Home | Custom` (a custom path is set in config.toml),
    `Warn before closing running commands` toggle,
    `Save block output for restore` toggle (*Output can contain secrets;
    turning this off deletes it*; `save_transcripts`),
    `Keep shells running through updates` toggle (*Session service;
    updates and crashes reattach (new shells)*; `durable_sessions`).
  - Updates: `Check for updates` toggle (*At launch and every 6 hours;
    Update Rift appears in the tab bar*; `check_for_updates`), then a
    `Rift <version>` row whose description is the update status (`Not
    checked yet`, `Checking for updates…`, `Up to date`, `vX.Y.Z available`
    or `Couldn't check for updates`) and a `Check now` button (one check at
    once, disabled while one runs; it works with the toggle off too), then a
    `What's New` row (*The changes in each version of Rift*) whose `Open`
    button opens What's New.
  - Automation: `Control socket` segmented `Off | Navigate | Full`
    (*Lets scripts control Rift (rift-terminal ctl); Full also lets them
    type*; `control_api`).
- **Appearance** — *Fonts, cursor, window and theme*.
  - Font: `Font family` select (JetBrains Mono first, then the embedded
    Hack and the installed fixed-width families, each shown in itself),
    `Font size` stepper (px, 8–28), `Line height` slider (1.0–2.2, default
    1.2).
  - Cursor: `Cursor style` segmented `Block | Bar | Underline`,
    `Cursor blink` toggle (*Pulse the cursor when the editor is focused*).
  - Window & theme: `Active theme` select (background swatch ringed in the
    theme's accent + display name), `Sync with OS appearance` toggle,
    `Light theme (OS sync)` select (same swatch style), `Window opacity`
    slider (1–100 %, value boxed right of the track), `Window blur radius`
    slider (1–64 px, macOS only),
    `Window padding` stepper (px, ±2), `Font ligatures` toggle (*Render -> >=
    != as combined glyphs*), `Tabs` segmented `Top | Side` (`tab_layout`).
    Font size and line height show the applied (clamped) value. A theme file
    that does not load is listed as `<name> (invalid theme file)`, choosing it
    toasts why and Dark Default is used; themes sharing a display name carry
    their file name.
- **Input** — *The command line and blocks*.
  - Input: `Vim keybindings` toggle, `Autosuggestions (ghost text)` toggle,
    `Completions menu` toggle, `Autocomplete quotes and brackets` toggle,
    `Input position` segmented `Bottom | Top`, `Blocks start` segmented
    `Bottom | Top`, `Prompt` segmented
    `Rift | Shell (PS1)`, `Prompt chips` (a preview of the chips with
    sample values; a click opens **Edit prompt**), `Expand aliases as you type`, `Underline unknown
    commands`, `Suggest corrections for failed commands` toggles.
- **Terminal** (clipboard icon) — *Mouse, clipboard and keys inside the
  terminal*.
  - Mouse: `Mouse reporting to programs (Shift selects)` toggle,
    `Middle-click to paste` toggle (*Pastes the clipboard into the pane under
    the pointer*; `middle_click_paste`), `Clicking a link` segmented
    `Show it | Open it` (*Show where it goes first, or open it at once
    (⌘-click always opens)*; `link_click`), `Lines per wheel notch` stepper
    (1–20, ±1; `wheel_scroll_lines`).
  - Clipboard: `Copy on select` toggle, `Ask before pasting lines that would
    run` toggle, `Programs' clipboard access (OSC 52)` segmented
    `Off | Copy | Copy and read` (*Read lets any program or ssh host see your
    clipboard*; `clipboard_access`).
  - Keys (macOS only): `Left Option key is Meta` (`option_as_meta`) and
    `Right Option key is Meta` (`right_option_as_meta`; shows the left one's
    value until set) toggles. Turning the left one changes only the left key.
- **Files** (file icon) — *Where files open, and the file explorer*.
  - Editor: `Open file links in` select (`link_editor`: ⌘-clicked paths,
    file links and hint-mode paths) and `Open files from the explorer and
    code review in` select (`file_editor`). Each lists `System default`,
    `Rift viewer`, then the editors installed here (VS Code, Cursor,
    Windsurf, Zed, Sublime Text, IntelliJ IDEA, PhpStorm, WebStorm,
    RustRover). A folder never opens in the viewer (it goes to the system
    opener); an editor that is no longer installed falls back to the viewer
    (or the system opener for a folder) with a toast.
  - File explorer: `Show hidden files` toggle (dotfiles; `.git`, `.DS_Store`
    and git-ignored entries stay hidden).
- **Notifications** (bell icon) — *Toasts, desktop notifications and sounds;
  everything also lands in the inbox*.
  - Alerts: `Notifications` master toggle, `In-window toasts` toggle,
    `Toast duration` stepper (s, 2–60).
  - Notify me when: `A long command finishes` toggle, `Long command
    threshold` stepper (s, ±5 within 0–3600), `A command fails`, `An agent finishes its
    task`, `A program or agent needs attention`, `A password prompt appears`
    toggles.
  - Sounds: `Play notification sounds` toggle; `Sound`, `Finished`, `Failed`,
    `Needs attention` selects — the list holds `Default`, `None`, then every
    sound file found, and picking one plays it; the per-event ones start at
    `Same as Sound`.
  - Terminal bell: `Bell` segmented `Visual | Audible | Both | None`.
- **Keybindings** — *Click a shortcut to record a new one · type to filter*:
  a filter row (placeholder `Type to filter actions or shortcuts`; `No matching
  actions` when nothing matches), then every bindable action (including ones that ship unbound)
  with its first chord as chips; overridden rows get accent-edged chips and a
  ↺ reset button, unbound rows say `unbound`. Typing filters by action name or
  shortcut (⌫ edits, Esc clears). Clicking a row records: the next chord with
  ⌘/⌃/⌥ (or a function/navigation key) becomes the shortcut, Esc cancels,
  ⌫ unbinds; a chord taken by another action moves over with a toast naming
  it. While recording, the row reads `Press a shortcut · Esc cancels · ⌫
  unbinds`. ⌃⌘K jumps straight here.
- **Themes** — *Built-in themes plus `~/.config/rift/themes` (TOML or Warp
  YAML)*: a row per available theme with four swatches (background, accent,
  success, error), its display name and an `active` tag on the current theme;
  the `light` tag marks the `theme_light` theme, and only while
  `sync_os_theme` is on. Clicking a row sets `theme`.
- **Remote access** (open icon) — *Watch and run commands from a paired phone, at home or away.*:
  - `Allow remote access` switch (`remote_access`) and a status line: `Off`,
    `Starting…`, `Listening on <addresses>`, or `N device(s) connected`.
  - `Pair a phone` (primary button; disabled while off) and a `View only`
    switch (changing it clears a shown link). Pairing shows the QR code
    (black 4 px modules on a white plate with a 4-module quiet zone, so phone
    cameras read it in any theme), `Expires in m:ss`, the wrapped link and a
    `Copy link` button; at zero, `Expired — Pair again`. While the gateway
    has no address yet, a `Pairing` row waits for it.
  - `Phones can open tabs while Rift is closed` (with remote access on, last):
    the `remote_when_closed` switch (*Only phones that may run commands; you
    get a notification, and the tabs appear when you open Rift*, or *Needs
    Keep shells running through updates (General)* while `durable_sessions`
    is off).
  - `Use a Cloudflare tunnel instead` (with remote access on, below `Relay`): the `remote_tunnel`
    switch; on, a checklist follows: `cloudflared` (`Installed · <path>` with
    a green check, or `Not installed` with an `Install` button when
    Homebrew is present, else `Download`) and `Tunnel` (`Starting…`,
    `Waiting for cloudflared`, `Ready · <host> · pair phones again after a
    restart` or `Failed, retrying: <reason>`). The `Relay` row then shows
    the tunnel relay's state (`Waiting for the tunnel` until it is ready)
    without the URL field, and `Pairing` waits with `Waiting for the
    tunnel…`.
  - `Relay`: a text field for `remote_relay` (click or Enter edits; typing
    and ⌘V fill it; Enter saves a valid `ws://`/`wss://` URL, an invalid one
    stays with a toast; Esc cancels; Tab or a click elsewhere saves) and its
    state: `Off · phones connect only on this network`, `Connecting…`,
    `Connected via <host>` or the error. It holds Rift's public relay by
    default.
  - `Paired devices`: per device its name and `platform (iOS, Android) · paired <date> ·
    connected|connected via relay|last seen …`, an `Operate` / `View only`
    segmented control (raising to Operate asks first), and a destructive
    `Revoke` button, which opens the confirm dialog `Revoke <name>? It
    disconnects now and cannot reconnect without pairing again.`; an empty
    state when none.
  - `Recent connections`: the newest 20, each with the device (or `Unknown
    device`), `LAN` or `Relay`, the address, the start time, the duration or
    `connected`, and why it ended.
  - The page refreshes its data at most once a second.
- **About** — no title: the app icon (64 px) beside the `Rift` wordmark,
  centred a little above the middle of the body; under it the version
  (`v0.1.0`) with a copy button (tooltip *Copy version*; copies the version
  and toasts `Copied version`), the update status — `Up to date`,
  `Checking for updates…` or `Couldn't check for updates` (muted), or
  `vX.Y.Z available` (`warning`) with an `Update…` button that opens the
  update dialog, or else a `Check now` button (none while a check runs) —
  and `Copyright 2026 botjaeger · MIT License`. Before any check, only
  `Check now` shows.
  Opens from Rift → About Rift and the palette (`open_about`).

## Scrollback

- Trackpad/wheel scrolling with fractional-delta accumulation (no dropped
  ticks). The wheel scrolls the pane under the pointer; programs that track
  the mouse get wheel reports, other full-screen apps get arrow keys
  (alternate-scroll mode, app-cursor aware).
- PageUp/PageDown page, Home/End jump to top/bottom (at the prompt; keys pass
  through to full-screen apps).
- `clear` performs a Warp-style soft clear: the viewport empties, old blocks
  stay reachable by scrolling up, and the next block starts right above the
  command line (at the top of the viewport with `blocks_start = "top"`). ⌘K is the destructive variant (blocks +
  scrollback wiped).
