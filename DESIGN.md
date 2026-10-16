# Rift design reference

The rules for anything Rift draws. The source of truth is the Claude Design
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
  node with a name, role and state (`crates/rift-terminal/src/a11y.rs`).

## Color tokens

Every rendered color resolves from a theme token (`rift_core::Theme`,
`crates/rift-core/src/theme/`). Values below are `dark-default`.

The default themes mirror Warp's colors on Rift's layout (user decision
2026-10-16; it supersedes the spec's colors, not its metrics): `dark-default`
is Warp's "Dark" (background `#050505`, foreground `#ffffff`, accent
`#19aad8`), `light-default` Warp's "Light". Warp derives its UI colors by
laying the foreground over the background at fixed opacities
(`warp_core/src/ui/theme/color.rs`: surfaces 5/10/15 %, text 90/60/40 %); the
tokens below are those values. Only where a Warp color falls below Rift's
contrast floor is it darkened along its hue (noted in the theme files).

| Token | Dark default | Use |
|---|---|---|
| `bg` | `#050505` | Terminal body |
| `bg_chrome` | `#000000` | Tab bar, vertical rail, settings sidebar |
| `bg_gutter` | `#000000` | Space between split panes |
| `surface` | `#1e1e1e` | Modals, dropdowns, menus (Warp surface 2) |
| `border` | `#ffffff1a` | Hairlines (white 10 %, Warp outline) |
| `pane_border_inactive` | `#383838` | Split pane divider |
| `fg` | `#ffffff` | Default terminal text |
| `text_bright` | `#ffffff` | Commands, emphasized text |
| `text_ui` | `#e6e6e6` | Chrome labels (Warp main text, 90 %) |
| `text_dim` | `#9b9b9b` | Secondary text (Warp sub text, 60 %) |
| `text_muted` | `#828282` | Tertiary text, metadata, hints (50 %) |
| `accent` | `#19aad8` | Focus, selection wash, running state, links |
| `success` | `#1ca05a` | Exit 0, prompt glyph, git branch |
| `error` | `#ff8272` | Non-zero exit, invalid input |
| `warning` | `#e5a01a` | Vim `NORMAL`, cautions |
| `cursor` | `#19aad8` | Cursor (Warp: the accent) |
| `selection` | `#76a7fa66` | Text selection |
| `search_match` / `search_active` | `#ffd700` / `#ff8000` | Find highlights |
| `syntax.command` / `flag` / `string` / `ghost` | `#19aad8` / `#a5d5fe` / `#fefdc2` / `#696969` | the command line in the editor and block headers (command, flags, strings; `ghost` the suggestion); keywords and numbers in the file viewer and review diff |
| `ansi[0..16]` | Warp Dark (`#616161` `#ff8272` `#b4fa72` `#fefdc2` `#a5d5fe` `#ff8ffd` `#d0d1fe` `#f1f1f1`, bright `#8e8e8e` `#ffc4bd` `#d6fcb9` `#fefdd5` `#c1e3fe` `#ffb1fe` `#e5e6fe` `#feffff`) | Program colors |

Builtin themes: `dark-default`, `light-default`, `catppuccin-mocha`,
`tokyo-night`. A Warp YAML theme is mapped onto the full token set
(`theme/warp.rs`). `light-default` has a contrast test; keep text tokens
readable on `bg` when adding a theme.

The only literal colors in views are the spec's overlay values: black scrims
and shadows, and white hairlines and hover tints at fixed alpha; and the
pairing QR code (Settings › Remote access), black modules on a white plate in
every theme so phone cameras can read it. New code uses
tokens; mix or alpha them (`Color::with_alpha`, `Color::mix`) instead of
adding hex values.

Shadows are the shape's box blurred by a gaussian (`Layer::shadow`, sigma in
px), like CSS `box-shadow`: past a corner they fade to a quarter of the edge's
darkness, so a rounded card keeps round corners. Hairlines inside a bordered
card stop 1 px short of each side so they never cross the border.

## Typography

| Role | Face | Size |
|---|---|---|
| Terminal grid | JetBrains Mono (default) or Hack, both embedded; any installed monospace family via `font_family` | 13 px default (`font_size`), line height 1.2 (Warp; the spec drew 1.65) |
| Command editor | JetBrains Mono | 13 px, 20 px lines (Warp's size) |
| UI chrome, palette, settings | Inter (variable, embedded) | per spec per surface |
| Emphasis | JetBrains Mono Medium, SemiBold, Bold, Italic, Bold Italic; Hack Bold, Italic, Bold Italic (static faces) | — |
| Prompt chip marks | Symbols Nerd Font (embedded subset, `views/glyphs.rs`) | 12 px on chips, 13 px in menus, tabs and title bars |

Fonts are embedded; the binary has no runtime assets. Text is shaped by
glyphon; shaped runs are cached by content.

## Metrics

| Element | Value | Constant |
|---|---|---|
| Tab bar height (Warp) | 34 px | `TAB_BAR_H` |
| Traffic light centres (x; y = strip middle) | 20 / 40 / 60 px | `TRAFFIC_X`, `TRAFFIC_GAP` |
| First tab x (after traffic lights) | 78 px | `TABS_X` |
| Tab width (fixed, Warp / squeezed minimum) | 200 px / 120 px | `TAB_W`, `MIN_TAB_W` |
| Chrome icon button | 26 px, icon 16 px | `tabbar::icon_button` |
| Vertical rail width / row height | 220 px / 54 px | `RAIL_W`, `RAIL_ROW_H` |
| Split pane padding | 8 px | `PANE_PAD` |
| Split pane title bar (and file viewer header) | 28 px | `PANE_HEADER_H`, `viewer::HEADER_H` |
| Split divider / grab width | 1 px / +4 px each side | `HANDLE`, `HANDLE_GRAB` |
| Block left border / gap between blocks | 2 px / 2 px | `BLOCK_GAP` |
| Selected block border | 3 px accent (error on a failed block) | — |
| Block indent | 16 px | `BLOCK_INDENT` |
| Editor height / shell-prompt mode | 72 px / 42 px | `EDITOR_H`, `EDITOR_COMPACT_H` |
| Editor extra line | +20 px per line | `LINE_H` |
| Palette width / search / mode chips / row | 640 px / 48 px / 40 px / 36 px | `MODAL_W`, `SEARCH_H`, `MODES_H`, `ROW_H` |
| Completion panel header / row / footer | 32 px / 28 px / 30 px (pane width, ≤ 8 rows) | `HEADER_H`, `ROW_H`, `FOOTER_H` |
| History pane header / row / footer | 32 px / 28 px / 30 px (pane width, ≤ 8 rows) | `history::HEADER_H`, `ROW_H`, `FOOTER_H` |
| Context menu width / row | 240 px / 28 px | `MENU_W`, `ROW_H` |
| Settings sidebar width / row / body | 232 px / 52 px / ≤ 720 px | `SIDEBAR_W`, `ROW_H`, `BODY_MAX_W` |
| Settings controls | 28 px tall; toggle 34 × 20; select 220 px | `CONTROL_H` |
| Welcome list width / row | 560 px / 36 px | `welcome::WIDTH`, `ROW_H` |
| File explorer width / row | 260 px / 24 px | `EXPLORER_W`, `ROW_H` |
| Code review width / diff line / file card | 460 px / 22 px / inset 10, radius 6 | `REVIEW_W`, `CARD_X` |
| Edit prompt card / chip × / chip picker width, row | 600 px, padding 24 / 18 px / 360 px, 26 px | `prompt_editor::MODAL_W`, `X_W`, `chip_menu::MENU_W` |
| Code review gutter / hunk toolbar button / maximized file list | ≥ 72 px / 20 px / 300 px, 26 px rows (panel ≥ 760 px) | `GUTTER_MIN`, `TOOL_W`, `SIDEBAR_W` |
| Block cursor | 8 × 17 px; bar and underline 2 px | `CURSOR_H` |

## Layers and compositing

`Scene` draws layers in order; within a layer quads draw first, then text.

| Layer | Contents |
|---|---|
| 0 | Chrome: tab bar or rail, gutters |
| 1 | Terminal grid, blocks, images, command editor; file explorer (`bg_chrome`) and code review (opaque `bg`; cards: header `surface`, code `bg_chrome`) panels |
| 2 | Completion panel, history pane, find bar, file viewer (over the pane area), sticky block header, visual-bell flash |
| 3 | The Settings tab's page; the maximized code review; the file viewer's inactive-pane dim |
| 4 | Palette and its modes (history, switcher); an open Settings select list |
| 5 | Block context menu, prompt chip pickers, hint labels, notification toasts and popover, pane drop zone, a lifted tab |
| 6 | Toasts, confirm dialogs, What's New, Edit prompt, the dragged pane's chip |
| 7 | Tooltips and chip hover cards (alone on top: text of a shared layer would draw over their plate) |

A theme's background image draws before layer 0. Inline images draw under
or over text by their z-index. Blending happens in sRGB
space (non-sRGB surface, glyphon `ColorMode::Web`) to match the mock's CSS
compositing — do not switch to linear.

Children of a rounded panel (selected rows, accent edges, footers) are drawn
under a rounded clip set to the panel's inner edge (`Layer::set_clip`), so
they follow its corners instead of squaring them off.

## Radii

| Radius | Used for |
|---|---|
| 0 | Docked surfaces: panes and their title bars, tabs, the completion panel, the history pane, side panels |
| 4 | Controls and menus: buttons, icon buttons, selects and their lists, segmented controls, steppers, chips, key caps, context menus, palette rows |
| 6 | Confirm dialog buttons |
| 8 | Floating cards: palette, notifications popover and toasts, find bar, What's New |
| 12 | Confirm dialogs |
| Pill | Toggles, block status badges |

Nothing is rounder than 12, and only the confirm dialog is.

## Icons

One stroked set (`views::icons::Icon`, drawn by `icons::draw`): a 16-unit
grid, 1.5 px strokes with round caps at 16 px, the caller's color. Icons are
line, outline and dot quads, never font glyphs, so they sit centred on the
pixel grid in every font. Sizes: 16 px in chrome and lists, 14 px in rows
and title bars, 11–12 px for × and steppers. Colors: `text_dim` at rest,
`text_ui` on hover, `accent` when on, `error` on destructive rows. The set:
terminal, sidebar, split right/down, plus, minus, close, more (⋮), grip,
search, history, revert, refresh, folder, file, inbox, settings, branch,
copy, check, open (↗), chevrons, sync, maximize, restore, keyboard,
bookmark, filter, appearance, themes, launch, info, lock. Prompt chips and their menu
rows carry Nerd Font glyphs instead (`glyphs.rs`): folder, git branch,
diff, user, laptop, Python, PHP, GitHub, Docker,
calendar, clock; notification badges use its bell and bell outline. A glyph added there must be added to the embedded subset
too. The running spinner in tabs and title bars is drawn, not a glyph: a
third-of-a-turn arc (radius 0.36 × size, stroke 0.13 × size) turning once
per 0.7 s, redrawn every output frame (≤ 120 Hz) so it moves smoothly.

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
  `~/path git:(branch)` (`text_dim`), then right-aligned badge pill, duration,
  copy button and ⋮ menu button (both `text_muted`, on hover; the copy
  button a `success` check after a copy). A long command is cut with `…` before the badge group; the
  context shows only when it fits.
- **Editor**: context chips (18 px tall, 5 px padding, 12 px glyph, `fg` 4 % fill,
  `border` hairline, radius 4, 6 px apart, 12 px mono label centred):
  folder icon + cwd in `text_bright`, branch icon in `success` + branch in
  `text_ui` + arrows and dirty dot muted, file icon + `files • +a −r` in
  `success` / `error` (opt-in chips: docs/ui.md). Then the logo's chevron (11 px
  tall, centred on the block cursor, the logo's proportions with a 2.2 px
  stroke) in `accent`, the logo's blue (`error` after a failure, muted when unfocused).
  Shell-prompt mode shows a 9 px muted chevron.
- **Tabs**: no icon on a shell tab (the sync icon in `accent` when its
  inputs are synced, the settings icon on the Settings tab), the title in
  Inter 12.5, a pane count
  chip (`3`) when split. The active tab sits on the pane ground (no rule
  under it) with a 2 px `accent` top edge (a tab color wins); a hovered tab
  lifts 3 %; tabs are divided by 1 px `border`. × shows on the active and
  hovered tab, else the alert badge or a 6 px `accent` activity dot sits in
  its place. A dragged tab lifts onto the pane ground with a shadow and
  follows the pointer; its slot shows a 2 px `accent` caret.
- **Update chip** (Warp's "Update Warp"): a 22 px pill (radius 11) left of
  the inbox, 1 px `warning` outline and `warning` text (Inter 12 medium, 10
  px padding), no fill; hover lifts 8 %. The **remote chip** (a phone is
  connected) is the same pill in `accent`.
- **Buttons**: 28 px, radius 4, Inter 12.5 medium. Primary `accent` fill
  with `bg` text; secondary 6 % white with a 12 % hairline; ghost text only;
  destructive `error` darkened 18 % with white text; disabled 3 % white with
  an 8 % hairline and a `text_muted` label. Icon buttons are 24–26
  px with a 8 % hover lift. Key caps (`⏎`, `esc`) sit inside buttons that
  have a key.
- **Selects** (Settings): 220 × 28 px field, value and a chevron; a click
  opens a list under it (above when there is no room): 28 px rows, a check
  on the current value, theme rows with their ground ringed in their accent,
  up to 12 rows with a `n–m of N` note. ↑ ↓ ⏎ Esc work in the list; a click
  outside closes it. Selects never cycle on click.
- **Segmented controls, steppers, toggles** (Settings): 28 px bordered
  groups (radius 4), the selected segment on 11 % white in `text_bright`;
  steppers are − value + with hairline dividers; toggles 34 × 20, knob 14.
- **Modals** (palette and its modes): `surface` ground, hairline border,
  soft shadow, 40 % black backdrop, radius 8. Rows show a fuzzy match
  highlight and right-aligned key caps resolved from the live keymap.
- **Confirm dialogs**: 460 px `surface` card, radius 12, 45 % black
  backdrop; title (Inter 16 semibold), description and note; the affected
  lines in an inset list (`bg` at 60 %, radius 8) — running processes as
  36 px rows with the program's badge (an agent's brand disc, else a
  terminal mark) and the agent's name, other lines as mono code; a sunken
  footer bar under a hairline with Cancel `esc` and the confirm button `⏎`
  (30 px, radius 6; red for discarding work).
- **Docked panels** (completion, history): opaque `bg`, pane width, square,
  a hairline on the edge toward the output. Header 32 px (source tabs or
  scope chips in small caps), rows 28 px with a 7 % selection (history adds a
  2 px `accent` edge), a key-hint footer of caps and labels.
- **Context menus**: 240 px `surface`, radius 4, 28 px rows: icon column,
  label, the shortcut in `text_muted` at the right; rows that close
  something are `error`; separators are hairlines.
- **Tooltips**: 24 px `surface` card, radius 4, hairline edge and a soft
  shadow like a context menu; 12 px Inter `text_bright`, then the shortcut
  (or a path) in `text_muted`; 8 px side padding, at most 420 px. Centred
  6 px under the control, above it at the window bottom, 8 px inside the
  window edges.
- **Chips and pills**: radius 4 (badges keep their pill), muted text, used
  for shortcuts, counts (`3`, `All tabs (N)`), and modes (`NORMAL`).
- **Notifications popover**: `surface` card (radius 8, hairline border,
  soft shadow) right-aligned under the inbox button, no backdrop. Kind colors
  carry meaning: finished `success`, failed `error`, waiting on you
  `warning`, program notification `accent`, bell `text_dim`. Kind badges are
  those colors at 16 % with the mark in full color; an agent badge is its
  24 px brand disc (Claude `#d97757` with the white Claude mark, Codex
  `#f2f2f2`, OpenCode `#3b3b40`, Gemini `#4285f4`) with an 8 px kind dot
  ringed in `surface`. Sizes follow Warp's inbox: header medium 13.5, chips
  and the meta line (agent, `⎇ branch`, age) 12, titles `text_bright`
  semibold 13 on 18 px lines, details `text_dim` 12.5. Unread is a 6 px
  `accent` dot.
- **Toasts** (status): 30 px pills (radius 15, `surface`, 12 % hairline;
  a failure's edge `error` at 45 %, 10 px soft shadow) right-aligned in the
  pane's command-line chip row, 14 px from its right edge, stacking up 6 px
  apart. An 18 px disc of the kind's color at 14 % holds its 11 px icon
  (`success` check, `error` `!`, `accent` info or phone), then the text
  (Inter 12.5, `text_ui`, elided), an `accent` medium button, and ✕
  (`text_muted`) on a failure.
- **Notification toasts**: 360 px `surface` cards (radius 8) under the
  inbox button, 8 px apart, no stripe (the badge's kind dot carries the
  kind); badge and text centred on the card's height, title semibold 13
  (two lines), detail 12.5. A notification without an agent gets a bell
  badge (Nerd Font `fa-bell`; a terminal bell `fa-bell_o`). Tab markers: a 7 px kind
  dot with a 12 px halo at 25 %.
- **Split panes** (Warp style): flush, square-cornered panes divided by a
  1 px `pane_border_inactive` line; the focused pane's title bar carries a
  2 px `accent` top edge; the others wash lighter under 10 % white
  (`INACTIVE_LIFT`, as Warp's inactive panes; 5 % black on a light theme),
  so the focused pane is the dark one. Each title bar is the pane ground shaded 8 % black: a
  terminal icon (`accent` when focused), the title centered in Inter 12
  (`text_bright` when focused, `text_dim` otherwise), and split, ⋮ and × at
  the right while the pane is focused or hovered. While a pane is dragged by
  its bar, it fades under 60 % `bg`, a 220 × 34 `surface` chip (grip, icon,
  title) follows the pointer, and the drop zone (half of the target pane,
  its whole rect for a swap, a tab, or a tab-sized slot at `+`) shows a 16 %
  `accent` fill with a 1.5 px `accent` border and an `accent` pill naming
  the drop (`Split right`, `Swap`, `New tab`); the pointer is a grabbing
  hand.
- **File viewer header** (Warp's file pane): the same shaded bar with a file
  icon, the path centred in Inter 12, then a segmented **Rendered | Raw**
  switch (Markdown only; 22 px tall, radius 4, 10 % white hairline; the
  selected segment raised on 11 % white in `text_bright`, the other
  `text_dim`), ⋮ and ×.
- **Welcome list** (a fresh session): the 40 px app icon (`welcome::logo`,
  the *App icon* geometry), *New session* in Inter 17 semibold over
  `shell · cwd · branch`, then five 36 px rows (icon tile, name,
  `text_dim` description, key caps); hover lifts a row 5 %. It sits on a
  94 % `bg` backing (radius 8) so it stays readable over a background
  image or a translucent window.
- **Settings tab**: a 232 px `bg_chrome` sidebar (search field, page rows
  with icons, *Open config.toml* at the foot) and a body at most 720 px wide, centred in the room right of the sidebar:
  title Inter 22 semibold, subtitle `text_dim`, group labels in small caps,
  flat 52 px rows divided by 6 % hairlines, label left (description
  `text_muted` under it), control right. A setting changed from its default
  shows a 6 px `accent` dot after its label and ↺ left of its control.
- **Rendered Markdown**: 28 px side padding; body Inter 13.5/22 in `fg`;
  H1 24, H2 19 (both over a `border` rule), H3 16, H4–H6 14, semibold
  `text_bright`; bold `text_bright` semibold; links `accent`; inline code
  in the mono font on 8 % white (radius 4); code blocks mono 12.5/19 on
  4.5 % white (radius 4); quotes `text_dim` beside a 3 px `border` bar;
  tables bordered in `border`, header row semibold on 4.5 % white.
- **Keyboard focus** (Settings controls): a 2 px `accent` ring, radius 6,
  2 px outside the control; no fill.
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
| macOS | Finder, Dock, app switcher (`Rift.app`) | `rift.icns` |
| macOS | Dock for an unbundled `cargo run` | `rift-window.png` (embedded, `icon.rs`) |
| Linux | Launcher, dock, taskbar (via `rift.desktop`, app id `rift`) | `hicolor/*/apps/rift.png` |
| Linux X11, Windows | Window title bar, taskbar, Alt-Tab | `rift-window.png` (embedded window icon) |
| Windows | Executable file | `rift.ico` (reserved: Windows is not shipped yet) |
| iOS | Home screen (companion app) | `mobile/assets/icon.png`: opaque `surface` square, the tile's area of the grid filling it (iOS masks the corners) |
| Android | Launcher (companion app) | `mobile/assets/adaptive-icon.png` foreground (the tile's area in the central 62 %) on a `surface` background |

Design explorations: the Rift App Icon canvas (Claude Design artifact).

## Verifying visual work

Rendering has no unit tests. Drive the change with the script driver and
compare the `shot` PNG against the design screen (see CLAUDE.md, "Verifying
UI changes"). Check every builtin theme and a split layout, and check the
`dump` accessibility tree names the new element.
