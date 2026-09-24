# Development

How to build, test, verify and release Shift. Agent-specific rules are in
[CLAUDE.md](../CLAUDE.md) (identical to `AGENTS.md`); visual rules are in
[DESIGN.md](../DESIGN.md).

## Setup

| Need | Version / package |
|---|---|
| Rust | 1.98.0, pinned in `rust-toolchain.toml` (rustup installs it and rustfmt + clippy on first use) |
| macOS | 11 or newer, Xcode Command Line Tools (`xcode-select --install`) |
| Linux build packages | `libxkbcommon-dev libwayland-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev pkg-config` |
| Gate tools | `shellcheck`, `zsh`, `fish` (shell gate); `docker` (linux gate) |
| Packaging (optional) | `python3` + Pillow (`gen-icon.py`), `dpkg-dev`, `file`, `libfuse2` (AppImage) |

```sh
git clone <repo> && cd shift
cargo run -p shift-app          # builds and opens a window
git config core.hooksPath .githooks   # optional: fmt + clippy + docs before each commit
```

## Build

```sh
cargo build                      # debug: target/debug/shift-terminal
cargo build --release            # release (thin LTO): target/release/shift-terminal
packaging/make-app.sh            # macOS: target/Shift.app
```

The binary has no runtime assets: fonts, themes and shell hooks are embedded.
`shift-terminal --help` prints the usage; `--version` the version.

## Test

```sh
cargo test --workspace           # every unit test (headless)
cargo test -p shift-core osc     # one module
cargo test -p shift-pty daemon   # the session service
```

Unit tests cover the core model (blocks, OSC scanner, keymap, config,
themes, completions, tokenizer, transcripts, SSH config), the PTY layer
(session feed, graphics, IPC, session service), image decoding and sprite
clipping in the renderer, and app logic that does not draw (panes, keyboard
encoding, accessibility tree, control API, editor, vim, palette ranking,
display-line building). Rendering is verified with the script driver below.

## Gates

`scripts/gates.sh` is the one gate runner, used locally and by CI:

| Gate | Runs |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `test` | `cargo test --workspace --locked` |
| `doc` | `cargo doc --workspace --no-deps --locked` with rustdoc warnings denied |
| `shell` | `shellcheck` on packaging, scripts, git hooks and the bash hooks; `zsh -n` and `fish --no-execute` on the zsh and fish hooks |
| `docs` | `CLAUDE.md` and `AGENTS.md` are identical; every relative link in every Markdown file resolves |
| `release` | `cargo build --release --locked` |
| `linux` | `clippy`, `test` and `release` inside the Linux docker image |

```sh
scripts/gates.sh                 # fmt clippy test doc shell docs
scripts/gates.sh all             # + release + linux
scripts/gates.sh clippy test     # a subset
scripts/gates.sh --list
```

Every requested gate runs even after one fails; the summary lists each result
with its time and the commit. A gate whose tool is missing fails instead of
passing silently. CI (`.github/workflows/ci.yml`) runs `scripts/gates.sh`
on macOS 14 (fmt clippy test doc release) and Ubuntu 24.04 (the same plus
`shell`), so a local pass predicts CI.

## Verifying UI changes

`shift-terminal --script <file>` drives the real app. Each step goes through
the same key and mouse routing as a user's input; `shot` renders the next
frame offscreen and writes a PNG; `dump` writes the app state, including the
accessibility tree and the attention center.

```sh
mkdir -p /tmp/shift-t/cfg /tmp/shift-t/data
cat > /tmp/shift-t/t.shift <<'EOF'
resize 1200 700
wait_prompt              # wait for the shell's first prompt
type echo hi\n           # \n presses Enter
wait 500
key cmd+p                # open the palette
shot /tmp/shift-t/a.png
dump /tmp/shift-t/a.txt
quit
EOF
XDG_CONFIG_HOME=/tmp/shift-t/cfg XDG_DATA_HOME=/tmp/shift-t/data SHIFT_NO_NOTIFY=1 \
  ./target/debug/shift-terminal --script /tmp/shift-t/t.shift
```

Always isolate config and data directories: otherwise the scripted instance
uses your real config, session file, session service and control sockets.
The command list is the module comment of `crates/shift-app/src/script.rs`.

**Headless Linux.** `packaging/linux-verify.Dockerfile` has Xvfb and Mesa's
software Vulkan, so the same scripts render on a machine without a GPU. The
header of the Dockerfile has the `docker build` / `docker run` lines.

**Performance.** `packaging/bench.sh` measures startup, output throughput,
memory, key-to-frame latency and TUI round trips; compare against
[performance.md](performance.md) on the same machine.

## Architecture

```
shift-app       winit app, views, input routing, a11y, control socket, script driver
  ├── shift-renderer   wgpu quad + sprite pipelines, glyphon text, Scene layers
  ├── shift-pty        PTY sessions, reader threads, vte, images, session service
  └── shift-core       config, themes, keymap, OSC scanner, blocks, completions, git
```

Dependencies flow downward only; `shift-core` has no graphics dependencies.
The load-bearing invariants (absolute line model, resize compensation, PTY
fd blocking, viewport ownership, editor vs raw mode, sRGB blending, marker
idempotence) are listed in [CLAUDE.md](../CLAUDE.md#load-bearing-invariants).
Read them before changing `shift-pty` or the terminal view.

### Data flow

1. The shell writes to the PTY. A reader thread in `shift-pty` reads it,
   splits it at OSC 133 / OSC 7 / APC markers (`feed`), and passes each part
   to alacritty's vte parser through `abs_term.rs`, which counts lines moved
   into scrollback.
2. Markers update the `BlockStore` (prompt, command, output, exit) and the
   image model; the reader wakes the UI thread.
3. The UI thread builds a `Scene` from the terminal grid, the blocks and the
   overlays, and the renderer draws it with wgpu.
4. Keys go to the owned editor while the shell is at a prompt, and raw to the
   PTY otherwise.

## Dependencies

Workspace dependencies (`Cargo.toml`):

| Crate | Used for | Note |
|---|---|---|
| `wgpu` 29 | GPU rendering | Pinned: glyphon 0.11 requires `^29` |
| `glyphon` 0.11 | Text shaping and glyph atlas | |
| `winit` 0.30 | Windows, input, IME | 0.31 is beta |
| `alacritty_terminal` 0.26 | VT emulation, PTY spawn | Only `tty`, `Term`, `vte`, `FairMutex` |
| `bytemuck`, `pollster` | GPU buffers, blocking on adapter setup | |
| `image`, `png`, `flate2` | Inline images, screenshots, kitty zlib payloads | |
| `serde`, `toml`, `serde_yaml_ng`, `serde_json` | Config, themes, Warp YAML, IPC | |
| `arc-swap`, `notify` | Lock-free config snapshot, hot reload | |
| `nucleo-matcher` | Fuzzy ranking in the palette | |
| `arboard` | Clipboard (Wayland data-control) | |
| `git2` (no default features) | Branch, ahead/behind, dirty | Background thread |
| `dirs` | Home and standard directories | |
| `log`, `env_logger` | Logging (`RUST_LOG=info`) | |

App-only: `accesskit` / `accesskit_winit` (screen readers), `global-hotkey`,
`regex` (find), `libc`; on macOS `muda` (menu bar), `objc2`,
`objc2-app-kit`. `shift-pty` also uses `libc` for PTY and socket calls.

Before adding a dependency, check whether the standard library or an existing
crate covers it. Keep default features off where the crate allows.

## Packaging and releases

Every script takes `--target <triple>`, `--target-dir <dir>` and
`--no-build`, honors `CARGO_TARGET_DIR`, and prints the artifact path.

| Script | Platform | Produces |
|---|---|---|
| `packaging/gen-icon.py` | any | `assets/icon/`: 1024 PNG, `.iconset`, `.icns` (macOS), hicolor PNGs (Linux), `.ico` (Windows), `shift-window.png` (embedded window icon) |
| `packaging/make-app.sh` | macOS | `target/Shift.app` (ad-hoc signed) |
| `packaging/make-dmg.sh` | macOS | `target/Shift-<version>-<arch>.dmg` (run `make-app.sh` first) |
| `packaging/make-deb.sh` | Linux | `target/shift-terminal_<version>_<arch>.deb` |
| `packaging/make-appimage.sh` | Linux | `target/Shift-<version>-<arch>.AppImage` (downloads `appimagetool` once) |
| `packaging/make-tarball.sh` | both | `target/shift-terminal-<version>-<os>-<arch>.tar.gz` |

The version lives only in `[workspace.package]` of `Cargo.toml`. Pushing a
`v*` tag runs `.github/workflows/release.yml`, which builds every package and
publishes a GitHub Release.

## Updating the toolchain

1. Change `channel` in `rust-toolchain.toml`.
2. Change the `FROM rust:<version>` line of `packaging/linux-verify.Dockerfile`.
3. Run `scripts/gates.sh all` and fix new clippy lints.

## Conventions

- Update `docs/` in the same change as the behavior it describes.
- Every new action gets a keymap entry (bound or unbound), a palette row, and
  a row in [keyboard-shortcuts.md](keyboard-shortcuts.md).
- Every new interactive surface gets an AccessKit node (`a11y.rs`).
- Terminal output, images, socket requests and restored files are untrusted:
  bound their sizes and never interpolate them into commands.
- Commits follow Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`).
