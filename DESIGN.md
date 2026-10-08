# Shift design reference

The rules for anything Shift draws. The source of truth is the Claude Design
project `b819116d-9d1b-4daa-a29d-edc798d62544` (`Runway.dc.html`, the screens,
and `Runway Design Spec.dc.html`, the tokens and metrics). Fetch it with the
DesignSync tool when a value is not listed here. Never invent a color, size,
radius or duration: take it from the spec, the theme tokens, or an existing
constant.

## Principles

- **Content first.** Terminal output is the product. Chrome is quiet: one
  hairline, muted labels, color only where it carries state.
- **State lives on edges and badges.** A block's result is its 2 px left
  border and its badge (`✓ 0`, `✕ 1`, `running`). Failed blocks are not
  tinted; only the selected block gets a wash.
- **Everything is reachable by keyboard** and has a palette entry. The mouse
  is a shortcut, never the only path.
- **One look across themes.** Views read colors from the active `Theme`, so a
  user theme (TOML or Warp YAML) restyles every surface.
- **Accessible by construction.** Every interactive surface has an AccessKit
  node with a name, role and state (`crates/shift-app/src/a11y.rs`).

## Color tokens

Every rendered color resolves from a theme token (`shift_core::Theme`,
`crates/shift-core/src/theme/`). Values below are `dark-default`.

| Token | Dark default | Use |
|---|---|---|
| `bg` | `#1e1e1e` | Terminal body |
| `bg_chrome` | `#181818` | Tab bar, vertical rail, settings sidebar |
| `bg_gutter` | `#1a1a1a` | Space between split panes |
| `surface` | `#232326` | Modals, dropdowns, menus |
| `border` | `#ffffff12` | Hairlines (white 7 %) |
| `pane_border_inactive` | `#3c3c3c` | Unfocused pane outline |
| `fg` | `#d4d4d4` | Default terminal text |
| `text_bright` | `#f0f1f3` | Commands, emphasized text |
| `text_ui` | `#e6e7ea` | Chrome labels |
| `text_dim` | `#8a8d96` | Secondary text |
| `text_muted` | `#6f7178` | Tertiary text, metadata, hints |
| `accent` | `#569cd6` | Focus, selection wash, running state, links |
| `success` | `#22c55e` | Exit 0, prompt glyph, git branch |
| `error` | `#ef4444` | Non-zero exit, invalid input |
| `warning` | `#d7ba7d` | Vim `NORMAL`, cautions |
| `cursor` | `#d4d4d4` | Cursor |
| `selection` | `#264f78` | Text selection |
| `search_match` / `search_active` | `#ffd700` / `#ff8000` | Find highlights |
| `syntax.command` / `flag` / `string` / `ghost` | `#569cd6` / `#9cdcfe` / `#ce9178` / `#5a5c63` | Editor highlighting and ghost text |
| `ansi[0..16]` | xterm values | Program colors |

Builtin themes: `dark-default`, `light-default`, `catppuccin-mocha`,
`tokyo-night`. A Warp YAML theme is mapped onto the full token set
(`theme/warp.rs`). `light-default` has a contrast test; keep text tokens
readable on `bg` when adding a theme.

The only literal colors in views are the spec's overlay values: black scrims
and shadows, and white hairlines and hover tints at fixed alpha. New code uses
tokens; mix or alpha them (`Color::with_alpha`, `Color::mix`) instead of
adding hex values.

## Typography

| Role | Face | Size |
|---|---|---|
| Terminal grid | JetBrains Mono (config `font_family`, embedded fallback) | 13 px default (`font_size`), line height 1.65 |
| Command editor | JetBrains Mono | 14 px, 20 px lines |
| UI chrome, palette, settings | Inter (variable, embedded) | per spec per surface |
| Emphasis | JetBrains Mono Bold, Italic, Bold Italic (static weights) | — |

Fonts are embedded; the binary has no runtime assets. Text is shaped by
glyphon; shaped runs are cached by content.

## Metrics

| Element | Value | Constant |
|---|---|---|
| Tab bar height | 42 px | `TAB_BAR_H` |
| First tab x (after traffic lights) | 78 px | `TABS_X` |
| Minimum tab width | 96 px | `MIN_TAB_W` |
| Vertical rail width / row height | 220 px / 54 px | `RAIL_W`, `RAIL_ROW_H` |
| Pane corner radius / padding | 9 px / 12 px | `PANE_RADIUS`, `PANE_PAD` |
| Split drag handle | 10 px | `HANDLE` |
| Block left border / gap between blocks | 2 px / 2 px | `BLOCK_GAP` |
| Selected block border | 3 px accent | — |
| Block indent | 16 px | `BLOCK_INDENT` |
| Editor height / compact (split) | 72 px / 42 px | `EDITOR_H`, `EDITOR_COMPACT_H` |
| Editor extra line | +20 px per line | `LINE_H` |
| Palette width / row | 600 px / 42 px | `MODAL_W`, `ROW_H` |
| Completion dropdown width / row | 560 px / 34 px | `DROPDOWN_W` |
| Context menu width / row | 220 px / 30 px | `MENU_W` |
| Settings sidebar width / row | 248 px / 52 px | `SIDEBAR_W` |
| File explorer width / row | 260 px / 24 px | `EXPLORER_W`, `ROW_H` |
| Code review width / diff line | 460 px / 22 px | `REVIEW_W` |
| Block cursor | 8 × 18 px; bar and underline 2 px | — |

## Layers and compositing

`Scene` draws layers in order; within a layer quads draw first, then text.

| Layer | Contents |
|---|---|
| 0 | Chrome: tab bar or rail, gutters |
| 1 | Terminal grid, blocks, images, command editor; file explorer and code review panels (`bg_chrome`) |
| 2 | Completion dropdown, find bar, file viewer (over the pane area) |
| 3 | Settings overlay |
| 4 | Palette and its modes (history, workflows, switcher) |
| 5 | Block context menu, hint labels, notification toasts and popover |
| 6 | Toasts, confirm dialogs |

A theme's background image draws before layer 0. Inline images draw under
or over text by their z-index. Blending happens in sRGB
space (non-sRGB surface, glyphon `ColorMode::Web`) to match the mock's CSS
compositing — do not switch to linear.

## Motion

| Effect | Timing |
|---|---|
| Cursor blink | 1.1 s period |
| Visual bell flash | 120 ms, full window |
| Running badge | spinning ring, live elapsed time |
| Background-tab activity | pulsing dot |
| Synchronized output stall | shown after 150 ms |

No other animation. Motion never delays input.

## Components

- **Block header**: chevron, command (`text_bright`), prompt-time
  `~/path git:(branch)` (`text_dim`), then right-aligned badge pill, duration
  and copy button. A long command is cut with `…` before the badge group; the
  context shows only when it fits.
- **Editor**: context row (`⇕ cwd on ⎇ branch ↑n ↓n •`: cwd in `accent`,
  branch in `success`, arrows and dirty dot muted), then `❯` in `success`
  (`error` after a failure, muted when unfocused). Shell-prompt mode shows a
  muted `›`.
- **Modals** (palette and its modes, confirm dialogs): `surface`
  ground, hairline border, soft shadow, dimmed backdrop. Rows show a fuzzy
  match highlight and right-aligned shortcut chips resolved from the live
  keymap.
- **Chips and pills**: rounded, muted text, used for shortcuts, counts
  (`N panes`, `All tabs (N)`), and modes (`NORMAL`).
- **Notifications popover**: `surface` card (radius 12, hairline border,
  soft shadow) right-aligned under the inbox button, no backdrop. Kind colors
  carry meaning: finished `success`, failed `error`, waiting on you
  `warning`, program notification `accent`, bell `text_dim`. Kind badges are
  those colors at 16 % with the mark in full color; an agent badge is its
  brand disc (Claude `#d97757`, Codex `#f2f2f2`, OpenCode `#3b3b40`, Gemini
  `#4285f4`) with a 9 px kind dot ringed in `surface`. Unread is a 6 px
  `accent` dot; titles `text_bright` semibold 14, details `text_dim` 13,
  ages `text_muted` 13.
- **Notification toasts**: 360 px `surface` cards (radius 12) under the
  inbox button, 8 px apart, a 3 px kind-colored stripe on the left edge,
  title semibold 13.5 (two lines), detail 12.5. Tab markers: a 7 px kind
  dot with a 12 px halo at 25 %.
- **Split panes**: focused pane has an accent border and glow; inactive panes
  dim and use `pane_border_inactive`.
- **Keyboard focus** (Settings controls): a 2 px `accent` ring, radius 8,
  3 px outside the control; no fill.
- **Find highlights**: `search_match`, the current match `search_active`,
  text on a highlight `#1e1e1e` — in the terminal and the file viewer alike.
  The find field clips its query and scrolls it so the caret stays visible;
  it never runs under the counter or toggles.
- **Vertical rail overflow**: `+N above` / `+N more` lines carry the most
  urgent hidden tab's marker (same tab-marker colors), so a hidden alert is
  never invisible.
- **Notices in lists** (unreadable folder, cut listing, a review hunk too long
  to act on): muted text in the row's place, never clickable.

## App icon

The prompt and the cursor: a `surface` (`#232326`) rounded tile with a white
10 % hairline, the `❯` in `accent` (`#569cd6`) and a block cursor in `cursor`
(`#d4d4d4`). On the 1024 grid: tile 100–924 with radius 185; chevron
polyline (336,360)–(506,512)–(336,664), 96 wide, round caps and joins; cursor
596,372 to 736,652, radius 16. At 48 px and below the tile grows toward the
canvas edge and loses the hairline, so small icons stay legible.

`packaging/gen-icon.py` draws every size from these numbers and the theme
tokens into `assets/icon/`. Where each file is used:

| Platform | Surface | File |
|---|---|---|
| macOS | Finder, Dock, app switcher (`Shift.app`) | `shift.icns` |
| macOS | Dock for an unbundled `cargo run` | `shift-window.png` (embedded, `icon.rs`) |
| Linux | Launcher, dock, taskbar (via `shift.desktop`, app id `shift`) | `hicolor/*/apps/shift.png` |
| Linux X11, Windows | Window title bar, taskbar, Alt-Tab | `shift-window.png` (embedded window icon) |
| Windows | Executable file | `shift.ico` (for the Windows build) |

Design explorations: the Shift App Icon canvas (Claude Design artifact).

## Verifying visual work

Rendering has no unit tests. Drive the change with the script driver and
compare the `shot` PNG against the design screen (see CLAUDE.md, "Verifying
UI changes"). Check every builtin theme and a split layout, and check the
`dump` accessibility tree names the new element.
