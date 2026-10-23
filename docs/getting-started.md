# Getting started

This guide takes you from the first launch to everyday use. Shortcuts are
macOS defaults; on Linux ⌘ is the Super key, and most actions also have a
⌃⇧ chord (see [keyboard shortcuts](keyboard-shortcuts.md)).

## First launch

Rift starts your login shell (`$SHELL`, or `shell = "…"` in config.toml).
On macOS it runs as a login shell like Terminal.app, so `~/.zprofile`
(Homebrew's `brew shellenv`) loads before `~/.zshrc`. On the first start it writes its shell hooks to `~/.local/share/rift/` and
loads them **after** your own rc files, so your prompt, aliases and plugins
keep working. zsh, bash and fish are supported; you edit no rc file.

You see two areas:

- **Blocks** above: each command you run, with its output.
- **The command editor** at the bottom: the line you are typing, with the
  current directory and git branch above it.

If commands do not become blocks, the shell is not loading the hooks (see
[Troubleshooting](#troubleshooting)). Rift then works as a classic terminal.

## Run commands

- Type and press ⏎. The command becomes a block with a green (`✓ 0`) or red
  (`✕ N`) badge and its duration.
- ⇧⏎ adds a second line; ⏎ runs the whole command.
- Gray text after the cursor is a suggestion from your history: Tab or → accepts
  it, ⌃→ accepts one word. The completion panel above the editor lists commands, subcommands, flags
  and paths; ↓ then ↑/↓ choose, Tab inserts.
- ↑ opens the history menu: your earlier commands that start with what you typed, previewed on the line as you move (↑ older, ↓ newer). ⏎ runs one, ⇥ keeps it to edit, and ↓ past the newest or Esc puts back what you typed.
- ⌃R opens the history pane above the command line: type to narrow it, ⏎ runs, ⇥ edits first, ⌘⏎ runs in a new split. ⌃R again cycles its scopes; **Results** finds past runs by their output too (`is:failed`, `since:7d`, `host:prod` filter), with a preview before you rerun.
- ⌃C interrupts, ⌃D sends end-of-file, ⌃L redraws.
- When a program takes over the terminal (vim, less, htop, ssh, a password
  prompt), keys go straight to it. The editor comes back at the next prompt.

## Work with blocks

| To | Do |
|---|---|
| Select a block | ⌘↑ / ⌘↓, or click its header or output |
| Copy its command / output | ⇧⌘C / ⌥⇧⌘C (the newest block when none is selected) |
| Put its command back in the editor | ⌘I |
| Open the block menu | Right-click, or ⌃M |
| Re-run it | Block menu → *Re-run command* |
| Collapse it | Click the chevron `▾` |
| Bookmark it and jump between bookmarks | ⌘B, then ⇧⌘↑ / ⇧⌘↓ |
| Show only matching output lines | ⌥⇧F and type (⌥C case, ⌥R regex, ⌥I invert, ⌥↑ / ⌥↓ context lines); ⌥⇧F again turns it off, Esc clears it |
| Select several blocks | ⌘-click each, or ⇧-click a range; the block menu then acts on all of them |
| Compare two outputs | Select two → block menu → *Compare outputs*, or block menu → *Compare with previous run* |
| Copy it as HTML or Markdown, save its output | Block menu → *Copy as HTML* / *Copy as Markdown* / *Save output…* |
| Feed its output to another command | Block menu → *Send output to command…* |
| Find text anywhere | ⌘F; ⌥C toggles case, ⌥R regex. With a block selected, ⌘F searches only that block |
| Clear the screen but keep history | `clear` |
| Delete all blocks and scrollback | ⌘K |

## Tabs, panes and windows

| To | Do |
|---|---|
| New tab / close pane | ⌘T / ⌘W |
| Split right / down | ⌘D / ⇧⌘D |
| Move between panes | ⌥⌘ + arrow |
| Resize a pane | ⌃⌘ + arrow, or drag the divider |
| Zoom a pane | ⇧⌘⏎ |
| Switch to any tab, pane, window or recent block | ⇧⌘O |
| Type into every pane of the tab at once | ⌥⌘I |
| Reopen a closed tab | ⇧⌘T |
| Open another window | ⌘N (same app; closing it ends its shells only) |
| Open a tab with a profile | Click the ⌄ beside +, or ⌃⇧1…⌃⇧9 (Settings › Profiles) |
| Rename, color, pin, group or close tabs | Right-click the tab; ⌘-click tabs first to group several |
| Tabs in a side rail | Settings → Appearance → *Tab position: Side*; ⇧⌘F searches it |
| Lock a pane against typing, or restart it | Right-click the terminal → *Make read-only* / *Restart pane* |

Quitting Rift (⌘Q, or closing its last window) ends every tab and shell: the
next launch starts fresh with one tab where the first window was. An
update's restart, a crash or a reboot instead bring every window, its tabs,
splits and directories back
(`restore_session`), and the shells themselves keep running and reattach
(durable sessions, on by default; see [durable
sessions](features.md#durable-sessions)). Turn on *Save block output for
restore* to also bring back the finished blocks. While a phone is connected,
quitting asks first.

When a new release is out, **Update Rift** appears in the tab bar: it
installs the update and restarts Rift with your tabs and running shells (see
[updates](features.md#updates)).

## Find anything: the palette

⌘P lists every action with its shortcut, plus settings, themes, launch
configurations, SSH hosts and recent commands. Type a few letters; ⏎ runs.
If you forget a shortcut, the palette shows it.

Other keyboard-first tools:

- **Hint mode** ⇧⌘H: letters appear over every visible link, path and block.
  Type one to open it; ⇧ + letter copies it.
- **Composer** ⌃⇧G or ⌘E: write a long prompt for a coding agent or any
  full-screen program in Rift's editor; ⏎ sends it as one paste.
- **Mark mode** ⇧⌘M: select scrollback from the keyboard.
- **File explorer** ⇧⌘E: the project tree; click a file to view it (Markdown
  renders, and its shell blocks have a *Run…* button that asks first),
  right-click to copy its path, add a file or reveal it.
- **Code review** ⇧⌘G: the diff of your changes; stage, revert, commit and
  push, or *Send to agent…* a hunk to a coding agent in another pane.
- **Notifications** ⇧⌘A or the inbox button at the top right: commands that
  finished or failed while you were elsewhere, coding-agent and program
  notifications, password prompts and bells. A blue dot on the button means
  something is unread; a toast pops up for tabs you are not looking at.
  Settings → Notifications picks what alerts and which sounds play. To get
  Claude Code's notifications run
  `claude config set --global preferredNotifChannel iterm2`; for Codex see
  [configuration](configuration.md#notifications--bell).

## Settings and themes

⌘, opens Settings. Every change is written to
`~/.config/rift/config.toml` and applies at once; editing the file by hand
applies the same way. Reference: [configuration](configuration.md).

- **Themes**: Settings → Themes, or palette → *Theme picker (live preview)*:
  13 built in, each previewed as you move, Esc goes back. Put your own TOML
  or Warp YAML theme in `~/.config/rift/themes/`. *Sync with OS appearance*
  follows light and dark mode. Palette → *Pane theme: name* gives one pane
  its own colours.
- **Hide secrets**: palette → *Toggle secret masking* (`mask_secrets`) hides
  keys and tokens on screen and in copies, before you share your screen.
- **Font and size**: Settings → Appearance; ⌘= / ⌘- / ⌘0 change the size.
- **Shortcuts**: Settings → Keybindings (⌃⌘K): click a row and press the new
  chord. Or add a `[keybindings]` table to config.toml.
- **New tabs start in**: Settings → General; the previous directory, home or
  a path.
- **Your own prompt**: set *Prompt* to *Shell (PS1)* to keep starship, p10k or
  your PS1 visible.

## Save commands for later

- **Launch configurations**: YAML files in
  `~/.config/rift/launch_configs/` (Warp format) open a set of tabs, splits,
  directories and startup commands from the palette. File → *Save Layout as
  Launch Configuration* writes the current layout.
- **Profiles**: Settings › Profiles (or `[[profiles]]` in config.toml) name
  a program, arguments, folder, environment, title, colour and theme; add,
  edit, reorder and pick the default there; open one from the ⌄ beside +.
- **Actions**: `[[actions]]` in config.toml chain Rift actions, typed text
  and launch configurations under one name and chord; palette → *Action: name* runs one.
  Reference: [configuration](configuration.md#user-actions).

## Remote machines

Palette → *SSH: host* opens a tab running your system `ssh` for any `Host`
in `~/.ssh/config` or any `[[ssh_profiles]]` entry. Once per host, palette →
*Install Rift shell integration on host* (asks first) makes remote commands
blocks too. Details: [SSH profiles](features.md#ssh-profiles).

## Automation

### Open a folder from another app

`rift-terminal ~/src/app` (or `.`, or a file: its folder) opens a tab there
in the running Rift; `--new-window` opens a window. Palette → *Install 'rift'
command* adds `~/.local/bin/rift`, so `rift .` works from any shell or as an
editor's external terminal. On macOS, drop a folder on Rift's Dock icon or
use Finder's *Open With → Rift*. `rift://action/open_file?path=<file>&line=12`
opens a file in your `link_editor` at line 12. Every shell gets
`RIFT_PANE_URL`: `open "$RIFT_PANE_URL"` (from a Claude Code hook, say)
brings you back to that pane. Details:
[Opening Rift from other apps](features.md#opening-rift-from-other-apps),
[pane focus links](features.md#pane-focus-links-rift_pane_url).

### Control socket

`rift-terminal ctl` controls running Rift (every window) over a local socket:

```sh
rift-terminal ctl list                    # tabs and panes of every window (JSON)
rift-terminal ctl focus --tab 2           # --tab/--pane count from 0, like `list`
rift-terminal ctl open --cwd ~/src -- make test   # needs control_api = "full"
```

`ctl --pid PID` picks a window (the newest answers otherwise);
`rift-terminal --version` and `--help` print the version and usage.
`control_api` is `navigate` by default (list, status, focus, blocks). Set it to
`full` in Settings → General → *Control socket* to allow split, open and
send-text.

### Phone companion

1. Settings → *Remote access* → turn on *Allow remote access*.
2. *Pair a phone*, then scan the QR code with the Rift app (or the phone's
   camera). The code works once, for 2 minutes; *Cancel* withdraws it. Tick
   *The new phone can only watch* first for a phone that should not run
   commands.
3. On the phone, pick a host, a tab and a pane: its blocks follow live. Type a
   command and confirm to run it at the prompt, or use the key row (^C, esc,
   tab, arrows) to talk to a running program.

On the same network the phone connects directly. Away from it, it connects
through Rift's relay, with nothing to set up. To use your own relay instead,
put its `wss://` address under *Connection settings* › *Relay address* (host one with `rift-terminal relay` or
`packaging/relay.Dockerfile`) and pair again so the link includes it; to keep
phones on your network only, clear the field. *Use a Cloudflare tunnel
instead* replaces the relay with a free tunnel from this computer (Settings
offers to install `cloudflared`; phones pair again after each restart), and a
private network such as Tailscale works too. A relay never sees your
terminal: it only forwards encrypted traffic. While a phone is connected, the tab bar
shows a *Remote* chip. Set a phone to *View only*, check *Recent
connections*, or revoke a phone under *Paired phones*; turning remote
access off disconnects every phone. In the app, Settings › Security can
require Face ID or a fingerprint.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Commands do not become blocks | Blocks need zsh, bash or fish; other shells run as a classic terminal. `echo $TERM_PROGRAM` should print `rift`. |
| A remote host shows no blocks | Run *Install Rift shell integration on host*, then reconnect. The server must accept `LC_*` variables (`AcceptEnv LC_*`, the default on most systems). |
| Keys in a program behave oddly | Programs get raw keys; ⌥ sends Meta on Linux, and on macOS only with *Left / Right Option key is Meta* (Settings › Terminal). |
| Rift does not start on Linux | Check the GPU driver (Vulkan, or GL through Mesa). `RUST_LOG=info rift-terminal` prints the chosen adapter. |
| AppImage: `Cannot mount AppImage` | `sudo apt install libfuse2t64`, or run with `--appimage-extract-and-run`. |
| No notifications | Linux needs `notify-send`. `RIFT_NO_NOTIFY=1` turns them off on purpose. |

## Files Rift uses

| Path | Holds |
|---|---|
| `~/.config/rift/config.toml` | Settings |
| `~/.config/rift/themes/` | Your themes |
| `~/.config/rift/launch_configs/` | Launch configurations |
| `~/.local/share/rift/` | Shell hooks, session layout, saved block output, sockets |

Both roots follow `XDG_CONFIG_HOME` and `XDG_DATA_HOME` when set.
