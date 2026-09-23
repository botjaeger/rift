# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Runway: a GPU-accelerated terminal emulator (macOS + Linux) implementing, 1:1, the Claude Design project `b819116d-9d1b-4daa-a29d-edc798d62544` (files `Runway.dc.html` and `Runway Design Spec.dc.html` — fetch via the DesignSync tool when pixel values or design intent are in question). All colors and metrics come from that spec; don't improvise visual values. Keybindings follow Warp's default macOS layout (user decision, supersedes spec §4 — e.g. ⌘K = clear blocks, ⌘P = palette).

## Documentation

`docs/` is the product reference — keep it in sync when changing behavior:

- `docs/ui.md` — every UI surface (tabs, blocks, editor + chips, palette, settings, search, context menu, splits/zoom)
- `docs/features.md` — feature inventory + behaviors (block lifecycle, clear semantics soft `clear` vs ⌘K, input model, completions, workflows, session restore, shell integration) and the deferred-by-decision list
- `docs/keyboard-shortcuts.md` — full default keymap (action names), fixed editor/search keys, `[keybindings]` chord syntax
- `docs/configuration.md` — config.toml reference, themes, workflows, file locations

## Commands

```sh
cargo build                    # dev build
cargo run -p runway-app        # run the terminal (binary name: runway)
cargo test --workspace         # all unit tests (headless; live in runway-core + runway-app/src/pane.rs)
cargo test -p runway-core osc  # single module's tests (osc, block, keybindings, highlight, completion, config, theme)
cargo clippy --workspace --all-targets -- -D warnings   # CI gate, keep at zero
cargo fmt --all
packaging/make-app.sh          # macOS Runway.app bundle (release)
./target/debug/runway --script <file>   # scripted UI verification (see below)
```

CI (`.github/workflows/ci.yml`) runs fmt-check, clippy `-D warnings`, tests, and a release build on macos-14 + ubuntu-24.04.

### Verifying UI changes

There is no test coverage for rendering. **Preferred: the script driver** (`crates/runway-app/src/script.rs`) — it synthesizes keys and mouse events through the app's *real* routing (`App::on_key`, `App::on_left`, …), `shot` writes a PNG via offscreen readback (no `screencapture`, no Screen Recording permission) and `dump` writes a state snapshot (tabs, panes, editor, palette/settings/search/completion state). Because nothing is delivered through the OS, it is safe to run while the user is at the machine.

```sh
cat > /tmp/t.runway <<'EOF'
wait_prompt
type echo hi\n
wait 500
shot /tmp/a.png
dump /tmp/a.txt
quit
EOF
./target/debug/runway --script /tmp/t.runway
```

One command per line, `#` comments; the full list is the module doc comment at the top of `script.rs` (`wait`, `key`, `type`, `shot`, `dump`, `resize`, `move`, `click`, `dblclick`, `rclick`, `drag`, `scroll`, `sh`, `focus`, `wait_prompt`, `quit`). Set `XDG_CONFIG_HOME=/tmp/rw-cfg` to run against an isolated config/theme/launch-config tree instead of the user's, and `RUNWAY_NO_NOTIFY=1` to keep notifications and the audible bell out of the session.

Theme/config hot-reload can be verified with zero keystrokes: write `~/.config/runway/config.toml` while the app runs and screenshot the repaint. Falling back to a real window plus `screencapture -x -R<x>,<y>,<w>,<h>` still works; if driving keys via osascript, check the frontmost process is `runway` first (synthetic keystrokes otherwise land in the user's other apps), and wait for the shell prompt before typing — earlier keys get kernel-echoed at the wrong grid line and look like rendering bugs.

## Architecture

Four-crate workspace; dependencies flow strictly downward:

- **runway-core** — no gfx deps, fully unit-testable: theme tokens + TOML themes (`theme/`), config + hot-reload watcher, keymap (defaults per spec §4, `[keybindings]` overrides win and also unbind the default), OSC 133/7 byte scanner (`osc.rs`), block model (`block.rs`), shell tokenizer, completion providers, git2 worker thread, shell-integration file materializer.
- **runway-pty** — one `Session` per pane: spawns via `alacritty_terminal::tty`, owns a blocking reader thread that splits PTY bytes at OSC-marker boundaries, feeds each segment to the vte parser, and applies markers to the shared `BlockStore`.
- **runway-renderer** — wgpu 29 SDF quad pipeline (`quad.wgsl`: rounded rects, borders, circles, soft shadows) + glyphon text engine. `Scene` is a per-frame draw list of layers; within a layer quads render first, then text; later layers composite over earlier ones (0 chrome, 1 terminal+editor, 2 completion/search, 3 settings, 4 palette).
- **runway-app** — winit `ApplicationHandler`, all views, input routing, per-frame `Hit` rects for mouse targets.

**Linux, headless:** `packaging/linux-verify.Dockerfile` (Xvfb + Mesa lavapipe software Vulkan) builds the release binary in docker and runs the same `--script` files; the header comment has the exact `docker build`/`docker run` lines. Verified 2026-09-04: blocks, tabs, splits, settings overlay, zsh and bash integration all render on `llvmpipe`.

### Load-bearing invariants

- **Absolute line model.** Blocks anchor to monotone line indices that survive scrollback trim. `runway-pty/src/abs_term.rs` is a GENERATED delegating wrapper over all 71 vte `Handler` methods; it intercepts linefeed/scroll/clear to count lines pushed into history (`scrolled`). `abs = scrolled + screen_row`; `grid Line = abs - scrolled` (negative = history). If vte's Handler trait changes, regenerate the wrapper — don't hand-patch method-by-method.
- **PTY fd blocking.** alacritty_terminal sets the master fd non-blocking (it expects mio polling). `Session::spawn` resets it to blocking with fcntl for the reader thread. Removing that call silently breaks all reads.
- **The app owns the viewport.** Rendering builds a filtered display-line list (block prompt echoes replaced by synthesized headers, collapsed output skipped, live prompt hidden when the owned editor is active); `view_offset` counts display lines from the bottom. alacritty's `display_offset` is never used for UI scroll.
- **Editor mode vs raw passthrough.** The app owns the command line only while the last block is in `Prompt` state and not alt-screen; otherwise keys go raw to the PTY (vim, password prompts, bash-without-integration all rely on this fallback).
- **sRGB compositing.** Surface format is deliberately non-sRGB and glyphon uses `ColorMode::Web`, so alpha blending happens in sRGB space — matching how the design mock composites in CSS. Don't "fix" this to linear.
- **Resize compensation.** Row-count changes move lines between history and screen; `Session::resize` folds the history delta into the `scrolled` atomic (canonical, all writers hold the term lock). Without it every abs-line anchor drifts after the editor shows/hides.
- **Selection & row cache.** Mouse→grid mapping never recomputes layout; it reads the per-frame `PaneState.rows` cache filled by `terminal::render`. Selection uses alacritty's `Selection` in the Term (rotates with scrollback automatically); `EditorState::replace_range` is the single buffer-mutation point that keeps workflow placeholder spans valid.
- **Marker idempotence.** iTerm2 shell integration in a user's zshrc emits its own OSC 133 alongside ours; `BlockStore` handlers tolerate duplicated A/B/C/D markers. Keep new handlers idempotent too.

### Version pins

wgpu is pinned to major 29 because glyphon 0.11 requires `^29` — do not bump wgpu until glyphon releases against the new major. winit stays on 0.30 (`ApplicationHandler` API; 0.31 is beta). alacritty_terminal 0.26 (only `tty`, `Term`, re-exported `vte`, `FairMutex` are used — never its `event_loop`).

### Assets

Fonts (JetBrains Mono ×4 weights, Inter variable), builtin theme TOMLs, and shell hooks are all embedded via `include_bytes!`/`include_str!` — the binary has zero runtime assets. Shell-integration files are materialized to `~/.local/share/runway/` on first spawn; zsh hooks inject via a ZDOTDIR shim that sources the user's real config first.
