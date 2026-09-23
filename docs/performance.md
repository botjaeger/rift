# Performance and compatibility baseline

`packaging/bench.sh` drives the release binary through the script driver in
an isolated config directory and prints one row per scenario. Compare runs on
the same machine only; the numbers are wall clock.

```sh
packaging/bench.sh                          # release build, then every scenario
SHIFT_BIN=path/to/shift-terminal packaging/bench.sh
BENCH_KEEP=1 packaging/bench.sh             # keep the work dir (logs, dumps)
```

| Scenario | How it is measured |
|---|---|
| Startup to first prompt | Process start to the first shell prompt, best of 3, minus the driver's fixed 1.5 s start delay. |
| `seq 1 500000` / `cat` 20 MiB | The finished block's duration (OSC 133 C→D) from the state dump. |
| Scrollback memory | Peak RSS after 300k lines into a 100k-line scrollback. |
| Key-to-frame latency | `SHIFT_LATENCY_LOG=1`: from a raw key write to the first presented frame after the program's echo arrives (40 keys into `cat`). |
| TUIs | vim, less and top enter the alternate screen and return to the prompt after their quit key. |

## Recorded baseline

macOS 27, Apple M3 Max, release build, 2026-09-23:

| Scenario | Result | Peak RSS |
|---|---|---|
| Startup to first prompt (best of 3) | 691 ms | 117 MiB |
| `seq 1 500000` | 664 ms | 191 MiB |
| `cat` 20 MiB file | 451 ms | (same run) |
| 300k lines into 100k scrollback | done | 533 MiB |
| Key-to-frame latency (41 keys) | p50 4.34 ms, p95 8.40 ms | |
| TUI: vim / less / top | enter + quit ok | |

The first run of this harness found the shape cache holding every trailing
blank cell of each row: the same `seq 1 500000` took 4161 ms and peaked at
2883 MiB. Rows are now shaped only up to their last visible character.
