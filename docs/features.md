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
embedded samples plus `~/.config/runway/workflows/`. ⌃⇧R opens the picker;
insertion activates `{{placeholder}}` spans — Tab cycles, typing replaces.
Warp's community workflow files drop in unchanged.

## Launch configurations

Warp-compatible YAML in `~/.config/runway/launch_configs/*.yaml` — a named set
of tabs, each with a (possibly nested) split layout, cwds and startup commands.
⌘P → `Launch configuration: <name>` opens one: tabs are appended, splits are
rebuilt, and each pane's `commands[].exec` are sent **at that pane's first
prompt**. Runway has one window, so all windows' tabs are concatenated in
document order; `~` in a `cwd` is expanded and a non-existent directory is
ignored. Warp's own files drop in unchanged.

## Appearance

- **Themes**: four builtins (`dark-default`, `light-default`,
  `catppuccin-mocha`, `tokyo-night`) plus user themes in
  `~/.config/runway/themes/` as Runway TOML **or Warp YAML** — a Warp theme
  file works unchanged; Runway derives its full token set from Warp's base
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
- `RUNWAY_NO_NOTIFY=1` suppresses both.

## Tabs, panes, session

- Tabs: create, close, reorder (⌃⇧←/→), select by number (⌘1–9, ⌘9 = last),
  reopen closed (⇧⌘T restores the tab's cwd, stack of 10).
- Panes: split right/down, directional focus (⌥⌘arrows), keyboard resize
  (⌃⌘arrows), drag resize, zoom toggle (⇧⌘⏎), close (⌃⇧W/⌘W — last pane closes
  the tab, last tab exits).
- **Session restore**: tab layout + cwds persisted atomically to
  `~/.local/share/runway/session.toml` on every structural change; reopened on
  launch when `restore_session = true`.

## Shell integration

Materialized to `~/.local/share/runway/` on first spawn, injected
automatically:

- **zsh** — ZDOTDIR shim that sources the user's real config first.
- **bash** — spawned with `--rcfile <data dir>/bashrc`, a shim that replays
  bash's own startup files (profile files on macOS, `~/.bashrc` on Linux),
  then sources `runway.bash`.
- **fish** — `runway.fish` auto-loaded via a `vendor_conf.d` XDG_DATA_DIRS
  append.

Sets `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=runway`;
emits OSC 133 (blocks) and OSC 7 (cwd tracking).

## Rendering & performance

- wgpu SDF quad pipeline (rounded rects, borders, circles, soft shadows) +
  glyphon text; sRGB-space alpha blending matching the design mock's CSS
  compositing.
- **Shape cache**: shaped text buffers are content-addressed and reused across
  frames — scrolling only shapes lines entering the viewport; the cursor line
  reshapes alone. Ages out after 300 unused frames past 4096 entries.
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
- **OSC 52** copy: programs (tmux, neovim, remote shells) can set the system
  clipboard. Reading the clipboard back is refused.
- **Color queries** (OSC 4/10/11/12): answered from the active theme, so
  programs that probe the background (vim, delta, bat) pick matching colors.
- **Text-area size queries** (CSI 14t/18t) are answered.
- **OSC 0/2 titles** and title reset; **OSC 7** cwd; **OSC 8** hyperlinks.
- **IME** composition and committed text in the editor and raw mode.

## Global hotkey

`global_hotkey = "alt+space"` registers a system-wide shortcut (macOS; X11 on
Linux): from any app it brings Runway forward, and when Runway is already the
focused app it hides it. Scripted runs and secondary windows never register
one.

## Deferred (by decision, not gaps)

SSH warpify, multi-cursor editing, vertical tabs, accessibility tree,
`runway://` URI scheme, Warp Drive / cloud / teams, Windows.
