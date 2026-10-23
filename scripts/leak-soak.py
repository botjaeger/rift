"""Memory soak: drive Rift through repeated work and fail when memory keeps growing.

Each scenario runs one isolated scripted instance. The script touches a
marker file after every cycle of identical work and waits for the sampler's
acknowledgment; at each marker this harness reads the process's memory
(macOS: phys_footprint, which counts GPU buffers
and compressed pages; Linux: VmRSS). After the warm-up cycles every cycle
repeats the same work, so the footprint must level off: growth from the end
of warm-up to the last cycle over the scenario's budget is a leak.

    python3 scripts/leak-soak.py target/release/rift-terminal [scenario ...]

The window stays hidden (--headless), which is the display-asleep path: a
frame is prepared but never presented. RIFT_LEAK_SHOWN=1 shows the window so
the presented path is measured too; set it only on a virtual display (Xvfb
in CI and the Linux docker gate), never on a desktop someone is using.
"""
import ctypes
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
only = set(sys.argv[2:])
shown = os.environ.get("RIFT_LEAK_SHOWN") == "1"
MB = 1024 * 1024


def footprint(pid):
    """Bytes the process holds, or None once it is gone."""
    if sys.platform == "darwin":
        # rusage_info_v2: uuid[16], then u64 fields; ri_phys_footprint is the 8th
        libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
        buf = ctypes.create_string_buffer(512)
        if libproc.proc_pid_rusage(pid, 2, buf) != 0:
            return None
        return int.from_bytes(buf.raw[16 + 7 * 8:16 + 8 * 8], "little")
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
    except OSError:
        return None
    return None


def ticking(_):
    # a program redrawing a spinner all night (the 2026-10-16 incident: a
    # skipped frame leaked its GPU uploads, 94 GB by morning)
    return ["type while :; do printf '\\r%s' $RANDOM; sleep 0.02; done\\n"], ["wait 4000"]


def flood(_):
    # far past the scrollback limit, then cleared: history is bounded
    return [], ["type seq 1 120000\\n", "wait 2500", "key cmd+k", "wait 300"]


def stream(_):
    # one command that never ends and keeps printing lines (an agent's
    # session, a log tail): the running block's output is far past scrollback
    return ["type while :; do seq 1 500; sleep 0.05; done\\n"], ["wait 3000"]


def churn(scratch):
    # open and close every transient surface: tabs, splits, palette,
    # settings, find, the file viewer, resizes, scrolling
    return [], [
        "key cmd+t", "wait 400", "type echo churn\\n", "wait 200",
        "key cmd+d", "wait 400", "type ls\\n", "wait 200",
        "key cmd+w", "wait 200", "key cmd+w", "wait 300",
        "key cmd+p", "type settings", "wait 150", "key escape",
        "key cmd+,", "wait 300", "key escape",
        "key cmd+f", "type churn", "wait 150", "key escape",
        "key shift+cmd+e", "wait 300", "key escape",
        "resize 900 600", "wait 200", "resize 1200 800", "wait 200",
        "scroll 5", "scroll -5", "wait 100",
    ]


def windows(_):
    # open a second window (in-process, ⌘N), use it, close it: its surface,
    # pipelines, tabs and shell go with it
    return [], [
        "key cmd+n", "wait_prompt", "wait 300", "type echo window\\n", "wait 300",
        "close_window", "wait 300",
    ]


# name: (steps, warm-up cycles, measured cycles, budget in MB). Warm-up lasts
# until caches (glyph atlas, allocator arenas, scrollback) are full: measured
# on macOS, ticking levels off after ~18 cycles and flood after ~9; on Linux
# (Xvfb, shown) flood takes ~24, and glibc grows new ~11 MB arenas until
# stream levels off at ~13 cycles and churn at ~11 (three runs, 2026-10-16:
# 180/180/179 MB and 186/186/185 MB, flat after). The budget covers noise
# only; the incident's leak grew ~2 MB/s.
SCENARIOS = {
    "ticking": (ticking, 22, 8, 8),
    "flood": (flood, 26, 6, 8),
    "stream": (stream, 15, 20, 8),
    "churn": (churn, 13, 10, 8),
    "windows": (windows, 12, 10, 8),
}


def run(name, steps, warm, measured, budget):
    scratch = Path(tempfile.mkdtemp(prefix=f"rift-leak-{name}-", dir="/tmp"))
    cfg, data, work = scratch / "cfg/rift", scratch / "data/rift", scratch / "work"
    for d in (cfg, data, work):
        d.mkdir(parents=True, exist_ok=True)
    (cfg / "config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        f'working_directory = {json.dumps(str(work))}\n')
    (work / "file.txt").write_text("churn\n")
    setup, cycle = steps(scratch)
    cycles = warm + measured
    lines = ["wait_prompt", "resize 1200 800", *setup]
    for i in range(cycles):
        lines += cycle + [
            f"sh touch {shlex.quote(str(scratch / f'mark-{i}'))}",
            f"wait_file {scratch}/sampled-{i} 10000",
        ]
    # the pause lets the last sample land before the closing tab allocates
    lines += ["wait 1500", "key cmd+t", "wait 300", f"dump {scratch}/end.txt", "quit"]
    (scratch / "soak.rift").write_text("\n".join(lines) + "\n")
    env = dict(os.environ, XDG_CONFIG_HOME=str(scratch / "cfg"),
               XDG_DATA_HOME=str(scratch / "data"), RIFT_NO_NOTIFY="1")
    args = [str(binary), "--script", str(scratch / "soak.rift")]
    if not shown:
        args.append("--headless")
    log = open(scratch / "rift.log", "w")
    app = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT)
    samples = []
    deadline = time.monotonic() + 30 + cycles * 15
    try:
        for i in range(cycles):
            mark = scratch / f"mark-{i}"
            while not mark.exists():
                if app.poll() is not None or time.monotonic() > deadline:
                    tail = "\n".join((scratch / "rift.log").read_text(errors="replace").splitlines()[-20:])
                    return f"{name}: stopped before cycle {i + 1}/{cycles} ({scratch}/rift.log):\n{tail}"
                time.sleep(0.05)
            samples.append(footprint(app.pid))
            # Hold the app at the cycle boundary until it has been sampled.
            # Otherwise the 50ms poll can catch the next cycle's allocations.
            (scratch / f"sampled-{i}").touch()
        app.wait(timeout=30)
    finally:
        if app.poll() is None:
            app.kill()
        log.close()
    if app.returncode != 0:
        return f"{name}: exit {app.returncode} ({scratch}/rift.log)"
    # the script ran to its end: every cycle closed what it opened, and the
    # last step opened the second tab
    if "tabs=2 " not in (scratch / "end.txt").read_text():
        return f"{name}: the cycles did not leave one tab ({scratch}/end.txt)"
    base, last = samples[warm - 1], samples[-1]
    growth = (last - base) / MB
    trace = " ".join(f"{s / MB:.0f}" for s in samples)
    print(f"  {name}: {base / MB:.0f} MB -> {last / MB:.0f} MB over {measured} cycles "
          f"(+{growth:.1f} MB, budget {budget} MB) [{trace}]", flush=True)
    if growth > budget:
        return f"{name}: grew {growth:.1f} MB over {measured} identical cycles (budget {budget} MB)"
    return None


# RIFT_LEAK_CYCLES: measured cycles for a longer soak; the budget stays, so
# a slow leak gets more cycles to exceed it
longer = int(os.environ.get("RIFT_LEAK_CYCLES", "0"))
failures = []
mode = "shown" if shown else "headless"
for name, (steps, warm, measured, budget) in SCENARIOS.items():
    if only and name not in only:
        continue
    print(f"leak soak: {name} ({mode})", flush=True)
    err = run(name, steps, warm, longer or measured, budget)
    if err and " grew " in err:
        # one sample against one: churn's footprint swings ~20 MB between
        # cycles on macOS (a 43-cycle run held a 166 MB floor, 2026-10-23),
        # so a spike on the last sample reads as growth. A leak grows again.
        print(f"  {name}: over budget once; measuring again", flush=True)
        err = run(name, steps, warm, longer or measured, budget)
    if err:
        failures.append(err)
for f in failures:
    print(f"FAIL {f}", file=sys.stderr)
sys.exit(1 if failures else 0)
