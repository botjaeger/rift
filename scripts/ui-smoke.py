"""Regression checks through Shift's real input routing, in scratch directories."""
import base64
import hashlib
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
(scratch / "api-off.sh").write_text(
    f'while ls {shlex.quote(str(data / "control"))} | grep -qv "^999999.sock$"; do sleep 0.05; done\n'
    f'touch {shlex.quote(str(scratch / "api-off"))}\n')

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
    # polled: the file watcher's latency grows under load
    *[step for i in range(10) for step in ("wait 500", f"dump {scratch}/theme{i}.txt")],
    cp(scratch / "config-off.toml", cfg / "config.toml"),
    # the reload lands after the file watcher's latency, which grows under
    # load: wait for the control socket to go, not a fixed time
    f"sh sh {shlex.quote(str(scratch / 'api-off.sh'))} >/dev/null 2>&1 &",
    f"wait_file {scratch}/api-off 15000",
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
    # About Shift opens Settings › About; its copy button copies the version
    "key cmd+p",
    "type About Shift",
    "wait 200",
    "key enter",
    "wait 200",
    "key tab",
    "key space",
    "wait 200",
    f"dump {scratch}/about.txt",
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
assert any(background(state("prompt")) != background(state(f"theme{i}")) for i in range(10)), \
    "theme-only edit did not reload"
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
about = state("about")
assert "settings=true page=About" in about, about
assert "settings_focus=Some(SettingsCopyVersion)" in about, about
assert 'toast=Some("Copied version")' in about, about
assert re.fullmatch(r"v\d+\.\d+\.\d+.*", json.loads(next(
    l for l in about.splitlines() if l.startswith("clipboard="))[len("clipboard="):])), about
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
    # a click (no drag) on a command's output selects its block (40 rows
    # fill the pane with output on every platform's metrics)
    "type seq 1 40\\n",
    "wait 600",
    f"click {grid_x + 40:.1f} 300",
    "wait 100",
    f"dump {geo}/output-click.txt",
    # a running command spins its tab (and title bar) until it ends
    "type sleep 2\\n",
    "wait 500",
    f"dump {geo}/tab-running.txt",
    "wait 2500",
    f"dump {geo}/tab-done.txt",
    "quit",
]
(geo / "edge.shift").write_text("\n".join(edge_steps) + "\n")
with (geo / "edge.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(geo / "edge.shift"), "--headless"],
                        env=geo_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, f"right-edge run failed; see {geo}/edge.log"
assert clipboard("right-edge").rstrip("\n") == full, clipboard("right-edge")
# the drag selected text, not the block; the plain click then selects it
assert "selected_block=None" in gstate("right-edge"), gstate("right-edge")[:900]
assert "selected_block=Some(" in gstate("output-click"), gstate("output-click")[:900]
assert re.search(r"^tab 0 .* running=true$", gstate("tab-running"), re.M), gstate("tab-running")[:900]
assert re.search(r"^tab 0 .* running=false$", gstate("tab-done"), re.M), gstate("tab-done")[:900]
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
assert "palette=true mode=Sessions" in gstate("palette-tab"), gstate("palette-tab")[:300]
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
# long enough to scroll sideways in the 460 px panel
src[39] = "    let v40 = recompute(40); // " + "wide " * 40
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
    # a hovered diff line shows its hunk's toolbar over the gutter; its copy
    # button copies the hunk; the code scrolls sideways; ⤢ covers the panes
    # with a file list, Esc puts the panel back
    "move 1100 182",
    "wait 100",
    f"dump {rv}/hover-line.txt",
    "click 1007 182",
    "wait 100",
    f"dump {rv}/copied.txt",
    "scroll 0 -10",
    "wait 100",
    f"dump {rv}/sideways.txt",
    "click 1350 61",
    "wait 200",
    f"dump {rv}/maximized.txt",
    "key Escape",
    "wait 200",
    f"dump {rv}/restored.txt",
    # rows: header 32, gap 24, 8 lines of 22 → the gap above hunk 2
    "click 1000 359",
    "wait 200",
    f"dump {rv}/revealed.txt",
    # no sticky header over a command still running and followed live: it
    # would hide the program's top row; finished, it pins again
    "type seq 1 300; sleep 3\\n",
    "wait 1200",
    f"dump {rv}/sticky-running.txt",
    "wait 3500",
    f"dump {rv}/sticky-done.txt",
    # DECSCNM reverses the pane while set; typed brackets inside quotes pair
    # nothing (the closing quote used to leave `]'` behind)
    "type printf '\\\\033[?5h'; sleep 2; printf '\\\\033[?5l'\\n",
    "wait 1000",
    f"dump {rv}/reverse-on.txt",
    "wait 2000",
    f"dump {rv}/reverse-off.txt",
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
hover_line = (rv / "hover-line.txt").read_text()
assert '"stage-hunk@957,172,20,20", "revert-hunk@977,172,20,20", "copy-hunk@997,172,20,20"' \
    in hover_line, hover_line
assert "review_hits=[\"maximize" in expanded, expanded
assert 'toast=Some("Hunk copied")' in (rv / "copied.txt").read_text()
sideways = (rv / "sideways.txt").read_text()
assert 'scroll_x=["main.rs:200"]' in sideways, sideways
maximized = (rv / "maximized.txt").read_text()
assert "review_maximized=true" in maximized and '"jump@' in maximized, maximized
assert "review_maximized=false" in (rv / "restored.txt").read_text()
revealed = (rv / "revealed.txt").read_text()
# lines 10..37 sit between the hunks around lines 6 and 41
assert "expanded=1 revealed=28" in revealed, revealed
running = (rv / "sticky-running.txt").read_text()
assert "sticky=None" in running and "sticky=Some" not in running, running
done = (rv / "sticky-done.txt").read_text()
assert "sticky=Some(" in done, done
assert "reverse_screen=true" in (rv / "reverse-on.txt").read_text()
off = (rv / "reverse-off.txt").read_text()
assert "reverse_screen=false" in off, off
assert "cmd=\"printf '\\\\033[?5h'; sleep 2; printf '\\\\033[?5l'\" exit" in off, off

# DEC double-height (ESC # 3/4) and double-width (ESC # 6) lines draw every
# cell two columns wide: a double click maps the pointer to the word drawn
# under it, not to the single-width column there
dec = scratch / "dec"
for sub in ("cfg/shift", "data"):
    (dec / sub).mkdir(parents=True, exist_ok=True)
# the pointer positions below assume blocks from the top of the pane
(dec / "cfg/shift/config.toml").write_text('blocks_start = "top"\n')
(dec / "lines.txt").write_text(
    "normal line\n\x1b#3Double height\n\x1b#4Double height\n\x1b#6Double width\nnormal again\n")
dec_steps = [
    "resize 700 400",
    "wait_prompt",
    f"type cat {dec / 'lines.txt'}\\n",
    "wait 800",
    "dblclick 170 125",
    "wait 200",
    "key cmd+c",
    "key cmd+v",
    "wait 200",
    f"dump {dec}/wide.txt",
    "dblclick 60 104",
    "wait 200",
    "key cmd+c",
    "key cmd+a",
    "key cmd+v",
    "wait 200",
    f"dump {dec}/tall.txt",
    "quit",
]
(dec / "dec.shift").write_text("\n".join(dec_steps) + "\n")
with (dec / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(dec / "dec.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(dec / "cfg"),
                                 XDG_DATA_HOME=str(dec / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("DEC line run failed", dec / "app.log")
wide = (dec / "wide.txt").read_text()
assert 'editor="width"' in wide, wide
tall = (dec / "tall.txt").read_text()
assert 'editor="Double"' in tall, tall

# soft `clear`: the next command starts right above the command line
# (blocks_start = "bottom", the default), or at the top of the viewport with
# blocks_start = "top", never at the far end. The completion panel docked
# above the command line moves the output up out from under it (Warp): the
# newest line stays in view
clr = scratch / "clear"
for sub in ("cfg/shift", "data", "top/cfg/shift", "top/data"):
    (clr / sub).mkdir(parents=True, exist_ok=True)
clr_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
              'sync_os_theme = false\nwindow_opacity = 0.8\n'
              + f'working_directory = {json.dumps(str(work))}\n')
(clr / "cfg/shift/config.toml").write_text(clr_config)
(clr / "top/cfg/shift/config.toml").write_text(clr_config + 'blocks_start = "top"\n')
clear_steps = lambda out: [
    "resize 1000 700",
    "wait_prompt",
    "type seq 1 60\\n",
    "wait 600",
    "type clear\\n",
    "wait 600",
    "type echo after-clear\\n",
    "wait 600",
    f"dump {out}/after.txt",
]
(clr / "top/clear.shift").write_text("\n".join([*clear_steps(clr / "top"), "quit"]) + "\n")
with (clr / "top/app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(clr / "top/clear.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(clr / "top/cfg"),
                                 XDG_DATA_HOME=str(clr / "top/data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("clear (top) run failed", clr / "top/app.log")
clr_steps = [
    *clear_steps(clr),
    # a command just submitted and still running stays down there too (it
    # used to flash at the top, riding on the blank screen rows below it)
    "type sleep 2\\n",
    "wait 500",
    f"dump {clr}/running.txt",
    "wait 2000",
    "type for i in $(seq 60); do printf '%0120d\\\\n' 0; done\\n",
    "wait 800",
    f"dump {clr}/pre-panel.txt",
    "type cat ./",
    "key tab",
    "wait 500",
    f"dump {clr}/panel.txt",
    "quit",
]
(clr / "clear.shift").write_text("\n".join(clr_steps) + "\n")
with (clr / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(clr / "clear.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(clr / "cfg"),
                                 XDG_DATA_HOME=str(clr / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("clear run failed", clr / "app.log")

def grid_rows(text):
    geometry = next(l for l in text.splitlines() if l.strip().startswith("geometry "))
    return float(re.search(r"line_h=([\d.]+)", geometry)[1]), \
        [(int(a), float(y)) for a, y in re.findall(r"(\d+):([\d.]+)@", geometry)]

def after_clear_y(text):
    line_h, shown = grid_rows(text)
    start = int(re.findall(r'cmd="echo after-clear".*?output_start=Some\((\d+)\)', text)[-1])
    return line_h, shown, next(y for a, y in shown if a == start)

# top: the output starts within the first rows of the pane
line_h, _, first = after_clear_y((clr / "top/after.txt").read_text())
assert first < 4 * line_h + 60, first
# bottom: it is the last row drawn, right above the command line (700 px
# window, editor under 120 px)
after = (clr / "after.txt").read_text()
line_h, shown, first = after_clear_y(after)
assert shown[-1][1] == first and first > 700 - 120 - 3 * line_h, (first, after[:1500])
running = (clr / "running.txt").read_text()
line_h, shown = grid_rows(running)
sleep_out = int(re.findall(r'cmd="sleep 2".*?output_start=Some\((\d+)\)', running)[-1])
assert next(y for a, y in shown if a == sleep_out) > 700 - 120 - 3 * line_h, running[:1500]

panel = (clr / "panel.txt").read_text()
assert "completion=true" in panel, panel[:1500]
line_h, shown = grid_rows(panel)
zeros_end = int(re.findall(r'cmd="for i in .*?end=Some\((\d+)\)', panel)[-1])
assert shown[-1][0] == zeros_end - 1, (shown[-3:], zeros_end)
# ... drawn higher than without the panel, by at least its header and row
_, unpaneled = grid_rows((clr / "pre-panel.txt").read_text())
assert unpaneled[-1][0] == zeros_end - 1, unpaneled[-3:]
assert shown[-1][1] < unpaneled[-1][1] - 2 * line_h, (shown[-1], unpaneled[-1])

# prompt chips (Warp): the branch chip lists the local branches and checks
# one out, Esc closes a picker, the cwd chip lists the sub-directories and
# cds into one. A probe run reads the chips' rects from the dump.
chips = scratch / "chips"
for sub in ("cfg", "data", "repo/src"):
    (chips / sub).mkdir(parents=True, exist_ok=True)
cgit = ["git", "-C", str(chips / "repo"), "-c", "user.name=t", "-c", "user.email=t@e.x"]
subprocess.run(["git", "init", "-q", "-b", "main", str(chips / "repo")], check=True)
(chips / "repo/src/a.txt").write_text("a\n")
subprocess.run([*cgit, "add", "-A"], check=True)
subprocess.run([*cgit, "commit", "-qm", "init"], check=True)
subprocess.run([*cgit, "branch", "feature/x"], check=True)

def run_chips(name, steps):
    (chips / f"{name}.shift").write_text("\n".join([
        # a loaded runner is slow to run the cd: wait for it, then for git
        "resize 1200 800", "wait_prompt",
        f"type cd {chips / 'repo'} && touch {chips / name}.ready\\n",
        f"wait_file {chips / name}.ready 30000", "wait 800",
        *steps, "quit"]) + "\n")
    with (chips / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(chips / f"{name}.shift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(chips / "cfg"),
                                     XDG_DATA_HOME=str(chips / "data")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"chips {name} run failed", chips / f"{name}.log")

run_chips("probe", [f"dump {chips}/probe.txt"])
probe = (chips / "probe.txt").read_text()
cwd_xy = re.search(r"Working directory@(\d+),(\d+),(\d+),(\d+)", probe).groups()
run_chips("rows", [f"click {int(cwd_xy[0]) + 10} {int(cwd_xy[1]) + 9}", "wait 200",
                   f"dump {chips}/rows.txt"])
second = re.search(r'"1@(\d+),(\d+),(\d+),(\d+)"', (chips / "rows.txt").read_text()).groups()
second_row = f"{int(second[0]) + 20} {int(second[1]) + 10}"
def chip_at(label):
    x, y, w, h = map(int, re.search(label + r"@(\d+),(\d+),(\d+),(\d+)", probe).groups())
    return f"{x + w // 2} {y + h // 2}"
cwd_chip, branch_chip = chip_at("Working directory"), chip_at("Git branch")
run_chips("main", [
    f"click {branch_chip}", "wait 200", "type feat", f"dump {chips}/branches.txt",
    "key Enter", "wait 1200", f"dump {chips}/checked-out.txt",
    f"click {cwd_chip}", "wait 200", "key Escape", f"dump {chips}/escaped.txt",
    f"click {cwd_chip}", "wait 200", f"dump {chips}/dirs.txt",
    # the highlight follows the pointer at once
    f"move {second_row}", f"dump {chips}/hover.txt",
    "type sr", "key Enter", "wait 800", f"dump {chips}/cd.txt",
])
branches = (chips / "branches.txt").read_text()
assert "chip_menu=Some((Branch, \"feat\", [\"feature/x\", \"Create branch feat\"], 0))" \
    in branches, branches
assert 'cmd="git checkout feature/x" exit=Some(0)' in (chips / "checked-out.txt").read_text()
assert "chip_menu=None" in (chips / "escaped.txt").read_text()
dirs = (chips / "dirs.txt").read_text()
assert 'chip_menu=Some((Dir, "", [".. (Parent Directory)", "src"], 0))' in dirs, dirs
hover = (chips / "hover.txt").read_text()
assert 'chip_menu=Some((Dir, "", [".. (Parent Directory)", "src"], 1))' in hover, hover
cd = (chips / "cd.txt").read_text()
assert 'cmd="cd ./src" exit=Some(0)' in cd and "/repo/src\")" in cd, cd

# the diff stats chip (a default chip, shown on a dirty tree) opens the
# code review panel (Warp)
(chips / "repo/src/a.txt").write_text("a\nb\n")
run_chips("diffprobe", [f"dump {chips}/diffprobe.txt"])
diffprobe = (chips / "diffprobe.txt").read_text()
assert "review=false" in diffprobe, diffprobe
x, y, w, h = map(int, re.search(r"Git diff stats@(\d+),(\d+),(\d+),(\d+)", diffprobe).groups())
run_chips("diff", [f"click {x + w // 2} {y + h // 2}", "wait 300", f"dump {chips}/diff.txt",
                   f"click {x + w // 2} {y + h // 2}", "wait 300", f"dump {chips}/undiff.txt"])
assert "review=true" in (chips / "diff.txt").read_text()
# a second click closes it again
assert "review=false" in (chips / "undiff.txt").read_text()

# chip menus scroll by pixels: a third of a line per step (the script's
# smallest wheel step, like a trackpad's) moves the list a third of a row
# each time instead of nothing, or a whole row
for i in range(30):
    subprocess.run([*cgit, "branch", f"b{i:02}"], check=True)
run_chips("scroll", [f"click {chip_at('Git branch')}", "wait 200",
                     "scroll -1", "scroll -1", "scroll -1", f"dump {chips}/scrolled.txt"])
scrolled = (chips / "scrolled.txt").read_text()
offset = float(re.search(r"chip_menu_scroll=([\d.]+)", scrolled).group(1))
assert 20.0 < offset < 30.0, scrolled

# tool chips: PHP and Python show the versions the shell's PATH runs and
# switch this shell to another (`shift-use`); the PR chip shows the branch's
# pull request (`gh`) and opens it; the Docker chip counts the compose
# project's running containers and lists them. Fake gh, docker, Homebrew
# and pyenv trees keep it hermetic; HOME is scratch so no rc file or version
# manager of the runner's leaks in.
tl = scratch / "tools"
for sub in ("cfg/shift", "data", "home", "bin", "repo"):
    (tl / sub).mkdir(parents=True, exist_ok=True)
def fake(path, body):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)
brew = tl / "brew"
for formula, version in (("php", "8.4.1"), ("php@8.2", "8.2.9")):
    fake(brew / f"Cellar/{formula}/{version}/bin/php", "echo fake php\n")
    (brew / "opt").mkdir(parents=True, exist_ok=True)
    os.symlink(brew / f"Cellar/{formula}/{version}", brew / f"opt/{formula}")
(brew / "bin").mkdir(exist_ok=True)
os.symlink(brew / "Cellar/php/8.4.1/bin/php", brew / "bin/php")
pyenv = tl / "pyenv"
for v in ("3.11.1", "3.12.2"):
    (pyenv / f"versions/{v}/bin").mkdir(parents=True)
(pyenv / "version").write_text("3.12.2\n")
fake(pyenv / "shims/python3", "echo fake python\n")
# what `brew shellenv` and `pyenv init` put in a profile (macOS's
# path_helper moves /usr/bin, and its python3, ahead of the inherited PATH)
for rc in (".bash_profile", ".bashrc"):
    (tl / "home" / rc).write_text(f'export PATH="{pyenv}/shims:{brew}/bin:$PATH"\n')
fake(tl / "bin/gh", "echo '{\"isDraft\":false,\"number\":42,\"state\":\"OPEN\","
     "\"url\":\"https://github.com/o/r/pull/42\"}'\n")
ps = [{"Name": "repo-web-1", "Service": "web", "State": "running", "Status": "Up 2 minutes",
       "Image": "nginx", "Publishers": [{"URL": "0.0.0.0", "TargetPort": 80,
                                         "PublishedPort": 8080, "Protocol": "tcp"}]},
      {"Name": "repo-db-1", "Service": "db", "State": "exited",
       "Status": "Exited (0) 1 minute ago", "Image": "postgres", "Publishers": []}]
fake(tl / "bin/docker", 'case "$*" in *" ps "*) cat <<\'EOF\'\n'
     + "\n".join(json.dumps(c) for c in ps) + "\nEOF\n;; esac\n")
tgit = ["git", "-C", str(tl / "repo"), "-c", "user.name=t", "-c", "user.email=t@e.x"]
subprocess.run(["git", "init", "-q", "-b", "feature", str(tl / "repo")], check=True)
(tl / "repo/compose.yaml").write_text("services: {}\n")
subprocess.run([*tgit, "add", "-A"], check=True)
subprocess.run([*tgit, "commit", "-qm", "init"], check=True)
(tl / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'prompt_chips = ["github_pr", "python", "php", "docker"]\n'
    + f'working_directory = {json.dumps(str(tl / "repo"))}\n')
tool_env = {k: v for k, v in env.items()
            if k not in ("VIRTUAL_ENV", "PYENV_VERSION")}
tool_env.update(
    XDG_CONFIG_HOME=str(tl / "cfg"), XDG_DATA_HOME=str(tl / "data"), HOME=str(tl / "home"),
    HOMEBREW_PREFIX=str(brew), PYENV_ROOT=str(pyenv),
    PATH=os.pathsep.join([str(tl / "bin"), str(brew / "bin"), str(pyenv / "shims"),
                          "/usr/bin", "/bin"]))

def run_tools(name, steps):
    (tl / f"{name}.shift").write_text("\n".join(["resize 1200 800", "wait_prompt", "wait 1500",
                                                 *steps, "quit"]) + "\n")
    with (tl / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tl / f"{name}.shift"), "--headless"],
                            env=tool_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"tools {name} run failed", tl / f"{name}.log")

run_tools("probe", [f"dump {tl}/probe.txt"])
probe = (tl / "probe.txt").read_text()
assert 'prompt_chips_shown=["GithubPr:PR #42", "Python:3.12.2", "Php:8.4.1", "Docker:repo 1/2"]' \
    in probe, probe
def tool_chip(label):
    x, y, w, h = map(int, re.search(re.escape(label) + r"@(\d+),(\d+),(\d+),(\d+)",
                                    probe).groups())
    return f"{x + w // 2} {y + h // 2}"
pr_chip, py_chip = tool_chip("GitHub pull request"), tool_chip("Python environment")
php_chip, docker_chip = tool_chip("PHP version"), tool_chip("Docker Compose")
run_tools("main", [
    # a chip's hover card: its name, what it shows and what a click does
    f"move {docker_chip}", "wait 900", f"dump {tl}/docker-card.txt", "move 600 300",
    f"click {php_chip}", "wait 200", f"dump {tl}/php-menu.txt",
    "type 8.2", "key Enter", "wait 1200", f"dump {tl}/php-used.txt",
    f"click {py_chip}", "wait 200", f"dump {tl}/py-menu.txt",
    "key Down", "key Enter", "wait 1200", f"dump {tl}/py-used.txt",
    f"click {docker_chip}", "wait 600", f"dump {tl}/docker-menu.txt", "key Escape",
    f"click {pr_chip}", "wait 200", f"dump {tl}/pr.txt",
])
card = (tl / "docker-card.txt").read_text()
assert 'tooltip=Some("Docker Compose | The Docker Compose project around this directory' in card \
    and "follow a service's logs." in card, card[:1500]
php_menu = (tl / "php-menu.txt").read_text()
assert 'chip_menu=Some((Php, "", ["8.4.1", "8.2.9"], 0))' in php_menu, php_menu
php_used = (tl / "php-used.txt").read_text()
assert f'cmd="shift-use php \'{brew}/opt/php@8.2/bin\'" exit=Some(0)' in php_used, php_used
assert '"Php:8.2.9"' in php_used, php_used
py_menu = (tl / "py-menu.txt").read_text()
assert 'chip_menu=Some((Python, "", ["3.12.2", "3.11.1", "system"], 0))' in py_menu, py_menu
py_used = (tl / "py-used.txt").read_text()
assert 'cmd="shift-use pyenv \'3.11.1\'" exit=Some(0)' in py_used, py_used
assert '"Python:3.11.1"' in py_used, py_used
docker_menu = (tl / "docker-menu.txt").read_text()
assert 'chip_menu=Some((Docker, "", ["web", "db"], 0))' in docker_menu, docker_menu
assert 'toast=Some("Headless: not opening https://github.com/o/r/pull/42")' \
    in (tl / "pr.txt").read_text()

# a chip turned on while tabs are open shows in each at once (no command,
# no new tab): config.toml rewritten under the running app, as Settings or
# another window would
config_with = (tl / "cfg/shift/config.toml").read_text()
(tl / "cfg/shift/config.toml").write_text(
    config_with.replace('["github_pr", "python", "php", "docker"]', '["cwd"]'))
(tl / "docker.toml").write_text(config_with.replace(
    '["github_pr", "python", "php", "docker"]', '["cwd", "docker"]'))
run_tools("reload", [
    "key cmd+t", "wait_prompt", "wait 1000", f"dump {tl}/reload-before.txt",
    f"sh cp {tl}/docker.toml {tl}/cfg/shift/config.toml", "wait 1500",
    f"dump {tl}/reload-tab2.txt", "key cmd+1", "wait 300", f"dump {tl}/reload-tab1.txt",
])
(tl / "cfg/shift/config.toml").write_text(config_with)
assert 'prompt_chips_shown=["Cwd:' in (tl / "reload-before.txt").read_text()
for tab in ("tab2", "tab1"):
    shown = (tl / f"reload-{tab}.txt").read_text()
    assert '"Docker:repo 1/2"]' in shown, (tab, shown)

# prompt chips (Warp "Edit prompt"): the configured chips draw in order and
# a chip with nothing to show drops out; the shell reports its virtualenv
# (OSC 1337 SetUserVar). Settings › Prompt chips opens the editor: a palette
# chip clicked joins the prompt, a prompt chip dragged moves, Enter saves.
# Probe runs read the rects the next run clicks.
pc = scratch / "prompt"
for sub in ("cfg/shift", "data"):
    (pc / sub).mkdir(parents=True, exist_ok=True)
(pc / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nprompt_chips = ["python", "cwd", "git_branch", "time24"]\n'
    + f'working_directory = {json.dumps(str(work))}\n')

def run_prompt(name, steps):
    (pc / f"{name}.shift").write_text("\n".join(["resize 1200 800", "wait_prompt", *steps,
                                                  "quit"]) + "\n")
    # the runner's own Python environment stays out of the shell
    clean = {k: v for k, v in env.items()
             if k not in ("VIRTUAL_ENV", "PYENV_VERSION")}
    with (pc / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(pc / f"{name}.shift"), "--headless"],
                            env=dict(clean, XDG_CONFIG_HOME=str(pc / "cfg"),
                                     XDG_DATA_HOME=str(pc / "data")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"prompt {name} run failed", pc / f"{name}.log")
    return rc

run_prompt("chips", [
    f"dump {pc}/before.txt",
    "type export VIRTUAL_ENV=/opt/envs/my-venv\\n", "wait 800", f"dump {pc}/venv.txt",
    "key cmd+,", "wait 400", "type chips", "wait 300", f"dump {pc}/settings.txt",
])
before = (pc / "before.txt").read_text()
# no virtualenv, no repository: only the cwd and the time
assert re.search(r'prompt_chips_shown=\["Cwd:[^"]*", "Time24:\d\d:\d\d"\]', before), before
venv = (pc / "venv.txt").read_text()
assert 'prompt_chips_shown=["Python:my-venv", "Cwd:' in venv, venv
def rect_center(text, label):
    x, y, w, h = map(int, re.search(re.escape(label) + r"@(\d+),(\d+),(\d+),(\d+)", text).groups())
    return f"{x + w // 2} {y + h // 2}"
preview = rect_center((pc / "settings.txt").read_text(), "Edit prompt")
open_editor = ["key cmd+,", "wait 400", "type chips", "wait 300", f"click {preview}", "wait 300"]
run_prompt("probe", [*open_editor, f"dump {pc}/editor.txt"])
editor_dump = (pc / "editor.txt").read_text()
assert "prompt_editor=Some(([Python, Cwd, GitBranch, Time24], None))" in editor_dump, editor_dump
host = rect_center(editor_dump, "Palette(Host)")
# the editor's chips explain themselves too: palette and prompt box alike
run_prompt("cards", [*open_editor, f"move {host}", "wait 900", f"dump {pc}/card-palette.txt",
                     f"move {rect_center(editor_dump, 'Chip(0)')}", "wait 900",
                     f"dump {pc}/card-box.txt"])
assert 'tooltip=Some("Host | This machine\'s host name.")' in (pc / "card-palette.txt").read_text()
assert 'tooltip=Some("Python environment | The active virtualenv' in (pc / "card-box.txt").read_text()
first = rect_center(editor_dump, "Chip(0)")
last = re.search(r"Chip\(3\)@(\d+),(\d+),(\d+),(\d+)", editor_dump).groups()
past_last = f"{int(last[0]) + int(last[2]) + 4} {int(last[1]) + int(last[3]) // 2}"
# drag first: adding a chip can reflow the palette and move the box
run_prompt("edit", [*open_editor, f"drag {first} {past_last}", "wait 100", f"click {host}",
                    "wait 100", f"dump {pc}/edited.txt", "key Enter", "wait 300",
                    f"dump {pc}/saved.txt"])
edited = (pc / "edited.txt").read_text()
assert "prompt_editor=Some(([Cwd, GitBranch, Time24, Python, Host], None))" in edited, edited
assert "prompt_chips=[Cwd, GitBranch, Time24, Python, Host] prompt_editor=None" in \
    (pc / "saved.txt").read_text()
saved = (pc / "cfg/shift/config.toml").read_text()
assert re.search(r'prompt_chips = \[\s*"cwd",\s*"git_branch",\s*"time24",\s*"python",\s*"host",?\s*\]',
                 saved), saved

# tooltips: none before the rest time, then the control's action and
# shortcut; a neighbour shows at once; a key hides it. On a fresh session the
# welcome list runs under the history pane: its scope chip still gets the
# tooltip and the click. A fixed history (10 commands) lays the pane out the
# same everywhere; a first run reads the controls' rects from the dump.
tip = scratch / "tip"
for sub in ("cfg/shift", "data", "home"):
    (tip / sub).mkdir(parents=True, exist_ok=True)
(tip / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(work))}\n')
(tip / "home/.bash_history").write_text("".join(f"echo past{i}\n" for i in range(10)))
tip_env = dict(env, XDG_CONFIG_HOME=str(tip / "cfg"), XDG_DATA_HOME=str(tip / "data"),
               HOME=str(tip / "home"), HISTFILE=str(tip / "home/.bash_history"),
               BASH_SILENCE_DEPRECATION_WARNING="1")

def tip_run(name, steps):
    (tip / f"{name}.shift").write_text("\n".join(["resize 1000 700", "wait_prompt", *steps, "quit"]) + "\n")
    with (tip / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tip / f"{name}.shift"), "--headless"],
                            env=tip_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"tooltip {name} run failed", tip / f"{name}.log")

tip_run("probe", ["wait 300", f"dump {tip}/probe-bar.txt", "key ctrl+r", "wait 300",
                  f"dump {tip}/probe-history.txt"])

def centre(name, label):
    tips = re.search(r"^tips=(\[.*\])$", (tip / f"{name}.txt").read_text(), re.M)[1]
    x, y, w, h = map(float, re.search(re.escape(label) + r"@([\d.,-]+)", tips)[1].split(","))
    return f"{x + w / 2:.0f} {y + h / 2:.0f}"

review, inbox = centre("probe-bar", "Review changes"), centre("probe-bar", "Notifications")
close_tab = centre("probe-bar", "Close tab")
chip = centre("probe-history", "Commands that failed")
tip_run("hover", [
    f"move {review}", "wait 200", f"dump {tip}/early.txt",
    "wait 800", f"dump {tip}/review.txt",
    f"move {inbox}", "wait 50", f"dump {tip}/warm.txt",
    "key escape", "wait 100", f"dump {tip}/key.txt",
    "key ctrl+r", "wait 300",
    f"move {chip}", "wait 800", f"dump {tip}/chip.txt",
    f"click {chip}", "wait 200", f"dump {tip}/chip-click.txt",
    # ⌘W closes the tab only while it has one pane: the tooltip says so then
    "key escape", f"move {close_tab}", "wait 800", f"dump {tip}/close-one.txt",
    "key cmd+d", "wait 1500", f"move {review}", f"move {close_tab}", "wait 800",
    f"dump {tip}/close-split.txt",
])

def tooltip(name):
    return re.search(r"tooltip=(None|Some\(\"[^\"]*\"\))", (tip / f"{name}.txt").read_text())[1]

assert tooltip("early") == "None", tooltip("early")
assert tooltip("review").startswith('Some("Review changes'), tooltip("review")
assert tooltip("warm").startswith('Some("Notifications'), tooltip("warm")
assert tooltip("key") == "None", tooltip("key")
assert tooltip("chip").startswith('Some("Commands that failed'), tooltip("chip")
chip_click = (tip / "chip-click.txt").read_text()
assert "history=true scope=Failed" in chip_click, chip_click[:1500]
assert tooltip("close-one").startswith('Some("Close tab '), tooltip("close-one")
assert tooltip("close-split") == 'Some("Close tab")', tooltip("close-split")

# Settings › Files: link_editor = "shift" opens a hinted path in the viewer;
# an editor that is not installed falls back to the viewer with a toast;
# dotfiles stay out of the explorer until Show hidden files is pressed
fl = scratch / "files"
for sub in ("cfg/shift", "data", "work"):
    (fl / sub).mkdir(parents=True, exist_ok=True)
(fl / "work/.env").write_text("SECRET=1\n")
(fl / "work/visible.txt").write_text("hi\n")
(fl / "work/other.txt").write_text("x\n")
(fl / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nexplorer_hidden_files = false\nfile_editor = "nope"\n'
    'link_editor = "shift"\n' + f'working_directory = {json.dumps(str(fl / "work"))}\n')
(fl / "files.shift").write_text("\n".join([
    "resize 1000 700", "wait_prompt",
    f"type echo {fl}/work/other.txt\\n", "wait 500",
    "key shift+cmd+h", "wait 200", "type a", "wait 600", f"dump {fl}/link.txt",
    "key escape", "wait 200",
    # rows: other.txt, visible.txt
    "key shift+cmd+e", "wait 600", "key down", "key down", "key enter", "wait 600",
    f"dump {fl}/fallback.txt",
    "key cmd+,", "wait 300", "type hidden", "wait 300", f"dump {fl}/page.txt",
    "key tab", "key tab", "key space", "wait 600",
    "key cmd+,", "wait 600", f"dump {fl}/shown.txt",
    "quit"]) + "\n")
with (fl / "files.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(fl / "files.shift"), "--headless"],
                        # bash's zsh notice would put a URL first in the hints
                        env=dict(env, XDG_CONFIG_HOME=str(fl / "cfg"),
                                 XDG_DATA_HOME=str(fl / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("files run failed", fl / "files.log")
link = (fl / "link.txt").read_text()
assert f'viewer=Some("{fl}/work/other.txt")' in link, link[:1500]
fallback = (fl / "fallback.txt").read_text()
assert 'toast=Some("nope is not installed; opening here instead")' in fallback, fallback[:1500]
# the explorer root is canonical (/tmp → /private/tmp on macOS)
assert re.search(r'viewer=Some\("[^"]*/files/work/visible\.txt"\)', fallback), fallback[:1500]
assert 'TreeItem "visible.txt, file"' in fallback and '".env, file"' not in fallback, fallback
page = (fl / "page.txt").read_text()
assert 'Switch "Show hidden files" toggled=False' in page, page
assert "Bell" not in page.split("Dialog", 1)[1], page  # a hidden row never renames a shown one
assert 'TreeItem ".env, file"' in (fl / "shown.txt").read_text(), (fl / "shown.txt").read_text()
assert "explorer_hidden_files = true" in (fl / "cfg/shift/config.toml").read_text()

# Settings › Terminal: an OSC 52 read is answered only with "read_write";
# middle-click pastes the clipboard (an OSC 52 copy) into the pane
tm = scratch / "termpage"
for sub in ("cfg/shift", "data", "work"):
    (tm / sub).mkdir(parents=True, exist_ok=True)

def term_run(name, access, steps):
    (tm / "cfg/shift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        f'sync_os_theme = false\nclipboard_access = "{access}"\n'
        + f'working_directory = {json.dumps(str(tm / "work"))}\n')
    (tm / f"{name}.shift").write_text("\n".join(["resize 1000 700", "wait_prompt", *steps,
                                                  "quit"]) + "\n")
    with (tm / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tm / f"{name}.shift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(tm / "cfg"),
                                     XDG_DATA_HOME=str(tm / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"terminal page {name} run failed", tm / f"{name}.log")
    return (tm / f"{name}.txt").read_text()

# "bWlkZGxl" is base64 for "middle"
osc_read = [
    "type printf '\\\\033]52;c;bWlkZGxl\\\\a'\\n", "wait 400",
    "type printf '\\\\033]52;c;?\\\\a'; read -rs -t 2 -d $'\\\\a' r; echo \"GOT[${r#*;c;}]\"\\n",
    "wait 3000",
]
read_rw = term_run("read", "read_write", [*osc_read, f"dump {tm}/read.txt"])
# the reply can reach the tty before `read` turns echo off: match the
# echoed result, not the whole line
assert re.search(r'Terminal "Terminal output" value="[^"]*GOT\[bWlkZGxl\]"', read_rw), read_rw
read_w = term_run("noread", "write", [*osc_read, f"dump {tm}/noread.txt"])
assert 'Terminal "Terminal output" value="GOT[]"' in read_w, read_w
paste = term_run("mclick", "write", [
    "type printf '\\\\033]52;c;bWlkZGxl\\\\a'\\n", "wait 400", "mclick 500 300", "wait 400",
    f"dump {tm}/mclick.txt",
    "key cmd+a", "key backspace", "key cmd+,", "wait 300", "type osc",
    "wait 300", f"dump {tm}/page.txt"])
assert 'editor="middle"' in paste, paste[:1500]
page = (tm / "page.txt").read_text()
assert "Programs' clipboard access (OSC 52): Copy, selected" in page, page

# settings sweep regressions: vim_mode reaches the first pane and new tabs;
# End in Settings opens the last page (the terminal's scroll_bottom used to
# steal it); a theme file that does not load is labelled and toasted; Tab
# scrolls to a row below the fold
sw = scratch / "sweep"
for sub in ("cfg/shift/themes", "data"):
    (sw / sub).mkdir(parents=True, exist_ok=True)
(sw / "cfg/shift/themes/broken.toml").write_text('bg = "not a color"\n')
(sw / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nvim_mode = true\ntheme = "broken"\n'
    + f'working_directory = {json.dumps(str(work))}\n')
(sw / "sweep.shift").write_text("\n".join([
    "resize 1180 760", "wait_prompt", f"dump {sw}/first.txt",
    "key cmd+t", "wait_prompt", f"dump {sw}/second.txt",
    "key cmd+,", "wait 300", "key end", "wait 300", f"dump {sw}/themes.txt",
    "key home", "key down", "key down", "wait 300",
    *["key tab"] * 15, "wait 300", f"dump {sw}/tabbed.txt",
    "quit"]) + "\n")
with (sw / "sweep.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "sweep.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("sweep run failed", sw / "sweep.log")
first, second = (sw / "first.txt").read_text(), (sw / "second.txt").read_text()
assert "vim=Some(Insert)" in first, first[:1500]
assert second.count("vim=Some(Insert)") == 2, second[:2000]
themes = (sw / "themes.txt").read_text()
assert "settings=true page=Themes" in themes, themes[:1500]
assert 'RadioButton "broken (invalid theme file), active"' in themes, themes
assert 'toast=Some("Theme \\"broken\\": broken.toml: invalid color' in first, first[:1500]
tabbed = (sw / "tabbed.txt").read_text()
assert 'settings_focus=Some(SettingsToggle("command_corrections"))' in tabbed, tabbed[:1500]
assert 'Switch "Suggest corrections for failed commands"' in tabbed, tabbed

# an agent idle at its own prompt (Claude Code's "✳" title) does not spin;
# one working (a spinner frame in its title) does
def agent_run(name, title_octal, command=None):
    typed = command or f"printf '\\\\033]0;{title_octal} Claude Code\\\\007'; sleep 3"
    (sw / f"{name}.shift").write_text("\n".join([
        "resize 1000 500", "wait_prompt",
        f"type {typed}\\n", "wait 800",
        f"dump {sw}/{name}.txt", "quit"]) + "\n")
    with (sw / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sw / f"{name}.shift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                     XDG_DATA_HOME=str(sw / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"agent {name} run failed", sw / f"{name}.log")
    return (sw / f"{name}.txt").read_text()

idle = agent_run("agent-idle", "\\\\342\\\\234\\\\263")
assert 'title="✳ Claude Code"' in idle and "running=false" in idle, idle[:2000]
busy = agent_run("agent-busy", "\\\\342\\\\240\\\\220")
assert 'title="⠐ Claude Code"' in busy and "running=true" in busy, busy[:2000]
# Codex titles itself with the bare project name while idle and leads with
# a braille spinner frame while working
(sw / "agent-bin").mkdir(exist_ok=True)
fake_codex = sw / "agent-bin" / "codex"
fake_codex.write_text("#!/bin/sh\nprintf \"\\033]0;$1\\007\"\nsleep 3\n")
fake_codex.chmod(0o755)
codex_idle = agent_run("codex-idle", "", f"{fake_codex} shift")
assert 'title="shift"' in codex_idle and "running=false" in codex_idle, codex_idle[:2000]
codex_busy = agent_run("codex-busy", "", f"{fake_codex} '\\\\342\\\\240\\\\213 shift'")
assert 'title="⠋ shift"' in codex_busy and "running=true" in codex_busy, codex_busy[:2000]

# ⌘⌫ in a program that owns the keys (Claude Code) sends ctrl+u, as in
# Warp: a raw-mode reader gets 0x15
(sw / "cmdbs.sh").write_text(
    "stty raw -echo\nb=$(dd bs=1 count=1 2>/dev/null | od -An -tx1)\n"
    "stty sane\necho \"KEY[$(echo $b)]\"\n")
(sw / "cmdbs.shift").write_text("\n".join([
    "wait_prompt", f"type sh {sw}/cmdbs.sh\\n", "wait 800", "key cmd+backspace",
    "wait 800", f"dump {sw}/cmdbs.txt", "quit"]) + "\n")
with (sw / "cmdbs.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "cmdbs.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("cmd+backspace run failed", sw / "cmdbs.log")
cmdbs = (sw / "cmdbs.txt").read_text()
assert "KEY[15]" in cmdbs, cmdbs[:2000]

# the font picker lists the embedded Hack after the default and no
# bitmap-only faces; a typed space reaches the settings search
(sw / "fonts.shift").write_text("\n".join([
    "wait_prompt", "key cmd+,", "wait 300", "type font family", "wait 200",
    "key tab", "wait 100", "key enter", "wait 300", f"dump {sw}/fonts.txt", "quit"]) + "\n")
with (sw / "fonts.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "fonts.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("font picker run failed", sw / "fonts.log")
fonts = (sw / "fonts.txt").read_text()
assert 'settings_query="font family"' in fonts, fonts[:2000]
font_opts = re.findall(r'ListBoxOption "([^"]+)"', fonts)
assert font_opts[:1] == ["JetBrains Mono"] and "Hack" in font_opts, font_opts
assert not any("Bitmap" in f for f in font_opts), font_opts

# Updates: a file:// manifest (SHIFT_UPDATE_URL) advertising a newer
# version shows the tab bar's chip; a click opens the update dialog (a
# development build cannot update itself: "Open download page"); Later
# dismisses that version, also after a relaunch. The running version or a
# malformed manifest shows no chip; About says why. Every manifest needs
# latest.json.sig, an Ed25519 signature: these are signed with a throwaway
# test key (SHIFT_UPDATE_TEST_KEY, honoured for file:// manifests only); an
# unsigned or tampered manifest shows no chip either.
def ed25519_sign(seed, msg):
    """RFC 8032 Ed25519 (pure Python: macOS's LibreSSL has no Ed25519):
    (public key, signature) of `msg` with a 32-byte seed."""
    p =2**255 - 19
    order = 2**252 + 27742317777372353535851937790883648493
    d = -121665 * pow(121666, p - 2, p) % p

    def add(P, Q):
        x1, y1, z1, t1 = P
        x2, y2, z2, t2 = Q
        a, b = (y1 - x1) * (y2 - x2) % p, (y1 + x1) * (y2 + x2) % p
        c, dd = 2 * t1 * t2 * d % p, 2 * z1 * z2 % p
        e, f, g, h = b - a, dd - c, dd + c, b + a
        return (e * f % p, g * h % p, f * g % p, e * h % p)

    def mul(s, P):
        Q = (0, 1, 1, 0)
        while s:
            if s & 1:
                Q = add(Q, P)
            P, s = add(P, P), s >> 1
        return Q

    def enc(P):
        zi = pow(P[2], p - 2, p)
        x, y = P[0] * zi % p, P[1] * zi % p
        return (y | (x & 1) << 255).to_bytes(32, "little")

    gy = 4 * pow(5, p - 2, p) % p
    w = (gy * gy - 1) * pow(d * gy * gy + 1, p - 2, p) % p
    gx = pow(w, (p + 3) // 8, p)
    if (gx * gx - w) % p:
        gx = gx * pow(2, (p - 1) // 4, p) % p
    if gx & 1:
        gx = p - gx
    G = (gx, gy, 1, gx * gy % p)
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little") & ((1 << 254) - 8) | (1 << 254)
    A = enc(mul(a, G))
    r = int.from_bytes(hashlib.sha512(h[32:] + msg).digest(), "little") % order
    R = enc(mul(r, G))
    k = int.from_bytes(hashlib.sha512(R + A + msg).digest(), "little") % order
    return A, R + ((r + k * a) % order).to_bytes(32, "little")

up_seed = os.urandom(32)
up_key = base64.b64encode(ed25519_sign(up_seed, b"")[0]).decode()

def write_signed(path, body, signed_body=None):
    """Write a manifest and its signature (of `signed_body`, default `body`)."""
    path.write_bytes(body)
    sig = ed25519_sign(up_seed, body if signed_body is None else signed_body)[1]
    path.with_name(path.name + ".sig").write_text(base64.b64encode(sig).decode() + "\n")

up = scratch / "update"
current = subprocess.run([str(binary), "--version"], capture_output=True,
                         text=True, check=True).stdout.split()[-1]
(up / "cfg/shift").mkdir(parents=True, exist_ok=True)
(up / "cfg/shift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
    + f'working_directory = {json.dumps(str(work))}\n')
manifest = {"version": "99.0.0", "date": "2026-10-12", "protocol": 1, "notes": "Newer.",
            "url": "https://github.com/botjaeger/shift-releases/releases/tag/v99.0.0",
            "assets": {}}
newer = json.dumps(manifest).encode()
write_signed(up / "newer.json", newer)
write_signed(up / "same.json", json.dumps(dict(manifest, version=current)).encode())
write_signed(up / "bad.json", b"{not json")
(up / "unsigned.json").write_bytes(newer)
# signed, then changed (98 -> 99): the signature no longer matches
write_signed(up / "forged.json", newer, newer.replace(b"99.0.0", b"98.0.0"))
about_steps = ["key cmd+p", "type About Shift", "wait 200", "key enter", "wait 400"]

def up_run(name, data, url, steps):
    (up / f"{name}.shift").write_text(
        "\n".join(["resize 1000 700", "wait_prompt", "wait 800", *steps, "quit"]) + "\n")
    with (up / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(up / f"{name}.shift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(up / "cfg"),
                                     XDG_DATA_HOME=str(up / data),
                                     SHIFT_UPDATE_URL=f"file://{up / url}",
                                     SHIFT_UPDATE_TEST_KEY=up_key,
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"update {name} run failed", up / f"{name}.log")

def up_state(name):
    return (up / f"{name}.txt").read_text()

up_run("probe", "data-new", "newer.json", [f"dump {up}/probe.txt"])
probe = up_state("probe")
assert 'update=available:99.0.0 chip=Some("99.0.0")' in probe, probe[:3000]
assert 'Button "Update Shift, v99.0.0 available"' in probe, probe[-3000:]
tips = re.search(r"^tips=(\[.*\])$", probe, re.M)[1]
x, y, w, h = map(float, re.search(r"v99\.0\.0 available@([\d.,-]+)", tips)[1].split(","))
up_run("click", "data-new", "newer.json", [
    f"move {x + w / 2:.0f} {y + h / 2:.0f}", "wait 900", f"dump {up}/hover.txt",
    f"click {x + w / 2:.0f} {y + h / 2:.0f}", "wait 2500", f"dump {up}/dialog.txt",
    "key escape", "wait 200", f"dump {up}/later.txt",
    *about_steps, f"dump {up}/about-new.txt"])
assert 'tooltip=Some("v99.0.0 available")' in up_state("hover"), up_state("hover")[:2000]
dialog = up_state("dialog")
assert "confirm=Some((Update(UpdatePlan" in dialog and "open_page: true" in dialog, dialog[:3000]
assert 'AlertDialog "Update to Shift v99.0.0?"' in dialog, dialog[-3000:]
assert 'Button "Later"' in dialog and 'Button "Open download page"' in dialog, dialog[-3000:]
later = up_state("later")
assert 'confirm=None' in later and 'chip=None dismissed=Some("99.0.0")' in later, later[:3000]
about_new = up_state("about-new")
assert 'Button "Update to v99.0.0 available"' in about_new, about_new[-3000:]
# dismissed stays dismissed across a relaunch (update.json)
up_run("relaunch", "data-new", "newer.json", [f"dump {up}/relaunch.txt"])
assert 'update=available:99.0.0 chip=None dismissed=Some("99.0.0")' in up_state("relaunch")
up_run("same", "data-same", "same.json", [*about_steps, f"dump {up}/same.txt"])
same = up_state("same")
assert "update=up-to-date chip=None" in same and 'Label "Up to date"' in same, same[:3000]
up_run("bad", "data-bad", "bad.json", [*about_steps, f"dump {up}/bad.txt"])
bad = up_state("bad")
assert "update=failed:" in bad and "chip=None" in bad, bad[:3000]
assert 'Label "Couldn\'t check for updates"' in bad, bad[-3000:]
for name, why in [("unsigned", "release signature is missing"),
                  ("forged", "release signature is invalid")]:
    up_run(name, f"data-{name}", f"{name}.json", [*about_steps, f"dump {up}/{name}.txt"])
    text = up_state(name)
    assert f'update=failed:"{why}" chip=None' in text, text[:3000]
    assert 'Label "Couldn\'t check for updates"' in text, text[-3000:]

# prepare-restart (an update asking the other windows to quit) needs the
# token of an update in flight: a program in a pane sending it at the
# default "navigate" level, without a token or with a made-up one, is
# refused and the window stays. The request runs in the background: the
# window answers on its UI thread, which runs the script.
(up / "rawctl.py").write_text(
    "import pathlib, socket, sys\n"
    "out = pathlib.Path(sys.argv[3])\n"
    "out.write_text('no control socket in ' + sys.argv[1])\n"
    "for sock in sorted(pathlib.Path(sys.argv[1]).glob('*.sock')):\n"
    "    s = socket.socket(socket.AF_UNIX)\n"
    "    s.settimeout(10)\n"
    "    s.connect(str(sock))\n"
    "    s.sendall(sys.argv[2].encode() + b'\\n')\n"
    "    out.write_text(s.makefile().readline())\n"
    "pathlib.Path(str(out) + '.done').touch()\n")
restart_steps = []
for i, request in enumerate(['{"cmd":"prepare-restart"}',
                             '{"cmd":"prepare-restart","token":"0123456789abcdef"}']):
    out = up / f"restart-{i}.json"
    restart_steps += [
        "sh python3 " + " ".join(shlex.quote(str(a)) for a in
                                 (up / "rawctl.py", up / "data-rr/shift/control", request, out))
        + " >/dev/null 2>&1 &",
        f"wait_file {out}.done 15000", "wait 500"]
up_run("restart", "data-rr", "same.json", [*restart_steps, f"dump {up}/restart.txt"])
for i in range(2):
    answer = (up / f"restart-{i}.json").read_text()
    assert answer.startswith("{"), answer
    refused = json.loads(answer)
    assert refused.get("ok") is False and "no update in progress" in refused.get("error", ""), refused
assert "update=" in up_state("restart"), "the window quit on a refused prepare-restart"
assert not list((up / "data-rr/shift").glob("restart/*.toml")), "a refused request saved a layout"

# a window an interrupted update saved (restart/<pid>.toml) and never
# reopened comes back on the next plain launch, and its file is deleted
left = up / "data-left/shift/restart"
left.mkdir(parents=True)
(left / "99999.toml").write_text('[[tabs]]\ncwd = "/tmp"\ntitle = "LEFTOVER"\n')
(up / "cfg-left").mkdir()
(up / "cfg-left/shift").mkdir()
(up / "cfg-left/shift/config.toml").write_text("durable_sessions = false\n")
(up / "left-window.shift").write_text(f"wait_prompt\nwait 300\ndump {up}/left-window.txt\nquit\n")
(up / "left-main.shift").write_text("wait_prompt\nwait 800\nquit\n")
with (up / "left-main.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(up / "left-main.shift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(up / "cfg-left"),
                                 XDG_DATA_HOME=str(up / "data-left"),
                                 SHIFT_PERSIST_SESSION="1",
                                 SHIFT_RELAUNCH_WINDOW_SCRIPT=str(up / "left-window.shift")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("leftover-window run failed", up / "left-main.log")
deadline = time.monotonic() + 30
while not (up / "left-window.txt").exists():
    assert time.monotonic() < deadline, died("a leftover restart file was not reopened",
                                             up / "left-main.log")
    time.sleep(0.1)
assert 'title="LEFTOVER"' in (up / "left-window.txt").read_text()
assert not list(left.glob("*.toml")), "the reopened window kept its restart file"

for client in clients:
    client.close()
stalled.close()
print(f"UI regressions passed ({time.monotonic() - started:.1f}s); {scratch}")
