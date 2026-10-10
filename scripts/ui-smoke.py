"""Regression checks through Shift's real input routing, in scratch directories."""
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time

repo = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
scratch = Path(tempfile.mkdtemp(prefix="shift-ui-", dir="/tmp"))
print(f"UI artifacts: {scratch}", flush=True)
cfg = scratch / "cfg" / "shift"
data = scratch / "data" / "shift"
work = scratch / "work"
for directory in (cfg / "themes", data / "control", work):
    directory.mkdir(parents=True, exist_ok=True)
(work / "hello.rs").write_text("fn main() {}\n")
theme = (repo / "crates/shift-core/src/theme/dark-default.toml").read_text()
(cfg / "themes/audit.toml").write_text(theme)
(scratch / "theme-next.toml").write_text(theme.replace('bg = "#1e1e1e"', 'bg = "#232326"'))
config = (
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'control_api = "full"\ntheme = "audit"\nsync_os_theme = false\n'
    f'working_directory = {json.dumps(str(work))}\n'
)
(cfg / "config.toml").write_text(config)
(scratch / "config-off.toml").write_text(
    "# keep this comment\n" + config.replace('"full"', '"off"') + "future_option = 1\n")
(scratch / "config-bad.toml").write_text("font_size = [\n")
(work / "my file.txt").write_text("one-arg-ok\n")
for project in ("a/src", "b"):
    (scratch / project).mkdir(parents=True, exist_ok=True)
(scratch / "a/src/only_a.rs").write_text("")
(scratch / "plain.sh").write_text('printf "\\033[?2004l"\nexec cat -v\n')
(scratch / "bracketed.sh").write_text('printf "\\033[?2004h"\nexec cat -v\n')
(scratch / "focus.sh").write_text('printf "\\033[?1004h"\nexec cat -v\n')

# A same-user window that accepts connections but never answers. Discovery
# must not delay palette input. No real window/service is contacted.
stalled = socket.socket(socket.AF_UNIX)
stalled.bind(str(data / "control/999999.sock"))
stalled.listen(32)
clients = []
def accept_stalled():
    while True:
        try:
            client, _ = stalled.accept()
            clients.append(client)
        except OSError:
            return
threading.Thread(target=accept_stalled, daemon=True).start()

def cp(source, target):
    return "sh cp " + shlex.quote(str(source)) + " " + shlex.quote(str(target))

def died(what, log):
    # CI keeps no artifacts: the log tail is the only trace of the cause
    tail = "\n".join(Path(log).read_text(errors="replace").splitlines()[-20:])
    return f"{what}; {log}:\n{tail}"

steps = [
    "wait_prompt",
    "key escape",
    "type echo shift-audit-ok\\n",
    "wait 500",
    f"dump {scratch}/prompt.txt",
    "key shift+cmd+o",
    "type audit",
    f"dump {scratch}/palette.txt",
    "key escape",
    "key shift+cmd+e",
    "wait 300",
    "key down",
    "key enter",
    "wait 700",
    f"dump {scratch}/viewer.txt",
    f"shot {scratch}/viewer.png",
    # ⌘F with a file open searches the file, not the terminal
    "key cmd+f",
    "type main",
    f"dump {scratch}/viewer-find.txt",
    "key escape",
    "key escape",
    "key escape",
    cp(scratch / "theme-next.toml", cfg / "themes/audit.toml"),
    "wait 1500",
    f"dump {scratch}/theme.txt",
    cp(scratch / "config-off.toml", cfg / "config.toml"),
    "wait 1500",
    f"dump {scratch}/off.txt",
    # an invalid edit keeps the last good config (automation stays off) and
    # Settings refuses to overwrite the user's file
    cp(scratch / "config-bad.toml", cfg / "config.toml"),
    "wait 1500",
    f"dump {scratch}/invalid.txt",
    "key cmd+=",
    "wait 300",
    f"dump {scratch}/invalid-save.txt",
    cp(cfg / "config.toml", scratch / "after-save.toml"),
    cp(scratch / "config-off.toml", cfg / "config.toml"),
    "wait 1500",
    "key cmd+p",
    "type settings",
    f"dump {scratch}/commands.txt",
    "key escape",
    # filename completion inserts one shell word
    "type cat ./my",
    "wait 300",
    "key tab",
    "wait 300",
    f"dump {scratch}/complete.txt",
    "key enter",
    "wait 500",
    f"dump {scratch}/complete-run.txt",
    # completion replaces the whole word: closing quote and mid-word suffix
    'paste cat "./my"',
    "key left",
    "wait 300",
    "key tab",
    "wait 300",
    f"dump {scratch}/complete-quoted.txt",
    "key cmd+a",
    "key backspace",
    "paste cat ./my --other",
    *["key left"] * len("y --other"),
    "wait 300",
    "key tab",
    "wait 300",
    f"dump {scratch}/complete-mid.txt",
    # word motion over multi-byte whitespace (U+3000) must not panic
    "key cmd+a",
    "key backspace",
    "paste a　b",
    "key alt+left",
    f"dump {scratch}/word-left.txt",
    "key cmd+a",
    "key backspace",
    # relative paths resolve against the directory their command ran in
    f"type cd {scratch}/a\\n",
    "wait 400",
    "type echo src/only_a.rs\\n",
    "wait 400",
    f"type cd {scratch}/b\\n",
    "wait 400",
    "key shift+cmd+h",
    f"dump {scratch}/hints.txt",
    "key escape",
    # find reaches the newest of more than 500 matches
    "type for i in $(seq 600); do echo findneedle; done\\n",
    "wait 1500",
    "key cmd+f",
    "type findneedle",
    f"dump {scratch}/find.txt",
    "key escape",
    # Enter typed ahead of a confirmation does not accept it
    "type sleep 30\\n",
    "wait 400",
    "key cmd+w",
    "key enter",
    f"dump {scratch}/confirm.txt",
    "key escape",
    "key ctrl+c",
    "wait 300",
    # Settings controls work from the keyboard
    "key cmd+,",
    "wait 200",
    # font size, not font family: a machine with one monospace font has
    # nothing to cycle to
    "key tab",
    "key tab",
    f"dump {scratch}/settings-focus.txt",
    cp(cfg / "config.toml", scratch / "settings-before.toml"),
    "key space",
    "wait 300",
    cp(cfg / "config.toml", scratch / "settings-after.toml"),
    "key escape",
    "key escape",
    f"type sh {scratch}/plain.sh\\n",
    "wait 400",
    "key cmd+d",
    "wait_prompt",
    f"dump {scratch}/split.txt",
    # synchronized paste is framed per recipient
    f"type sh {scratch}/bracketed.sh\\n",
    "wait 400",
    "key alt+cmd+i",
    "paste zz",
    "wait 400",
    f"dump {scratch}/sync.txt",
    # DECSET 1004: switching tabs tells the programs who lost/gained focus,
    # whatever the mouse-reporting setting (a headless window starts unfocused)
    "focus on",
    "key cmd+t",
    "wait_prompt",
    f"type sh {scratch}/focus.sh\\n",
    "wait 400",
    "key cmd+1",
    "key cmd+2",
    "wait 300",
    f"dump {scratch}/focus-tabs.txt",
    # wide glyphs: find sees "日本" across the spacer cell
    "key cmd+t",
    "wait_prompt",
    "type printf '\\346\\227\\245\\346\\234\\254 wide\\n'\\n",
    "wait 500",
    "key cmd+f",
    "type 日本",
    f"dump {scratch}/find-wide.txt",
    "key escape",
    "quit",
]
script = scratch / "smoke.shift"
script.write_text("\n".join(steps) + "\n")
env = dict(os.environ, XDG_CONFIG_HOME=str(scratch / "cfg"),
           XDG_DATA_HOME=str(scratch / "data"), SHIFT_NO_NOTIFY="1")
started = time.monotonic()
with (scratch / "app.log").open("w") as log:
    # headless: never shown or focused, so typing elsewhere cannot leak in
    app = subprocess.Popen([str(binary), "--script", str(script), "--headless"],
                           env=env, stdout=log, stderr=log)
    # The palette snapshot must appear promptly after the prompt snapshot,
    # well before the stalled socket's six-second response timeout.
    deadline = time.monotonic() + 40
    while not (scratch / "prompt.txt").exists():
        assert app.poll() is None, died("app exited", scratch / "app.log")
        assert time.monotonic() < deadline, "prompt snapshot timed out"
        time.sleep(0.02)
    prompt_at = time.monotonic()
    while not (scratch / "palette.txt").exists():
        assert app.poll() is None, died("app exited", scratch / "app.log")
        assert time.monotonic() - prompt_at < 3, "session discovery blocked palette input"
        time.sleep(0.02)
    while not (scratch / "off.txt").exists():
        assert app.poll() is None, died("app exited", scratch / "app.log")
        assert time.monotonic() < deadline, "config reload snapshot timed out"
        time.sleep(0.02)
    assert not (data / f"control/{app.pid}.sock").exists(), "disabled automation kept its socket"
    assert app.wait(timeout=60) == 0, died("app failed", scratch / "app.log")

def state(name):
    return (scratch / f"{name}.txt").read_text()

assert "editor_active=true" in state("prompt"), state("prompt")
assert "headless=true" in state("prompt") and "window_visible=Some(true)" not in state("prompt"), \
    state("prompt")
assert "shift-audit-ok" in state("prompt"), state("prompt")
assert 'palette=true mode=Sessions query="audit"' in state("palette"), state("palette")
assert 'lines=1 error=None' in state("viewer"), state("viewer")
assert (scratch / "viewer.png").read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
assert 'find=Some("1/1")' in state("viewer-find") and "search=false" in state("viewer-find"), \
    state("viewer-find")
def background(text):
    return next(line.split(" control_api=")[0] for line in text.splitlines() if line.startswith("theme_bg="))
assert background(state("prompt")) != background(state("theme")), "theme-only edit did not reload"
assert "control_api=off" in state("off"), state("off")
assert not (data / f"control/{app.pid}.sock").exists()
assert 'palette=true mode=Commands query="settings"' in state("commands"), state("commands")
assert "control_api=off" in state("invalid"), state("invalid")
assert 'toast=Some("config.toml not applied' in state("invalid"), state("invalid")
assert 'toast=Some("Settings not saved' in state("invalid-save"), state("invalid-save")
assert (scratch / "after-save.toml").read_text() == "font_size = [\n", "Settings overwrote an invalid config"
assert 'editor="cat ./my\\\\ file.txt"' in state("complete"), state("complete")
assert "one-arg-ok" in state("complete-run"), state("complete-run")
assert 'editor="cat ./my\\\\ file.txt" cursor=18' in state("complete-quoted"), state("complete-quoted")
assert 'editor="cat ./my\\\\ file.txt --other" cursor=18' in state("complete-mid"), state("complete-mid")
assert 'editor="a\\u{3000}b" cursor=4' in state("word-left"), state("word-left")
assert f'{scratch}/a/src/only_a.rs"' in state("hints"), state("hints")
found = re.search(r"matches=(\d+) total=(\d+)", state("find"))
assert found and found[1] == "500" and int(found[2]) >= 600, state("find")
assert "confirm=Some" in state("confirm") and "tabs=1" in state("confirm"), state("confirm")
assert 'settings_focus=Some(SettingsStep("font_size"' in state("settings-focus"), state("settings-focus")
assert (scratch / "settings-before.toml").read_text() != (scratch / "settings-after.toml").read_text(), \
    "Space did not press the focused Settings control"
settings_after = (scratch / "settings-after.toml").read_text()
assert settings_after.startswith("# keep this comment\n"), settings_after
assert "future_option = 1" in settings_after, settings_after
assert "panes=2" in state("split"), state("split")
sync = state("sync").split("\n  pane ")
assert "| zz" in sync[1] and "200~zz" not in sync[1], "plain pane got a bracketed paste"
assert "| ^[[200~zz^[[201~" in sync[2], "bracketed pane lost its paste framing"
assert "^[[O^[[I" in state("focus-tabs"), state("focus-tabs")
wide = re.search(r"matches=(\d+) total=(\d+)", state("find-wide"))
assert wide and int(wide[2]) >= 1, state("find-wide")
assert "wait_prompt timed out" not in (scratch / "app.log").read_text()
# --- second run: fresh state, fixed 1000x700 window, mouse and hint geometry ---
geo = scratch / "geo"
(geo / "cfg/shift").mkdir(parents=True)
(geo / "data/shift").mkdir(parents=True)
(geo / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(work))}\n')
(geo / "stream.sh").write_text(
    "for i in $(seq 1 60); do echo line$i; done\nsleep 2\n"
    "for i in $(seq 61 120); do echo line$i; done\n")
geo_steps = [
    "wait_prompt",
    "focus on",
    "resize 1000 700",
    "wait 300",
    "key cmd+k",
    "wait 300",
    # output scrolling between press and the first drag move must not shift
    # the selection anchor: press while static, scroll, then drag
    f"type sh {geo}/stream.sh\\n",
    "wait 1000",
    f"dump {geo}/stream-press.txt",
    "press 100 300",
    "wait 2800",
    "move 300 360",
    "move 400 360",
    "release 400 360",
    "key cmd+c",
    f"dump {geo}/stream-drag.txt",
    "key escape",
    # hint mode labels the sticky header when its block header is scrolled off
    "type seq 1 60\\n",
    "wait 800",
    "key shift+cmd+h",
    "wait 200",
    f"dump {geo}/hints-sticky.txt",
    "type a",
    "wait 200",
    f"dump {geo}/hints-picked.txt",
    "key escape",
    # → at the end of the line takes the whole history ghost
    "type seq 1",
    "wait 800",
    "key right",
    "wait 200",
    f"dump {geo}/ghost-right.txt",
    "key cmd+a",
    "key backspace",
    # a split pane's title bar drags the pane: onto the bottom half of its
    # neighbour, then onto the empty tab strip (a tab of its own)
    "key cmd+d",
    "wait 1500",
    f"dump {geo}/pane-split.txt",
    "press 200 56",
    "move 210 58",
    "move 750 650",
    "wait 200",
    f"dump {geo}/pane-dragging.txt",
    "release 750 650",
    "wait 300",
    f"dump {geo}/pane-below.txt",
    "press 300 56",
    "move 310 58",
    "move 700 20",
    "release 700 20",
    "wait 300",
    f"dump {geo}/pane-tab.txt",
    # a dragged tab lifts and reorders live; Esc puts it back where it was
    "press 180 21",
    "move 186 21",
    "move 600 21",
    "wait 100",
    f"dump {geo}/tabs-dragging.txt",
    "key escape",
    "release 600 21",
    "wait 200",
    f"dump {geo}/tabs-esc.txt",
    # a pane header dropped on the middle third of its neighbour swaps them
    "key cmd+d",
    "wait 1500",
    f"dump {geo}/swap-before.txt",
    "press 200 56",
    "move 210 58",
    "move 750 380",
    "wait 100",
    "release 750 380",
    "wait 300",
    f"dump {geo}/swap-after.txt",
    # ⌃R: the history pane filters by the command line; ⌃R again changes scope
    "type echo hist1\\n",
    "wait 500",
    "key ctrl+r",
    "type hist",
    "wait 200",
    f"dump {geo}/history-open.txt",
    "key ctrl+r",
    f"dump {geo}/history-scope.txt",
    "key escape",
    "key cmd+a",
    "key backspace",
    # palette: a typed prefix switches mode, ⇥ moves to the next one
    "key cmd+p",
    "type #",
    "wait 100",
    f"dump {geo}/palette-prefix.txt",
    "key tab",
    f"dump {geo}/palette-tab.txt",
    "key escape",
    # Settings is a tab: typing searches; a shell tab hides it; Esc closes it
    "key cmd+,",
    "wait 200",
    "type curs",
    f"dump {geo}/settings-search.txt",
    "key escape",
    "click 180 21",
    "wait 200",
    f"dump {geo}/settings-hidden.txt",
    "key cmd+,",
    "wait 200",
    "key escape",
    f"dump {geo}/settings-closed.txt",
    # a shell tab dragged past the Settings tab never swaps with it (panicked)
    "key cmd+,",
    "wait 200",
    "click 180 21",
    "wait 200",
    "press 180 21",
    "move 190 21",
    "move 700 21",
    "move 950 21",
    "release 950 21",
    "wait 200",
    f"dump {geo}/settings-drag.txt",
    "key cmd+,",
    "key escape",
    # ⏎ with no history match runs the typed line; a click outside closes it
    "key ctrl+r",
    "type nomatchqq",
    "key enter",
    "wait 500",
    f"dump {geo}/history-fallthrough.txt",
    "key ctrl+r",
    "wait 100",
    "click 500 200",
    f"dump {geo}/history-click.txt",
    # the review panel auto-hides in a narrow window: button unlit, toast
    "resize 600 500",
    "wait 300",
    "key shift+cmd+g",
    "wait 300",
    f"dump {geo}/review-narrow.txt",
    "resize 1500 600",
    "wait 300",
    f"dump {geo}/review-wide.txt",
    "quit",
]
(geo / "geo.shift").write_text("\n".join(geo_steps) + "\n")
geo_env = dict(env, XDG_CONFIG_HOME=str(geo / "cfg"), XDG_DATA_HOME=str(geo / "data"))
with (geo / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(geo / "geo.shift"), "--headless"],
                        env=geo_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("geometry run failed", geo / "app.log")

def gstate(name):
    return (geo / f"{name}.txt").read_text()

def clipboard(name):
    line = next(l for l in gstate(name).splitlines() if l.startswith("clipboard="))
    return json.loads(line[len("clipboard="):])

press = gstate("stream-press")
geometry = next(l for l in press.splitlines() if l.strip().startswith("geometry "))
line_h = float(re.search(r"line_h=([\d.]+)", geometry)[1])
rows = re.findall(r"(\d+):([\d.]+)@", geometry)
# dragging into the right margin includes the last column's character: fill
# every column, then release a quarter cell past the grid (font metrics differ
# per platform, so the point comes from this run's geometry)
cell_w = float(re.search(r"cell_w=([\d.]+)", geometry)[1])
cols = int(re.search(r"cols=(\d+)", geometry)[1])
grid_x = float(re.search(r"@([\d.]+)", geometry)[1])
edge_x = grid_x + (cols + 0.25) * cell_w
assert edge_x < 1000, (edge_x, geometry)
full = "0" * (cols - 1) + "9"
edge_steps = [
    "wait_prompt",
    "focus on",
    "resize 1000 700",
    "wait 300",
    "key cmd+k",
    "wait 300",
    f"type printf '%0{cols - 1}d9' 0; echo\\n",
    "wait 600",
    f"drag {grid_x + 1:.1f} 170 {edge_x:.1f} 170",
    "key cmd+c",
    f"dump {geo}/right-edge.txt",
    "quit",
]
(geo / "edge.shift").write_text("\n".join(edge_steps) + "\n")
with (geo / "edge.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(geo / "edge.shift"), "--headless"],
                        env=geo_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, f"right-edge run failed; see {geo}/edge.log"
assert clipboard("right-edge").rstrip("\n") == full, clipboard("right-edge")
row = next(i for i, (_, y) in enumerate(rows) if float(y) <= 300 < float(y) + line_h)
screen = [l[6:] for l in press.splitlines() if l.startswith("    | ")]
# the first selected line is the (cut) row under the press, the next one the
# row below it — not rows that scrolled in after the press
assert clipboard("stream-drag").split("\n")[1] == screen[row + 1].rstrip(), \
    (screen[row + 1], clipboard("stream-drag")[:80])
sticky = gstate("hints-sticky")
assert re.search(r'hints=\["block a \d+"\]', sticky) and "No links" not in sticky, sticky[:600]
assert "selected_block=None" in sticky
assert "selected_block=Some(" in gstate("hints-picked"), gstate("hints-picked")[:900]
assert 'editor="seq 1 60"' in gstate("ghost-right"), gstate("ghost-right")[:900]
split = re.search(r"layout=Split \{ axis: Horizontal, ratio: [\d.]+, first: Leaf\((\d+)\), "
                  r"second: Leaf\((\d+)\) \}", gstate("pane-split"))
assert split, gstate("pane-split")[:900]
left, right = split[1], split[2]
assert "mouse_cursor=Grabbing" in gstate("pane-dragging"), gstate("pane-dragging")[:300]
assert re.search(rf"layout=Split \{{ axis: Vertical, ratio: [\d.]+, first: Leaf\({right}\), "
                 rf"second: Leaf\({left}\) \}}", gstate("pane-below")), gstate("pane-below")[:900]
moved = gstate("pane-tab")
assert "tabs=2 active=1" in moved, moved[:300]
assert f"layout=Leaf({right})" in moved and f"layout=Leaf({left})" in moved, moved[:900]
def tab_leaves(name):
    return re.findall(r"^tab \d+ .*?\n  layout=(.*)$", gstate(name), re.M)

assert tab_leaves("tabs-dragging") != tab_leaves("pane-tab"), gstate("tabs-dragging")[:600]
assert tab_leaves("tabs-esc") == tab_leaves("pane-tab"), gstate("tabs-esc")[:600]
swap = re.search(r"layout=Split \{ axis: Horizontal, ratio: [\d.]+, first: Leaf\((\d+)\), "
                 r"second: Leaf\((\d+)\) \}", gstate("swap-before"))
assert swap, gstate("swap-before")[:900]
assert re.search(rf"layout=Split \{{ axis: Horizontal, ratio: [\d.]+, first: Leaf\({swap[2]}\), "
                 rf"second: Leaf\({swap[1]}\) \}}", gstate("swap-after")), gstate("swap-after")[:900]
history = gstate("history-open")
assert 'history=true scope=All' in history and 'history_selected=Some("echo hist1")' in history, \
    history[:1200]
assert "history=true scope=Failed" in gstate("history-scope"), gstate("history-scope")[:1200]
assert "palette=true mode=History" in gstate("palette-prefix"), gstate("palette-prefix")[:300]
assert "palette=true mode=Workflows" in gstate("palette-tab"), gstate("palette-tab")[:300]
assert 'settings=true' in gstate("settings-search") and \
    'settings_tab=true settings_query="curs"' in gstate("settings-search"), gstate("settings-search")[:1200]
assert "settings=false" in gstate("settings-hidden") and \
    "settings_tab=true" in gstate("settings-hidden"), gstate("settings-hidden")[:1200]
assert "settings_tab=false" in gstate("settings-closed"), gstate("settings-closed")[:1200]
assert "settings_tab=true" in gstate("settings-drag"), gstate("settings-drag")[:1200]
assert 'cmd="nomatchqq"' in gstate("history-fallthrough"), gstate("history-fallthrough")[:1500]
assert "history=false" in gstate("history-click"), gstate("history-click")[:1200]
narrow, wide = gstate("review-narrow"), gstate("review-wide")
assert "review=true" in narrow and "shown=false" in narrow, narrow
assert 'toast=Some("Window too narrow for the review panel' in narrow, narrow
assert "review=true" in wide and "shown=true" in wide, wide

# --- third run: shell-syntax filenames, Unicode text, partial lines, child shells ---
edge = scratch / "edge"
for directory in ("cfg/shift", "data/shift", "bin", "home"):
    (edge / directory).mkdir(parents=True)
(edge / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(work))}\n')
(edge / "home/.bashrc").write_text("PS1='edge$ '\n")
(edge / "home/.bash_profile").write_text("source ~/.bashrc\n")
for name in ("zzprobe;echo INJECTED", "zzsafe"):
    exe = edge / "bin" / name
    exe.write_text('#!/bin/sh\nprintf "RAN %s\\n" "$(basename "$0")"\n')
    exe.chmod(0o755)
edge_steps = [
    "wait_prompt",
    # a completed executable name stays one shell word
    "type zzpr",
    "wait 400",
    "key tab",
    "wait 300",
    "key enter",
    "wait 500",
    f"dump {edge}/completion.txt",
    # combining marks: searchable, and reinput runs the exact command
    "paste echo café",
    "key enter",
    "wait 400",
    "key cmd+i",
    f"dump {edge}/reinput.txt",
    "key cmd+a",
    "key backspace",
    "key cmd+f",
    "type café",
    "wait 300",
    f"dump {edge}/find-mark.txt",
    "key escape",
    # output without a final newline stays in its block
    "type printf NO_NEWLINE\\n",
    "wait 500",
    f"dump {edge}/partial.txt",
    # a child shell keeps the block integration
    "type exec bash\\n",
    "wait 800",
    "type echo INNER_OK\\n",
    "wait 500",
    f"dump {edge}/child.txt",
    "quit",
]
(edge / "edge.shift").write_text("\n".join(edge_steps) + "\n")
edge_env = dict(env, XDG_CONFIG_HOME=str(edge / "cfg"), XDG_DATA_HOME=str(edge / "data"),
                HOME=str(edge / "home"), BASH_SILENCE_DEPRECATION_WARNING="1",
                PATH=f"{edge / 'bin'}:{os.environ['PATH']}")
with (edge / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(edge / "edge.shift"), "--headless"],
                        env=edge_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("edge-case run failed", edge / "app.log")

def estate(name):
    return (edge / f"{name}.txt").read_text()

completion = estate("completion")
assert "RAN zzprobe;echo INJECTED" in completion and "| INJECTED" not in completion, completion
assert 'editor="echo cafe\\u{301}"' in estate("reinput"), estate("reinput")
marks = re.search(r"matches=(\d+) total=(\d+)", estate("find-mark"))
assert marks and int(marks[2]) >= 1, estate("find-mark")
assert 'ListItem "printf NO_NEWLINE" desc="succeeded' in estate("partial") and \
    'value="NO_NEWLINE"' in estate("partial"), estate("partial")
child = estate("child")
assert 'cmd="echo INNER_OK" exit=Some(0)' in child and "editor_active=true" in child, child

# review diff: a file card expands into hunks; a gap row opens the
# unmodified lines between hunks (all of a short gap at once)
rv = scratch / "review"
(rv / "repo").mkdir(parents=True)
for d in ("cfg", "data"):
    (rv / d).mkdir()
git = ["git", "-C", str(rv / "repo"), "-c", "user.name=t", "-c", "user.email=t@e.x"]
src = [f"    let v{n} = compute({n});" for n in range(1, 81)]
(rv / "repo" / "main.rs").write_text("\n".join(["fn main() {", *src, "}"]) + "\n")
subprocess.run(["git", "init", "-q", str(rv / "repo")], check=True)
subprocess.run([*git, "add", "-A"], check=True)
subprocess.run([*git, "commit", "-qm", "init"], check=True)
src[4] = "    let v5 = compute_fast(5);"
src[39] = "    let v40 = recompute(40);"
(rv / "repo" / "main.rs").write_text("\n".join(["fn main() {", *src, "}"]) + "\n")
rv_steps = [
    # hover repaints when the pointer crosses into another welcome row, and
    # only then (rows at y 240 and 278 in a 1000×700 window)
    "resize 1000 700",
    "wait_prompt",
    "wait 300",
    "move 500 400",
    f"dump {rv}/hover0.txt",
    "move 500 240",
    "move 520 241",
    f"dump {rv}/hover1.txt",
    "move 500 278",
    f"dump {rv}/hover2.txt",
    "resize 1400 900",
    "wait 300",
    f"type cd {rv / 'repo'}\\n",
    "wait 500",
    "key shift+cmd+g",
    "wait 800",
    "click 1000 131",
    "wait 200",
    f"dump {rv}/expanded.txt",
    # rows: header 32, gap 24, 8 lines of 22 → the gap above hunk 2
    "click 1000 359",
    "wait 200",
    f"dump {rv}/revealed.txt",
    "quit",
]
(rv / "rv.shift").write_text("\n".join(rv_steps) + "\n")
with (rv / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rv / "rv.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rv / "cfg"),
                                 XDG_DATA_HOME=str(rv / "data")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("review run failed", rv / "app.log")
hovers = [int(re.search(r"hover_repaints=(\d+)", (rv / f"hover{i}.txt").read_text())[1])
          for i in range(3)]
assert hovers[1] == hovers[0] + 1 and hovers[2] == hovers[1] + 1, hovers
expanded = (rv / "expanded.txt").read_text()
assert "expanded=1 revealed=0" in expanded, expanded
revealed = (rv / "revealed.txt").read_text()
# lines 10..37 sit between the hunks around lines 6 and 41
assert "expanded=1 revealed=28" in revealed, revealed

for client in clients:
    client.close()
stalled.close()
print(f"UI regressions passed ({time.monotonic() - started:.1f}s); {scratch}")
