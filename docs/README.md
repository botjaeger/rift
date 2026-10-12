# Rift documentation

Rift is a GPU-accelerated, block-based terminal for macOS and Linux, written
in Rust on wgpu and alacritty_terminal. Every command becomes a block, the
command line is an app-owned editor, and every action is reachable from the
keyboard. The [project README](../README.md) explains what Rift is and why.

## Using Rift

| Document | Contents |
|---|---|
| [getting-started.md](getting-started.md) | First launch, everyday tasks, troubleshooting, files Rift uses |
| [features.md](features.md) | Every feature and its exact behavior |
| [ui.md](ui.md) | Every UI surface: tabs, blocks, editor, palette, settings, dialogs |
| [keyboard-shortcuts.md](keyboard-shortcuts.md) | Full default keymap (Warp layout), fixed keys, rebinding |
| [configuration.md](configuration.md) | `config.toml` reference, themes, launch configs, file locations |

## Building Rift

| Document | Contents |
|---|---|
| [development.md](development.md) | Setup, build, test, gates, UI verification, architecture, dependencies, releases |
| [../DESIGN.md](../DESIGN.md) | Design tokens, metrics, typography, layers, visual rules |
| [../CLAUDE.md](../CLAUDE.md) | Guide for coding agents (same as `AGENTS.md`): invariants, pins, safety |
| [performance.md](performance.md) | Benchmark harness and recorded baseline |
| [project-audit-2026-10-07.md](project-audit-2026-10-07.md) | Feature coverage, confirmed defects, UX findings, and Claude review handoff |

## Plans

| Document | Contents |
|---|---|
| [terminal-landscape-roadmap.md](terminal-landscape-roadmap.md) | Terminal survey, prioritized plan and status |
| [warp-parity.md](warp-parity.md) | Non-AI Warp parity matrix and gaps |
| [mobile-remote-plan.md](mobile-remote-plan.md) | Mobile companion plan and open decisions |
