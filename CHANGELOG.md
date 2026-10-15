# Changelog

User-facing changes per release, newest first. The in-app What's New page
(Help → What's New, or `whats_new` in the palette) renders this file, so every
entry is written for someone using Rift, not for someone building it.

Format: `## <version> — <YYYY-MM-DD>`, an optional one-paragraph summary, then
`### New`, `### Improved` or `### Fixed` sections of `- **Title** — text`
items.

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
