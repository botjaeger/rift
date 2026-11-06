# UI Surfaces

Every visual element in Rift, top to bottom. Layer order for compositing:
0 chrome → 1 terminal + editor → 2 completion, history, search, file viewer →
3 settings page, maximized review → 4 palette, settings select lists →
5 context and chip menus, hints, notifications popover → 6 toasts, confirm
dialogs, What's New, Edit prompt, Edit toolbar → 7 tooltips (and the item
dragged in Edit prompt or Edit toolbar). Visual
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
  when `restore_session` is on; without a saved frame it opens
  `window_width` × `window_height`, and `launch_mode` opens it maximized or
  full screen. A named window ("Rename window") has its name as its title;
  `always_on_top` and *Toggle always on top* float it above other apps.
  With `request_attention`, a bell or an alert (finished, failed, waiting)
  from a pane while no Rift window has focus bounces the Dock icon once
  (Linux: the window's urgency hint).
- **Gradient themes**: a Warp theme whose `background` is a gradient draws
  it behind the panes of an opaque window (a translucent window or a
  background image keeps the solid ground); a gradient `accent` runs along
  the active tab's top edge.
- **Resize overlay** (`resize_overlay`): while the window is resized, the
  grid's `columns × rows` sits in the window's centre on a tooltip's plate
  (12 px label, `surface` fill, hairline and shadow), gone a second after
  the last change. `snap_to_cells` makes the window resize by whole cells.
- Mouse pointer: a hand over anything clickable (tabs, buttons, dialog
  buttons, menu and palette rows, settings controls, ⌘-hovered links),
  resize arrows over split handles, the I-beam over terminal text.
- Hover highlights (tabs, welcome rows, pane controls, settings rows) repaint
  the moment the pointer crosses into or out of a control, not on the next
  frame something else draws.
- While a modal dialog is open (the command palette, a confirm dialog,
  What's New, the block comparison, Edit prompt, Edit toolbar), nothing under it reacts to
  the pointer: no hover highlight, tooltip or hand cursor. Only the
  dialog's own controls respond.
- **Drop-down hotkey window** (`hotkey_window`): the global hotkey slides a
  borderless window in from the `hotkey_window_edge` (top by default) of the
  pointer's screen (or `hotkey_window_screen`), `hotkey_window_width`
  percent along that edge (full by default, centred) and
  `hotkey_window_height` percent deep, floating above other apps
  (every Space and over full-screen apps on macOS). It keeps the tab strip,
  with the space the window buttons would take left empty (no buttons; the
  strip neither drags nor zooms it). It slides up when the hotkey is pressed again
  or, with `hotkey_window_hide_on_blur`, when it loses focus. Settings ›
  General › Hotkey window has its settings, the hotkey itself
  (`global_hotkey`) among them. The session switcher (⇧⌘O) lists its
  tabs as "… — hotkey window". See [features](features.md#global-hotkey).

## Menu bar (macOS)

Native menus: **Rift** (About Rift — Settings › About, Check for Updates…, Settings…, Keyboard Shortcuts, Services,
Hide/Hide Others/Show All, Quit) · **File** (New Tab, New Window, Split Pane
Right/Down, Reopen Closed Tab, Save Layout as Launch Configuration, Close
Pane/Other Tabs/Window) · **Edit** (Undo, Redo, Cut, Copy, Paste, Select All,
Find…, Filter Block Output…, Clear Blocks) · **View** (Command Palette,
Command History, File Explorer, Code Review, Bigger/Smaller/Actual
Size, Maximize Pane, Equalize Panes, Toggle Full Screen) · **Blocks** (select/copy/reinput/bookmark/scroll actions, Block Actions…) ·
**Window** (Minimize, Zoom, Rename Window…, Keep Window on Top, tab and
pane navigation, Synchronize Inputs, Bring All to Front) · **Help** (What's New, Keyboard Shortcuts).

Menu items act on the window that last had focus. New Window opens another
window of the same process, offset from the current one; Close Window closes
only it while others are open ("Close window?" when commands run in it) and
quits with the last one; Quit asks "Quit Rift?" for every window at once
(features.md, Durable sessions).

Shortcut labels come from the live keymap (`[keybindings]` overrides show up
after the reload); only ⌘ chords become menu key equivalents so ctrl chords
stay with the terminal. Choosing an item replays its chord through the same
key routing as the keyboard (overlays, editor vs raw mode). Quit goes through
the running-command warning. Linux has no native menu bar (the palette lists
every action).

**Menu bar icon** (macOS, while Remote access is on, which is off by default):
the app icon as a line
drawing (tile outline, chevron and cursor; a template image) in the menu
bar's status area, tooltip and
VoiceOver name *Rift*. Its menu while Rift runs: *New Window*, *Settings…*
(both bring Rift to the front), separator, *Quit Rift Completely* (the quit
dialog as for Quit; then the phone gateway and the shells phones opened stop
too). Once no window is open, the phone gateway shows the same icon while it
runs, with *Open Rift* and *Quit Rift Completely*. Turning Remote access off
removes both within a moment.

## Tab bar

- One tab per shell session group. Its **title** is the name it was given
  (*Rename tab*), else the folder of its focused pane (`~` for home; before
  any directory is known, the program's OSC 0/2 title): two tabs on folders
  of the same name both show `parent/name` (`rift/src`, `flow/src`); a
  renamed tab does not count. The title follows the focused pane of a split
  tab, as code review does. A program's own title (an agent's session name,
  `vim main.rs`) is never the tab's: it is in the tab's tooltip, without an
  agent's status glyph (`◑`, `✳`), which would repeat the tab's status. As
  in Warp, the strip is 34 px tall (44 px under the detailed style) and
  every tab 200 px wide (a longer title is cut short); tabs that do not fit
  shrink evenly to 120 px, then scroll.
- **One status per tab**, the most urgent of its panes: *needs you* (an
  agent waits for your input or permission, or a password prompt out of
  view), *failed* (a command or agent failed out of view, not yet seen),
  *working* (a command runs, or an agent works), *done* (one finished out of
  view, not yet seen), *new output* (output, a bell or a notification since
  the tab was last viewed; a fresh shell's startup before its first command
  does not count). A tie goes to the focused pane. Viewing a pane
  clears its done, failed and new output; needs-you stays until answered. A
  renamed tab keeps its status. `tab_style` (Settings › Appearance › Tabs ›
  *Tab style*, three picture cards) says how:
  - **Dot** (default): a 22 px slot at the tab's left end holds the running
    spinner (accent) or an 8 px dot — `warning` needs you (it pulses unless
    macOS *Reduce motion* is on), `error` failed, `success` done, `accent`
    new output. The title is centred between it and the ×'s slot.
  - **Underline**: the title is underlined in the status colour (2 px,
    rounded, its width — at least 24 px, centred, under a one-letter title
    like `~` — 5 px below it, clear of every border); working is
    an accent third sweeping along a faint track (standing with Reduce
    motion).
  - **Detailed**: a 44 px strip; under the title a second line says it in
    words (`Needs you`, `Failed`, `Working`, `Done`, `New output`) after a
    small spinner or dot, in the status colour (working and new output
    muted); with no status it shows where the folder lives (`~/zzz` under
    `rift`, keeping its end when cut: `…/zzz`; the whole path under a given
    name); a tab with nothing for it (home, idle) centres its title. The ×
    and the tab's icon sit on the title's line, so the second line runs the
    tab's whole width.

  A status that comes from another pane than the focused one shows as a
  ring instead of a dot, a faded spinner, a dashed underline, or a second
  line naming that pane (`atlas · Needs you`). Agents are known by their
  command (`claude`, `codex`, `opencode`, `gemini`, after env assignments,
  `npx`/`bunx` and wrappers) or their own notification. An agent or a
  full-screen program spins only while it says it works (`◐`, a braille
  spinner or `✦` leading its title, or OSC 9;4 progress); OpenCode never
  does, and vim or htop left open show no spinner. A working agent counts
  as running even when its start was lost (reattached after a restart). Tabs carry no agent
  brand marks; notification toasts and the popover do.
- The active tab joins the pane below (same ground, no rule) with a 2 px
  accent top edge (its tab color when it has one); a hovered tab lifts. A
  colored tab shows its color as the 2 px top edge when inactive too. A tab
  without a color of its own takes the deepest `[[tab_color_rules]]` entry
  matching its active pane's directory.
- `×` shows on the hovered tab (the active one too only while hovered) and
  closes it (all its panes); `tab_close_side = "left"` puts the ×'s slot at
  the tab's left end (the macOS style) and the status slot at its right.
  `tab_close_button = "always"` shows it on every tab; `"never"` on none
  (⌘W, the tab menu and the palette still close tabs).
- **Tab numbers**: hold ⌘ alone (the modifiers of `tab_select_1`) for
  300 ms and every tab shows its shortcut in the ×'s place, on active and
  inactive tabs: `⌘1`…`⌘8` by place among the shown tabs, `⌘9` on the last
  (`tab_select_9` is the last tab; tabs 9…n−1 show none). Releasing ⌘ or
  pressing any key (a chord such as ⌘C never flashes them) takes them away;
  a rebound `tab_select_N` shows its own chord, an unbound one none. The
  vertical rail shows them in the same place on its rows.
- The tab's tooltip is a card: its title over every pane's full path
  (`~`-shortened) with its state words (`· focused` on a split tab), the
  program's own title or the agent's (`Claude Code — waiting for you:
  Classic template sweep`), a linked worktree (`Worktree <dir> · <branch>`)
  and its shortcut. Its accessible label adds the status (`, needs you in
  another pane` when it is another pane's), the agent and the progress.
- `+` spawns a new tab
  (the default profile), at the end of the strip or, with
  `new_tab_placement = "after_current"`, right after the active tab. A `⌄`
  is joined to its right (one pill, a divider between; the same in the
  vertical tab layout's strip): a click on it, or a right-click on `+`, opens
  the **profile menu** under the button: one row per profile (its glyph or a
  terminal, the name, the default profile in bold, its ⌃⇧1…⌃⇧9 shortcut),
  then, below a divider, Settings, Command Palette and About Rift. ↑/↓,
  Enter and Esc work in it; each row is a menu item for screen readers.
- `tab_bar = "hide_fullscreen"` hides the strip while the window is full
  screen, `"on_hover"` always; the panes take its height. The pointer at the
  window's top edge (4 px) slides it in over the panes with a shadow, and it
  goes again once the pointer leaves it (an open menu keeps it). On macOS the
  window buttons hide with it. The vertical tab layout keeps its strip.
- With `tab_bar_with_one_tab = false` and a single tab open (the Settings tab
  counts), the strip shows no tab: the traffic lights, panel buttons and `+`
  stay; the vertical rail goes away and the panes take its width.
- Drag a tab: past 4 px it lifts and follows the pointer, its neighbours
  reorder as it passes their midpoints, and its slot shows an accent caret.
  Esc while dragging puts it back where it was pressed. Neighbours reorder
  only while the pointer is over the strip (or rail): pulled out of it, the
  tab moves to another window on release — over another window it lands
  there (before the tab under the pointer when over that window's strip,
  else last) and that window takes focus; outside every window it opens a
  window of its own at the pointer. Released inside its own window it stays
  where it is.
- When the tabs no longer fit they shrink to 120 px, then scroll around the
  active one behind `‹ N` / `N ›` chips.
- Panel buttons (Warp layout), accent while their panel is open: a sidebar
  icon right after the traffic lights toggles the file explorer; at the far
  right, `+added −removed` for the active pane's repository (reloaded on
  `cd`, tab or pane switch and after each command) toggles code review — a
  bare `±` outside a repository or with no changes. The Settings tab has no
  repository: there the button is a dimmed `±` with no counts that does
  nothing (tooltip `No repository on the Settings tab`), and an open review
  panel hides until a shell tab is shown again, as it was. Left of it, an inbox
  (bell) icon opens the notifications popover; an accent dot marks unread
  entries. The vertical-tab layout's top strip has the same buttons.
- **Configurable toolbar** (Warp): those buttons, plus *Command palette*
  (search icon) and *Settings*, can go left of the tabs or at the strip's
  right end, in the order `toolbar_left` / `toolbar_right` list them, or be
  hidden. Right-click a toolbar button or the empty strip → *Rearrange
  toolbar items* opens **Edit toolbar** (also Settings › Appearance ›
  Toolbar buttons). The chips below sit left of the leftmost right-side
  button.
- **Update Rift** — while a newer release is downloaded and ready (and not
  dismissed with *Later*), a 22 px pill left of the right-side buttons: 1 px `warning` outline,
  `warning` text (12 px medium), no fill, a hover lift. Tooltip `vX.Y.Z
  available`; a click opens the update dialog (see *Dialogs and toasts*).
- **Remote · <device>** — while a phone is connected through remote access,
  the same pill in `accent`, left of *Update Rift* (or of the inbox):
  `Remote · <device name>` or `Remote · N devices` (label cut at 200 px).
  Tooltip `Connected: <names>` (`<name> via relay` for a relayed phone); a
  click opens Settings › Remote access.
- A tab with synchronized inputs on shows the sync icon in the accent, and
  a read-only one a lock, in the status slot's place (dot style: when it
  has no status).
- A tab whose panes report progress (OSC 9;4, `show_progress`) carries a
  2 px bar along its bottom edge on a 25 % track of the same colour:
  `accent` filled to the percentage (normal), `error` (error), `warning`
  (paused), and for indeterminate a third of the width sweeping across in
  `accent` (with Reduce motion on, a still full bar at 60 %). An error or
  paused report without a percentage fills the bar. With several reporting
  panes the tab shows the most severe (error, then paused, then the rest),
  the newest of equals. The tab's accessible label gains `, 42% done`,
  `, progress error`, `, progress paused`, `, progress paused at 60%` or
  `, in progress` (after its status and agent: *flow, working, Claude Code,
  progress error*). On macOS the
  Dock tile's badge shows the percentage of the focused window's active tab
  (`!` for an error; none for indeterminate).
- The worktree a tab's tooltip names is the repository's common git
  directory plus its path, never the branch: `git switch` changes the
  branch shown, not which worktree the tab is. Remote (ssh) panes never
  inspect a local path.
- A program that owns the screen (Claude Code, vim) sits on the bottom
  padding: the part of a row left over goes above its first row.
- The Settings tab (⌘,) sits after the shell tabs with a settings icon; see
  *Settings tab*.
- Pinned tabs sit first, 44 px wide, showing only one mark: a held ⌘'s
  number, else the status (dot style's mark in every style), else what the
  tab is (a terminal icon, or the sync or lock mark); no ×.
- A tab group draws a 20 px pill before its first tab: the name on its
  color at 18 % with a 1 px border (folded: the color at 85 %, dark text and
  the tab count). Its tabs carry the group color as their top edge. Picked
  tabs (⌘-click) are tinted in the accent at 14 %. A tab with a read-only
  pane shows a lock.

## Vertical tab rail

`tab_layout = "vertical"` (Settings → Appearance → *Tab position: Side*) replaces the
tab strip with a 220 px rail on the left: one 54 px row per tab with the
title (as the strip names it), after the tab's status as the dot style
draws it (a spinner, a dot, or a ring for another pane's; else its color
dot), a sync icon when synced, then the directory and git branch
(`<worktree dir> · <branch>` inside a linked worktree) and pane count; the
active row sits on the pane ground with a 2 px accent left edge. The rail
keeps a 34 px top strip under every `tab_style`. A progress report (OSC
9;4) draws the tab strip's bar along the row's bottom edge. Click selects,
× closes (as on the strip: the hovered row's, every row's with
`tab_close_button = "always"`, none with `"never"`; a held ⌘'s number takes
its place); the directory line keeps its end (`…/zzz/rift · main`); ⌘T adds a tab; drag a row up or down to
reorder. When the tabs do not fit, the rail keeps the active tab visible: a
`+N above` line at the top and a `+N more · palette lists all` line at the
bottom stand in for the hidden tabs, each marked with the most urgent
status among them. Clicking one selects the nearest hidden tab.

Above the rows a 26 px search field (*Search tabs*, search icon; accent
border and caret while it has the keyboard) filters them, with "No tabs
match" when nothing does, and a 26 px toggle beside it (accent tint while
on) switches to the panes view: 28 px rows indented under a split tab, one
per pane (its program's title without a coding agent's status mark — `✳`,
`◑`, a braille frame — or its directory, a spinner or lock, a 2 px accent mark on
the focused one). Pinned tabs come first under a *PINNED* label as 32 px
one-line rows (bookmark mark), the rest under *TABS*. A group's 28 px header
row shows a chevron (open / folded), a 3 px bar in its color, its name and
tab count; its tabs sit indented with a 2 px bar in that color. Rows end
their second line with the repository's `+added` (success) `−removed`
(error) and `#PR` (accent) in mono 11 px.

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
  git-ignored entries hidden; 2 000 entries per page — a bigger folder
  shows the first 2 000 in this order, then a selectable `Show 2000 more
  entries` row (pointer or ↑↓ + ⏎; a button in the accessibility tree) that
  loads the next page. A folder that cannot be read shows the error in its place (e.g.
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
- Right-click a row: Open (a folder: Expand or collapse) · Copy path · Copy
  relative path (from the explorer's root; `.` for the root itself) · New
  file… · Reveal in Finder (Linux: Show in file manager; a headless run only
  toasts). New file… asks for a name in the palette's text input; the row
  below shows the path it creates in the folder (the row's folder, or the
  file's parent): ⏎ creates the empty file, opens its folder in the tree and
  selects it. A name only: a path (any `/`), `.`, `..` or nothing is refused,
  and so is a symlinked folder leading out of the explorer's root; a name
  that exists already is refused (*… already exists*), never overwritten.

## File viewer

Opens from the explorer or the review panel's ↗ as its own pane: the
leftmost column of the tab, next to the explorer (two panes become three; the tab's pill
counts it). Opening another file reuses that pane. It takes focus like any
pane (click, ⌥⌘arrows) and ⌘W, Esc or ✕ closes it. Its header is the
pane's title bar (same height, drags the pane): the path centred (it moves
to the left edge when the pane is too narrow to centre it), then, for
Markdown files, a **Rendered | Raw** switch, ⋮ (Insert path, Copy path, Find
in file, Send to agent…, Show raw/rendered, Close) and ✕.

In the source view a click selects a line and ⇧-click or ⇧↑ ⇧↓ extend the
selection (an `accent` wash with a 2 px bar at the left edge); Esc clears it
before it closes the viewer. `a` or ⋮ › Send to agent… sends the selected
lines (else the top line on screen) to a pane; see Code review below.

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

In Rendered, a fenced block of a shell language (`sh`, `bash`, `zsh`,
`shell`, `fish`, `console`, `shell-session`) has a label row with its
language and a **Run…** button (an accessibility button *Run sh block…*,
the next `sh` block's *Run sh block 2…*);
a `mermaid` block's label reads *Mermaid diagram · source (not rendered)*
and shows its source (there is no diagram renderer). Run never runs on its
own: it opens a dialog — *Run this code block?* with the command (masked
while `mask_secrets` is on) — focused on Cancel, so ⏎ alone runs nothing.
Run sends the block's own text (for `console` / `shell-session`, the `$ `
lines without the prompt) to the tab's terminal pane as a submitted command;
a read-only pane refuses it (toast, no dialog), and a pane running a program
is refused until its prompt is back.

⌘F (the `search` shortcut) opens a find bar under the header for the file
instead of the terminal search: case-insensitive plain text, the counter
shows `n/total` or `No matches`, Enter / ⇧Enter step to the next / previous
match (wrapping), Esc closes the bar (a second Esc closes the viewer).

### config.toml editor

Settings › *Open config.toml* opens the same pane beside the Settings page,
and only there is the file editable (every other file stays read-only). A
config.toml that is not UTF-8 text, too large or binary stays read-only too,
and the bar under the text says why; once the file is fixed elsewhere the
pane loads it again, editable. A caret (`cursor` token, 2 px) follows typing;
Enter adds a line, Tab two spaces; ← → ↑ ↓ (↑ ↓ keep the column), ⌥← ⌥→ by
word, Home/End and ⌘← ⌘→ the line's ends, ⌘↑ ⌘↓ the file's ends, Page
Up/Down a page; ⇧ with any of them selects (`selection` token), as does a
drag; a click places the caret. ⌘A, ⌘C, ⌘X, ⌘V, ⌘Z / ⇧⌘Z, ⌫ / ⌥⌫ / ⌘⌫ and
⌦ work as in the command line. Highlighting follows the edits. ⌘F still
finds in the text.

- **Clean** (the text is the file's): the pane follows the file — a Settings
  change, a hot reload or another editor's save shows at once, scroll and
  caret kept.
- **Dirty**: the header shows **Dismiss** (back to the file's text) and
  **Save** (accent; ⌘S). Save writes the text exactly as typed and applies
  it at once (a Settings change right after never writes the old values
  back); *Saved* with a green check shows for 1.5 s. While the
  text has an error Save is dimmed and does nothing; its tooltip says
  `Fix N error(s) to save`.
- **Lint** runs 150 ms after the last keystroke: a 6 px dot left of the line
  number (`error` red, `warning` amber; hover: a card with the line's
  problems) and a 1.5 px underline under the span. A 28 px bar under the
  text reads `2 errors · 1 warning — line 4: …` (the first problem); a click
  moves the caret to the next problem, wrapping. Screen readers hear it as a
  polite live region; the text is a multi-line text input. Toasts in the
  window's corner sit above the bar.
- **Conflict**: the file changes on disk while the text is dirty — the edits
  are never replaced; a 28 px bar under the header says *config.toml changed
  outside this pane* with **Keep mine** (the bar goes; Save then overwrites
  the file) and **Reload** (the file's text replaces yours, one ⌘Z away).
- **Leaving with unsaved edits** — ✕, Esc, ⌘W, closing the Settings tab,
  opening another file in the viewer, closing the window, ⌘Q or the update
  dialog's Update (its restart) — asks *Save
  changes to config.toml?*: **Discard** (⌘⌫) at the left, **Cancel** (esc)
  and **Save** (⏎; shut while the text has errors, and the keyboard then
  starts on Cancel). Save or Discard goes on with what was asked. ⌘Q from
  another window asks in the window holding the edits, which comes to the
  front (a hidden drop-down window slides in); Cancel keeps every window.

## Code review (⇧⌘G)

A 460 px panel right of the panes listing the repository's changes. The
header shows the branch, the base chip (`Uncommitted ▾` or `vs <branch> ▾`)
and total `+added −removed`; ⤢ maximizes and ↻ reloads. The panel is opaque
(`bg`) even with a translucent window. The title reads `CODE REVIEW`. An
error notice (a failed action) shows as a red strip under the header. With
nothing to list the body says `Loading changes…`, `Not a git repository`,
`No changes — working tree clean` or (against a branch) `No changes against
<branch>`.

- **Base chip**: opens a palette picker (`COMPARE AGAINST`: Uncommitted,
  `Branch: <default>`; `BRANCHES`: every local then remote branch, filtered
  as you type). Against a branch a strip under the header reads `vs <branch>
  (merge-base <7-char sha>) · read-only: stage and revert need Uncommitted`;
  the list is one `CHANGES` section (merge base → working tree, untracked
  files included) with no Stage all, no stage / unstage / revert buttons and a
  copy-only hunk toolbar, and `s` `u` `r` show a toast with the reason. The
  base is remembered per repository for the session.
- **Commit bar** (the panel's foot): a message field (`bg_chrome`, hairline
  border, `accent` while focused; grows to 4 lines; placeholder `Commit message
  · ⌘⏎ commits`), then the reason Commit is off (`Nothing staged`, `Write a
  commit message`, `Commit from Uncommitted, which shows what is staged`) or
  a spinner with `Pushing…` / `Committing…`, and the **Push** and **Commit**
  buttons (a disabled one is muted and takes no click). Click the field to
  type; Enter adds a line, ⌘⏎ commits, ⌘V pastes, Esc or a click elsewhere
  leaves it. Success shows a toast (`[branch sha] message`, `Pushed <branch>
  to <remote>`); a failure an error toast with git's output (a hook's stderr,
  a rejected push).
- **Send to agent…**: `a` on a hunk or file (keyboard mode), or right-click a
  file header, hunk band or diff line → menu `Send to agent…`. The target
  picker (`Send to which pane…`) lists `AGENTS` (panes whose running command
  is Claude Code, Codex, OpenCode or Gemini CLI) then `OTHER PANES`, each as
  `<agent or title> — window N · tab N` (plus ` · pane N` in a split tab) with
  its directory; from the review panel the active pane is marked `(this
  pane)`. The footer reads `⏎ paste (no Enter)`. A sent paste focuses the
  target and toasts `Sent <file> to <agent>`; into a shell's command editor,
  `Sent <file> to the pane's command line`.

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

- **Keyboard** (⌥⌘G, `focus_review`): the panel takes the keys, with a 2 px
  `accent` focus ring on the section, file or hunk band it stands on. ↑ ↓ (`j`
  `k`, ⇥ ⇧⇥) move between sections, files and the hunks of expanded files,
  ← → fold and unfold, Enter / Space toggle, `s` stage, `u` unstage, `r` revert
  (it keeps its confirm dialog), `o` open, `c` copy the hunk, `a` send the hunk
  or file to an agent; on a section `s` /
  `u` act on every file. After a write the focus stays at its place (a staged
  hunk moves it to the file's next hunk). Esc returns the keys to the command
  line, as does any other plain key; a click elsewhere also leaves the panel.
  Full list in keyboard-shortcuts.md.

Both panels hide when the window is too narrow to leave 360 px for the panes
(review first). While the review panel is hidden this way, its toolbar button
is not lit, and ⇧⌘G / the button show a "Window too narrow" toast instead of
toggling; widening the window brings the panel back.

## Blocks (terminal area)

Each command becomes a block via shell-integration OSC 133 markers:

- **Header row** (synthesized, replaces the shell's prompt echo):
  - collapse chevron `▾`/`▸` — click toggles collapse
  - command text (bright), cut with `…` at the cells it is drawn in (a CJK
    char takes two) so it never reaches the note or the right side; copy and
    re-input still use the whole command
  - context: `~/path git:(branch)` — cwd and branch *at the time the command ran*
    (dim; collapsed blocks show an italic `N lines hidden` here instead —
    the command gives way to it, and it is cut short before the badge; a
    block with an output filter active shows a warning-tinted pill `filter "q"
    · shown/total lines` instead; a regex reads `/q/`, and `Aa`, `not` and
    `±N` follow for match case, invert and context lines)
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
- **Block wash**: only selected blocks get an accent tint + 3 px accent
  border, from the header (sticky header too) through the output; a selected
  failed block takes the error red instead. Unselected failed blocks are not
  tinted (exit state lives on the border and badge, design 01).
- **Selected block**: selected via a click on the header or the output, or ⌘↑/⌘↓; target of
  ⇧⌘C / ⌥⇧⌘C / ⌘I / ⌘B / ⌃M; Esc or clicking again deselects. ⌘-click,
  ⇧-click, ⇧↑ / ⇧↓ and ⌘A (with a block selected) select several; each
  gets the wash, each block node in the accessibility tree is `selected`.
- **Filter separator**: with a block filter's context lines on, a dashed
  `text_muted` rule (4 px dashes, 8 px apart) fills a row between runs of
  kept lines; the block border runs through it.
- **Sticky command header**: when a block's output scrolls but its header would
  leave the viewport, the header pins to the top edge; clicking it jumps to the
  block. Not over a command that is still running while the view follows its
  output: the program owns the whole PTY height and redraws its top row
  (vttest, progress UIs). Scrolled up, or once the command ends, it pins again.
  `sticky_command_header = false` turns it off. Per pane it minimizes
  (`toggle_sticky_header`, the pane menu): a 28 × 16 px tab (`surface`,
  hairline border, accent on hover, a ⌄) at the grid's top right stands in
  for it; a click on the tab or the toggle brings the header back.
- **Block spacing and dividers**: blocks sit flush by default (Warp's compact
  mode); `block_spacing = "normal"` puts a blank row above each block header
  that follows other rows, and `block_dividers = true` draws a 1 px `border`
  rule between blocks (centred in that blank row).
- **Jump to bottom of block** (Warp): while the pointer is over a block that
  runs past the bottom of the pane and shows more than 70 px of it, a 28 px
  round button with a ⌄ sits 10 px in from the pane's bottom-right corner
  (accent ring on hover; tooltip *Jump to the bottom of this block*); a click
  scrolls the block's last row to the bottom, as the block menu's *Scroll to
  bottom of block* does. `jump_to_bottom_button = false` removes it.
- **Background blocks**: a background job's output sits in a block with no
  header and a muted left rule, its first row at the indent (the prompt it
  landed after is left out).
- **Subshell offer**: a 36 px bar (`surface`, accent hairline, shadow) at
  the bottom of a pane whose command started a subshell without Rift's
  hooks (see *features*): *Use Rift's blocks and editor in this subshell?*,
  an accent **Enable ⌘⏎** button, **Don't ask again** and ✕ (each in the
  accessibility tree); it goes when the command ends or the subshell's
  prompt arrives.
- **Rich input composer** (⌃⇧G / ⌘E over a running program, see
  *features*): the command editor's zone at the bottom of the pane (same
  divider, padding, chevron and input font, no fill of its own), its chip
  row replaced by *Compose for* (12 px sans, `text_muted`) and the program's
  name (semibold, `text_bright`), the text in `fg` without syntax colors,
  and a hint row under the input (`⏎ send   ⇧⏎ new line   esc close`, 11 px
  sans, `text_muted`; `⌘⏎ send   ⏎ new line` with `rich_input_submit =
  "cmd_enter"`). The program's rows end above it; its grid cursor hides.
  An unsent draft's toast carries a **Copy** button.
- **Reverse screen** (DECSCNM, `CSI ? 5 h`, used by vttest and `tput flash`):
  the pane's default foreground and background trade places until the program
  resets it (or a terminal reset).
- **Masked secrets** (`mask_secrets`, *Toggle secret masking*): API keys,
  tokens and the rows of a `-----BEGIN … PRIVATE KEY-----` block draw as
  `•`, one per character, so nothing reflows; a click on a masked span shows
  it for 5 s. Block headers, the history pane, Results, toasts, the side
  rail, copies, exports and comparisons show the masked text too (*Copy
  output unmasked* in the block menu is the exception).
- **Double-click** selects the whole unit under the pointer — a URL, a file
  path with its `:line:col`, an email, an IPv4/IPv6 address, a number, a
  UUID or a git SHA — else a word bounded by `word_chars`.
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
  or relative to the directory the command that printed it ran in, the
  pane's for unattributed output; the line and column printed with it are
  read from `f:12:3`, `f(12,3)`, `f(12, 3)`, `f[12, 3]`, `f, line 12, column
  3`, `File "f", line 12` and `f line 12`) — ⌘-click opens it where
  `link_editor` says, at that line. Output of a command that
  ran on another host (ssh, with Rift's remote hooks) names that host's
  files: none of its paths is a link or a hint, even when a local file of the
  same name exists. Hint mode (⇧⌘H) uses the same rule. Only http, https, file,
  mailto and ftp targets open.
- **Cell styles**: bold, italic (the embedded Hack and JetBrains Mono carry Italic/Bold Italic faces),
  dim, inverse, hidden, strikethrough, blinking (SGR 5: hidden and shown every
  600 ms), and underline variants (single, double, curly, dotted, dashed; SGR
  58 colored).
- **Block elements** (U+2580–U+259F: `█ ▀ ▄ ▌ ▐`, eighths, quadrants such as
  `▛ ▜ ▘ ▝`, shades `░ ▒ ▓`; legacy-computing sextants and eighth blocks
  U+1FB00–U+1FB3B, U+1FB70–U+1FB8B) are drawn as quads filling the whole cell
  and line height, not from the font, so block art (Claude Code's logo) has
  no gaps, as in Warp.
- **Box drawing and Powerline** (U+2500–U+257F: light, heavy and double
  lines, dashes, rounded corners, diagonals; U+E0B0–U+E0BF: arrows, half
  circles, triangles) are drawn as lines on device pixels from edge to edge of
  the line box (light lines 1 px per 13 px of font at 1×, heavy twice that),
  so frames and prompt segments join across rows and cells. They keep their
  colours under `minimum_contrast`.
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
- **Prompt chips on the phone** (the companion app's command line; mockup
  `PromptChips`): the pane's chips as this section draws them, sent with the
  `prompt_chips` feature (`docs/remote-protocol.md`) as the pane's order,
  tones (`bright`, `ui`, `dim`, `muted`, `success`, `error`, `warning`,
  `accent`, `magenta`, `indigo`: the theme's roles, drawn in the app's
  palette) and a short value per chip. One row of 32 pt chips (hairline, 4 %
  fill, radius 6, glyph and parts) that never wraps or scrolls: the chips
  that fit by their measured widths, then *+N* (white 8 %, a 6 pt `warning`
  dot per folded chip with `attention`: a Docker container down or
  unhealthy) opening the *This pane* sheet (glyph, name, full value, chevron
  per chip). While typing, completions replace the row.
- **Edit prompt** (Warp) and **Edit toolbar** share one arranger dialog
  (layer 6, 600 px `surface` card, radius 8, over a 40 % black backdrop):
  the title with `Restore default` on its row (accent; muted and inert
  while the draft is the default), a palette of the items not in use under
  *Not in your prompt* / *Not in the toolbar*, then the items in use in
  dashed buckets (1.5 px) under *Your prompt* / *Your toolbar*, each item
  with ×. Edit prompt has one bucket of chips (sample values; the Python,
  PHP and Docker chips read `python`, `php`, `docker`); every chip there
  has the same hover card as in the prompt (none while one is dragged).
  Edit toolbar has two equal buckets side by side, *Left* and *Right*, of
  framed buttons (icon and name). A click on a palette item adds it (a
  chip at the prompt's end, a button at the right end); dragging an item
  (from the palette or a bucket) lights the bucket under the pointer and
  shows an accent `|` where it lands — before the first item past the
  pointer on its row, or before the first item of a lower row — and the
  dragged item follows the pointer above the dialog; an item dropped
  outside every bucket is removed. Tab / ⇧Tab walk the bucket items (an
  accent ring; a click on one focuses it too), ← → move the focused one a
  place in reading order (across into the next or previous bucket), Delete
  or ⌫ removes it. An empty palette says `Every chip is in your prompt.` /
  `Every button is in your toolbar.`, an empty bucket `Drag chips here, or
  click one above` / `Drop a button here`. `Cancel` (key cap `esc`) and
  `Save changes` (`⏎`; enabled once something changed) close it; nothing
  is written before Save. Screen readers get one button per item and
  action, with Restore default and Save changes listed disabled while they
  do nothing, and the draft read out bucket by bucket.
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
- **Cursor**: `cursor_style` = block (8×17 px), bar (2 px), underline
  (2 px), hollow (a 1 px outline; the glyph keeps its colour) or
  double_underline (two 1 px lines), 1.1 s blink; with
  `cursor_hollow_unfocused` the editor and grid cursors draw hollow, without
  blinking, outside the focused pane of the focused window; exact shaped-text positioning, translucent when a glyph
  sits under it. Vim NORMAL and VISUAL always draw a solid, non-blinking
  block, REPLACE a non-blinking underline.
- **Vim mode pill** at the right edge of the input line when `vim_mode` is on:
  `NORMAL` (warning color), `INSERT` (muted), `VISUAL` / `VISUAL LINE`
  (accent) or `REPLACE` (error color). An open `/` or `?` search prompt
  shows there instead (`/text`, its last 10 chars), in bright text. The
  visual selection draws with the editor's selection highlight.
- **Ghost text**: inline dimmed completion preview continuing the line at
  the cursor (Warp), followed by a small chip naming its accept key (`→`,
  `Tab`, `⇧→` or the `accept_autosuggestion` binding; `autosuggestion_hint`;
  16 px, 4 px radius, hairline border, inverted while hovered or its menu is
  open; tooltip *Change keybinding*). A click opens a context menu above it
  (below with the input on top): `→`, `Tab`, `⇧→`, `Custom…`, the current one
  checked; a screen reader reads the chip as *Accept suggestion key: →* and
  the menu as *Accept suggestion key* with radio items. With
  `autosuggestion_ignore_button` a 16 px `✕` chip follows (tooltip and label
  *Ignore this suggestion*): a click ignores the suggested line. At the end of the
  line → (⌃F, ⌃E, ⌘→, End) accepts it all, ⌥→ one word, ⌃⌥→ one word part,
  ⌃→ one word anywhere; Esc clears it. Tab takes one word or path part of it only with
  `tab_behavior = "autosuggestion"`. Hidden while the menu Tab opened shows.
- **Completion panel** (the menu): opened by Tab (or as you type with
  `completions_open_while_typing`): the word candidates only — commands,
  git subcommands with descriptions, path completions (with `~/` expansion);
  history lines stay the ghost text, the ↑ menu and ⌃R.
  Docked to the editor across the full pane width (Warp's completion pane):
  directly above the editor when it sits at the bottom, below it with
  `input_position = "top"`. The output moves out from under it, as in Warp:
  the newest lines stay in view right above it (below it with the input at
  the top), and the program keeps its rows. The header names it with its
  count (`COMPLETIONS 7`, an `accent` underline; the ↑ menu's reads
  `HISTORY`). Rows show an icon (folder, file, branch, terminal), the name
  with its matched characters bold in the accent (the typed prefix, or each
  character of a fuzzy match), and the description at the right; ⇥ / ⇧⇥
  (wrapping) or ↑ / ↓ select, ⏎ or a click accepts a row. Your shell's own
  completions join the rows (as you type: within 300 ms of Rift's, keeping
  the picked row picked; a Tab waits for them). A pane on another host that
  cannot complete there (an `ssh` typed without a shared connection) ends the
  rows with a muted hint row (info icon) "Remote completions need an SSH
  profile" (alone, it opens no panel); accepting it only closes the panel.
  The footer shows `↑ ↓ navigate`, `⏎ accept`, `→ accept ghost` (with ghost
  text), `esc dismiss`. Square corners, hairline edge, up to 8 rows.
- **Welcome list**: a fresh session (no command run yet, single pane with
  room) shows *New session* with the shell, directory and branch, then five
  rows with their shortcuts: Split pane, Command palette, Command history,
  File explorer, Settings. Clicking a row does it; the list goes away after
  the first command or background output and stays hidden after clearing blocks;
  a shell reattached after a restart or update never shows it, and it is not
  shown while the pane shows any text (a shell's startup output).
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
  wash lighter (a 10 % white veil, as in Warp; a light 5 % shade on a light
  theme), so the pane you are in is the dark one. `inactive_pane_dim` sets
  the veil (1–40 %, half of it on a light theme); `dim_inactive_panes =
  false` drops it (the 1 px dividers keep theirs).
- A pane with its own colour scheme (a profile's `theme`, or *Pane theme:
  <name>*) draws its body, blocks and command line in that theme; its title
  bar, the dividers and the inactive wash stay in the window theme.
- Each pane has a 28 px title bar (Warp pane header): a terminal icon, the
  program's title (OSC 0/2), else the pane's directory, centered (after a
  spinner while a command runs); while the
  pane is focused or hovered, split (a new pane to its right), ⋮ (the pane
  menu) and × (close) show at the right. Clicking the bar focuses the pane.
- Pane menu (⋮, or right-click in the pane outside a block): Copy, Paste,
  Select all, Export buffer…, Split pane right / down, Maximize pane and
  Move to new tab (split tabs), Move to new window (disabled for the only
  pane of a window's only tab), Make read-only / Allow input, Restart pane,
  Minimize sticky command header / Show sticky command header (while
  `sticky_command_header` is on), Clear blocks, Close pane.
- A read-only pane's title bar shows a lock in place of the terminal icon.
- A pane kept after its program exited (`close_on_exit = "never"`, or
  `"graceful"` after a non-zero exit; the default `"always"` closes it) shows a 36 px bar
  over its bottom (`surface`, edged in the error color after a failure):
  "Process exited with code N" and Restart ⏎ (accent) and Close ⌘W.
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
  the keyboard (10 px per press).
- Zoom (⇧⌘⏎): the active pane temporarily takes the whole tab rect and renders
  like a single pane. Navigating or splitting unzooms.
- Every pane keeps the full editor with its prompt context row.
- The tab's tooltip lists its panes; its title follows the focused one.

## History pane (⌃R)

⌃R in the command line opens the history pane (unless the shell binds ⌃R
to a widget of its own, which then draws below the shell's prompt; see
features.md, Shell widgets), docked above the command line
across the pane like the completion panel; the output moves up out from
under it, so the newest lines stay in view. The command line is its search
field: what you type narrows the rows (any case, anywhere in the command).

- Header: `HISTORY`, scope chips **This pane** · **All shells** · **Failed**
  · **Results** (⌃R again or a click switches), `N of M` (Results: `N runs`,
  `N+` when the search stopped at its bound), and ×.
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
- **Results** scope (features.md, History pane): under the header a filter
  line — the query's filter words as accent chips (`Failed`, `Last 7 days`,
  `Host prod`, …), else the words you can type (`is:failed is:ok
  since:today since:7d dir:… host:… branch:…`) — and at its right what was
  searched (`Open panes and saved transcripts`, `Open panes only (saved
  transcripts are off)`, `Searching…`, or that it stopped at its limit). Up
  to six rows: status dot, the command (match in the accent), `host · branch
  · directory` in muted mono, and the age or `date unknown`. Below them the
  selected run's output preview (eight mono lines on a faint ground): a
  heading (`Output · match on line 12`, counted from the output's first
  line, each line gone from scrollback counting as one; `match in the command only`, `Output
  no longer in scrollback`, `restored from a previous session`, `saved
  transcript`) and the lines around the first match, that line tinted and
  the match bold in the accent. The footer's ⏎ reads `rerun here`. A click
  selects a run (the preview follows); a double-click puts it on the command
  line; nothing runs from the preview.
- ⏎ with no matching row runs the typed line; ⇥ with none closes the pane. A click outside it, focusing another pane, the viewer,
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
  vim keybindings`, `Toggle secret masking`, one
  `Launch configuration: <name>` row per file in
  `~/.config/rift/launch_configs/` (re-scanned each time the palette opens),
  `Theme: <name>` rows, `Pane theme: default` and one `Pane theme: <name>`
  row per theme (the active pane's own colour scheme), `Action: <name>` per
  `[[actions]]` entry (with its chord), `New tab: <profile>`, `Split right:
  <profile>` and `Split down: <profile>` per profile, `Install 'rift'
  command`, `Cycle tab color`, `Set tab color…`, plus recent commands.
- **History** — imported shell history (zsh, bash or fish, from the
  configured shell's history file) + per-pane local history; Enter runs, or
  insert into the editor. (⌃R in the command line opens the history pane.)
- **Session switcher** (⇧⌘O) — tabs, panes of split tabs (directory and
  running job), the other windows' tabs (`<title> — window N`, N counting
  from 1 in opening order; Enter raises that window), tabs of other Rift
  processes, detached session-service shells, and the 40 newest finished
  blocks of this window. Enter jumps there.

- **Send output to command…** — a fourth mode, opened from the block menu or
  the action `Send block output to a command`, with no mode chips. The
  placeholder reads `Command that reads the output on stdin…`; an
  `OUTPUT (STDIN)` preview lists the selected block's lines under the row that
  says what will run, and the footer reads `⏎ run with output on stdin`.

- **New worktree tab…** — a fifth mode, opened from that action, with no
  mode chips. The query is the new branch's name, prefilled with the first
  free `wt-<n>`, selected (the selection colour behind it): the first key
  typed replaces it, ⌫ clears it, ⏎ takes it (placeholder `Name of the new branch…`); the one row reads
  `Create branch <name> from HEAD in <repo-parent>/<repo>.worktrees/<name>`
  (or `Not inside a Git repository`), and the footer
  `⏎ create worktree and open a tab`. A pane without a local directory
  (ssh) gets an error toast instead.

- **Rename window…** — opened from that action, with no mode chips. The
  query is the window's name, prefilled with its current one and selected;
  the one row reads `Name this window “<name>”` (with nothing typed, `Clear
  this window's name`), the placeholder `Name of this window…` and the
  footer `⏎ name the window`. Control characters are dropped and the name
  is cut at 100 characters.

- **Theme picker** (*Theme picker (live preview)*) and **Set tab color…**
  (the tab menu's *Custom color…* too) — text modes with no mode chips:
  `Search themes: ↑ ↓ preview, ⏎ keeps, esc reverts…` and `Tab color: a name
  or #rrggbb…`. The theme rows are every theme, the one in use tagged *in
  use*.

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

**Tab markers** — a tab with unread entries in a pane shows them as its
status (failed, done; a request is needs you; a notification or bell is
new output), see *Tab bar*.

## Hint mode (⇧⌘H)

Every visible URL, existing file path and block header gets a short letter
label drawn over it. Typing a label opens the link or path, or selects the
block; ⇧ + label copies it (a block copies its output). Esc cancels.

## Recent tabs (⌃Tab, `tab_switch_order = "mru"`)

While ⌃ is held after ⌃Tab: a 460 px card (radius 8, surface colour, the
palette's shadow) under the tab bar, headed RECENT TABS, one 44 px row per
tab of the window, last used first: its title, and its directory in the
mono face below. The highlighted row (accent at 16 %) is the tab releasing
⌃ switches to; it starts on the previous tab. No dimmed backdrop. Screen
readers get a *Recent tabs* list dialog.

## Mark mode (⇧⌘M)

A cell-sized block cursor (accent border, accent at 35 % inside) over the
terminal rows; the selection it extends is the ordinary selection colour.
Nothing else changes on screen.

## Dialogs and toasts

- **Confirm dialogs** — a 460 px card (radius 12): title, description and
  note, the affected lines in an inset list (at most three, then `and N
  more…`), and a footer bar with Cancel `esc` and a confirm button `⏎` (red
  for discarding work). Running processes list one row each with the
  program's badge (a coding agent's brand disc, such as the Claude mark,
  else a terminal mark), the command, and the agent's name; pasted lines
  and files are plain code lines. Used for closing
  running commands ("Close window?" for one of several windows, "Quit
  Rift?" for the last one or ⌘Q, which lists every window's), pasting multi-line text into a shell without bracketed
  paste ("Paste and run?"), a paste over `large_paste_warning` ("Paste a
  large amount of text?": its size and line count, its first five lines
  masked like the screen), installing shell integration on an SSH host,
  opening a saved layout from a link when it has startup commands or a
  custom program, arguments or environment (Cancel focused; environment
  values stay private), and
  updating ("Update to Rift vX.Y.Z?": what the restart keeps — tabs,
  running shells, windows that cannot be reopened — with *Later* `esc` and
  *Update* `⏎`, or *Open download page* for an install that cannot update
  itself; Later hides the chip for that version).
  Esc cancels. The confirm button has the keyboard when the dialog opens;
  ⇥ / ⇧⇥ and ← → move it between the two buttons (a 2 px `accent` focus
  ring marks it) and Enter presses the focused one. An Enter that is
  auto-repeated, or that arrives within 300 ms of the dialog opening (a held
  or typed-ahead key), never confirms; pressing Cancel needs no such wait.
- **Toasts** — short notices about what you just did (copied, a keybinding
  moved, config.toml was not applied, a copy, paste, open or save failed, a
  phone's action, …), as a 30 px chip in the pane they are about: in its
  command line's chip row, right-aligned, newest lowest, more stacking up
  over the output. Each has a kind icon in a tinted disc — a green check
  (it worked), a red `!` (it failed, with a red edge), an accent `i`, a
  phone (a paired phone did it) — and, when there is one, a button
  (*Open config.toml* for a config.toml that was not applied or a setting
  that could not be saved). Done goes after 2.6 s, information and a
  phone's action after 4 s, hovering keeps them; a failure stays until its
  ✕. The same text replaces itself; at most six are kept. A pane out of
  view hands its toasts to the pane you are in; with no pane showing (the
  Settings tab) they sit in the window's bottom-right corner. Updates never use them: Rift's own
  update notices are notification cards (*Notifications*, Toasts: the same
  card, with the app icon as its badge; a click acts on it — the update
  dialog, What's New — and ✕ dismisses). *Rift vX.Y.Z is ready* and a
  failed update stay until dismissed; the others leave after `toast_secs`.
- **Tooltips** — resting the pointer on a control for 0.6 s shows what it
  does and its shortcut from the live keymap: the tab bar's +, ✕, explorer,
  review and inbox buttons, a block's chevron and copy button, a split
  pane's split, ⋮ and ✕, the sticky header ("Scroll to this command"), a
  block's duration (the full time), the history pane's scope chips
  and ✕, Settings' ↺, "Edit prompt", "Edit toolbar" and "Open config.toml" (with its path),
  the explorer's and review panel's buttons, the file viewer's ✕ and ⋮, and
  on Linux the drawn traffic lights ("Close window", "Minimize", "Zoom"), the
  toolbar's palette and Settings buttons, and the jump-to-bottom button. A
  part of the command line shows the **command inspector**'s card (its text
  as the title, then *Role · what it does*; ⇧⌘I pins it at the cursor). A
  prompt chip shows its hover card; the **Time on this tab** card is
  interactive: it stays open while the pointer moves from the chip into it, and
  its **Reset** button (pointer cursor; also in the accessibility tree and as
  the palette action `reset_tab_timer`) zeroes the tab's time. A tab shows its full
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

Bar overlay above the pane's sticky command header; live yellow highlight of
all matches, orange for the current one.
Enter/⇧Enter or ⌘G/⇧⌘G cycle matches (viewport jumps to each), Esc closes.
⌘A selects the query (a `selection` band; typing replaces it, ⌫ clears it,
⌘C copies it); ⌫ removes a whole character. Two toggle chips at the right:
`Aa` match case (⌥C) and `.*` regular expression (⌥R), also toggle buttons
in the accessibility tree; an invalid regex shows `invalid regex` in red instead of the
counter. The counter shows the current match's position among all matches
(`n/total`; `search` while the query is empty, `0 results` when nothing
matches; `· N hidden by the filter` after it when matches lie in output
lines a block filter hides — those are counted, not visited); only the
newest 500 matches are kept for navigation (older
ones are counted, not visited), so with 600 matches it runs 101/600 …
600/600. While the pane prints, matches are recounted (at most every
100 ms) without moving the view: the selected match stays selected, and
once scrollback drops it the oldest match left is selected. Text is matched as printed: wide glyphs (日本) and combining marks
(a decomposed `é`, e + U+0301) are found, a typed `é` finds both spellings
(plain text and query are compared canonically decomposed; so does the block
filter; a regex is matched, unchanged, against the text canonically
composed, so `[é]` is one character and also finds e + U+0301), and a match
highlights whole cells; hidden (SGR 8) text is not searchable. A query longer
than the field scrolls so its end stays visible. With the file viewer open,
⌘F finds in the file instead (see File viewer).

With a block selected, ⌘F finds in that block's output only: an accent chip
`in block: <command> ×` sits after the magnifier (the dialog is named *Find
in block: <command>*); matches and ⌘G / Enter wrap inside the block. A click
on the chip (its accessibility button *Search the whole pane*) or
deselecting the block widens find to the whole pane.

With a block filter active (⌥⇧F), the same bar edits the filter: only that
block's output lines matching it stay visible. The bar is 340 px wide plus
30 px per toggle chip (find: `Aa` `.*`; filter: `Aa` `.*` `!` `±N`, the
last one wider) plus the in-block chip, labelled `filter`, with the hint
`Enter keep · Esc clear` in place of the counter (`invalid regex` in red
while the regex does not compile). The filter's chips are toggle buttons in
the accessibility tree (*Match case*, *Regular expression*, *Invert match*,
*Context lines: N*).

## Block context menu (right-click or ⌃M)

Copy command · Copy output · Copy output unmasked (only while
`mask_secrets` is on) · Copy both · Copy working directory ·
Copy git branch · Re-run command · Filter output…/Turn off output filter/Turn
on output filter · Send output to command… · Bookmark/Remove bookmark ·
Collapse/Expand · Scroll to top of block · Scroll to bottom of block · Copy
as HTML · Copy as Markdown · Save output… · Compare with previous run (how
long ago; disabled with the reason: *none found*, *not finished*, *remote
host unknown*) · Clear output filter (when the block has one) · Open in
viewer (only on a block that printed one local Markdown file — `cat`,
`bat`, `less`, `more`, `glow`, `head` or `tail` with one `.md` argument and
no pipe, redirection or expansion — that still exists: the file opens
Rendered in the viewer).
On a block of a multi-block selection: Copy N commands · Copy N outputs ·
Copy commands and outputs · Insert N commands into the input · Compare
outputs (exactly two finished) · Bookmark all/Remove bookmarks · Collapse
all/Expand all.
Right-clicked on a link, the menu starts with Open link · Copy URL; on an
existing file path, with Open (at the printed line) · Copy path · Reveal in
Finder (Linux: Show in file manager) — the pane menu too.

## Compare outputs

A 960 px modal (radius 8, `surface`, layer 6 under a confirm dialog, black
55 % scrim): title *Compare outputs* with the baseline note (*against the
previous run of the same command (3 min ago)*) and ✕; one row per block —
`−` (error) / `+` (success), the command in mono, then `exit N · duration ·
~/dir · age · restored`, in `warning` with what is missing (*first N rows
trimmed from scrollback*, *cut at 20000 lines / 4 MB*, *output unavailable
(trimmed from scrollback)*); then the diff, 18 px rows: line number, mark,
text (cut at 400 characters), removed rows tinted error 14 %, added success
14 %, each with a 3 px edge; *Comparing…* while the worker runs, *No
differences* (*… in the compared part* when a side is incomplete) when
equal. Footer: the result (`N added, M removed`, plus *too many changes: the
changed part is shown whole*; `warning` when incomplete) and the keys. In
the accessibility tree a dialog *Compare outputs* whose status reads both
blocks, the result and the first 50 changed lines, with a *Close* button.

## Save prompt (Save output… / Export buffer…)

The palette's text input (no mode chips) prefilled with
`<dir>/<command>-output.txt` or `<dir>/rift-buffer.txt` (selected: typing
replaces it); one row: *Save the output to ~/x.txt*, *Replace ~/x.txt with
the output* when the file exists, *~/x is a directory*. Enter writes; over
an existing file it first asks *Replace the file?* (Replace / Cancel). A
toast says *Saved the output to …* or why it could not. With `mask_secrets`
on, the text is written masked, as shown. Cancel returns from replacement
confirmation to the path prompt. Errors keep the captured output and select
the path for correction; pipes and devices are refused.

## Menus

Every menu (block, tab, pane, file viewer) is a 240 px list (up to 320 px when a label and its shortcut need it) with an icon
column and the row's shortcut from the live keymap at the right. Only Close
pane and the three Close tab rows are red. Disabled items are dimmed (e.g.
Copy output on an output-less block). In a short window the rows shrink
(down to half height) so every item stays reachable.

**Tab menu** (right-click a tab): Rename tab · Duplicate tab · Synchronize
inputs in panes / Stop synchronizing inputs (disabled for a tab with one
pane) · color swatches Red, Green, Yellow, Blue, Magenta, Cyan · Custom
color… (the palette's text input: a name or any `#rrggbb`) · No color ·
Pin tab / Unpin tab · Group tab / Group picked tabs (not on a pinned tab) ·
for a grouped tab Remove from group, Collapse/Expand group, Rename group,
Ungroup, Close group ·
Move to new window (disabled for a window's only tab) · one *Move to window
N: <its active tab's title>* row per other window (titles cut to 12
characters), and *Move to hotkey window* while the drop-down window exists ·
Close tab · Close other tabs · Close tabs to the right.

**Group menu** (right-click a group's pill or rail header): Collapse group /
Expand group · Rename group · Ungroup · Close group.

**File viewer menu** (⋮): Insert path · Copy path · Find in file · Send to
agent… · Show raw / Show rendered (Markdown files only) · Close.

**Profile menu** (the `+` button's ⌄), **pane menu**, **block menu** and
**explorer menu**: see *Tab bar*, *Split panes*, *Block context menu* and
*File explorer*.

## Settings tab (⌘,)

Settings opens as a tab after the shell tabs (settings icon); the shells keep
running and the tab strip stays live. ⌘, again goes back to the shell tab;
clicking any tab switches; the tab's ×, ⌘W or Esc closes it. Going back to
the Settings tab (its tab, ⌘, or *Open Settings* in the palette) shows the
page it showed; once closed, it opens again on General. Other app
shortcuts (new tab, next tab, palette…) work from it and leave it.

A 232 px sidebar: a search field, the twelve pages (thirteen on macOS, with Permissions) with icons (arrow keys
navigate pages; PageUp/PageDown scroll the body; in a short window the page
rows close their 2 px gaps, then shorten to as little as 22 px, so the foot keeps its room) and *Open config.toml* at
the foot (opens the file beside the page, in the Settings tab; ✕ closes it; for 3 s after config.toml is
written or reloaded, a green check sits at its end, a green dot on its icon
in the icon-only sidebar). When the settings area is
under 700 px wide the sidebar shrinks to 56 px of icons, with no search
field or labels. The body (at most 720 px wide, centred right of the sidebar like Warp's)
has the page title and description, group labels, and flat rows: label (and
description) left, control right; it runs to the bottom edge.

- **Search**: typing anywhere outside Keybindings fills the search field and
  lists the matching settings of General, Profiles (its two settings, not
  the list), Appearance, Input, Terminal, Files, Notifications, Remote
  access's switch (by name or description; not Keybindings) and the themes
  by name (a theme's row applies it, `active` on the one in use), each
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
  Home opens General; End opens Themes. Scalar controls write that file
  live (the hot-reload watcher applies changes < 300 ms, including external
  edits); while config.toml is invalid, controls do not save and a toast says
  to fix it first; the wheel scrolls the page body (24 px per line, clamped
  to the content) and switching pages resets the scroll. Clicks elsewhere in
  the page are swallowed rather than reaching the terminal.
- **Text fields** (the relay's, `Shell path`, `Global hotkey`, a profile's
  fields, …): a click edits one (accent border, caret at the end); typing
  and ⌘V fill it, ⌫ deletes (⌘⌫ / ⌥⌫ clear it), ⏎ saves, Esc cancels, Tab
  or a click elsewhere saves. A value that is not valid is never written:
  ⏎ keeps it in the field to fix, a click away drops it; a toast (a
  profile's: a red line in its form) says why. Lists (`Font features`,
  `More subshell commands`, a profile's arguments and environment) are
  words separated by spaces, quoted as in a shell when one has spaces.
- Scalar config.toml keys have rows; structured collections have dedicated
  editors: SSH profiles on Profiles, custom actions on Actions, secret patterns
  on Terminal, and tab color rules on Appearance. Profiles and Keybindings have
  their own pages. `settings::ROWS` and the dedicated-editor coverage list guard
  new keys. Internal metadata and generated state follow the
  [Settings UI exposure policy](configuration.md#settings-ui-exposure-policy).
- Structured editors use the same fields, buttons, separators and focus order
  as Settings: each entry is a row with **Edit** at its right, and a
  **New …** row ends the list with **Add**, as on Profiles. **Add/Edit** opens a draft; **Save** validates and persists it,
  **Cancel** discards it, and **Delete** asks for confirmation. **Move up/down**
  saves valid edits and changes order. Stale drafts remain visible with an error
  after another window or external editor changes their list. Search finds each
  collection. The Actions page includes shortcut recording and ordered step
  editing: each step's row has **Edit step** (**Editing** on the one the fields
  below edit) and a **Step type** segmented control picks Run action, Send text
  or Open layout; Send text is multiline (Shift+Enter for a newline). Secret
  patterns have a temporary sample and a preview of what that pattern alone
  masks, updated as you type either field. Tab color rules have RGB sliders,
  named colors (the picked one drawn selected), hex input and a color preview. SSH's **Connect** saves valid edits
  before opening the normal SSH connection.

- **Actions** — named command sequences with an optional shortcut. Add, edit,
  reorder or delete an action; its ordered steps run an existing action, send
  multiline terminal input, or open a saved launch configuration. Save reports
  shortcut conflicts, missing references and cycles without executing the steps.

- **General** — *Shell and session behavior*.
  - App: `Launch at login` toggle (macOS only; `launch_at_login`: adds or
    removes Rift's login item; shows the login item as it is). There is no
    background-processes row: Remote access decides whether a quit keeps Rift
    running (see *Menu bar icon* above).
  - Shell: `Shell path` text field (`shell`; empty shows the login shell
    as its placeholder; a bare name is looked up on PATH and saved as its
    path; not an executable: refused), then the `Scrollback` stepper (lines,
    ±1000 within 1,000–1,000,000, shown as `10,000`; described *Maximum
    lines kept in the buffer*). Profiles have a page of their own.
  - Session: `Restore saved sessions on normal launches` toggle (crash
    recovery and updates restore regardless), `New tabs start in` segmented
    `Previous | Home | Custom` (a custom path is set in config.toml),
    `Warn before closing running commands` toggle,
    `Save block output for restore` toggle (*Output can contain secrets;
    turning this off deletes it*; `save_transcripts`), `Keep shells running
    through updates` toggle (*An update or a crash leaves them running*;
    `durable_sessions`).
  - Hotkey window: `Global hotkey` text field (`global_hotkey`, e.g.
    `alt+space`; placeholder `Off`; a chord that does not parse is refused),
    `Drop-down hotkey window` toggle (*The global hotkey
    slides a window down from the top of the screen*; `hotkey_window`),
    `Hotkey window height` stepper (%, ±5 within 20–100; *Share of the
    screen it covers*), `Screen edge` segmented `Top | Bottom | Left |
    Right` (`hotkey_window_edge`), `Hotkey window width` stepper (%, ±5
    within 20–100; `hotkey_window_width`), `Screen` text field (`pointer`,
    `primary` or a screen's number; `hotkey_window_screen`), `Hide it when
    it loses focus` toggle (`hotkey_window_hide_on_blur`).
  - Updates: `Check for updates` toggle (*At launch and every 6 hours;
    the update downloads, then Update Rift appears in the tab bar*; `check_for_updates`), then a
    `Rift <version>` row whose description is the update status (`Not
    checked yet`, `Checking for updates…`, `Up to date`, `Downloading
    vX.Y.Z…`, `vX.Y.Z available`, `Installing vX.Y.Z…` or `Couldn't check
    for updates`) and a `Check now` button (one check at once, disabled
    while one runs, downloads or installs; it works with the toggle off too), then a
    `What's New` row (*The changes in each version of Rift*) whose `Open`
    button opens What's New.
  - Automation: `Control socket` segmented `Off | Navigate | Full`
    (*Lets scripts control Rift (rift-terminal ctl); Full also lets them
    type*; `control_api`).
- **Profiles** (launch icon) — *The shells and programs tabs open with. Drag
  a row (or ⌥↑ ⌥↓) to reorder.*
  - New tabs: `Default profile` select (`Shell path`, then every profile;
    `default_profile`), `Add installed shells as profiles` toggle (*Each
    shell in /etc/shells, once; a deleted one stays deleted*;
    `discover_shells`; turned on, the shells not offered yet are added at
    once).
  - Your profiles: one row per `[[profiles]]` entry in file order — a grip,
    the name over what it runs (command and arguments, or `Shell path`),
    tags left of the button (`Default` in accent, `Detected` for a shell
    from /etc/shells, `Hidden`, `Not found` in error for a detected shell
    that is gone) and `Edit` (`Done` while open). A click on the row (or
    Edit) opens its form under it; a drag moves it (a 2 px accent caret
    shows where it lands); ⌥↑ / ⌥↓ move the focused row or Edit button.
  - The form: `Name`, `Command` (a detected profile's is shown, not
    editable: *Duplicate to change the command*), `Arguments`, `Directory`,
    `Environment` (`NAME=value …`) and `Tab title` text fields; `Tab color`
    (`None` and the six tab colours), `Icon` (`Terminal` and the Nerd Font
    marks: folder, git, python, php, github, docker, user, laptop) and
    `Colour scheme` (`Window theme` and every theme) selects; then `Set as
    default` (disabled on the default one), `Duplicate` (`<name> copy`,
    the user's own: no `source`), `Hide` / `Show` and a destructive
    `Delete`. A change that `profiles::validate` refuses (a name used
    twice, an empty command) is not written and says why in red above the
    buttons. Renaming the default profile keeps it the default.
  - `Delete` asks first: *Delete <name>?* with what it runs; a detected
    profile *Rift found this shell installed and won't add it again. Hide
    keeps it in the file instead.* (its shell joins the seen list), any
    other *It is removed from config.toml.*; deleting the default one adds
    *New tabs then run the Shell path.* and clears `default_profile`.
  - `New profile` → `Add`: `New profile` (then `New profile 2`, …) running
    the login shell, its form open.
  - Every change writes config.toml in place: every entry keeps its
    comments, which move with it through a reorder or a rename.
  - Another window's Settings, or the file changing on disk, reorders or
    removes profiles: an open form follows its profile (closed, draft
    dropped, when it is gone).
  - While config.toml does not load, the list is not the file's: Add, the
    form's buttons, a field and a move are refused (*Settings not saved:
    fix config.toml first*).
- **Appearance** — *Fonts, cursor, window and theme*.
  - Font: `Font family` select (JetBrains Mono first, then the embedded
    Hack and the installed fixed-width families, each shown in itself),
    `Font size` stepper (px, 8–28), `Line height` slider (1.0–2.2, default
    1.2), `Font weight` and `Bold font weight` steppers (±100 within
    100–900), `Bold text in bright colors` toggle (`bold_is_bright`),
    `Minimum contrast` stepper (±0.5 within 1–21, `Off` at 1;
    `minimum_contrast`), `Font features` text field (`font_features`, e.g.
    `ss01 zero -calt`; an unknown feature is refused),
    `Ambiguous-width characters` segmented `Narrow | Wide`
    (`ambiguous_width`).
  - Cursor: `Cursor style` segmented `Block | Bar | Underline | Hollow |
    Double`, `Hollow when unfocused` toggle (*Outline the cursor while its
    window or pane is not focused*; `cursor_hollow_unfocused`),
    `Cursor blink` toggle (*Pulse the cursor when the editor is focused*).
  - Window & theme: `Active theme` select (background swatch ringed in the
    theme's accent + display name), `Sync with OS appearance` toggle,
    `Light theme (OS sync)` select (same swatch style), `Window opacity`
    slider (1–100 %, value boxed right of the track), `Window blur radius`
    slider (1–64 px, macOS only),
    `Window padding` stepper (px, ±2: the left and right padding in
    effect; a step or its reset sets `window_padding` and removes
    `window_padding_left`/`_right`), `Top padding` and `Bottom padding`
    steppers (px, ±2 within 0–40; `window_padding_top` / `_bottom`), `Pad
    full-screen programs` toggle (`alt_screen_padding`), `App icon` select (a picture of
    each colourway before its name; `app_icon`), `Font ligatures` toggle (*Render -> >=
    != as combined glyphs*).
  - New windows: `Open new windows` segmented `As saved | Maximized | Full
    screen` (`launch_mode`), `Window width` / `Window height` steppers (px,
    ±20; *Without a saved size*), `Keep windows on top`
    (`always_on_top`), `Resize in whole cells` (`snap_to_cells`) and `Show
    the size while resizing` (`resize_overlay`) toggles.
  - Tabs: `Tab position` segmented `Top | Side` (`tab_layout`), `Tab style`
    (*How a tab shows what its panes are doing*; three picture
    cards in a row, each two small tabs drawn in that style — one needing
    you, one done — over a radio, its name and a line: **Dot** (tagged
    *Default*) *A dot by the title*, **Underline** *The title
    underlined*, **Detailed** *The state in words*; a click or Space/Enter
    on a focused card picks it; `tab_style`), `Close
    buttons` segmented `On hover | Always | Never` (*On hover shows × only on
    the tab under the pointer*; `tab_close_button`), `Close button side`
    segmented `Right | Left` (`tab_close_side`), `Show the tab bar` segmented
    `Always | When windowed | Only on hover` (*Out of sight, it slides in at the
    window's top edge*; `tab_bar`), `New tabs open`
    segmented `At the end | After the current tab` (`new_tab_placement`),
    `⌃Tab switches` segmented `In order | Last used first`
    (`tab_switch_order`),
    `Show tabs with only one tab open` toggle (*Off hides the lone tab; the
    bar's buttons stay*; `tab_bar_with_one_tab`).
  - Toolbar: `Toolbar buttons` (*Click to choose and arrange them*;
    `toolbar_left` / `toolbar_right`, one changed dot and ↺ for both): a
    preview of the strip — traffic lights, the left buttons, two sample
    tabs, the right buttons — with `✎ Edit` at its right end (brighter on
    hover, like the box's accent border); a click opens **Edit toolbar**.
  - Panes: `Dim inactive panes` toggle (`dim_inactive_panes`), `Dimming
    strength` slider (1–40 %, shown while dimming is on;
    `inactive_pane_dim`), `Focus follows mouse` toggle (*The split pane under
    the pointer takes focus*; `focus_follows_mouse`).
    Font size and line height show the applied (clamped) value. A theme file
    that does not load is listed as `<name> (invalid theme file)`, choosing it
    toasts why and Dark Default is used; themes sharing a display name carry
    their file name.
- **Input** — *The command line and blocks*.
  - Input: `Vim keybindings` toggle, `Vim's unnamed register` segmented
    `Rift only | System clipboard` (`vim_clipboard`), `⌃R, ⌃T and ⌥C run`
    segmented `The shell's | Rift's` (`shell_widgets`),
    `Autosuggestions (ghost text)` toggle, `Tab key` segmented
    `Completions | Ghost text` (`tab_behavior`), `Show the accept-key hint`
    toggle (`autosuggestion_hint`), `Show the ignore button` toggle
    (`autosuggestion_ignore_button`), `Open completions as you
    type` toggle (`completions_open_while_typing`), `Completions from your
    shell` toggle
    (`shell_completions`), `Autocomplete quotes and brackets` toggle,
    `Input position` segmented `Bottom | Top`, `Blocks start` segmented
    `Bottom | Top`, `Sticky command header` toggle (*Pins a long block's
    command to the top while you scroll its output*;
    `sticky_command_header`), `Block spacing` segmented `Compact | Normal`
    (*Normal leaves a blank row above each block*; `block_spacing`), `Show
    block dividers` toggle (`block_dividers`), `Jump to bottom of block
    button` toggle (*Over a block that runs past the bottom of the pane*;
    `jump_to_bottom_button`), `Prompt` segmented
    `Rift | Shell (PS1)`, `Prompt chips` (a preview of the chips with
    sample values and `✎ Edit` at its right end; a click opens **Edit
    prompt**; changed dot and ↺ like other rows), `Time on this tab counts`
    segmented `Since first command | Only while running`, `Expand aliases as you type`, `Underline unknown
    commands`, `Suggest corrections for failed commands` toggles, `Offer Rift
    in subshells` toggle (*docker exec … bash, poetry shell, and the
    commands below*; `subshell_offer`), `More subshell commands` text field
    (regular expressions; one that does not compile is refused;
    `subshell_commands`), `Never offer it for` text field
    (`subshell_denylist`), `Rich input sends with`
    segmented `⏎ | ⌘⏎` (`rich_input_submit`), `Open rich input when an agent
    waits` toggle (*Claude Code, Codex, OpenCode, Gemini CLI at their
    prompt*; `rich_input_auto`).
- **Terminal** (copy icon) — *Mouse, clipboard and keys inside the
  terminal*.
  - Mouse: `Mouse reporting to programs (Shift selects)` toggle,
    `Middle-click to paste` toggle (*Pastes the clipboard into the pane under
    the pointer*; `middle_click_paste`), `Clicking a link` segmented
    `Show it | Open it` (*Show where it goes first, or open it at once
    (⌘-click always opens)*; `link_click`), `Lines per wheel notch` stepper
    (1–20, ±1; `wheel_scroll_lines`), `⌘-scroll zooms the font` (Linux:
    `Ctrl-scroll`; `wheel_zoom`) toggle, `Word characters` and `Word
    delimiters` text fields (`word_chars`, `word_delimiters`; kept as typed).
  - Clipboard: `Copy on select` toggle, `Ask before pasting lines that would
    run` toggle, `Programs' clipboard access (OSC 52)` segmented
    `Off | Copy | Copy and read` (*Read lets any program or ssh host see your
    clipboard*; `clipboard_access`), `Ask before pasting more than` stepper
    (`Off`, then 16 KiB doubling; `large_paste_warning`), `Trim trailing
    blanks from pastes` toggle (`trim_paste`).
  - Panes: `Scrollbar` segmented `Auto | Always | Never` (`scrollbar`),
    `When its program exits` segmented `Close | Close on success | Keep`
    (`close_on_exit`), `Mask secrets` (`mask_secrets`) and `Show programs'
    progress` (`show_progress`) toggles.
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
- **Notifications** (inbox icon) — *Toasts, desktop notifications and sounds;
  everything also lands in the inbox*.
  - Alerts: `Notifications` master toggle, `In-window toasts` toggle,
    `Toast duration` stepper (s, 2–60), `Bounce the Dock icon` (Linux: `Mark
    the window urgent`; `request_attention`) toggle.
  - Desktop notifications: Rift's icon beside `macOS permission` with a
    status pill (`Allowed`, `Off`, `Not asked yet`) and `Allow…` (not asked
    yet: macOS asks) or `Open System Settings` (its Notifications page for
    Rift); read again when Rift comes back to the front. Outside an app
    bundle it says banners come from Script Editor; on Linux the row is
    `Desktop notifications` (notify-send). `Banners show` explains the
    title / subtitle / body.
  - Notify me when: `A long command finishes` toggle, `Long command
    threshold` stepper (s, ±5 within 0–3600), `A command fails`, `An agent finishes its
    task`, `A program or agent needs attention`, `A password prompt appears`
    toggles.
  - Sounds: `Play notification sounds` toggle; `Sound`, `Finished`, `Failed`,
    `Needs attention` selects — the list holds `Default`, `None`, then every
    sound file found, and picking one plays it; the per-event ones start at
    `Same as Sound`. ▷ before each select plays its sound without opening
    the list (also with sounds off); `None`, or a file that is gone, says so
    in a toast.
  - Try it: `In-app toast` and `Desktop notification` rows, each with
    `Finished`, `Failed` and `Needs attention` buttons: a sample alert shown
    the real way (the desktop one also while Rift is in front), with its
    sound while sounds are on; it lands in the inbox read, and its Rerun
    only says it works.
  - Terminal bell: `Bell` segmented `Visual | Audible | Both | None`
    (Visual flashes the whole window, Audible plays the system's Tink sound,
    whatever the sounds above say); `Try the bell` → `Ring` rings it as set.
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
  YAML)*: an `All N | Dark N | Light N` filter (a theme file that does not
  load counts under All only) with `Active: <name>` at its right, then a
  grid of cards (three across, two or one in a narrow body), one per theme:
  a small Rift window in that theme's own colours — the chrome strip with
  three dots, an active tab (accent top edge) and an inactive one, a passing
  block (`cargo test`, success edge, accent command), a failing one (`git
  push`, error edge) and the prompt's chevron with a cursor in the theme's
  cursor colour — then its name and a `Dark` / `Light` tag. The active theme
  has an accent ring and a check; a file that does not load draws `This file
  does not load` in the error colour and an `Invalid` tag (its accessible
  name says `(invalid theme file)`); the `theme_light` theme carries a `light
  mode` tag while `sync_os_theme` is on. Clicking a card sets `theme`; with
  a card focused (Tab), ← → move to the previous / next card, ↑ ↓ to the
  nearest card in the row above / below, and Space or Enter applies it. The
  last card, dashed, is `Add your own`: *Drop a .toml or a Warp .yaml theme
  into the folder; it shows here at once.* and `Open themes folder`, which
  makes the folder when it is missing and opens it with the system opener.
  The palette's *Theme picker (live preview)* (`theme_picker`) lists the
  same themes (the one in use tagged *in use*) and applies the selected row
  live; ⏎ keeps it, Esc puts the previous theme back.
- **Remote access** (open icon) — *Watch and run commands from a paired phone, at home or away.*:
  - A card at the top: a phone icon, a title and a line, and the `Allow
    remote access` switch (`remote_access`; changed dot and ↺ beside it).
    Off: *Remote access is off — Turn it on to watch and run commands from
    your phone. Until then nothing listens and no phone can reach this
    computer.* On: *Ready for your phone* (or *Starting remote access…*,
    *Not on a network*, *N phones connected*), then two tiles — `On this
    Wi-Fi` (the listening addresses; `Starting…`, or `Listening, but on no
    network`) and `Away from home` (the relay's host and its ping round trip
    as a tag: `Registering this computer at the relay…`, `Phones away from
    home connect through it`, `No relay set: phones connect only on this
    Wi-Fi`, or `Can’t reach the relay, so phones away from home can’t
    connect. Retrying in N s.` with the whole error wrapped under it, never
    cut off, and a `Try again` button that ends the retry wait at once;
    with the Cloudflare tunnel, `Waiting for the tunnel` until it is ready
    and then the tunnel relay's state) — and a lock line: `End-to-end
    encrypted: the relay passes bytes it cannot read.`
  - Off, nothing else shows but `Paired phones (N)` when there are any: *They
    can't connect while remote access is off. Revoke one to remove it for
    good.*, each phone's name, `Can’t connect while off · <platform> ·
    paired <date>` and `Revoke`.
  - `Pair a phone` (on): `Add a phone` — *Show a code to scan with the Rift
    app · one use, valid 2 minutes* — with the `Pair a phone` button, and
    the check box `The new phone can only watch` (*It sees your tabs but
    can't run commands or send keys*; changing it withdraws a shown code).
    Pairing shows the code inline in an accent-edged panel: the QR code
    (black 4 px modules on a white plate with a 4-module quiet zone, so phone
    cameras read it in any theme), `Scan with the Rift app` and how (*It will
    be able to run commands.* / *It will only be able to watch.*), `Expires
    in m:ss`, `Copy link` and `Cancel` (which withdraws the code: it stops
    working at once), and the wrapped link. At zero, `The code expired` with
    `Show a new code`. While the gateway has no address yet (or the tunnel
    is not ready), a `Pairing` row waits for it.
  - `Paired phones (N)` (on): per phone its name with a dot (green while
    connected) and `Connected on this network|Connected through the
    relay|Not connected · last seen … · platform (iOS, Android) · paired
    <date>`, a `Can run commands` / `View only` segmented control (raising
    a phone to run commands asks first), and an outlined `Revoke` (error
    outline and label), which opens the confirm dialog `Revoke <name>? It
    disconnects now and cannot reconnect without pairing again.`; `No phones
    yet` when none.
  - `When you quit` (on): what a quit does, in full: *Your shells end; the
    menu bar icon stays — Phones that can run commands can still open tabs.
    Quit Rift completely from the menu bar icon.* (macOS), *Rift quits —
    Keep shells running through updates is off in General, so nothing stays
    behind.*, or *Rift quits — Phones can no longer reach it until you open
    Rift again.* (Linux).
  - `Recent connections (N)` (on; folded away, a click opens it): the newest
    20, each with the device (or `Unknown device`), `LAN` or `Relay`, the
    address, the start time, the duration or `connected`, and why it ended.
    Five rows show at once; with more, the wheel over the list scrolls it a
    row at a time (a thumb at its right edge shows where), and past either
    end the page scrolls on.
  - `Connection settings` (on; folded away): `Listen address` — a text field
    for `remote_listen` (`IP:PORT`; anything else is refused); `Relay
    address` — a text field for `remote_relay` (click or Enter edits; typing
    and ⌘V fill it; Enter saves a valid `ws://`/`wss://` URL, an invalid one
    stays with a toast; Esc cancels; Tab or a click elsewhere saves; Rift's
    public relay by default; not shown while the tunnel replaces it); `Use a
    Cloudflare tunnel instead` — the `remote_tunnel` switch; on, a checklist
    follows: `cloudflared` (`Installed · <path>` with a green check, or `Not
    installed` with an `Install` button when Homebrew is present, else
    `Download`) and `Tunnel` (`Starting…`, `Waiting for cloudflared`, `Ready
    · <host> · pair phones again after a restart` or `Failed, retrying:
    <reason>`).
  - The page refreshes its data at most once a second.
  - Phones draw with this computer's theme (Settings → Themes, light or dark
    as the system is) unless the phone chooses its own in the app's
    *Settings › This phone › Appearance*: *Same as the computer* (default),
    *Match this phone* (Light or Dark Default) or *Choose a theme* (a radio
    list of the builtin themes, each with a swatch of its ground and accent,
    the chosen one ticked). The phone's choice never changes the computer.
- **Permissions** (macOS only; lock icon, between Remote access and About) —
  *macOS privacy switches Rift can use. Status refreshes when you return from
  System Settings.* A checklist, each row with a status pill (`Granted` green,
  `Not granted` red, neutral otherwise) and an `Open Settings` button into the
  System Settings pane:
  - `Recommended` › `Full Disk Access` — *Desktop, Documents, Downloads, iCloud
    Drive, external and network volumes*; its button opens Privacy & Security ›
    Full Disk Access. Under it, one indented row per app that needs the
    switch, the app that runs the shells first. With `durable_sessions` the
    shells run in the session service, so there are two: `Rift Helper` (*Runs
    your shells*; on an installed bundle the service is Rift Helper.app, which
    macOS treats as its own app, so the switch for `Rift` alone does not help
    the shells; without the helper bundle the row reads `Rift session
    service`) and `Rift` (*This window: file explorer, editor, drops*). With
    shells in the window process (`durable_sessions = false`) there is one,
    `Rift` (*Runs your shells and this window*). A row whose switch is off adds
    *· switch it on, or add it with +*; the `Rift Helper` row then also has
    `Show in Finder`, which reveals Rift Helper.app (inside
    `Rift.app/Contents/Library/LoginItems`) to drag into the list, since
    macOS lists an app there only once it was added. Pill words: `Checking…`, `Granted`, `Not
    granted`, `Unknown` (the app could not tell), `Unknown · restart Rift` (the
    running session service is older than this Rift and cannot be asked; once
    it runs no shell, the next shell replaces it with this release's) and
    `Not running` (no session service).
  - `Optional` › `Notifications` — *Banners when a command finishes or an agent
    needs you*; the pill follows the system's notification permission
    (`Granted`, `Not granted`, `Not asked yet`, or `Unknown · not running as
    Rift.app`); the button opens System Settings › Notifications on Rift's
    page, or reads `Allow…` and asks macOS while it has not been asked.
  - Full Disk Access is read by opening a TCC database for reading (the
    system one, then the user one: a Mac may have no user one), which
    succeeds only with the grant and never prompts; the window probes
    itself, the session service answers for itself over its socket. The page
    reads when it opens and when the window regains focus while it is shown;
    the service is asked off the UI thread. Settings search finds its rows
    (`disk`, `downloads`). A headless run's `Open Settings` and `Show in
    Finder` only log. Rift does not need, and the page does not list, Accessibility,
    Screen Recording, Automation, Local Network or Developer Tools.
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
- **Scrollbar** (`scrollbar = "auto" | "always" | "never"`): a 6 px pill
  thumb 1 px inside the pane's right edge, in the padding beside the text
  (never over it; the grid does not move), as tall as the rows in view. The
  thumb (`text_muted` at 45 %, `text_dim` at 70 % with a 6 % `fg` track while
  under the pointer or dragged) is the view over the pane's blocks as drawn:
  collapsed output and a block filter shrink the list. 2 px ticks across it
  mark prompts (`text_dim`), find matches (`search_match`), failed commands
  (`error`) and bookmarks (`accent`), drawn in that order. Drag the thumb to
  scroll; a click above or below it pages one view. `auto` shows it for
  1.2 s after the pane scrolls and while the pointer is over the right
  padding; nothing shows on the alternate screen or when nothing scrolls.
- `clear` performs a Warp-style soft clear: the viewport empties, old blocks
  stay reachable by scrolling up, and the next block starts right above the
  command line (at the top of the viewport with `blocks_start = "top"`). ⌘K is the destructive variant (blocks +
  scrollback wiped).
