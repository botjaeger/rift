# Agent guide — Rift

Guidance for coding agents (Claude Code, Codex, others) working in this
repository. `CLAUDE.md` and `AGENTS.md` are identical copies — edit both
(`scripts/gates.sh docs` fails when they differ). Visual rules live in
[DESIGN.md](DESIGN.md); the product reference lives in [docs/](docs/README.md).

## What this is

Rift is a GPU-accelerated, block-based terminal emulator for macOS and Linux,
written in Rust (winit, wgpu, glyphon, alacritty_terminal). It implements the
Claude Design project `b819116d-9d1b-4daa-a29d-edc798d62544` 1:1 (files
`Runway.dc.html` and `Runway Design Spec.dc.html`; fetch them with the
DesignSync tool when a pixel value or design intent is in question). Metrics
come from that spec — never improvise visual values. Colors mirror Warp's
default themes (user decision 2026-10-16; supersedes the spec's colors; the
tokens are in DESIGN.md and `crates/rift-core/src/theme/`). Keybindings follow
Warp's default macOS layout (user decision; supersedes spec §4: ⌘K = clear
blocks, ⌘P = palette).

## Safety when an agent runs here

- The user may be running an agent **inside a Rift window**. Never send
  OS-wide keystrokes, close terminal windows or tabs, or signal (`kill`,
  `pkill`, `killall`) the user's Rift, shell or agent processes. Do not use
  job control (`fg`, `bg`) on the parent shell; report suspended jobs instead.
- Every `--script` run must be isolated: set `XDG_CONFIG_HOME` and
  `XDG_DATA_HOME` to scratch directories and `RIFT_NO_NOTIFY=1`. Without
  them the scripted instance reads the user's config, writes their session
  file, and can attach to the user's live session service and control sockets.
- Scripted runs keep shells local (no session service) unless
  `RIFT_PERSIST_SESSION` is set, and never check for updates unless
  `RIFT_UPDATE_URL` is set; a test that sets either must kill the
  `--session-daemon` it started (confirm its `XDG_DATA_HOME` with `ps eww`).
- `--script` synthesizes input inside the app (no OS input), but it still opens
  a real window that takes focus, and the user's typing then lands in it. Add
  `--headless` to every agent run: no window (⌘N's included) is ever shown
  or focused, the clipboard is private to the run, and links and the global
  hotkey are skipped. On a machine without a display, use the Linux docker
  image.

## Commands

```sh
scripts/gates.sh               # quality gates: fmt clippy test doc shell docs mobile
scripts/gates.sh ui            # scripted UI regressions (scripts/ui-smoke.py)
scripts/gates.sh leak          # memory soak: repeated work must level off (scripts/leak-soak.py)
scripts/gates.sh all           # + release build + scripted UI + memory soak + Linux gates in docker (beside the others)
scripts/gates.sh --list        # what each gate runs
cargo run                      # dev build + launch (binary: rift-terminal)
cargo test -p rift-core osc   # one module's tests
cargo fmt --all                # format
packaging/make-app.sh          # macOS Rift.app (release)
packaging/bench.sh             # performance baseline (docs/performance.md)
scripts/publish-release.sh vX.Y.Z  # publish a tag's release artifacts (docs/development.md)
```

The compiler is pinned in `rust-toolchain.toml` (1.98.0 with rustfmt and
clippy). CI (`.github/workflows/ci.yml`) runs `scripts/gates.sh` on
macos-14 and ubuntu-24.04. Only `scripts/gates.sh all` covers both jobs (the
`linux` gate is the Ubuntu one): run it before every push, so a green local
run means a green CI. Run the gates before reporting work done, and name the
gates you ran.

Optional pre-commit hook (fmt + clippy + docs): `git config core.hooksPath .githooks`.

## Releases

Rift is in use as an installed app: users get changes only through a release.
The terminal updates through *Check for updates* (tags `vX.Y.Z`, published to
`botjaeger/rift-releases`). The phone app gets an over-the-air update, or a
native rebuild when its native code changed (tags `app-vX.Y.Z[-uN]`).
`/release` (the local skill in `.claude/skills/release/`) decides which of the
two ship, sets the versions, writes `CHANGELOG.md` / `mobile/CHANGELOG.md`, then
tags, builds and publishes.

- After pushing work to `main`, run `.claude/skills/release/plan.sh`. When it
  shows user-facing changes that have not been released, end the report with one
  line naming what would ship, e.g. "Release terminal (2 fixes) and an app update?".
  Docs, tests, CI and scripts alone never prompt a release.
- Never tag, publish or run `pnpm ota` without the user's yes or `/release`.
- Never edit a released `CHANGELOG.md` section or move a pushed tag.

## Verifying UI changes

Rendering has no unit tests. Use the script driver
(`crates/rift-terminal/src/script.rs`): it synthesizes keys and mouse events
through the app's real routing (`App::on_key`, `App::on_left`, …); `shot`
writes a PNG by offscreen readback (works with the display asleep, needs no
Screen Recording permission); `dump` writes a state snapshot (tabs, panes,
editor, overlays, accessibility tree, attention center).

```sh
mkdir -p /tmp/rift-t/cfg /tmp/rift-t/data
cat > /tmp/rift-t/t.rift <<'EOF'
wait_prompt
type echo hi\n
wait 500
shot /tmp/rift-t/a.png
dump /tmp/rift-t/a.txt
quit
EOF
XDG_CONFIG_HOME=/tmp/rift-t/cfg XDG_DATA_HOME=/tmp/rift-t/data RIFT_NO_NOTIFY=1 \
  ./target/debug/rift-terminal --script /tmp/rift-t/t.rift --headless
```

Commands (one per line, `#` comments; full list in the `script.rs` module
doc): `wait`, `wait_prompt`, `key`, `keyup`, `keyrepeat`, `type`, `paste`,
`copyfiles`, `shot`, `dump`, `resize`, `move`, `click`, `dblclick`, `rclick`,
`mclick`, `drag`, `press`, `release`, `mods`, `scroll`, `sh`, `focus`, `ime`,
`preedit`, `drop`, `wait_file`, `new_window`, `window`, `close_window`, `hotkey`,
`quit`. `menu` (native menu bar) works on macOS only; scripts that
must also pass on Linux use key chords instead.

Lasting UI regressions belong in `scripts/ui-smoke.py` (the `ui` gate): add
steps and assert on the `dump` output there.

Config hot-reload needs no keystrokes: write the scratch `config.toml` while
the app runs and `shot` the repaint.

**Linux, headless:** `packaging/linux-verify.Dockerfile` (Xvfb + Mesa
lavapipe software Vulkan) builds the release binary and runs the same script
files; its header has the `docker build`/`docker run` lines.
`scripts/gates.sh linux` runs clippy, tests, rustdoc and a release build in it, then
the shell gate and the scripted UI regressions (under Xvfb) in
`packaging/ci-ubuntu.Dockerfile`, which installs the CI runner's packages
(`packaging/ci-apt.txt`) — the same checks CI runs on Linux.

## Architecture

Five crates; dependencies flow strictly downward. The Expo companion app
lives outside the workspace in `mobile/` (pnpm; the `mobile` gate).

- **rift-core** — no graphics deps, unit-tested: theme tokens and TOML/Warp
  themes (`theme/`), config + hot-reload watcher, keymap (`[keybindings]`
  overrides win and unbind the default), OSC/APC byte scanner (`osc.rs`),
  block model (`block.rs`), shell tokenizer, completion providers, git2
  worker, shell-integration materializer, transcripts (`transcript.rs`), SSH
  profiles (`ssh.rs`), session state, update check and install (`update.rs`).
- **rift-pty** — one `Session` per pane. Spawns through
  `alacritty_terminal::tty`, or, with `durable_sessions`, through the per-user
  session service (`daemon.rs`, `service.rs`, `ipc.rs`). A reader thread
  splits output at marker boundaries (`feed`), feeds segments to the vte
  parser, and applies markers to the `BlockStore` and the inline-image model
  (`graphics.rs`).
- **rift-renderer** — wgpu SDF quad pipeline (`quad.wgsl`: rounded rects,
  borders, circles, line strokes, soft shadows, rounded clips), image sprites
  (`sprite.rs`), glyphon text.
  `Scene` is a per-frame list of layers; within a layer quads draw first, then
  text; later layers composite over earlier ones (0 chrome, 1 terminal +
  editor, 2 completion/search, 3 settings, 4 palette, 5 context menu/hints,
  6 toasts/dialogs, 7 tooltips).
- **rift-terminal** — winit `ApplicationHandler` (`app.rs`), views, input
  routing, per-frame `Hit` rects, kitty keyboard encoder (`keyboard.rs`),
  AccessKit tree (`a11y.rs`), control socket and `ctl` client (`control.rs`), script
  driver, offscreen capture, update restarts across processes (`updater.rs`),
  the remote gateway's control-socket source (`remote.rs`). One process holds
  every window: `App` keeps the shared state (config, fonts and glyph atlas,
  workers, hotkey, menu bar, control socket) and `win: Box<WindowState>`, the
  window being served (winit window, GPU surface on the shared device,
  pipelines, tabs, overlays); the rest wait in `App::parked`, and
  `app/windows.rs` swaps the right one in per event (window id, pane id, or
  the last focused window). Per-window code reads `self.win.<field>`;
  whatever must cover every window uses `each_window` / `all_panes`.
  `app/hotkey_window.rs` is the drop-down hotkey window (`hotkey_window`), a
  `WindowState` with `dropdown` set: show, hide, slide, focus loss; it does
  not keep the app open (`ordinary_window_count`, `App::quits`).
  `dropdown.rs` holds its geometry and the macOS hooks (window level,
  Spaces, pointer screen, reduced motion, handing focus back).
- **rift-remote** — no app or graphics deps: the mobile wire protocol
  (`protocol.rs`, fixtures shared with `mobile/` in `fixtures/`), the Noise
  transport (`transport.rs`), the device registry (`registry.rs`), the
  WebSocket gateway (`server.rs`: auth, limits, diff, replay ring) over a
  `Source` trait, its relay client (`uplink.rs`) and the relay itself
  (`relay.rs`, also the standalone `rift-relay` binary). Spec:
  `docs/remote-protocol.md`; a protocol change edits the fixtures first.

### Load-bearing invariants

- **Absolute line model.** Blocks and images anchor to monotone line indices
  that survive scrollback trim. `rift-pty/src/abs_term.rs` is a GENERATED
  wrapper over every vte `Handler` method that counts lines pushed into
  history (`scrolled`): `abs = scrolled + screen_row`, grid
  `Line = abs - scrolled` (negative = history). A method it does not forward
  silently runs the trait's no-op default; `every_handler_method_is_forwarded`
  fails then. Add the delegation; do not hand-patch the existing ones.
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
  (undo snapshots, selection reset).
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
- alacritty_terminal 0.26 and vte 0.15 are vendored in `vendor/` (wired in by
  `[patch.crates-io]`, outside the workspace) and carry the VT420 emulation,
  the Xcms color parser (`vendor/vte/src/xcms.rs`) and wcwidth-compatible
  cluster widths (`term/width.rs`; regenerate `width_tables.rs` with
  `scripts/gen-width-tables.py`), `Term::primary_grid` (the remote
  gateway reads block output from the primary grid under the alternate
  screen), and compacted scrollback rows (`grid/row.rs`: a row entering the
  history drops its run of equal trailing cells; reads clamp to the last
  stored cell, writes restore the row, and row slices exist only through
  `cells_mut`. The trailing-cell scan trusts `Row::occ` — every cell from
  it on equals the last — so a new row mutation must keep that true). The
  fmt, clippy and test gates run on them too. Only `tty`, `Term`,
  re-exported `vte` and `FairMutex` are used — never alacritty's
  `event_loop`.
- Rust 1.98.0 (`rust-toolchain.toml`); bump together with the `FROM` line of
  `packaging/linux-verify.Dockerfile`.

### Assets

Fonts (JetBrains Mono ×6 faces, Hack ×4 faces, Inter variable, a Nerd
Font symbols subset), builtin themes and shell hooks are embedded with `include_bytes!`/`include_str!`; the binary has no
runtime assets. Shell-integration files are materialized to
`~/.local/share/rift/` on first spawn; zsh loads through a ZDOTDIR shim that
sources the user's real config first.

## Documentation map

`docs/` is the product reference; update it in the same change as behavior.

| File | Holds |
|---|---|
| `README.md` | What Rift is, why, feature summary, install, quick start |
| `DESIGN.md` | Design tokens, metrics, typography, layering, visual rules |
| `docs/getting-started.md` | First run, everyday use, how each feature is used |
| `docs/features.md` | Feature inventory and exact behaviors |
| `docs/ui.md` | Every UI surface |
| `docs/keyboard-shortcuts.md` | Full keymap (action names), fixed keys, chord syntax |
| `docs/configuration.md` | `config.toml` reference, themes, launch configs, file locations |
| `docs/development.md` | Build, test, gates, dependencies, architecture, releases |
| `docs/performance.md` | Benchmark harness and recorded baseline |
| `docs/warp-parity.md`, `docs/warp-gaps.md`, `docs/terminal-landscape-roadmap.md`, `docs/mobile-remote-plan.md` | Plans and parity status |
| `docs/remote-protocol.md`, `docs/mobile-threat-model.md` | Mobile gateway wire format, limits, trust and threat model |
| `docs/project-audit-2026-10-07.md`, `docs/audits/`, `docs/workstreams/` | Dated audit records and their evidence (historical; not updated with behavior) |
