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
| ⇧⌘C | `copy_block_command` | Copy selected (else last) block's command; every selected command with a multi-block selection |
| ⌥⇧⌘C | `copy_block_output` | Copy selected (else last) block's output; every selected output with a multi-block selection |
| ⌘I | `reinput` | Insert block's command into the editor (at a prompt) |
| ⇧⌘I / ⌃⇧I | `inspect_command` | Command inspector: what the part of the command line at the cursor is and does (again, or any other key, puts it away) |
| ⌃M | `block_menu` | Open context menu on selected/last block (passes through as CR in raw mode) |
| ⌘B / ⌃⇧B | `bookmark_toggle` | Bookmark selected (else last finished) block |
| ⌘⇧↑ / ⌥⇧↑ | `bookmark_prev` | Jump to previous bookmarked block |
| ⌘⇧↓ / ⌥⇧↓ | `bookmark_next` | Jump to next bookmarked block |
| ⌃⇧↑ / ⌃⇧↓ | `add_cursor_above` / `add_cursor_below` | Multi-cursor: a caret on the line above / below (Warp; the same on every platform) |
| ⌃G | `add_next_occurrence` | Multi-cursor: select the word at the cursor, then add a caret at the selection's next occurrence |
| ⌥↑ / ⌥↓ | `bookmark_closest_prev` / `_next` | Same jumps (Warp's closest-bookmark keys; editor active only, passes to the program otherwise — Codex reads them) |
| ⌘K | `clear_blocks` | Clear blocks + scrollback (destructive; `clear` is the soft variant) |
| ⌥⇧F | `filter_block` | Show only the selected (else newest) block's output lines that match the typed text; Enter keeps the filter, Esc clears it, ⌥⇧F again turns it off (kept; ⌥⇧F once more turns it back on). In the filter bar: ⌥C match case, ⌥R regex, ⌥I invert, ⌥↑ / ⌥↓ context lines (editor active only; passes to a running program) |
| — | `pipe_block_output` | Send the selected (else newest) block's output to a command's stdin |
| Esc | — | Deselect block(s) (in raw mode also still reaches the app in the PTY) |
| ⌘-click / ⇧-click | — | Add or remove a block in the selection / select the range from the selected block |
| ⇧↑ / ⇧↓ | — | With a block selected: grow or shrink the block selection |
| ⌘A | `select_all` | With a block selected: select every block; else the editor's text (when it has any), else all scrollback |

## Scrolling

| Shortcut | Action name | Effect |
|---|---|---|
| PageUp / PageDown | `scroll_page_up` / `_down` | Page (editor active only; passes to vim/less otherwise) |
| Home / End | `scroll_top` / `scroll_bottom` | Jump to scrollback top / bottom (same gating; they never move the command line's cursor, ⌘← / ⌘→ do); End at the end of the command line also accepts the ghost text |
| ⇧PageUp / ⇧PageDown | `scroll_up` / `scroll_down` | Scroll 10 lines |

## Tabs

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘T / ⌃⇧T | `new_tab` | New tab |
| ⌘1 … ⌘9 | `tab_select_1..9` | Select tab by number (⌘9 = last). Hold ⌘ alone for 300 ms and every tab shows its number (its bound chord) in the ×'s place; any key or releasing ⌘ hides them |
| ⌃⇧1 … ⌃⇧9 | `new_tab_profile_1..9` | New tab with the nth profile of the + menu ([profiles](configuration.md#profiles)) |
| ⇧⌘} / ⌃Tab | `tab_next` | Next tab (`tab_switch_order = "mru"`: ⌃Tab opens the recent-tabs switcher, see below) |
| ⇧⌘{ / ⌃⇧Tab | `tab_prev` | Previous tab |
| ⌃⇧← / ⌃⇧→ | `move_tab_left` / `_right` | Reorder current tab |
| ⇧⌘T | `reopen_closed_tab` | Reopen last closed tab (restores cwd) |
| ⇧⌘O | `session_switcher` | Fuzzy switcher over tabs, panes and recent blocks |
| — | `new_worktree_tab` | Palette *New worktree tab…*: name a branch, add its git worktree, open a tab there |
| — | `toggle_tab_layout` | Tabs on top ↔ vertical rail |
| — | `rename_tab` | Rename the current tab |
| — | `pin_tab` | Pin or unpin the current tab (pinned tabs sit first; ⌘W asks first) |
| — | `cycle_tab_color` | The current tab's colour: none → red … cyan → none |
| — | `set_tab_color` | Ask for the current tab's colour: a name or any `#rrggbb` |
| ⇧⌘F | `focus_rail_search` | Search the vertical rail's tabs (without the rail: the session switcher) |
| — | `toggle_rail_panes` | Vertical rail: a row per pane of split tabs, or per tab |
| — | `close_other_tabs` | Close every tab except the current one |
| — | `reset_tab_timer` | Zero the tab's "Time on this tab" chip (the card's Reset button) |
| — | `save_launch_config` | Save the current tabs and panes as a launch configuration |

With `tab_switch_order = "mru"` the ⌃ chords of `tab_next` / `tab_prev`
follow the tabs last used first: ⌃Tab with ⌃ held opens the *Recent tabs*
switcher on the previous tab, more ⌃Tab / ⌃⇧Tab (or ↓ / ↑) move the highlight,
releasing ⌃ (or ⏎) switches, Esc cancels. A quick ⌃Tab toggles between the
two most recent tabs. It lists the tabs you can see (none folded in a
collapsed group); a tab that closes while it shows drops out of it. ⇧⌘} /
⇧⌘{ keep the strip's order.

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
| — | `toggle_read_only` | Make the pane read-only (input refused) or let input in again |
| — | `restart_pane` | Start the pane's program again in place (asks first over a running job) |
| — | `toggle_sticky_header` | Minimize the pane's sticky command header to a small tab, or show it again |

## Window

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘N | `new_window` | New window |
| ⇧⌘W | `close_window` | Close the window (warns about running commands) |
| ⌘M | `minimize` | Minimize |
| ⌘H | `hide_app` | Hide Rift |
| ⌃⌘F | `toggle_fullscreen` | Full screen |
| — | `toggle_always_on_top` | The window floats above other apps' windows, or stops (`always_on_top` sets new windows) |
| — | `rename_window` | Name the window (palette; nothing typed clears it): its title, saved with the session |
| ⌘Q | `quit` | Quit (warns about running commands) |

## App

| Shortcut | Action name | Effect |
|---|---|---|
| ⌘P / ⌃⇧P | `palette` | Command palette |
| ⌘F / ⌃⇧F | `search` | Scrollback search |
| ⌘, | `open_settings` | Settings tab (again: back to the shell tab) |
| — | `theme_picker` | Theme picker: the selected theme previews live; ⏎ keeps it, Esc reverts |
| ⌃⌘K | `open_keybindings` | Settings → Keybindings page |
| ⌘L | `focus_input` | Scroll to bottom, deselect, focus the editor |
| ⇧⌘A | `attention_center` | Notifications popover: finished commands, failures, notifications and bells |
| ⌘E / ⌃⇧G | `rich_input` | Rich input: open (or close) the composer over the running program; ⏎ sends, ⇧⏎ new line, Esc closes keeping the draft (Warp's ⌃G is `add_next_occurrence` here) |
| ⇧⌘E | `toggle_explorer` | File explorer panel (left) |
| ⇧⌘G | `toggle_review` | Code review panel (right) |
| ⌥⌘G | `focus_review` | Open the code review panel if needed and give it the keyboard (see Code review keys) |
| ⇧⌘H | `hint_mode` | Letter hints over visible links, paths and blocks: type one to open/select, ⇧ + letter copies |
| — | `clear_transcripts` | Delete saved block transcripts |
| — | `check_for_updates` | Look for a new release now; the answer toasts (also Rift → Check for Updates…) |
| — | `update_rift` | The update dialog for an available release (else a check) |
| — | `install_cli` | Install 'rift' command: `~/.local/bin/rift` → this Rift, so `rift .` opens a folder; refuses to replace a file that is not Rift's, and says when the folder is not on `PATH` |
| — | `open_about` | Settings → About page: version, copy button, copyright (also Rift → About Rift) |
| — | `whats_new` | What's New: release notes (also Help → What's New) |
| ⌘= / ⇧⌘+ | `font_increase` | Font size +1 (persisted to config.toml) |
| ⌘- | `font_decrease` | Font size −1 |
| ⌘0 | `font_reset` | Font size → default (13) |
| — | `opacity_increase` / `opacity_decrease` | Window opacity ±5 % (persisted to config.toml) |
| — | `opacity_reset` | Window opacity → default (50 %) |
| ⌘C / ⌃⇧C | `copy` | Copy selection, else last block's output |
| ⇧⌘M / ⌃⇧M | `mark_mode` | Mark mode: select text in the scrollback from the keyboard (see Mark mode keys) |
| ⌥. | `insert_last_argument` | At the prompt: insert the last word of the previous command; again: of the one before (⌥ as Meta: always on Linux, `option_as_meta` on macOS; otherwise ⌥. types its character) |
| ⌃⌥← / ⌃⌥→ | `subword_left` / `subword_right` | At the prompt: move by word parts (`camelCase`, `snake_case`, `kebab-case`, path segments); ⌃⌥→ at the end of the line accepts one word part of the ghost text |
| — | `accept_autosuggestion` | At the prompt, at the end of the line: accept the whole ghost text (unbound; Warp's "Accept autosuggestion"); with no ghost text to take the key does what it does unbound. The ghost text's accept-key chip binds it to ⇧→ or opens this row (Custom…) |
| — | `clear_ignored_suggestions` | Suggest again every command line the ghost text's ✕ ignored |
| — | `[[actions]]` | Your own step sequences, by their `key` ([user actions](configuration.md#user-actions)) |
| ⌘V / ⌃⇧V | `paste` | Paste |

## Editor keys (fixed, not remappable)

Active only while the app owns the command line.

| Key | Effect |
|---|---|
| Enter | Submit command (an empty or blank line does nothing); with the completion menu open, accept its selected row (as you type: unless the word already is it, then submit) |
| ⇧⏎ / ⌥⏎ | Insert a newline (multi-line command; Enter still submits) |
| Tab | Complete the word (Warp): one match is inserted, several insert their common prefix and open the completion menu, else the menu opens; never takes the ghost text (`tab_behavior = "autosuggestion"`: takes it, and completes with none). In the open menu: the common prefix of the rows matching as typed when longer than the word, else the next row (wrapping) |
| ⇧Tab | Previous completion row (wrapping) while the menu is open |
| ⌃Space | Open the completions (the way to open them where `tab_behavior = "autosuggestion"`) |
| Esc | Close the completion menu and clear the ghost text (after deselecting any block; vim: the ghost text in Normal mode only) |
| ↑ / ↓ | History menu (Warp): ↑ on the first line opens it on the history lines that start with the typed text and previews the newest; ↑ older, ↓ newer; ↓ past the newest or Esc closes it and puts back the typed text; ⏎ runs the preview, ⇥ / → keep it to edit, typing keeps it and types. A completion panel opened as you type gives ↑ to the menu; one asked for (Tab, or ↓ into it) keeps ↑ / ↓ moving its selection. Inside a multi-line command: the line above / below |
| ← / → | Move cursor; → at the end of the line accepts the ghost text, ⌃→ one word of it (anywhere) |
| ⌥← / ⌥→ | Word left / right; ⌥→ at the end of the line accepts one word of the ghost text |
| ⌘← / ⌘→, ⌃A / ⌃E | Start / end of the cursor's line (of the command, when it has one line); ⌘→ / ⌃E at the end of the line accept the ghost text |
| ⌘↑ / ⌘↓ | Start / end of a multi-line command while there are lines that way; on its first / last line they select blocks (`select_block_prev/next`) |
| ⇧⌘↑ / ⇧⌘↓ | Extend the selection to the start / end of a multi-line command; on a one-line command, or its first / last line, they are `bookmark_prev` / `bookmark_next` (previous / next bookmarked block) |
| ⌫ / ⌦ (Delete) | Delete back / forward |
| ⌥⌫ / ⌥⌦ | Delete word back / forward |
| ⌘⌫ / ⌘⌦ | Kill to the start / end of the cursor's line; at the line's start / end they join it to the line above / below |
| ⌃W / ⌃K | Cut word left / cut to the end of the cursor's line, at its end the newline (emacs) |
| ⌃B / ⌃F | Cursor left / right (emacs) |
| ⌃U | Clear the whole command; the cleared text goes to the clipboard |
| ⌘Z / ⇧⌘Z | Undo / redo |
| ⇧← / ⇧→ | Extend the selection by one character |
| ⌥⇧← / ⌥⇧→ | Extend the selection by one word |
| ⇧⌘← / ⇧⌘→, ⇧Home / ⇧End | Extend the selection to line start / end |
| ⌘X | Cut the selection (⌘C copies it, ⌘A selects all) |
| ⌃R | History pane above the command line (again: next scope — this pane, all shells, failed, results); in it ↑/↓ move (PageUp/PageDown by 8), ⏎ runs, ⇥ edits first, ⌘⏎ runs in a new split, Esc or ↓ past the last row closes; with no match ⏎ runs the typed line and ⇥ only closes the pane. Where your shell binds ⌃R to its own widget (fzf, atuin), that widget runs on the command line instead (`shell_widgets`) |
| ⌃T / ⌥C | Where your shell binds them to its own widgets (fzf's file picker / directory jump): run them on the command line (`shell_widgets`; ⌥C with Option as Meta); otherwise nothing |
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
| Esc | Insert / Replace → Normal (cursor steps left; the ghost text stays); leaves Visual; in Normal cancels a pending operator/count and clears the ghost text |
| `i` / `a` / `I` / `A` | Insert here / after cursor / at first non-blank / at line end |
| `o` / `O` | Open a line below / above and insert |
| `v` / `V` | Visual (charwise) / Visual Line mode; again to leave |
| `R` | Replace mode: typed chars overwrite, ⌫ restores |
| Enter | Submit the command (any mode); submitting returns to Insert |
| `h` `l` (or Space) | Left / right (also ← → and ⌫); `l` on the last character accepts the ghost text |
| `0` / `$` / `^` / `_` | Line start / last char / first non-blank / first non-blank (linewise for operators); `$` on the last character accepts the ghost text |
| `w` `b` `e` `ge` | Word forward / back / end / previous end (`W` `B` `E` `gE` = whitespace-delimited); `w` / `e` on the last character accept one word of the ghost text |
| `gg` / `G` | First / last line (`3G` = line 3) |
| `%` | Matching bracket |
| `f` `F` `t` `T` + char | Find forward/back, on/before the char |
| `;` / `,` | Repeat the last `f` / `F` / `t` / `T` / in reverse |
| `/` / `?` + text, Enter | Search forward / backward (literal, wrapping); Esc cancels |
| `n` / `N` | Next match in the same / opposite direction |
| `*` / `#` | Search the word under the cursor forward / backward (whole word) |
| `d` `c` `y` + motion | Delete / change / yank over the motion (`cw` acts like `ce`) |
| `gu` `gU` `g~` + motion | Lowercase / uppercase / toggle case |
| `>` `<` + motion | Indent (two spaces) / outdent (a tab or two spaces) the lines |
| operator + `i`/`a` + object | Text objects: `w` `W` words, `"` `'` `` ` `` quotes, `(` `b` `[` `{` `B` `<` brackets, `l` the line (`il` without its edge blanks) — `ciw`, `da"`, `yi(`, `dil` |
| `dd` / `cc` / `yy` / `S` | Same operators over the whole buffer |
| `>>` `<<` `guu` `gUU` `g~~` | Same operators over the cursor's line (a count takes more) |
| `D` / `C` | Delete / change to end of line |
| `x` / `X` / `s` | Delete under / before cursor / substitute |
| `r` + char | Replace the character under the cursor |
| `~` | Toggle case and advance |
| `J` | Join the next line (a count joins more) |
| `p` / `P` | Put the register after / before the cursor |
| `"` + name | The register the next operator or put uses: `a`–`z` (`A`–`Z` append), `0` last yank, `_` black hole, `+` / `*` system clipboard (`"+yy`, `"ap`) |
| `.` | Repeat the last change (a count replaces its count) |
| `u` / ⌃R | Undo / redo one change (in Insert, ⌃R stays the history pane) |
| `U` | Undo the latest changes on the last changed line (a change itself: `U` again redoes them) |
| `k` / `j` | The command line's ↑ / ↓ (also ↑ / ↓): `k` opens the history menu and goes older, `j` newer; in a multi-line command, the line above / below |
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
| type / ⌫ | Edit query (live highlight); ⌫ removes a whole character (an emoji with its modifiers) |
| ⌘A | Select the query: typing replaces it, ⌫ clears it, ⌘C copies it |
| Enter / ⌘G | Next match |
| ⇧Enter / ⇧⌘G | Previous match |
| ⌥C | Toggle match case (`Aa`) |
| ⌥R | Toggle regular expression (`.*`) |
| Esc, or ⌘F again | Close search, snap back to bottom |

With the file viewer open, ⌘F finds in the file instead: type the query
(case-insensitive), Enter / ⇧Enter next / previous match, Esc closes the bar.

## File viewer and commit message keys

File viewer (focused): click selects a line, ⇧-click or ⇧↑ ⇧↓ extend the
selection · `a` sends the selected lines (else the top line on screen) to an
agent pane through the target picker · Esc clears the selection, then closes
the viewer.

Code review commit message (click the field at the panel's foot): typing and
⌘V go into the message · Enter adds a line · ⌘⏎ (or ⌃⏎) commits the staged
changes · Esc leaves the field. Other ⌘ chords stay app shortcuts.

Target picker ("Send to agent…"): type to filter · ↑ ↓ · Enter pastes into
the chosen pane (no Enter is sent after it) · Esc cancels.

## Mouse

| Gesture | Effect |
|---|---|
| drag / double-click / triple-click | Select characters / word / line |
| ⌥-drag | Rectangular (column) selection |
| ⇧-click/drag | Local selection even when the program tracks the mouse |
| ⌘-click | Open a URL, OSC 8 hyperlink or existing file path |
| wheel | Scroll the pane under the pointer (reported to mouse-tracking programs) |
| ⌘ + wheel / ⌘⇧ + wheel (Ctrl, Ctrl+⇧ on Linux) | With `wheel_zoom`: font size ±1 per notch / window opacity ±5 % (both saved) |
| drag the scrollbar thumb / click its track | Scroll there / page one view toward the click (`scrollbar`) |
| click a masked secret (`••••`) | Show it for 5 s (`mask_secrets`) |
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
⌘⌫ sends ⌃U (kill to line start), ⌘← ⌃A (line start) and ⌘→ ⌃E (line
end), as in Warp, iTerm2 and Ghostty — Claude Code, Codex, readline and zle
honour them — in the program's kitty keyboard encoding when it asked for
one; other ⌘ chords stay with Rift.

## Mark mode keys

While mark mode is on (`mark_mode`), a block cursor moves over the pane's
rows, scrollback included, on the primary and the alternate screen; the view
follows it.

| Key | Effect |
|---|---|
| ← → ↑ ↓ / h l k j | Move one cell / row |
| ⇧ + a move | Extend the selection from where it started |
| w / b | Next / previous word |
| 0 / $, Home / End | Start / end of the row |
| PgUp / PgDn | One screen up / down |
| g / G | First / last row |
| ⏎ or ⌘C | Copy the selection and leave |
| Esc (or `mark_mode` again) | Leave, nothing selected |
| Any other ⌘ / ⌃ shortcut | Leave, nothing selected, and run it |

## Palette keys

Esc (or the palette shortcut again) close · Enter run selected · ↑/↓ move selection · type to filter · ⇥ next mode · `>` `#` `@` typed into an empty query switch to Actions, History, Sessions. The block action *Send output to command…* opens a fourth mode with no prefix, where ⇥ and the prefixes do nothing.

## Code review keys

⌥⌘G (`focus_review`) opens the review panel if needed and gives it the keyboard; a 2 px `accent` ring marks the row it stands on · ↑ ↓ / `j` `k` / ⇥ ⇧⇥ move between sections, files and the hunks of expanded files · Home / End first / last · ← / `h` folds the section or file (else steps out to the file or section) · → / `l` unfolds (else steps in) · Enter or Space folds or unfolds · `s` stages and `u` unstages the file or hunk (on a section: all) · `r` reverts an unstaged file or hunk after the confirm dialog · `o` opens the file · `c` copies the hunk · `a` sends the hunk or file to an agent pane (a picker; Enter pastes, no Enter after it) · against a branch `s` `u` `r` only say why they are off · Esc gives the keys back to the command line (maximized: the first Esc, then a second restores the panel). Any other plain key leaves the panel and goes to the command line; ⌘ ⌃ ⌥ chords stay app shortcuts. Confirm dialogs: ⇥ ⇧⇥ ← → move between the buttons, Enter presses the focused one, Esc cancels.

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

No two default chords are the same. Where an override, an `[[actions]]`
`key` and a default name one chord, the override wins, then the action, then
the default. The fixed keys above share chords with actions only in context:
⇧⌘↑ / ⇧⌘↓ select in a multi-line command, ⌥↑ / ⌥↓ step context lines in the
filter bar, and while search is open ⇧⌘G (`toggle_review`) is the previous
match and ⌘A (`select_all`) selects the query.

Or record shortcuts in Settings → Keybindings (⌃⌘K): click a row, press the
chord. A recorded chord needs ⌘, ⌃ or ⌥, or a function or navigation key; a
bare letter is refused with a toast. The macOS menu bar shows the resolved shortcuts. Where an action has
two (a ⌃⇧ chord and a ⌘ one), the palette, Settings, menus, the welcome list
and tooltips show the ⌘ one on macOS and the other elsewhere.
