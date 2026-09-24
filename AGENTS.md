# Agent guide — Shift

Guidance for coding agents (Claude Code, Codex, others) working in this
repository. `CLAUDE.md` and `AGENTS.md` are identical copies — edit both
(`scripts/gates.sh docs` fails when they differ). Visual rules live in
[DESIGN.md](DESIGN.md); the product reference lives in [docs/](docs/README.md).

## What this is

Shift is a GPU-accelerated, block-based terminal emulator for macOS and Linux,
written in Rust (winit, wgpu, glyphon, alacritty_terminal). It implements the
Claude Design project `b819116d-9d1b-4daa-a29d-edc798d62544` 1:1 (files
`Runway.dc.html` and `Runway Design Spec.dc.html`; fetch them with the
DesignSync tool when a pixel value or design intent is in question). Colors and
metrics come from that spec — never improvise visual values. Keybindings follow
Warp's default macOS layout (user decision; supersedes spec §4: ⌘K = clear
blocks, ⌘P = palette).

## Safety when an agent runs here

- The user may be running an agent **inside a Shift window**. Never send
  OS-wide keystrokes, close terminal windows or tabs, or signal (`kill`,
  `pkill`, `killall`) the user's Shift, shell or agent processes. Do not use
  job control (`fg`, `bg`) on the parent shell; report suspended jobs instead.
- Every `--script` run must be isolated: set `XDG_CONFIG_HOME` and
  `XDG_DATA_HOME` to scratch directories and `SHIFT_NO_NOTIFY=1`. Without
  them the scripted instance reads the user's config, writes their session
  file, and can attach to the user's live session service and control sockets.
- `--script` synthesizes input inside the app (no OS input), but it still opens
  a real window. On a machine without a display, use the Linux docker image.

## Commands

```sh
scripts/gates.sh               # quality gates: fmt clippy test doc shell docs
scripts/gates.sh all           # + release build + Linux gates in docker
scripts/gates.sh --list        # what each gate runs
cargo run -p shift-app         # dev build + launch (binary: shift-terminal)
cargo test -p shift-core osc   # one module's tests
cargo fmt --all                # format
packaging/make-app.sh          # macOS Shift.app (release)
packaging/bench.sh             # performance baseline (docs/performance.md)
```

The compiler is pinned in `rust-toolchain.toml` (1.98.0 with rustfmt and
clippy). CI (`.github/workflows/ci.yml`) runs `scripts/gates.sh` on
macos-14 and ubuntu-24.04, so a green local run predicts CI. Run the gates
before reporting work done, and name the gates you ran.

Optional pre-commit hook (fmt + clippy): `git config core.hooksPath .githooks`.

## Verifying UI changes

Rendering has no unit tests. Use the script driver
(`crates/shift-app/src/script.rs`): it synthesizes keys and mouse events
through the app's real routing (`App::on_key`, `App::on_left`, …); `shot`
writes a PNG by offscreen readback (works with the display asleep, needs no
Screen Recording permission); `dump` writes a state snapshot (tabs, panes,
editor, overlays, accessibility tree, attention center).

```sh
mkdir -p /tmp/shift-t/cfg /tmp/shift-t/data
cat > /tmp/shift-t/t.shift <<'EOF'
wait_prompt
type echo hi\n
wait 500
shot /tmp/shift-t/a.png
dump /tmp/shift-t/a.txt
quit
EOF
XDG_CONFIG_HOME=/tmp/shift-t/cfg XDG_DATA_HOME=/tmp/shift-t/data SHIFT_NO_NOTIFY=1 \
  ./target/debug/shift-terminal --script /tmp/shift-t/t.shift
```

Commands (one per line, `#` comments; full list in the `script.rs` module
doc): `wait`, `wait_prompt`, `key`, `keyup`, `type`, `paste`, `shot`, `dump`,
`resize`, `move`, `click`, `dblclick`, `rclick`, `drag`, `scroll`, `sh`,
`focus`, `quit`.

Config hot-reload needs no keystrokes: write the scratch `config.toml` while
the app runs and `shot` the repaint.

**Linux, headless:** `packaging/linux-verify.Dockerfile` (Xvfb + Mesa
lavapipe software Vulkan) builds the release binary and runs the same script
files; its header has the `docker build`/`docker run` lines.
`scripts/gates.sh linux` runs clippy, tests and a release build in it.

## Architecture

Four crates; dependencies flow strictly downward.

- **shift-core** — no graphics deps, unit-tested: theme tokens and TOML/Warp
  themes (`theme/`), config + hot-reload watcher, keymap (`[keybindings]`
  overrides win and unbind the default), OSC/APC byte scanner (`osc.rs`),
  block model (`block.rs`), shell tokenizer, completion providers, git2
  worker, shell-integration materializer, transcripts (`transcript.rs`), SSH
  profiles (`ssh.rs`), session state.
- **shift-pty** — one `Session` per pane. Spawns through
  `alacritty_terminal::tty`, or, with `durable_sessions`, through the per-user
  session service (`daemon.rs`, `service.rs`, `ipc.rs`). A reader thread
  splits output at marker boundaries (`feed`), feeds segments to the vte
  parser, and applies markers to the `BlockStore` and the inline-image model
  (`graphics.rs`).
- **shift-renderer** — wgpu SDF quad pipeline (`quad.wgsl`: rounded rects,
  borders, circles, soft shadows), image sprites (`sprite.rs`), glyphon text.
  `Scene` is a per-frame list of layers; within a layer quads draw first, then
  text; later layers composite over earlier ones (0 chrome, 1 terminal +
  editor, 2 completion/search, 3 settings, 4 palette, 5 context menu/hints,
  6 toasts/dialogs).
- **shift-app** — winit `ApplicationHandler` (`app.rs`), views, input routing,
  per-frame `Hit` rects, kitty keyboard encoder (`keyboard.rs`), AccessKit
  tree (`a11y.rs`), control socket and `ctl` client (`control.rs`), script
  driver, offscreen capture.

### Load-bearing invariants

- **Absolute line model.** Blocks and images anchor to monotone line indices
  that survive scrollback trim. `shift-pty/src/abs_term.rs` is a GENERATED
  wrapper over all 71 vte `Handler` methods that counts lines pushed into
  history (`scrolled`): `abs = scrolled + screen_row`, grid
  `Line = abs - scrolled` (negative = history). If vte's `Handler` trait
  changes, regenerate the wrapper; do not hand-patch methods.
- **Resize compensation.** Row-count changes move lines between history and
  screen; `Session::resize` folds the delta into `scrolled` and remaps block
  and image anchors across column reflow. Without it every anchor drifts when
  the editor shows or hides.
- **PTY fd blocking.** alacritty_terminal makes the master fd non-blocking;
  `Session::spawn` resets it to blocking for the reader thread. Removing that
  silently breaks all reads.
- **The app owns the viewport.** Rendering builds a filtered display-line
  list (prompt echoes replaced by synthesized headers, collapsed output
  skipped, live prompt hidden while the editor is active). `view_offset`
  counts display lines from the bottom; alacritty's `display_offset` is never
  used for UI scroll.
- **Editor mode vs raw passthrough.** The app owns the command line only while
  the last block is in `Prompt` state and not in alt-screen; otherwise keys go
  raw to the PTY (vim, password prompts, shells without integration).
- **sRGB compositing.** The surface format is deliberately non-sRGB and
  glyphon uses `ColorMode::Web`, so blending happens in sRGB space like the
  design mock's CSS. Do not "fix" this to linear.
- **Selection and row cache.** Mouse-to-grid mapping reads the per-frame
  `PaneState.rows` cache filled by `terminal::render`; it never recomputes
  layout. `EditorState::replace_range` is the single buffer-mutation point
  (keeps workflow placeholder spans valid).
- **Marker idempotence.** Users' own iTerm2 integration emits OSC 133 next to
  ours; `BlockStore` handlers tolerate duplicated A/B/C/D markers. Keep new
  handlers idempotent.
- **Locks.** `std::sync::Mutex` is not re-entrant: never call a function that
  locks `blocks` while holding the `blocks` lock — take a snapshot first.
- **Untrusted input.** Terminal output, image payloads, control-socket
  requests and restored transcripts are untrusted: keep the size bounds in
  `graphics.rs`, `ipc.rs` and `transcript.rs`, only direct image transmission
  (`t=d`), and never interpolate block output into a command line.

### Version pins

- wgpu stays on major 29: glyphon 0.11 requires `^29`. Bump only when glyphon
  releases against the new major.
- winit stays on 0.30 (`ApplicationHandler`; 0.31 is beta).
- alacritty_terminal 0.26: only `tty`, `Term`, re-exported `vte` and
  `FairMutex` are used — never its `event_loop`.
- Rust 1.98.0 (`rust-toolchain.toml`); bump together with the `FROM` line of
  `packaging/linux-verify.Dockerfile`.

### Assets

Fonts (JetBrains Mono ×4 weights, Inter variable), builtin themes and shell
hooks are embedded with `include_bytes!`/`include_str!`; the binary has no
runtime assets. Shell-integration files are materialized to
`~/.local/share/shift/` on first spawn; zsh loads through a ZDOTDIR shim that
sources the user's real config first.

## Documentation map

`docs/` is the product reference; update it in the same change as behavior.

| File | Holds |
|---|---|
| `README.md` | What Shift is, why, feature summary, install, quick start |
| `DESIGN.md` | Design tokens, metrics, typography, layering, visual rules |
| `docs/getting-started.md` | First run, everyday use, how each feature is used |
| `docs/features.md` | Feature inventory and exact behaviors |
| `docs/ui.md` | Every UI surface |
| `docs/keyboard-shortcuts.md` | Full keymap (action names), fixed keys, chord syntax |
| `docs/configuration.md` | `config.toml` reference, themes, workflows, file locations |
| `docs/development.md` | Build, test, gates, dependencies, architecture, releases |
| `docs/performance.md` | Benchmark harness and recorded baseline |
| `docs/warp-parity.md`, `docs/terminal-landscape-roadmap.md`, `docs/mobile-remote-plan.md` | Plans and parity status |
