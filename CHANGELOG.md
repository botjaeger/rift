# Changelog

User-facing changes per release, newest first. The in-app What's New page
(Help → What's New, or `whats_new` in the palette) renders this file, so every
entry is written for someone using Rift, not for someone building it.

Format: `## <version> — <YYYY-MM-DD>`, an optional one-paragraph summary, then
`### New`, `### Improved` or `### Fixed` sections of `- **Title** — text`
items.

## 0.19.0 — 2026-10-19

config.toml is now the one place your settings live: change something in Settings and it shows in the
file at once, edit the file and Settings follows.

### New

- **Your shells as profiles** — the shells installed on your computer are added to config.toml as
  profiles, and new ones are added when you install them. Hide one to keep it out of the new-tab menu;
  delete one and it stays deleted. A shell you uninstall stays in the file but leaves the menu. On a
  first start, new tabs open your login shell.
- **Settings › Profiles** — add, edit, duplicate, hide, delete and reorder profiles, and pick the
  default one, without opening the file.
- **Edit config.toml beside Settings** — type in the file pane next to Settings. Mistakes are marked as
  you type (unknown settings, with a suggestion for a misspelt one, wrong values, broken lines), and
  Save turns on only when the file is valid; ⌘S saves, Dismiss puts the file back. If the file changes
  while you are editing, you choose to reload it or keep your edits, and Rift asks before closing or
  quitting with unsaved changes.

### Improved

- **Every setting in Settings** — about forty settings that were only in config.toml now have a row in
  Settings, among them *Keep shells running through updates*, *Ask before keeping Rift running*, the
  shell path, font weights, window size and padding, paste trimming and word characters.
- **Your comments stay** — saving from Settings keeps comments inside profiles and other lists, and
  whole numbers you wrote stay whole numbers. Before Rift first rewrites a file you wrote, it keeps a
  copy as `config.toml.bak`.

## 0.18.0 — 2026-10-19

Keeping Rift running after you quit now follows Remote access, so there is one switch instead of three.

### Improved

- **Keep Rift running with Remote access** — with Remote access on (macOS), the first ⌘Q asks *Keep
  running?*: your shells, the phone connection and the menu bar icon stay, and opening Rift again brings
  back the same tabs. With Remote access off (the default), ⌘Q quits everything, as before. *Allow
  background processes*, *Ask before keeping Rift running* and *Keep shells running through updates*
  are gone from Settings; shells still survive updates and crashes, and the quit question can be turned
  back on with `confirm_background_quit = true` in config.toml.
- **After you quit** — Settings › Remote access says what a quit does.

### Fixed

- **Keep running only when it can be undone** — the question is offered only while the phone connection
  is running, so a quit never leaves shells behind with no menu bar icon to reopen or end them.
- **Menu bar icon after a normal quit** — it no longer shows for about 30 seconds after a quit that
  ended everything.
- **Remote access turned off while Rift is kept** — the kept shells end instead of being left running.

## 0.17.0 — 2026-10-19

Opening, quitting and restarting Rift is steadier: no more empty Dock icons or "quit unexpectedly" pop-ups, and quitting can leave your shells running.

### New

- **Keep Rift running when you quit** — with *Allow background processes* and *Remote access* on (macOS),
  the first ⌘Q asks *Keep running?* with a *Don't ask again* box. Your shells keep going, the menu bar
  icon stays, and opening Rift again brings back the same tabs. *Quit Rift Completely* in the menu bar
  icon ends everything.
- **Tabs come back after a crash** — if Rift did not quit normally, the next launch restores your tabs
  and says so.
- **Phones are told why Rift closed** — a quit shows "Rift quit on this computer"; an update shows
  "Updating Rift, back in about a minute" instead of an error.

### Fixed

- **Empty Dock icon** — opening Rift after quitting it always shows a window; Rift's background helpers
  are now a separate app (*Rift Helper*), so a launch can no longer reach one with no window.
- **"rift-terminal quit unexpectedly"** — the pop-ups that appeared the first time Rift started after an
  update no longer happen.

## 0.16.0 — 2026-10-19

Rift's helpers can keep running after you quit, with a menu bar icon to show it, and two bugs fixed.

### New

- **Allow background processes** — Settings › General › App. On, Rift's helpers keep running after you
  quit, so paired phones can open tabs, and (macOS) a Rift icon shows in the menu bar while they do.
  It replaces *Show menu bar icon* and *Phones can open tabs while Rift is closed*, and is off by
  default: turn it on to keep the icon. A `remote_when_closed = true` you already had still counts.

### Fixed

- **Programs start at the right size** — a program started from the command line, such as `less`,
  now reads the size it will run at, so its first screen no longer leaves blank rows at the top and
  copying in mark mode no longer selects blanks.
- **A damaged saved date** — a transcript with a corrupt date no longer crashes the history list or
  the search over past results.

## 0.15.0 — 2026-10-18

Profiles and a new-tab menu, your own actions, tab groups and a searchable side rail, filtering,
comparing and exporting block output, secret masking, and a long list of appearance and window options.

### New

- **Profiles and the new-tab menu** — a ⌄ next to the new-tab button lists your profiles, SSH hosts
  and launch configurations; pick one to open it in a tab, a split or a window.
- **Your own actions** — define named actions of several steps (keys, commands, Rift actions) in
  `[[actions]]`; they appear in the palette and can have a shortcut.
- **Tab groups, pinned tabs and a side rail** — group, colour and pin tabs; the rail lists every pane
  with its folder, branch and changes, and searches them. Tab colours take any hex, can cycle, and can
  follow the folder (`[[tab_color_rules]]`).
- **Kept exited panes** — a pane whose shell ended stays with its output and a Restart button.
- **Read-only panes** — mark a pane so nothing can be typed into it.
- **Work with block output** — filter a block (regex, case, invert, context lines), find inside one
  block, select several blocks, compare two blocks side by side, copy as HTML or Markdown, save
  output to a file or export the whole buffer.
- **Search past output** — the Results tab in ⌃R searches earlier output by text, status, date, host
  and folder.
- **Secret masking** — with `mask_secrets` on, keys and tokens are hidden in the screen, copies,
  exports, comparisons, search results, notifications and the side rail.
- **Review: commit and push** — stage, commit and push from the review panel.
- **Send to an agent, composer and progress** — send a block or selection to another pane running an
  agent, write long prompts in a composer, and see a command's progress in the tab.
- **Install the `rift` command** — one click puts the command-line tool on your PATH.
- **Themes** — nine more built-in themes (Dracula, Nord, Gruvbox, Solarized, One Dark, Monokai,
  Catppuccin Latte), a theme picker that previews as you move and reverts on Escape, a colour scheme
  per pane, and Warp gradient themes.
- **Fonts and drawing** — font weight, OpenType features (ss01, zero), a minimum contrast ratio,
  bold-is-bright, and box drawing, Powerline and block graphics drawn pixel-exact.
- **Cursor shapes** — hollow block, double underline, and a hollow cursor in panes without focus.
- **Window options** — padding per side, a drop-down window from any edge with its own width and
  screen, Dock bounce on a bell, a warning before huge pastes, trim paste, word delimiters, a size
  overlay while resizing, snap to cells, ⌘-wheel zoom, opacity actions, default size and launch mode,
  always on top, and named windows.
- **Smarter selection and editing** — double-click picks a whole URL, path, email, address or hash;
  more file-link formats with a right-click menu; ⌥. inserts the last argument; ⌃U copies what it
  clears; vim registers share the clipboard.
- **Markdown viewer** — run a shell block after confirming, Mermaid blocks labelled as source, and
  *Open in viewer* after `cat file.md`. The file explorer's right-click adds copy path, copy relative
  path, New file and Reveal.
- **Scroll bar and sticky header** — a scroll bar with marks for blocks and matches; the sticky
  header can be minimised per pane.
- **`RIFT_PANE_URL` links** — a link that brings you back to the pane that made it.

### Improved

- **Closing and quitting** — Rift no longer hangs on quit when a shell ignores the hang-up, and no
  longer sends a signal to an unrelated process that reused a closed shell's number.
- **One bad setting no longer breaks the file** — a wrong value in `config.toml` falls back by itself
  and the rest still applies.
- **Vim keys** — `U` undoes the line and ⌃R redoes outside insert mode.

### Fixed

- **Pushing to a differently named upstream**, commands typed into a read-only pane by an action, and
  several places where masked secrets could still show are fixed.

## 0.14.0 — 2026-10-18

Links that open Rift, a toolbar you arrange, app icon colours, several carets, a command inspector,
background job output in its own blocks, and Rift inside subshells.

### New

- **`rift://` links** — Finder, Raycast, an editor or a README can open a tab or window in a folder,
  a launch configuration or Tab Config by name, or Settings with a search filled in. A layout that
  would run commands asks first.
- **Arrange the toolbar** — the file explorer, code review, notifications, command palette and
  Settings buttons can sit left of the tabs or at the right end, in your order, or be hidden.
  Right-click the toolbar → *Rearrange toolbar items*.
- **App icon colours** — Default, Light, Mono, Ocean, Ember, Forest or Neon, in Settings ›
  Appearance; the Dock shows your choice.
- **Menu bar icon** — Rift's icon in the menu bar; turn it off in Settings › General.
- **Multiple carets** — ⌃⇧↑ / ⌃⇧↓ add a caret above or below, ⌃G adds the next occurrence of the
  word, ⌘-click adds a caret where you click; typing, deleting, moving and pasting act at each.
- **Command inspector** — rest the pointer on part of the command line, or press ⇧⌘I, to see
  whether it is a command, subcommand, option, argument or operator and what it does.
- **Background job output** — what a background job prints while the prompt waits goes into its own
  block between commands, instead of under the prompt.
- **Rift in subshells** — when you start a Docker shell, an SSH session or another subshell Rift
  knows, a bar offers to bring blocks and completions into it (⌘⏎), or to never ask again for that
  command.
- **More block and tab settings** — space between blocks, divider lines, a jump-to-bottom button,
  the tab close button on the left, and a tab bar that hides until you reach the window's top edge.

### Improved

- **Code review from the keyboard** — ⌥⌘G puts the keys in the review panel.
- **Completions for bash builtins** — signals, variables, aliases, jobs and options complete even
  without bash-completion installed.

### Fixed

- **"Running in Background"** — Rift's session service and phone gateway no longer leave a
  separate background entry in the Dock on macOS 27; they show in Login Items as Rift's.
- **`bash -o pipefail -c` commands** — commands started this way are recognized for
  notifications again.

## 0.13.0 — 2026-10-17

Completions from your own shell and from remote machines, several windows in one Rift, a drop-down
terminal, a fuller vim mode, and panes that size like Warp's.

### New

- **Completions from your shell** — the completion menu now also offers what your own zsh, bash or
  fish would complete (your plugins and completion functions included), for tools Rift has no
  completions for.
- **Completions over SSH** — in a pane opened from an SSH profile, completions list the remote
  machine's files, commands and git branches, over the connection that is already open.
- **Drop-down terminal** — a hotkey window that slides down from the top of the screen above other
  apps, on every Space, and can hide when you click away.
- **Windows in one Rift** — new windows open inside the running Rift, instantly and with less
  memory; every window comes back after a restart or an update.
- **Vim visual mode and more** — `v` and `V` selections, `.` to repeat, `/` `?` `n` `N` search,
  `%`, `gg` `G`, `o` `O`, `J`, `R`, case changes and more in the command editor.
- **Warp Tab Configs** — Warp's TOML tab configs open from the palette, and *Save layout* writes
  that format; YAML launch configs still load.
- **Block and tab settings** — sticky command headers, dimming of inactive panes, the tab close
  button, where new tabs open, the tab bar with one tab, and focus follows mouse, all in Settings.

### Improved

- **Faster, smarter completions** — results are ranked (prefix first, then word starts, then
  recent), and files, git branches and SSH hosts are no longer re-read on every key.
- **Pane sizes** — splitting, closing and resizing panes divide space like Warp: a closed pane's
  space goes to its neighbours, ⌃⌘ arrows move a divider, panes keep a minimum size, and
  *Equalize panes* evens them out.
- **⌘ arrows in the command editor** — ⌘← and ⌘→ go to the start and end of the line, ⌘↑ and
  ⌘↓ to the start and end of a multi-line command; held keys repeat.
- **Review panel and prompt chips follow the pane** — with panes in different folders, the code
  review panel and the git and Docker chips show the focused pane's project, and they update while
  an agent or an editor changes files, without waiting for a command to finish.

### Fixed

- **Remote folders** — a pane on another machine no longer completes or shows git and Docker
  status from a folder of the same name on this Mac.
- **Restored focus** — after a restart, the pane that had focus has it again.

## 0.12.0 — 2026-10-17

Richer desktop notifications, a way to test every alert, and a smoother terminal under load.

### New

- **Detailed desktop notifications** — on macOS a finished or failed command now notifies with
  Rift's icon, the outcome, how long it ran, and the tab, folder and branch it ran in. Click it to
  jump to the pane; a failed command also offers **Rerun**.
- **Test your notifications** — Settings › Notifications shows whether macOS allows Rift's
  notifications (with a button to allow them), a play button beside each sound, and buttons that
  send a test toast, a test desktop notification and a test bell.

### Improved

- **New defaults** — settings you have not changed now use Hack, a translucent blurred window,
  copy on select, a visual and audible bell, links that open on click (files in Rift's editor),
  and prompt chips for elapsed time and Docker.
- **Pane dividers** — the divider between panes is translucent, so it reads the same at any
  window opacity.
- **Command headers fit** — a long command or folder in a block header no longer runs into its
  status and buttons.

### Fixed

- **Heavy input no longer freezes the window** — pasting or typing a lot into a busy program
  no longer stalls Rift while the program catches up.
- **Startup with a stuck session service** — if the background session service stops answering,
  Rift opens normal shells within a few seconds instead of hanging.
- **Completing commands with `=` in the name** — accepting such a completion no longer changes
  the shell's variables.
- **Regex find with accented text** — regex search now matches accented text as it appears.
- **Toasts with Settings open** — in-app toasts now show while Settings is open.

## 0.11.1 — 2026-10-16

### Improved

- **Clearer pane dividers** — the line between split panes is a little brighter, as in Warp, so
  panes stay apart at a glance.

### Fixed

- **New panes open in the right folder after an update** — splitting a pane, or opening a tab,
  from a pane that was busy through an update (an agent, a server) opened in your home folder.
  It now opens in that pane's folder.

## 0.11.0 — 2026-10-16

Updates that are ready before they ask, quieter notices, and clearer panes.

### New

- **Updates download first** — Rift fetches and checks a new version in the background, then
  tells you once it is ready to install, with a notice in the notification center. No more
  "checking" and "downloading" pop-ups.
- **Notices beside your prompt** — short messages now appear as small chips in the pane's chip
  row: done, error, info and phone. Errors stay until you close them, and a config error offers
  *Open config.toml*.

### Improved

- **Active pane stands out** — panes you are not typing in are slightly lighter, so the active
  one is easy to spot.
- **Settings opens on General** — and starts there again after you close it; switching to another
  tab and back keeps your page.
- **About shows the Rift icon** — the About and welcome pages use the app's own icon.
- **Recent connections** — Remote access lists the newest first, five at a time, and scrolls with
  the wheel or the arrow keys.

### Fixed

- **Update leftovers** — installing an update no longer leaves temporary files behind.
- **Stale config errors** — a config error notice now goes away once the file is fixed.

## 0.10.0 — 2026-10-16

Every prompt chip on your phone, with its menu.

### New

- **Every prompt chip, on the phone** — a paired phone's command line now shows the chips Rift
  draws above yours (folder, branch, changes, pull request, Python, PHP, Docker, time on tab,
  user, host, date and time), in your order and with the same colours. A Docker container that is
  down or unhealthy is marked.
- **Chip menus on the phone** — the pull request chip opens the PR in the phone's browser, Python
  and PHP list the installed versions and switch the shell like Rift's menu, Docker lists the
  containers with ports, uptime and health and follows a service's logs, and the time on tab chip
  has Reset.

## 0.9.0 — 2026-10-16

A time log on each tab, a clearer Remote access page, and selections that scroll.

### New

- **Time log on the tab** — set *Time on this tab counts* to *Only while running* (Settings ›
  Input, or `elapsed_counts = "running"`) and the tab's time chip counts only while a command
  runs in it. Panes that run at the same time count once, a coding agent counts until it exits,
  and the total is kept across an update's restart, a crash and a reboot (quitting starts
  fresh, like the tabs). The hourglass dims while it is paused.
- **Reset the time** — the chip's hover card now stays open while you move into it and has a
  **Reset** button; the palette has *Reset time on this tab*.

### Improved

- **Remote access, at a glance** — Settings › Remote access opens with *Connection*: this
  network's address and the relay's state, with its delay while connected. When the relay
  fails you see the whole error and a *Try again* button that retries at once. The relay
  address and the Cloudflare tunnel moved under *Relay settings…*, and each paired phone says
  whether it is connected on this network or through the relay.

### Fixed

- **Selecting past the edge** — dragging a selection above or below a pane now scrolls it,
  faster the farther you go, so you can select more than one screen of output.

## 0.8.0 — 2026-10-16

Warp's colours, a fresh start after you quit, and more from your phone.

### New

- **Warp's colours** — the default dark and light themes now use Warp's Dark and Light
  colours: the near-black ground, white text, the cyan accent and Warp's program colours.
- **Block menu button** — hovering a command's header shows a ⋮ button next to copy; it
  opens the same menu as a right-click.
- **Paste copied files** — files copied in Finder paste as their full paths. Claude Code and
  Codex attach a pasted image path as an image.
- **Close from your phone** — the phone can close a pane or a whole tab. Closing your last
  tab from the phone leaves a fresh one, so the phone never quits Rift.

### Improved

- **Quitting starts fresh** — quitting Rift ends every shell, and the next launch opens one
  new tab where the window was. An update, a crash or a restart of the Mac still bring your
  tabs and running shells back. While a phone is connected, quitting asks first.
- **Your phone sees Rift's look** — the phone's terminal view uses your theme's colours,
  the coloured bar beside each command and the status badges.
- **Clearer alerts** — phone alerts read like "✓ 0 · cargo test" with how long it took and
  on which computer.

### Fixed

- **Copy gave no feedback** — copying a command's output turns the button into a check and
  shows "Copied output".
- **Buttons on the pinned header scrolled away** — copy and the menu button on the header
  pinned at the top act on the command instead of scrolling back to it.

## 0.7.1 — 2026-10-16

### Fixed

- **Memory while the screen is off** — with the display asleep or the window hidden behind
  others, a program that keeps redrawing (an agent's spinner, a progress bar) made Rift's
  memory grow without end; left overnight it could use up the Mac's memory and freeze it.
  Rift's memory now stays level.

## 0.7.0 — 2026-10-15

Your phone can fit a pane to its screen, and it sees panes the way Rift draws them.

### New

- **Phone-sized panes** — while your phone shows a pane's Terminal view, the pane takes the
  phone's width, so Claude Code and other programs lay themselves out for it; zooming on the
  phone resizes it again. The pane goes back to its own size when the phone leaves it.

### Improved

- **Panes on your phone look like Rift** — the phone's Terminal view shows each command's
  header as Rift draws it, instead of the shell's raw prompts.

## 0.6.0 — 2026-10-15

Your phone can now tell a Claude Code or Codex session that is working from one that waits
for you, and colored rows render without seams.

### New

- **Idle agents on your phone** — the phone sees each Claude Code and Codex session's name and
  whether it is working or waiting at its own prompt, so Now lists the ones that wait apart
  from the ones that work.

### Improved

- **Pane count on tabs** — a tab's pane count sits at its right end, beside the close button.
- **Translucent windows** — prompt chips and the editor's suggestion text stay readable when
  the window is see-through.

### Fixed

- **Old notifications after an update** — commands that finished before Rift restarted for an
  update no longer come back as "Just now" notifications.
- **Lines between colored rows** — colored rows, such as Claude Code's diffs, meet without a
  thin light line between them.
- **Spinner after a restart** — a tab running Claude Code keeps its spinner while the agent
  works after Rift reattaches to the session, renamed tab or not.

## 0.5.0 — 2026-10-15

Tabs sized like Warp's, links that work inside Claude Code and Codex, and prompt chips
that fit narrow panes.

### New

- **Tabs like Warp's** — every tab is the same width on a slimmer tab bar; a long title
  is cut short, and many tabs shrink evenly to fit.
- **When this tab started** — resting on the time-on-this-tab chip shows when its clock
  started, with the date once it has run a day.

### Improved

- **Chips in a narrow pane** — prompt chips that do not fit one row wrap onto the next
  instead of being cut off.
- **Links** — links are colored and underline when you rest on them; a link that wraps
  onto the next line opens whole.
- **config.toml beside Settings** — Settings → Open config.toml shows the file next to
  the Settings page instead of leaving it, and What's New opens over Settings.

### Fixed

- **Links in Claude Code and Codex** — links inside full-screen programs are found and
  open on click, instead of the click going to the program.
- **Tools shown as unknown after opening Rift from the Dock** — Homebrew tools such as
  docker are no longer underlined red when Rift starts from the Dock or Finder.

## 0.4.0 — 2026-10-15

Links you can hover and click, colored command lines, and your shell's aliases and
tools recognized.

### New

- **Click a link** — links show a hand, and resting on one shows where it goes. A plain
  click shows that, or opens the link: Settings → Terminal → *Clicking a link*. ⌘-click
  always opens.
- **Colored command lines** — the command, its flags and quoted text are colored as you
  type, and in the headers of finished commands.

### Fixed

- **Aliases and tools shown as unknown** — aliases from your shell setup, and tools your
  shell adds to its PATH (pnpm, nvm), are no longer underlined red.
- **Text out of line after a symbol** — after a symbol your font lacks (such as ⧉), the
  rest of the line sits on its columns again, so selecting and copying get the text you
  see.

## 0.3.1 — 2026-10-15

Claude Code and Codex work fully inside Rift.

### Improved

- **Symbols like ⏱ ✔ ☀** — they draw as plain text symbols, as in Warp and
  iTerm2, instead of color emoji (add the emoji selector to get color).
- **Light and dark theme switches** — Claude Code follows them while it runs.

### Fixed

- **⌘V pastes images into Claude Code** — with an image on the clipboard,
  ⌘V now pastes it, as ⌃V already did.
- **Elapsed time on restored tabs** — the hourglass starts over with a new
  shell instead of counting from yesterday's session.
- **⌥↑ / ⌥↓ and ⌥⇧F in programs** — while a program runs (Codex, for
  example) these keys reach it instead of jumping between bookmarks or
  opening the block filter.
- **⌥⏎ for a new line** — it inserts a line break in Claude Code and Codex
  again.
- **Codex picks up your theme colors** — it now reads the background color
  at startup.

## 0.3.0 — 2026-10-15

Your phone can open a tab on this computer even while Rift is closed.

### New

- **Tabs from your phone while Rift is closed** — turn on Settings →
  Remote access → *Phones can open tabs while Rift is closed* (it needs
  *Keep shells running when Rift quits*). A paired phone that may run
  commands can then open a shell here; you get a notification, and the tab
  is waiting in Rift when you open it.
- **What's New in Settings** — Settings → General → Updates opens the
  release notes.

### Fixed

- **Rift stays in the Dock after quitting** — the shells kept running and
  the phone gateway no longer leave Rift "Running in Background".
- **Quitting could hang** — quitting, or restarting for an update, while a
  menu was open could freeze Rift; it now always quits.
- **Release notes** — stray asterisks no longer show in What's New.

## 0.2.0 — 2026-10-14

Your phone now reaches this computer from anywhere, with nothing to set up.

### New

- **Rift's relay** — away from your network, the phone connects through
  Rift's own relay, which only ever forwards encrypted traffic. Point
  Settings → Remote access → *Relay* at your own relay, or clear it to keep
  phones on your network only.
- **Check now** — Settings → General and About check for an update on the
  spot.

### Improved

- **Remote access settings** — the Cloudflare option is now *Use a
  Cloudflare tunnel instead*, below the relay, and an empty relay says that
  phones connect only on this network.
- **Control socket** — the setting explains what it lets scripts do.

### Fixed

- **Empty Enter** — pressing Enter on an empty command line no longer adds
  empty blocks or bare prompt lines.
- **Gaps between blocks** — Enter pressed while a command ran no longer
  leaves a blank line between blocks.

## 0.1.0 — 2026-10-14

The first release of Rift: a GPU-accelerated, block-based terminal for macOS
and Linux.

### New

- **Command blocks** — every command is a block with its exit code, duration
  and a copy button; collapse, filter, bookmark and re-run blocks.
- **Command editor** — a real text editor for the command line, with syntax
  highlighting, completions, ghost suggestions, history search and vim mode.
- **Tabs and split panes** — horizontal or vertical tabs, splits you can drag,
  maximize and navigate with ⌥⌘ arrows, and synchronized input across panes.
- **Command palette** — ⌘P finds every action, setting and history entry;
  ⇧⌘O switches between tabs, panes and recent blocks.
- **Durable sessions** — with `durable_sessions` on, the main window's
  shells keep running when it closes and reattach on the next launch.
- **Inline images** — kitty graphics and iTerm2 inline images, with size and
  memory limits.
- **SSH profiles** — saved hosts, and a confirmed installer that brings shell
  integration to a remote machine.
- **File explorer** — ⇧⌘E shows the project tree with git status; click a
  file to read it in a highlighted viewer, ⌥-click to insert its path.
- **Code review** — ⇧⌘G lists staged and unstaged changes as diffs; stage,
  unstage or revert a whole file or a single hunk.
- **Hint mode** — ⇧⌘H puts letter hints over links, paths and blocks.
- **Notifications** — the inbox button at the top right (or ⇧⌘A) lists
  finished commands, failures, coding-agent and program notifications,
  password prompts and bells with their branch and age, filtered by all,
  unread or errors. Tabs you are not looking at raise toasts and mark their
  tab; Settings → Notifications chooses which events alert and which sound
  each plays. Claude Code, Codex, OpenCode and Gemini CLI get their own
  badges.
- **Scripting** — the control socket and `rift-terminal ctl` drive Rift from
  other programs.
- **Phone companion** — pair an iPhone or Android phone by QR code under
  Settings → Remote access to follow panes live, run commands and send ⌃C,
  end to end encrypted, on your network or through a relay that only sees
  ciphertext.
- **Reach from anywhere** — one switch under Settings → Remote access lets
  your phone reach this computer from any network through a free Cloudflare
  tunnel, with no server of your own; Settings offers to install
  `cloudflared` when it is missing.
- **Elapsed time** — the prompt can show how long the tab has been working
  since its first command.
- **Updates** — Rift checks for a new release at launch and every 6 hours;
  *Update Rift* in the tab bar installs it and keeps your tabs and running
  shells.
- **Session restore** — tabs, splits, directories and the window position
  come back on launch, and optionally each block's output.
- **Warp files** — Warp themes and launch configurations work unchanged.
- **Settings** — every option in a settings window that writes
  `config.toml`, plus a keybinding recorder and four built-in themes.
- **Accessibility** — tabs, panes, blocks and the editor are exposed to screen
  readers.

### Improved

- **Terminal compatibility** — kitty keyboard protocol, mouse reporting,
  synchronized output, OSC 8 links, OSC 52 copy and bracketed paste, VT420
  emulation, and wide characters and emoji measured the way your shell
  measures them.
- **Vim mode** — text objects after an operator: `ciw`, `da"`, `yi(` and the
  rest of the word, quote and bracket objects.
- **Search** — an accented letter finds the text however it was typed, in
  find and in the block filter.
- **Failed commands** — a selected failed block is outlined in red; clicking
  a selected block again deselects it.
- **Shortcut labels** — ⏎, Esc, ⌫, ⌦, Home and End show as keys.
- **Speed** — large outputs render several times faster with far less memory.
- **Pointer shapes** — a hand over everything clickable and resize arrows over
  split handles.
- **App name** — the menu bar and Dock say Rift.

### Fixed

- **Links** — a link to a file that would run as a program (an app, a
  `.webloc`, a configuration profile, or a shortcut to one) is shown in
  Finder instead of opened.
- **Running blocks** — the block border now stops at the last line of output
  instead of running to the bottom of the window.
- **Startup** — the “Starting zsh…” placeholder sits on the input line
  instead of the top of the window.
- **Multi-line paste** — pasting several lines into a program without
  bracketed paste asks before running them.
- **Completion list** — long paths no longer spill out of the list; the start
  of a deep path is shortened to `…` and descriptions line up.
