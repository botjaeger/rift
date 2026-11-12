# Development

How to build, test, verify and release Rift. Agent-specific rules are in
[CLAUDE.md](../CLAUDE.md) (identical to `AGENTS.md`); visual rules are in
[DESIGN.md](../DESIGN.md).

## Setup

| Need | Version / package |
|---|---|
| Rust | 1.98.0, pinned in `rust-toolchain.toml` (rustup installs it and rustfmt + clippy on first use) |
| macOS | 11 or newer, Xcode Command Line Tools (`xcode-select --install`) |
| Linux build packages | `libxkbcommon-dev libwayland-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev pkg-config` |
| Gate tools | `python3` (test gate: `release-manifest.py --self-test`; ui gate); `shellcheck`, `zsh`, `fish` (shell gate); Node 22 + pnpm via `corepack enable` (mobile gate); `docker` (linux and windows gates); `mingw-w64` and `rustup target add x86_64-pc-windows-gnu` (windows gate) |
| Packaging (optional) | `python3` + Pillow (`gen-icon.py`), `dpkg-dev`, `file`, `libfuse2` (AppImage) |

```sh
git clone <repo> && cd rift
cargo run                             # builds and opens a window
git config core.hooksPath .githooks   # optional: fmt + clippy + docs before each commit
```

## Build

```sh
cargo build                      # debug: target/debug/rift-terminal
cargo build --release            # release (thin LTO): target/release/rift-terminal
packaging/make-app.sh            # macOS: target/Rift.app
cargo build --release --target x86_64-pc-windows-gnu   # Windows (cross, mingw-w64): rift-terminal.exe
```

### Windows (in progress)

Rift builds for Windows 10 1809+ (ConPTY) and its platform layer is
tested under Wine (the `windows` gate); it has not yet been verified on a
Windows machine. What differs there:

- Shells: the default is PowerShell 7 (`pwsh`), else Windows PowerShell,
  else `%ComSpec%` (cmd). PowerShell gets command blocks through
  `shell/rift.ps1` (dot-sourced after the user's profile); cmd and others
  run as a plain terminal. Enter is sent as CR.
- Panes always run local ConPTY shells: the session service (durable
  sessions), the control socket (`rift-terminal ctl`), updates, the `rift`
  command install and phone captures are Unix-only so far (`rift_pty::uds`
  makes their socket types uninhabited on Windows).
- `~` is `%USERPROFILE%` when `HOME` is unset (`rift_core::sys::home_os`);
  config and data stay under `~/.config/rift` and `~/.local/share/rift`.
- Platform differences live in `rift_core::sys` (and `rift_remote::sys`):
  process groups (`CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`, `taskkill
  /T`), liveness, file modes, PATHEXT, `\\?\`-free `canonicalize`, renames
  retried while another process holds the file.
- The vendored `alacritty_terminal` ConPTY backend keeps its pipes as plain
  blocking files (`Pty::reader_file`, `writer_file`, `child_pid`): Rift reads
  them on its own thread, as on Unix.

Dev builds optimize dependencies (`[profile.dev.package."*"] opt-level = 2`
in `Cargo.toml`): text shaping and the GPU path dominate a debug frame, and a
fully unoptimized build scrolls a large diff at ~11 ms per frame instead of
~2 ms. Rift's own crates stay unoptimized, so incremental rebuilds stay
around 1–2 s; the first build after a clean takes longer.

The binary has no runtime assets: fonts, themes and shell hooks are embedded.
`rift-terminal --help` prints the usage; `--version` the version. On macOS,
`crates/rift-terminal/build.rs` embeds a minimal Info.plist in the binary so an
unbundled run shows "Rift" in the menu bar instead of `rift-terminal`. The
Dock and ⌘Tab name a process after its bundle, so `cargo run` goes through
`scripts/cargo-runner.sh` (`.cargo/config.toml`): it launches the binary from a
dev bundle, `target/<profile>/Rift.app`, hard-linked to the fresh build.
Running `target/debug/rift-terminal` directly still shows `rift-terminal`
in the Dock.

## Test

```sh
cargo test --workspace           # every unit test (headless)
cargo test -p rift-core osc     # one module
cargo test -p rift-pty daemon   # the session service
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
| `fmt` | `cargo fmt --all --check`, and on each vendored crate |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings`, on each vendored crate (all features), and on `rift-remote --no-default-features` (the relay image's build, no TLS) |
| `test` | `cargo test --workspace --locked`, and each vendored crate (all features; built into `target/vendor`); `scripts/release-manifest.py --self-test`. Locally on macOS the workspace tests run once, throttled (`taskpolicy -c background`, 32 threads, after a full-speed build), which reproduces the timing of CI's few slow cores; CI and Linux run them plainly |
| `doc` | `cargo doc --workspace --no-deps --locked` with rustdoc warnings denied |
| `shell` | `shellcheck` on packaging, scripts, git hooks and the bash hooks; `zsh -n` and `fish --no-execute` on the zsh and fish hooks |
| `docs` | `CLAUDE.md` and `AGENTS.md` are identical; relative Markdown link targets exist (heading anchors are not checked; the vendored crates' upstream docs excepted) |
| `mobile` | In `mobile/`: `pnpm install --frozen-lockfile`, `pnpm typecheck`, `pnpm lint`, `pnpm test` (vitest; decodes `crates/rift-remote/fixtures/`, the files the Rust protocol tests decode too) |
| `release` | `cargo build --release --locked` |
| `ui` | `scripts/ui-smoke.sh`, which runs `scripts/ui-smoke.py`: isolated scripted input, state assertions and PNG capture, in `RIFT_UI_SHARDS` (default 2) parallel shards (`RIFT_UI_SHARDS=1`: the file serially); `--only NAME[,NAME]` (or `RIFT_UI_ONLY`) runs only the groups holding those sections, in one process; requires Python 3 and a display (or `xvfb-run`) |
| `ssh` | `scripts/ssh-ui-smoke.py`: actual palette install/cancel, SSH profile timeout/retry, remote blocks and path/shell completions against a disposable Docker SSH server. Scratch keys, HOME and XDG roots; requires Docker and OpenSSH clients. Captures remain in `target/ssh-ui-*`; fixture keys, control masters and container are removed |
| `leak` | `scripts/leak-soak.py` on the release build: a program redrawing every 20 ms, output far past the scrollback limit, one command printing lines without end, and opening and closing windows, tabs, splits, the palette, Settings, find and the explorer, each repeated in one isolated instance; after a warm-up the memory footprint (macOS `phys_footprint`, which counts GPU buffers; Linux `VmRSS`) must not grow more than 8 MB over the measured cycles. An over-budget result is measured once more in a fresh instance and fails if it repeats. Headless on macOS (the display-asleep path); with the window shown under Xvfb (`RIFT_LEAK_SHOWN=1`) in CI and the `linux` gate. `RIFT_LEAK_CYCLES=N` soaks longer |
| `windows` | Cross-compiled for `x86_64-pc-windows-gnu` (mingw-w64): `clippy` on the workspace and the vendored `alacritty_terminal`, a release build, then every test binary under Wine in `packaging/windows-wine.Dockerfile` (`scripts/wine-tests.sh`; the repository is mounted at its own path so compiled-in fixture paths resolve through Wine's `Z:`). One test thread, and a binary is run again only when Docker's x86 emulation itself crashed (`rosetta error`). Wine has no ConPTY or GPU: those need a Windows machine |
| `linux` | `clippy`, `test`, `doc` and a release build (with `examples/phone` and `rift-relay`) inside the Linux docker image (which also carries PowerShell, for the `rift.ps1` test), then `shell`, the scripted UI regressions and the memory soak (Xvfb, software Vulkan) in `packaging/ci-ubuntu.Dockerfile`, which installs the CI runner's packages from `packaging/ci-apt.txt` |

```sh
scripts/gates.sh                 # fmt clippy test doc shell docs mobile
scripts/gates.sh all             # + release + ui + ssh + leak + windows + linux
scripts/gates.sh clippy test     # a subset
scripts/gates.sh ui --only NAME  # only these ui-smoke.py sections
scripts/gates.sh --list
```

Every requested gate runs even after one fails; the summary lists each result
with its time, start and end clock times, the total, and the commit. Each
gate prints one line (`-- test pass 212s`); its output goes to
`target/gates/<gate>.log`, and the summary ends with the last 25 lines of
every failed gate's log. `RIFT_GATES_VERBOSE=1` (and CI) streams every
gate's output too. Every gate has a wall-clock limit (`limit_for` in the
script: 5 min for fmt/shell/docs, 15 for test, 20 for clippy/doc/mobile/leak,
30 for release/ui/ssh, 60 for windows, 90 for linux; `RIFT_GATE_TIMEOUT=<seconds>` overrides
them): past it, the processes still running are listed in the log (naming
the hung test binary), stopped, and the gate fails. Interrupting the run
stops the running gate and the background `linux` run with everything
under them. With other gates,
`linux` runs beside them: it starts once `test` is done (the
timing-sensitive unit tests run alone), on all but two cores, into
`target/gates/linux.log`; the moment it ends it is reported with each of its
steps timed (images, clippy/test/doc and the release builds, then
shell/ui/leak under Xvfb) and, on a failure, the end of its log. That cuts
`all` from about 33 to about 19 minutes. `RIFT_GATES_SERIAL=1` runs it in
turn instead. A gate whose tool is missing fails instead of
passing silently. CI (`.github/workflows/ci.yml`) runs `scripts/gates.sh`
on macOS 14 (fmt clippy test doc docs release) and Ubuntu 24.04 (the same plus
`shell`, `mobile`, `ui` and `leak` under Xvfb/Mesa). Only `scripts/gates.sh all`
covers both (the `linux` gate is the Ubuntu job): run it before every push,
so CI is a formality. The Release workflow runs that same
reusable validation workflow on the tagged revision before it writes
`latest.json`.

The `ui` gate checks, among others: command blocks, palette input while
another window's socket is stalled, background file preview, theme-only hot
reload, automation config reload (polled, not timed: file-system events can
arrive late), pane splitting, where the next block lands after `clear`, the
completion panel moving the output out from under it, mouse and hint
geometry, drag selection, the review diff (hunk expand and gap rows), DEC
double-width/double-height lines, shell-syntax file names, Unicode and
partial lines, the Settings Files/Terminal pages and keyboard reach, agent
idle titles (Claude Code, Codex), ⌘⌫ in raw mode, the font picker, and
updates (the tab bar's chip from a `file://` manifest, the dialog, *Later*
across a relaunch, Settings › About for an up-to-date and a malformed
manifest), and remote access (`remote-input` refused while off; a command
and Ctrl-C from a device with `control_api = "off"`; *busy* while a command
runs; the managed gateway starting and stopping with the setting; the
tab-bar chip appearing on an idle window within 2 s of a phone connecting;
a phone's `scroll` moving a prompt's view and reaching a mouse-tracking
program as SGR wheel reports, and its `upload` saved and its path pasted;
driven by `examples/phone`, which `scripts/ui-smoke.sh` builds unless
`RIFT_PHONE_BIN` names one). It uses scratch XDG
config/data directories with notifications disabled and retains captures, state dumps and logs under
`/tmp/rift-ui-*`. Set `RIFT_TEST_BIN` to test an already built binary;
otherwise it builds the debug binary first.

`scripts/ui-smoke.py` is one file that runs top to bottom (`RIFT_UI_SHARDS=1`
runs it that way), but its sections are independent enough to run apart:
`scripts/ui_smoke_sections.py` parses it at run time. Everything above the
`# === sections:` line is the preamble; after it, a top-level comment block
that follows a blank line (or opens with `# ---`) starts a section named after
its first line. Sections that share state join one group: a name one reads
whose last assignment (or `del`, `+=`, `x.append`, `x[k] =`, import, `def`) is
in another, counting what a called function's body reads and changes; a
directory variable (`x = scratch / "name"`) and its files; the same literal
path under `scratch`; the preamble's own profile (`cfg`, `data`). Imports,
`def`s and pure assignments of preamble names (`env = dict(os.environ, …)`)
run in every shard, and so does the closing epilogue.

```sh
python3 scripts/ui-smoke.py target/debug/rift-terminal --list          # groups, sections, line ranges, weights
python3 scripts/ui-smoke.py target/debug/rift-terminal --only dec,mark-mode  # those sections' groups
python3 scripts/ui-smoke.py target/debug/rift-terminal --shard 2/3     # one shard of three
scripts/gates.sh ui --only multi-cursor                                # the same through the gate
```

`--only` takes section names as `--list` prints them, or any part of a
section's name; the groups holding them run, nothing else. The gate starts
`RIFT_UI_SHARDS` shards at once (default 2: more loads a laptop past its UI
timeouts), each with its own scratch directory and log in
`target/ui-shards/shard-K.log`; shards share groups out by weight (line
counts, then the seconds each group took, kept in
`target/ui-smoke-timings.json`), heaviest first, the same way in every
shard. A failed group is reported with its section, line and the end of its
traceback, and its shard goes on with the next group, so one run shows every
failure; the gate fails when any shard failed. Every resource a section uses
is per run, so shards never meet: a scratch directory from `mkdtemp`, its own
XDG directories, port 0 for every listener, launchd labels and socket
directories derived from the data directory. A section that must not run
beside another goes in `SERIAL` in `scripts/ui_smoke_sections.py` (pinned to
shard 1; empty today). Adding a section needs nothing but its comment block
and its own directory (`xx = scratch / "name"`); reading another section's
variables or files joins its group.

## Verifying UI changes

`rift-terminal --script <file>` drives the real app. Each step goes through
the same key and mouse routing as a user's input; `shot` renders the next
frame offscreen and writes a PNG; `dump` writes the app state, including the
accessibility tree and the attention center.

```sh
mkdir -p /tmp/rift-t/cfg /tmp/rift-t/data
cat > /tmp/rift-t/t.rift <<'EOF'
resize 1200 700
wait_prompt              # wait for the shell's first prompt
type echo hi\n           # \n presses Enter
wait 500
key cmd+p                # open the palette
shot /tmp/rift-t/a.png
dump /tmp/rift-t/a.txt
quit
EOF
XDG_CONFIG_HOME=/tmp/rift-t/cfg XDG_DATA_HOME=/tmp/rift-t/data RIFT_NO_NOTIFY=1 \
  ./target/debug/rift-terminal --script /tmp/rift-t/t.rift --headless
```

Always isolate config and data directories: otherwise the scripted instance
uses your real config, session file, session service and control sockets.
A scripted run does not import your shell history (`$HISTFILE`,
`~/.zsh_history`, …) into ⌃R and the palette, so dumps and shots never show
your real commands; a test that needs an imported history points `HOME` or
`HISTFILE` at a fixture and sets `RIFT_SCRIPT_HISTORY=1`. It records nothing
in Rift's own command history (`history.json`) unless it sets
`RIFT_SCRIPT_RECORD_HISTORY=1` with its own `XDG_DATA_HOME`, and neither
reads nor writes the ignored suggestions (`ignored-suggestions.json`, the
ghost text's ✕) unless it sets `RIFT_SCRIPT_RECORD_IGNORED=1` with its own
`XDG_DATA_HOME` (its ignores otherwise last for the run).
Settings › Permissions (macOS) reads the system's real grants; a test fixes
them with `RIFT_SCRIPT_PERMISSIONS` (scripted runs only), words of `key=value`:
`host=helper|service|window`, `window=` and `service=`
`granted|missing|unknown|restart|down`, `notifications=allowed|off|unasked`.
Nor does it read `/etc/shells`: config.toml gets no generated shell profiles
and none is marked orphaned, unless `RIFT_SHELLS_FILE` names a fixture shells
file, which then stands in for `/etc/shells` (the startup sync and the orphan
check on every load).
The command list is the module comment of `crates/rift-terminal/src/script.rs`.

**`--headless`** (debugging and testing only; needs `--script`). The window
is created but never shown or focused, so you can keep working while a script
runs and your typing cannot land in it. `shot` still renders offscreen and
`dump` reports `headless=true window_visible=Some(false)`. The run uses a
private clipboard (copy, paste and `dump`'s `clipboard=` never touch the
system clipboard). It does not open links (a toast says `Headless: …`
instead), never minimizes, maximizes or goes fullscreen, ignores the control
API's `focus` raise, and registers no global hotkey. New windows (⌘N,
`new_window`) open in the same process and stay hidden too. On macOS it has
no Dock icon and launching does not activate it.
`scripts/gates.sh ui` runs headless.

**Accessibility and failure paths.** `a11y <action> <label>` sends an
assistive technology's action request (`click`, `focus`, `blur`, `expand`,
`collapse`, `scroll_into_view`, `show_context_menu`) to the node with that
label, through the handler VoiceOver's and Orca's requests reach; only
`click` presses. Headless, `fault clipboard` makes the clipboard backend
unavailable and `fault opener` the OS opener missing (`fault none` clears
both), so their error toasts can be asserted.

**Links.** `link rift://…` opens a `rift://` link through the same handler
the macOS Apple Event and a second process's control request reach.

**Several windows.** Steps act on the focused window, as typing does:
`new_window` (or `key cmd+n`) opens one and focuses it, `window <n>` focuses
window n (0 = the first, in opening order), `close_window` closes the
focused one. `dump` starts with `windows=N window=I focused_window=Some(F)`
and one `window i: tabs=… active=… panes=[…] titles=[…] kind=…` line per
window; the rest of it is the targeted window's state. `hotkey` runs the
global hotkey's toggle (no key is registered): with `hotkey_window = true` it
creates, shows and hides the drop-down window, which `dump` lists as
`kind=dropdown shown=… sliding=… frame=[x, y, w, h] screen=[…] size=[w, h]`
(ordinary windows: `kind=normal`). Headless, it never slides, and `focus on`
then `focus off` stands for a focus loss (`hotkey_window_hide_on_blur`).

**Headless Linux.** `packaging/linux-verify.Dockerfile` has Xvfb and Mesa's
software Vulkan, so the same scripts render on a machine without a GPU. The
header of the Dockerfile has the `docker build` / `docker run` lines.

**Shell and remote completions.** `cargo test -p rift-core --test
shell_complete` runs the completion servers against the installed zsh, bash
(and macOS's `/bin/bash` 3.2) and fish with a scratch home (a shell that is
not installed is reported `BLOCKED` and skipped). Remote completions need
docker: `cargo test -p rift-core --test remote_complete -- --ignored` builds
`packaging/sshd-fixture.Dockerfile`, starts an sshd on `127.0.0.1` with a
throwaway key and a scratch ssh config (`-F`; `~/.ssh` is never read),
connects the way an SSH profile does and completes over the shared
connection in bash, zsh and fish.

**Performance.** `packaging/bench.sh` measures startup, a second window,
output throughput, memory, key-to-frame latency and TUI round trips;
compare against
[performance.md](performance.md) on the same machine.

## Architecture

```
rift-terminal     winit app, views, input routing, a11y, control socket, script driver, update restarts, `remote` command
  ├── rift-renderer   wgpu quad + sprite pipelines, glyphon text, Scene layers
  ├── rift-pty        PTY sessions, reader threads, vte, images, session service
  ├── rift-core       config, themes, keymap, OSC scanner, blocks, completions, git, update check/install
  └── rift-remote     mobile wire protocol, Noise XX transport, device registry, WebSocket gateway (over a Source trait), relay client and relay (`rift-relay`)

mobile/           Expo companion (iOS, Android): pnpm, outside the Cargo workspace
```

**One process, many windows.** `App` (`app.rs`) holds what every window
shares — config and its watcher, the theme and font metrics, the
`TextEngine` (fonts, glyph atlas, shape cache), the git/PR/Docker and
completion workers, the global hotkey, menu bar, update state, control
socket and attention center — and `win: Box<WindowState>`, the window being
served: its winit window, GPU surface (`Gpu::for_window` shares the first
window's device and queue), quad/sprite/image pipelines, scene, tabs,
overlays, panels, menus, selection, toasts and accessibility adapter. The
other windows wait in `App::parked`. `app/windows.rs` routes: a window event
serves its `WindowId`'s window, a PTY event or a control request naming a
pane serves that pane's window (pane ids come from one counter), and
everything else (menu commands, the hotkey, `split`/`open`) serves the
window that last had focus; `each_window` runs per-window work (config
changes, timers, repaints) over all of them. Code reads the window it
serves as `self.win.<field>`; anything that must cover every window uses
`all_tabs`/`all_panes`.

The remote gateway (`rift-terminal remote`) is a separate process, started
by Rift while `remote_access` is on: it reads and drives every window
through the control socket (`status`, `blocks`, `screen`, `output`,
`menu`, `diff`, `complete`, `remote-input`, `remote-open`,
`remote-close`, `remote-reset-elapsed`), so
networking never runs inside a window. Its format is
[remote-protocol.md](remote-protocol.md); the app and the crate decode the
same fixtures, and the TypeScript Noise implementation replays the
transcript `snow` writes to `crates/rift-remote/fixtures/noise/xx.json`
(regenerate with `RIFT_UPDATE_FIXTURES=1 cargo test -p rift-remote
--test noise_fixture`). `crates/rift-remote/examples/phone.rs` is a scripted
phone for live checks (`--relay-only` forces the relay path).
`rift-terminal ctl request '<json>'` sends any raw control request.

The relay (`rift_remote::relay`) builds as the standalone `rift-relay`
binary with `cargo build -p rift-remote --no-default-features --bin
rift-relay`: without the default `tls` feature (rustls with `ring` and
webpki roots, which only the gateway needs to dial `wss://` relays) it has
no TLS or GUI dependencies. `packaging/relay.Dockerfile` builds it into a
slim non-root image; its header has a compose snippet with Caddy in front
for TLS. The `ui` gate uses `RIFT_RELAY_BIN` (built by
`scripts/ui-smoke.sh` when unset).

Checking the companion against a real `wss://` relay without a domain: run
`rift-relay --listen 127.0.0.1:PORT --trust-forwarded` and
`cloudflared tunnel --url http://127.0.0.1:PORT` (a temporary
`*.trycloudflare.com` address with a real certificate), and set
`remote_relay` to its `wss://` form. The app's hidden `rift://diagnostics`
screen times a Noise handshake and 1 MiB sealed and opened in gateway-sized
chunks on the device's JavaScript engine (Hermes on an arm64 emulator and
the iOS 27 simulator: about 90 ms per handshake and 220–280 ms per MiB each
way, so about 15–18 ms per 65 000-byte frame); run it on a real phone before
deciding a native cipher is needed. Android emulators need `-dns-server
1.1.1.1` when headless for a working network; a phone image
(`system-images;android-35;google_apis`) has the cellular radio that
`svc wifi disable` hands over to.

Dependencies flow downward only; `rift-core` has no graphics dependencies.
The load-bearing invariants (absolute line model, resize compensation, PTY
fd blocking, viewport ownership, editor vs raw mode, sRGB blending, marker
idempotence) are listed in [CLAUDE.md](../CLAUDE.md#load-bearing-invariants).
Read them before changing `rift-pty` or the terminal view.

### Data flow

1. The shell writes to the PTY. A reader thread in `rift-pty` reads it,
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
| `ttf-parser` 0.25 (renderer) | Font picker: fixed-width and outline checks | Already built for glyphon |
| `winit` 0.30 | Windows, input, IME | 0.31 is beta |
| `alacritty_terminal` 0.26, `vte` 0.15 | VT emulation, PTY spawn | Vendored in `vendor/` with the VT420 emulation, Xcms colors, wcwidth cluster widths, `Term::primary_grid` and compacted scrollback rows; only `tty`, `Term`, `vte`, `FairMutex` are used |
| `bytemuck`, `pollster` | GPU buffers, blocking on adapter setup | |
| `image`, `png`, `flate2` | Inline images, screenshots, kitty zlib payloads | |
| `serde`, `toml`, `serde_yaml_ng`, `serde_json` | Config, themes, Warp YAML, IPC | |
| `toml_edit` | Settings saves edit `config.toml` in place (comments, order, unknown keys) | |
| `arc-swap`, `notify` | Lock-free config snapshot, hot reload | |
| `nucleo-matcher` | Fuzzy ranking in the palette | |
| `arboard` | Clipboard (Wayland data-control) | |
| `git2` (no default features) | Branch, ahead/behind, dirty | Background thread |
| `dirs` | Home and standard directories | |
| `ed25519-compact` (no default features) | Verifies the release manifest's signature | No dependencies; verify only |
| `log`, `env_logger` | Logging (`RUST_LOG=info`) | |
| `tungstenite` 0.30 (`handshake` only) | The remote gateway's WebSocket server (`rift-remote`) | Synchronous, thread per client like the control socket; no TLS features |
| `snow` 0.10 (pure-Rust resolver: curve25519, chacha20poly1305, blake2, getrandom) | Noise XX for the remote gateway | No `std` feature, so no `ring` |
| `qrcode` 0.14 (no default features), `sha2` 0.10, `base64` 0.22 | Pairing QR matrix, hashed pairing secrets, key encoding | |
| `rustls` 0.23 (`std`, `ring`, `tls12`; via `tungstenite/rustls-tls-webpki-roots`) | The gateway dialing a `wss://` relay | `rift-remote`'s default `tls` feature; no OpenSSL or cmake; the relay binary builds without it |

App-only: `accesskit` / `accesskit_winit` (screen readers), `global-hotkey`,
`regex` (find), `libc`, `unicode-segmentation` (grapheme-aware editing), `unicode-normalization` (find matches canonically equivalent text), `zeno` (rasterizes brand marks Nerd Fonts lacks,
such as the Claude mark; already built for glyphon); on macOS `muda` (menu bar), `objc2`,
`objc2-app-kit`, `objc2-foundation`, `objc2-user-notifications` and `block2`
(desktop notifications posted by Rift.app). `rift-pty` also uses `libc` for PTY
and socket calls, and on macOS `objc2` / `objc2-foundation`; `rift-core` uses
`regex` (secret masking) and `libc`; `rift-remote` uses `libc`.

Before adding a dependency, check whether the standard library or an existing
crate covers it. Keep default features off where the crate allows.

## Packaging and releases

`make-app.sh`, `make-deb.sh`, `make-appimage.sh` and `make-tarball.sh` take
`--target <triple>`, `--target-dir <dir>` and `--no-build`; `make-dmg.sh`
takes `--target-dir <dir>` and `--arch <arch>`. They honor
`CARGO_TARGET_DIR` and print the artifact path; `gen-icon.py` takes no
options. `make-app.sh` also takes `--sign <identity>` (default
`$RIFT_SIGN_IDENTITY`, else ad hoc). `release-local.sh` signs with the
self-signed `Rift Signing` code-signing certificate from the login keychain
and stops without it: macOS ties privacy grants (Full Disk Access, Files and
Folders, Accessibility) to the signature, and an ad-hoc one changes with every
build, so each update lost them.

| Script | Platform | Produces |
|---|---|---|
| `packaging/gen-icon.py` | any | `assets/icon/`: 1024 PNG, `.iconset`, `.icns` (macOS), hicolor PNGs (Linux), `.ico` (Windows), `rift-window.png` (embedded window icon) |
| `packaging/make-app.sh` | macOS | `target/Rift.app` (ad hoc, or `--sign`) |
| `packaging/make-dmg.sh` | macOS | `target/Rift-<version>-<arch>.dmg` (run `make-app.sh` first) |
| `packaging/make-deb.sh` | Linux | `target/rift-terminal_<version>_<arch>.deb` |
| `packaging/make-appimage.sh` | Linux | `target/Rift-<version>-<arch>.AppImage` (pins appimagetool 1.9.1 and verifies SHA-256 before every execution, including cached copies) |
| `packaging/make-tarball.sh` | both | `target/rift-terminal-<version>-<os>-<arch>.tar.gz`, named for the `--target` triple (`macos-arm64`, `macos-x86_64`, `linux-x86_64`), not the build host |

The version lives only in `[workspace.package]` of `Cargo.toml`. Every bump
needs a matching `## <version> — <date>` entry at the top of `CHANGELOG.md`:
the app embeds that file as its What's New page, and
`changelog::tests::embedded_changelog_lists_the_running_version_first` fails
without it.

Releases are published to the **public** repository
[botjaeger/rift-releases](https://github.com/botjaeger/rift-releases)
(this source repository is private, so nothing attached to it can be
downloaded). The app's updater reads
`https://github.com/botjaeger/rift-releases/releases/latest/download/latest.json`.

1. Bump `[workspace.package] version`, add the `CHANGELOG.md` section, run
   `scripts/gates.sh all`, commit, and push a tag `vX.Y.Z` (the tag must
   equal the Cargo.toml version).
2. The tag runs `.github/workflows/release.yml`: the validation workflow
   alongside the package builds — Rift.app/DMG/tarball for Apple silicon and
   Intel (both on `macos-14`; Intel is cross-compiled), the `.deb`, AppImage
   and tarball for Linux. Once validation and all package builds pass,
   `latest.json` is written by `scripts/release-manifest.py` from the
   downloaded artifacts (version,
   date and summary from `CHANGELOG.md`, `protocol` =
   `ipc::PROTOCOL_VERSION`, each asset's URL in the public release and its
   SHA-256) and checked with `--verify`. Everything is uploaded as workflow
   artifacts; the workflow publishes nothing and signs nothing.
3. On your machine, logged in with `gh` (access to both repositories):
   `scripts/publish-release.sh vX.Y.Z` downloads that run's artifacts,
   refuses unless `latest.json` names the tag's version and every asset
   matches its SHA-256, signs `latest.json` (below), and creates the release
   `vX.Y.Z` in `botjaeger/rift-releases` with every package, `latest.json`,
   `latest.json.sig`, and the CHANGELOG section as notes (`--draft` to
   review first; `--run ID` to pick a workflow run).

   `publish-release.sh` also mounts every DMG and refuses one whose
   Rift.app is not signed by a certificate. The workflow signs ad hoc (the
   certificate never reaches CI), so releases are built on the maintainer's
   Mac instead: `scripts/release-local.sh vX.Y.Z` checks out
   the tag in a temporary worktree, builds both macOS targets on the Mac
   (cross-compiling the other architecture) and Linux x86_64 in docker
   (`linux/amd64`, emulated on Apple silicon), and
   writes `latest.json` into `target/release-local/vX.Y.Z`; then
   `scripts/publish-release.sh vX.Y.Z --dist target/release-local/vX.Y.Z`.
   It builds without checking: run `scripts/gates.sh all` on the tag first.
4. Running copies see the release within 6 hours (or at once via *Check for
   updates*).

The maintainer's `/release` skill (`.claude/skills/release/`) runs this end to
end. `.claude/skills/release/plan.sh` prints, per product, the last tag, the
user-facing files changed since it and, for the phone app, whether its expo
fingerprint still matches the last native build's. The terminal and the
phone app release independently:

| Product | Tag | Version | Changelog | Reaches users as |
|---|---|---|---|---|
| Terminal | `vX.Y.Z` | `[workspace.package]` in `Cargo.toml` | `CHANGELOG.md` | the steps above |
| Phone app (`mobile/`) | `app-vX.Y.Z` (native build), `app-vX.Y.Z-uN` (over-the-air update N to it) | `mobile/app.json` (+ `mobile/package.json`); `APP_UPDATE` in `mobile/src/release.ts` | `mobile/CHANGELOG.md` | an EAS update (`mobile/scripts/eas.sh update`) while the native code is unchanged, else a native build installed on the devices |

Each app tag is annotated with the `runtime ios=<fp> android=<fp>`
fingerprints it targets (`runtimeVersion` policy `fingerprint`): an update
reaches only the installed builds with the same fingerprint, so an OTA
release keeps the version, and a change to native code, the version or
`package.json` `scripts` needs a native build.

**Native iOS build under Xcode 27.** `mobile/ios/` is generated
(`npx expo prebuild -p ios`, git-ignored); never edit it by hand. Xcode 27
(Swift 6.4) writes module selectors into `.swiftinterface` files
(`extension __ObjC::expo…`), which `expo-modules-jsi`'s
`apple/scripts/build-xcframework.sh` does not strip (`/^extension __ObjC\./`),
so `ExpoModulesCore` fails with `cannot find type '__ObjC::expo'`. Until an
`expo-modules-jsi` release fixes it (a dependency bump would change the
fingerprint), patch the pattern to `/^extension __ObjC[.:]/` in that script (a
transitive dependency: `mobile/node_modules/.pnpm/expo-modules-jsi@*/node_modules/expo-modules-jsi`;
the fingerprint does not cover it), delete the `.build-hash` of the `ios-arm64*` slices in
`apple/Products/ExpoModulesJSI.xcframework`, and keep
`EXPO_USE_PRECOMPILED_MODULES` `false` (the generated
`ios/Podfile.properties.json`). Re-apply after every `pnpm install`.

### Release signing

The asset digests in `latest.json` come from the same public repository as
the assets, so anyone who can write to `botjaeger/rift-releases` could
otherwise ship code to every install. The updater therefore fetches
`latest.json.sig` next to the manifest — base64 of an Ed25519 signature over
`latest.json`'s exact bytes — and refuses the manifest (*Couldn't check for
updates*; the log says `release signature is missing` or `is invalid`)
unless it verifies against one of `RELEASE_KEYS` in
`crates/rift-core/src/update.rs`. The cached copy in `update.json` is
re-verified every time it is read.

- **The private key** lives only at
  `~/.config/rift-release/signing-key.pem` on the maintainer's machine
  (directory `0700`, file `0600`; `RIFT_SIGNING_KEY` points elsewhere). It
  is never committed and never given to CI. **Back it up** offline (a
  password manager or an encrypted drive): losing it means installed copies
  cannot verify any new release until they are updated by hand.
- `scripts/sign-manifest.sh latest.json` writes `latest.json.sig` with
  `openssl pkeyutl -sign -rawin` (OpenSSL 3 or newer; macOS's LibreSSL has no
  Ed25519 — `brew install openssl@3`), refuses a key readable by group or
  others, and checks the signature against `RELEASE_KEYS` before it
  finishes; `--verify latest.json` only checks an existing signature.
  `publish-release.sh` runs both.
- **A new key pair**: `openssl genpkey -algorithm ed25519 -out
  signing-key.pem`; the public key for `RELEASE_KEYS` is the last 32 bytes
  of `openssl pkey -in signing-key.pem -pubout -outform DER`, base64.
- **Rotation**: add the new public key to `RELEASE_KEYS` and ship a release
  signed with the old key; sign later releases with the new key; drop the
  old public key once installs have moved past that release. A compromised
  key is removed the same way, but copies that never installed the rotating
  release keep trusting it.

When a change alters the session service's wire format,
`ipc::tests::protocol_matches_the_golden_wire_format` fails: bump
`PROTOCOL_VERSION` and update the golden. The manifest then carries the new
protocol, and the update dialog warns that running shells cannot be
reattached across that update. Prefer an addition both sides tolerate
instead: an optional field (`#[serde(default, skip_serializing_if = …)]`)
that an older peer ignores, plus a `FEATURE_*` name the service lists in its
hello when the window must know the service honours it (profile `args`:
a service without `FEATURE_ARGS` is not asked to start such a shell; the
window starts it locally; `FEATURE_PRIVACY`: only a service that lists it is
asked `Request::Privacy`, Settings › Permissions shows "Unknown · restart
Rift" for the others). Update the golden; the version stays. Check it
against the installed release's service with
`scripts/service-compat.py`.

`scripts/release-manifest.py VERSION DIR --base-url file://DIR
--allow-missing` writes a manifest for local files; sign it with
`scripts/sign-manifest.sh`, or with a throwaway test key whose public half
goes in `RIFT_UPDATE_TEST_KEY` (trusted for `file://` manifests only;
`scripts/ui-smoke.py` signs its manifests that way). With
`RIFT_UPDATE_URL=file://…/latest.json` a scratch copy of Rift.app (built
twice with `make-app.sh`, the newer one packed by `make-dmg.sh`) can run a
whole update headless (`RIFT_RELAUNCH_SCRIPT` names the script the
restarted app runs).

The helper starts the new version only once the old process is gone. Every
quit goes through `quit::exit` (`crates/rift-terminal/src/quit.rs`): on macOS
`event_loop.exit()` cannot stop the app while a menu opened from the menu bar
is open (the loop spins, and memory grows, until the menu closes), so after
3 s a watchdog ends the process. Every caller has saved the session by then.

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
