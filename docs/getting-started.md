# Getting started

This guide takes you from the first launch to everyday use. Shortcuts are
macOS defaults; on Linux ⌘ is the Super key, and most actions also have a
⌃⇧ chord (see [keyboard shortcuts](keyboard-shortcuts.md)).

## First launch

Shift starts your login shell (`$SHELL`, or `shell = "…"` in config.toml).
On macOS it runs as a login shell like Terminal.app, so `~/.zprofile`
(Homebrew's `brew shellenv`) loads before `~/.zshrc`. On the first start it writes its shell hooks to `~/.local/share/shift/` and
loads them **after** your own rc files, so your prompt, aliases and plugins
keep working. zsh, bash and fish are supported; you edit no rc file.

You see two areas:

- **Blocks** above: each command you run, with its output.
- **The command editor** at the bottom: the line you are typing, with the
  current directory and git branch above it.

If commands do not become blocks, the shell is not loading the hooks (see
[Troubleshooting](#troubleshooting)). Shift then works as a classic terminal.

## Run commands

- Type and press ⏎. The command becomes a block with a green (`✓ 0`) or red
  (`✕ N`) badge and its duration.
- ⇧⏎ adds a second line; ⏎ runs the whole command.
- Gray text after the cursor is a suggestion from your history: Tab or → accepts
  it, ⌃→ accepts one word. The completion panel above the editor lists commands, subcommands, flags
  and paths; ↑/↓ choose, Tab inserts.
- ⌃R opens the history pane above the command line: type to narrow it, ⏎ runs, ⇥ edits first, ⌘⏎ runs in a new split. ↑/↓ walk history without it.
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
| Show only matching output lines | ⌥⇧F and type; ⌥⇧F again clears |
| Feed its output to another command | Block menu → *Send output to command…* |
| Find text anywhere | ⌘F; ⌥C toggles case, ⌥R regex |
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
| Open another window | ⌘N (its shells end with it) |
| Rename or color a tab, close the others | Right-click the tab |
| Tabs in a side rail | Settings → Appearance → *Tabs: Side* |

Tabs, splits and directories come back on the next launch
(`restore_session`). Turn on *Save block output for restore* to also bring
back the finished blocks, and *Keep shells running when Shift quits* to keep
the shells themselves alive (see [durable sessions](features.md#durable-sessions)).

## Find anything: the palette

⌘P lists every action with its shortcut, plus settings, themes, launch
configurations, SSH hosts and recent commands. Type a few letters; ⏎ runs.
If you forget a shortcut, the palette shows it.

Other keyboard-first tools:

- **Hint mode** ⇧⌘H: letters appear over every visible link, path and block.
  Type one to open it; ⇧ + letter copies it.
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
`~/.config/shift/config.toml` and applies at once; editing the file by hand
applies the same way. Reference: [configuration](configuration.md).

- **Themes**: Settings → Themes. Put your own TOML or Warp YAML theme in
  `~/.config/shift/themes/`. *Sync with OS appearance* follows light and dark
  mode.
- **Font and size**: Settings → Appearance; ⌘= / ⌘- / ⌘0 change the size.
- **Shortcuts**: Settings → Keybindings (⌃⌘K): click a row and press the new
  chord. Or add a `[keybindings]` table to config.toml.
- **New tabs start in**: Settings → General; the previous directory, home or
  a path.
- **Your own prompt**: set *Prompt* to *Shell (PS1)* to keep starship, p10k or
  your PS1 visible.

## Save commands for later

- **Launch configurations**: YAML files in
  `~/.config/shift/launch_configs/` (Warp format) open a set of tabs, splits,
  directories and startup commands from the palette. File → *Save Layout as
  Launch Configuration* writes the current layout.

## Remote machines

Palette → *SSH: host* opens a tab running your system `ssh` for any `Host`
in `~/.ssh/config` or any `[[ssh_profiles]]` entry. Once per host, palette →
*Install Shift shell integration on host* (asks first) makes remote commands
blocks too. Details: [SSH profiles](features.md#ssh-profiles).

## Automation

`shift-terminal ctl` controls a running window over a local socket:

```sh
shift-terminal ctl list                    # tabs and panes of every window (JSON)
shift-terminal ctl focus --tab 2           # --tab/--pane count from 0, like `list`
shift-terminal ctl open --cwd ~/src -- make test   # needs control_api = "full"
```

`ctl --pid PID` picks a window (the newest answers otherwise);
`shift-terminal --version` and `--help` print the version and usage.
`control_api` is `navigate` by default (list, status, focus). Set it to
`full` in Settings → General → *Control socket* to allow split, open and
send-text.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Commands do not become blocks | Blocks need zsh, bash or fish; other shells run as a classic terminal. `echo $TERM_PROGRAM` should print `shift`. |
| A remote host shows no blocks | Run *Install Shift shell integration on host*, then reconnect. The server must accept `LC_*` variables (`AcceptEnv LC_*`, the default on most systems). |
| Keys in a program behave oddly | Programs get raw keys; ⌥ sends Meta on Linux, and on macOS only with *Left / Right Option key is Meta* (Settings › Terminal). |
| Shift does not start on Linux | Check the GPU driver (Vulkan, or GL through Mesa). `RUST_LOG=info shift-terminal` prints the chosen adapter. |
| AppImage: `Cannot mount AppImage` | `sudo apt install libfuse2t64`, or run with `--appimage-extract-and-run`. |
| No notifications | Linux needs `notify-send`. `SHIFT_NO_NOTIFY=1` turns them off on purpose. |

## Files Shift uses

| Path | Holds |
|---|---|
| `~/.config/shift/config.toml` | Settings |
| `~/.config/shift/themes/` | Your themes |
| `~/.config/shift/launch_configs/` | Launch configurations |
| `~/.local/share/shift/` | Shell hooks, session layout, saved block output, sockets |

Both roots follow `XDG_CONFIG_HOME` and `XDG_DATA_HOME` when set.
