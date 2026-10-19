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
| [../mobile/README.md](../mobile/README.md) | The Expo phone companion: build, run, test |
| [project-audit-2026-10-07.md](project-audit-2026-10-07.md) | Feature coverage, confirmed defects, UX findings, and Claude review handoff |
| [workstreams/](workstreams/) | Dated audit trackers (`AUD-NNN` items, the mobile audits, the sweeps) and their evidence; [audits/](audits/) holds earlier probe captures and the round-2 competitive audits |

## Plans

| Document | Contents |
|---|---|
| [terminal-landscape-roadmap.md](terminal-landscape-roadmap.md) | Terminal survey, prioritized plan and status |
| [warp-parity.md](warp-parity.md) | Non-AI Warp parity matrix and gaps |
| [warp-gaps.md](warp-gaps.md) | Every Warp feature Rift lacks or has only in part, what each is for, and the plan to close them |
| [competitive-audit-2026-10-18.md](competitive-audit-2026-10-18.md) | Warp and Windows Terminal vs Rift, feature by feature: ranked gaps, what Rift leads on, recommended order |
| [audits/warp-round2-2026-10-18.md](audits/warp-round2-2026-10-18.md) | Warp gap audit, round 2 (Rift 0.15.0): what round 1 missed |
| [audits/windows-terminal-round2-2026-10-18.md](audits/windows-terminal-round2-2026-10-18.md) | Windows Terminal vs Rift, round 2 (Rift 0.15.0): what round 1 missed |
| [warp-windows-terminal-opportunities.md](warp-windows-terminal-opportunities.md) | Warp/Windows Terminal comparison and implementation handoff: profiles, block comparison, result search, project grouping, custom actions, and phone continuity |
| [mobile-remote-plan.md](mobile-remote-plan.md) | Mobile companion plan and open decisions |
| [remote-protocol.md](remote-protocol.md) | Wire format and limits between the remote gateway and the companion |
| [mobile-threat-model.md](mobile-threat-model.md) | Trust model and threat model of the remote gateway |
