# Keyboard Shortcuts

Default layout follows Warp's macOS defaults. Every binding below is an
*action name* that can be re-bound in `[keybindings]` (see
[configuration.md](configuration.md)); overriding an action also unbinds its
listed alternates.

## Blocks

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘↑ | `select_block_prev` | Select previous block (from newest; auto-scrolls) |
| ⌘↓ | `select_block_next` | Select next block; past the newest → deselect + focus input |
| ⇧⌘C | `copy_block_command` | Copy selected (else last) block's command |
| ⌥⇧⌘C | `copy_block_output` | Copy selected (else last) block's output |
| ⌘I | `reinput` | Insert block's command into the editor (at a prompt) |
| ⌃M | `block_menu` | Open context menu on selected/last block (passes through as CR in raw mode) |
| ⌘B / ⌃⇧B | `bookmark_toggle` | Bookmark selected (else last finished) block |
| ⌘⇧↑ / ⌃⇧↑ | `bookmark_prev` | Jump to previous bookmarked block |
| ⌘⇧↓ / ⌃⇧↓ | `bookmark_next` | Jump to next bookmarked block |
| ⌥↑ / ⌥↓ | `bookmark_closest_prev` / `_next` | Same jumps (Warp's closest-bookmark keys) |
| ⌘K | `clear_blocks` | Clear blocks + scrollback (destructive; `clear` is the soft variant) |
| ⌥⇧F | `filter_block` | Show only the selected (else newest) block's output lines that contain the typed text; Enter keeps the filter, Esc or ⌥⇧F again clears it |
| — | `pipe_block_output` | Send the selected (else newest) block's output to a command's stdin |
| Esc | — | Deselect block (in raw mode also still reaches the app in the PTY) |

## Scrolling

| Shortcut | Action name | Effect |
|---|---|---|
| PageUp / PageDown | `scroll_page_up` / `_down` | Page (editor active only; passes to vim/less otherwise) |
| Home / End | `scroll_top` / `scroll_bottom` | Jump to scrollback top / bottom (same gating) |
| ⇧PageUp / ⇧PageDown | `scroll_up` / `scroll_down` | Scroll 10 lines |

## Tabs

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘T / ⌃⇧T | `new_tab` | New tab |
| ⌘1 … ⌘9 | `tab_select_1..9` | Select tab by number (⌘9 = last) |
| ⇧⌘} / ⌃Tab | `tab_next` | Next tab |
| ⇧⌘{ / ⌃⇧Tab | `tab_prev` | Previous tab |
| ⌃⇧← / ⌃⇧→ | `move_tab_left` / `_right` | Reorder current tab |
| ⇧⌘T | `reopen_closed_tab` | Reopen last closed tab (restores cwd) |
| ⇧⌘O | `session_switcher` | Fuzzy switcher over tabs, panes and recent blocks |
| — | `toggle_tab_layout` | Tabs on top ↔ vertical rail |
| — | `rename_tab` | Rename the current tab |
| — | `close_other_tabs` | Close every tab except the current one |
| — | `save_launch_config` | Save the current tabs and panes as a launch configuration |

## Panes

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘D / ⌃⇧D | `split_vertical` | Split right |
| ⇧⌘D / ⌃⇧E | `split_horizontal` | Split down |
| ⌥⌘← → ↑ ↓ | `navigate_left/right/up/down` | Directional pane focus (unzooms first) |
| ⌃⌘← → ↑ ↓ | `resize_pane_*` | Move the split divider (±5 %) |
| ⇧⌘⏎ | `maximize_pane` | Toggle pane zoom |
| ⌘W / ⌃⇧W | `close_pane` | Close pane (last pane closes tab) |
| ⌥⌘I | `toggle_sync_inputs` | Type and submit into every pane of the tab at once |

## Window

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘N | `new_window` | New window |
| ⇧⌘W | `close_window` | Close the window (warns about running commands) |
| ⌘M | `minimize` | Minimize |
| ⌘H | `hide_app` | Hide Rift |
| ⌃⌘F | `toggle_fullscreen` | Full screen |
| ⌘Q | `quit` | Quit (warns about running commands) |
| ⌘A | `select_all` | Select the editor text, else all scrollback |

## App

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘P / ⌃⇧P | `palette` | Command palette |
| ⌘F / ⌃⇧F | `search` | Scrollback search |
| ⌘, | `open_settings` | Settings tab (again: back to the shell tab) |
| ⌃⌘K | `open_keybindings` | Settings → Keybindings page |
| ⌘L | `focus_input` | Scroll to bottom, deselect, focus the editor |
| ⇧⌘A | `attention_center` | Notifications popover: finished commands, failures, notifications and bells |
| ⇧⌘E | `toggle_explorer` | File explorer panel (left) |
| ⇧⌘G | `toggle_review` | Code review panel (right) |
| ⇧⌘H | `hint_mode` | Letter hints over visible links, paths and blocks: type one to open/select, ⇧ + letter copies |
| — | `clear_transcripts` | Delete saved block transcripts |
| — | `check_for_updates` | Look for a new release now; the answer toasts (also Rift → Check for Updates…) |
| — | `update_rift` | The update dialog for an available release (else a check) |
| — | `open_about` | Settings → About page: version, copy button, copyright (also Rift → About Rift) |
| — | `whats_new` | What's New: release notes (also Help → What's New) |
| ⌘= / ⇧⌘+ | `font_increase` | Font size +1 (persisted to config.toml) |
| ⌘- | `font_decrease` | Font size −1 |
| ⌘0 | `font_reset` | Font size → default (13) |
| ⌘C / ⌃⇧C | `copy` | Copy selection, else last block's output |
| ⌘V / ⌃⇧V | `paste` | Paste |

## Editor keys (fixed, not remappable)

Active only while the app owns the command line.

| Key | Effect |
|---|---|
| Enter | Submit command |
| ⇧⏎ / ⌥⏎ | Insert a newline (multi-line command; Enter still submits) |
| Tab | Accept completion |
| ⇧Tab | Next completion source tab (All · Path · History …) while the panel is open |
| Esc | Close completion (after deselecting any block) |
| ↑ / ↓ | History up / down (or move completion selection while the completion panel is open) |
| ← / → | Move cursor; → at the end of the line accepts the ghost text, ⌃→ one word of it |
| ⌥← / ⌥→ | Word left / right |
| ⌘← / ⌘→, ⌃A / ⌃E | Line start / end |
| ⌫ / ⌦ (Delete) | Delete back / forward |
| ⌥⌫ / ⌥⌦ | Delete word back / forward |
| ⌘⌫ / ⌘⌦ | Kill to line start / end |
| ⌃W / ⌃K | Cut word left / cut to end (emacs) |
| ⌃B / ⌃F | Cursor left / right (emacs) |
| ⌃U | Clear line |
| ⌘Z / ⇧⌘Z | Undo / redo |
| ⇧← / ⇧→ | Extend the selection by one character |
| ⌥⇧← / ⌥⇧→ | Extend the selection by one word |
| ⇧⌘← / ⇧⌘→, ⇧Home / ⇧End | Extend the selection to line start / end |
| ⌘X | Cut the selection (⌘C copies it, ⌘A selects all) |
| ⌃R | History pane above the command line (again: next scope); in it ↑/↓ move (PageUp/PageDown by 8), ⏎ runs, ⇥ edits first, ⌘⏎ runs in a new split, Esc closes; with no match ⏎ runs the typed line and ⇥ only closes the pane |
| ⌃C | Interrupt (clears line, sends SIGINT) |
| ⌃D | EOF (only when the line is empty) |
| ⌃L | Clear screen (shell redraw; blocks survive) |

## Vim mode

Active in the owned editor when `vim_mode = true` (Settings → Input, or the
palette's *Toggle vim keybindings*). The buffer is one command: motions are
within-line and "linewise" operators act on the whole buffer. Only the keys
below are implemented.

| Key | Effect |
|---|---|
| Esc | Insert → Normal (cursor steps left); in Normal cancels a pending operator/count |
| `i` / `a` / `I` / `A` | Insert here / after cursor / at first non-blank / at end |
| Enter | Submit the command (either mode); submitting returns to Insert |
| `h` `l` (or Space) | Left / right (also ← → and ⌫) |
| `0` / `$` / `^` | Line start / last char / first non-blank |
| `w` `b` `e` | Word forward / back / end (`W` `B` `E` = whitespace-delimited) |
| `f` `F` `t` `T` + char | Find forward/back, on/before the char |
| `;` | Repeat the last `f` / `F` / `t` / `T` |
| `d` `c` `y` + motion | Delete / change / yank over the motion (`cw` acts like `ce`) |
| `dd` / `cc` / `yy` / `S` | Same operators over the whole buffer |
| `D` / `C` | Delete / change to end of line |
| `x` / `X` / `s` | Delete under / before cursor / substitute |
| `r` + char | Replace the character under the cursor |
| `~` | Toggle case and advance |
| `p` / `P` | Put the unnamed register after / before the cursor |
| `u` / **`U`** | Undo / **redo** (⌃R stays the history pane) |
| `k` / `j` | History previous / next (also ↑ / ↓) |
| digits | Count, before and/or after the operator — the two multiply (`2d3w`) |

## Search-mode keys

| Key | Effect |
|---|---|
| type / ⌫ | Edit query (live highlight) |
| Enter / ⌘G | Next match |
| ⇧Enter / ⇧⌘G | Previous match |
| ⌥C | Toggle match case (`Aa`) |
| ⌥R | Toggle regular expression (`.*`) |
| Esc, or ⌘F again | Close search, snap back to bottom |

With the file viewer open, ⌘F finds in the file instead: type the query
(case-insensitive), Enter / ⇧Enter next / previous match, Esc closes the bar.

## Mouse

| Gesture | Effect |
|---|---|
| drag / double-click / triple-click | Select characters / word / line |
| ⌥-drag | Rectangular (column) selection |
| ⇧-click/drag | Local selection even when the program tracks the mouse |
| ⌘-click | Open a URL, OSC 8 hyperlink or existing file path |
| wheel | Scroll the pane under the pointer (reported to mouse-tracking programs) |
| drop files | Paste their shell-quoted paths |
| drag a tab | Reorder tabs (Esc while dragging puts it back) |
| drag a pane title bar | Move the pane: a pane's half splits, its middle swaps, a tab joins (hold 600 ms to open it), the empty strip makes a new tab; Esc cancels |

In the Settings tab, typing outside Keybindings searches every page (⌫
edits, Esc clears, a second Esc closes the tab); ⌘W or ⌘, leave it.

## Raw-mode keys

When a program owns the terminal (vim, less, htop), keys go out as xterm
sequences: F1–F12, modified arrows/Home/End/PgUp/PgDn/Insert/Delete
(`\e[1;5D` for ⌃←), application cursor mode (`\eOA`), ⌃ control codes, and
ESC-prefixed Meta for ⌥ (always on Linux; on macOS with `option_as_meta` for the left key and `right_option_as_meta` for the right one).
⌘⌫ sends ⌃U (kill to line start, as in Warp; Claude Code, readline and zle
honour it), in the program's kitty keyboard encoding when it asked for one;
other ⌘ chords stay with Rift.

## Palette keys

Esc (or the palette shortcut again) close · Enter run selected · ↑/↓ move selection · type to filter · ⇥ next mode · `>` `#` `@` typed into an empty query switch to Actions, History, Sessions. The block action *Send output to command…* opens a fourth mode with no prefix, where ⇥ and the prefixes do nothing.

## Settings keys

Typing searches every page (Keybindings filters its own list) · ⇥ / ⇧⇥ move focus between controls · Space or Enter presses the focused control · ← / → adjust a focused slider · ↑ ↓ ← → switch pages (except ← / → on a focused slider) · Home / End go to the first / last page · PageUp / PageDown scroll · Esc drops control focus, then clears the search, then closes the tab.

## Customization

`~/.config/rift/config.toml`:

```toml
[keybindings]
new_tab   = "cmd+n"          # replaces BOTH defaults for the action
palette   = "cmd+shift+space"
font_increase = "cmd+plus"   # "plus" = the '+' key (can't be written literally)
```

Chord syntax: `ctrl|control|^`, `shift|⇧`, `alt|opt|option|meta`,
`cmd|super|command|⌘` joined with `+` plus one key: a character, or
`tab left right up down pageup pagedown home end enter escape space plus
backspace delete insert f1`–`f12`. `"none"` unbinds an action. A chord with any other key name (`cmd+foo`) is
invalid, and an unknown action name is ignored; both are logged.

Or record shortcuts in Settings → Keybindings (⌃⌘K): click a row, press the
chord. A recorded chord needs ⌘, ⌃ or ⌥, or a function or navigation key; a
bare letter is refused with a toast. The macOS menu bar shows the resolved shortcuts. Where an action has
two (a ⌃⇧ chord and a ⌘ one), the palette, Settings, menus, the welcome list
and tooltips show the ⌘ one on macOS and the other elsewhere.
