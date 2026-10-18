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
  are tolerated — handlers are idempotent.
- Prompt-time context: the app stamps cwd + git branch onto the block at
  submit; the PTY reader fills cwd for raw-passthrough commands. Old blocks
  keep the context that was true when they ran.
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
Sticky command header, turns it off). Hovering the duration shows the full time and when the
command ended. A header's copy button and its ⋮ menu button (the block's
context menu, as a right-click) show only on hover; a copy turns the button
into a check for 1.5 s and toasts "Copied output".
Block element characters (U+2580–U+259F) are drawn as cell-filling quads, so
they join without gaps.

Per-block operations: select (click / ⌘↑ / ⌘↓), copy command/output/both,
copy cwd, copy git branch, re-run (only at a prompt), reinput into the editor
(⌘I), bookmark (⌘B), collapse (chevron), jump to top/bottom of block.

**Block filter** (⌥⇧F or block menu → *Filter output…*): the find bar edits
a filter for one block; only output lines containing the text (case
insensitive) stay visible. Running it again on the same block, or choosing
*Clear output filter*, shows every line again.

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
- Cursor motion and deletion step over whole grapheme clusters (an emoji
  with modifiers, a letter with combining accents), never part of one.
- Editor niceties: word motion & kills (whitespace-delimited), undo/redo
  (snapshot stack, cap 100, cleared on submit), history walk (↑/↓ preserving
  the in-progress line), ⌃R history pane, multi-source completion with
  ghost text, and configurable quote/bracket
  auto-pairing (typing a closer steps over it; Backspace removes an empty pair;
  inside a quoted string nothing pairs, so `'[a'` stays `'[a'`).
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
  command-not-found, a "Did you mean …" suggestion shows; → / Tab / End in
  an empty editor accepts it. `alias_expansion` (off) replaces an alias with
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
- **Prompt chips** (`prompt_chips`, Warp's "Edit prompt"): the context row
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
  While a Rift window is in use, its active tab's panes are also asked again
  every 3 s for git status and every 10 s for Docker Compose state, so files
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
  (words), `"` `'` `` ` `` (quotes), `(` `b` `[` `{` `B` `<` (brackets) —
  `ciw`, `da"`, `yi(`, `viw`.
- Visual mode selects on the editor's own selection, inclusive of the char
  under the cursor (whole lines in Visual Line), so ⌘C copies exactly it.
  Motions, searches and `j` / `k` (↑ / ↓) extend it; `o` swaps its ends;
  `v` / `V` switch or leave. Operators: `d` `x` delete, `c` `s` change, `y`
  yank, `~` `u` `U` case, `>` `<` indent, `J` join, `gu` `gU` `g~`; `D` `X`
  `Y` `C` `S` `R` take whole lines.
- Edits: `x` `X` `s` `S` `C` `D` `r`<char> `~` `p` `P` (unnamed register),
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
- Undo `u`, **redo `U`** (not `⌃R` — that stays the history pane). One `u`
  undoes one change: a whole Insert or Replace session, a visual-mode
  operator, a `.` repeat.
- History recall: `k` = previous entry, `j` = next (also ↑/↓) in Normal mode.

## Selection & mouse

- Drag = character selection, double-click = semantic (word) selection,
  triple-click = line selection; selection survives scrollback rotation.
- ⌘C copies the selection when one exists, else the last block's output.
- ⌥-drag makes a rectangular (column) selection.
- Typing clears the selection.
- **Mouse reporting** (`mouse_reporting`, on): programs that enable xterm
  mouse tracking (vim, tmux, htop, fzf) receive presses, releases, drags
  (1002), motion (1003) and the wheel in SGR (1006), UTF-8 (1005) or legacy
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
  mode resolves paths the same way.
- A copy, paste or open that fails shows a toast with the reason (in the
  pane, until dismissed); `Copied` appears only when the clipboard took the
  text.
- Click a block header: select the block (again: deselect). A click without a drag on its output also selects it (again: deselect); a drag selects text. Click its chevron: collapse.
- **Copy on select** (`copy_on_select`, on by default): a drag-selection is
  copied to the clipboard as soon as the mouse is released.

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
only, ≥ 2 chars, max 12 — every executable on the pane shell's `$PATH`), command specs (subcommands and flags with descriptions for common tools — git, cargo, docker, npm and more; `completion_specs.rs`) with argument generators (git refs, branches and remotes; ssh hosts from the ssh config and `known_hosts`; Makefile targets; `package.json` scripts; up to 50 each), filesystem paths (`~/` expanded;
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

**Ranking.** A word candidate matches the typed word by prefix (exactly as
typed), else by word start (a word inside it starts with what was typed,
any case: after `-` `_` `.` `/` `:` `@` or a space, or at a lower→upper case
change — `pick` finds `cherry-pick`), else as a subsequence (its characters
in order, any case — `cko` finds `checkout`). Word start and subsequence need
at least 3 typed characters. A command name matches other than by prefix
only when it starts with the same character as the typed word (any case),
and as a subsequence only from 4 typed characters (`tfa` never offers
`convertformat`). Candidates sort by that match kind, then by
source (command names, subcommands, options, generator values, paths), then
by how recently the word appears in the history (the pane's last 200 lines,
then the shell's newest 500), then in the provider's own order (paths and
values by name; git refs branches, remote branches, tags). The order is
deterministic. At most 45 word candidates are kept, so the history lines
(at most 5) always fit in the 50. A prefix match always ranks first, so the
ghost text and Tab on a unique prefix behave as with prefix-only matching.
When no candidate starts with the typed word and a history line drives the
ghost text, the word-start and subsequence candidates do not open the menu,
and Tab accepts the ghost text. Only a candidate that starts with the typed
word shows that part bold in the menu.

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

Providers run on a background thread, never on the UI thread. Tab or ⌃→
pressed while an answer is still in flight is applied once, when it arrives,
and only if the pane, buffer and cursor are unchanged; Escape cancels the
request, so a late answer cannot reopen the menu. If that thread cannot start
or has died, completion pauses (no menu or ghost text; editing is unaffected)
and the first edit after 5 seconds starts a new one.

History matches drive the ghost text: whole-buffer prefix matches, the pane's
own history first (last 200) then the imported shell history (newest 3000),
max 5. A request shares the pane's history with the editor instead of
copying it. Inline ghost text shows the top suggestion; Tab accepts, ⌃→
accepts one word.

The completion panel lists the word candidates, then the history lines; its
header has a tab per source with counts (⇧⇥ or a click narrows to one), and
accepting a history line replaces the whole command line.

**History pane** (⌃R): this window's finished commands (exit status,
directory, finish time), the pane's typed history and the shell's history
file, one row per command, newest first; the command line filters it, ⌃R
cycles the scope (this pane, all shells, failed), ⏎ runs, ⇥ edits first,
⌘⏎ runs in a new split.

`autosuggestions = false` turns the ghost text off; `completions_menu = false`
turns the completion panel off (the ghost text and Tab still work).

## Launch configurations

Warp-compatible YAML or TOML Tab Configs in `~/.config/rift/launch_configs/`
(and, read-only, Warp's own `tab_configs` directory) — a named set
of tabs, each with a (possibly nested) split layout, cwds and startup commands.
⌘P → `Launch configuration: <name>` opens one: tabs are appended, splits are
rebuilt, and each pane's `commands[].exec` are sent **at that pane's first
prompt**. A launch configuration opens in the current window; the tabs of
all `windows:` entries are appended in document order; `~` in a `cwd` is expanded and a non-existent directory is
ignored. Warp's own files drop in unchanged. "Save current layout as launch
configuration" writes one TOML Tab Config per open tab.

## Links (`rift://`)

Finder, Raycast, an editor or a README can open Rift through `rift://`
links (Warp's `warp://` paths):

| Link | Opens |
|---|---|
| `rift://action/new_tab?path=<folder>` | A tab in that folder (no `path`: where a new tab starts) |
| `rift://action/new_window?path=<folder>` | A window, its tab in that folder |
| `rift://launch/<name>` or `rift://tab_config/<name>` | The launch configuration or Tab Config of that name (case ignored); `?new_window=true` opens it in a window of its own |
| `rift://settings?q=<text>` | Settings, its search filled in |

`path` is percent-decoded (`+` is a space), `~` is expanded, and it must be
an absolute path to an existing folder. A link runs no command of its own: a
file (a script included) or an unknown action is refused with a toast saying
why (control characters removed, cut at 120 characters). The link opens in the last focused window (never
the drop-down window while another is open) and brings Rift to the front.
macOS registers the scheme with `Rift.app` and delivers the link as an Apple
Event (also to a Rift it launches); on Linux the `.desktop` file registers
`x-scheme-handler/rift`, and `rift-terminal rift://…` hands the link to the
running Rift over the control socket (with `control_api = "off"` it takes
links and nothing else) or opens it itself when none answers. A layout
whose panes run commands asks first (*Open this layout from a link?*, its
commands listed, Cancel focused), and more than ten links in three
seconds are dropped. A link while only the drop-down window exists opens
an ordinary window for it.

## Appearance

- **App icon** (`app_icon`, Settings › Appearance): Default, Light, Mono,
  Ocean, Ember, Forest or Neon — the "Prompt" icon in other colours. macOS
  shows it in the Dock at once and, through the Dock tile plugin in
  `Rift.app`, while Rift is not running (the app's own `.icns` and Finder
  icon stay the default: rewriting a signed bundle's icon breaks its
  signature). On Linux it is the window icon; docks keep the installed
  hicolor icon.
- **Themes**: four builtins (`dark-default`, `light-default`,
  `catppuccin-mocha`, `tokyo-night`) plus user themes in
  `~/.config/rift/themes/` as Rift TOML **or Warp YAML** — a Warp theme
  file works unchanged (solid colors only: a gradient `accent` or `background`
  is not read yet); Rift derives its full token set from Warp's base
  colors.
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
  ligatures on, `cursor_style` and `cursor_blink` set the cursor.
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
  background tab also raises its activity dot.
- `RIFT_NO_NOTIFY=1` suppresses both.

## Tabs, panes, session

- Tabs: create, close, reorder (⌃⇧←/→ or drag: the tab lifts and follows the
  pointer; Esc puts it back), select by number (⌘1–9, ⌘9 = last), reopen
  closed (⇧⌘T restores the tab's cwd, stack of 10).
- Tab settings (Settings › Appearance › Tabs): ⌘T and + open the new tab at
  the end or right after the active tab (`new_tab_placement`); the × shows
  on hover (active and hovered tab), always, or never (`tab_close_button`);
  `tab_bar_with_one_tab = false` hides the lone tab (and the vertical rail)
  until a second tab opens.
- Tab menu (right-click): *Rename tab* (inline), a tab color, duplicate,
  sync, *Close other tabs*.
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
- **New tabs start in** (`working_directory`): `previous` (the active pane's
  directory, default), `home`, or a path (`~` expands; a missing directory
  falls back to home). `confirm_close_running` (on) asks before closing a
  pane, tab or window with a running job; the dialog lists each running
  process with its agent badge.
- A fresh session shows a welcome list (split, palette, history, explorer,
  settings, with their shortcuts) until its first command.
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
  pane and Move to new tab.
- **Synchronized inputs** (⌥⌘I, tab context menu or Window menu): typing and
  submitting in one pane of a split tab goes to the other panes of that tab;
  the tab shows a sync marker while it is on. Input only reaches panes in the
  same mode: raw keys and pastes go to the panes running a program, each
  encoded for that pane's own cursor-key, kitty keyboard and bracketed-paste
  modes; a command submitted from the editor runs only in the panes at a
  prompt, and a toast counts the panes it skipped. A multi-line paste asks
  first if any recipient lacks bracketed paste.
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
  Loading also enforces the 200-block / 1 MiB decoded text budget (including
  commands and directories), with 16 KiB per command/directory and a bounded
  JSON read. Oversized or malformed saved transcripts are ignored.
- **Vertical tabs** (`tab_layout = "vertical"`, Settings → Appearance, or
  "Toggle vertical tabs"): a 220px rail lists every tab with its icon,
  title, directory and git branch, pane count, sync state, activity dot and
  color; click selects, × closes, dragging a row up or down reorders. When
  the tabs do not fit, the active one stays visible and `+N above` /
  `+N more` lines stand in for the rest, marked with the most urgent alert
  among them.
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
  is never interpolated into the command line.

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

## Shell integration

Materialized to `~/.local/share/rift/` on first spawn, injected
automatically:

- **zsh** — ZDOTDIR shim that sources the user's real config first (a
  `ZDOTDIR` set in the user's `~/.zshenv` is honoured for their other files).
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
`LC_TERMINAL=Rift` and `LC_TERMINAL_VERSION`;
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
  that never opt in keep the legacy xterm bytes.
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
  a newline asks first ("Paste and run?"), because each newline would press
  Enter (`confirm_multiline_paste`). With no text on the clipboard (an image,
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
demand) instead of the window process. On macOS 13+ it and the phone
gateway are `SMAppService` agents that Rift.app carries
(`Contents/Library/LaunchAgents/dev.rift.session.plist`,
`dev.rift.gateway.plist`: registered once, started on demand, never kept
alive), not children of the window, so a quit Rift leaves the Dock instead
of staying there as "Running in Background" (macOS 27 keeps an app there
while a launchd job it bootstrapped runs). They appear under System
Settings › General › Login Items as Rift's background items. Runs with
their own `XDG_DATA_HOME`/`XDG_CONFIG_HOME` (scripted and test runs), runs
outside Rift.app and older macOS bootstrap launchd jobs instead
(`<data dir>/launchd/*.plist`, run once). An agent whose registration
still names another build (after an update, or a copy of Rift elsewhere
with the same labels) is refused by launchd (`spawn failed`, exit 78): Rift
registers it again, which pins this build, and retries once, else starts
the one-shot job instead; every `launchctl` call gives up after 5 s. A pane asks the service on its
own thread, never the window's: starting the service or the gateway, and a
slow or stalled service, never hold the window (⌘T, typing and splits go
on; the pane shows `Starting zsh…` and keeps what is typed). A service that does
not answer a request within 5 s (hello, spawn, attach) counts as
unavailable: that pane starts a local shell in its place with a notice (it
runs what was typed meanwhile), and the rest of the window's run stays
local instead of waiting on it again. A reattach whose shell ended while
detached starts a fresh one in the service.

**Quitting** Rift (⌘Q, Quit Rift in the menu or the palette, or closing the
window) is intentional: every shell ends and the session file keeps only the
window's frame, so the next launch starts fresh with one tab where the window
was (saved block output goes too). While a phone is connected, the quit
dialog names it first (*iPhone is connected and loses its tabs.*), with the
running commands when *Warn before closing running commands* is on. Shells a
phone opens after the quit (`remote_when_closed`) still appear as tabs at the
next launch.

**Menu bar icon** (macOS, `menu_bar_icon`, on by default; Settings › General
› App › *Show menu bar icon*): the app icon as a line drawing in the menu bar
(a template image, tinted by macOS). While Rift runs its menu has *New Window*, *Settings…* and
*Quit Rift Completely*. After the window quits, the phone gateway shows it
for as long as it runs (`remote_access`; 30 s after the last window, or
until stopped with `remote_when_closed`), with *Open Rift* and *Quit Rift
Completely*. *Quit Rift Completely* is the usual quit (the same warnings)
that also stops the gateway (`<data dir>/remote/stop`, seen within 0.5 s)
and ends the shells phones started in the session service, which then
exits; nothing of Rift keeps running. ⌘Q leaves the gateway as before.
*Launch at login* (`launch_at_login`, beside it) adds or removes Rift's
login item.

An update's restart, a crash, a killed process or a reboot of the window
only (the service survives) leave the shells running; the next launch
restores the layout and **reattaches** each pane: the service replays the output it kept (the newest 8 MiB per
shell) into a fresh terminal, with the original timing so block durations
stay right, then continues live. Nothing is shown twice, and automatic
terminal replies (device attributes, image OKs) are not re-sent during the
replay. Commands that finished before the reattach keep their own finish
time and do not appear again in Notifications.

- Closing a pane (⌘W) ends its shell, as without the service. ⌘K also drops
  the retained output, so a reattach does not bring it back.
- With `restore_session = false`, Rift ends its shells however it exits
  when it can, instead of orphaning them.
- Palette → "Attach detached session: …" opens a tab on any shell the
  service keeps that no window shows.
- The service listens on `~/.local/share/rift/sessiond/sock` (a 0700
  directory; other users' connections are refused) and exits 10 s after its
  last shell and window are gone. A slow window is disconnected rather than
  buffering without bound; the pane reattaches on its own and replays only
  what it missed (it closes only when the shell is really gone). Input typed
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
  *Update Rift* (`update_rift`). Update messages are cards, never a status
  toast.
- The chip, **Update…** and `update_rift` open a dialog that says what the
  restart keeps: with durable sessions, *Your N tabs and M shells keep running
  and come back where they were*; without them, open shells end (turn on
  *Keep shells running through updates* to keep them). When the new
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
- The new version counts as started 10 s after its first frame: it then
  deletes the old copy, shows the card *Updated to Rift vX.Y.Z* (a click
  opens What's New) and opens the other
  processes' windows beside its own. If it exits or crashes before that,
  the helper puts the old version back and starts it (*Couldn't update to
  Rift vX.Y.Z*), and that one reopens the windows. A crash mid-update
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
window closes. It listens on `remote_listen` (`0.0.0.0:7878`). The page's
*Connection* group shows *This network* (the listening addresses) and the
*Relay* (its round trip while connected, or the whole error, a retry
countdown and *Try again*, which retries at once).

- **Pairing.** *Pair a phone* shows a QR code, the `rift-remote://pair?…`
  link with a *Copy link* button, and a 2-minute countdown (then *Expired —
  Pair again*). The link works once. *View only* makes the next link pair a
  device that can watch but not run commands or send keys.
- **Paired devices** lists each phone with how it is connected (*Connected
  on this network*, *Connected through the relay*, or *Not connected* with
  when it was last seen), its platform and pairing date, an *Operate* / *View
  only* choice (raising a phone to *Operate* asks first; its connections close
  and reconnect with the new permission within a second, and a lowered
  permission applies at once), and *Revoke*, which asks first ("Revoke
  <name>? It disconnects now and cannot reconnect without pairing again."),
  then closes its connections within a second.
- **Recent connections** lists the newest 20 connections and failed
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
  Its address and the Cloudflare tunnel sit under *Relay settings…*; its
  state shows in *Connection*.
  The relay only forwards encrypted traffic; host one with `rift-terminal
  relay` or `packaging/relay.Dockerfile` (behind a TLS proxy such as Caddy).
  A Tailscale or other private-network address of the computer works
  without a relay: it is listed in the pairing link like any other address.
  Pairing links made in Settings carry the running gateway's relay too.
- **Tabs while Rift is closed** (`remote_when_closed`, *Phones can open
  tabs while Rift is closed*; off by default, needs `durable_sessions`): the
  gateway stays up after the last window closes, and a paired phone that may
  run commands can open a tab (after Face ID or the passcode on the phone).
  The shell runs in the session service; the computer shows a notification
  naming the device. The phone follows and drives it like any pane. The next
  Rift window shows these shells as tabs (with a toast) and the gateway lets
  go of them. At most 8 at a time.
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
  the alternate screen (tmux), look like programs and take typed text. Its Terminal view shows the pane
  the way Rift draws it (a header row per command, not the shell's prompt
  echo), and the phone can size the pane to its width while it follows it:
  the shell is the smaller of the two sizes, so programs reflow, until the
  phone leaves the pane or disconnects, or 30 seconds pass without a
  refresh. With a hardware
  keyboard, ctrl/alt combinations, Esc, Tab and F-keys pressed in the
  command field go to the pane too.
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
- **Phone-first app.** *Portals* is the home page (no tab bar); its gear
  opens *Settings* and its **+** pairs a computer. *Portals* lists each
  computer in a card (connection line, *New tab*, *Retry* or *Pair again*)
  with its tabs, each with its state and time (*Needs you · 2m*, *Working
  3:56*, *Idle 12m*); chips (*needs you*, *working*, *idle*) filter the tabs. With
  one computer paired, an agent's question answers in place on its tab. A cold
  start reopens the pane shown last. A pane's header title opens the *Tabs*
  sheet (every tab of the computer, filtered by state) and its pane picker
  the *Panes* sheet; a sideways swipe also moves between the tab's panes.
  Successes before the newest failure fold away; blocks are cards with a thin
  border in their state's colour (a table as a list, with *Raw*), failed ones
  with *Run again* and *Copy error*, running ones with *Stop* and *Notify when
  done*; a tap opens the block sheet. The header's *Terminal view* shows the
  pane's screen whatever runs, with pinch-to-zoom. Above the command line: the
  folder and branch chips opening the chip menus, suggestions, the key row (⌃;
  sticky ⌃ ⌥ ⇧, Paste) and a round field with a run button; commands run from
  there, there is no separate Run page. *Run* sends at once; Settings ›
  Running commands › *Ask before running* (off by default) shows the
  confirmation first. *Wrap long lines* (Settings › Panes, and the pane's ⋯
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
  An `operate` phone answers: a numbered entry is typed as its digit, an
  unnumbered one as arrow keys and Enter, "No" can carry what to do
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
  home upgrades to the direct path), shows *Offline* without retrying while
  there is no network, closes its connections 30 seconds after going to the
  background (3 minutes while a "notify me" command runs; Android 15+ and iOS
  may cut them sooner, and it reconnects on return), and can require
  Face ID / fingerprint on launch, after 5 minutes away, and (with *Ask
  before running* on) before running commands. With a push token it hands the computers, they alert through the
  Expo push service while the app is closed (`mobile/README.md` › Push).
  Settings › Notifications › *While the app is closed* › *Alerts through
  ntfy* also posts them to a random ntfy topic (ntfy.sh
  or self-hosted), which the free ntfy app shows: no push setup needed.
  Settings › Notifications › *My commands* (on by default) asks to be
  told about every command run from the phone; for other commands, *Other
  commands on your computers* chooses *Nothing*, *Failures* or *Everything*
  and *Only if it ran longer than* how long a command must run (10 s – 5
  min); each computer's page (Settings › Computers) and the pane's ⋯ menu
  have a *Notifications* switch that turns its notifications off. "Notify
  me" always notifies.

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
  badge (the Claude mark for Claude Code). An agent waiting at its own prompt
  is idle and its tab does not spin: Claude Code titles itself `✳ …`, Gemini
  CLI `◇ …`, and Codex is idle unless its title leads with a braille
  spinner. The wording sorts notifications into waiting (permission, approval,
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
Raw switch); ⌥-click inserts its path into the command
line. Preview reads and
highlighting run in the background. Only regular files are accepted, and
the byte limit is enforced during reading, including when a file grows.

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
  It spans the full width of the screen that has the mouse pointer (Linux:
  the screen of the focused Rift window), hangs from that screen's top edge
  (macOS: below the menu bar) and is `hotkey_window_height` percent of the
  screen tall (20–100, default 40). It has no title bar or window buttons;
  the tab bar stays.
- It floats above other apps' windows. On macOS it appears on every Space
  and over full-screen apps (floating level; joins all Spaces, full-screen
  auxiliary, stationary in Mission Control) and slides down and back up
  over 150 ms, appearing at once when Reduce motion is on. On Linux (X11)
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
Everything else on the [roadmap](terminal-landscape-roadmap.md) is planned.
