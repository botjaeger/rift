# Features & Behaviors

## Command blocks

Blocks are delimited by shell-integration OSC 133 markers
(`A` prompt start · `B` command start · `C` output start · `D` done + exit code):

- The shell's prompt echo is replaced by a synthesized header (command,
  prompt-time cwd/branch context, status badge, duration, copy button).
- Blocks anchor to **absolute line indices** that survive scrollback trim; a
  block whose lines fall out of scrollback is pruned.
- A missing `D` (e.g. `ssh` hijacking the terminal) is closed off by the next
  `A`; duplicated markers (iTerm2 integration in the user's rc alongside ours)
  are tolerated — handlers are idempotent.
- Prompt-time context: the app stamps cwd + git branch onto the block at
  submit; the PTY reader fills cwd for raw-passthrough commands. Old blocks
  keep the context that was true when they ran.
- The block's command is the exact text submitted from the editor (newlines
  and combining marks included), so reinput, re-run and copy-command reuse
  it unchanged. Commands typed in raw passthrough are read back from the
  screen instead; transcripts keep the exact text.
- Output that ends without a newline (`printf x`) stays in its block: when
  the command finishes mid-row, the terminal moves to a fresh row for the
  prompt (never sent to the shell), and copying the output adds no newline
  it did not have.

When a block's header scrolls off the top, a sticky header stays there; a
click jumps back to the block, except on its buttons, which act as on the
block's own header. Hovering the duration shows the full time and when the
command ended. A header's copy button and its ⋮ menu button (the block's
context menu, as a right-click) show only on hover; a copy turns the button
into a check for 1.5 s and toasts "Copied output".
Block element characters (U+2580–U+259F) are drawn as cell-filling quads, so
they join without gaps.

Per-block operations: select (click / ⌘↑ / ⌘↓), copy command/output/both,
copy cwd, copy git branch, re-run (only at a prompt), reinput into the editor
(⌘I), bookmark (⌘B), collapse (chevron), jump to top/bottom of block.

**Block filter** (⌥⇧F or block menu → *Filter output…*): the find bar edits
a filter for one block; only output lines containing the text (case
insensitive) stay visible. Running it again on the same block, or choosing
*Clear output filter*, shows every line again.

## Clear semantics

- **`clear` command (soft, Warp-style)** — detected via ED3 (`\e[3J`, terminfo
  E3; full-screen TUIs never send it). Scrollback and blocks are preserved;
  the viewport is padded so old content moves above the fold and stays
  reachable by scrolling. The next commands start at the top of the
  viewport and fill it downward, as in a fresh session; the padding shrinks
  as they grow. Alacritty never sees the ED3, so its history survives.
- **⌘K Clear Blocks (destructive)** — wipes screen, scrollback, and the block
  list app-side (no shell round-trip), re-anchoring a fresh prompt block so the
  next command's header extracts correctly.
- **⌃L (editor)** — passes `\x0c` to the shell: classic screen redraw, blocks
  untouched.

## Input model

- **Editor mode**: while the last block is at `Prompt` state and the terminal
  is not in alt-screen, the app owns the command line — local editing, history,
  completions; the shell sees the command only on Enter.
- **Raw passthrough**: any other state sends keys as bytes (vim, less,
  password prompts, integration-less shells). Bound keys that would shadow
  terminal keys (PageUp/PageDown/Home/End/⌃M) automatically fall through to
  the PTY when the editor is inactive.
- Cursor motion and deletion step over whole grapheme clusters (an emoji
  with modifiers, a letter with combining accents), never part of one.
- Editor niceties: word motion & kills (whitespace-delimited), undo/redo
  (snapshot stack, cap 100, cleared on submit), history walk (↑/↓ preserving
  the in-progress line), ⌃R history pane, multi-source completion with
  ghost text, and configurable quote/bracket
  auto-pairing (typing a closer steps over it; Backspace removes an empty pair;
  inside a quoted string nothing pairs, so `'[a'` stays `'[a'`).
- Cmd-chords never insert text into the editor nor leak bytes to the PTY,
  except ⌘⌫ in raw mode, which sends ⌃U (kitty-encoded when the program
  asked for it).
- Editor keys: ⇧←/→ select, ⌥⇧←/→ select by word, ⇧⌘←/→ and ⇧Home/End
  select to the line ends; ⌘Z / ⇧⌘Z undo and redo, ⌘X cuts; readline ⌃A ⌃E
  ⌃B ⌃F move, ⌃K kills to the end, ⌃W deletes the word before, ⌃U clears the
  line; ⌘⌫ / ⌘⌦ kill to the start / end, ⌥⌫ / ⌥⌦ delete a word.
- **Command corrections** (`command_corrections`, on): after a failed
  command-not-found, a "Did you mean …" suggestion shows; → / Tab / End in
  an empty editor accepts it. `alias_expansion` (off) replaces an alias with
  its definition on Space. `error_underline` (on) underlines unknown command
  names in red.
- **Multi-line editing**: ⇧⏎ / ⌥⏎ insert a newline into the command instead of
  submitting; the editor zone grows one line (20 px) per newline and extra
  rows are marked with a dim `…` in the prompt column. Enter still submits the
  whole buffer.
- **Input position** (`input_position`): the editor sits at the bottom
  (default) or the top of the pane.
- **Blocks start** (`blocks_start`): with the editor at the bottom, blocks
  fill up from right above it (default, Warp's "Pin to the bottom"; after
  `clear` the next command sits there again) or start at the top of the pane.
- **Prompt chips** (`prompt_chips`, Warp's "Edit prompt"): the context row
  shows the chosen chips in order — working directory, git branch (↑↓ and
  dirty dot), git diff stats (`3 • +10 −2`: changed files incl. untracked,
  added and removed lines, untracked files' lines as added), the branch's
  GitHub pull request (`PR #42` beside the GitHub mark, both yellow while
  open, dim draft, magenta
  merged, red closed; `gh pr view`, looked up in the background and reused
  for two minutes per branch), user, host, Python (the virtualenv, else the
  version `python3` runs: pyenv's shim resolved through `.python-version` /
  `$(pyenv root)/version`, else the install path), PHP (the version `php`
  runs, from its install path, else asked once), Docker
  Compose (the nearest `compose.yaml` / `docker-compose.yml` at or above
  the cwd, not above home: project name and running/total containers,
  `down` when none run, `off` when Docker is not running; `docker compose
  ps` in the background after each command), date,
  12- and 24-hour time, time on this tab (`elapsed`: hourglass, `2h 05m`
  since the tab's first submitted command, hidden before it; it restarts
  with each new shell and is kept only when a restart reattaches the same
  shell; its hover card says when it started, with the date after 24 hours).
  With `elapsed_counts = "running"` (Settings › Input › Time on this tab
  counts: *Only while running*) it is a time log instead: it counts only
  while a command runs in the tab (a coding agent counts until it exits;
  panes running at the same time count once, not per pane), the hourglass is
  lit while counting and dimmed while paused, and the total is kept across
  an update's restart, a crash and a reboot (it is saved when a command ends
  and every minute while one runs; quitting starts fresh, like the tabs; time
  while Rift is closed is not counted, and neither is time the Mac sleeps; a
  command still running in a reattached shell counts again from the reattach). Its
  hover card ("Counted 6h 33m · running now" or "· paused") stays open while
  the pointer moves into it and has a **Reset** button (also the palette
  action `reset_tab_timer`) that zeroes the tab's time in both modes (the
  clock restarts from now). Default: working directory, git branch, git diff
  stats. A chip with nothing to show is left out. Git, PR and Docker state
  refresh after each command, and in every pane of every tab as soon as the
  chip list changes (Edit prompt or config.toml). Settings › Input › Prompt
  chips previews them and opens the editor: click a chip to add it, drag
  chips to order them (or out of the prompt to remove them), × removes,
  Restore default, Save changes.
- **Prompt chip pickers**: click the cwd chip to `cd` into a sub-directory or
  the parent, the branch chip to check out or create a branch, the diff stats
  chip to open the code review panel on that pane's repository (Warp; a
  second click closes it), the
  PR chip to open the pull request in the browser. The PHP and Python chips
  list the installed versions (PHP: Homebrew `php` / `php@X.Y`, phpenv, asdf,
  mise; Python: pyenv's and `system`, else Homebrew `python@X.Y`, asdf,
  mise; `$HOMEBREW_PREFIX` and `$PYENV_ROOT` honoured), the active one
  checked; picking one runs `rift-use php|python <bin dir>` (that
  directory first on PATH, the previous pick removed) or `rift-use pyenv
  <version>` (`PYENV_VERSION`) — this shell only. The Docker chip lists the
  project's containers as a table — name, published ports, uptime and the
  health check's verdict (healthy / starting / unhealthy), running ones
  first; picking one runs `docker compose logs --follow --tail 200` for
  its service.
- **Shell-prompt mode** (`prompt = "shell"`): the shell's own PS1 (starship,
  p10k, …) stays visible on the grid and the editor drops the context row,
  showing a smaller muted chevron instead of the blue one.

## Vim mode

`vim_mode = true` (Settings → Input, or the palette's *Toggle vim
keybindings*) puts the owned editor into modal editing. The buffer is one
command, so every motion is within-line and the "linewise" operators act on
the whole buffer.

- Modes: Insert (default) and Normal; Esc leaves Insert (stepping the cursor
  left, vim-style), Esc in Normal cancels a half-typed operator/count. A mode
  pill renders at the right of the input line — `NORMAL` (warning color) or
  `INSERT` (muted) — and Normal always draws a solid block cursor.
- Enter submits from either mode; submitting resets to Insert.
- Motions: `h` `l` (and Space) `0` `$` `^` `w` `W` `b` `B` `e` `E`,
  `f` `F` `t` `T` + char, `;` repeats the last find. Arrows/Backspace move too.
- Operators: `d` `c` `y` with a motion; doubled (`dd` `cc` `yy`) is the
  linewise form over the whole buffer. `cw` behaves like `ce`.
- Text objects after an operator: `i`/`a` + `w` `W` (words), `"` `'` `` ` ``
  (quotes), `(` `b` `[` `{` `B` `<` (brackets) — `ciw`, `da"`, `yi(`.
- Edits: `x` `X` `s` `S` `C` `D` `r`<char> `~` `p` `P` (unnamed register).
- Insert entry: `i` `a` `I` `A`.
- Counts: digits before and/or after the operator, multiplied (`2d3w`).
- Undo `u`, **redo `U`** (not `⌃R` — that stays the history pane).
- History recall: `k` = previous entry, `j` = next (also ↑/↓).

## Selection & mouse

- Drag = character selection, double-click = semantic (word) selection,
  triple-click = line selection; selection survives scrollback rotation.
- ⌘C copies the selection when one exists, else the last block's output.
- ⌥-drag makes a rectangular (column) selection.
- Typing clears the selection.
- **Mouse reporting** (`mouse_reporting`, on): programs that enable xterm
  mouse tracking (vim, tmux, htop, fzf) receive presses, releases, drags
  (1002), motion (1003) and the wheel in SGR (1006), UTF-8 (1005) or legacy
  encoding; ⇧ forces local selection. Focus in/out (1004) is reported when
  the window gains or loses focus.
- Links draw in the accent color, lighter and underlined under the pointer,
  and show a hand and a tooltip with their target; a plain click shows it or
  opens the link (`link_click`), also over a program that tracks the mouse
  (Claude Code, Codex) and on the alternate screen. A URL soft-wrapped onto
  the next rows is one link.
- ⌘-click opens URLs, OSC 8 hyperlinks and existing file paths (see ui.md);
  files and folders open where `link_editor` says (default app, the viewer
  pane or an installed editor), and the explorer / review ↗ follow
  `file_editor` (Settings › Files).
  A relative path resolves against the directory the command that printed
  it ran in (the pane's current directory for output outside a block); hint
  mode resolves paths the same way.
- A copy, paste or open that fails shows a toast with the reason (in the
  pane, until dismissed); `Copied` appears only when the clipboard took the
  text.
- Click a block header: select the block (again: deselect). A click without a drag on its output also selects it (again: deselect); a drag selects text. Click its chevron: collapse.
- **Copy on select** (`copy_on_select`, off by default): a drag-selection is
  copied to the clipboard as soon as the mouse is released.

## Completions

Sources merged, deduped, capped at 50: **`$PATH` command names** (first word
only, ≥ 2 chars, max 12 — scanned once per process from every executable on
`$PATH`), command specs (subcommands and flags with descriptions for common tools — git, cargo, docker, npm and more; `completion_specs.rs`) with argument generators (git refs, branches and remotes; ssh hosts from the ssh config and `known_hosts`; Makefile targets; `package.json` scripts; up to 50 each), filesystem paths (`~/` expanded;
hidden files only when the prefix starts with `.`), and command history.

Path completions are inserted backslash-escaped as one shell word (valid in
bash, zsh and fish; a leading `~/` stays unescaped so it still expands), and
a token typed with backslashes or quotes (`My\ Dir/`, `"My Dir/`) still
completes. Names containing control characters are not offered. A command
name shaped like an assignment (`name=value`) gets its `=` escaped too, so the
shell runs the file instead of setting a variable; reserved-word names
(`time`, `[[`) stay bare, as the shell's keyword is what they usually mean.

Completion works on the whole word around the cursor, wherever the cursor
sits in it (like zsh without `complete_in_word`): accepting replaces that
word, including a closing quote or the rest of the word right of the cursor,
and leaves the following arguments untouched (`cat "./hel"` and
`cat ./hel|lo.txt --other` both become `cat ./hello.txt …`).

A typed word is read by shell rules: unquoted, a backslash escapes any
character; inside `"…"` it escapes only `$`, `` ` ``, `"`, `\` and newline and is
otherwise literal (`"./a\b"` completes `a\b.txt`, not `ab.txt`); inside `'…'`
everything is literal.

Providers run on a background thread, never on the UI thread. Tab or ⌃→
pressed while an answer is still in flight is applied once, when it arrives,
and only if the pane, buffer and cursor are unchanged; Escape cancels the
request, so a late answer cannot reopen the menu. If that thread cannot start
or has died, completion pauses (no menu or ghost text; editing is unaffected)
and the first edit after 5 seconds starts a new one.

History matches drive the ghost text: whole-buffer prefix matches, the pane's
own history first (last 200) then the imported shell history (newest 3000),
max 5. Inline ghost text shows the top suggestion; Tab accepts, ⌃→
accepts one word.

The completion panel lists the word candidates, then the history lines; its
header has a tab per source with counts (⇧⇥ or a click narrows to one), and
accepting a history line replaces the whole command line.

**History pane** (⌃R): this window's finished commands (exit status,
directory, finish time), the pane's typed history and the shell's history
file, one row per command, newest first; the command line filters it, ⌃R
cycles the scope (this pane, all shells, failed), ⏎ runs, ⇥ edits first,
⌘⏎ runs in a new split.

`autosuggestions = false` turns the ghost text off; `completions_menu = false`
turns the completion panel off (the ghost text and Tab still work).

## Launch configurations

Warp-compatible YAML in `~/.config/rift/launch_configs/*.yaml` — a named set
of tabs, each with a (possibly nested) split layout, cwds and startup commands.
⌘P → `Launch configuration: <name>` opens one: tabs are appended, splits are
rebuilt, and each pane's `commands[].exec` are sent **at that pane's first
prompt**. A launch configuration opens in the current window; the tabs of
all `windows:` entries are appended in document order; `~` in a `cwd` is expanded and a non-existent directory is
ignored. Warp's own files drop in unchanged.

## Appearance

- **Themes**: four builtins (`dark-default`, `light-default`,
  `catppuccin-mocha`, `tokyo-night`) plus user themes in
  `~/.config/rift/themes/` as Rift TOML **or Warp YAML** — a Warp theme
  file works unchanged; Rift derives its full token set from Warp's base
  colors.
- **OS theme sync** (`sync_os_theme`): follow the system appearance —
  `theme` while dark, `theme_light` while light — switching live when the OS
  does.
- **Window opacity** (`window_opacity`, 1–100 %, default 50 % with a 20 px
  blur), as Warp draws it: below
  100 % the tab bar, side panels, gutter and panes lose their own fills and
  share one window ground — Warp's `surface_2` (bg + 10 % fg) at the opacity
  with `bg` at the opacity over it, so 50 % covers 75 %. Overlays, modals,
  text and non-default cell colours stay opaque. With a theme background
  image the image is drawn at the opacity and veiled by `bg` at
  `(1 − image opacity) × opacity`. The window keeps its macOS shadow.
- **Window blur** (`window_blur`, 1–64 px, macOS): blurs what is behind a
  translucent window (Warp's private `CGSSetWindowBackgroundBlurRadius`);
  1 is effectively none.
- **Background image**: a theme's `background_image` (path + opacity) is drawn
  under every layer; relative paths resolve against the themes directory.
- **Font family** (`font_family`): the embedded Hack (default) or
  JetBrains Mono, or any installed monospace family; an unavailable name logs a
  warning and falls back to JetBrains Mono. Settings → Appearance lists the
  families whose regular face has outline glyphs of one width (Monaco
  included, bitmap-only faces left out), each previewed in itself.
- **Text and cursor**: `font_size` 8–28, `line_height` 1.0–2.2 (default
  1.2), `window_padding` 0–40 px, `scrollback` 0–1,000,000 lines (default
  10,000); out-of-range values are clamped. `ligatures` (on) turns font
  ligatures on, `cursor_style` and `cursor_blink` set the cursor.
- **Settings tab** (⌘,): a tab beside the shells, not an overlay; a sidebar
  search finds a setting on any page, a changed setting shows a dot and a
  reset button, selects open a list.

## Notifications & bell

- **Command alerts**: an alert fires when the command's pane is out of view
  (another tab, a hidden pane, or the window unfocused). While Rift is
  focused it is a toast (`notifications.toasts`), otherwise a desktop
  notification; `notifications.enabled` is the master switch and a sound
  plays when `play_sound` is on. A failed command alerts when
  `notifications.command_failed` is on, whatever its duration; a command that
  ran at least `notify_threshold_secs` (default 10, `notify_on_completion`)
  alerts whatever its exit. Both are set in Settings → Notifications (*A long
  command finishes*, *Long command threshold*).
- **Desktop notifications**: title = who and what (`✓ Finished in 1m12s`,
  `✕ Failed · exit 101 after 2m04s`, `Claude Code · needs your approval`,
  `Codex · task done`, `Password requested`), subtitle = `tab — ~/folder ·
  branch`, body = the command (a failure adds its last output line) or the
  program's message. On macOS Rift.app posts them itself (UserNotifications):
  Rift's icon, banners from one pane stack together, a click brings Rift
  forward on that pane, and a failed command's banner has *Rerun* (only at
  the shell's prompt; a sample from *Try it* never runs). macOS asks for
  permission the first time; Settings › Notifications shows it, with
  *Allow…* or *Open System Settings*. A build that is not an app bundle
  (`cargo run`) falls back to `osascript` (Script Editor's icon); Linux uses
  `notify-send --icon=rift`.
- **Toasts**: at most 2 show at once, for `toast_secs` (default 8, clamped
  2–60).
- **Bell** (`bell`): `visual` (120 ms full-window flash), `audible`
  (`afplay` / `paplay` / `canberra-gtk-play`), `both`, or `none`. A bell in a
  background tab also raises its activity dot.
- `RIFT_NO_NOTIFY=1` suppresses both.

## Tabs, panes, session

- Tabs: create, close, reorder (⌃⇧←/→ or drag: the tab lifts and follows the
  pointer; Esc puts it back), select by number (⌘1–9, ⌘9 = last), reopen
  closed (⇧⌘T restores the tab's cwd, stack of 10).
- Tab menu (right-click): *Rename tab* (inline), a tab color, duplicate,
  sync, *Close other tabs*. ⌘N opens a secondary window; its shells end with
  it.
- **New tabs start in** (`working_directory`): `previous` (the active pane's
  directory, default), `home`, or a path (`~` expands; a missing directory
  falls back to home). `confirm_close_running` (on) asks before closing a
  pane, tab or window with a running job; the dialog lists each running
  process with its agent badge.
- A fresh session shows a welcome list (split, palette, history, explorer,
  settings, with their shortcuts) until its first command.
- Panes: split right/down, directional focus (⌥⌘arrows), keyboard resize
  (⌃⌘arrows), drag resize, zoom toggle (⇧⌘⏎), close (⌃⇧W/⌘W or the title
  bar's × — last pane closes the tab, last tab exits).
- Pane title bars (split tabs): split, ⋮ and × on the focused or hovered
  pane; drag one to move the pane beside another pane (on the side under the
  pointer), swap it with one (the middle third), into another tab (hold to
  open it and place the pane among its panes), or out into a new tab; the
  shell keeps running and keeps its scrollback. The pane menu adds Maximize
  pane and Move to new tab.
- **Synchronized inputs** (⌥⌘I, tab context menu or Window menu): typing and
  submitting in one pane of a split tab goes to the other panes of that tab;
  the tab shows a sync marker while it is on. Input only reaches panes in the
  same mode: raw keys and pastes go to the panes running a program, each
  encoded for that pane's own cursor-key, kitty keyboard and bracketed-paste
  modes; a command submitted from the editor runs only in the panes at a
  prompt, and a toast counts the panes it skipped. A multi-line paste asks
  first if any recipient lacks bracketed paste.
- **Session restore**: tab layout + cwds persisted atomically to
  `~/.local/share/rift/session.toml` on every structural change; reopened on
  launch when `restore_session = true`.
- **Saved block output** (`save_transcripts`, off by default): each pane's
  finished blocks (command, exit code, duration, directory, output — newest
  200 blocks / 1 MiB) are written 0600 to `transcripts/<id>.json` when a
  command finishes and on every session save. A relaunch replays them as
  ordinary blocks marked **restored** above the new shell; the processes are
  not running. Replay is text only (escape sequences stripped). Turning the
  setting off, closing the pane, or "Clear saved block transcripts" deletes
  the files; nothing is redacted, so keep it off where output holds secrets.
  Loading also enforces the 200-block / 1 MiB decoded text budget (including
  commands and directories), with 16 KiB per command/directory and a bounded
  JSON read. Oversized or malformed saved transcripts are ignored.
- **Vertical tabs** (`tab_layout = "vertical"`, Settings → Appearance, or
  "Toggle vertical tabs"): a 220px rail lists every tab with its icon,
  title, directory and git branch, pane count, sync state, activity dot and
  color; click selects, × closes, dragging a row up or down reorders. When
  the tabs do not fit, the active one stays visible and `+N above` /
  `+N more` lines stand in for the rest, marked with the most urgent alert
  among them.
- **Session switcher** (⇧⌘O): one fuzzy list of tabs, panes of split tabs
  (directory and running job), tabs of other Rift windows (focusing one
  raises that window), detached session-service shells, and the 40 newest
  finished blocks; Enter jumps there (a block is selected and scrolled into
  view). Other windows and detached shells load in the background; the list
  remains usable while an unresponsive window times out.
- **Hint mode** (⇧⌘H): every visible URL, existing file path and block
  header gets a letter label; typing it opens the link/path or selects the
  block, ⇧ + letter copies it (a block copies its output). Esc cancels.
  Past 26 hints every label is two letters.
- **Send output to a command** (block menu, palette): the block's output is
  previewed, then written to a private temporary file and run as
  `( command ) < file` (fish: `begin; …; end < file`) in the pane. The output
  is never interpolated into the command line.

## Shell integration

Materialized to `~/.local/share/rift/` on first spawn, injected
automatically:

- **zsh** — ZDOTDIR shim that sources the user's real config first (a
  `ZDOTDIR` set in the user's `~/.zshenv` is honoured for their other files).
  An interactive `zsh` or `exec zsh` typed at a Rift prompt gets the shim
  too (for that command only; the shell keeps the user's `ZDOTDIR`).
- **bash** — spawned with `--rcfile <data dir>/bashrc`, a shim that replays
  bash's own startup files (profile files on macOS, `~/.bashrc` on Linux),
  then sources `rift.bash`. The command-start marker uses `PS0` on bash
  4.4+ and a `DEBUG` trap before that; `PROMPT_COMMAND` may be a string or an
  array (5.1+), and entries a user rc appends after sourcing `rift.bash`
  itself are moved before Rift's last entry before the first prompt.
  `bash` and `exec bash` typed at a Rift prompt start the child with
  `--rcfile <data dir>/bashrc-child` (`~/.bashrc`, then `rift.bash`) through
  the aliases `bash` and `exec ` — not added when the user already has
  aliases of those names. `command bash`, `/bin/bash` and scripts bypass it.
- **fish** — `rift.fish` auto-loaded via a `vendor_conf.d` XDG_DATA_DIRS
  append.

Sets `TERM=xterm-256color` and `COLORTERM=truecolor` (only when not already
set), `TERM_PROGRAM=rift`, `TERM_PROGRAM_VERSION` (Rift's own),
`LC_TERMINAL=Rift` and `LC_TERMINAL_VERSION`;
emits OSC 133 (blocks), OSC 7 (cwd tracking) and, when they change, OSC 1337
SetUserVar `RiftPython` (the prompt chips' Python environment) and, when PATH changes, `RiftPhpBin` / `RiftPythonBin` (the
`php` and `python3` it resolves to); base64, bounded and control characters
dropped on read. Defines `rift-use`, which the PHP and Python chips run.

## Rendering & performance

- wgpu SDF quad pipeline (rounded rects, borders, circles, soft shadows) +
  glyphon text; sRGB-space alpha blending matching the design mock's CSS
  compositing.
- **Shape cache**: shaped text buffers are content-addressed and reused across
  frames — scrolling only shapes lines entering the viewport; the cursor line
  reshapes alone. Ages out after 300 unused frames past 4096 entries.
  Trailing blank cells are never shaped (backgrounds are quads), which keeps
  each cached row small during output floods.
- **Baseline**: `packaging/bench.sh` measures startup, output throughput,
  scrollback memory, key-to-frame latency and TUI round trips through the
  script driver; recorded numbers live in [performance.md](performance.md).
- **Frame pacing**: frames that program output or the running-command
  spinner ask for come at most every 8.3 ms (120 Hz), so drawing never keeps
  the terminal locked away from the thread parsing the output.
- Scroll input accumulates fractional trackpad deltas — no dropped events.
- Full VT emulation via alacritty_terminal (vim/htop/alt-screen, 10k
  scrollback default; VT420 level, see [Terminal protocol
  support](#terminal-protocol-support)), theme & config hot-reload < 300 ms (a once-a-second stat check backs up
  file-system events, which a backlogged fseventsd can drop).
- Git status (branch, ahead/behind, dirty) resolved on a background git2
  worker; never blocks a frame.

## Startup

If the `rift` config or data directory does not exist but one from an earlier
name of the app does (`shift`, then `runway`), Rift keeps using that directory.

`Starting zsh…` placeholder on the prompt chips' row until the shell's first prompt; no terminal
cursor shows while it is up. For a shell without integration, the placeholder leaves and the
cursor shows 1.5 s after its first byte. Session restore
respawns tabs at their saved cwds. The shell's history is imported once for
the history pane and the palette's History mode, from the configured shell's own file: zsh
(`$HISTFILE`, else `~/.zsh_history`; EXTENDED format, multiline entries,
0x83 unmetafy), bash (`$HISTFILE`, else `~/.bash_history`; `#<epoch>`
timestamp lines skipped) or fish (`$XDG_DATA_HOME/fish/fish_history`, else
`~/.local/share/fish/fish_history`). Only the last 4 MiB of the file is
read. The first launch after an update opens What's New (release notes
from `CHANGELOG.md`); the last seen version lives in
`~/.local/share/rift/last-seen-version`, and a first install only records
it.

## Terminal protocol support

- **Keys** (raw mode): F1–F12, xterm modifier encoding for cursor/editing
  keys, DECCKM application cursor, control codes, Meta (see
  keyboard-shortcuts.md).
- **Kitty keyboard protocol**: programs that push flags with `CSI > flags u`
  (Neovim, Helix, fish, kakoune) get unambiguous `CSI u` keys —
  disambiguation, repeat/release events, shifted alternates, every key as an
  escape code, and associated text. `CSI ? u` queries are answered; programs
  that never opt in keep the legacy xterm bytes.
- **Synchronized output** (DECSET 2026): a frame drawn between begin/end
  markers appears at once. A program that stalls mid-update is shown after
  150 ms rather than freezing the pane; a shell marker inside an update
  flushes it first so block anchors stay correct.
- **Inline images**: the kitty graphics protocol (APC `G`: transmit, display,
  put, delete, query; delete selectors `a`/`A`, `i`/`I` and `n`/`N` only —
  others are ignored; PNG, JPEG, GIF, WebP via `f=100`, raw RGB/RGBA `f=24/32`,
  zlib `o=z`, chunked `m=1`, source crop, cell size, z-index under/over text,
  `C=1`, quiet levels) and iTerm2 inline images (OSC 1337 `File=inline=1`, with
  `width`/`height` in cells, px, % or auto and aspect ratio). Images anchor to
  the absolute line model: they scroll with output, follow reflow on resize,
  leave with their line from the scrollback, clear with ⌘K, and alternate-screen
  images go when the program exits. Only direct transmission (`t=d`) is
  accepted — file, temp-file and shared-memory transfers would let a program
  (possibly over ssh) make the terminal read local files — and iTerm2
  downloads (`inline=0`) are ignored. Bounds: 32 MiB payload and 40 M pixels
  per image, 320 MiB decoded and 512 placements per pane, 256 MiB of GPU
  textures (least recently drawn evicted). Placements span at most 1,000 cells
  in each dimension; percentage height uses the terminal's screen rows.
  Invalid or non-finite iTerm dimensions are rejected. Kitty animation frames
  and Unicode placeholders are not implemented. Sixel is not implemented: the
  kitty and iTerm2 protocols cover the TUIs that draw images (yazi, timg,
  chafa, icat, imgcat, viu), and a Sixel decoder would add the largest parser
  surface for the least-used format.
- **Input never freezes the window**: keys, pastes and the terminal's reports
  go to each pane's shell in order from its own queue, so a program that
  stops reading (a paused TUI, a stuck agent) holds only its own input. Up to
  16 MB waits for it; more is refused whole, never half a paste, with a
  notice.
- **Paste**: bracketed paste when the program enables it. Otherwise text with
  a newline asks first ("Paste and run?"), because each newline would press
  Enter (`confirm_multiline_paste`). With no text on the clipboard (an image,
  say), ⌘V sends an empty bracketed paste; Claude Code answers it by reading
  the clipboard image. Files copied in Finder (or the Linux file manager)
  paste as their full paths, shell-quoted and space-separated like a drop,
  instead of the bare file name. Image files too: Claude Code and Codex
  attach an image from its pasted path (the clipboard's image of a copied
  file is only its icon); a screenshot copied as an image keeps the empty
  paste above.
- **OSC 52** copy: programs (tmux, neovim, remote shells) can set the system
  clipboard. Reading the clipboard back is refused unless
  `clipboard_access = "read_write"` (Settings › Terminal › *Copy and read*);
  `"off"` refuses copies too.
- **Middle-click** pastes the clipboard into the pane under the pointer (which
  takes focus) unless its program takes mouse clicks (`middle_click_paste`).
  One wheel notch scrolls `wheel_scroll_lines` lines (default 3).
- **Option as Meta** (macOS): the left and right Option keys are set apart
  (`option_as_meta`, `right_option_as_meta`; the right one follows the left
  until set).
- **Colors** (OSC 4/5/10–12, resets OSC 104/105/110–112): set, query and
  reset the 256-color palette, the special colors and the default
  foreground, background and cursor; queries answer from the active theme, so
  programs that probe the background (vim, delta, bat, Codex) pick matching
  colors, in order with the replies around them (`OSC 11;? CSI c` reads the
  color before the DA1 reply).
- **Color scheme** (mode 2031, `CSI ? 996 n`): a program that sets
  `CSI ? 2031 h` (Claude Code) gets `CSI ? 997 ; 1 n` (dark) or `; 2 n`
  (light) when the theme switches between dark and light; `CSI ? 996 n` asks
  for it.
  Color specs take every X11 form: names, `#rgb`…`#rrrrggggbbbb`, `rgb:`,
  `rgbi:` and the device-independent `CIEXYZ:`, `CIEuvY:`, `CIExyY:`,
  `CIELab:`, `CIELuv:` and `TekHVC:` (ported from Xlib's Xcms, with its gamut
  clipping).
- **Window operations** (XTWINOPS `CSI … t`): iconify, move, resize in
  pixels or characters (the window follows), raise, refresh, maximize,
  fullscreen, and reports of the window state, position, size in pixels and
  characters, screen size, icon label and title; the title stack (22/23).
- **OSC 0/1/2 titles** and icon names, title reset; **OSC 7** cwd; **OSC 8**
  hyperlinks.
- **VT420 emulation** (DA1 answers level 4, `CSI ? 64 ; … c`); the vendored
  `vte` and `alacritty_terminal` carry it:
  - left/right margins (DECLRMM, DECSLRM) for cursor motion, wrapping,
    scrolling, insert/delete line and character, and erase; DECBI/DECFI,
    DECIC/DECDC;
  - rectangular editing: copy (DECCRA), fill (DECFRA), erase and selective
    erase (DECERA, DECSERA), attribute change and reverse (DECCARA, DECRARA,
    extent by DECSACE);
  - protected text: DECSCA with selective erase (DECSED, DECSEL), and ISO
    protected areas (SPA/EPA);
  - double-width and double-height lines (DECDWL, DECDHL): each cell draws
    two columns wide, double-height halves at twice the size; selection and
    mouse reports map to the characters drawn;
  - 132-column mode (DECCOLM when `?40` allows it, DECNCSM), DECSLPP page
    length; the window resizes to fit;
  - reports: DECRQCRA checksums, DECRQSS, DECRQM for ANSI and DEC modes,
    presentation state (DECCIR, DECTABSR, restored by DECRSPS), DECRQTSR,
    DECXCPR, the DSR family, DA2/DA3, XTVERSION and XTGETTCAP;
  - DECSTR soft reset, DECSCL conformance levels, S7C1T/S8C1T (replies stay
    7-bit), the DEC locator, NRCS national charsets and the DEC supplemental
    and technical sets, reverse wraparound (`?45` as xterm 383+, `?1045`
    extended), XTSAVE/XTRESTORE, blinking text (SGR 5, 600 ms) and reverse
    screen (DECSCNM).
- **Unicode widths** follow wcwidth, the table ucs-detect measures against:
  VS16 widens a text-default emoji to two cells and draws it in color, VS15
  narrows an emoji to one cell and draws its text glyph, ZWJ sequences, flags
  and skin tones are one cluster, Indic conjuncts take two cells. The command
  editor wraps by the same widths. The tables are generated
  (`scripts/gen-width-tables.py`).
- **IME** composition and committed text in the editor and raw mode.

## Durable sessions

With `durable_sessions = true` (the default; Settings → General → *Keep
shells running through updates*), new shells run in a
per-user **session service** (`rift-terminal --session-daemon`, started on
demand) instead of the window process. On macOS it and the phone gateway
start as launchd jobs (`<data dir>/launchd/*.plist`, run once, never kept
alive), not as children of the window, so a quit Rift leaves the Dock
instead of staying there as "Running in Background". A service that does
not answer a request within 5 s (hello, spawn, attach) counts as
unavailable: that pane starts a local shell with a notice, and the rest of
the window's run stays local instead of waiting on it again.

**Quitting** Rift (⌘Q, Quit Rift in the menu or the palette, or closing the
window) is intentional: every shell ends and the session file keeps only the
window's frame, so the next launch starts fresh with one tab where the window
was (saved block output goes too). While a phone is connected, the quit
dialog names it first (*iPhone is connected and loses its tabs.*), with the
running commands when *Warn before closing running commands* is on. Shells a
phone opens after the quit (`remote_when_closed`) still appear as tabs at the
next launch.

An update's restart, a crash, a killed process or a reboot of the window
only (the service survives) leave the shells running; the next launch
restores the layout and **reattaches** each pane: the service replays the output it kept (the newest 8 MiB per
shell) into a fresh terminal, with the original timing so block durations
stay right, then continues live. Nothing is shown twice, and automatic
terminal replies (device attributes, image OKs) are not re-sent during the
replay. Commands that finished before the reattach keep their own finish
time and do not appear again in Notifications.

- Closing a pane (⌘W) ends its shell, as without the service. ⌘K also drops
  the retained output, so a reattach does not bring it back.
- Windows that do not restore sessions (secondary ⌘N windows, or
  `restore_session = false`) end their shells however they exit when they
  can, instead of orphaning them.
- Palette → "Attach detached session: …" opens a tab on any shell the
  service keeps that no window shows.
- The service listens on `~/.local/share/rift/sessiond/sock` (a 0700
  directory; other users' connections are refused) and exits 10 s after its
  last shell and window are gone. A slow window is disconnected rather than
  buffering without bound; the pane reattaches on its own and replays only
  what it missed (it closes only when the shell is really gone). Input typed
  meanwhile is held (up to 64 KiB) and sent, in order, once reconnected; a
  toast says so if some of it did not fit, or if output produced meanwhile
  had already left the 8 MiB the service keeps. A lock file
  next to the socket keeps concurrent starts from replacing each other's
  socket. If the configured data
  directory is too long for a Unix socket path, the `sessiond/` and
  `control/` directories both move to `/tmp/rift-<uid>-<hash>/…`, specific
  to that data directory.
- If the service cannot start, the shell runs locally and a toast says so.
- Scripted runs (`--script`) keep their shells local unless
  `RIFT_PERSIST_SESSION` is set.

## Updates

Rift looks for a new release at launch and every 6 hours
(`check_for_updates`, Settings → General → *Updates*): it fetches
`latest.json` from the public
[rift-releases](https://github.com/botjaeger/rift-releases) repository with
the system `curl` (https only, 15 s, at most 64 KiB) and remembers the answer
in `update.json`, so several windows check once.

The manifest must be signed: Rift also fetches `latest.json.sig` and
verifies its Ed25519 signature over the manifest's exact bytes against the
release keys built into the app (the private key stays with the maintainer,
never in CI or the releases repository) before reading anything in it. A
missing or invalid signature offers no update and Settings → About shows
*Couldn't check for updates* (the log says why). The cached answer in
`update.json` is re-verified whenever it is read, so editing that file
cannot inject a release either. The asset's SHA-256 then comes from the
signed manifest.

- A newer version downloads in the background first (verified like the
  install below, into `updates/ready.dmg` or `ready.AppImage`; two windows
  fetching at once never leave a half-written file there). Nothing shows
  while it downloads except Settings, which reads *Downloading vX.Y.Z…*.
  Once it is here, a notification card says *Rift vX.Y.Z is ready —
  Downloaded · click to restart into it* (top right, with the app icon; it
  stays until clicked or dismissed; a desktop notification instead while
  another app is in front; with in-window notifications off,
  `notifications.toasts`, only the chip says it and Rift's other update
  messages become status toasts), once per version, and **Update Rift** shows in
  the tab bar (left of the inbox; tooltip `vX.Y.Z available`). A copy that
  cannot update itself has nothing to download and is told at once
  (*Click to see how to update*); a download that fails is offered anyway
  and the install downloads it. Settings → About shows *Up to date*, *vX.Y.Z
  available* with an **Update…** button, *Installing vX.Y.Z…*, or
  *Couldn't check for updates*. The palette has *Check for updates*
  (`check_for_updates`; the answer is a card: *Rift is up to date*,
  *Couldn't check for updates*, or the ready card once it downloaded) and
  *Update Rift* (`update_rift`). Update messages are cards, never a status
  toast.
- The chip, **Update…** and `update_rift` open a dialog that says what the
  restart keeps: with durable sessions, *Your N tabs and M shells keep running
  and come back where they were*; without them, open shells end (turn on
  *Keep shells running through updates* to keep them). When the new
  version's session service speaks another protocol, running shells keep
  running but cannot be reattached; the dialog says so. **Later** (Esc)
  hides the chip until a newer version.
- **Update** installs the download made ahead (its SHA-256 checked again),
  or downloads this computer's asset now (macOS: the DMG for its
  architecture; Linux: the AppImage) from the public releases only (any
  other URL in the manifest is rejected), checks its size (512 MiB at most)
  and its SHA-256 against the manifest (a mismatch changes nothing), checks
  that the new binary reports the expected version, and swaps it in
  atomically (the app path is never missing): macOS keeps the old bundle
  as `.Rift-old.app` beside `Rift.app`, Linux as `<AppImage>.old`. Only
  one window installs at a time (*Another window is updating Rift*);
  closing or quitting the window meanwhile waits for the update (*Rift is
  updating… it quits when the update is done*). Leftovers of an
  interrupted install are cleared at the next one.
- Every window restarts: the updating window writes the rollback marker,
  saves its own layout and starts the relaunch helper (any failure there
  puts the old version back and changes nothing else), then asks each
  other window over its control socket to save its layout and quit, and
  quits with durable shells still running; the new version reopens every
  window, tabs, splits and directories included, and reattaches the
  shells. Windows whose control socket is off cannot be asked; the dialog
  counts them and they stay open on the old version.
- The new version counts as started 10 s after its first frame: it then
  deletes the old copy, shows the card *Updated to Rift vX.Y.Z* (a click
  opens What's New) and reopens the
  other windows. If it exits or crashes before that, the helper puts the
  old version back and starts it (*Couldn't update to Rift vX.Y.Z*), and
  that one reopens the windows. A crash mid-update leaves the windows'
  layouts in `restart/`; the next plain launch reopens them.
- Updating in place needs Rift.app in a writable folder (macOS) or an
  AppImage in a writable folder (Linux). A copy running from the disk
  image or from macOS's temporary translocated copy says *Move Rift to
  Applications to update it*. Development builds, the `.deb` and the
  tarball get **Open download page** instead.

## Control API

Each window listens on `~/.local/share/rift/control/<pid>.sock`
(`control_api`, Settings → General → Automation):

- `"navigate"` (default): `list`, `status`, `focus`, `blocks` (a pane's
  last command blocks, read-only: up to 30 per request, each output cut to
  its last 500 lines and 64 KiB; `--before ID` pages back), and
  `prepare-restart`, which an updating window sends to the others (save
  the layout, keep durable shells, quit). It must carry the token from that
  update's marker (`update-pending.json`, readable by you only, under 60 s
  old); without it, or while the receiving window is updating itself, it
  is refused and the window stays.
- `"full"`: also `split`, `open` (optionally running a command) and
  `send-text` (types into a shell).
- `"off"`: no socket. Any unrecognised value (`"false"`, a typo) is
  treated as `"off"` and logs a warning. While `remote_access` is on, the
  socket stays open for the remote gateway's requests only: `status`,
  `blocks`, `screen`, `output`, `menu`, `diff`, `complete`, `remote-input`,
  `remote-open`, `remote-close` and `remote-status` (the gateway's notice
  that a phone connected or left).
- `remote-input` (types a command at a pane's prompt, sends keys, or types
  text into a running program), `remote-open` (a new tab or split next to a
  pane) and `remote-close` (closes a pane or its whole tab, without the
  running-command dialog: the phone asked its user first; the window toasts
  "<device> closed a pane") are accepted only while `remote_access` is on,
  whatever `control_api` says.
- `blocks` with `"meta": true` leaves the output out.
- `ctl focus --tab N` / `--pane ID`: tabs count from 0, as in `ctl list`;
  a non-numeric value is rejected with exit code 2.

`rift-terminal ctl [--pid PID] list|status|focus|blocks|split|open|send-text|request JSON` is
the client (`ctl help` for flags); the newest window answers unless `--pid`
names one. `rift-terminal --version` and `--help` print the version and usage; requests are newline-delimited JSON, answered on the
UI thread. The session switcher (⇧⌘O) uses it to list and focus tabs of
other Rift windows. Permission changes from Settings or config reload revoke
existing connections and queued requests before execution. Connections are
limited to 32, with five-second read/write inactivity timeouts; request lines
are bounded to 1 MiB while reading.

## Remote access (phone companion)

Off by default. **Settings › Remote access › Allow remote access**
(`remote_access`) starts the gateway (`rift-terminal remote --managed`, one
per user, log in `<data dir>/remote/gateway.log`); turning it off stops it
within about two seconds, and it also exits 30 seconds after the last Rift
window closes. It listens on `remote_listen` (`0.0.0.0:7878`). The page's
*Connection* group shows *This network* (the listening addresses) and the
*Relay* (its round trip while connected, or the whole error, a retry
countdown and *Try again*, which retries at once).

- **Pairing.** *Pair a phone* shows a QR code, the `rift-remote://pair?…`
  link with a *Copy link* button, and a 2-minute countdown (then *Expired —
  Pair again*). The link works once. *View only* makes the next link pair a
  device that can watch but not run commands or send keys.
- **Paired devices** lists each phone with how it is connected (*Connected
  on this network*, *Connected through the relay*, or *Not connected* with
  when it was last seen), its platform and pairing date, an *Operate* / *View
  only* choice (raising a phone to *Operate* asks first; its connections close
  and reconnect with the new permission within a second, and a lowered
  permission applies at once), and *Revoke*, which asks first ("Revoke
  <name>? It disconnects now and cannot reconnect without pairing again."),
  then closes its connections within a second.
- **Recent connections** lists the newest 20 connections and failed
  attempts: device (or *Unknown device*), via LAN or Relay, address, start,
  duration or *connected*, and why it ended (`remote history` shows the
  newest 500). Five show at once; the list scrolls through the rest.
- **Relay** (outside your network): a `wss://` relay URL (`remote_relay`)
  makes the gateway also reachable through it, and pairing links carry it.
  It defaults to Rift's public relay, `wss://rift-relay.playhaws.com`, so a
  phone reaches the computer from any network with nothing to set up; an
  empty value turns relaying off (phones then connect only on the local
  network). The gateway contacts the relay only while remote access is on,
  and scripted runs (`--script`) skip the default relay
  (`RIFT_NO_DEFAULT_RELAY`, inherited by the gateway they start).
  Its address and the Cloudflare tunnel sit under *Relay settings…*; its
  state shows in *Connection*.
  The relay only forwards encrypted traffic; host one with `rift-terminal
  relay` or `packaging/relay.Dockerfile` (behind a TLS proxy such as Caddy).
  A Tailscale or other private-network address of the computer works
  without a relay: it is listed in the pairing link like any other address.
  Pairing links made in Settings carry the running gateway's relay too.
- **Tabs while Rift is closed** (`remote_when_closed`, *Phones can open
  tabs while Rift is closed*; off by default, needs `durable_sessions`): the
  gateway stays up after the last window closes, and a paired phone that may
  run commands can open a tab (after Face ID or the passcode on the phone).
  The shell runs in the session service; the computer shows a notification
  naming the device. The phone follows and drives it like any pane. The next
  Rift window shows these shells as tabs (with a toast) and the gateway lets
  go of them. At most 8 at a time.
- **Cloudflare tunnel** (`remote_tunnel`, *Use a Cloudflare tunnel instead*; no server needed): the gateway
  runs a relay on a loopback port of this computer and gives it a public
  address with a Cloudflare quick tunnel (`cloudflared tunnel --url`,
  free, no account), then registers there instead of at `remote_relay`.
  Cloudflare and the relay see only Noise ciphertext, and only this
  computer's gateway may register at that relay. Settings checks for
  `cloudflared` (PATH, `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`;
  `RIFT_CLOUDFLARED` names another) and offers *Install* (`brew install
  cloudflared` in a new tab) or *Download* (Cloudflare's page); the gateway
  starts the tunnel by itself within 5 s of the install. The address is
  used 20 s after cloudflared prints it (so the new name resolves), and
  "Pair a phone" waits for it. It is new on every start: phones pair again
  after a restart. A tunnel whose cloudflared exits is restarted after
  30 s; cloudflared ends with the gateway, even a killed one. State:
  `<data dir>/remote/tunnel.json`.
- **Indicator.** While a phone is connected, every window shows a
  *Remote · <device>* chip (*Remote · N devices*) in the tab bar; its
  tooltip names the devices and a click opens the Remote access page.
- **What a phone can do.** See windows, tabs and panes; follow a pane's
  blocks live, page back through older ones, and resume after a dropped
  connection without a gap; get notified when commands finish. An
  `operate` device also runs a command at a pane's prompt (it becomes a
  normal block and history entry; a pane that is running something answers
  *busy*) and sends keys (Ctrl-C, Esc, Tab, arrows, …), which reach the
  program when one is running. Each one shows a toast: "<device> ran <cmd>"
  or "<device> sent ctrl+c". Page Up and Page Down at a prompt do nothing
  (the phone scrolls itself), and keys from a phone skip vim mode.
- **Prompt chips on the phone.** The phone's command line shows the chips
  the desktop draws above that pane's command line (*Edit prompt*'s
  `prompt_chips`, in that order, only those with something to show, none in
  shell-prompt mode) in the desktop's colours, each in its short form (a
  folder's name, the branch with its dirty dot, `102 +2.1k −970`, `#42`, a
  version without its patch, `3/4`), on one row: what does not fit folds into
  *+N* (a dot for each folded Docker project with a container down or
  unhealthy), which opens *This pane*, every chip with its full value. A tap
  opens the chip's menu: the folder, branch and changes menus; the pull
  request in the phone's browser; Python and PHP's versions (a pick runs the
  same `rift-use …`); Docker's containers (a pick follows that service's
  logs, the same command as the desktop's menu); Time on tab with *Reset*
  (`operate` devices; it resets that tab's clock as the card's button does).
  User, Host, Date and Time just show. Picks run as the command line's Run
  does, so a pane running a command answers *busy* and the phone's *Ask
  before running* applies.
- **Full-screen programs and typing (Phase 3).** A pane on the phone shows
  blocks only; while it shows the alternate screen (vim, less, htop) the
  phone shows the pane's screen with its colours instead ("htop is full
  screen · showing its screen"), and the blocks again when the program
  ends. There an `operate` device
  types into the running program (the soft keyboard or a hardware one:
  ctrl/alt combinations, Esc, Tab, arrows, F1–F12, Delete), shown on the
  desktop as "<device> typed into a program", never with the text. Typed
  text never reaches a pane at its prompt (nor one whose command was just
  submitted), and keys at a prompt never recall history, complete or submit
  a line that has text: commands still go through *Run*. A shell without Rift's integration, and a prompt drawn on
  the alternate screen (tmux), look like programs and take typed text. Its Terminal view shows the pane
  the way Rift draws it (a header row per command, not the shell's prompt
  echo), and the phone can size the pane to its width while it follows it:
  the shell is the smaller of the two sizes, so programs reflow, until the
  phone leaves the pane or disconnects, or 30 seconds pass without a
  refresh. With a hardware
  keyboard, ctrl/alt combinations, Esc, Tab and F-keys pressed in the
  command field go to the pane too.
- **Block actions, earlier output, control keys (Phase 3).** A tap on a
  block opens its sheet: it runs the command again, puts it in the command
  field (*Edit*), or copies the command or its output; a running block's
  sheet offers *Stop* (ctrl+c), and its card the *Notify when done* bell
  (on by default for commands run from the phone). A
  block cut to its last 500 lines loads earlier output 500 lines at a time
  (up to 5,000 more; lines gone from the desktop's scrollback say so). The
  row of control keys is chosen, ordered (drag) and added to in Settings ›
  Panes › Choose keys; its *+tab* and *+pane* keys open a new tab in the pane's
  window or split the pane on the desktop (starting in the pane's folder,
  with a toast naming the phone), and the phone goes to the new pane.
- **Phone-first app.** *Portals* is the home page (no tab bar); its gear
  opens *Settings* and its **+** pairs a computer. *Portals* lists each
  computer in a card (connection line, *New tab*, *Retry* or *Pair again*)
  with its tabs, each with its state and time (*Needs you · 2m*, *Working
  3:56*, *Idle 12m*); chips (*needs you*, *working*, *idle*) filter the tabs. With
  one computer paired, an agent's question answers in place on its tab. A cold
  start reopens the pane shown last. A pane's header title opens the *Tabs*
  sheet (every tab of the computer, filtered by state) and its pane picker
  the *Panes* sheet; a sideways swipe also moves between the tab's panes.
  Successes before the newest failure fold away; blocks are cards with a thin
  border in their state's colour (a table as a list, with *Raw*), failed ones
  with *Run again* and *Copy error*, running ones with *Stop* and *Notify when
  done*; a tap opens the block sheet. The header's *Terminal view* shows the
  pane's screen whatever runs, with pinch-to-zoom. Above the command line: the
  folder and branch chips opening the chip menus, suggestions, the key row (⌃;
  sticky ⌃ ⌥ ⇧, Paste) and a round field with a run button; commands run from
  there, there is no separate Run page. *Run* sends at once; Settings ›
  Running commands › *Ask before running* (off by default) shows the
  confirmation first. *Wrap long lines* (Settings › Panes, and the pane's ⋯
  menu for one pane) off lets each block's output scroll sideways. The
  header's search filters the blocks. The folder chip opens what the
  desktop's chips do (feature `chips`): folders (`cd`), local branches
  (`git checkout`) and the code review panel's changed files, each with its
  diff; the names are shell-quoted; it shows the folder and branch the
  desktop computes for its own chips (feature `git`).
- **Needs you (answering from the phone).** The desktop tells the phone
  when a pane's running command waits for the user (feature `waiting`): a
  line read without echo (`sudo`, `ssh`, `read -s`: the terminal's echo
  off in line mode, read from the PTY, or the session service for durable
  sessions; or a cursor line that reads like a password prompt), a coding
  agent's menu on the screen's last rows (Claude Code and Codex: the
  question, the entries under the `❯` / `›` selector, numbered or not, and
  up to 12 lines above the question), or an unread "needs your input" for
  the pane in the attention center (until anything is typed into the pane).
  An `operate` phone answers: a numbered entry is typed as its digit, an
  unnumbered one as arrow keys and Enter, "No" can carry what to do
  instead, and a password is typed (never shown, logged or kept on either
  side). Each answer is ordinary input: the desktop shows its toast and the
  pane goes on. The phone alerts when a question starts (Yes / No / Open on
  the lock screen) and docks the question at the bottom of the pane (Yes, the
  other entries, *No…* with a reply field, and the keys Esc, ⇧Tab, ↑, ↓).
- **Completion and history from the phone.** A pane's suggestions ask the desktop (feature `complete`) for what its
  command line would complete for the typed line (the same completion
  providers, each result the whole line with its kind) and for the
  commands that ran in the pane before (its blocks newest first with when
  they finished, then its command-line history and the imported shell
  history), bounded to 50 each.
- **Running list.** With feature `running` each pane running a command
  carries when it started and the screen's latest line, so the phone's
  lock-screen Live Activity shows the elapsed time and the
  last line without following every pane. A coding agent (Claude Code,
  Codex) also sends its name for the session (its title without the status
  mark) and whether it waits at its own prompt, so the phone titles the
  session by it and tells an agent that waits from one that works.
- **Security.** Traffic is end-to-end encrypted (Noise XX) and the phone pins
  the desktop's key; see [mobile-threat-model.md](mobile-threat-model.md)
  and the wire format in [remote-protocol.md](remote-protocol.md).
- **CLI.** `rift-terminal remote pair [--view]` prints the QR code and link
  in the terminal, `remote devices` lists devices, `remote revoke ID`
  revokes one, `remote permission ID operate|view` changes one,
  `remote history [--limit N]` lists connections, `remote [--listen ADDR]`
  runs a gateway in the foreground, and `rift-terminal relay [--listen
  ADDR]` (or the standalone `rift-relay` binary) runs a relay
  (`0.0.0.0:8790`; `--trust-forwarded` behind a proxy).
- **The app** (`mobile/`): tries the computer's own addresses first and the
  relay second, reconnects at once when the phone changes networks (moving
  home upgrades to the direct path), shows *Offline* without retrying while
  there is no network, closes its connections 30 seconds after going to the
  background (3 minutes while a "notify me" command runs; Android 15+ and iOS
  may cut them sooner, and it reconnects on return), and can require
  Face ID / fingerprint on launch, after 5 minutes away, and (with *Ask
  before running* on) before running commands. With a push token it hands the computers, they alert through the
  Expo push service while the app is closed (`mobile/README.md` › Push).
  Settings › Notifications › *While the app is closed* › *Alerts through
  ntfy* also posts them to a random ntfy topic (ntfy.sh
  or self-hosted), which the free ntfy app shows: no push setup needed.
  Settings › Notifications › *My commands* (on by default) asks to be
  told about every command run from the phone; for other commands, *Other
  commands on your computers* chooses *Nothing*, *Failures* or *Everything*
  and *Only if it ran longer than* how long a command must run (10 s – 5
  min); each computer's page (Settings › Computers) and the pane's ⋯ menu
  have a *Notifications* switch that turns its notifications off. "Notify
  me" always notifies.

## Notifications (attention center)

The inbox button in the top bar (or ⇧⌘A) opens a popover listing, newest
first: commands that finished while their pane was out of view or the window
unfocused, every failed command, programs' notification requests (OSC 9,
OSC 777 `notify`, OSC 99), coding-agent notifications, password prompts in
background panes, and bells from background panes — each with the agent,
the pane's git branch at the time, the title, a status line and its age.
Chips filter to all, unread or errors. A click or ⏎ jumps to the pane and
selects the block. The list keeps the newest 200.

- **Read state**: an entry is unread until you open it, look at its pane, or
  press `Mark all as read`. Unread entries put an accent dot on the inbox
  button and a kind-colored dot on their tab.
- **Alerts** (Settings → Notifications, `[notifications]`): for a pane out
  of view, an enabled event also raises a toast (Rift focused) or a desktop
  notification (Rift in the background), and plays its sound. Events: long
  command finished, command failed, agent task completed, needs attention,
  password prompt. Toasts show over the Settings tab too (only the open
  inbox hides them). Settings › Notifications › *Try it* raises a sample of
  each kind as a toast or a desktop notification.
- **Coding agents**: Claude Code, Codex, OpenCode and Gemini CLI are
  recognized from the running command; their notifications show the agent's
  badge (the Claude mark for Claude Code). An agent waiting at its own prompt
  is idle and its tab does not spin: Claude Code titles itself `✳ …`, Gemini
  CLI `◇ …`, and Codex is idle unless its title leads with a braille
  spinner. The wording sorts notifications into waiting (permission, approval,
  input), error, or task completed. Words match whole (`incomplete` is not
  `complete`); an explicit request (`needs your permission`) wins, then
  error words (`permission denied`, `failed`) beat a bare `permission`, and
  negated errors (`no errors`, `0 errors`, `without error`) do not count.
  Setup per agent:
  [configuration](configuration.md#notifications--bell).
- **Password prompts**: a running command in a background pane whose cursor
  line ends like `Password:` or `Enter passphrase for key …:` raises one
  alert per command.

## File explorer and code review

⇧⌘E opens the file explorer: the active pane's project (git working tree,
else its directory) as a lazily expanded tree with ignored files hidden
(dotfiles too with Settings › Files › Show hidden files off) and
git status colors. A folder that cannot be read shows the error (e.g.
`Permission denied`); one with more than 2,000 entries shows the first
2,000 in display order, then `… more entries not shown` (a folder is read
no further than 100,000 entries, ignored ones included). A file opens in a read-only viewer pane (or the editor `file_editor` names), the leftmost column of the tab
(line numbers, keyword/string/comment highlighting, 4 MB limit, binary
detection, ⌘F finds in the file; Markdown opens rendered, with a Rendered |
Raw switch); ⌥-click inserts its path into the command
line. Preview reads and
highlighting run in the background. Only regular files are accepted, and
the byte limit is enforced during reading, including when a file grows.

⇧⌘G opens code review: staged and unstaged changes (untracked included) as
file cards of hunks: line numbers, syntax colors, the changed characters of
an edited line highlighted, and "N unmodified lines" bands that open the
unchanged lines between hunks. Panel and cards are opaque; long lines scroll
sideways per file (trackpad, ⇧ + wheel). Stage, unstage or revert a whole
file from its header, or one hunk from the toolbar a hovered line shows (which
also copies the hunk as a unified diff); ⤢ maximizes the panel over the panes
with a file list on the right, Esc restores it. Revert asks first and only discards unstaged work. Hunk actions re-diff,
match the hunk by its line ranges and content, and apply that same checked
diff, so a stale panel never applies the wrong hunk. A hunk cut by the
4,000-line limit offers only file actions. The model loads on a background
thread and reloads when a command finishes; writes run in order on a
background writer thread. Details in [ui.md](ui.md).

## SSH profiles

Palette → "SSH: <name>" opens a tab titled after the profile that runs the
system `ssh` (so your ssh config, agent, `known_hosts` checking and prompts
all apply). Profiles are `[[ssh_profiles]]` in config.toml plus every concrete
`Host` of `~/.ssh/config` (and its `Include`s); trailing comments are ignored
and aliases containing `=` are preserved.

"Install Rift shell integration on <name>" asks first, then runs an
installer over ssh in a new tab: it writes `~/.local/share/rift/rift.{bash,
zsh,fish}` on the host and adds one line ending `# rift-shell-integration`
to `~/.bashrc` and `~/.zshrc` (plus `~/.config/fish/conf.d/rift.fish`). The
hooks load only when `LC_TERMINAL=Rift` arrives with the session: Rift sets
it and passes `-o SendEnv=LC_TERMINAL`, and most servers `AcceptEnv LC_*`.
After reconnecting, remote commands become blocks with the owned editor,
exit codes and the remote directory; images and tmux work over ssh as
locally. Without the hooks (or the env), ssh is an ordinary raw terminal.
The block of the `ssh` command itself ends when the remote prompt starts.

## Accessibility

Rift draws every surface itself, so it publishes an AccessKit tree
(`crates/rift-terminal/src/a11y.rs`) to VoiceOver, Orca and other screen readers:

- **Tabs** as a tab list (selected state; activating one switches tabs).
- **Panes** as groups labelled with their directory and running job, each
  holding a list of the newest 50 **command blocks** (label = command, description =
  succeeded/failed with exit code and duration, value = output, last 4000
  characters), the visible **terminal** text, and the owned **command
  editor** as a text field.
- **Panels**: the file explorer and code review, with their buttons; the
  explorer's notices (an unreadable folder, a cut listing) are its text.
- **Overlays**: the palette (query field, result options, selection), find
  bar (query and match count), block filter (its own label, query and
  keep/clear instructions), settings (pages, switches with their state,
  buttons and choices with their current values), block menu, and the
  confirm dialogs.
- **Announcements**: a polite live region says "`command` succeeded/failed …"
  when a command finishes in the visible tab.
- **Actions**: Click on a node clicks its on-screen rect through the normal
  mouse routing, so it behaves exactly like the pointer. Only Click
  activates; Focus is not advertised, so moving the screen reader's focus
  never presses anything.

The tree is only built while an assistive technology is attached and is
pushed only when it changes. The script driver's `dump` prints it.

## Global hotkey

`global_hotkey` is empty (off) by default; `global_hotkey = "alt+space"`
registers a system-wide shortcut (macOS; X11 on Linux): from any app it brings Rift forward, and when Rift is already the
focused app it hides it. Scripted runs and secondary windows never register
one.

## Deferred (by decision, not gaps)

Multi-cursor editing, `rift://` URI scheme, Warp Drive / cloud / teams.
Everything else on the [roadmap](terminal-landscape-roadmap.md) is planned.
