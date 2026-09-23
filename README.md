# Shift

A GPU-accelerated terminal emulator with block-based output, for macOS and Linux.
Rust · winit · wgpu · glyphon · alacritty_terminal.

![Shift](assets/screenshot.png)

## Features

- **Command blocks** — every command is a bounded block (OSC 133 shell
  integration): exit-code colored left border, `✓ 0` / `✕ 1` badge, duration,
  collapse, one-click output copy.
- **Real app chrome** — tabs with activity indicators, tree-based split panes
  with draggable resize, focused-pane accent border.
- **Owned input editor** — syntax-highlighted command line (command / flag /
  string), prompt context (cwd · git branch · ↑ahead), inline ghost
  completions, `$PATH`/git/path/history completion dropdown, multi-line
  commands with `⇧⏎`, quote/bracket auto-pairing, optional vim keybindings
  (`NORMAL` pill) and an input line that can sit at the top or defer to your
  own shell prompt.
- **Command palette** — `⌘P`, fuzzy-ranked access to every terminal action
  with resolved custom-shortcut chips, settings, themes, launch configurations
  and shell history.
- **Settings** — five-page overlay (General, Appearance, Features,
  Keybindings, Themes) writing `~/.config/shift/config.toml`, hot-reloaded
  (arc-swap style, <300 ms), plus `⌃⇧F` search, four built-in themes (Dark
  Default, Light Default, Catppuccin Mocha, Tokyo Night), user themes in
  `~/.config/shift/themes/` as TOML **or Warp YAML**, OS light/dark sync,
  window opacity and theme background images.
- **Launch configurations** — Warp-compatible YAML in
  `~/.config/shift/launch_configs/`: tabs, nested splits, working directories
  and startup commands, opened from the palette.
- **Selection & links** — drag/double(semantic)/triple(line) select with
  `⌘C` copy, optional copy-on-select, `⌘`-click opens URLs.
- **Notifications & bell** — desktop notification when a long command finishes
  while the window is unfocused; visual and/or audible bell.
- **Block actions** — right-click menu (copy command/output/both, re-run,
  bookmark, collapse), `⌘B` bookmarks with edge indicators and
  `⌃⇧↑/↓` jumps, sticky command header for tall outputs.
- **History & workflows** — `⌃R` fuzzy shell-history search (imports
  `~/.zsh_history`), `⌃⇧R` Warp-compatible YAML workflows from
  `~/.config/shift/workflows/` with Tab-cycled `{{placeholders}}`,
  `⌃→` accepts ghost suggestions word-by-word.
- **Sessions & shells** — tabs and working directories restored on relaunch
  (`restore_session = false` to disable); zsh, bash and fish all integrate
  automatically (nothing to add to your rc files).

## Install

Prebuilt packages are attached to every [release](../../releases).

**macOS** — download `Shift-<version>-<arch>.dmg` (`arm64` for Apple silicon,
`x86_64` for Intel), open it and drag Shift to Applications. The bundle is
ad-hoc signed, so the first launch needs *right-click → Open* once.

**Debian / Ubuntu** — download `shift-terminal_<version>_amd64.deb`:

```sh
sudo dpkg -i shift-terminal_<version>_amd64.deb
sudo apt-get install -f        # only if dpkg reports missing dependencies
```

**AppImage** (any distro) — download `Shift-<version>-x86_64.AppImage`:

```sh
chmod +x Shift-<version>-x86_64.AppImage
./Shift-<version>-x86_64.AppImage
```

AppImages mount themselves with FUSE 2, which Ubuntu 24.04 no longer installs
by default. If you get `Cannot mount AppImage, please check your FUSE setup`,
either `sudo apt install libfuse2t64` or run it without FUSE:

```sh
./Shift-<version>-x86_64.AppImage --appimage-extract-and-run
```

**Tarball** — `shift-terminal-<version>-<os>-<arch>.tar.gz` holds the bare binary,
the `.desktop` file, the icon and `docs/`; copy `shift-terminal` onto your `PATH`.
The executable uses `shift-terminal` because `shift` is a shell builtin.

## Build

```sh
cargo build --release          # both OSes; single binary target/release/shift-terminal
packaging/make-app.sh          # macOS: build target/Shift.app
```

Linux build deps: `libxkbcommon-dev libwayland-dev libxcb1-dev
libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev pkg-config`.
Install the binary plus `packaging/shift.desktop` for a launcher entry.

### Build packages locally

Every script takes `--target <triple>`, `--target-dir <dir>` and `--no-build`,
honours `CARGO_TARGET_DIR`, and prints the artifact path it produced.

| Script | Platform | Produces |
|---|---|---|
| `packaging/gen-icon.py` | any | `assets/icon/` — 1024 PNG, `.iconset`, `.icns`, hicolor PNGs |
| `packaging/make-app.sh` | macOS | `target/Shift.app` (ad-hoc signed) |
| `packaging/make-dmg.sh` | macOS | `target/Shift-<version>-<arch>.dmg` |
| `packaging/make-deb.sh` | Linux | `target/shift-terminal_<version>_<arch>.deb` |
| `packaging/make-appimage.sh` | Linux | `target/Shift-<version>-<arch>.AppImage` |
| `packaging/make-tarball.sh` | both | `target/shift-terminal-<version>-<os>-<arch>.tar.gz` |

```sh
packaging/make-app.sh && packaging/make-dmg.sh          # macOS
packaging/make-deb.sh && packaging/make-appimage.sh     # Linux
```

`make-dmg.sh` packages an existing `target/Shift.app`, so run `make-app.sh`
first. `make-appimage.sh` downloads `appimagetool` into the target dir on first
run. The icon is committed, so `gen-icon.py` only needs re-running when the
default theme's accent changes.

`.github/workflows/release.yml` runs all of these on a `v*` tag and publishes
the artifacts as a GitHub Release.

## Shell integration

Everything is injected at spawn — your own rc files are always sourced first:

- **zsh** — a `ZDOTDIR` shim sources your real config, then the Shift hooks.
- **bash** — spawned with `--rcfile ~/.local/share/shift/bashrc`, a shim that
  replays bash's own startup order (profile files on macOS, `~/.bashrc` on
  Linux) and then sources `shift.bash`.
- **fish** — `shift.fish` is picked up from a `vendor_conf.d` directory
  appended to `XDG_DATA_DIRS`.

Without integration Shift degrades gracefully to a classic terminal
(no blocks, shell-side prompt and line editing).

## Default keybindings

| Action | Keys |
|---|---|
| New tab | `⌃⇧T` / `⌘T` |
| Close pane | `⌃W` / `⌘W` |
| Split right / down | `⌃⇧D` / `⌃⇧E` |
| Navigate panes | `⌥←→↑↓` |
| Next / prev tab | `⌃Tab` / `⌃⇧Tab` |
| Copy / paste | `⌃⇧C` / `⌃⇧V` |
| Search | `⌃⇧F` |
| Scroll | `⇧PgUp` / `⇧PgDn` |
| Command palette | `⌘P` / `⌃⇧P` |
| Clear blocks | `⌘K` |
| Settings | `⌘,` |

Override any of these in `config.toml`:

```toml
[keybindings]
new_tab = "cmd+n"
```

## Config

`~/.config/shift/config.toml`, hot-reloaded on save:

```toml
font_size = 13.0
line_height = 1.65
cursor_style = "block"    # block | bar | underline
cursor_blink = true
theme = "dark-default"    # or light-default, catppuccin-mocha, tokyo-night, <your-theme>
theme_light = "light-default"
sync_os_theme = false     # follow the OS appearance
window_opacity = 1.0      # 0.3–1.0
scrollback = 10000
ligatures = true
input_position = "bottom" # bottom | top
prompt = "shift"         # shift | shell (keep your own PS1)
vim_mode = false
autocomplete_pairs = true # quotes, brackets and braces
copy_on_select = false
bell = "visual"           # visual | audible | both | none
```

Full reference, themes (including Warp YAML), launch configurations and
notification/bell details: [docs/configuration.md](docs/configuration.md).

## Scripted verification

`shift-terminal --script <file>` drives the real app with synthesized input — every
step goes through the same key/mouse routing a user's events do, and `shot`
captures the frame by offscreen readback, so nothing depends on screen-capture
or accessibility permissions:

```text
wait_prompt              # block until the shell shows its first prompt
type echo hi\n           # characters; \n submits
wait 500
shot /tmp/a.png          # PNG of the next rendered frame
dump /tmp/a.txt          # textual state snapshot (tabs, overlays, editor)
quit
```

The full command list (`key`, `click`, `dblclick`, `rclick`, `drag`, `scroll`,
`move`, `resize`, `focus`, `sh`, …) is the module doc comment at the top of
`crates/shift-app/src/script.rs`.

`shift-terminal --version` prints the version; `shift-terminal --help` the usage line.

## Workspace layout

- `shift-core` — config, themes, keymap, OSC 133 scanner, block model,
  tokenizer, completions, git worker (no gfx deps, fully unit-tested)
- `shift-pty` — PTY sessions, reader threads, absolute-line tracking
- `shift-renderer` — wgpu SDF quad pipeline + glyphon text engine
- `shift-app` — winit app, views (terminal, tabs, editor, palette,
  completions, settings, search)
