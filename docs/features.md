# Features & Behaviors

## Command blocks

Blocks are delimited by shell-integration OSC 133 markers
(`A` prompt start · `B` command start · `C` output start · `D` done + exit code):

- The shell's prompt echo is replaced by a synthesized header (command,
  prompt-time cwd/branch context, status badge, duration, copy button).
  The command is the text the editor submitted, else the line the shell
  reports in `133;C;cmdline_url=` (a reattached shell's output is replayed
  at the new window's width, where readline's and zle's echo no longer reads
  back as the command), else the echo read from the screen. A `C` with the
  line right after a bare one (another integration's, same row) fills in the
  command; one the command's own output prints later never renames its block.
- Blocks anchor to **absolute line indices** that survive scrollback trim; a
  block whose lines fall out of scrollback is pruned.
- **Background blocks** (Warp): output a background job prints while the
  prompt waits (`sleep 5 && echo done &`, zsh's `[1] + done` notice) goes
  into a block with no command and no header, between the blocks, instead
  of under the prompt or the next command's header. It starts where the
  output starts (the prompt it landed after is left out) and grows until
  the next command or prompt; it has a muted left rule and is not saved in
  transcripts. Output that stays on the prompt's row (a right prompt, a
  redraw) is the prompt's.
- A submitted command the shell never marked as started (bash 3.2's DEBUG
  trap misses `cmd &`; a syntax error) still becomes a block when the next
  prompt arrives: its echoed command, then what it printed (`[1] 4242`),
  with no exit status.
- A missing `D` (e.g. `ssh` hijacking the terminal) is closed off by the next
  `A`; duplicated markers (iTerm2 integration in the user's rc alongside ours)
  are tolerated — handlers are idempotent. A prompt whose PS1 carries `A`
  (iTerm2's does) is drawn again on its row when a line is accepted; that
  redraw keeps the submitted command waiting for its `C`, so the command's
  output stays in its block.
- Prompt-time context: the app stamps cwd + git branch onto the block at
  submit; the PTY reader fills cwd for raw-passthrough commands. Old blocks
  keep the context that was true when they ran.
- Commands receive the running viewport size before starting. Closing the
  command editor cannot leave a full-screen program using its shorter grid.
- The block's command is the exact text submitted from the editor (newlines
  and combining marks included), so reinput, re-run and copy-command reuse
  it unchanged. Commands typed in raw passthrough are read back from the
  screen instead; transcripts keep the exact text.
- Output that ends without a newline (`printf x`) stays in its block: when
  the command finishes mid-row, the terminal moves to a fresh row for the
  prompt (never sent to the shell), and copying the output adds no newline
  it did not have.

When a block's header scrolls off the top, a sticky header stays there; a
click jumps back to the block, except on its buttons, which act as on the
block's own header (`sticky_command_header = false`, or Settings › Input ›
Sticky command header, turns it off). Per pane, `toggle_sticky_header` (or
the pane menu) minimizes it to a small tab at the top right; a click on the
tab restores it. Hovering the duration shows the full time and when the
command ended. A header's copy button and its ⋮ menu button (the block's
context menu, as a right-click) show only on hover; a copy turns the button
into a check for 1.5 s and toasts "Copied output".
Block element characters (U+2580–U+259F), the legacy-computing sextants and
eighth blocks (U+1FB00–U+1FB3B, U+1FB70–U+1FB8B), box drawing (U+2500–U+257F)
and Powerline separators (U+E0B0–U+E0BF) are drawn as geometry filling the
cell and its line height, so frames, block art and prompt segments join
without gaps.

Per-block operations: select (click / ⌘↑ / ⌘↓), copy command/output/both,
copy cwd, copy git branch, re-run (only at a prompt), reinput into the editor
(⌘I), bookmark (⌘B), collapse (chevron), jump to top/bottom of block.

**Block filter** (⌥⇧F or block menu → *Filter output…*): the find bar edits
a filter for one block; only output lines containing the text (case
insensitive) stay visible. Toggles (chips at the bar's right, also ⌥ keys):
`Aa` match case (⌥C), `.*` regular expression (⌥R), `!` invert — keep the
lines that do *not* match (⌥I) — and `±N` context lines (⌥↑ / ⌥↓, 0–20; a
click adds one, ⇧-click removes one): N lines before and after each match
stay, with a dashed separator row between runs. An invalid regular
expression shows `invalid regex` in red and the last valid query stays
applied (the header pill names it); regexes are the `regex` crate's (linear
time, no backtracking) capped at 1 MiB compiled. Each block keeps its own
filter until it is cleared: ⌥⇧F (or *Turn off output filter*) turns it off
and keeps its query and toggles, ⌥⇧F again turns it back on with them;
Esc in the bar, or *Clear output filter*, forgets it.

**Multi-block selection**: ⌘-click a block (header or output) adds it to the
selection or takes it out; ⇧-click selects every block from the selected one
to the clicked one; ⇧↑ / ⇧↓ grow or shrink that range from the selected
block; ⌘A with a block selected selects every block; Esc clears. The
selection is by block, so it survives scrolling and new output. The block
menu of a selected block then acts on all of them, oldest first: *Copy N
commands* (one per line), *Copy N outputs* and *Copy commands and outputs*
(blocks separated by a blank line), *Insert N commands into the input* (one
per line, not run), *Compare outputs* (exactly two finished blocks), *Bookmark
all* / *Remove bookmarks*, *Collapse all* / *Expand all*. ⇧⌘C and ⌥⇧⌘C copy
the whole selection too.

**Compare two blocks** (two-block selection → *Compare outputs*, or block
menu → *Compare with previous run*): a modal with both commands, exit codes,
durations, directories and when they finished, and a line diff of their
outputs (`-` only in the older block, `+` only in the newer; empty lines
count). Equal output says *No differences*. *Compare with previous run*
picks the newest earlier finished block of the same pane with the same exact
command in the same directory on the same host (the menu row names how long
ago it ran). A local directory is compared resolved, so `/tmp` and
`/private/tmp` (one before a relaunch, one after) match; a remote block
matches only runs on the same host, and has no automatic baseline when its
shell did not report the host name. The outputs are read as *Copy output* reads them
(soft-wrapped rows joined, no added final newline) and bounded: at most
20 000 lines / 4 MB per block, a diff of at most 2 000 changes or 2 s, on a
worker thread (a newer comparison drops an older result). Anything
incomplete is labelled: rows gone from scrollback (*output unavailable* when
all are), a cut, or a diff that gave up (the changed part shown whole).
Comparing never runs anything. ↑↓ PgUp PgDn Home End scroll, Esc or ✕
closes.

**Copy as HTML / Markdown, save, export**: block menu → *Copy as HTML* puts
the command and output on the clipboard as HTML (a monospace `<pre>` in the
theme's colours, each run's ANSI colour, bold, italic, underline and dim as
inline styles; output text escaped) with the plain text beside it. *Copy as
Markdown*: the command in a `sh` fence, the output in a plain fence longer
than any backtick run inside it, and `… (N lines trimmed)` when scrollback
dropped its start. *Save output…* (block menu) and *Export buffer…* (pane
menu: everything scrollback still holds and the screen) ask for a path in
the palette, prefilled in the pane's directory (`~` is home; a relative path
is from the pane's directory); Enter writes the plain text; an existing file
is replaced only after *Replace the file?* is confirmed. A failed save or
cancelled replacement returns to the path prompt with the captured output
kept for retry. Destinations must be regular files (or links to them), not
pipes or devices. RTF is not offered.

## Clear semantics

- **`clear` command (soft, Warp-style)** — detected via ED3 (`\e[3J`, terminfo
  E3; full-screen TUIs never send it). Scrollback and blocks are preserved;
  the viewport is padded so old content moves above the fold and stays
  reachable by scrolling. The next commands start at the top of the
  viewport and fill it downward, as in a fresh session; the padding shrinks
  as they grow. Alacritty never sees the ED3, so its history survives.
- **⌘K Clear Blocks (destructive)** — wipes screen, scrollback, and the block
  list app-side (no shell round-trip), re-anchoring a fresh prompt block so the
  next command's header extracts correctly.
- **⌃L (editor)** — passes `\x0c` to the shell: classic screen redraw, blocks
  untouched.

## Input model

- **Editor mode**: while the last block is at `Prompt` state and the terminal
  is not in alt-screen, the app owns the command line — local editing, history,
  completions; the shell sees the command only on Enter.
- **Raw passthrough**: any other state sends keys as bytes (vim, less,
  password prompts, integration-less shells). Bound keys that would shadow
  terminal keys (PageUp/PageDown/Home/End/⌃M) automatically fall through to
  the PTY when the editor is inactive.
- **A program that died on the alternate screen** (a crash, `kill -9`) never
  switched back: when the shell prompts again after that command finished,
  Rift returns to the main screen and turns off the mouse reporting, hidden
  cursor, application keys and keyboard flags it left on, so the blocks and
  the editor come back without `reset`. A program still running (tmux, vim,
  one suspended with ⌃Z that switched back) is never touched.
- Cursor motion and deletion step over whole grapheme clusters (an emoji
  with modifiers, a letter with combining accents), never part of one.
  Mouse carets, selections and Vim operations use the same boundaries;
  Ctrl-G skips occurrences that would match only part of a cluster.
- Editor niceties: word motion & kills (whitespace-delimited), undo/redo
  (snapshot stack, cap 100, cleared on submit), the history menu (↑, below),
  ⌃R history pane, multi-source completion with
  ghost text, and configurable quote/bracket
  auto-pairing (typing a closer steps over it; Backspace removes an empty pair;
  inside a quoted string nothing pairs, so `'[a'` stays `'[a'`).
- **History menu** (↑, Warp): ↑ with the cursor on the command line's first
  line, empty or not, opens a menu docked to it: the lines that start with
  what is typed from Rift's own history (ranked by directory, below), then
  the pane's history, then the imported shell history, each once, the best
  nearest the command line. The selected line
  previews in the command line; more ↑ goes older, ↓ newer, and ↓ past the
  newest closes the menu and puts back what was typed (with its completions
  and ghost text); Esc does too. ⏎ runs the previewed line, ⇥ or → keep it
  to edit, and typing anything else keeps it and applies the key. A
  completion panel opened as you type (`completions_open_while_typing`)
  gives ↑ to the menu; one asked for (by Tab, or by ↓ into it) keeps ↑ / ↓.
  Inside a multi-line command ↑ / ↓ move between its lines. In vim mode `k`
  / `j` in Normal mode are the same keys.
- Cmd-chords never insert text into the editor nor leak bytes to the PTY,
  except ⌘⌫ in raw mode, which sends ⌃U (kitty-encoded when the program
  asked for it).
- Editor keys: ⇧←/→ select, ⌥⇧←/→ select by word, ⇧⌘←/→ and ⇧Home/End
  select to the line ends; ⌘Z / ⇧⌘Z undo and redo, ⌘X cuts; readline ⌃A ⌃E
  ⌃B ⌃F move, ⌃K kills to the end of the cursor's line (at its end, the
  newline), ⌃W deletes the word before, ⌃U clears the whole command; ⌘⌫ / ⌘⌦
  kill to the start / end of the cursor's line (at its start / end they join
  it to the line above / below), ⌥⌫ / ⌥⌦ delete a word.
- **Command corrections** (`command_corrections`, on): after a failed
  command, a "Did you mean …" suggestion shows; → / Tab / End in an empty
  editor accepts it (never runs it). From the command's output and
  directory: `git push` on a branch without an upstream → the `git push
  --set-upstream <remote> <branch>` git prints; `git <typo>` → git's first
  "most similar" command; `cd <typo>` → the closest existing directory, each
  path component within two edits; `./script` refused for want of the
  execute bit (exit 126) → `chmod +x ./script && ./script`; "Permission
  denied" / `EACCES` from a simple command (no pipes or redirections) → the
  same under `sudo`. Otherwise an unknown command word or git subcommand →
  the closest known one. Only the output's last 16 KiB is read, and only
  tokens of a git ref or subcommand shape are taken from it. `alias_expansion` (off) replaces an alias with
  its definition on Space. `error_underline` (on) underlines unknown command
  names in red.
- **Multi-line editing**: ⇧⏎ / ⌥⏎ insert a newline into the command instead of
  submitting; the editor zone grows one line (20 px) per newline and extra
  rows are marked with a dim `…` in the prompt column. Enter still submits the
  whole buffer.
- **Input position** (`input_position`): the editor sits at the bottom
  (default) or the top of the pane.
- **Blocks start** (`blocks_start`): with the editor at the bottom, blocks
  fill up from right above it (default, Warp's "Pin to the bottom"; after
  `clear` the next command sits there again) or start at the top of the pane.
- **Prompt chips** (`prompt_chips`, Warp's "Edit prompt", by mouse or
  keyboard: Tab, ← →, Delete): the context row
  shows the chosen chips in order — working directory, git branch (↑↓ and
  dirty dot), git diff stats (`3 • +10 −2`: changed files incl. untracked,
  added and removed lines, untracked files' lines as added), the branch's
  GitHub pull request (`PR #42` beside the GitHub mark, both yellow while
  open, dim draft, magenta
  merged, red closed; `gh pr view`, looked up in the background and reused
  for two minutes per branch), user, host, Python (the virtualenv, else the
  version `python3` runs: pyenv's shim resolved through `.python-version` /
  `$(pyenv root)/version`, else the install path), PHP (the version `php`
  runs, from its install path, else asked once), Docker
  Compose (the nearest `compose.yaml` / `docker-compose.yml` at or above
  the cwd, not above home: project name and running/total containers,
  `down` when none run, `off` when Docker is not running; `docker compose
  ps` in the background after each command), date,
  12- and 24-hour time, time on this tab (`elapsed`: hourglass, `2h 05m`
  since the tab's first submitted command, hidden before it; it restarts
  with each new shell and is kept only when a restart reattaches the same
  shell; its hover card says when it started, with the date after 24 hours).
  With `elapsed_counts = "running"` (Settings › Input › Time on this tab
  counts: *Only while running*) it is a time log instead: it counts only
  while a command runs in the tab (a coding agent counts until it exits;
  panes running at the same time count once, not per pane), the hourglass is
  lit while counting and dimmed while paused, and the total is kept across
  an update's restart, a crash and a reboot (it is saved when a command ends
  and every minute while one runs; quitting starts fresh, like the tabs; time
  while Rift is closed is not counted, and neither is time the Mac sleeps; a
  command still running in a reattached shell counts again from the reattach). Its
  hover card ("Counted 6h 33m · running now" or "· paused") stays open while
  the pointer moves into it and has a **Reset** button (also the palette
  action `reset_tab_timer`) that zeroes the tab's time in both modes (the
  clock restarts from now). Default: working directory, git branch, git diff
  stats. A chip with nothing to show is left out. Git, PR and Docker state
  refresh after each command, when a pane takes focus, and in every pane of
  every tab as soon as the chip list changes (Edit prompt or config.toml).
  While a Rift window is in use, its active tab's panes (every tab's, and
  also while the window is not in use, when a phone is connected) are also
  asked again every 3 s for git status and every 10 s for Docker Compose state, so files
  an agent or an editor changes while a pane is busy show without a command
  ending. Each pane reads its own directory's repository; the review panel and
  the top bar's `+added −removed` button follow the focused pane, reload when
  its status changes, and never show another repository's changes while the
  focused one's load. Settings › Input › Prompt
  chips previews them and opens the editor: click a chip to add it, drag
  chips to order them (or out of the prompt to remove them), × removes,
  Restore default, Save changes.
- **Prompt chip pickers**: click the cwd chip to `cd` into a sub-directory or
  the parent, the branch chip to check out or create a branch, the diff stats
  chip to open the code review panel on that pane's repository (Warp; a
  second click closes it), the
  PR chip to open the pull request in the browser. The PHP and Python chips
  list the installed versions (PHP: Homebrew `php` / `php@X.Y`, phpenv, asdf,
  mise; Python: pyenv's and `system`, else Homebrew `python@X.Y`, asdf,
  mise; `$HOMEBREW_PREFIX` and `$PYENV_ROOT` honoured), the active one
  checked; picking one runs `rift-use php|python <bin dir>` (that
  directory first on PATH, the previous pick removed) or `rift-use pyenv
  <version>` (`PYENV_VERSION`) — this shell only. The Docker chip lists the
  project's containers as a table — name, published ports, uptime and the
  health check's verdict (healthy / starting / unhealthy), running ones
  first; picking one runs `docker compose logs --follow --tail 200` for
  its service.
- **Shell-prompt mode** (`prompt = "shell"`): the shell's own PS1 (starship,
  p10k, …) stays visible on the grid and the editor drops the context row,
  showing a smaller muted chevron instead of the blue one.

## Vim mode

`vim_mode = true` (Settings → Input, or the palette's *Toggle vim
keybindings*) puts the owned editor into modal editing. The buffer is one
command, usually one line; when it holds several (⇧Enter), `0` `$` `^` `_`
`I` `A` `%` `J` `o` `O` `gg` `G` `>` `<` and visual-line mode work on lines,
the other motions run across the newlines, and `dd` `cc` `yy` act on the
whole buffer.

- Modes: Insert (default), Normal, Visual (`v`), Visual Line (`V`) and
  Replace (`R`). Esc leaves Insert / Replace (stepping the cursor left,
  vim-style) and the visual modes; in Normal it cancels a half-typed
  operator/count. A mode pill renders at the right of the input line —
  `NORMAL` (warning color), `INSERT` (muted), `VISUAL` / `VISUAL LINE`
  (accent), `REPLACE` (error color). Normal and the visual modes draw a solid
  block cursor, Replace an underline.
- Enter submits from any mode; submitting resets to Insert.
- Motions: `h` `l` (and Space) `0` `$` `^` `_` `w` `W` `b` `B` `e` `E` `ge`
  `gE`, `f` `F` `t` `T` + char, `;` repeats the last find and `,` repeats it
  backwards, `%` jumps to the matching bracket (the first one at or after the
  cursor on its line), `gg` / `G` go to the first / last line (`3G`, `3gg` to
  line 3). Arrows/Backspace move too.
- Search: `/` or `?` opens a prompt in the mode pill (`/text`); Enter jumps
  to the next / previous literal match (wrapping; an empty prompt reuses the
  last pattern), Esc — or ⌫ on an empty prompt — cancels. `n` / `N` repeat
  it in the same / opposite direction. Searches are motions too (`d/foo`).
- Operators: `d` `c` `y`, `gu` `gU` `g~` (lower / upper / toggle case) and
  `>` `<` (indent / outdent) with a motion or text object. Doubled, `dd` `cc`
  `yy` act on the whole buffer; `>>` `<<` `guu` `gUU` `g~~` on the cursor's
  line (a count takes more lines). `cw` behaves like `ce`; `gg` `G` `_` make
  an operator linewise.
- Indent: `>` puts two spaces in front of every non-empty line it covers;
  `<` takes one leading tab, else up to two leading spaces, off each line.
- Text objects after an operator or in visual mode: `i`/`a` + `w` `W`
  (words), `"` `'` `` ` `` (quotes), `(` `b` `[` `{` `B` `<` (brackets), `l`
  (the cursor's line; `il` without its leading and trailing blanks) —
  `ciw`, `da"`, `yi(`, `viw`, `yil`.
- `*` / `#` search the word under the cursor (else the next one on its
  line) forward / backward as a whole word; `n` / `N` repeat it.
- Visual mode selects on the editor's own selection, inclusive of the char
  under the cursor (whole lines in Visual Line), so ⌘C copies exactly it.
  Motions, searches and `j` / `k` (↑ / ↓) extend it; `o` swaps its ends;
  `v` / `V` switch or leave. Operators: `d` `x` delete, `c` `s` change, `y`
  yank, `~` `u` `U` case, `>` `<` indent, `J` join, `gu` `gU` `g~`; `D` `X`
  `Y` `C` `S` `R` take whole lines.
- Registers: `"` + name before an operator or put. `"a`–`"z` hold their own
  text (`"A`–`"Z` append to it), `"0` the last yank into the unnamed register,
  `"_` drops the text, `"+` / `"*` are the system clipboard (text ending in a
  newline puts as whole lines). The unnamed register holds the latest stored
  text; `vim_clipboard = "unnamed"` makes it the clipboard (vim's
  `clipboard=unnamed`).
- Edits: `x` `X` `s` `S` `C` `D` `r`<char> `~` `p` `P` (a register),
  `J` joins the next line (one space between; a count joins more), `o` / `O`
  open a line below / above and enter Insert.
- Replace mode (`R`): typed chars overwrite (and append past the end), ⌫
  puts back what the last one overwrote.
- `.` repeats the last change: an operator with its motion or text object,
  `x` `r` `~` `p` `J` and the rest, a visual-mode operator (from the
  cursor), a Replace session, and an Insert session with the text it typed
  (`cw` + text, `A` + text, `o` + text). A count replaces the change's count
  (`3.`).
- Insert entry: `i` `a` `I` `A` `o` `O`.
- Counts: digits before and/or after the operator, multiplied (`2d3w`).
- Undo `u`, redo ⌃R (outside Insert; in Insert ⌃R stays the history pane).
  One `u` undoes one change: a whole Insert or Replace session, a
  visual-mode operator, a `.` repeat. `U` puts back the last changed line as
  it was before its latest changes (itself a change: `U` again undoes it).
- History recall: `k` and `j` (also ↑/↓) in Normal mode are ↑ / ↓ of the
  command line: `k` opens the history menu and goes older, `j` newer (past
  the newest it puts back what was typed); in a multi-line command they
  move between its lines.

## Selection & mouse

- Drag = character selection, double-click = semantic (word) selection,
  triple-click = line selection; selection survives scrollback rotation.
- **Smart double-click**: inside a URL (any `scheme://`, `mailto:`), a file
  path (with its `:line:col`), an email address, an IPv4 address (with a
  `:port` or `/prefix`) or IPv6 address, a number (`-2.5e-3`, `1_000`,
  `0xFF`), a UUID or a git SHA (7–40 hex digits), the double-click selects
  it whole; surrounding quotes and brackets and trailing `.` `:` `!` `?`
  are left out; one soft-wrapped over several rows is selected whole.
  Elsewhere it selects a word; `word_chars` lists characters
  that then count as part of one (default boundaries: blanks and
  `` ,│`|:"'()[]{}<> ``), and `word_delimiters` characters that end one
  (they win over `word_chars`; the smart units above stay whole).
- **Word moves at the prompt** (⌥← ⌥→, ⌥⌫, ⌥D): stop at blanks, and also
  at the `word_delimiters` characters (e.g. `"/.-"`: `src/app/main.rs` is
  then four words).
- ⌘C copies the selection when one exists, else the last block's output.
- ⌥-drag makes a rectangular (column) selection.
- Typing clears the selection.
- **Mouse reporting** (`mouse_reporting`, on): programs that enable xterm
  mouse tracking (vim, tmux, htop, fzf) receive presses, releases, drags
  (1002), motion (1003) and the wheel, sideways too (buttons 66/67), in SGR (1006), UTF-8 (1005) or legacy
  encoding; ⇧ forces local selection. Focus in/out (1004) is reported when
  the window gains or loses focus.
- Links draw in the accent color, lighter and underlined under the pointer,
  and show a hand and a tooltip with their target; a plain click shows it or
  opens the link (`link_click`), also over a program that tracks the mouse
  (Claude Code, Codex) and on the alternate screen. A URL soft-wrapped onto
  the next rows is one link.
- ⌘-click opens URLs, OSC 8 hyperlinks and existing file paths (see ui.md);
  files and folders open where `link_editor` says (default app, the viewer
  pane or an installed editor), and the explorer / review ↗ follow
  `file_editor` (Settings › Files).
  A relative path resolves against the directory the command that printed
  it ran in (the pane's current directory for output outside a block); hint
  mode resolves paths the same way. The line and column printed with a path
  go to the editor (`code --goto f:12:3`, `zed f:12:3`, JetBrains `--line`)
  or the viewer: `f:12:3`, `f(12,3)`, `f(12, 3)`, `f[12, 3]`, `f, line 12,
  column 3`, `File "f", line 12` (Python tracebacks) and `f line 12`.
- Right-click a link or an existing file path in output: *Open link* / *Copy
  URL*, or *Open* / *Copy path* / *Reveal in Finder* (Linux: *Show in file
  manager*), above the block or pane menu.
- A copy, paste or open that fails shows a toast with the reason (in the
  pane, until dismissed); `Copied` appears only when the clipboard took the
  text.
- Click a block header: select the block (again: deselect). A click without a drag on its output also selects it (again: deselect); a drag selects text. Click its chevron: collapse.
- **Copy on select** (`copy_on_select`, on by default): a drag-selection is
  copied to the clipboard as soon as the mouse is released.
- **Mark mode** (⇧⌘M / ⌃⇧M, Windows Terminal): a block cursor on the
  screen's cursor row (or the selected block's first row) moves with the
  arrows or hjkl, w / b, 0 / $, PgUp / PgDn and g / G over the rows the pane
  shows, scrollback included; ⇧ extends a selection, ⏎ or ⌘C copies it and
  leaves, Esc leaves with nothing selected, and any other ⌘ / ⌃ shortcut
  leaves and runs. It works over a program on the
  alternate screen too, and a screen reader hears that it started.
- **Secret masking** (`mask_secrets`, off): tokens, keys and passwords show
  as `••••` (same width) on screen and are masked in every copy (single and
  multi-block, HTML and its plain text, Markdown), Save output / Export
  buffer files, the comparison, the history pane's Results (a secret's
  value finds nothing there), palette history rows, notifications, inbox
  entries and their announcements, saved transcripts and the phone mirror; a click shows one for 5 s, and the block
  menu's *Copy output unmasked* copies the real text. A display aid, not a
  guarantee: [configuration.md › Secret masking](configuration.md#secret-masking).
- **Scrollbar** (`scrollbar`, `auto`): a 6 px overlay in the pane's right
  padding, its thumb the view over the blocks as drawn (collapsed output and
  filters count as shown); ticks mark prompts (dim), failed commands
  (error), bookmarks (accent) and find matches. Drag the thumb to scroll; a
  click on the track pages. `auto` shows it while the pane scrolls (1.2 s)
  and under the pointer; `always` while there is anything to scroll.

## Editor keys from Warp

- ⌥. (Meta+.) inserts the last word of the previous command, as typed
  (quotes and `\ ` kept); pressed again it replaces it with the last word
  of the command before. ⌥ is Meta where Option is (`option_as_meta` on
  macOS, always elsewhere); otherwise ⌥. types its character.
- ⌃⌥← / ⌃⌥→ move by word parts: `camelCase` and `HTTPServer` humps,
  `snake_case`, `kebab-case` and path segments.
- ⌃U clears the command as before, and puts what it cleared on the
  clipboard.

## Multi-cursor editing

As in Warp: ⌃⇧↑ / ⌃⇧↓ add a caret on the line above or below (same
column, or the line's end), ⌃G selects the word at the cursor and then
adds a caret selecting its next occurrence (after the last caret,
wrapping), and ⌘-click (⌥-click on Linux) adds a caret where you click.
Typing, Backspace, Delete, word and line deletes, ⌘X, the arrows,
Home/End and their Shift selections act at every caret, each edit one undo
step; carets that meet merge. A paste with as many lines as there are
carets gives each caret its line, any other paste goes in whole at each.
Esc, a plain click, undo, history and Enter go back to one caret (Enter
runs the whole command). While there are several carets the completion
menu and the inline suggestion stay away; vim mode keeps one cursor.

## Command inspector

Warp's Command X-Ray. Rest the pointer on a part of the command line for
600 ms (the tooltip delay), or press ⇧⌘I (⌃⇧I) for the part at the
cursor: a card names it — command, subcommand, option, option value,
argument or operator — and says what it does. Options and subcommands come
from the completion specs at their level (`git -p log` is git's `-p`, `git
log -p` log's), an argument says what kind it is (a folder of `cd`, a git
ref), operators say what they do (`&&`, `|`, `>`). A command Rift has no
words for is looked up once with `whatis` off the UI thread (up to 10 s;
"Looking it up in the manual…" until then). The card moves with the
cursor while it is pinned; another key puts it away, and a screen reader
hears it.

## Completions

Sources merged, deduped, capped at 50: **`$PATH` command names** (first word
only, from 1 typed char, max 12 — every executable on the pane shell's `$PATH`), command specs (subcommands and flags with descriptions for common tools — git, cargo, docker, npm and more; `completion_specs.rs`) with argument generators (git refs, branches and remotes; ssh hosts from the ssh config and `known_hosts`; Makefile targets; `package.json` scripts; up to 50 each), filesystem paths (`~/` expanded;
hidden files only when the typed name starts with `.`; max 30), and command
history. A pane whose shell is on another host (or never reported a
directory) gets no path, file-based argument or command-name completions
from this machine (its `$PATH` is not the remote one); ssh hosts and history
still complete, and an SSH profile's pane completes on the remote machine
(see SSH profiles).

**Completions from your shell** (`shell_completions`, on by default;
Settings › Input). For an argument (not the command name, not a
word that looks like a path) Rift also asks your own shell, so tools Rift
has no spec for (`terraform`, `just`, your `_mycmd`) complete as they do on
Tab in the shell, with your plugins and completion functions. One warm
completion shell per shell program runs in the background (`shell/complete.
{zsh,bash,fish}`), started on the first request, reused, and started again
when it died; each request sets its directory:
- **zsh** runs your interactive zsh (rc files, `compinit`, plugins) in a
  `zpty`; a widget reads the directory and the line as data (not as keys,
  so your widgets on printable keys — autopair, magic-space, abbreviations
  — never see them), sets the editor buffer, runs complete-word (Return is
  unbound there) and captures what `compadd` adds, with descriptions (the
  technique of zsh-capture-completion). `compinit` is loaded when your rc
  does not. The `_expand` completer is disabled there: it evaluates the word
  (glob qualifiers such as `*(e:…:)`, substitutions). The server also ends
  on its own when Rift is gone while a completion function hangs.
- **bash** calls the function the command's `complete -F` spec names, with
  `COMP_WORDS` set as on Tab, after bash-completion's loader
  (`__load_completion` / `_comp_load` / `_completion_loader`) when it is
  loaded; word-list (`-W '…'`) and action specs (`-A …`, `-d`, `-f`…) go
  through `compgen`. A builtin with no spec (bash-completion not installed)
  still completes the names bash knows for it: signals (`kill -SIGT`,
  `trap`), variables (`unset`, `export`, `declare`…), aliases, jobs (`fg`,
  `kill`), help topics, `shopt` and `set -o` options, commands (`type`,
  `which`) and builtins (`builtin`, `enable`).
- **fish** runs `complete --do-complete=<line>`.

Its words are merged in as source `shell` and ranked like the rest; a word
Rift's own specs also offer keeps Rift's row. Rift's own candidates show at
once; the shell's join within 300 ms (later answers are dropped), and the
shell runs on its own thread, so a slow or hanging completion function never
delays the menu, editing or the next keystroke's candidates. A request that
misses the budget stops that shell (its process group, so the function's
children too), and a new one starts after a second. The command line is
data: lines holding a control character or a command substitution (`$( )`,
backticks, `<( )`, `>( )`, `=( )`, and in zsh and fish any `( )`: zsh glob
qualifiers and parameter flags run code) are never sent, and each server
passes the line to the shell as a variable or as the editor buffer, never
as code. Completion functions may run commands, as they do
on Tab in the shell itself. Replies are bounded (400 words, 512 bytes each)
and stripped of escape sequences and control characters. Never for a pane on
another host. A directory bash lists without its `/` (`complete -d cd`)
joins Rift's `dir/` row; bash's file-name specs (`-d`, `-o filenames`,
`-o dirnames`, `-A directory`) mark directories with `/`. Off:
`shell_completions = false` (the running shells stop, also when config.toml
turns it off or names another `shell`). The shells, and whatever their
completion functions started, stop when Rift quits.

**Ranking** (Warp's matcher). A word candidate matches the typed word by
prefix, in any case (the word itself too: `git` lists `git` before `gitk`),
else as a subsequence when the word has a letter or digit (its characters in
order, any case — `cko` finds `checkout`, `tfa` `convertformat`; from one
typed character). Candidates sort by that match kind — subsequences by score:
each matched character counts, more at a word start (after `-` `_` `.` `/`
`:` `@` or a space, or at a lower→upper case change), runs gain, gaps cost —
then by source (command names, subcommands, options, generator values,
paths), then by how recently the word appears in the history (the pane's
last 200 lines, then the shell's newest 500), then in the provider's own
order (paths and values by name; git refs branches, remote branches, tags).
The order is deterministic. At most 45 word candidates are kept, so the
history lines (at most 5) always fit in the 50. The menu orders its rows as
Warp does: the word exactly as typed, then exactly in another case, then the
prefix matches in that order, then the subsequence matches by score; the
matched characters show bold (the prefix, or each character of a
subsequence).

**Caching.** Providers run on every keystroke, so what they read is cached
and checked, not re-read: directory listings (16 kept; entry types from the
directory entry, only symlinks are `stat`ed), git refs per repository
(`HEAD`, `packed-refs` and every directory under `refs/`), git remotes
(`.git/config`), ssh hosts (`~/.ssh/config`, every file it `Include`s and
each directory an `Include` glob lists, `known_hosts`), Makefile targets and
`package.json` scripts (32 lists kept). Each request compares the
modification time, size and inode of those paths (one `stat` each) and reads
again when any changed, so a new branch, file or target shows on the next
keystroke. A value read within 2 seconds of one of its paths' modification
time is never reused (a second change in the same timestamp tick could hide).
The `$PATH` scan is cached per `$PATH` value (4 kept): a new value (the
shell probe, or the pane's own `$PATH` its hooks report after `rift-use` or
an `export PATH=…`) is scanned at once; otherwise its directories' stamps
are checked at most every 2 seconds and a changed directory rescans.

Path completions are inserted backslash-escaped as one shell word (valid in
bash, zsh and fish; a leading `~/` stays unescaped so it still expands), and
a token typed with backslashes or quotes (`My\ Dir/`, `"My Dir/`) still
completes. Names containing control characters are not offered. A command
name shaped like an assignment (`name=value`) gets its `=` escaped too, so the
shell runs the file instead of setting a variable; reserved-word names
(`time`, `[[`) stay bare, as the shell's keyword is what they usually mean.

Completion works on the whole word around the cursor, wherever the cursor
sits in it (like zsh without `complete_in_word`): accepting replaces that
word, including a closing quote or the rest of the word right of the cursor,
and leaves the following arguments untouched (`cat "./hel"` and
`cat ./hel|lo.txt --other` both become `cat ./hello.txt …`).

A typed word is read by shell rules: unquoted, a backslash escapes any
character; inside `"…"` it escapes only `$`, `` ` ``, `"`, `\` and newline and is
otherwise literal (`"./a\b"` completes `a\b.txt`, not `ab.txt`); inside `'…'`
everything is literal.

Providers run on a background thread, never on the UI thread. Tab, ⌃Space or
⌃→ pressed while an answer is still in flight is applied once, when it
arrives, and only if the pane, buffer and cursor are unchanged; Escape cancels the
request, so a late answer cannot reopen the menu. If that thread cannot start
or has died, completion pauses (no menu or ghost text; editing is unaffected)
and the first edit after 5 seconds starts a new one.

History matches drive the ghost text: whole-buffer prefix matches, max 5,
one per command. First comes what usually follows the pane's previous
command (Warp's next-command history): the last command the pane finished
from the command line in this run (a restored block never counts) is looked
up in Rift's command history — its 25 newest runs with the same command
line, directory and exit code — and the command each was followed by in its
own shell session; those starting with the typed text, most frequent first
(ties: the newer), at most 5. So in a project where every new shell runs
`cd project1`, then `bash scripts/db.sh backup`, then `… restore`,
typing `bash scripts/db.sh` after `cd project1` suggests the backup,
not the newer restore. Then Rift's own command history, ranked by
directory: commands run in the pane's directory, then in the same git
repository (its working tree; outside a repository only the directory
counts), then anywhere, newest first within each, so `pnpm de` in a project
where `pnpm dev` ran suggests that even when another project ran
`pnpm dev:web` later. Then the pane's own history (last 200), then the
imported shell history (newest 3000), which has no directories. A pane on
another machine (ssh) has no local directory and skips Rift's history. A
request shares the pane's history with the editor instead of copying it.
History lines that name a missing path are left out (Warp's autosuggestion
validation): in `cat`, `cd`, `ls`, `python` and `python3` (the commands
whose path argument must already exist) the first path argument must exist
in the pane's directory (a directory for `cd`), and the script of a script
runner (`bash`, `sh`, `zsh`, `fish`, `node`, `ruby`, `perl`, `source`, `.`:
its first argument that is not an option) must be a file there, so
`cat gone.txt` and `bash scripts/gone.sh` are skipped and the next match is
suggested. Commands without that spec, options the spec does not know, code
given inline (`bash -c`, `sh -s`, `node -e`, `ruby -e`, `perl -e`), words
with a variable, glob or `~user`, a remote `host:path` and redirections
count as valid; a pane on another machine checks nothing, and the checks stop
after 150 ms (the rest count as valid).

**Ghost text** shows the top suggestion after the cursor: the best history
line, else the rest of the first candidate going on from the word as typed.
At the end of the command line (Warp) → , ⌃F, ⌃E, ⌘→ and End take all of
it, ⌥→ one word, ⌃⌥→ one subword (camelCase, `_`, `-` parts), ⌃→ one word
wherever the cursor is; a partial accept leaves the rest as the ghost, and
typing what it shows shrinks it. Elsewhere on the line those keys only move.
Esc clears it (and closes the menu). In vim mode the ghost text (and its
chips) goes on after Normal's block cursor on the last character, where
Normal's `l` and `$` take all of it, `w` and `e` one word; Insert's Esc keeps it
(Normal mode starts), Normal's Esc clears it.

**The accept-key chip** (`autosuggestion_hint`, on by default; Warp's
keybinding hint) follows the ghost text and names the key that takes it all:
→, Tab (`tab_behavior = "autosuggestion"`), ⇧→ or the `accept_autosuggestion`
binding. Hover says *Change keybinding*; a click opens a menu of →, Tab, ⇧→
and Custom…, the current one checked (↑ ↓ ⏎ or a click pick, Esc closes).
→ is the default (Tab completes, no binding); Tab sets `tab_behavior =
"autosuggestion"`; ⇧→ binds `accept_autosuggestion = "shift+right"` (with no
ghost text to take, ⇧→ still extends the selection); each writes config.toml
like a Settings change. Custom… opens Settings › Keybindings filtered to
`accept_autosuggestion`. → always takes the ghost text at the end of the
line, whichever is picked.

**Ignoring a suggestion** (`autosuggestion_ignore_button`, off by default;
Warp's ignore button): a ✕ after the ghost text (*Ignore this suggestion*)
ignores the whole suggested command line — exactly that line, from every
source: what usually follows the previous command, Rift's history, the pane's
and the shell's history, and the first completion candidate. The next
suggestion takes its place at once. Ignored lines are not offered by the ↑
history menu or the phone's history rows either. Running an ignored line
suggests it again (Warp), as does the palette's *Clear ignored suggestions*
(`clear_ignored_suggestions`). They are kept in `ignored-suggestions.json` in
the data directory, bounded, private and merged across Rift processes like
`history.json` (configuration.md, File locations). A scripted run reads and
writes it only with `RIFT_SCRIPT_RECORD_IGNORED=1` under its own
`XDG_DATA_HOME`; otherwise its ignores last for the run.

**Rift's command history** survives restarts: every command a local pane
runs from the command line is recorded when it finishes, with its
directory, that directory's repository, the finish time, the exit code and
its shell session (a new id for every shell a pane starts, and its place in
it), in `history.json` in the data directory (configuration.md, File locations).
Not recorded: a command typed with a leading space (the shells'
`HIST_IGNORE_SPACE` / `ignorespace` convention), one run over ssh (its
directory is on the other host), one the secret patterns match (the user's
`secret_patterns` while `mask_secrets` is on, else the built-in ones), and
command text a program printed with OSC 133 rather than the editor
submitting it. It keeps the newest 10,000 commands within 2 MiB of command
and directory text (a single command or directory over 4 KiB is not kept);
the file is written atomically, readable by the user only, and merged with
what other Rift processes wrote (they take turns through a lock file). A
file that is corrupt, too large or of another version is not read, and the
next write moves it aside to `history.json.bad` rather than replacing it. A
scripted run (`--script`) reads it only with `RIFT_SCRIPT_HISTORY=1` and
writes it only with `RIFT_SCRIPT_RECORD_HISTORY=1` under its own
`XDG_DATA_HOME`.

**Tab** (Warp's default, `tab_behavior = "completions"`) completes the word
at the cursor on a line with text, one caret and no selection; it never
takes the ghost text there. One candidate starting with the word as typed
(or, with none such, one in another case) is inserted with no menu; several
insert their common prefix when it is longer than the word and open the
menu; otherwise the menu opens with the first row selected; no candidate:
nothing. In the open menu, Tab inserts the common prefix of the rows
matching as typed when it is longer than the word (one such row left: the
menu closes), else selects the next row; ⇧Tab the previous (both wrap); ↑ / ↓
move; ⏎ or a click accepts the selected row; Esc closes it. Typing with the
menu open narrows its rows to the word; it closes when none matches, when the
cursor moves before the word, or when the line no longer starts with what it
was opened on. An accepted completion that ends the command line gets a space
after it, unless it ends in `/` (a path goes on) or `=` (a value follows). A
selection keeps Tab as it was (the ghost text goes on the end).
`tab_behavior = "autosuggestion"`: Tab takes one word of the ghost text, or a path
part up to and including its `/` (→ takes the whole of it), and with none completes as above; ⌃Space then opens the completions.

The completion panel (the menu) lists the word candidates only — history
lines drive the ghost text, ↑ and ⌃R list them — under a header naming it
with its count.

**As you type** (`completions_open_while_typing = true`, off by default; it
was `completions_menu`, still read): the menu opens on a typed edit (typing
or deleting) with the cursor at the end of a line of two characters or more,
and follows every edit while open; ⏎ accepts the selected row unless the
word already is it (a trailing `/` aside), then runs the line; accepting adds
no space.

**History pane** (⌃R): this window's finished commands and Rift's command
history (exit status, directory, finish time), ranked like the ghost text
(the active pane's directory, then its repository, then anywhere; newest
first within each), then the pane's typed history and the shell's history
file, one row per command; the command line filters it, ⌃R cycles the scope
(this pane, all shells, failed, results), ⏎ runs, ⇥ edits first, ⌘⏎ runs in
a new split; Esc, or ↓ past the last row, closes it. After a restart it
lists the previous sessions' commands with their directories.

**Results** (the history pane's fourth scope): past runs found by their
command *or output*, across every window's open panes (restored blocks
included) and, while `save_transcripts` is on, the saved transcripts no open
pane shows. Each run is its own row — the same command run twice, or on two
hosts or branches, stays apart — with its status, host (`local`, the ssh
host's name, `remote host` when the name was not recorded, `unknown host`
when the shell reported no directory), branch when recorded, directory and
age (`date unknown` when not recorded). Query words filter:
`is:failed` / `is:ok`, `since:today` / `since:7d`, `dir:<text>`,
`host:<name>` (`local`, `remote`, `unknown`), `branch:<name>`; the rest is
the text searched (any case). Below the rows the selected run's output shows
around the first matching line, the match marked; previewing runs nothing and
never connects anywhere. ⇥ puts the command on this pane's command line,
⏎ reruns it here (the pane's own directory and host, under the usual prompt
rules), a click selects, a double-click edits. The search runs on a worker
thread, bounded (32 MiB of text read, 1 MiB per block, 200 results; the
pane says when it stopped at the bound); a newer query replaces an older one's
answer. There is no index: output that left the scrollback, or was never
saved, is shown as unavailable, saved transcripts are not read while
`save_transcripts` is off, and restored blocks are not searched then either;
"Clear saved block transcripts" removes them from search at once (and they
are not saved again). Imported shell history has no output and stays in the
other scopes, labelled `imported`.

**Shell widgets** (Warp): where your shell config binds ⌃R, ⌃T or ⌥C to a
widget of its own (fzf, atuin, fzf.fish: a zsh user widget, a bash
`bind -x` or macro, a fish user binding in the current mode), that key in
the command line runs it instead. Rift hands the command line and cursor to
the shell, the widget's interface draws below the prompt and takes the keys,
and the line it leaves comes back into the command line, not run; a
cancelled widget leaves the line and cursor as they were. A widget that runs
a command itself (fzf's ⌥C `cd`, atuin's `enter_accept`; in fish a binding
that queues `commandline -f execute`) runs it as a block, and the command
line keeps what it held. While a widget runs, the rich input composer does
not open (⌃⇧G does nothing); in an open composer ⌃R, ⌃T and ⌥C are never
handed to the shell. Each prompt reports the bound keys, so an unbound key
keeps Rift's behaviour (⌃R the history pane). ⌥C counts only where Option is
Meta (`option_as_meta`; elsewhere Alt always is). Needs Rift's shell
integration (bash 4.3 or newer); a line over 4 KiB keeps Rift's behaviour.
`shell_widgets = "rift"` always uses Rift's own.

`autosuggestions = false` turns the ghost text off.

## Launch configurations

Warp-compatible YAML or TOML Tab Configs in `~/.config/rift/launch_configs/`
(and, read-only, Warp's own `tab_configs` directory) — a named set
of tabs, each with a (possibly nested) split layout, cwds and startup commands.
⌘P → `Launch configuration: <name>` opens one: tabs are appended, splits are
rebuilt, and each pane's `commands[].exec` are sent **at that pane's first
prompt**. A launch configuration opens in the current window; the tabs of
all `windows:` entries are appended in document order; `~` in a `cwd` is expanded and a non-existent directory is
ignored. Warp's own files drop in unchanged. "Save current layout as launch
configuration" writes one TOML Tab Config per open tab. A pane may name its
own program (`shell`, as in Warp), a profile (`profile`) and arguments
(`args`); a pane whose profile or program cannot open stops the whole
configuration with a toast.

## Profiles

Named launch settings (`[[profiles]]` in config.toml, Windows Terminal
profiles): program, arguments, start directory, environment, tab title and
colour. The shells in `/etc/shells` are profiles too (`zsh`, `fish`, …;
`discover_shells`). `default_profile` is what ⌘T, + and splits open, and
every other new tab too (the first tab, ⌘N, links, `rift-terminal <path>`,
`ctl open`, the phone, the drop-down window): its directory unless a folder
was named, its title and colour. Duplicate tab keeps the source's folder,
title and colour (the profile's where the source has none). The ⌄
beside + (or right-click +) lists them with ⌃⇧1…⌃⇧9 shortcuts, then
Settings, the command palette and About; the palette has `New tab: <name>`
and `Split right/down: <name>`. Each pane runs its own program with its own
shell integration and completions; a profile's environment reaches only its
panes. A pane remembers its profile through duplicate, save layout and
restore; a reattached shell is never started twice. An unknown profile or a
missing program is a toast naming both, and nothing opens. A profile's
`theme` gives its panes their own colour scheme (terminal content only; the
chrome keeps the window theme); the palette's `Pane theme: <name>` sets the
active pane's, `Pane theme: default` restores the window theme, and the
session keeps it. **Settings › Profiles** manages them: the default
profile, the installed-shells switch, and a row per profile (Default,
Detected, Hidden and Not found tags) with an edit form, Duplicate, Hide /
Show, Delete (asked; a deleted detected shell never comes back) and Set as
default; drag a row or press ⌥↑ / ⌥↓ to reorder. Changes are validated
and written into config.toml in place, comments kept. Reference:
[configuration.md › Profiles](configuration.md#profiles).

## User actions

`[[actions]]` in config.toml (Windows Terminal `sendInput` and
`multipleActions`): a name, an optional chord and steps that run an action,
type text into the pane (`\n` runs the line) or open a launch configuration,
in order; a step may name another user action, which runs inline (at most
32 steps once expanded). Each is a palette row "Action: <name>" with its
chord (a `[keybindings]` entry on the same chord wins; the action's chord
wins over a default). A step that
opens a pane makes it the target of the next ones, and text sent to it waits
for its first prompt (including a new window), so it lands in the new pane whatever you focus
meanwhile. The first step that fails stops the rest with a toast naming the
action and the step; earlier steps stay done. Invalid or cyclic actions make
config.toml not apply. Reference: [configuration.md › User actions](configuration.md#user-actions).

## Pane focus links (`RIFT_PANE_URL`)

Every shell Rift starts gets `RIFT_PANE_ID` and
`RIFT_PANE_URL=rift://action/focus?pane=<RIFT_PANE_ID>`. Opening that link
(`open "$RIFT_PANE_URL"` on macOS, `rift-terminal "$RIFT_PANE_URL"` or
`xdg-open` anywhere) raises the window holding the pane and focuses its tab
and pane, the drop-down window included. A local shell's id is
`<run token>.<pane>`: it names a pane of that Rift run only (after a restart
the shell is gone, and the link says so in a toast). A shell in the session
service (`durable_sessions`) is named by its service session,
`service.<session id>`, so the link still finds the pane after Rift restarts
and reattaches it. The link focuses; it never runs or opens anything.

A Claude Code hook that brings you back to the pane that needs you
(`~/.claude/settings.json`):

```json
{
  "hooks": {
    "Notification": [
      { "hooks": [ { "type": "command", "command": "open \"$RIFT_PANE_URL\"" } ] }
    ]
  }
}
```

An ntfy notification whose click returns to the pane (the ntfy app opens the
`Click` URL on the computer that has Rift):

```sh
long-job; curl -H "Title: long-job finished" -H "Click: $RIFT_PANE_URL" -d "exit $?" ntfy.sh/my-topic
```

## Links (`rift://`)

Finder, Raycast, an editor or a README can open Rift through `rift://`
links (Warp's `warp://` paths):

| Link | Opens |
|---|---|
| `rift://action/new_tab?path=<folder>` | A tab in that folder (no `path`: where a new tab starts) |
| `rift://action/new_window?path=<folder>` | A window, its tab in that folder |
| `rift://launch/<name>` or `rift://tab_config/<name>` | The launch configuration or Tab Config of that name (case ignored); `?new_window=true` opens it in a window of its own |
| `rift://settings?q=<text>` | Settings, its search filled in |
| `rift://action/focus?pane=<RIFT_PANE_ID>` | Raise the window holding that pane and focus its tab and pane (every shell has its own link in `RIFT_PANE_URL`); a pane that is gone, or a local shell of an earlier Rift run, says so in a toast |
| `rift://action/open_file?path=<file>&line=<n>[&col=<n>]` | The file where `link_editor` says, at that line: an editor's CLI on `PATH` takes the line (`code --goto f:12:3`, `zed f:12:3`, `subl f:12:3`, JetBrains `--line 12 --column 3 f`; an app bundle without its CLI opens the file without the line), the viewer (`link_editor = "rift"`, or the editor is not installed) scrolls to it |

`path` is percent-decoded (`+` is a space), `~` is expanded, and it must be
an absolute path to an existing folder (`open_file`: an existing regular
file; `line` and `col` are whole numbers from 1, `col` counts only with a
`line`). A link runs no command of its own: `open_file` opens the file in an
editor or the viewer and never runs it (`link_editor = "system"` reveals a
program instead of running it); a file in a tab link (a script included) or
an unknown action is refused with a toast saying
why (control characters removed, cut at 120 characters). The link opens in the last focused window (never
the drop-down window while another is open) and brings Rift to the front.
macOS registers the scheme with `Rift.app` and delivers the link as an Apple
Event (also to a Rift it launches); on Linux the `.desktop` file registers
`x-scheme-handler/rift`, and `rift-terminal rift://…` hands the link to the
running Rift over the control socket (with `control_api = "off"` it takes
links and nothing else) or opens it itself when none answers. A layout
whose panes run commands or customize the startup program, arguments or
environment asks first (*Open this layout from a link?*, its commands and
startup arguments listed, custom environment values kept private, Cancel
focused), and more than ten links in three
seconds are dropped. A link while only the drop-down window exists opens
an ordinary window for it.

## Opening Rift from other apps

A folder opens in Rift from wherever it is named; a file opens its folder.
Each becomes a `rift://action/new_tab` link (`new_window` with
`--new-window`), so it reaches the running Rift the way a link does.

- **Command line and scripts:** `rift-terminal <path>…` opens each folder in
  a new tab of the running Rift (`--new-window`: a window). A relative path
  is resolved against the caller's directory; a path that does not exist
  prints `rift-terminal: <path>: no such file or directory` and exits 2, and
  nothing opens. `file://` URLs are taken too (a Linux launcher's `%U`). The
  running Rift is reached over its control socket (macOS: needs
  `control_api` at `navigate` or `full`, the default; Linux: always); when
  none answers, the process starts Rift itself and its first tab opens in
  the first folder (a restored session keeps its tabs and gets a new one).
  `--new-window` alone opens a window where a new tab starts (on a cold
  start, that window is the first one). A folder named like a subcommand
  (`ctl`, `remote`, `relay`) is opened by its path: `rift-terminal ./remote`.
- **The `rift` command:** the palette's *Install 'rift' command*
  (`install_cli`) links `~/.local/bin/rift` to the running Rift binary, so
  `rift .` works in any shell; JetBrains IDEs and VS Code can use it as
  their external terminal. An earlier link to a Rift binary is replaced; any
  other file at that path is never replaced (an error toast says so). When
  the folder is not on the shell's `PATH`, the toast says to add it.
  `RIFT_CLI_DIR` installs into another folder (tests).
- **macOS — Finder, the Dock, editors:** `Rift.app` declares folders
  (`CFBundleDocumentTypes`, `public.folder`, role Shell, rank Alternate: it
  never becomes the default for opening folders). A folder dropped on the
  Dock icon, *Open With → Rift* in Finder, and `open -a Rift <dir>` (VS
  Code's and JetBrains' external terminal on macOS) arrive as an
  open-documents event and open a tab there; one that launches Rift opens
  its first tab there.
- **Linux — launchers:** the `.desktop` entry runs `rift-terminal %U`, so a
  folder dropped on the launcher opens a tab there. It does not claim
  `inode/directory`: that would let Rift replace the file manager as the
  folder default on desktops without a set preference.

## Appearance

- **App icon** (`app_icon`, Settings › Appearance): Default, Light, Mono,
  Ocean, Ember, Forest or Neon — the "Prompt" icon in other colours. macOS
  shows it in the Dock at once and, through the Dock tile plugin in
  `Rift.app`, while Rift is not running (the app's own `.icns` and Finder
  icon stay the default: rewriting a signed bundle's icon breaks its
  signature). On Linux it is the window icon; docks keep the installed
  hicolor icon.
- **Themes**: thirteen builtins (`dark-default`, `light-default`,
  `catppuccin-mocha`, `tokyo-night`, `catppuccin-latte`, `dracula`,
  `gruvbox-dark`, `gruvbox-light`, `monokai`, `nord`, `one-dark`,
  `solarized-dark`, `solarized-light`) plus user themes in
  `~/.config/rift/themes/` as Rift TOML **or Warp YAML** — a Warp theme
  file works unchanged, gradient `background` / `accent` included (drawn as a
  vertical or horizontal two-stop gradient behind the panes and on the
  active tab's edge); Rift derives its full token set from Warp's base
  colors. A user file named like a builtin replaces it.
- **Theme picker** (`theme_picker`, palette *Theme picker (live preview)*):
  the selected theme applies live, nothing saved; ⏎ keeps it, Esc reverts.
- **OS theme sync** (`sync_os_theme`): follow the system appearance —
  `theme` while dark, `theme_light` while light — switching live when the OS
  does.
- **Window opacity** (`window_opacity`, 1–100 %, default 50 % with a 20 px
  blur), as Warp draws it: below
  100 % the tab bar, side panels, gutter and panes lose their own fills and
  share one window ground — Warp's `surface_2` (bg + 10 % fg) at the opacity
  with `bg` at the opacity over it, so 50 % covers 75 %. Overlays, modals,
  text and non-default cell colours stay opaque. With a theme background
  image the image is drawn at the opacity and veiled by `bg` at
  `(1 − image opacity) × opacity`. The window keeps its macOS shadow.
- **Opacity steps**: `opacity_increase`, `opacity_decrease` (±5 %) and
  `opacity_reset` (back to 50 %) change `window_opacity` and save it
  (palette *Increase / Decrease / Reset window opacity*; no default keys).
  With `wheel_zoom`, ⌘⇧ + wheel (Ctrl+⇧ + wheel on Linux) steps it too.
- **Window blur** (`window_blur`, 1–64 px, macOS): blurs what is behind a
  translucent window (Warp's private `CGSSetWindowBackgroundBlurRadius`);
  1 is effectively none.
- **Background image**: a theme's `background_image` (path + opacity) is drawn
  under every layer; relative paths resolve against the themes directory.
- **Font family** (`font_family`): the embedded Hack (default) or
  JetBrains Mono, or any installed monospace family; an unavailable name logs a
  warning and falls back to JetBrains Mono. Settings → Appearance lists the
  families whose regular face has outline glyphs of one width (Monaco
  included, bitmap-only faces left out), each previewed in itself.
- **Text and cursor**: `font_size` 8–28, `line_height` 1.0–2.2 (default
  1.2), `window_padding` 0–40 px, `scrollback` 0–1,000,000 lines (default
  10,000); out-of-range values are clamped. `ligatures` (on) turns font
  ligatures on, `cursor_style` (block, bar, underline, hollow,
  double_underline), `cursor_hollow_unfocused` and `cursor_blink` set the
  cursor. Terminal output also takes `font_weight` / `bold_font_weight`,
  `font_features` (OpenType tags such as `ss01`, `zero`, `-calt`),
  `minimum_contrast` (text lifted to a WCAG ratio against its cell),
  `bold_is_bright` and `ambiguous_width` (`wide`: East Asian Ambiguous
  characters take two cells; set per pane from the next output on).
- **Padding per side**: `window_padding` is the left and right padding of
  a pane alone in its tab; `window_padding_left`, `_right`, `_top` and
  `_bottom` (0–40 px) set one side each (unset: `window_padding`, 14 at the
  top, 8 under a full-screen program's last row; the command editor sits on
  the bottom edge). Split panes keep their own 8 px. With
  `alt_screen_padding = false` a full-screen program (alternate screen:
  vim, htop, less) gets no padding at all, in split panes too.
- **Wheel zoom** (`wheel_zoom`, off): ⌘ + wheel (Ctrl + wheel on Linux)
  makes the font a size bigger or smaller per notch (a trackpad: per 40 px)
  and saves it, like ⌘= / ⌘-; ⇧ added steps the window opacity instead.
  Off, those wheels scroll as before.
- **Settings tab** (⌘,): a tab beside the shells, not an overlay; a sidebar
  search finds a setting on any page, a changed setting shows a dot and a
  reset button, selects open a list.

## Notifications & bell

- **Command alerts**: an alert fires when the command's pane is out of view
  (another tab, a hidden pane, or the window unfocused). While Rift is
  focused it is a toast (`notifications.toasts`) in the focused window, also
  for a pane in another, background window; otherwise a desktop
  notification; `notifications.enabled` is the master switch and a sound
  plays when `play_sound` is on. A failed command alerts when
  `notifications.command_failed` is on, whatever its duration; a command that
  ran at least `notify_threshold_secs` (default 10, `notify_on_completion`)
  alerts whatever its exit. Both are set in Settings → Notifications (*A long
  command finishes*, *Long command threshold*).
- **Desktop notifications**: title = who and what (`✓ Finished in 1m12s`,
  `✕ Failed · exit 101 after 2m04s`, `Claude Code · needs your approval`,
  `Codex · task done`, `Password requested`), subtitle = `tab — ~/folder ·
  branch`, body = the command (a failure adds its last output line) or the
  program's message. On macOS Rift.app posts them itself (UserNotifications):
  Rift's icon, banners from one pane stack together, a click brings Rift
  forward on that pane, and a failed command's banner has *Rerun* (only at
  the shell's prompt; a sample from *Try it* never runs). macOS asks for
  permission the first time; Settings › Notifications shows it, with
  *Allow…* or *Open System Settings*. A build that is not an app bundle
  (`cargo run`) falls back to `osascript` (Script Editor's icon); Linux uses
  `notify-send --icon=rift`.
- **Toasts**: at most 2 show at once, for `toast_secs` (default 8, clamped
  2–60).
- **Bell** (`bell`): `visual` (120 ms full-window flash), `audible`
  (`afplay` / `paplay` / `canberra-gtk-play`), `both`, or `none`. A bell in a
  background tab also gives it the new-output status.
- **Dock bounce / urgency** (`request_attention`, off): a bell, or an alert
  for a pane out of view, while no Rift window has keyboard focus bounces
  the Dock icon once (macOS) or sets the window's urgency hint (Linux; the
  taskbar or window manager shows it). A focused window is never
  interrupted.
- `RIFT_NO_NOTIFY=1` suppresses both.

## Tabs, panes, session

- Tabs: create, close, reorder (⌃⇧←/→ or drag: the tab lifts and follows the
  pointer; Esc puts it back), select by number (⌘1–9, ⌘9 = last; hold ⌘
  alone for 300 ms and each tab shows its number in the ×'s place), reopen
  closed (⇧⌘T restores the tab's cwd, its name, colour, pin and group (while the group still has tabs) and its place in the strip, with a fresh shell; stack of 10).
- Tab titles and status: a tab is named for its focused pane's folder
  (`parent/name` when two tabs share the folder name), or the name it was
  given; the program's own title is its tooltip's. It shows one status, the
  most urgent of its panes (needs you › failed › working › done › new
  output), as a ring or dashed when it is another pane's; `tab_style`
  (Settings › Appearance › Tabs › *Tab style*, picture cards) draws it as a
  dot, an underline, or a second line in words. The tooltip card lists
  every pane's path and state.
- Tab settings (Settings › Appearance › Tabs): ⌘T and + open the new tab at
  the end or right after the active tab (`new_tab_placement`); the × shows
  on the hovered tab, always, or never (`tab_close_button`);
  `tab_bar_with_one_tab = false` hides the lone tab (and the vertical rail)
  until a second tab opens.
- Toolbar (Warp's configurable toolbar; `toolbar_left` / `toolbar_right`):
  the file explorer, code review, notifications, command palette and
  Settings buttons go left of the tabs, at the strip's right end, or off.
  *Edit toolbar* arranges them (right-click a toolbar button or the empty
  strip → *Rearrange toolbar items*, or Settings › Appearance › Toolbar
  buttons): drag between the Left and Right buckets, click a hidden one to
  add it at the right end, × or drag out to hide one, Tab / ← → / Delete
  by keyboard; Save writes the config.
- ⌃Tab order (`tab_switch_order`): the strip's order (default), or `mru`:
  the tabs last used first, per window, with a *Recent tabs* switcher while
  ⌃ is held (release switches, Esc cancels; a quick ⌃Tab toggles the two
  most recent tabs); it skips tabs folded in a collapsed group.
- Tab menu (right-click): *Rename tab* (inline), a tab color (six named,
  or *Custom color…*: any `#rrggbb`), duplicate, sync, *Move to new window*,
  *Move to window N*, *Close other tabs*. `cycle_tab_color` steps the active
  tab through none → red … cyan; `[[tab_color_rules]]` colour a tab by its
  directory while it has no colour of its own
  ([configuration.md › Tab colours](configuration.md#tab-colours)).
- **Windows** (⌘N, File › New Window): every window runs in the one Rift
  process (it shares the GPU device, fonts, config, workers, the control
  socket and the session file), opens in about 12 ms, and costs about
  9 MiB (`docs/performance.md`). Its first tab starts where a new tab of the
  window it came from would (`working_directory`), and it opens offset from
  that window. Each window has its own tabs, panes, overlays and focus;
  menu commands, the global hotkey and the control API act on the window
  that last had focus. Closing a window (its close button, ⇧⌘W, or its last
  tab) ends its shells and leaves the others open, after the running-job
  warning (`confirm_close_running`) names its running commands; closing the
  last window quits, and ⌘Q quits every window ("Quit Rift?").
- **Window size and launch mode**: a window opened without a saved frame
  (first launch, `restore_session = false`) is `window_width` ×
  `window_height` (default 1180 × 760 logical px, at least 320 × 200); ⌘N
  takes the size of the window it came from. `launch_mode` opens every new
  window `maximized` or `fullscreen` (`default`: as its saved frame says).
- **Always on top** (`always_on_top`, off): new windows float above other
  apps' windows; *Toggle always on top* (`toggle_always_on_top`, Window ›
  Keep Window on Top) switches the current window, until the setting
  itself changes.
- **Named windows**: *Rename window* (`rename_window`, Window › Rename
  Window…) names the current window in the palette (prefilled with its
  name; nothing typed clears it). The name is its title (the Window menu,
  Mission Control, the taskbar; secrets masked while `mask_secrets` is on)
  and is saved with the session; a quit clears it with the tabs. The
  drop-down hotkey window has no title: both actions say so in a toast
  there.
- **Resize**: `resize_overlay` (off) shows the grid's columns × rows in the
  window's centre while it is resized, for a second after the last change;
  `snap_to_cells` (off) makes a window resize by whole cells (macOS, X11;
  Wayland has no resize steps).
- **Move tabs and panes between windows** (Warp, Windows Terminal): drag a
  tab out of its strip or rail and release it over another window (it lands
  at the slot under the pointer on that window's strip, else last) or
  outside every window (a new window opens there); the tab menu's *Move to
  new window* and *Move to window N: …*, and the pane menu's *Move to new
  window*, do the same. The shells keep running with their scrollback,
  blocks, pane ids and command-line drafts (selections clear); the
  destination window takes focus, and notifications from a moved pane come
  forward in its new window. A window whose last tab leaves closes without
  ending a shell. The drop-down hotkey window gives and takes tabs: one
  moved in drops it down; when its last tab leaves it hides and gets a fresh
  tab, as when the hotkey first creates it. The last ordinary window's last
  tab does not move into the drop-down window. The session file saves the
  new layout, and a relaunch restores it (the drop-down window is not saved).
  A window's only tab does not move to a new window of its own.
- **New tabs start in** (`working_directory`): `previous` (the active pane's
  directory, default), `home`, or a path (`~` expands; a missing directory
  falls back to home). `confirm_close_running` (on) asks before closing a
  pane, tab or window with a running job; the dialog lists each running
  process with its agent badge.
- A fresh session shows a welcome list (split, palette, history, explorer,
  settings, with their shortcuts) until its first command or background
  output; clearing blocks does not bring it back, a shell reattached after a
  restart or update never shows it, and it never covers text the pane shows
  (a shell's startup output).
- Panes: split right/down, directional focus (⌥⌘arrows), keyboard resize
  (⌃⌘arrows), drag resize, zoom toggle (⇧⌘⏎), close (⌃⇧W/⌘W or the title
  bar's × — last pane closes the tab, last tab exits).
- Pane settings (Settings › Appearance › Panes): unfocused split panes wash
  lighter (`dim_inactive_panes`, strength `inactive_pane_dim`, 10 % by
  default); with `focus_follows_mouse` the pane under the pointer (the file
  viewer's too) takes focus as the pointer moves, without a click, while no
  menu, dialog, drag or notifications popover is open.
- Pane sizes follow Warp's pane group: each split holds its panes in a row
  (or column) with a weight each. Splitting along the row adds an equal-weight
  pane to it (⌘D twice: three thirds); splitting across it halves the pane.
  A closed pane's space goes to all its siblings in proportion. ⌃⌘arrows move
  the divider beside the focused pane (after it; before it for the last one)
  10 px that way; dragging a divider trades space between the two panes beside
  it only. Neither leaves a pane under 50 px. A window resize keeps every
  pane's share. Double-clicking a divider gives that row equal shares again;
  `equalize_panes` (palette: *Equalize pane sizes*, View menu) does it for the
  whole tab. Saved sessions keep the shares; sessions saved by older versions
  (nested two-pane splits) restore with the same sizes.
- Pane title bars (split tabs): split, ⋮ and × on the focused or hovered
  pane; drag one to move the pane beside another pane (on the side under the
  pointer), swap it with one (the middle third), into another tab (hold to
  open it and place the pane among its panes), or out into a new tab; the
  shell keeps running and keeps its scrollback. The pane menu adds Maximize
  pane, Move to new tab and Move to new window.
- **Synchronized inputs** (⌥⌘I, tab context menu or Window menu): typing and
  submitting in one pane of a split tab goes to the other panes of that tab;
  the tab shows a sync marker while it is on. Input only reaches panes in the
  same mode: raw keys and pastes go to the panes running a program, each
  encoded for that pane's own cursor-key, kitty keyboard and bracketed-paste
  modes; a command submitted from the editor runs only in the panes at a
  prompt, and a toast counts the panes it skipped. A multi-line paste asks
  first if any recipient lacks bracketed paste. A read-only sibling is left
  out.
- **Read-only panes** (pane menu *Make read-only* / *Allow input*, palette
  "Toggle read-only pane", `toggle_read_only`, no default chord): the pane
  refuses typing (editor and raw mode, the rich-input composer), pastes,
  dropped files, a synchronized sibling's input, *Send to agent…*,
  `ctl send-text` and a paired phone's input (`pane is read-only`), with a
  brief "Pane is read-only" toast. Recalling commands from blocks or the
  history palette also preserves the locked draft;
  moving the caret, selecting blocks (including ⇧↑ / ⇧↓), scrolling
  (the wheel as arrow keys on the alternate screen) and mouse reporting still
  work. Its title bar and tab show a lock; it is kept in the session file.
- **Kept panes and restart** (`close_on_exit`): `always` (default) closes a
  pane when its program exits; `never` keeps it with its last output and a
  bar "Process exited with code N — Restart ⏎ · Close ⌘W"; `graceful` keeps
  it only after a non-zero exit (or a signal: 128 + its number). Enter or
  Restart starts the same launch again in place (profile, program,
  arguments, directory; the same pane id, layout, pane theme and read-only
  state; in the
  session service with durable sessions). *Restart pane* (pane menu,
  palette, `restart_pane`) also restarts a live pane, asking first while a
  job runs in it.
- **Pinned tabs** (tab menu *Pin tab* / *Unpin tab*, palette, `pin_tab`):
  pinned tabs move first, compact (their mark only in the strip, one line in
  the rail's *Pinned* section) and leave any group; ⌘W, × or *Close tab* on
  a pinned tab asks first. Dragging reorders within the pinned tabs, the
  others, or one group.
- **Tab groups** (Warp): ⌘-click tabs to pick them (⇧-click picks a range)
  and choose *Group picked tabs* (*Group tab* for one) in the tab menu: a
  group named "Group N" in the next tab color. Its header (a pill before its
  tabs in the strip, a row in the rail) folds and opens the group on click,
  renames it on double-click (Enter keeps, Esc drops), and right-click offers
  Collapse/Expand, Rename, Ungroup and Close group (which warns about
  running commands as closing tabs does). The active tab is never folded
  away: selecting a tab of a folded group opens it, and folding the active
  tab's group moves to the nearest tab outside it. ⌘1–⌘9, ⌃Tab and ⌃⇧Tab
  count only the tabs shown. A new tab opened from a grouped tab (⌘T, a
  profile, *Duplicate tab*) joins its group; a tab moved to another window
  leaves its group (a pinned tab stays pinned). Pins and groups (name,
  color, folded) are kept in the session file (version 6).
- **Session restore**: every window's tab layout, cwds and frame persisted
  atomically to `~/.local/share/rift/session.toml` on every structural
  change; reopened on launch when `restore_session = true`, each window
  where it was. The first window stays where older versions read it (they
  reopen it alone); version 4 adds the others (`docs/configuration.md`).
  With `durable_sessions`, every window's shells run in the session service.
- **Saved block output** (`save_transcripts`, off by default): each pane's
  finished blocks (command, exit code, duration, directory, output — newest
  200 blocks / 1 MiB) are written 0600 to `transcripts/<id>.json` when a
  command finishes and on every session save. A relaunch replays them as
  ordinary blocks marked **restored** above the new shell; the processes are
  not running. Replay is text only (escape sequences stripped). Turning the
  setting off, closing the pane, or "Clear saved block transcripts" deletes
  the files; nothing is redacted, so keep it off where output holds secrets.
  Each block also keeps when it finished, its branch and its ssh host's name
  (older files without them still load: those are unknown), which the
  history pane's Results scope searches.
  Loading also enforces the 200-block / 1 MiB decoded text budget (including
  commands and directories), with 16 KiB per command/directory and a bounded
  JSON read. Oversized or malformed saved transcripts are ignored.
- **Vertical tabs** (`tab_layout = "vertical"`, Settings → Appearance, or
  "Toggle vertical tabs"): a 220px rail lists every tab with its icon,
  title, directory and git branch, pane count, sync state, status and
  color; click selects, × closes, dragging a row up or down reorders. When
  the tabs do not fit, the active one stays visible and `+N above` /
  `+N more` lines stand in for the rest, marked with the most urgent status
  among them. A search field at its top (click, or ⇧⌘F `focus_rail_search`)
  filters the rows by title, directory, branch, the command a pane runs or
  last ran, and coding agent (Esc clears it, Enter opens the first match,
  in an empty field only lets go; "No tabs match" when nothing does; with
  `mask_secrets` on, rows and the search see secrets masked). The toggle beside it
  (`toggle_rail_panes`) adds a row per pane under each split tab; a click
  focuses that pane. Rows show their repository's uncommitted `+added
  −removed` lines (the git chip's numbers, untracked files counted as added)
  and the branch's pull request number from the PR chip; the rail starts no
  git work of its own.
- **Agent tabs** (Warp's vertical-tab agent badges): a tab running Claude
  Code, Codex, OpenCode or Gemini CLI (recognized from the command line, or
  from the agent's own notification) shows its state as the tab's status:
  working, needs you (it asked for permission or input and nothing was
  typed since; pulses unless Reduce motion is on), done (exit 0, or at its
  own prompt) and failed (non-zero exit). Done and failed stay until the
  pane is viewed. No brand mark: the tooltip names the agent (`Claude Code —
  waiting for you`), and the screen reader hears the status and the agent.
- **Worktree identity**: a pane inside a linked git worktree shows
  `<worktree dir> · <branch>` (rail subtitle, strip tooltip); the main
  checkout keeps `directory · branch`. Identity is the repository's common
  git directory plus the worktree path, so a branch switch updates the
  label without changing the pane. Remote panes are never inspected locally.
- **New worktree tab…** (palette, `new_worktree_tab`, unbound): asks for a
  branch name (suggests the first free `wt-<n>`), runs `git worktree add -b
  <branch> <repo-parent>/<repo>.worktrees/<branch> HEAD` (an argument list,
  no shell, `GIT_TERMINAL_PROMPT=0`, off the UI thread) from the active
  pane's checkout, then opens a tab there. A failure (branch exists, path
  exists, not a repository, invalid name) toasts git's message; a branch git
  created before refusing is deleted again, and anything that could not be
  cleaned up is named in the toast.
- **Session switcher** (⇧⌘O): one fuzzy list of tabs, panes of split tabs
  (directory and running job), the other windows' tabs (`title — window N`;
  focusing one raises that window), tabs of other Rift processes (their
  control sockets), detached session-service shells, and the 40 newest
  finished blocks of this window; Enter jumps there (a block is selected and
  scrolled into view). Other processes and detached shells load in the
  background; the list remains usable while an unresponsive one times out.
- **Hint mode** (⇧⌘H): every visible URL, existing file path and block
  header gets a letter label; typing it opens the link/path or selects the
  block, ⇧ + letter copies it (a block copies its output). Esc cancels.
  Past 26 hints every label is two letters.
- **Send output to a command** (block menu, palette): the block's output is
  previewed, then written to a private temporary file and run as
  `( command ) < file` (fish: `begin; …; end < file`) in the pane. The output
  is never interpolated into the command line. This requires a local,
  writable shell prompt; a running program, remote prompt, closed or
  read-only pane refuses it before any temporary output is written.

## Subshells

Warp's "Warpify subshells". A command that starts an interactive shell
Rift's hooks do not reach — `docker`/`podman` `run`/`exec` ending in `bash`,
`zsh` or `fish`, `poetry shell`, `pipenv shell`, `aws-vault exec`, `flox
activate` (Warp's list), plus your own `subshell_commands` (regular
expressions) — shows a bar at the bottom of its pane 0.8 s after it starts:
*Use Rift's blocks and editor in this subshell?* with **Enable** (⌘⏎;
⌃⇧⏎ on Linux), **Don't ask again** (adds the command to
`subshell_denylist`) and ✕. Enable types one line into that shell: Rift's
bash or zsh hooks (fish's when a word of the command is `fish`) as base64,
decoded with `base64 -d` and evaluated once (a leading space keeps it out
of a history that ignores such lines; one line, because macOS drops typed
input queued between the lines of a multi-line one). The
command's own block then folds (it holds the echo of that line), the
subshell's prompt gets the editor, its commands get blocks with exit codes,
and `exit` returns to the outer shell's prompt. Other shells (`sh`, `dash`)
are left as they are. Plain `bash` and `zsh` typed at a Rift prompt need no
offer: they load the hooks themselves (below), and fish 4 marks its own
prompts. Settings › Input › *Offer Rift in subshells* (`subshell_offer`)
turns the bar off.

## Rich input (composer over a running program)

Warp's "rich input for CLI agents". While a program owns the keys (a
command runs, or a full-screen program is on the alternate screen: Claude
Code, Codex, vim, less), ⌃⇧G / ⌘E (`rich_input`, also in the palette) opens
the **composer**: Rift's command editor docked at the bottom of that pane,
headed *Compose for* and the agent's name (Claude Code, Codex, OpenCode,
Gemini CLI) or the command line. The program keeps running and drawing
above it; its newest rows move up out from under the composer (render only:
the program keeps its size, so it never redraws for it) and its cursor
hides. The composer is the command editor: multi-line, mouse selection,
undo, multi-cursor, vim mode when `vim_mode` is on (Esc leaves Insert first;
in Normal it closes), pasted and dropped files type their paths (Claude
Code and Codex attach an image from its path), an image-only clipboard
reaches the program as an empty paste (they read the image themselves).
Nothing of the shell's applies: no completions, history search, syntax
colors or corrections; ↑ / ↓ recall what was sent from this pane.

- **Send**: ⏎ (⇧⏎ / ⌥⏎ insert a newline); with `rich_input_submit =
  "cmd_enter"`, ⌘⏎ sends and ⏎ inserts a newline. The text goes to that
  pane's program only (never to synchronized panes) as one bracketed paste
  when the program enabled bracketed paste (an embedded end marker is
  removed), else as typed, then Enter (CR). The composer then closes and the
  keys go back to the program. An empty composer sends nothing.
- **Esc** closes it and keeps the draft for that pane; ⌃⇧G again restores
  it. ⌃C clears the draft.
- **The program ends** (its block finishes) or **the pane closes** while
  the composer is open, or while a draft for it waits: nothing is sent
  anywhere; the composer closes and a toast offers the draft with **Copy**
  (it stays until dismissed). A draft never carries over to the pane's next
  program.
- At a shell prompt the action does nothing: the command editor is already
  there.
- **Auto-open** (`rich_input_auto`, Settings › Input): over a coding agent
  (its command names one, as for notifications) that waits for input: its
  own "waiting for your input" notification, or one second of quiet after
  output while its title does not say it is working. Esc suppresses it until
  the agent asks again (its notification), works again (its title), or
  produces new output while you typed nothing into it (the echo of your own
  typing never reopens it).
- `dump` shows `composer=<open|closed> draft_len=N` per pane; the
  accessibility tree names the editor *Compose for …* while it is open.

## Shell integration

Materialized to `~/.local/share/rift/` on first spawn, injected
automatically:

- **zsh** — ZDOTDIR shim that sources the user's real config first (a
  `ZDOTDIR` set in the user's `~/.zshenv` is honoured for their other files).
  The history goes to the user's own `~/.zsh_history` (macOS `/etc/zshrc`
  names it after `ZDOTDIR`, the shim; a `HISTFILE` the user's rc sets wins),
  and lines earlier versions left in the shim's `.zsh_history` move there
  once. The completion server's hidden zsh keeps no history.
  An interactive `zsh` or `exec zsh` typed at a Rift prompt gets the shim
  too (for that command only; the shell keeps the user's `ZDOTDIR`).
- **bash** — spawned with `--rcfile <data dir>/bashrc`, a shim that replays
  bash's own startup files (profile files on macOS, `~/.bashrc` on Linux),
  then sources `rift.bash`. The command-start marker uses `PS0` on bash
  4.4+ and a `DEBUG` trap before that; `PROMPT_COMMAND` may be a string or an
  array (5.1+), and entries a user rc appends after sourcing `rift.bash`
  itself are moved before Rift's last entry before the first prompt.
  `bash` and `exec bash` typed at a Rift prompt start the child with
  `--rcfile <data dir>/bashrc-child` (`~/.bashrc`, then `rift.bash`) through
  the aliases `bash` and `exec ` — not added when the user already has
  aliases of those names. `command bash`, `/bin/bash` and scripts bypass it.
- **fish** — `rift.fish` auto-loaded via a `vendor_conf.d` XDG_DATA_DIRS
  append.

Sets `TERM=xterm-256color` and `COLORTERM=truecolor` (only when not already
set), `TERM_PROGRAM=rift`, `TERM_PROGRAM_VERSION` (Rift's own),
`LC_TERMINAL=Rift` and `LC_TERMINAL_VERSION`, and `RIFT_PANE_ID` /
`RIFT_PANE_URL` (the pane's `rift://action/focus` link, below);
emits OSC 133 (blocks), OSC 7 (cwd tracking) and, when they change, OSC 1337
SetUserVar `RiftPython` (the prompt chips' Python environment) and, when PATH changes, `RiftPhpBin` / `RiftPythonBin` (the
`php` and `python3` it resolves to) and `RiftPath` (PATH itself, for
command-name completion); base64, bounded and control characters
dropped on read (`RiftPath` up to the 4 KiB OSC bound and never cut: a
longer one is dropped and the probed shell's PATH is used). Defines `rift-use`, which the PHP and Python chips run.

## Rendering & performance

- wgpu SDF quad pipeline (rounded rects, borders, circles, soft shadows) +
  glyphon text; sRGB-space alpha blending matching the design mock's CSS
  compositing.
- **Shape cache**: shaped text buffers are content-addressed and reused across
  frames — scrolling only shapes lines entering the viewport; the cursor line
  reshapes alone. Ages out after 300 unused frames past 4096 entries.
  Trailing blank cells are never shaped (backgrounds are quads), which keeps
  each cached row small during output floods.
- **Baseline**: `packaging/bench.sh` measures startup, output throughput,
  scrollback memory, key-to-frame latency and TUI round trips through the
  script driver; recorded numbers live in [performance.md](performance.md).
- **Frame pacing**: frames that program output or the running-command
  spinner ask for come at most every 8.3 ms (120 Hz), so drawing never keeps
  the terminal locked away from the thread parsing the output.
- Scroll input accumulates fractional trackpad deltas — no dropped events.
- Full VT emulation via alacritty_terminal (vim/htop/alt-screen, 10k
  scrollback default; VT420 level, see [Terminal protocol
  support](#terminal-protocol-support)), theme & config hot-reload < 300 ms (a once-a-second stat check backs up
  file-system events, which a backlogged fseventsd can drop).
- Git status (branch, ahead/behind, dirty) resolved on a background git2
  worker; never blocks a frame.
- `dump` reports `scene_us frames=… mean=… max=…`: the time spent building
  each frame's scene, for scripted before/after comparisons.

## Startup

If the `rift` config or data directory does not exist but one from an earlier
name of the app does (`shift`, then `runway`), Rift keeps using that directory.

`Starting zsh…` placeholder on the prompt chips' row until the shell's first prompt; no terminal
cursor shows while it is up. For a shell without integration, the placeholder leaves and the
cursor shows 1.5 s after its first byte. Session restore
respawns tabs at their saved cwds. The shell's history is imported at launch
(and again, replacing the imported history, when `shell` changes in
`config.toml`) for the history pane and the palette's History mode, from the configured shell's own file: zsh
(`$HISTFILE`, else `~/.zsh_history`; EXTENDED format, multiline entries,
0x83 unmetafy), bash (`$HISTFILE`, else `~/.bash_history`; `#<epoch>`
timestamp lines are entry boundaries, so a multiline command is one entry;
without them each line is one) or fish (`$XDG_DATA_HOME/fish/fish_history`, else
`~/.local/share/fish/fish_history`). Only the last 4 MiB of the file is
read. The first launch after an update opens What's New (release notes
from `CHANGELOG.md`); the last seen version lives in
`~/.local/share/rift/last-seen-version`, and a first install only records
it.

## Terminal protocol support

- **Keys** (raw mode): F1–F12, xterm modifier encoding for cursor/editing
  keys, DECCKM application cursor, control codes, Meta (see
  keyboard-shortcuts.md).
- **Kitty keyboard protocol**: programs that push flags with `CSI > flags u`
  (Neovim, Helix, fish, kakoune) get unambiguous `CSI u` keys —
  disambiguation, repeat/release events, shifted alternates, every key as an
  escape code, and associated text. `CSI ? u` queries are answered; programs
  that never opt in keep the legacy xterm bytes. Flags a command set on the
  main screen end with it (its `133;D`): one killed before popping them
  (Codex, Claude Code) no longer leaves the shell receiving `CSI u` codes —
  key releases such as `9;5:3u` — that its line editor kept as text.
- **Synchronized output** (DECSET 2026): a frame drawn between begin/end
  markers appears at once. A program that stalls mid-update is shown after
  150 ms rather than freezing the pane; a shell marker inside an update
  flushes it first so block anchors stay correct.
- **Inline images**: the kitty graphics protocol (APC `G`: transmit, display,
  put, delete, query; delete selectors `a`/`A`, `i`/`I` and `n`/`N` only —
  others are ignored; PNG, JPEG, GIF, WebP via `f=100`, raw RGB/RGBA `f=24/32`,
  zlib `o=z`, chunked `m=1`, source crop, cell size, z-index under/over text,
  `C=1`, quiet levels) and iTerm2 inline images (OSC 1337 `File=inline=1`, with
  `width`/`height` in cells, px, % or auto and aspect ratio). Images anchor to
  the absolute line model: they scroll with output, follow reflow on resize,
  leave with their line from the scrollback, clear with ⌘K, and alternate-screen
  images go when the program exits. Only direct transmission (`t=d`) is
  accepted — file, temp-file and shared-memory transfers would let a program
  (possibly over ssh) make the terminal read local files — and iTerm2
  downloads (`inline=0`) are ignored. Bounds: 32 MiB payload and 40 M pixels
  per image, 320 MiB decoded and 512 placements per pane, 256 MiB of GPU
  textures (least recently drawn evicted). Placements span at most 1,000 cells
  in each dimension; percentage height uses the terminal's screen rows.
  Kitty image numbers expire with their image on replacement, deletion or
  eviction, so a stale number cannot target a new image reusing its ID.
  Invalid or non-finite iTerm dimensions are rejected. Kitty animation frames
  and Unicode placeholders are not implemented. Sixel is not implemented: the
  kitty and iTerm2 protocols cover the TUIs that draw images (yazi, timg,
  chafa, icat, imgcat, viu), and a Sixel decoder would add the largest parser
  surface for the least-used format.
- **Input never freezes the window**: keys, pastes and the terminal's reports
  go to each pane's shell in order from its own queue, so a program that
  stops reading (a paused TUI, a stuck agent) holds only its own input. Up to
  16 MB waits for it; more is refused whole, never half a paste, with a
  notice.
- **Paste**: bracketed paste when the program enables it. Otherwise text with
  a newline asks first ("Paste and run?", its lines with secrets masked),
  because each newline would press Enter (`confirm_multiline_paste`). A
  paste larger than
  `large_paste_warning` KiB (0, the default, never asks) asks first, at the
  prompt too ("Paste a large amount of text?", its size, line count and
  first lines with secrets masked); *Paste* goes on into the pane that was
  active, and the newline question can follow. A read-only pane refuses it
  without asking. `trim_paste` (off) drops trailing blanks and newlines from
  pasted text, so a copied line does not run; copied or dropped files'
  paths keep the space after them. With no text on the clipboard (an image,
  say), ⌘V sends an empty bracketed paste; Claude Code answers it by reading
  the clipboard image. Files copied in Finder (or the Linux file manager)
  paste as their full paths, shell-quoted and space-separated like a drop,
  instead of the bare file name. Image files too: Claude Code and Codex
  attach an image from its pasted path (the clipboard's image of a copied
  file is only its icon); a screenshot copied as an image keeps the empty
  paste above.
- **OSC 52** copy: programs (tmux, neovim, remote shells) can set the system
  clipboard. Reading the clipboard back is refused unless
  `clipboard_access = "read_write"` (Settings › Terminal › *Copy and read*);
  `"off"` refuses copies too.
- **Middle-click** pastes the clipboard into the pane under the pointer (which
  takes focus) unless its program takes mouse clicks (`middle_click_paste`).
  One wheel notch scrolls `wheel_scroll_lines` lines (default 3).
- **Option as Meta** (macOS): the left and right Option keys are set apart
  (`option_as_meta`, `right_option_as_meta`; the right one follows the left
  until set).
- **Colors** (OSC 4/5/10–12, resets OSC 104/105/110–112): set, query and
  reset the 256-color palette, the special colors and the default
  foreground, background and cursor; queries answer from the active theme, so
  programs that probe the background (vim, delta, bat, Codex) pick matching
  colors, in order with the replies around them (`OSC 11;? CSI c` reads the
  color before the DA1 reply).
- **Color scheme** (mode 2031, `CSI ? 996 n`): a program that sets
  `CSI ? 2031 h` (Claude Code) gets `CSI ? 997 ; 1 n` (dark) or `; 2 n`
  (light) when the theme switches between dark and light; `CSI ? 996 n` asks
  for it.
  Color specs take every X11 form: names, `#rgb`…`#rrrrggggbbbb`, `rgb:`,
  `rgbi:` and the device-independent `CIEXYZ:`, `CIEuvY:`, `CIExyY:`,
  `CIELab:`, `CIELuv:` and `TekHVC:` (ported from Xlib's Xcms, with its gamut
  clipping).
- **Window operations** (XTWINOPS `CSI … t`): iconify, move, resize in
  pixels or characters (the window follows), raise, refresh, maximize,
  fullscreen, and reports of the window state, position, size in pixels and
  characters, screen size, icon label and title; the title stack (22/23).
- **OSC 0/1/2 titles** and icon names, title reset; **OSC 7** cwd; **OSC 8**
  hyperlinks.
- **VT420 emulation** (DA1 answers level 4, `CSI ? 64 ; … c`); the vendored
  `vte` and `alacritty_terminal` carry it:
  - left/right margins (DECLRMM, DECSLRM) for cursor motion, wrapping,
    scrolling, insert/delete line and character, and erase; DECBI/DECFI,
    DECIC/DECDC;
  - rectangular editing: copy (DECCRA), fill (DECFRA), erase and selective
    erase (DECERA, DECSERA), attribute change and reverse (DECCARA, DECRARA,
    extent by DECSACE);
  - protected text: DECSCA with selective erase (DECSED, DECSEL), and ISO
    protected areas (SPA/EPA);
  - double-width and double-height lines (DECDWL, DECDHL): each cell draws
    two columns wide, double-height halves at twice the size; selection and
    mouse reports map to the characters drawn;
  - 132-column mode (DECCOLM when `?40` allows it, DECNCSM), DECSLPP page
    length; the window resizes to fit;
  - reports: DECRQCRA checksums, DECRQSS, DECRQM for ANSI and DEC modes,
    presentation state (DECCIR, DECTABSR, restored by DECRSPS), DECRQTSR,
    DECXCPR, the DSR family, DA2/DA3, XTVERSION and XTGETTCAP;
  - DECSTR soft reset, DECSCL conformance levels, S7C1T/S8C1T (replies stay
    7-bit), the DEC locator, NRCS national charsets and the DEC supplemental
    and technical sets, reverse wraparound (`?45` as xterm 383+, `?1045`
    extended), XTSAVE/XTRESTORE, blinking text (SGR 5, 600 ms) and reverse
    screen (DECSCNM).
- **Unicode widths** follow wcwidth, the table ucs-detect measures against:
  VS16 widens a text-default emoji to two cells and draws it in color, VS15
  narrows an emoji to one cell and draws its text glyph, ZWJ sequences, flags
  and skin tones are one cluster, Indic conjuncts take two cells. The command
  editor wraps by the same widths. The tables are generated
  (`scripts/gen-width-tables.py`).
- **IME** composition and committed text in the editor and raw mode.

## Durable sessions

With `durable_sessions = true` (the default; Settings → General → *Keep
shells running through updates*), new shells run in a
per-user **session service** (`rift-terminal --session-daemon`, started on
demand) instead of the window process. On macOS the service and the phone
gateway run as **Rift Helper.app**, an app of its own inside Rift.app
(`Contents/Library/LoginItems`, its own bundle identifier, no Dock icon):
the window asks LaunchServices to start each as a new instance of it (`open
-n -g -j`), so they are not children of the window and a quit Rift leaves the
Dock instead of staying there as "Running in Background" (macOS 27 keeps an
app there while a launchd job it bootstrapped runs). Because it is a
separate app, macOS never takes a helper for the running Rift (a launch of
Rift from the Dock, Spotlight or `open -a` always reaches a window process),
and nothing is registered with launchd, so no build is pinned (a registered
helper of an older build was killed on the first launch after an update:
"rift-terminal quit unexpectedly"). Runs with their own
`XDG_DATA_HOME`/`XDG_CONFIG_HOME` (scripted and test runs), runs outside
Rift.app and a failed `open` bootstrap launchd jobs instead
(`<data dir>/launchd/*.plist`, run once). A pane asks the service on its
own thread, never the window's: starting the service or the gateway, and a
slow or stalled service, never hold the window (⌘T, typing and splits go
on; the pane shows `Starting zsh…` and keeps what is typed). A service that does
not answer a request within 5 s (hello, spawn, attach) counts as
unavailable: that pane starts a local shell in its place with a notice (it
runs what was typed meanwhile), and the rest of the window's run stays
local instead of waiting on it again. A reattach whose shell ended while
detached starts a fresh one in the service.

**macOS privacy.** macOS holds the app that runs a shell responsible for what
the shell touches, so with the service as Rift Helper.app, "Operation not
permitted" in `~/Downloads` or `~/Documents` needs Full Disk Access for *Rift
Helper* (granting only *Rift* helps the window, not the shells). Settings ›
Permissions lists the switches with their live status and a button into the
right System Settings pane: Full Disk Access per app that runs shells (the
helper's status is read inside the service, over its socket; a service older
than this Rift reads `Unknown · restart Rift`; while it is off, `Show in Finder`
reveals Rift Helper.app to drag into the list) and Notifications. The page
exists on macOS only. Releases are signed with one fixed identity, so a grant
survives updates.

**Quitting** ends every tab. ⌘Q, Quit Rift in the menu or the palette, or
closing the last window ends every shell, and the session file keeps only the
window's frame, so the next launch starts fresh with one tab where the window
was (saved block output goes too), whatever *Restore saved sessions on normal
launches* (`restore_session`) says. While a phone is connected, the quit
dialog names it first (*iPhone is connected and loses its tabs.*), with the
running commands when *Warn before closing running commands* is on. The phone
is told Rift quit (`closing`, below). On macOS with Remote access on (and
durable sessions), the gateway and the menu bar icon stay after a quit, so a
phone that can run commands can still open shells; they appear as tabs when
Rift opens again. (Rift 0.17–0.23 kept the shells running after ⌘Q here and
restored the tabs; `confirm_background_quit` and the session file's `kept`
are now ignored.)

*Quit Rift Completely* (the icon's menu, or the quit dialog's note) ends
everything: every shell, the gateway and the session service. The phone is told with `closing` (reason
`quit`): it shows "<computer> quit Rift" instead of retrying.

**Menu bar icon** (macOS, with Remote access on; it also lets phones open
tabs while Rift is closed): the app icon as a line drawing in the menu bar
(a template image, tinted by macOS). While Rift runs its menu has *New Window*, *Settings…* and
*Quit Rift Completely*. After the window quits, the phone gateway shows it
for as long as it runs (`remote_access`; until *Quit Rift Completely*),
with *Open Rift* and *Quit Rift
Completely*. *Quit Rift Completely* is the usual quit (the same warnings)
that also stops the gateway (`<data dir>/remote/stop`, seen within 0.5 s)
and ends the shells phones started in the session service, which then
exits; nothing of Rift keeps running. ⌘Q leaves the gateway as before.
*Launch at login* (`launch_at_login`, beside it) adds or removes Rift's
login item. Opening Rift again while only its helpers run (the Dock icon,
Spotlight, Finder or the icon's *Open Rift*) starts a window: the helpers
are a separate app (`Rift Helper`), so the launch always reaches Rift itself.

An update's restart, a crash or a killed window process leave the shells
running while the session service survives; the next launch
restores the layout and **reattaches** each pane (after a crash whatever
`restore_session` says, with a toast: *Rift didn't quit normally last time.
Your tabs are back.*; every ordinary way out removes `<data dir>/running`,
and a leftover one whose process is gone is what says it crashed): the service replays the output it kept (the newest 8 MiB per
shell) into a fresh terminal, with the original timing so block durations
stay right, then continues live. Nothing is shown twice, and automatic
terminal replies (device attributes, image OKs) are not re-sent during the
replay. Commands that finished before the reattach keep their own finish
time and do not appear again in Notifications.
A computer reboot ends the service and its shells: the saved layout starts
fresh shells, with saved block output when enabled.

- Closing a pane (⌘W) ends its shell, as without the service. ⌘K also drops
  the retained output, so a reattach does not bring it back.
- Turning off `restore_session` does not prevent an update restart or crash
  recovery from bringing the tabs back. A quit ends the shells and clears the
  tabs either way.
- Palette → "Attach detached session: …" opens a tab on any shell the
  service keeps that no window shows.
- The service listens on `~/.local/share/rift/sessiond/sock` (a 0700
  directory; other users' connections are refused) and exits 10 s after its
  last shell and window are gone. A slow window is disconnected rather than
  buffering without bound; the pane reattaches on its own and replays only
  what it missed (it closes only when the service says the shell is gone). A
  service that does not answer is retried for 30 s; after that the pane
  stays with "Lost the session service; the shell may still be running" and
  *Reconnect* (⏎), which reattaches the same shell — it is never closed or
  hung up for it. Input typed
  meanwhile is held (up to 64 KiB) and sent, in order, once reconnected; a
  toast says so if some of it did not fit, or if output produced meanwhile
  had already left the 8 MiB the service keeps. A lock file
  next to the socket keeps concurrent starts from replacing each other's
  socket. If the configured data
  directory is too long for a Unix socket path, the `sessiond/` and
  `control/` directories both move to `/tmp/rift-<uid>-<hash>/…`, specific
  to that data directory.
- If the service cannot start, the shell runs locally and a toast says so.
- Scripted runs (`--script`) keep their shells local unless
  `RIFT_PERSIST_SESSION` is set.

## Updates

Rift looks for a new release at launch and every 6 hours
(`check_for_updates`, Settings → General → *Updates*): it fetches
`latest.json` from the public
[rift-releases](https://github.com/botjaeger/rift-releases) repository with
the system `curl` (https only, 15 s, at most 64 KiB) and remembers the answer
in `update.json`, so several windows check once.

The manifest must be signed: Rift also fetches `latest.json.sig` and
verifies its Ed25519 signature over the manifest's exact bytes against the
release keys built into the app (the private key stays with the maintainer,
never in CI or the releases repository) before reading anything in it. A
missing or invalid signature offers no update and Settings → About shows
*Couldn't check for updates* (the log says why). The cached answer in
`update.json` is re-verified whenever it is read, so editing that file
cannot inject a release either. The asset's SHA-256 then comes from the
signed manifest.

- A newer version downloads in the background first (verified like the
  install below, into `updates/ready.dmg` or `ready.AppImage`; two windows
  fetching at once never leave a half-written file there). Nothing shows
  while it downloads except Settings, which reads *Downloading vX.Y.Z…*.
  Once it is here, a notification card says *Rift vX.Y.Z is ready —
  Downloaded · click to restart into it* (top right, with the app icon; it
  stays until clicked or dismissed; a desktop notification instead while
  another app is in front; with in-window notifications off,
  `notifications.toasts`, only the chip says it and Rift's other update
  messages become status toasts), once per version, and **Update Rift** shows in
  the tab bar (left of the inbox; tooltip `vX.Y.Z available`). A copy that
  cannot update itself has nothing to download and is told at once
  (*Click to see how to update*); a download that fails is offered anyway
  and the install downloads it. Settings → About shows *Up to date*, *vX.Y.Z
  available* with an **Update…** button, *Installing vX.Y.Z…*, or
  *Couldn't check for updates*. The palette has *Check for updates*
  (`check_for_updates`; the answer is a card: *Rift is up to date*,
  *Couldn't check for updates*, or the ready card once it downloaded) and
  *Update Rift* (`update_rift`).
- The chip, **Update…** and `update_rift` open a dialog that says what the
  restart keeps: with durable sessions, *Your N tabs and M shells keep running
  and come back where they were*; without them, open shells end (set
  `durable_sessions = true` in config.toml to keep them). When the new
  version's session service speaks another protocol, running shells keep
  running but cannot be reattached; the dialog says so. **Later** (Esc)
  hides the chip until a newer version.
- **Update** installs the download made ahead (its SHA-256 checked again),
  or downloads this computer's asset now (macOS: the DMG for its
  architecture; Linux: the AppImage) from the public releases only (any
  other URL in the manifest is rejected), checks its size (512 MiB at most)
  and its SHA-256 against the manifest (a mismatch changes nothing), checks
  that the new binary reports the expected version, and swaps it in
  atomically (the app path is never missing): macOS keeps the old bundle
  as `.Rift-old.app` beside `Rift.app`, Linux as `<AppImage>.old`. Only
  one window installs at a time (*Another window is updating Rift*);
  closing or quitting the window meanwhile waits for the update (*Rift is
  updating… it quits when the update is done*). Leftovers of an
  interrupted install are cleared at the next one.
- Every window restarts: Rift writes the rollback marker, saves every
  window's layout to its restart file and starts the relaunch helper (any
  failure there puts the old version back and changes nothing else), then
  asks each other Rift process (a separate launch, or a window of a version
  that ran one process per window) over its control socket to save its
  layout and quit, and quits with durable shells still running; the new
  version reopens every window in its one process, tabs, splits and
  directories included, and reattaches the shells. Processes whose control
  socket is off cannot be asked; the dialog counts them and they stay open
  on the old version.
- The new version counts as started 10 s after its first frame (30 s after
  startup if no frame is presented): it then
  deletes the old copy, shows the card *Updated to Rift vX.Y.Z* (a click
  opens What's New) and opens the other
  processes' windows beside its own. If it crashes before that, the
  helper puts the old version back and starts it (*Couldn't update to
  Rift vX.Y.Z*), and that one reopens the windows. Quitting it (or closing
  its last window) before then keeps the new version; the other windows'
  layouts reopen at the next launch. A crash mid-update
  leaves the layouts in `restart/` (files of older versions, one per
  window, included); the next plain launch reopens them as its windows.
- Updating in place needs Rift.app in a writable folder (macOS) or an
  AppImage in a writable folder (Linux). A copy running from the disk
  image or from macOS's temporary translocated copy says *Move Rift to
  Applications to update it*. Development builds, the `.deb` and the
  tarball get **Open download page** instead.

## Control API

The Rift process listens on `~/.local/share/rift/control/<pid>.sock`
(`control_api`, Settings → General → Automation). It answers for every
window it holds: `list` and `status` return every window's tabs, window by
window, each with `index` (counting from 0 across windows) and `window`
(from 0, in opening order; left out for the first, so older clients read
one window); a request naming a pane acts in that pane's window,
`focus --tab N` raises the window of tab N, and `split`, `open` and the rest
act in the window that last had focus. A drop-down hotkey window is listed
in `windows` (`{"window": N, "kind": "dropdown"}`); `focus` on one of its
panes drops it down.

- `"navigate"` (default): `list`, `status`, `focus`, `blocks` (a pane's
  last command blocks, read-only: up to 30 per request, each output cut to
  its last 500 lines and 64 KiB; `--before ID` pages back), and
  `prepare-restart`, which an updating window sends to the others (save
  the layout, keep durable shells, quit). It must carry the token from that
  update's marker (`update-pending.json`, readable by you only, under 60 s
  old); without it, or while the receiving window is updating itself, it
  is refused and the window stays.
- `"full"`: also `split`, `open` (optionally running a command) and
  `send-text` (types into a shell).
- `"off"`: no socket. Any unrecognised value (`"false"`, a typo) is
  treated as `"off"` and logs a warning. While `remote_access` is on, the
  socket stays open for the remote gateway's requests only: `status`,
  `blocks`, `screen`, `output`, `menu`, `diff`, `complete`, `remote-input`,
  `remote-open`, `remote-close` and `remote-status` (the gateway's notice
  that a phone connected or left).
- `remote-input` (types a command at a pane's prompt, sends keys, or types
  text into a running program), `remote-open` (a new tab or split next to a
  pane) and `remote-close` (closes a pane or its whole tab, without the
  running-command dialog: the phone asked its user first; the window toasts
  "<device> closed a pane") are accepted only while `remote_access` is on,
  whatever `control_api` says.
- `blocks` with `"meta": true` leaves the output out.
- `ctl focus --tab N` / `--pane ID`: tabs count from 0 across windows, as
  in `ctl list`; a non-numeric value is rejected with exit code 2.

`rift-terminal ctl [--pid PID] list|status|focus|blocks|split|open|send-text|request JSON` is
the client (`ctl help` for flags); the newest Rift process answers unless
`--pid` names one. `rift-terminal --version` and `--help` print the version and usage; requests are newline-delimited JSON, answered on the
UI thread. The session switcher (⇧⌘O) uses it to list and focus tabs of
other Rift processes. Permission changes from Settings or config reload revoke
existing connections and queued requests before execution. Connections are
limited to 32, with five-second read/write inactivity timeouts; request lines
are bounded to 1 MiB while reading.

## Remote access (phone companion)

Off by default. **Settings › Remote access › Allow remote access**
(`remote_access`) starts the gateway (`rift-terminal remote --managed`, one
per user, log in `<data dir>/remote/gateway.log`); turning it off stops it
within about two seconds, and it also exits 30 seconds after the last Rift
window closes. It listens on `remote_listen` (`0.0.0.0:7878`). The page
opens on one card with the switch; while on, it shows *On this Wi-Fi* (the
listening addresses) and *Away from home* (the relay: its round trip while
connected, or the whole error, a retry countdown and *Try again*, which
retries at once). While off, the page shows only the card and the paired
phones (to revoke one). With `mask_secrets` on,
every screen, block output and block command the gateway sends is masked as
on the desktop (also for shells a phone started while no window is open).

- **Pairing.** *Pair a phone* shows the code on the page: a QR code, the
  `rift-remote://pair?…` link with *Copy link*, *Cancel* (the code stops
  working at once), and a 2-minute countdown (then *The code expired* ›
  *Show a new code*). The link works once. *The new phone can only watch*
  makes the next link pair a device that can watch but not run commands or
  send keys (changing it withdraws a shown code).
- **Paired phones** lists each phone with how it is connected (*Connected
  on this network*, *Connected through the relay*, or *Not connected* with
  when it was last seen), its platform and pairing date, a *Can run
  commands* / *View only* choice (raising a phone to run commands asks first; its connections close
  and reconnect with the new permission within a second, and a lowered
  permission applies at once), and *Revoke*, which asks first ("Revoke
  <name>? It disconnects now and cannot reconnect without pairing again."),
  then closes its connections within a second.
- **Recent connections** (folded away until opened) lists the newest 20 connections and failed
  attempts: device (or *Unknown device*), via LAN or Relay, address, start,
  duration or *connected*, and why it ended (`remote history` shows the
  newest 500). Five show at once; the list scrolls through the rest.
- **Relay** (outside your network): a `wss://` relay URL (`remote_relay`)
  makes the gateway also reachable through it, and pairing links carry it.
  It defaults to Rift's public relay, `wss://rift-relay.playhaws.com`, so a
  phone reaches the computer from any network with nothing to set up; an
  empty value turns relaying off (phones then connect only on the local
  network). The gateway contacts the relay only while remote access is on,
  and scripted runs (`--script`) skip the default relay
  (`RIFT_NO_DEFAULT_RELAY`, inherited by the gateway they start).
  Its address and the Cloudflare tunnel sit under *Connection settings*;
  its state shows in the card's *Away from home* tile.
  The relay only forwards encrypted traffic; host one with `rift-terminal
  relay` or `packaging/relay.Dockerfile` (behind a TLS proxy such as Caddy).
  A Tailscale or other private-network address of the computer works
  without a relay: it is listed in the pairing link like any other address.
  Pairing links made in Settings carry the running gateway's relay too.
- **Tabs while Rift is closed** (macOS; part of `remote_access`, needs
  `durable_sessions`, on by default): the
  gateway stays up after the last window closes, and a paired phone that may
  run commands can open a tab (after Face ID or the passcode on the phone).
  The shell runs in the session service; the computer shows a notification
  naming the device. The phone follows and drives it like any pane. The next
  Rift window shows these shells as tabs (with a toast) and the gateway lets
  go of them. At most 8 at a time.
- **Copy to phones** (`remote_clipboard`, *Copy to phones too*, off by default): what a program copies with OSC 52 also reaches the phones that may operate. A phone's tap is a click for a program that tracks the mouse, its several lines reach the program as one paste, and a window in use keeps its size, focused and touched within the last minute (`fit`): a full-screen program is captured at the phone's size for the phone alone, about every 2 s at most once it pauses, and the live frame shows otherwise. A window left alone fits the phone live until it is touched again.
- **Cloudflare tunnel** (`remote_tunnel`, *Use a Cloudflare tunnel instead*; no server needed): the gateway
  runs a relay on a loopback port of this computer and gives it a public
  address with a Cloudflare quick tunnel (`cloudflared tunnel --url`,
  free, no account), then registers there instead of at `remote_relay`.
  Cloudflare and the relay see only Noise ciphertext, and only this
  computer's gateway may register at that relay. Settings checks for
  `cloudflared` (PATH, `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`;
  `RIFT_CLOUDFLARED` names another) and offers *Install* (`brew install
  cloudflared` in a new tab) or *Download* (Cloudflare's page); the gateway
  starts the tunnel by itself within 5 s of the install. The address is
  used 20 s after cloudflared prints it (so the new name resolves), and
  "Pair a phone" waits for it. It is new on every start: phones pair again
  after a restart. A tunnel whose cloudflared exits is restarted after
  30 s; cloudflared ends with the gateway, even a killed one. State:
  `<data dir>/remote/tunnel.json`.
- **Indicator.** While a phone is connected, every window shows a
  *Remote · <device>* chip (*Remote · N devices*) in the tab bar; its
  tooltip names the devices and a click opens the Remote access page.
- **What a phone can do.** See windows, tabs and panes; follow a pane's
  blocks live, page back through older ones, and resume after a dropped
  connection without a gap; get notified when commands finish. An
  `operate` device also runs a command at a pane's prompt (it becomes a
  normal block and history entry; a pane that is running something answers
  *busy*) and sends keys (Ctrl-C, Esc, Tab, arrows, …), which reach the
  program when one is running. Each one shows a toast: "<device> ran <cmd>"
  or "<device> sent ctrl+c". Page Up and Page Down at a prompt do nothing
  (the phone scrolls itself), and keys from a phone skip vim mode.
- **Prompt chips on the phone.** The phone's command line shows the chips
  the desktop draws above that pane's command line (*Edit prompt*'s
  `prompt_chips`, in that order, only those with something to show, none in
  shell-prompt mode) in the desktop's colours, each in its short form (a
  folder's name, the branch with its dirty dot, `102 +2.1k −970`, `#42`, a
  version without its patch, `3/4`), on one row: what does not fit folds into
  *+N* (a dot for each folded Docker project with a container down or
  unhealthy), which opens *This pane*, every chip with its full value. A tap
  opens the chip's menu: the folder, branch and changes menus; the pull
  request in the phone's browser; Python and PHP's versions (a pick runs the
  same `rift-use …`); Docker's containers (a pick follows that service's
  logs, the same command as the desktop's menu); Time on tab with *Reset*
  (`operate` devices; it resets that tab's clock as the card's button does).
  User, Host, Date and Time just show. Picks run as the command line's Run
  does, so a pane running a command answers *busy* and the phone's *Ask
  before running* applies.
- **Full-screen programs and typing (Phase 3).** A pane on the phone shows
  blocks only; while it shows the alternate screen (vim, less, htop) the
  phone shows the pane's screen with its colours instead ("htop is full
  screen · showing its screen"), and the blocks again when the program
  ends. There an `operate` device
  types into the running program (the soft keyboard or a hardware one:
  ctrl/alt combinations, Esc, Tab, arrows, F1–F12, Delete), shown on the
  desktop as "<device> typed into a program", never with the text. Typed
  text never reaches a pane at its prompt (nor one whose command was just
  submitted), and keys at a prompt never recall history, complete or submit
  a line that has text: commands still go through *Run*. A shell without Rift's integration, and a prompt drawn on
  the alternate screen (tmux), look like programs and take typed text. Their TUI shows the pane
  the way Rift draws it (a header row per command, not the shell's prompt
  echo), and the phone can size the pane to its width while it follows it:
  the shell is the smaller of the two sizes, so programs reflow, until the
  phone leaves the pane or disconnects, or 30 seconds pass without a
  refresh. With a hardware
  keyboard, ctrl/alt combinations, Esc, Tab and F-keys pressed in the
  command field go to the pane too.
- **Scrolling and pasted images from the phone.** A drag on the pane's
  screen scrolls it as the desktop's mouse wheel over it would: wheel
  reports to a program that tracks the mouse (Claude Code's fullscreen
  renderer, htop), arrow keys under alternate scroll (less), else the
  phone's own view of the pane: the desktop's view stays where its user
  left it, and the desktop's scrolling never moves the phone's. A
  view-only phone scrolls a view, never a program. *Paste* (on the key row
  wherever images can go, whatever Settings › Keys holds) puts it in the
  phone's message box; *Send* sends an image from the phone's clipboard (an operate phone, a running program, at most
  10 MB; PNG, or JPEG for a photo too large as PNG) saves it under
  `<data dir>/remote/uploads/` (kept 24 hours) and pastes its path into
  the program as dropping the file does, so Claude Code and Codex attach
  it; the desktop toasts "<device> pasted an image". One image goes at a
  time (the phone says while it is sending). A file on the phone's clipboard
  (a Files copy on iOS, a content URI on Android; feature `upload_files`)
  goes the same way with any bytes up to 10 MB, saved under a name the
  computer makes with the file's extension (1 to 8 letters or digits; other
  names are refused, and the rest of the name stays on the phone), 0600 and
  never run; the toast says "pasted a file". At a shell prompt an image or
  file is refused with a message. Nothing pasted reaches the program before
  *Send*: *Attach* (the message box's +) or *Paste* puts an image, a file or a
  long text (more than 3 lines or over 4 KB) in the box as a token at the cursor,
  `[Image 1]`, `[File note.pdf]`, `[Text 12 lines]`, and the user types around
  it. Deleting part of a token deletes all of it. *Send* sends the box in order,
  then Return, one image or file at a time as above, waiting for Claude Code to
  show each image before what follows it; once begun it finishes even if the
  screen is left, and a failure keeps what did not go in the box and names it. *Paste* under a full-screen program that has its own
  keys (vim, htop) opens the message box under the screen. A coding
  agent's pane always has the phone's message input, whichever screen it
  draws on.
- **Block actions, earlier output, control keys (Phase 3).** A tap on a
  block opens its sheet: it runs the command again, puts it in the command
  field (*Edit*), or copies the command or its output; a running block's
  sheet offers *Stop* (ctrl+c), and its card the *Notify when done* bell
  (on by default for commands run from the phone). A
  block cut to its last 500 lines loads earlier output 500 lines at a time
  (up to 5,000 more; lines gone from the desktop's scrollback say so). The
  row of control keys is chosen, ordered (drag) and added to in Settings ›
  Panes › Choose keys; its *+tab* and *+pane* keys open a new tab in the pane's
  window or split the pane on the desktop (starting in the pane's folder,
  with a toast naming the phone), and the phone goes to the new pane.
  *New tab* (the Tabs sheet, a computer's card on Portals) and *New pane* (the
  Panes sheet) open sheets that choose what the new tab or pane starts with: a
  shell, or Claude Code, Codex or OpenCode. A program is an `open`, then a
  `run` of its name in the new pane, once its shell shows a prompt (the
  computer answers `busy` until then; the phone tries again for a few seconds);
  *Ask before running* and Face ID apply as to any run. View-only phones get
  neither button. In the Tabs sheet (whole list, no filter) a long press lifts
  a tab and dragging it over the others reorders the desktop window's tabs
  (the tab bar follows, ⌃⇧← / ⌃⇧→ order; a pinned tab stays among the pinned
  and a grouped one in its group, other windows' places are refused); the
  Panes sheet does the same for a tab's panes, each pane exchanging its place
  in the split layout with the ones it passes. Screen readers get *Move up* /
  *Move down* on each row. A desktop without the `move` feature, and
  view-only phones, show no drag.
- **Phone-first app.** *Portals* is the home page (no tab bar); its gear
  opens *Settings* and its **+** pairs a computer. *Portals* lists each
  computer in a card (connection line, *New tab*, *Try again* or *Pair again*;
  swipe the card left for *Unpair*, which asks first) with its tabs, each with its state and time (*Needs you · 2m*, *Working
  3:56*, *Idle 12m*); chips (*needs you*, *working*, *idle*) filter the tabs. With
  one computer paired, an agent's question answers in place on its tab. A cold
  start reopens the pane shown last. A pane's header title opens the *Tabs*
  sheet (every tab of the computer, filtered by state) and its pane picker
  the *Panes* sheet, the one way to move between the tab's panes (a swipe
  would fight with output that scrolls sideways; a screen reader has *Next
  pane* and *Previous pane* on the picker).
  Successes before the newest failure fold away; blocks are cards with a thin
  border in their state's colour (a table as a list, with *Raw*), failed ones
  with *Run again* and *Copy error*, running ones with *Stop* and *Notify when
  done*; a tap on a card's header opens the block sheet, and selecting its output copies the selection (a read-only field; the system menu still shows). There is no view switch: the pane
  shows its screen (the TUI, with pinch-to-zoom) for the alternate screen, a coding agent or a program no
  block holds, and blocks for everything else. Above the command line: the
  folder and branch chips opening the chip menus, suggestions, the key row (⌃;
  sticky ⌃ ⌥ ⇧, Paste) and a round field with a run button; commands run from
  there, there is no separate Run page. The phone keyboard's Return is a new line in the
  field and only the round *Run* (or *Send*, to a program) button sends; with a hardware
  keyboard Return sends and Shift+Return is a new line. A command is one line, so a field with
  line breaks says so and *Run* stays off. *Run* sends at once; Settings ›
  Running commands › *Ask before running* (off by default) shows the
  confirmation first. Repeated taps while authentication or a send is pending
  send the command only once. Leaving a command or answer screen while
  authentication is pending prevents the send, even after returning to that
  screen. A command already sent keeps running; its late acknowledgement
  preserves the screen you returned to. Pending tab, pane and pairing actions
  also leave your subsequent navigation alone. Cancelling confirmation opened
  from a block or chip returns to that sheet; a successful run returns to the
  pane. *Wrap long lines* (Settings › Panes, and the pane's ⋯
  menu for one pane) off lets each block's output scroll sideways. The
  header's search filters the blocks. The folder chip opens what the
  desktop's chips do (feature `chips`): folders (`cd`), local branches
  (`git checkout`) and the code review panel's changed files, each with its
  diff; the names are shell-quoted; it shows the folder and branch the
  desktop computes for its own chips (feature `git`).
- **Needs you (answering from the phone).** The desktop tells the phone
  when a pane's running command waits for the user (feature `waiting`): a
  line read without echo (`sudo`, `ssh`, `read -s`: the terminal's echo
  off in line mode, read from the PTY, or the session service for durable
  sessions; or a cursor line that reads like a password prompt), a coding
  agent's menu on the screen's last rows (Claude Code and Codex: the
  question, the entries under the `❯` / `›` selector, numbered or not, and
  up to 12 lines above the question), or an unread "needs your input" for
  the pane in the attention center (until anything is typed into the pane).
  An `operate` phone answers with arrow keys from the selected entry and
  Enter, "No" can carry what to do
  instead, and a password is typed (never shown, logged or kept on either
  side). Each answer is ordinary input: the desktop shows its toast and the
  pane goes on. The phone alerts when a question starts (Yes / No / Open on
  the lock screen) and docks the question at the bottom of the pane (Yes, the
  other entries, *No…* with a reply field, and the keys Esc, ⇧Tab, ↑, ↓).
- **Completion and history from the phone.** A pane's suggestions ask the desktop (feature `complete`) for what its
  command line would complete for the typed line (the same completion
  providers, each result the whole line with its kind) and for the
  commands that ran in the pane before (its blocks newest first with when
  they finished, then its command-line history and the imported shell
  history), bounded to 50 each.
- **Running list.** With feature `running` each pane running a command
  carries when it started and the screen's latest line, so the phone's
  lock-screen Live Activity shows the elapsed time and the
  last line without following every pane. A coding agent (Claude Code,
  Codex) also sends its name for the session (its title without the status
  mark) and whether it waits at its own prompt, so the phone titles the
  session by it and tells an agent that waits from one that works.
- **Security.** Traffic is end-to-end encrypted (Noise XX) and the phone pins
  the desktop's key; see [mobile-threat-model.md](mobile-threat-model.md)
  and the wire format in [remote-protocol.md](remote-protocol.md).
- **CLI.** `rift-terminal remote pair [--view]` prints the QR code and link
  in the terminal, `remote devices` lists devices, `remote revoke ID`
  revokes one, `remote permission ID operate|view` changes one,
  `remote history [--limit N]` lists connections, `remote [--listen ADDR]`
  runs a gateway in the foreground, and `rift-terminal relay [--listen
  ADDR]` (or the standalone `rift-relay` binary) runs a relay
  (`0.0.0.0:8790`; `--trust-forwarded` behind a proxy).
- **The app** (`mobile/`): tries the computer's own addresses first and the
  relay second, reconnects at once when the phone changes networks (moving
  home upgrades to the direct path), retries an unreachable computer 5 times
  (0.5 s doubling to 10 s; the card counts *retry 2 of 5*) and then stops
  (*Can't reach it · stopped after 5 retries*, with *Try again*): it tries
  again every 30 seconds while the app is open, when you tap *Try again*, when
  the phone is back online and when the app opens, and *Cancel* on a pane that is reconnecting stops it at once and
  goes to Portals. When the computer quits Rift it says so first (`closing`,
  `docs/remote-protocol.md`): the card reads *Rift quit on this computer*
  with *Try again*, and it tries every 30 seconds while the app is open; when an update restarts Rift it
  reads *Updating Rift · back in about a minute* ("Nothing is wrong"), and
  the app reconnects every 3 s for up to 3 minutes without counting those
  tries against the 5. It shows *Offline* without retrying while
  there is no network, closes its connections 30 seconds after going to the
  background (3 minutes while a "notify me" command runs; Android 15+ and iOS
  may cut them sooner, and it reconnects on return), and can require
  Face ID / fingerprint on launch, after 5 minutes away, and (with *Ask
  before running* on) before running commands. With a push token it hands the computers, they alert through the
  Expo push service while the app is closed (`mobile/README.md` › Push).
  Settings › Notifications › *While the app is closed* › *Alerts through
  ntfy* also posts them to a random ntfy topic (ntfy.sh
  or self-hosted), which the free ntfy app shows: no push setup needed.
  If saving its server address fails, the typed address stays available to
  correct or retry; the previous saved server remains in use.
  Settings › Notifications › *My commands* (on by default) asks to be
  told about every command run from the phone. On the first request, the
  notification permission dialog finishes before confirmation closes;
  declining notifications does not cancel the command. For other commands, *Other
  commands on your computers* chooses *Nothing*, *Failures* or *Everything*
  and *Only if it ran longer than* how long a command must run (10 s – 5
  min); each computer's page (Settings › Computers) and the pane's ⋯ menu
  have a *Notifications* switch that turns its notifications off. "Notify
  me" always notifies. A computer's notification preference changes only
  after the phone saves it; a failed save shows an error and leaves the
  control available to retry. A failed Unpair also leaves a retry path.
  Settings keeps an unsaved phone name available after a failed save, and a
  completed save never replaces a newer edit still being typed.

## Progress reports (OSC 9;4)

Programs that report progress the ConEmu / Windows Terminal way
(`ESC ] 9 ; 4 ; st ; pr` ended by ST or BEL — cargo, winget-style tools,
systemd, Claude Code) show it on their tab (*docs/ui.md*, tab strip) and, on
macOS, as the Dock tile's badge. `st`: 0 removes it, 1 is normal with `pr`
0–100, 2 is an error, 3 indeterminate (no `pr`), 4 paused (`pr` optional).
`pr` above 100 counts as 100; an error or paused report without `pr` keeps
the pane's previous value. Anything else (another state, a non-numeric or
signed value, extra fields, a sequence past the 4 KiB OSC bound) is ignored.

- **Clearing**: a report stands until state 0, until the running command
  finishes (OSC 133;D), until its pane closes, or — only when it was set
  while the alternate screen showed — until the program leaves the
  alternate screen.
- **Several panes**: the tab shows the most severe report (error, paused,
  then normal and indeterminate alike), the newest of equals.
- **Dock badge** (macOS): the percentage of the focused window's active tab,
  `!` for an error, nothing for indeterminate or without a report. Linux:
  no taskbar progress.
- `show_progress = false` hides the bar, the badge and the accessible
  label's progress (hot reload); panes still track reports (`dump` lists
  each pane's `progress=<state>:<pct>`).

## Notifications (attention center)

The inbox button in the top bar (or ⇧⌘A) opens a popover listing, newest
first: commands that finished while their pane was out of view or the window
unfocused, every failed command, programs' notification requests (OSC 9,
OSC 777 `notify`, OSC 99), coding-agent notifications, password prompts in
background panes, and bells from background panes — each with the agent,
the pane's git branch at the time, the title, a status line and its age.
Chips filter to all, unread or errors. A click or ⏎ jumps to the pane and
selects the block. The list keeps the newest 200.

- **Read state**: an entry is unread until you open it, look at its pane, or
  press `Mark all as read`. Unread entries put an accent dot on the inbox
  button and a kind-colored dot on their tab.
- **Alerts** (Settings → Notifications, `[notifications]`): for a pane out
  of view, an enabled event also raises a toast (Rift focused) or a desktop
  notification (Rift in the background), and plays its sound. Events: long
  command finished, command failed, agent task completed, needs attention,
  password prompt. Toasts show over the Settings tab too (only the open
  inbox hides them). Settings › Notifications › *Try it* raises a sample of
  each kind as a toast or a desktop notification.
- **Coding agents**: Claude Code, Codex, OpenCode and Gemini CLI are
  recognized from the running command; their notifications show the agent's
  badge (the Claude mark for Claude Code). No standard says a program is
  busy, so an agent works (its tab spins) only while it says so: its title
  leads with `◐ ◓ ◑ ◒` (Claude Code), a braille spinner frame (Codex) or `✦`
  (Gemini CLI), or it reports OSC 9;4 progress (normal or indeterminate).
  Otherwise it is idle at its own prompt: Claude Code's `✳ …`, Gemini CLI's
  `◇ …`, Codex's bare project name, OpenCode's `OC | …` (which never says it
  works). A full-screen program (vim, htop, less) spins only on the same
  signals; a plain command spins while it runs. The wording sorts notifications into waiting (permission, approval,
  input), error, or task completed. Words match whole (`incomplete` is not
  `complete`); an explicit request (`needs your permission`) wins, then
  error words (`permission denied`, `failed`) beat a bare `permission`, and
  a negated word of any kind does not count (`no errors`, `0 errors`,
  `This is not an error`, `Not done yet`, `No approval needed`).
  Setup per agent:
  [configuration](configuration.md#notifications--bell).
- **Password prompts**: a running command in a background pane whose cursor
  line ends like `Password:` or `Enter passphrase for key …:` raises one
  alert per command.

## File explorer and code review

⇧⌘E opens the file explorer: the active pane's project (git working tree,
else its directory) as a lazily expanded tree with ignored files hidden
(dotfiles too with Settings › Files › Show hidden files off) and
git status colors. A folder that cannot be read shows the error (e.g.
`Permission denied`); one with more than 2,000 entries shows the first
2,000 in display order, then a `Show 2000 more entries` row (click, or
arrows then ⏎) that adds the next 2,000 in the same order, so every entry is
reachable (a folder is read no further than 100,000 entries, ignored ones
included; past that a notice says so). A file opens in a read-only viewer pane (or the editor `file_editor` names), the leftmost column of the tab
(line numbers, keyword/string/comment highlighting, 4 MB limit, binary
detection, ⌘F finds in the file; Markdown opens rendered, with a Rendered |
Raw switch, `sh` / `bash` / `zsh` / `fish` / `console` code blocks with a
Run… button that asks first and runs the block's text in the tab's terminal
pane, only at a prompt and never in a read-only one, and
`mermaid` blocks as labelled source); ⌥-click inserts its path into the
command line; right-click a row for Copy path, Copy relative path, New
file… and Reveal in Finder. A block that printed a Markdown file (`cat
README.md`) offers *Open in viewer* in its menu. Preview reads and
highlighting run in the background. Only regular files are accepted, and
the byte limit is enforced during reading, including when a file grows.
config.toml opened from Settings is the one file the pane edits: Save
(⌘S, shut while lint finds an error) / Dismiss, lint marks 150 ms after
typing stops, a Reload / Keep mine bar when the file changes under unsaved
edits, and a Save / Discard / Cancel question before they are lost
([ui.md › config.toml editor](ui.md#configtoml-editor)).

⇧⌘G opens code review: staged and unstaged changes (untracked included) as
file cards of hunks: line numbers, syntax colors, the changed characters of
an edited line highlighted, and "N unmodified lines" bands that open the
unchanged lines between hunks. Panel and cards are opaque; long lines scroll
sideways per file (trackpad, ⇧ + wheel). Stage, unstage or revert a whole
file from its header, or one hunk from the toolbar a hovered line shows (which
also copies the hunk as a unified diff); ⤢ maximizes the panel over the panes
with a file list on the right, Esc restores it. Revert asks first and only discards unstaged work. Hunk actions re-diff,
match the hunk by its line ranges and content, and apply that same checked
diff, so a stale panel never applies the wrong hunk. A hunk cut by the
4,000-line limit offers only file actions. The model loads on a background
thread and reloads when a command finishes; writes run in order on a
background writer thread. Details in [ui.md](ui.md).

The header's base chip picks what the review compares against:
**Uncommitted** (the default: staged and unstaged changes), **Branch:
<default>** (what `origin/HEAD` points at, else `main` / `master`, else
`origin/main` / `origin/master`) or any local or remote branch from a
filterable list. Against a branch the diff runs from `merge-base(branch,
HEAD)` to the working tree, so the branch's commits and the uncommitted work
show together; a strip under the header reads `vs <branch> (merge-base
<sha>)`, and stage, unstage and revert are hidden (`s` / `u` / `r` say why:
reverting against a branch would discard committed work). The choice is kept
per repository until Rift quits.

The commit bar at the panel's foot commits the staged changes: a message
field (Enter adds a line, ⌘⏎ commits), **Commit** (off with the reason when
nothing is staged, the message is empty or the panel compares against a
branch) and **Push**. Both run `git` (so your hooks, credential helpers and
ssh config apply) with an argument array, never a shell string, without a
terminal (`GIT_TERMINAL_PROMPT=0`, ssh in batch mode unless you set your own
ssh command) and off the UI thread with a spinner. A failing hook's output
shows in an error toast. Push sends the current branch to its upstream branch
of the same name; with none it runs `push --set-upstream` to the only remote or
`origin`. An upstream with another name (a branch made with `git checkout -b
feat origin/main`) is refused, like `push.default=simple`: `Upstream is
origin/main, not feat: push from a terminal or set the upstream`. A branch with
no commits yet says `No commits yet: commit first`. It never forces: a
rejected push (non-fast-forward) shows git's refusal; one that needs a password
or passphrase fails with a note that prompts are not supported. A commit or
push that runs past 120 s is stopped together with every process it started
(hooks included). A result whose window closed meanwhile is dropped.

**Send to agent…** pastes code into another pane — a coding agent's, usually
(Warp's "send to CLI agent"): a review hunk (`a`, or right-click a hunk or
line), a review file's hunks (right-click its header) or the file viewer's
selected lines (click a line, ⇧-click or ⇧↑ ⇧↓ to extend; `a` or ⋮ › Send to
agent…; without a selection, the top line on screen — on a rendered Markdown
page, which maps to no source line, a toast asks to switch to Raw). The text is
`path:start-end` (relative to the target pane's directory when the file is
under it) and a fenced block tagged with the file's language (a hunk keeps its
` ` / `+` / `-` marks), with no Enter after it. A hunk or file the diff line
cap cut names its last kept line and says `(truncated)` after the range. A
picker lists panes running a known agent (Claude Code, Codex, OpenCode, Gemini
CLI) first, then every other pane, across windows, each with its window, tab
(and pane number when the tab is split), title and directory; from the review
panel, the pane that was active is marked `(this pane)`. A shell waiting at
Rift's command editor gets the text in its editor, as a paste would (nothing
runs). Otherwise Enter pastes it bracketed when the target turned bracketed
paste on; a target without it would run each line, so Rift asks first, and
the paste goes to that pane only (not its synchronized siblings). A target
that closed meanwhile gets nothing (error toast).

## SSH profiles

Palette → "SSH: <name>" opens a tab titled after the profile that runs the
system `ssh` (so your ssh config, agent, `known_hosts` checking and prompts
all apply). Profiles are `[[ssh_profiles]]` in config.toml plus every concrete
`Host` of `~/.ssh/config` (and its `Include`s); trailing comments are ignored
and aliases containing `=` are preserved.

The profile's `ssh` shares its connection: Rift adds `-o ControlMaster=auto
-o ControlPath=<data dir>/ssh/%C -o ControlPersist=60` (the directory is
created mode 0700; a path too long for a socket name connects without
sharing). When the profile's args or your ssh config already set a
`ControlPath` (`ssh -G` reports one) or `ControlMaster`, Rift adds nothing
and uses yours.

"Install Rift shell integration on <name>" asks first, then runs an
installer over ssh in a new tab: it writes `~/.local/share/rift/rift.{bash,
zsh,fish}` and the completion servers `complete.{bash,zsh,fish}` on the host
and adds one line ending `# rift-shell-integration`
to `~/.bashrc` and `~/.zshrc` (plus `~/.config/fish/conf.d/rift.fish`). The
hooks load only when `LC_TERMINAL=Rift` arrives with the session: Rift sets
it and passes `-o SendEnv=LC_TERMINAL`, and most servers `AcceptEnv LC_*`.
After reconnecting, remote commands become blocks with the owned editor,
exit codes and the remote directory; images and tmux work over ssh as
locally. Without the hooks (or the env), ssh is an ordinary raw terminal.
The block of the `ssh` command itself ends when the remote prompt starts.
Rift tells a remote directory from a local one by the host the hooks report
(OSC 7 `file://host/path`; empty, `localhost` and this machine's name are
local). While a pane's directory is on another host, nothing reads this
machine's disk for it, and the git, pull request and compose chips clear
instead of describing a local directory with the same path.

**Remote completions.** A pane on another host completes on that host over
the shared connection of the newest `ssh` command submitted from Rift's
editor (an SSH profile or one typed there; with
`shell_completions` on): one `ssh -S <socket> -o BatchMode=yes` exec per
request, no new login, about 10–35 ms. It lists the directory being completed
(in the remote directory), the remote `$PATH` commands (once per host,
listed again after 10 minutes), git refs where a spec wants them, and the
remote shell's own completions through `rift-complete`, a function the
installed hooks define (it runs the same completion server on the host).
Answers are cached per host: a cached one shows at once while a refresh
runs; otherwise a request waits 400 ms and a later answer fills the cache.
The request travels on the exec's stdin and the command is a fixed script,
so nothing typed is put into a command; remote output is read up to 1 MiB,
5000 lines per section and 512 bytes per line, with escape sequences and
control characters stripped (file names are listed with `ls -q`). Only the
options that pick the destination (`-p -l -F -J -i -S -4 -6` and the `Port`,
`User`, `HostName`, `ControlPath`, `ProxyJump`, `IdentityFile`, `SendEnv`
`-o` keys) are reused from the `ssh` command; a request whose master is gone
fails at once (`ProxyCommand=false`) instead of logging in again. A command
read back from the screen (a remote program can print one with OSC 133
markers) is used only when no editor command is there (a session replayed
after a restart), and keeps only `-p -l -4 -6` and the `Port`, `User`,
`HostName` keys with plain values and a plain `[user@]host`: never a config
file, control socket, jump host or identity. The target is read again only
when the pane's blocks change. An `ssh` typed by hand without a live shared
connection (no `ControlPath` of its own) stays local-only (history, ssh
hosts, spec options), with the menu row "Remote completions need an SSH
profile" after real candidates (alone it opens no menu). Opening an SSH
profile opens its tab at once; `ssh -G` (which can take seconds with
`Match exec`) runs off the UI thread and the command follows at the tab's
first prompt.

## Accessibility

Rift draws every surface itself, so it publishes an AccessKit tree
(`crates/rift-terminal/src/a11y.rs`) to VoiceOver, Orca and other screen readers:

- **Tabs** as a tab list (selected state; activating one switches tabs).
- **Panes** as groups labelled with their directory and running job, each
  holding a list of the newest 50 **command blocks** (label = command, description =
  succeeded/failed with exit code and duration, value = output, last 4000
  characters), the visible **terminal** text, and the owned **command
  editor** as a text field.
- **Panels**: the file explorer and code review, with their buttons; the
  explorer's notices (an unreadable folder, a cut listing) are its text.
- **Overlays**: the palette (query field, result options, selection), find
  bar (query and match count), block filter (its own label, query and
  keep/clear instructions), settings (pages, switches with their state,
  buttons and choices with their current values), block menu, and the
  confirm dialogs.
- **Announcements**: a polite live region says "`command` succeeded/failed …"
  when a command finishes in the visible tab.
- **Actions**: Click on a node clicks its on-screen rect through the normal
  mouse routing, so it behaves exactly like the pointer. Only Click
  activates; Focus is not advertised, so moving the screen reader's focus
  never presses anything.

The tree is only built while an assistive technology is attached and is
pushed only when it changes. The script driver's `dump` prints it.

## Global hotkey

`global_hotkey` is empty (off) by default; `global_hotkey = "alt+space"`
registers a system-wide shortcut (macOS; X11 on Linux): from any app it brings Rift forward, and when Rift is already the
focused app it hides it (bringing forward the window that last had focus).
Scripted runs never register one.

**Drop-down hotkey window.** With `hotkey_window = true` (Settings ›
General › Hotkey window) the hotkey instead toggles a dedicated window,
Warp's hotkey window:

- The first press creates it with one tab (where a new tab would start).
  It drops onto the screen that has the mouse pointer (Linux: the screen of
  the focused Rift window), or the one `hotkey_window_screen` names
  (`primary`, or `1`, `2`… with 1 the primary; one that is not connected
  falls back to the pointer's). It hangs from that screen's
  `hotkey_window_edge` (`top`, the default, `bottom`, `left` or `right`;
  macOS: inside the menu bar and Dock), is `hotkey_window_height` percent of
  the screen deep (its height at the top or bottom, its width at the sides;
  20–100, default 40) and `hotkey_window_width` percent long along the edge,
  centred (20–100, default 100). A change to any of these (a connected
  numbered screen too) moves a shown window at once. It has no title bar or
  window buttons; the tab bar stays.
- It floats above other apps' windows. On macOS it appears on every Space
  and over full-screen apps (floating level; joins all Spaces, full-screen
  auxiliary, stationary in Mission Control) and slides in from its edge and
  back out over 150 ms, appearing at once when Reduce motion is on. On Linux (X11)
  it is an always-on-top window that appears at once; Wayland has no
  global hotkey.
- The hotkey hides it again when it is shown and focused (and focuses it
  when it is shown but another app is in front). On macOS hiding hands focus
  back to the app that was in front before it dropped down. With
  `hotkey_window_hide_on_blur` (default on) it also hides when it loses
  keyboard focus. Hidden, its shells keep running; the next press shows the
  same tabs.
- It is not saved in the session file: a relaunch or update does not bring
  its tabs back, and quitting ends its shells. An update's restart ends its
  shells too (durable ones included), so none run on unreachable in the
  session service. Closing its last tab closes it; the next press makes a
  new one. Ordinary windows are unaffected.
- It does not keep Rift open: closing the last ordinary window (⇧⌘W, the
  close button, ⌘W on its last tab) quits even while it exists, so that
  close asks first like ⌘Q does: about commands running in any window,
  the drop-down's included, about connected phones, and during an update
  it waits for the install. A phone closing the last ordinary window's
  last tab leaves a fresh tab, as with one window.
- Bringing one of its panes forward while it is hidden (`ctl focus --pane`,
  the session switcher, a notification or its inbox entry) drops it down
  first, so typing never lands in an invisible window. The session switcher
  lists its tabs as "… — hotkey window", and `ctl status`/`list` add
  `"windows": [{"window": N, "kind": "dropdown"}]` (absent while every
  window is ordinary).
- Turning `hotkey_window` off closes it (its shells end) when no command
  runs in it. With a command running it stays and drops down, so the
  command stays in view; it no longer hides on a focus loss, and closes
  like any window (⇧⌘W, or its last tab). The hotkey shows and hides the
  whole app again.
- The script driver's `hotkey` step runs the same toggle; `dump` lists the
  window as `kind=dropdown shown=… frame=[x, y, w, h] screen=[…]`.

## Deferred (by decision, not gaps)

Warp Drive / cloud / teams.
