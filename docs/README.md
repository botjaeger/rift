# Shift Documentation

Shift is a GPU-accelerated, block-based terminal emulator for macOS and Linux,
built in Rust on wgpu + alacritty_terminal. Its interaction model follows Warp:
commands become discrete blocks, the command line is an app-owned editor, and
everything is keyboard-drivable.

| Document | Contents |
|---|---|
| [ui.md](ui.md) | Every UI surface: tabs, blocks, editor, palette, settings, search, menus |
| [features.md](features.md) | Feature inventory and behaviors (block lifecycle, clear semantics, selection, completions, workflows, session restore) |
| [keyboard-shortcuts.md](keyboard-shortcuts.md) | Full default keymap (Warp layout), editor keys, search keys, customization |
| [configuration.md](configuration.md) | `config.toml` reference, themes, keybinding overrides, file locations |
| [warp-parity.md](warp-parity.md) | Current non-AI Warp parity matrix, gaps, and implementation order |
| [terminal-landscape-roadmap.md](terminal-landscape-roadmap.md) | Cross-platform terminal survey and prioritized Shift implementation plan |
| [performance.md](performance.md) | Benchmark harness and the recorded performance/compatibility baseline |

## Quick start

```sh
cargo run -p shift-app        # dev build + launch (binary name: shift-terminal)
packaging/make-app.sh          # macOS Shift.app bundle (release)
```

Shell integration (zsh, bash, fish) is injected automatically on first spawn —
no rc-file edits needed. Without it, Shift degrades gracefully to a plain
terminal (no blocks).

## Architecture


See the repository's `CLAUDE.md` for the four-crate architecture
(`shift-core` / `shift-pty` / `shift-renderer` / `shift-app`), load-bearing
invariants, and dependency pins.
