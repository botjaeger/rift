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
| ⌥↑ / ⌥↓ | `bookmark_closest_prev` / `_next` | Same jumps (Warp's closest-bookmark keys; editor active only, passes to the program otherwise — Codex reads them) |
| ⌘K | `clear_blocks` | Clear blocks + scrollback (destructive; `clear` is the soft variant) |
| ⌥⇧F | `filter_block` | Show only the selected (else newest) block's output lines that contain the typed text; Enter keeps the filter, Esc or ⌥⇧F again clears it (editor active only; passes to a running program) |
| — | `pipe_block_output` | Send the selected (else newest) block's output to a command's stdin |
| Esc | — | Deselect block (in raw mode also still reaches the app in the PTY) |

## Scrolling

| Shortcut | Action name | Effect |
|---|---|---|
| PageUp / PageDown | `scroll_page_up` / `_down` | Page (editor active only; passes to vim/less otherwise) |
| Home / End | `scroll_top` / `scroll_bottom` | Jump to scrollback top / bottom (same gating; they never move the command line's cursor, ⌘← / ⌘→ do) |
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
| — | `reset_tab_timer` | Zero the tab's "Time on this tab" chip (the card's Reset button) |
| — | `save_launch_config` | Save the current tabs and panes as a launch configuration |

## Panes

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘D / ⌃⇧D | `split_vertical` | Split right |
| ⇧⌘D / ⌃⇧E | `split_horizontal` | Split down |
| ⌥⌘← → ↑ ↓ | `navigate_left/right/up/down` | Directional pane focus (unzooms first) |
| ⌃⌘← → ↑ ↓ | `resize_pane_*` | Move the divider beside the pane 10 px that way (no pane under 50 px) |
| — | `equalize_panes` | Give every pane of the tab an equal share (double-click a divider: that row only) |
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
| Enter | Submit command (an empty or blank line does nothing) |
| ⇧⏎ / ⌥⏎ | Insert a newline (multi-line command; Enter still submits) |
| Tab | Accept completion |
| ⇧Tab | Next completion source tab (All · Path · History …) while the panel is open |
| Esc | Close completion (after deselecting any block) |
| ↑ / ↓ | History up / down (or move completion selection while the completion panel is open) |
| ← / → | Move cursor; → at the end of the line accepts the ghost text, ⌃→ one word of it |
| ⌥← / ⌥→ | Word left / right |
| ⌘← / ⌘→, ⌃A / ⌃E | Start / end of the cursor's line (of the command, when it has one line) |
| ⌘↑ / ⌘↓ | Start / end of a multi-line command while there are lines that way; on its first / last line they select blocks (`select_block_prev/next`) |
| ⇧⌘↑ / ⇧⌘↓ | Extend the selection to the start / end of a multi-line command; on a one-line command, or its first / last line, they are `bookmark_prev_alt` / `bookmark_next_alt` (previous / next bookmarked block) |
| ⌫ / ⌦ (Delete) | Delete back / forward |
| ⌥⌫ / ⌥⌦ | Delete word back / forward |
| ⌘⌫ / ⌘⌦ | Kill to the start / end of the cursor's line; at the line's start / end they join it to the line above / below |
| ⌃W / ⌃K | Cut word left / cut to the end of the cursor's line, at its end the newline (emacs) |
| ⌃B / ⌃F | Cursor left / right (emacs) |
| ⌃U | Clear the whole command |
| ⌘Z / ⇧⌘Z | Undo / redo |
| ⇧← / ⇧→ | Extend the selection by one character |
| ⌥⇧← / ⌥⇧→ | Extend the selection by one word |
| ⇧⌘← / ⇧⌘→, ⇧Home / ⇧End | Extend the selection to line start / end |
| ⌘X | Cut the selection (⌘C copies it, ⌘A selects all) |
| ⌃R | History pane above the command line (again: next scope); in it ↑/↓ move (PageUp/PageDown by 8), ⏎ runs, ⇥ edits first, ⌘⏎ runs in a new split, Esc closes; with no match ⏎ runs the typed line and ⇥ only closes the pane |
| ⌃C | Interrupt (clears line, sends SIGINT) |
| ⌃D | EOF (only when the line is empty) |
| ⌃L | Clear screen (shell redraw; blocks survive) |

A held key repeats at the system's key-repeat rate: every editing and cursor
key above, and the stepping shortcuts (scroll, tab and pane focus, divider
moves, block and bookmark selection, font size). One-shot actions (new tab,
split, close, palette…) run once per press, so holding them does not open a
row of tabs.

## Vim mode

Active in the owned editor when `vim_mode = true` (Settings → Input, or the
palette's *Toggle vim keybindings*). The buffer is one command; when it holds
several lines (⇧Enter), the line keys below work on lines and other motions
run across them, while `dd` `cc` `yy` act on the whole buffer. Only the keys
below are implemented.

| Key | Effect |
|---|---|
| Esc | Insert / Replace → Normal (cursor steps left); leaves Visual; in Normal cancels a pending operator/count |
| `i` / `a` / `I` / `A` | Insert here / after cursor / at first non-blank / at line end |
| `o` / `O` | Open a line below / above and insert |
| `v` / `V` | Visual (charwise) / Visual Line mode; again to leave |
| `R` | Replace mode: typed chars overwrite, ⌫ restores |
| Enter | Submit the command (any mode); submitting returns to Insert |
| `h` `l` (or Space) | Left / right (also ← → and ⌫) |
| `0` / `$` / `^` / `_` | Line start / last char / first non-blank / first non-blank (linewise for operators) |
| `w` `b` `e` `ge` | Word forward / back / end / previous end (`W` `B` `E` `gE` = whitespace-delimited) |
| `gg` / `G` | First / last line (`3G` = line 3) |
| `%` | Matching bracket |
| `f` `F` `t` `T` + char | Find forward/back, on/before the char |
| `;` / `,` | Repeat the last `f` / `F` / `t` / `T` / in reverse |
| `/` / `?` + text, Enter | Search forward / backward (literal, wrapping); Esc cancels |
| `n` / `N` | Next match in the same / opposite direction |
| `d` `c` `y` + motion | Delete / change / yank over the motion (`cw` acts like `ce`) |
| `gu` `gU` `g~` + motion | Lowercase / uppercase / toggle case |
| `>` `<` + motion | Indent (two spaces) / outdent (a tab or two spaces) the lines |
| operator + `i`/`a` + object | Text objects: `w` `W` words, `"` `'` `` ` `` quotes, `(` `b` `[` `{` `B` `<` brackets (`ciw`, `da"`, `yi(`) |
| `dd` / `cc` / `yy` / `S` | Same operators over the whole buffer |
| `>>` `<<` `guu` `gUU` `g~~` | Same operators over the cursor's line (a count takes more) |
| `D` / `C` | Delete / change to end of line |
| `x` / `X` / `s` | Delete under / before cursor / substitute |
| `r` + char | Replace the character under the cursor |
| `~` | Toggle case and advance |
| `J` | Join the next line (a count joins more) |
| `p` / `P` | Put the unnamed register after / before the cursor |
| `.` | Repeat the last change (a count replaces its count) |
| `u` / **`U`** | Undo / **redo** one change (⌃R stays the history pane) |
| `k` / `j` | History previous / next (also ↑ / ↓) |
| digits | Count, before and/or after the operator — the two multiply (`2d3w`) |

In Visual / Visual Line mode motions, searches and `j` `k` (↑ ↓) extend the
selection, `iw` `a"` … select a text object, and:

| Key | Effect |
|---|---|
| `d` `x` / `c` `s` / `y` | Delete / change / yank the selection |
| `D` `X` / `C` `S` `R` / `Y` | The same over whole lines |
| `~` / `u` / `U` (or `g~` `gu` `gU`) | Toggle / lower / upper case |
| `>` / `<` | Indent / outdent the selected lines |
| `J` | Join the selected lines |
| `o` | Jump to the other end of the selection |
| `v` / `V` | Switch between charwise and linewise, or leave |

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
