# Configuration

## File locations

| Path | Purpose |
|---|---|
| `~/.config/rift/config.toml` | User configuration (hot-reloaded < 300 ms, including external edits; a once-a-second stat check catches edits the file-system events miss) |
| `~/.config/rift/themes/*.toml` | User themes, Rift TOML (file stem = theme name) |
| `~/.config/rift/themes/*.yaml` · `*.yml` | User themes, Warp YAML (same directory, same naming) |
| `~/.config/rift/launch_configs/*.yaml` · `*.yml` | Launch configurations (Warp-compatible YAML) |
| `~/.local/share/rift/` | Materialized shell-integration files |
| `~/.local/share/rift/session.toml` | Saved session (tabs + cwds), written atomically |
| `~/.local/share/rift/last-seen-version` | Version whose What's New was last shown (opens it once after an update) |
| `~/.local/share/rift/update.json` | Last update check: when, the newest release's manifest, the version *Later* dismissed |
| `~/.local/share/rift/updates/` | Update downloads (deleted after installing; an interrupted install's leftovers go at the next one), `install.lock` (one window installs at a time) and `relaunch.log` (the restarted app's output until it is confirmed; emptied past 1 MB) |
| `~/.local/share/rift/update-pending.json` | An update in progress (readable by you only; holds the token other windows' `prepare-restart` needs); the new version removes it 10 s after its first frame, and if it dies before that the old app is put back (`update-failed.json`) |
| `~/.local/share/rift/remote/` | Remote access (directory 0700, files 0600): `gateway.key` (the desktop's key phones pin), `devices.json` (paired phones), `pairing.json` (live pairing secrets, hashed), `status.json` and `gateway.lock` (the running gateway), `gateway.log`, `relay.json` (the gateway's secret at the relay), `tunnel.json` (the Cloudflare tunnel's state), `history.jsonl` (the newest 500 connections) |
| `~/.local/share/rift/restart/` | Each window's layout while an update restarts them (consumed on relaunch; left by a crash, reopened at the next plain launch) |

`XDG_CONFIG_HOME` and `XDG_DATA_HOME` are honored. When the `rift` config or data directory does not exist, an existing one from an earlier name of the app (`shift`, then `runway`) stays active; move it to `rift` when you want the new paths.

## config.toml reference

All keys optional; omitted keys use the defaults shown.

```toml
font_family   = "JetBrains Mono"  # embedded JetBrains Mono or Hack, or any installed
                                  # fixed-width outline font (Menlo, Monaco, SF Mono…);
                                  # unknown names fall back to JetBrains Mono
font_size     = 13.0              # settings stepper clamps 8–28 (⌘= / ⌘- / ⌘0 also write this)
line_height   = 1.2               # multiplier, clamped 1.0–2.2 when applied
cursor_style  = "block"           # block | bar | underline
cursor_blink  = true              # the cursor toggles every 550 ms
theme         = "dark-default"    # builtin, or a stem in ~/.config/rift/themes
theme_light   = "light-default"   # theme used while the OS is light (needs sync_os_theme)
sync_os_theme = false             # follow the OS appearance: theme = dark, theme_light = light
scrollback    = 10000             # lines kept per pane, clamped 0–1000000
                                  # (the Settings stepper offers 1000–1000000)
window_padding = 8.0              # left/right content padding in px, clamped 0–40
                                  # (single-pane layout; splits keep their gutter)
window_opacity = 1.0              # window ground opacity, clamped 0.01–1.0 (Warp 1–100 %; overlays and prompt chips stay opaque)
window_blur = 1                   # macOS: blur behind a translucent window, 1–64 px (1 = none)
ligatures     = true              # JetBrains Mono ligatures + contextual alternates
restore_session = true            # reopen tabs + cwds after a crash or reboot
                                  # (a quit starts fresh; an update restores anyway)
save_transcripts = false          # also save finished blocks' output and replay it
                                  # (marked "restored") on relaunch; off = deleted
tab_layout    = "horizontal"      # horizontal (tabs on top) | vertical (side rail)
durable_sessions = true           # shells live in the session service: they keep
                                  # running through an update's restart or a
                                  # crash and reattach on launch (a quit ends
                                  # them; false keeps them in the window process)
check_for_updates = true          # look for a new release at launch and every 6 h
                                  # ("Update Rift" chip in the tab bar)
control_api   = "navigate"        # off | navigate (list/status/focus/blocks, and an
                                  # update's prepare-restart) | full (also
                                  # split/open/send-text); `rift-terminal ctl`
remote_access = false             # phone companion: windows start the gateway
                                  # (`rift-terminal remote --managed`; log in
                                  # <data dir>/remote/gateway.log); with
                                  # control_api = "off" the socket then answers
                                  # only the gateway's requests (status,
                                  # blocks, screen, output, menu, diff,
                                  # complete, remote-input, remote-open,
                                  # remote-close, remote-status)
remote_listen = "0.0.0.0:7878"    # where the gateway listens
remote_relay  = "wss://rift-relay.playhaws.com"
                                  # relay for phones outside your network:
                                  # Rift's public relay by default ("": none,
                                  # phones connect only on your network;
                                  # scripted runs skip it); `rift-terminal relay` or
                                  # packaging/relay.Dockerfile hosts one;
                                  # scripts/relay-local.sh runs a temporary one
                                  # on this machine and sets this key
remote_when_closed = false        # paired phones that may run commands can open
                                  # tabs while no Rift window is open: the gateway
                                  # stays up and keeps them in the session service
                                  # (needs durable_sessions); Rift shows them as
                                  # tabs when it opens
remote_tunnel = false             # Cloudflare tunnel: a relay on this computer
                                  # behind a Cloudflare quick tunnel (needs
                                  # cloudflared); replaces remote_relay while on.
                                  # New address each start: phones pair again
input_position = "bottom"         # bottom | top — where the command editor sits
blocks_start = "bottom"           # bottom | top — blocks fill up from the editor (Warp), or start at the pane's top
prompt        = "rift"           # rift = context chips + chevron
                                  # shell   = keep the shell's own PS1 on screen, editor shows ›
prompt_chips  = ["cwd", "git_branch", "git_diff"]  # the Rift prompt's chips, in order (Settings › Input ›
                                  # Prompt chips edits them): cwd, git_branch, git_diff,
                                  # github_pr, user, host, python, php, docker,
                                  # date, time12, time24, elapsed
copy_on_select = false            # copy the selection on mouse release
autosuggestions = true            # inline ghost text after the cursor
completions_menu = true           # completion panel (Tab still accepts the ghost text)
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
                                  # (hold Shift to select locally); focus events go regardless
option_as_meta = false            # macOS: left Option sends ESC-prefixed Meta keys instead
                                  # of composing characters (Linux Alt is always Meta)
# right_option_as_meta = false    # macOS: the right Option key; unset = follows option_as_meta
middle_click_paste = true         # middle-click pastes the clipboard into the pane under it
wheel_scroll_lines = 3            # terminal lines per mouse-wheel notch (1–20)
clipboard_access = "write"        # OSC 52: off | write (programs may copy) | read_write
                                  # (also read: any program or ssh host can see your clipboard)
global_hotkey = ""                # system-wide show/hide chord, e.g. "alt+space";
                                  # empty = off (X11 on Linux)
link_editor   = "system"          # ⌘-clicked file paths / file links: system (default
                                  # app) | rift (viewer pane) | an editor id below
link_click    = "prompt"          # plain click on a link: prompt (show it and ⌘ Click)
                                  # | open (open it at once); ⌘-click always opens
file_editor   = "rift"            # explorer and code review ↗: rift | system | editor id
                                  # editors: vscode cursor windsurf zed sublime intellij
                                  # phpstorm webstorm rustrover (macOS: the .app in
                                  # /Applications or ~/Applications, else the CLI on PATH)
explorer_hidden_files = true      # file explorer lists dotfiles (.git and ignored stay hidden)
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
toasts = true                     # in-window cards while Rift is focused
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

The Settings tab (⌘,) edits the same file live; the hot-reload watcher
applies external edits without restarting.

- **Invalid or unreadable file**: it is not applied. The last good config
  stays live (at startup, the defaults with `control_api = "off"`), a toast
  names the error, and Settings refuses to save over the broken file until it
  is fixed. A missing file means defaults.
- **Saves are atomic**: Rift writes a temporary file next to `config.toml`,
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

A user theme is `~/.config/rift/themes/<name>.{toml,yaml,yml}`, referenced by
file stem (`toml` wins if several exist). It appears in Settings → Themes and
the palette's theme switcher. Editing the selected theme file reloads its
colors without changing `config.toml` or restarting.

### Rift TOML themes

A theme TOML must set every token (only `background_image` is optional):
`bg`, `bg_chrome`, `bg_gutter`, `surface`, `border`, `pane_border_inactive`,
`fg`, `text_bright`, `text_ui`, `text_dim`, `text_muted`, `accent`,
`success`, `error`, `warning`, `cursor`, `selection`, `search_match`,
`search_active`, `ansi` (16 colors), and a `[syntax]` table with `command`,
`flag`, `string` and `ghost`. Colors are `#rgb`, `#rrggbb` or `#rrggbbaa`. A
file with a missing token or a bad color does not load: Rift shows a toast
naming the error, keeps `dark-default`, and the picker labels the file
"(invalid theme file)".

### Warp YAML themes

`.yaml` / `.yml` files use Warp's schema unchanged — drop a Warp theme in and
it works. Warp carries only the base colors; every other Rift token is
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
path = "wall.png"   # relative → resolved against ~/.config/rift/themes
opacity = 0.5       # TOML: 0–1 (default 1.0). Warp YAML: 0–100.
```

Pair it with `window_opacity` for a translucent window over the desktop.

## Launch configurations

Warp's launch-configuration YAML, read from
`~/.config/rift/launch_configs/*.yaml` (`.yml` too) — Warp's own files from
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

- Rift has a single window, so **every window's tabs are concatenated** in
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
at, an enabled event also alerts: a toast while Rift is focused, a desktop
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
  name is looked up as `<name>.{aiff,aif,wav,caf,mp3,m4a,oga}` in
  `~/Library/Sounds`, `~/.local/share/sounds`, `/System/Library/Sounds` and
  `/usr/share/sounds/freedesktop/stereo`; a value with `/` is a file path.
  Played with `afplay` (macOS) or `paplay` / `canberra-gtk-play` (Linux).
  In Settings, a sound picker opens the list of sounds; picking one plays it.
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
- `RIFT_NO_NOTIFY=1` suppresses desktop notifications and sounds (they are
  logged instead) — useful for scripted runs.
- `RIFT_PERSIST_SESSION=1` lets a `--script` run save and restore the
  session file and use the session service (scripted runs leave both alone
  by default: their shells are local, so no service outlives a test); point
  `XDG_DATA_HOME` at a scratch directory when using it.
- `RIFT_UPDATE_URL` replaces the update manifest's URL (tests: a
  `file://` manifest, whose assets may then be `file://` too). Scripted runs
  check for updates only when it is set. The manifest still needs a valid
  `latest.json.sig` beside it.
- `RIFT_UPDATE_TEST_KEY` (tests): an extra Ed25519 public key (base64 of
  the raw 32 bytes) trusted for the update manifest's signature, so a test
  can sign with a throwaway key. Honoured only while `RIFT_UPDATE_URL` is a
  `file://` URL; an https manifest is verified with the release keys alone.
- `RIFT_RELAUNCH_SCRIPT` / `RIFT_RELAUNCH_WINDOW_SCRIPT` (tests): the app
  an update restarts runs that `--script` (reopened windows: the second one,
  headless) instead of starting plainly. A scripted run with
  `RIFT_PERSIST_SESSION` counts as a plain main-window launch: it also
  reopens windows an interrupted update left in `restart/`.
- `RIFT_TEST_NO_FIRST_FRAME` (tests): a headless run does not count its
  start as a presented frame, so it confirms through the fallback timer (the
  planned wait + 20 s) like a window that never draws.

## Keybinding overrides

See [keyboard-shortcuts.md](keyboard-shortcuts.md) for action names and chord
syntax. Semantics: a `[keybindings]` entry for a base action *replaces* the
default chord **and** unbinds the action's built-in alternate (e.g. overriding
`copy` removes both ⌃⇧C and ⌘C). Invalid chords are logged and skipped —
leaving the action unbound, so check logs if a key stops working.

## Shell integration details

Injected automatically at spawn; users' own rc files are always sourced.

- Emits OSC 133 A/B/C/D (block boundaries + exit codes) and OSC 7 (cwd).
- Environment: `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `TERM_PROGRAM=rift`, `TERM_PROGRAM_VERSION` (Rift's version, not the
  launching terminal's).
- Coexists with iTerm2/other integrations that emit their own OSC 133 —
  duplicate markers are tolerated.
- To opt out, unset the injection (zsh: launch with a custom `shell` pointing
  at a wrapper) — Rift then behaves as a plain terminal without blocks.
