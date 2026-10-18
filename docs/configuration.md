# Configuration

## File locations

| Path | Purpose |
|---|---|
| `~/.config/rift/config.toml` | User configuration (hot-reloaded < 300 ms, including external edits; a once-a-second stat check catches edits the file-system events miss) |
| `~/.config/rift/themes/*.toml` | User themes, Rift TOML (file stem = theme name) |
| `~/.config/rift/themes/*.yaml` · `*.yml` | User themes, Warp YAML (same directory, same naming) |
| `~/.config/rift/launch_configs/*.yaml` · `*.yml` · `*.toml` | Launch configurations (Warp-compatible YAML) and Tab Configs (Warp TOML) |
| `~/.warp/tab_configs/*.toml`, `~/.warp-preview/tab_configs/*.toml` (macOS); `${XDG_DATA_HOME:-~/.local/share}/warp-terminal[-preview]/tab_configs/*.toml` (Linux) | Warp's own Tab Configs, read-only (see [Tab Configs](#tab-configs-toml)) |
| `~/.local/share/rift/` | Materialized shell-integration files |
| `~/.local/share/rift/session.toml` | Saved session (every window's tabs, cwds and frame), written atomically; see [Session file](#session-file) |
| `~/.local/share/rift/last-seen-version` | Version whose What's New was last shown (opens it once after an update) |
| `~/.local/share/rift/update.json` | Last update check: when, the newest release's manifest, the version *Later* dismissed |
| `~/.local/share/rift/updates/` | Update downloads (deleted after installing; an interrupted install's leftovers go at the next one), `install.lock` (one window installs at a time) and `relaunch.log` (the restarted app's output until it is confirmed; emptied past 1 MB) |
| `~/.local/share/rift/update-pending.json` | An update in progress (readable by you only; holds the token other windows' `prepare-restart` needs); the new version removes it 10 s after its first frame, and if it dies before that the old app is put back (`update-failed.json`) |
| `~/.local/share/rift/remote/` | Remote access (directory 0700, files 0600): `gateway.key` (the desktop's key phones pin), `devices.json` (paired phones), `pairing.json` (live pairing secrets, hashed), `status.json` and `gateway.lock` (the running gateway), `gateway.log`, `relay.json` (the gateway's secret at the relay), `tunnel.json` (the Cloudflare tunnel's state), `history.jsonl` (the newest 500 connections) |
| `~/.local/share/rift/ssh/` | SSH profiles' shared-connection sockets (`ControlPath=…/%C`, directory 0700) |
| `~/.local/share/rift/stalls/` | Stall reports: the window's main thread blocked over 2 s (no repaint, a Quit that does nothing), or the process not ending after a quit; each `stall-<unix time>-<pid>.txt` says what was under way, how long it lasted and, on macOS, every thread's stack (`/usr/bin/sample`; if Rift ended before `sample` finished, in the `stall-<unix time>-<pid>.sample.txt` beside it); the newest 10 are kept |
| `~/.local/share/rift/restart/` | Each Rift process's windows while an update restarts them (`<pid>.toml` or `main-<pid>.toml`, the session file's format; consumed on relaunch, every window reopening in the one new process; left by a crash, reopened at the next plain launch) |

`XDG_CONFIG_HOME` and `XDG_DATA_HOME` are honored. When the `rift` config or data directory does not exist, an existing one from an earlier name of the app (`shift`, then `runway`) stays active; move it to `rift` when you want the new paths.

### Session file

`session.toml` (and each restart file) is TOML written by Rift; you do not
edit it. Version 4 holds every window: the first window's `active` tab,
`[[tabs]]` and `[window]` frame (logical px, `maximized`, `fullscreen`) stay
at the top level, where versions up to 3 read them, and each further window
is an `[[other_windows]]` table with the same three keys:

```toml
version = 4
active = 1

[[tabs]]
cwd = "/Users/me/code/rift"

[[tabs]]
cwd = "/Users/me"
title = "notes"

[window]
x = 120.0
y = 80.0
width = 1180.0
height = 760.0

[[other_windows]]
active = 0

[[other_windows.tabs]]
cwd = "/Users/me/code/site"

[other_windows.window]
x = 144.0
y = 104.0
width = 1180.0
height = 760.0
```

Files of earlier versions (one window, no `other_windows`) load as one
window; a version-3 build reading a version-4 file reopens the first window
only. A tab may also carry `layout` (its split tree), `color`, `transcript`
and `service` (a durable shell to reattach), `started` and `counted` (the
Elapsed chip). Version 5 adds `launch` to an unsplit tab and to each pane of a
`layout`: the [profile](#profiles) name, own `shell` and `args` the pane was
opened with (absent: the default launch), so a restore opens the same profile
again. The file is rewritten when tabs or panes change, when focus
moves to another pane (so a restart focuses the pane last used), and every
minute while a tab's time counts. A restored split never lays a pane out
under 50 px while the window has room, whatever ratio was saved.

## config.toml reference

All keys optional; omitted keys use the defaults shown. A key written in
your file keeps its value when a release changes that default: Rift never
rewrites your settings to new defaults (only keys you left out follow them).

```toml
font_family   = "Hack"            # embedded Hack or JetBrains Mono, or any installed
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
window_opacity = 0.5              # window ground opacity, clamped 0.01–1.0 (Warp 1–100 %; overlays and prompt chips stay opaque)
window_blur = 20                  # macOS: blur behind a translucent window, 1–64 px (1 = none)
ligatures     = true              # JetBrains Mono ligatures + contextual alternates
app_icon      = "default"         # default | light | mono | ocean | ember | forest | neon:
                                  # the Dock icon (macOS, also while Rift is not running)
                                  # and the window icon (Linux); unknown = default
restore_session = true            # reopen tabs + cwds after a crash or reboot
                                  # (a quit starts fresh; an update restores anyway)
save_transcripts = false          # also save finished blocks' output and replay it
                                  # (marked "restored") on relaunch; off = deleted
tab_layout    = "horizontal"      # horizontal (tabs on top) | vertical (side rail)
tab_close_button = "hover"        # hover (× on the active and hovered tab) | always
                                  # (every tab without an alert/activity dot) | never
                                  # (⌘W, tab menu, palette still close); the vertical
                                  # rail always shows it unless never
tab_close_side = "right"          # right | left (macOS style): the × end of a tab on
                                  # top (the vertical rail keeps it right)
tab_bar       = "always"          # always | hide_fullscreen | on_hover: a hidden strip
                                  # gives the panes its height and slides in at the
                                  # window's top edge (tabs on top only)
new_tab_placement = "end"         # end | after_current — where ⌘T and + put the new tab
                                  # (restores, launch configs and ctl always append)
tab_bar_with_one_tab = true       # false: no tab in the strip (and no vertical rail)
                                  # while one tab is open; the bar's buttons and + stay
toolbar_left  = ["explorer"]      # toolbar buttons left of the tabs, in order
toolbar_right = ["notifications", "review"]  # and at the strip's right end; each of
                                  # explorer | review | notifications | palette |
                                  # settings, in neither list = hidden (listed twice:
                                  # the first place wins; an unknown name is skipped)
sticky_command_header = true      # pin a block's header while its output scrolls past
block_spacing = "compact"         # compact (blocks flush) | normal (a blank row above
                                  # each block header)
block_dividers = false            # a rule between blocks
jump_to_bottom_button = true      # a button over a block running past the pane's bottom
                                  # that scrolls to its end
dim_inactive_panes = true         # wash the unfocused split panes (Warp)
inactive_pane_dim = 10            # that wash in %, clamped 1–40 (half on a light theme)
focus_follows_mouse = false       # the split pane under the pointer takes focus
durable_sessions = true           # shells live in the session service: they keep
                                  # running through an update's restart or a
                                  # crash and reattach on launch (a quit ends
                                  # them; false keeps them in the window process)
check_for_updates = true          # look for a new release at launch and every 6 h
                                  # ("Update Rift" chip in the tab bar)
control_api   = "navigate"        # off | navigate (list/status/focus/blocks, and an
                                  # update's prepare-restart) | full (also
                                  # split/open/send-text); `rift-terminal ctl`.
                                  # Linux, off: the socket still takes rift://
                                  # links from a second rift-terminal process
remote_access = false             # phone companion (off by default, never turned
                                  # on for you): windows start the gateway
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
prompt_chips  = ["elapsed", "cwd", "git_branch", "git_diff", "docker"]  # the Rift prompt's chips, in order (Settings › Input ›
                                  # Prompt chips edits them): cwd, git_branch, git_diff,
                                  # github_pr, user, host, python, php, docker,
                                  # date, time12, time24, elapsed
elapsed_counts = "always"         # always | running — what the elapsed chip counts: the time since the tab's
                                  # first command, or only while a command runs in it (time logging; kept
                                  # across an update's restart (a quit starts fresh); Settings › Input › Time on this tab counts)
copy_on_select = true             # copy the selection on mouse release
autosuggestions = true            # inline ghost text after the cursor
completions_menu = true           # completion panel (Tab still accepts the ghost text)
shell_completions = true          # also ask your shell's own completion functions (zsh,
                                  # bash, fish), and the remote machine of an SSH
                                  # profile's pane; false (also on reload) stops the
                                  # warm shells; they always stop when Rift quits
shell_widgets = "auto"            # auto | rift — ⌃R, ⌃T, ⌥C run your shell's own widgets
                                  # (fzf, atuin) where its config binds them; rift: always
                                  # Rift's (the history pane for ⌃R)
autocomplete_pairs = true         # auto-close quotes/brackets; Backspace removes empty pairs
vim_mode      = false             # modal (vim) editing in the command editor
notify_on_completion = true       # alert when a long command finishes out of view
notify_threshold_secs = 10        # what counts as long (stepper: 0–3600, ±5)
show_progress = true              # programs' OSC 9;4 progress: a bar under the tab,
                                  # the Dock badge (macOS); false hides both
bell          = "both"            # visual | audible | both | none
confirm_close_running = true      # warn before closing a pane/tab/window with a running job
confirm_multiline_paste = true    # ask before a paste with newlines reaches a program
                                  # without bracketed paste (each newline presses Enter)
working_directory = "previous"    # new tabs/panes: previous | home | "/abs/path" (~ expands)
alias_expansion = false           # Space after an alias replaces it with its definition
error_underline = true            # red underline under unknown command names
command_corrections = true        # "Did you mean …" after a failed command-not-found
subshell_offer = true             # offer Rift's hooks in docker exec … bash, poetry shell…
subshell_commands = []            # more such commands, as regular expressions
subshell_denylist = []            # commands never to offer it for ("Don't ask again")
rich_input_submit = "enter"       # rich input composer: "enter" sends (⇧⏎ new line) |
                                  # "cmd_enter" (⌘⏎ sends, ⏎ new line)
rich_input_auto = false           # open the composer by itself when a coding agent waits
                                  # (its own notification, or 1 s of quiet after output)
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
hotkey_window = false             # the hotkey slides a drop-down window from the top of
                                  # the screen instead of showing/hiding the whole app
hotkey_window_height = 40         # its height, percent of the screen (20–100)
hotkey_window_hide_on_blur = true # it hides when it loses keyboard focus
launch_at_login = false           # macOS: Rift opens at login (its login item); shows
                                  # the login item as it is (System Settings › Login
                                  # Items can remove it) and only Settings changes it
menu_bar_icon = true              # macOS: a Rift icon in the menu bar (New Window,
                                  # Settings…, Quit Rift Completely); after the window
                                  # quits, the phone gateway shows it while it runs
                                  # (Open Rift, Quit Rift Completely: stops the
                                  # gateway and the shells phones opened)
link_editor   = "rift"            # ⌘-clicked file paths / file links: system (default
                                  # app) | rift (viewer pane) | an editor id below
link_click    = "open"            # plain click on a link: open (open it at once)
                                  # | prompt (show it and ⌘ Click); ⌘-click always opens
file_editor   = "rift"            # explorer and code review ↗: rift | system | editor id
                                  # editors: vscode cursor windsurf zed sublime intellij
                                  # phpstorm webstorm rustrover (macOS: the .app in
                                  # /Applications or ~/Applications, else the CLI on PATH;
                                  # a rift://action/open_file link's line goes to the
                                  # CLI on PATH first)
explorer_hidden_files = true      # file explorer lists dotfiles (.git and ignored stay hidden)
# shell        = "/bin/zsh"       # default: $SHELL
# default_profile = "dev"         # the profile ⌘T, + and splits open (see Profiles);
#                                 # unset: `shell` as before
discover_shells = true            # the shells in /etc/shells are profiles too (zsh, fish…)

# [[profiles]]                    # see Profiles below
# name    = "dev"                 # required, unique
# command = "fish"                # absolute path, ~/…, or a name on PATH; unset: `shell`
# args    = ["--private"]         # after Rift's own integration arguments
# cwd     = "~/code"              # start directory (a pane's own directory wins)
# title   = "Dev"                 # tab title of a tab it opens
# color   = "green"               # red green yellow blue magenta cyan
# icon    = "python"              # + menu glyph: python php docker github git folder
#                                 # user laptop; unset: a terminal
# [profiles.env]                  # this profile's panes only
# NODE_ENV = "development"

# [[ssh_profiles]]                # palette "SSH: <name>" (hosts from ~/.ssh/config
# name = "prod"                   # are listed automatically)
# host = "me@prod.example.com"    # user@host or a Host alias
# args = ["-p", "2222"]           # extra ssh arguments
#                                 # the profile shares its connection
#                                 # (ControlMaster=auto, ControlPath=<data dir>/ssh/%C,
#                                 # ControlPersist=60) unless these args or your ssh
#                                 # config set a ControlPath; remote completions use it

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

`.yaml` / `.yml` files use Warp's schema unchanged (solid colors only: a gradient
`accent` or `background` is not read yet) — drop a Warp theme in and
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

Warp's launch-configuration YAML and its newer TOML Tab Configs, read from
`~/.config/rift/launch_configs/` (`*.yaml`, `*.yml`, `*.toml`) — Warp's own
files from `~/.warp/launch_configurations/` drop in unchanged. Files over
256 KiB are skipped.

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

- A launch configuration opens in the current window, so **every window's
  tabs are concatenated** in document order into one flat tab list rather
  than taking `windows[0]`.
  `active_tab_index` is resolved against that flat list and clamped.
- `cwd` gets `~` / `~/…` expanded via `$HOME`; a YAML-null `cwd: ~` means the
  home directory. A pane without `cwd`, or with one that isn't an existing
  directory, opens in the `working_directory` start directory.
- `commands[].exec` are written to the pane verbatim, in order, **at the pane's
  first prompt** (not before the shell is ready).
- `is_focused` picks the active pane; unknown `split_direction` values and a
  missing `windows` key are errors — the file is logged and skipped, others
  still load; the palette names skipped files once, with the reason, in a
  toast (again only when the set of broken files changes). A single-element `panes` list collapses to a plain pane.
- Tab `color` maps to Rift's tab colors (`red`, `green`, `yellow`, `blue`,
  `magenta`, `cyan`); `black` and `white` have no Rift equivalent: the tab
  opens with no color. Other Warp-only keys are ignored.

### Tab Configs (TOML)

Source: <https://docs.warp.dev/terminal/windows/tab-configs> (Warp's docs, read
2026-10-17). A Tab Config is one TOML file per tab layout. Warp stores them in
`~/.warp/tab_configs/` (Preview: `~/.warp-preview/tab_configs/`) on macOS and
`${XDG_DATA_HOME:-~/.local/share}/warp-terminal/tab_configs/` (Preview:
`warp-terminal-preview`) on Linux. Rift loads `*.toml` from its own
`launch_configs/` directory and, **read-only**, from those Warp directories;
when a name exists in both, Rift's own file wins.

```toml
name = "Server Plus Logs"     # required by Warp; Rift falls back to the file stem
title = "api {{branch}}"      # optional tab title; {{param}} allowed
color = "blue"                # black red green yellow blue magenta cyan white

[params.branch]               # optional; Rift substitutes `default`
type = "branch"               # text (default) | branch | repo
default = "main"

[[panes]]                     # flat list; the FIRST entry is the root
id = "root"
split = "horizontal"          # horizontal = left to right, vertical = top to bottom
children = ["server", "logs"] # at least two pane ids, in visual order

[[panes]]
id = "server"
type = "terminal"             # terminal | agent | cloud
directory = "~/code/api"      # ~ and {{param}} allowed
commands = ["make serve"]     # run in sequence
shell = "zsh"                 # this pane's program (a path or a name on PATH)
is_focused = true             # at most one pane

[[panes]]
id = "logs"
type = "terminal"
```

Rift honours `name`, `title`, `color`, `directory`, `commands`, `is_focused`,
`shell`, `split`, `children` and `params.*.default`, and adds per-pane
`profile = "<name>"` (a [profile](#profiles)) and `args = [...]` (the pane's
own arguments). YAML launch-configuration panes take the same `shell`,
`profile` and `args` keys. Differences from Warp: all pane
types open a local terminal; a `{{param}}` with no
`default` (including `autogenerated_branch_name`) makes the file fail to load
(named in a toast when the palette opens), because Rift has no fill-in dialog;
`color = "black"` or `"white"` opens the tab with no color; unknown keys are ignored; at most 64 panes
and 16 levels of nesting. A Tab Config holds one tab. Splits are always
equal-sized, as in Warp.

**Save current layout as launch configuration** (palette) writes one Tab Config
`.toml` per open tab to `~/.config/rift/launch_configs/` (`layout-<time>-1.toml`,
`-2`, …), never overwriting an existing file, and never writing to Warp's
directories. Each pane keeps its `profile`, `shell` and `args`.

## Profiles

A profile is a named set of launch settings (Windows Terminal profiles):

```toml
default_profile = "dev"

[[profiles]]
name = "dev"
command = "fish"                 # an executable: /path, ~/path or a name on PATH
args = ["--private"]
cwd = "~/code/app"
title = "Dev"
color = "green"

[profiles.env]
NODE_ENV = "development"

[[profiles]]
name = "clean bash"
command = "/bin/bash"
args = ["--noprofile"]
```

- **Precedence:** the global settings (`shell`, `working_directory`), then the
  profile, then what the pane itself names (a launch configuration's
  `directory`, `shell`, `args`). A pane `shell` replaces the profile's
  `command` and its `args`; pane `args` replace the profile's. A pane naming
  neither a profile nor a shell gets `default_profile`.
- **Installed shells:** every executable listed in `/etc/shells` is a profile
  named after it (`zsh`, `bash`, `fish`; one per name, the one `PATH` finds
  first), unless `discover_shells = false`. A `[[profiles]]` entry with the
  same name wins.
- **Opening one:** the ⌄ beside the tab bar's + (or right-click +) lists the
  profiles (the default one bold) with their shortcuts, ⌃⇧1…⌃⇧9
  (`new_tab_profile_1`…`_9`, in menu order); the palette has
  `New tab: <name>`, `Split right: <name>` and `Split down: <name>`; a launch
  configuration pane names one with `profile = "<name>"`. ⌘T, + and ⌘D/⇧⌘D
  open the default profile, as does every other new tab (the first tab, ⌘N,
  links, `rift-terminal <path>`, `ctl open`, the phone, the drop-down
  window): in the profile's `cwd` unless a folder was named, with its
  `title` and `color`. Settings › General lists the profiles
  (read-only; edit them in config.toml).
- **Shell integration** follows the program each pane runs: zsh, bash and fish
  load Rift's hooks, as do completions from the shell itself
  (`shell_completions`).
- **Environment** values reach that profile's panes only; the app and other
  panes never see them.
- **Identity:** a pane remembers the profile name (not its values) it was
  opened with. Duplicate tab, save layout and session restore open that
  profile again; a reattached durable shell keeps running as it was. Editing a
  profile never changes a running pane.
- **Errors:** an unknown profile name, or a `command` that is not an executable
  file, is a toast naming the profile and the path, and nothing opens (a
  launch configuration opens none of its tabs). Only a pane Rift must open
  (a window's first tab, a restored tab, a link, a durable pane whose shell
  had to be started again) runs the global `shell` instead, with that toast. An invalid `[[profiles]]` entry (no or repeated
  `name`, an unknown `color`, an empty `command`, a NUL byte, `=` in an
  environment name) makes config.toml not apply, like any parse error.

## Notifications & bell

Every event lands in the notifications inbox. For a pane you are not looking
at, an enabled event also alerts: a toast while Rift is focused, a desktop
notification (posted by Rift.app with its icon on macOS, `osascript` outside
an app bundle; `notify-send` on Linux) while it is not, plus its sound.

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
  `codex`, `opencode`, `gemini`, also behind `npx`, `env`, `sudo`, `doas`,
  `nice`, `caffeinate`, `VAR=…` or `&&`, and inside `sh -c '…'` or
  `env -S '…'`).
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
- `RIFT_RELAUNCH_SCRIPT` (tests): the app an update restarts runs that
  `--script` instead of starting plainly (the reopened windows are its
  own; `window <n>` reaches them). A scripted run with
  `RIFT_PERSIST_SESSION` counts as a plain launch: it also reopens, as its
  windows, the windows an interrupted update left in `restart/`.
- `RIFT_TEST_NO_FIRST_FRAME` (tests): a headless run does not count its
  start as a presented frame, so it confirms through the fallback timer (the
  planned wait + 20 s) like a window that never draws.
- `RIFT_TEST_HEADLESS_EDITOR` (tests): a headless run launches the
  `link_editor` / `file_editor` editor (a fake one first on `PATH`) instead
  of only reporting it in a toast.
- `RIFT_CLI_DIR`: the folder the palette's *Install 'rift' command* puts the
  `rift` link in (default `~/.local/bin`).

## Keybinding overrides

See [keyboard-shortcuts.md](keyboard-shortcuts.md) for action names and chord
syntax. Semantics: a `[keybindings]` entry for a base action *replaces* the
default chord **and** unbinds the action's built-in alternate (e.g. overriding
`copy` removes both ⌃⇧C and ⌘C). Invalid chords are logged and skipped —
leaving the action unbound, so check logs if a key stops working.

## Shell integration details

Injected automatically at spawn; users' own rc files are always sourced.

- Emits OSC 133 A/B/C/D (block boundaries + exit codes) and OSC 7 (cwd).
  `C` carries the command line as the shell read it, percent-encoded
  (`133;C;cmdline_url=…`, kitty's form; fish 4 sends it too). bash sends it
  only when the command entered the history (`HISTCONTROL` or
  `set +o history` can leave it out); then the header reads the line from
  the screen.
- Environment: `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `TERM_PROGRAM=rift`, `TERM_PROGRAM_VERSION` (Rift's version, not the
  launching terminal's).
- Coexists with iTerm2/other integrations that emit their own OSC 133 —
  duplicate markers are tolerated.
- To opt out, unset the injection (zsh: launch with a custom `shell` pointing
  at a wrapper) — Rift then behaves as a plain terminal without blocks.
