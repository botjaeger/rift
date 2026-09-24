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
  reachable by scrolling. Alacritty never sees the ED3, so its history
  survives.
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
- Editor niceties: word motion & kills (whitespace-delimited), undo/redo
  (snapshot stack, cap 100, cleared on submit), history walk (↑/↓ preserving
  the in-progress line), ⌃R history palette, multi-source completion with
  ghost text, workflow placeholder cycling, and configurable quote/bracket
  auto-pairing (typing a closer steps over it; Backspace removes an empty pair).
- Cmd-chords never insert text into the editor nor leak bytes to the PTY.
- **Multi-line editing**: ⇧⏎ / ⌥⏎ insert a newline into the command instead of
  submitting; the editor zone grows one line (20 px) per newline and extra
  rows are marked with a dim `…` in the prompt column. Enter still submits the
  whole buffer.
- **Input position** (`input_position`): the editor sits at the bottom
  (default) or the top of the pane.
- **Shell-prompt mode** (`prompt = "shell"`): the shell's own PS1 (starship,
  p10k, …) stays visible on the grid and the editor drops the context row,
  showing a quiet `›` continuation glyph instead of `❯`.

## Vim mode

`vim_mode = true` (Settings → Features, or the palette's *Toggle vim
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
- Edits: `x` `X` `s` `S` `C` `D` `r`<char> `~` `p` `P` (unnamed register).
- Insert entry: `i` `a` `I` `A`.
- Counts: digits before and/or after the operator, multiplied (`2d3w`).
- Undo `u`, **redo `U`** (not `⌃R` — that stays the history palette).
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
- ⌘-click opens URLs, OSC 8 hyperlinks and existing file paths (see ui.md).
- Click a block header: select the block. Click its chevron: collapse.
- **Copy on select** (`copy_on_select`, off by default): a drag-selection is
  copied to the clipboard as soon as the mouse is released.

## Completions

Sources merged, deduped, capped at 50: **`$PATH` command names** (first word
only, ≥ 2 chars, max 12 — scanned once per process from every executable on
`$PATH`), command specs (subcommands and flags with descriptions for common tools — git, cargo, docker, npm and more; `completion_specs.rs`), filesystem paths (`~/` expanded;
hidden files only when the prefix starts with `.`), and command history.

History matches drive the ghost text: whole-buffer prefix matches, the pane's
own history first (last 200) then the imported shell history (newest 3000),
max 5. Inline ghost text shows the top suggestion; Tab accepts (placeholder
cycling wins when active), ⌃→ accepts one word.

`autosuggestions = false` turns the ghost text off; `completions_menu = false`
turns the dropdown off (the ghost text and Tab still work).

## Workflows

Warp-compatible YAML (`name`, `command`, `tags`, `arguments`) loaded from
embedded samples plus `~/.config/shift/workflows/`. ⌃⇧R opens the picker;
insertion activates `{{placeholder}}` spans — Tab cycles, typing replaces.
Warp's community workflow files drop in unchanged.

## Launch configurations

Warp-compatible YAML in `~/.config/shift/launch_configs/*.yaml` — a named set
of tabs, each with a (possibly nested) split layout, cwds and startup commands.
⌘P → `Launch configuration: <name>` opens one: tabs are appended, splits are
rebuilt, and each pane's `commands[].exec` are sent **at that pane's first
prompt**. Shift has one window, so all windows' tabs are concatenated in
document order; `~` in a `cwd` is expanded and a non-existent directory is
ignored. Warp's own files drop in unchanged.

## Appearance

- **Themes**: four builtins (`dark-default`, `light-default`,
  `catppuccin-mocha`, `tokyo-night`) plus user themes in
  `~/.config/shift/themes/` as Shift TOML **or Warp YAML** — a Warp theme
  file works unchanged; Shift derives its full token set from Warp's base
  colors.
- **OS theme sync** (`sync_os_theme`): follow the system appearance —
  `theme` while dark, `theme_light` while light — switching live when the OS
  does.
- **Window opacity** (`window_opacity`, 0.3–1.0): only the terminal grounds
  (background, chrome, gutter) go translucent; overlays keep full contrast.
- **Background image**: a theme's `background_image` (path + opacity) is drawn
  under every layer; relative paths resolve against the themes directory.
- **Font family** (`font_family`): any installed monospace family; an
  unavailable name logs a warning and falls back to the embedded JetBrains
  Mono. Settings → Appearance cycles through the installed families.

## Notifications & bell

- **Completion notifications** (`notify_on_completion`, on): a desktop
  notification when a command that ran at least `notify_threshold_secs`
  (default 10) finishes while the window is unfocused — title carries the
  status (`finished` / `failed (exit N)`), body the command and duration.
  Uses `osascript` on macOS, `notify-send` on Linux; no extra dependencies.
- **Bell** (`bell`): `visual` (120 ms full-window flash), `audible`
  (`afplay` / `paplay` / `canberra-gtk-play`), `both`, or `none`. A bell in a
  background tab also raises its activity dot.
- `SHIFT_NO_NOTIFY=1` suppresses both.

## Tabs, panes, session

- Tabs: create, close, reorder (⌃⇧←/→), select by number (⌘1–9, ⌘9 = last),
  reopen closed (⇧⌘T restores the tab's cwd, stack of 10).
- Panes: split right/down, directional focus (⌥⌘arrows), keyboard resize
  (⌃⌘arrows), drag resize, zoom toggle (⇧⌘⏎), close (⌃⇧W/⌘W — last pane closes
  the tab, last tab exits).
- **Synchronized inputs** (⌥⌘I, tab context menu or Window menu): typing and
  submitting in one pane of a split tab goes to every pane of that tab; the
  tab shows a sync marker while it is on.
- **Session restore**: tab layout + cwds persisted atomically to
  `~/.local/share/shift/session.toml` on every structural change; reopened on
  launch when `restore_session = true`.
- **Saved block output** (`save_transcripts`, off by default): each pane's
  finished blocks (command, exit code, duration, directory, output — newest
  200 blocks / 1 MiB) are written 0600 to `transcripts/<id>.json` when a
  command finishes and on every session save. A relaunch replays them as
  ordinary blocks marked **restored** above the new shell; the processes are
  not running. Replay is text only (escape sequences stripped). Turning the
  setting off, closing the pane, or "Clear saved block transcripts" deletes
  the files; nothing is redacted, so keep it off where output holds secrets.
- **Vertical tabs** (`tab_layout = "vertical"`, Settings → Appearance, or
  "Toggle vertical tabs"): a 220px rail lists every tab with its shell,
  title, directory and git branch, pane count, sync state, activity dot and
  color; click selects, × closes.
- **Session switcher** (⇧⌘O): one fuzzy list of tabs, panes of split tabs
  (directory and running job), tabs of other Shift windows (focusing one
  raises that window), detached session-service shells, and the 40 newest
  finished blocks; Enter jumps there (a block is selected and scrolled into
  view).
- **Hint mode** (⇧⌘H): every visible URL, existing file path and block
  header gets a letter label; typing it opens the link/path or selects the
  block, ⇧ + letter copies it (a block copies its output). Esc cancels.
- **Send output to a command** (block menu, palette): the block's output is
  previewed, then written to a private temporary file and run as
  `( command ) < file` (fish: `begin; …; end < file`) in the pane. The output
  is never interpolated into the command line.

## Shell integration

Materialized to `~/.local/share/shift/` on first spawn, injected
automatically:

- **zsh** — ZDOTDIR shim that sources the user's real config first.
- **bash** — spawned with `--rcfile <data dir>/bashrc`, a shim that replays
  bash's own startup files (profile files on macOS, `~/.bashrc` on Linux),
  then sources `shift.bash`.
- **fish** — `shift.fish` auto-loaded via a `vendor_conf.d` XDG_DATA_DIRS
  append.

Sets `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=shift`;
emits OSC 133 (blocks) and OSC 7 (cwd tracking).

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
- Scroll input accumulates fractional trackpad deltas — no dropped events.
- Full VT emulation via alacritty_terminal (vim/htop/alt-screen, 10k
  scrollback default), theme & config hot-reload < 300 ms.
- Git status (branch, ahead/behind, dirty) resolved on a background git2
  worker; never blocks a frame.

## Startup

`Starting zsh…` placeholder until the shell's first byte. Session restore
respawns tabs at their saved cwds. Shell history (`~/.zsh_history`, EXTENDED
format, multiline entries, 0x83 unmetafy) imported once for the history
palette.

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
  put, delete, query; PNG, JPEG, GIF, WebP via `f=100`, raw RGB/RGBA `f=24/32`,
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
  textures (least recently drawn evicted). Sixel is not implemented: the
  kitty and iTerm2 protocols cover the TUIs that draw images (yazi, timg,
  chafa, icat, imgcat, viu), and a Sixel decoder would add the largest parser
  surface for the least-used format.
- **Paste**: bracketed paste when the program enables it. Otherwise text with
  a newline asks first ("Paste and run?"), because each newline would press
  Enter (`confirm_multiline_paste`).
- **OSC 52** copy: programs (tmux, neovim, remote shells) can set the system
  clipboard. Reading the clipboard back is refused.
- **Color queries** (OSC 4/10/11/12): answered from the active theme, so
  programs that probe the background (vim, delta, bat) pick matching colors.
- **Text-area size queries** (CSI 14t/18t) are answered.
- **OSC 0/2 titles** and title reset; **OSC 7** cwd; **OSC 8** hyperlinks.
- **IME** composition and committed text in the editor and raw mode.

## Durable sessions

With `durable_sessions = true` (Settings → General), new shells run in a
per-user **session service** (`shift-terminal --session-daemon`, started on
demand) instead of the window process. Quitting Shift or closing the window
leaves them running; the next launch restores the layout and **reattaches**
each pane: the service replays the output it kept (the newest 8 MiB per
shell) into a fresh terminal, with the original timing so block durations
stay right, then continues live. Nothing is shown twice, and automatic
terminal replies (device attributes, image OKs) are not re-sent during the
replay.

- Closing a pane (⌘W) ends its shell, as without the service. ⌘K also drops
  the retained output, so a reattach does not bring it back.
- Windows that do not restore sessions (secondary ⌘N windows, or
  `restore_session = false`) end their shells on exit instead of orphaning
  them.
- Palette → "Attach detached session: …" opens a tab on any shell the
  service keeps that no window shows.
- The service listens on `~/.local/share/shift/sessiond/sock` (a 0700
  directory; other users' connections are refused) and exits 10 s after its
  last shell and window are gone. A slow window is disconnected rather than
  buffering without bound; it reattaches and replays.
- If the service cannot start, the shell runs locally and a toast says so.

## Control API

Each window listens on `~/.local/share/shift/control/<pid>.sock`
(`control_api`, Settings → General → Automation):

- `"navigate"` (default): `list`, `status`, `focus`.
- `"full"`: also `split`, `open` (optionally running a command) and
  `send-text` (types into a shell).
- `"off"`: no socket.

`shift-terminal ctl list|status|focus|split|open|send-text` is the client
(`ctl help` for flags); requests are newline-delimited JSON, answered on the
UI thread. The session switcher (⇧⌘O) uses it to list and focus tabs of
other Shift windows.

## Attention center

⇧⌘A lists, newest first: commands that finished while their pane was out of
view or the window unfocused, every failed command, programs' notification
requests (OSC 9, OSC 777 `notify`, OSC 99), and bells from background panes —
each with its tab, status, duration and age. Enter jumps to the pane and
selects the block. Unread entries show as "● N alerts ⇧⌘A" in the top bar;
opening the center marks them read. Notification requests from panes out of
view also raise a desktop notification.

## SSH profiles

Palette → "SSH: <name>" opens a tab titled after the profile that runs the
system `ssh` (so your ssh config, agent, `known_hosts` checking and prompts
all apply). Profiles are `[[ssh_profiles]]` in config.toml plus every concrete
`Host` of `~/.ssh/config` (and its `Include`s).

"Install Shift shell integration on <name>" asks first, then runs an
installer over ssh in a new tab: it writes `~/.local/share/shift/shift.{bash,
zsh,fish}` on the host and adds one line ending `# shift-shell-integration`
to `~/.bashrc` and `~/.zshrc` (plus `~/.config/fish/conf.d/shift.fish`). The
hooks load only when `LC_TERMINAL=Shift` arrives with the session: Shift sets
it and passes `-o SendEnv=LC_TERMINAL`, and most servers `AcceptEnv LC_*`.
After reconnecting, remote commands become blocks with the owned editor,
exit codes and the remote directory; images and tmux work over ssh as
locally. Without the hooks (or the env), ssh is an ordinary raw terminal.
The block of the `ssh` command itself ends when the remote prompt starts.

## Accessibility

Shift draws every surface itself, so it publishes an AccessKit tree
(`crates/shift-app/src/a11y.rs`) to VoiceOver, Orca and other screen readers:

- **Tabs** as a tab list (selected state; activating one switches tabs).
- **Panes** as groups labelled with their directory and running job, each
  holding a list of **command blocks** (label = command, description =
  succeeded/failed with exit code and duration, value = output, last 4000
  characters), the visible **terminal** text, and the owned **command
  editor** as a text field.
- **Overlays**: the palette (query field, result options, selection), find
  bar (query and match count), settings (pages, switches with their state,
  buttons and choices with their current values), block menu, and the
  confirm dialogs.
- **Announcements**: a polite live region says "`command` succeeded/failed …"
  when a command finishes in the visible tab.
- **Actions**: Click/Focus on a node clicks its on-screen rect through the
  normal mouse routing, so it behaves exactly like the pointer.

The tree is only built while an assistive technology is attached and is
pushed only when it changes. The script driver's `dump` prints it.

## Global hotkey

`global_hotkey = "alt+space"` registers a system-wide shortcut (macOS; X11 on
Linux): from any app it brings Shift forward, and when Shift is already the
focused app it hides it. Scripted runs and secondary windows never register
one.

## Deferred (by decision, not gaps)

Multi-cursor editing, `shift://` URI scheme, Warp Drive / cloud / teams.
Everything else on the [roadmap](terminal-landscape-roadmap.md) is planned.
