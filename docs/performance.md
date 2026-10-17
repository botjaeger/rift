# Performance and compatibility baseline

`packaging/bench.sh` drives the release binary through the script driver in
an isolated config directory and prints one row per scenario. Compare runs on
the same machine only; the numbers are wall clock.

```sh
packaging/bench.sh                          # release build, then every scenario
RIFT_BIN=path/to/rift-terminal packaging/bench.sh
BENCH_KEEP=1 packaging/bench.sh             # keep the work dir (logs, dumps)
BENCH_VISIBLE=1 packaging/bench.sh          # also measure key-to-frame latency
```

Every scenario runs `--headless` with scratch `XDG_CONFIG_HOME` and
`XDG_DATA_HOME`. A headless window presents no frames, so the latency row is
skipped unless `BENCH_VISIBLE=1` shows the window for that one scenario (it
takes focus: do not type while it runs).

| Scenario | How it is measured |
|---|---|
| Startup to first prompt | Process start to the first shell prompt, best of 3, minus the driver's fixed 1.5 s start delay. |
| New window, in-process | The app's log of how long `new_window` held the UI thread (window, GPU surface, pipelines, first tab), best of 3; peak RSS of a run with two windows minus one with one. |
| `seq 1 500000` / `cat` 20 MiB | The finished block's duration (OSC 133 C→D) from the state dump. |
| Scrollback memory | Peak RSS after 300k lines into a 100k-line scrollback. |
| Key-to-frame latency | `RIFT_LATENCY_LOG=1`: from a raw key write to the first presented frame after the program's echo arrives (40 keys into `cat`). |
| TUIs | vim, less and top enter the alternate screen and return to the prompt after their quit key. |

## Recorded baseline

macOS 27, Apple M3 Max, release build, 2026-10-14 (key-to-frame latency
from 2026-09-23: it needs a visible window):

| Scenario | Result | Peak RSS |
|---|---|---|
| Startup to first prompt (best of 3) | 322 ms | 107 MiB |
| `seq 1 500000` | 544 ms | 138 MiB |
| `cat` 20 MiB file | 286 ms | (same run) |
| 300k lines into 100k scrollback | done | 169 MiB |
| Key-to-frame latency (41 keys) | p50 4.34 ms, p95 8.40 ms | |
| TUI: vim / less / top | enter + quit ok | |

The first run of this harness found the shape cache holding every trailing
blank cell of each row: the same `seq 1 500000` took 4161 ms and peaked at
2883 MiB. Rows are now shaped only up to their last visible character.

Scrollback rows used to keep every cell of the window's width (24 bytes
each): 100k history lines of a 147-column window held 442 MB (physical
footprint, 514 MiB peak RSS). A row entering the history now drops its run
of equal trailing cells and keeps one; under a full history the row that
enters takes the small buffer of the one recycled (unless that buffer is
over twice the size needed), so steady scrolling allocates nothing. The same
run settles at 76 MB, and `cat` of 20 MiB went from 320 ms to 290 ms (A/B on
the same machine, three runs each).

## A second window (2026-10-17)

⌘N used to start another process: a window cost what startup costs (322 ms
to its prompt, 107 MiB above). Windows now share one process, its GPU
device, fonts, glyph atlas, config and workers; each window adds only its
surface, its quad/sprite/image pipelines, its tabs and their shells.

macOS 27, Apple M3 Max, release build, headless; the machine was loaded
(startup measured 458 ms in the same run):

| Measure | Result |
|---|---|
| Opening it (the UI thread: window, surface, pipelines, first tab), `bench.sh` best of 3 | 12 ms |
| Same, 3 runs opening 3 windows each 2 s apart (the app's `new window in N ms` log) | 10–21 ms |
| Peak RSS over one window, `bench.sh` best of 3 | +9 MiB |
| Physical footprint per window, those runs (where macOS did not compress meanwhile) | +9.6 to +9.9 MB each |

The `windows` leak scenario (`scripts/leak-soak.py`) opens a window, runs a
command in it and closes it, 22 times: 176 MB → 176 MB over the 10 measured
cycles.

## Completion latency (2026-10-17)

`crates/rift-core/tests/completion_bench.rs` times what the completion
thread does per keystroke (`completion::complete`), 300 requests per
scenario rotating through 4–5 buffers, after one warm-up each. The fixtures
are written, then left alone for 2 s (a path changed more recently is
never served from a cache). It fails when a p95 is over 5 ms.

```sh
cargo test --release -p rift-core --test completion_bench -- --ignored --nocapture
```

macOS 27, Apple M3 Max, release build; the machine was loaded (load average
25–35), so the spread between runs is wide. Two runs each, p50 / p95:

| Scenario | Before | After |
|---|---|---|
| Path, 10 000 files + 50 directories in one directory | 4.75 / 27.30 ms; 9.47 / 33.15 ms | 0.04 / 2.02 ms; 0.03 / 1.26 ms |
| Git refs, 5 000 loose (50 directories) + 20 000 packed | 3.72 / 4.92 ms; 5.11 / 8.08 ms | 0.17 / 0.31 ms; 0.10 / 0.19 ms |
| History, 50 000 shell lines + 200 pane lines | 0.01 / 0.02 ms; 0.02 / 0.03 ms | 0.03 / 0.19 ms; 0.02 / 0.12 ms |
| Command names (`$PATH`) | 0.00 / 0.00 ms | 0.00 / 0.25 ms; 0.00 / 0.15 ms |

Before, every request listed the directory and `stat`ed each entry, and
walked `refs/` and read `packed-refs` again. Now each reads a cached listing
after one `stat` per path it came from (one for a directory; one per `refs/`
directory plus `HEAD` and `packed-refs` for a repository). The history and
command rows gained the recency ranking and the fuzzy (word start,
subsequence) passes, which stay well under the budget; the history row was
never the cost in `complete` itself: the app copied the pane's last 200
history lines into every request, which it now shares instead.

## Standard terminal suites (2026-10-10)

Run inside Rift (release build, headless, macOS 27, Apple M3 Max, 80×24 for
vttest, 124×29 for ucs-detect) through the script driver, which waits on a
file the suite writes when it ends (`wait_file`). The machine also ran
virtual machines (load average 15–60), so throughput is compared A/B: the
previous build (`b1dde18`) and this one alternated in the same minutes.

**vtebench** ([alacritty/vtebench](https://github.com/alacritty/vtebench),
`--max-secs 2`): median milliseconds to read 1 MiB of each workload; lower is
faster. Upstream's scripts find the tty with `ps -o tty=`, which pads its
output on macOS, so 7 of 12 benchmarks wrote only shell errors there; the
numbers come from a clone with `| tr -d " "` added.

| Benchmark | Before | Now | Benchmark | Before | Now |
|---|---|---|---|---|---|
| cursor_motion | 18 ms | 11 ms | scrolling_bottom_region | 37 ms | 24 ms |
| dense_cells | 22 ms | 11 ms | scrolling_bottom_small_region | 52 ms | 34 ms |
| light_cells | 13 ms | 9 ms | scrolling_fullscreen | 15 ms | 10 ms |
| medium_cells | 18 ms | 11 ms | scrolling_top_region | 49 ms | 32 ms |
| sync_medium_cells | 19 ms | 16 ms | scrolling_top_small_region | 34 ms | 24 ms |
| unicode | 9 ms | 9 ms | scrolling | 63 ms | 36 ms |

The gain is frame pacing: while a command ran, the running-block spinner
requested its next frame from inside the last one, so frames ran back to back
and each locked the terminal the reader thread parses into. Output and
spinner frames now wait at least 8.3 ms (120 Hz) after the previous one. The
VT420 and grapheme-width work costs the parser and grid about 17 % more
instructions per printable character (measured with `/usr/bin/time -l`
against crates.io `alacritty_terminal` 0.26, the parser itself unchanged);
ASCII and other non-joining characters take an inline fast path, and widths
come from a two-level table instead of a search.

**esctest** ([esctest2](https://github.com/ThomasDickey/esctest2),
`--expected-terminal=xterm --max-vt-level=4 --xterm-reverse-wrap=383
--timeout=0.5`): **524 passed, 0 failed**, 43 known xterm bugs (tests
esctest marks as failing on xterm itself). The theme for the run has xterm's
black foreground: `ResetSpecialColor_Dynamic` expects OSC 110 to restore
black, xterm's default. The first run of this suite (101 passed, 421 failed)
drove the VT420 work listed in [features.md](features.md#terminal-protocol-support).

**ucs-detect** 2.3.8 (Unicode 18, wcwidth 0.9.2), share of strings the
terminal measured as wcwidth does:

| Category | Before | Now |
|---|---|---|
| Narrow | 5570 / 5595 | 5595 / 5595 (100 %) |
| Wide (East Asian) | 480 / 506 | 506 / 506 (100 %) |
| Languages (scripts, combining) | 5558 / 6589 | 6572 / 6589 (99.7 %); 6589 / 6589 with `--probe-silently` |
| Regional indicators (flags) | 262 / 262 | 262 / 262 (100 %) |
| Standalone regional indicators | 0 / 26 | 26 / 26 (100 %) |
| Emoji VS16 (text → emoji) | 213 / 426 | 426 / 426 (100 %) |
| Emoji VS15 (emoji → text) | 0 / 158 | 158 / 158 (100 %) |
| Emoji ZWJ sequences | 10 / 1445 | 1445 / 1445 (100 %) |

The 17 language strings of the default run all begin with a Tai Tham
spacing mark, and ucs-detect prints each right after `·` and a space: by
UAX #29 (GB9a) a spacing mark joins the character before it, so the space
and the mark form one cluster. Measured alone (`--probe-silently`, each
string after a carriage return) all 6589 match.

Also detected: DECRQCRA and DECRQSS yes; kitty keyboard protocol with all
five flags; kitty and iTerm2 graphics; styled and colored underlines
(XTGETTCAP `Smulx`, `Setulc`). Sixel is not implemented (see
[features.md](features.md#terminal-protocol-support)).

**vttest** 2.7 (20251205), menus 1–4 and 8 checked by screenshot: cursor
movement, autowrap with mixed controls, scrolling regions, origin mode,
SGR attributes including blink, save/restore cursor, DEC line drawing and the
British character set, insert/delete character and line, 132-column mode
(the window resizes) and double-width and double-height lines. Double-width
lines draw normal-size glyphs centered across two cells rather than
stretched ones.

The suites found and fixed, besides the VT420 work: a tab's first cell drawn
as a real tab (later glyphs on the row shifted right), the reverse-screen
mode (DECSCNM), the sticky header hiding the top row of a running program,
quote and bracket auto-pairing leaving `]'` behind inside a quoted string,
`TERM_PROGRAM_VERSION` carrying the launching terminal's version, kitty
keyboard flags set with `CSI = … u` never reported back, the tab label
resolving the shell binary on every frame, and the unpaced spinner frames
above.
