# Configuration

## File locations

| Path | Purpose |
|---|---|
| `~/.config/shift/config.toml` | User configuration (hot-reloaded < 300 ms, including external edits) |
| `~/.config/shift/themes/*.toml` | User themes, Shift TOML (file stem = theme name) |
| `~/.config/shift/themes/*.yaml` · `*.yml` | User themes, Warp YAML (same directory, same naming) |
| `~/.config/shift/workflows/*.yaml` | User workflows (Warp-compatible YAML) |
| `~/.config/shift/launch_configs/*.yaml` · `*.yml` | Launch configurations (Warp-compatible YAML) |
| `~/.local/share/shift/` | Materialized shell-integration files |
| `~/.local/share/shift/session.toml` | Saved session (tabs + cwds), written atomically |
| `~/.local/share/shift/last-seen-version` | Version whose What's New was last shown (opens it once after an update) |

`XDG_CONFIG_HOME` and `XDG_DATA_HOME` are honored. Existing `runway` config and data directories remain active when the corresponding `shift` directory does not exist; move them to `shift` when you want the new paths.

## config.toml reference

All keys optional; omitted keys use the defaults shown.

```toml
font_family   = "JetBrains Mono"  # any installed monospace family; unknown
                                  # names fall back to the embedded JetBrains Mono
font_size     = 13.0              # settings stepper clamps 8–28 (⌘= / ⌘- / ⌘0 also write this)
line_height   = 1.65              # multiplier, clamped 1.0–2.2 when applied
cursor_style  = "block"           # block | bar | underline
cursor_blink  = true              # 1.1s steps blink
theme         = "dark-default"    # builtin, or a stem in ~/.config/shift/themes
theme_light   = "light-default"   # theme used while the OS is light (needs sync_os_theme)
sync_os_theme = false             # follow the OS appearance: theme = dark, theme_light = light
scrollback    = 10000             # lines kept per pane (stepper: 1000–1000000)
window_padding = 8.0              # left/right content padding in px, clamped 0–40
                                  # (single-pane layout; splits keep their gutter)
window_opacity = 1.0              # background opacity, clamped 0.3–1.0 (overlays stay opaque)
ligatures     = true              # JetBrains Mono ligatures + contextual alternates
restore_session = true            # reopen last session's tabs + cwds on launch
save_transcripts = false          # also save finished blocks' output and replay it
                                  # (marked "restored") on relaunch; off = deleted
tab_layout    = "horizontal"      # horizontal (tabs on top) | vertical (side rail)
durable_sessions = false          # shells live in the session service: they keep
                                  # running after quit and reattach on launch
control_api   = "navigate"        # off | navigate (list/status/focus) | full (also
                                  # split/open/send-text); `shift-terminal ctl`
input_position = "bottom"         # bottom | top — where the command editor sits
prompt        = "shift"          # shift = context row + ❯
                                  # shell   = keep the shell's own PS1 on screen, editor shows ›
copy_on_select = false            # copy the selection on mouse release
autosuggestions = true            # inline ghost text after the cursor
completions_menu = true           # completion dropdown (Tab still accepts the ghost text)
autocomplete_pairs = true         # auto-close quotes/brackets; Backspace removes empty pairs
vim_mode      = false             # modal (vim) editing in the command editor
notify_on_completion = true       # alert when a long command finishes out of view
notify_threshold_secs = 10        # what counts as long (stepper: 0–3600, ±5)
bell          = "visual"          # visual | audible | both | none
confirm_close_running = true      # warn before closing a pane/tab/window with a running job
confirm_multiline_paste = true    # ask before a paste with newlines reaches a program
                                  # without bracketed paste (each newline presses Enter)
working_directory = "previous"    # new tabs/panes: previous | home | "/abs/path" (~ expands)
alias_expansion = false           # Space after an alias replaces it with its definition
error_underline = true            # red underline under unknown command names
command_corrections = true        # "Did you mean …" after a failed command-not-found
mouse_reporting = true            # forward clicks/drags/wheel to vim, tmux, htop…
                                  # (hold Shift to select locally); also focus events
option_as_meta = false            # macOS: Option sends ESC-prefixed Meta keys instead
                                  # of composing characters (Linux Alt is always Meta)
global_hotkey = ""                # system-wide show/hide chord, e.g. "alt+space";
                                  # empty = off (X11 on Linux)
# shell        = "/bin/zsh"       # default: $SHELL

# [[ssh_profiles]]                # palette "SSH: <name>" (hosts from ~/.ssh/config
# name = "prod"                   # are listed automatically)
# host = "me@prod.example.com"    # user@host or a Host alias
# args = ["-p", "2222"]           # extra ssh arguments

[notifications]                   # Settings → Notifications
enabled = true                    # master switch for toasts, desktop notifications, sounds
command_failed = true             # a command failed out of view
agent_task_completed = true       # Claude Code / Codex / OpenCode / Gemini CLI finished
needs_attention = true            # OSC 9/777/99, an agent asking for input or approval
password_prompt = true            # a running command shows a password prompt
toasts = true                     # in-window cards while Shift is focused
toast_secs = 8                    # 2–60; hovering keeps a toast
play_sound = true
sound = "default"                 # default | none | a name (Glass, Hero, a file stem
                                  # in ~/Library/Sounds) | a path to an audio file
sound_finished = ""               # per-event overrides; "" = use `sound`
sound_failed = ""
sound_attention = ""

[keybindings]                     # action name → chord; overrides also unbind
# new_tab = "cmd+n"               # the action's default alternates
# clear_blocks = "none"           # "none" unbinds an action
```

Settings → Keybindings writes this table for you: click a row, press the new
chord (Esc cancels, ⌫ unbinds, ↺ restores the default). A chord already used
by another action moves over; the other action keeps its remaining shortcut.

The session file also stores the window frame (`[window]` position, size,
maximized, fullscreen); the next launch reopens at the same place.

The Settings overlay (⌘,) edits the same file live; the hot-reload watcher
applies external edits without restarting.

- **Invalid or unreadable file**: it is not applied. The last good config
  stays live (at startup, the defaults with `control_api = "off"`), a toast
  names the error, and Settings refuses to save over the broken file until it
  is fixed. A missing file means defaults.
- **Saves are atomic**: Shift writes a temporary file next to `config.toml`,
  keeps the old file's permissions and renames it over, so a failed write
  never truncates the config.
- **Saves edit the file in place**: Settings changes only the lines whose
  values changed, keeping comments, key order and unknown keys. A setting
  the file never mentioned is added only when it differs from its default.
- A non-finite `font_size` or `line_height` (`nan`, `inf`) falls back to its
  default. The value stays accepted and is kept as written on save, along
  with the file's comments and unknown keys.

## Themes

Built-ins, in picker order: `dark-default`, `light-default`,
`catppuccin-mocha`, `tokyo-night`.

A user theme is `~/.config/shift/themes/<name>.{toml,yaml,yml}`, referenced by
file stem (`toml` wins if several exist). It appears in Settings → Themes and
the palette's theme switcher. Editing the selected theme file reloads its
colors without changing `config.toml` or restarting.

### Shift TOML themes

A theme TOML defines the full token set: background/surface/border colors,
text tiers (`fg`, `text_bright`, `text_ui`, `text_muted`, `text_dim`), accent,
success/warning/error, cursor, selection, search highlight colors, the
16-color ANSI palette, and syntax colors for the editor (`command`, `flag`,
`string`, `ghost`).

### Warp YAML themes

`.yaml` / `.yml` files use Warp's schema unchanged — drop a Warp theme in and
it works. Warp carries only the base colors; every other Shift token is
derived from them (chrome/gutter sink away from the text, surfaces lift toward
it, the hairline border is a low-alpha overlay).

```yaml
name: Dracula                # optional; defaults to the file stem
accent: '#bd93f9'
background: '#282a36'
foreground: '#f8f8f2'
details: darker              # darker (default) | lighter
terminal_colors:
  normal:  { black: '#21222c', red: '#ff5555', green: '#50fa7b', yellow: '#f1fa8c',
             blue: '#bd93f9', magenta: '#ff79c6', cyan: '#8be9fd', white: '#f8f8f2' }
  bright:  { black: '#6272a4', red: '#ff6e6e', green: '#69ff94', yellow: '#ffffa5',
             blue: '#d6acff', magenta: '#ff92df', cyan: '#a4ffff', white: '#ffffff' }
background_image:
  path: wall.jpg             # relative paths resolve against the themes dir
  opacity: 50                # Warp spells this 0–100
```

Derived mappings worth knowing: `cursor` and `syntax.command` = `accent`,
`error`/`success`/`warning` = normal red/green/yellow, `syntax.flag` = normal
cyan, `selection` = accent at 25 %.

### Background images

Either theme format can name a wallpaper, drawn under every layer:

```toml
[background_image]
path = "wall.png"   # relative → resolved against ~/.config/shift/themes
opacity = 0.5       # TOML: 0–1 (default 1.0). Warp YAML: 0–100.
```

Pair it with `window_opacity` for a translucent window over the desktop.

## Launch configurations

Warp's launch-configuration YAML, read from
`~/.config/shift/launch_configs/*.yaml` (`.yml` too) — Warp's own files from
`~/.warp/launch_configurations/` drop in unchanged.

```yaml
name: dev-server             # optional; defaults to the file stem
windows:
  - tabs:
      - title: server
        layout:
          cwd: ~/proj/api
          commands:
            - exec: npm run dev
      - title: split
        layout:
          split_direction: vertical    # vertical (default) = side by side
          panes:
            - cwd: ~/proj/web
              is_focused: true
            - split_direction: horizontal
              panes:
                - cwd: ~/logs
                - cwd: ~
active_window_index: 0
active_tab_index: 1
```

Open one from the command palette (⌘P → `Launch configuration: <name>`);
configurations are re-scanned when the palette opens.

Semantics:

- Shift has a single window, so **every window's tabs are concatenated** in
  document order into one flat tab list rather than taking `windows[0]`.
  `active_tab_index` is resolved against that flat list and clamped.
- `cwd` gets `~` / `~/…` expanded via `$HOME`; a YAML-null `cwd: ~` means the
  home directory. A cwd that isn't an existing directory is ignored.
- `commands[].exec` are written to the pane verbatim, in order, **at the pane's
  first prompt** (not before the shell is ready).
- `is_focused` picks the active pane; unknown `split_direction` values and a
  missing `windows` key are errors — the file is logged and skipped, others
  still load. A single-element `panes` list collapses to a plain pane.
- Warp-only keys (e.g. tab `color`) are ignored.

## Notifications & bell

Every event lands in the notifications inbox. For a pane you are not looking
at, an enabled event also alerts: a toast while Shift is focused, a desktop
notification (macOS `osascript display notification`, Linux `notify-send`)
while it is not, plus its sound.

| Event | Switch | Default sound (macOS / Linux) |
|---|---|---|
| Long command finished (≥ `notify_threshold_secs`) | `notify_on_completion` | Glass / `complete` |
| Command failed | `notifications.command_failed` (a long failure also alerts via `notify_on_completion`) | Basso / `dialog-error` |
| Agent task completed | `notifications.agent_task_completed` | Glass / `complete` |
| Program or agent needs attention; agent error | `notifications.needs_attention` | Ping / `message-new-instant` (Basso for errors) |
| Password prompt | `notifications.password_prompt` | Ping / `message-new-instant` |

- **Sounds**: `default` plays the event's sound above; `none` is silent; a
  name is looked up as `<name>.{aiff,wav,caf,mp3,m4a,oga}` in
  `~/Library/Sounds`, `~/.local/share/sounds`, `/System/Library/Sounds` and
  `/usr/share/sounds/freedesktop/stereo`; a value with `/` is a file path.
  Played with `afplay` (macOS) or `paplay` / `canberra-gtk-play` (Linux).
  In Settings, clicking a sound picker steps to the next sound and plays it.
- **Coding agents** are recognized from the running command (`claude`,
  `codex`, `opencode`, `gemini`, also behind `npx`, `env`, `VAR=…` or `&&`).
  Their notifications carry the agent's badge. Turn them on in the agent:
  - Claude Code: `claude config set --global preferredNotifChannel iterm2`
    (sends OSC 9).
  - Codex: in `~/.codex/config.toml`, under `[tui]`:
    `notification_method = "osc9"` (its `auto` rings the bell in terminals
    it does not know) and `notification_condition = "always"`.
  - Any other tool: print OSC 9 (`\e]9;message\a`) or OSC 777
    (`\e]777;notify;title;body\a`).
- The wording decides the kind: "permission", "approval", "waiting for your
  input" → needs attention; "error", "failed" → error; "complete", "done"
  from an agent → task completed.
- `bell` handles BEL from the PTY: `visual` = a brief full-window flash,
  `audible` = a sound (`afplay` on macOS, `paplay` / `canberra-gtk-play` on
  Linux), `both`, or `none`. A bell in a background tab also lands in the
  inbox and marks the tab.
- `SHIFT_NO_NOTIFY=1` suppresses desktop notifications and sounds (they are
  logged instead) — useful for scripted runs.

## Keybinding overrides

See [keyboard-shortcuts.md](keyboard-shortcuts.md) for action names and chord
syntax. Semantics: a `[keybindings]` entry for a base action *replaces* the
default chord **and** unbinds the action's built-in alternate (e.g. overriding
`copy` removes both ⌃⇧C and ⌘C). Invalid chords are logged and skipped —
leaving the action unbound, so check logs if a key stops working.

## Workflows

Warp's YAML schema:

```yaml
name: Push branch upstream
command: git push -u {{remote}} {{branch}}
tags: [git]
arguments:
  - name: remote
    default_value: origin
  - name: branch
```

`{{placeholders}}` become Tab-cyclable spans in the editor. Files from Warp's
community workflow library work unchanged.

## Shell integration details

Injected automatically at spawn; users' own rc files are always sourced.

- Emits OSC 133 A/B/C/D (block boundaries + exit codes) and OSC 7 (cwd).
- Environment: `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `TERM_PROGRAM=shift`.
- Coexists with iTerm2/other integrations that emit their own OSC 133 —
  duplicate markers are tolerated.
- To opt out, unset the injection (zsh: launch with a custom `shell` pointing
  at a wrapper) — Shift then behaves as a plain terminal without blocks.
