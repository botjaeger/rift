# Changelog

User-facing changes per release, newest first. The in-app What's New page
(Help → What's New, or `whats_new` in the palette) renders this file, so every
entry is written for someone using Rift, not for someone building it.

Format: `## <version> — <YYYY-MM-DD>`, an optional one-paragraph summary, then
`### New`, `### Improved` or `### Fixed` sections of `- **Title** — text`
items.

## 0.1.0 — 2026-09-24

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
- **Settings** — every option in a settings window that writes
  `config.toml`, plus a keybinding recorder and four built-in themes.
- **Accessibility** — tabs, panes, blocks and the editor are exposed to screen
  readers.

### Improved

- **Terminal compatibility** — kitty keyboard protocol, mouse reporting,
  synchronized output, OSC 8 links, OSC 52 copy and bracketed paste.
- **Speed** — large outputs render several times faster with far less memory.
- **Pointer shapes** — a hand over everything clickable and resize arrows over
  split handles.
- **App name** — the menu bar and Dock say Rift.

### Fixed

- **Running blocks** — the block border now stops at the last line of output
  instead of running to the bottom of the window.
- **Startup** — the “Starting zsh…” placeholder sits on the input line
  instead of the top of the window.
- **Multi-line paste** — pasting several lines into a program without
  bracketed paste asks before running them.
- **Completion list** — long paths no longer spill out of the list; the start
  of a deep path is shortened to `…` and descriptions line up.
