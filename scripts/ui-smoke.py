"""Regression checks through Rift's real input routing, in scratch directories."""
import atexit
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import signal
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

# A run started from a Rift pane inherits that pane's shell integration
# (ZDOTDIR points the scripted zsh at the real ~/.zshrc instead of the
# fixture HOME's); scripted shells must start from the fixtures alone
for inherited in ("ZDOTDIR", "RIFT_ORIG_ZDOTDIR", "RIFT_INTEGRATION_BASH",
                  "RIFT_INTEGRATION_ZSH", "RIFT_PANE_ID", "RIFT_PANE_URL"):
    os.environ.pop(inherited, None)

# --list, --only NAME[,NAME] and --shard K/N run the sections below in groups
# (scripts/ui_smoke_sections.py); without them the file runs top to bottom
if len(sys.argv) > 2:
    sys.dont_write_bytecode = True
    import ui_smoke_sections
    sys.exit(ui_smoke_sections.main(__file__, sys.argv[1:]))

repo = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
scratch = Path(tempfile.mkdtemp(prefix="rift-ui-", dir="/tmp"))

def remove_launchd_jobs():
    # runs with their own data directory start their helpers as one-shot
    # launchd jobs (<data>/launchd/<label>.plist) that stay loaded after
    # they end: unload this run's, pass or fail
    for plist in scratch.glob("**/launchd/dev.rift.*.plist"):
        try:
            subprocess.run(["launchctl", "bootout", f"gui/{os.getuid()}/{plist.stem}"],
                           capture_output=True, timeout=10)
        except subprocess.TimeoutExpired:
            pass
if sys.platform == "darwin":
    atexit.register(remove_launchd_jobs)
print(f"UI artifacts: {scratch}", flush=True)
cfg = scratch / "cfg" / "rift"
data = scratch / "data" / "rift"
work = scratch / "work"
for directory in (cfg / "themes", data / "control", work):
    directory.mkdir(parents=True, exist_ok=True)
(work / "hello.rs").write_text("fn main() {}\n")
theme = (repo / "crates/rift-core/src/theme/dark-default.toml").read_text()
(cfg / "themes/audit.toml").write_text(theme)
# the same theme with another ground, whatever the builtin's value is
(scratch / "theme-next.toml").write_text(re.sub(r'(?m)^bg = "#[0-9a-f]{6}"', 'bg = "#232326"', theme, count=1))
assert (scratch / "theme-next.toml").read_text() != theme, "the theme edit changed nothing"
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

def session_daemons(data_dir):
    """Pids of `--session-daemon`s run with XDG_DATA_HOME=data_dir (the CI
    image has no `ps`: /proc there)."""
    mark = f"XDG_DATA_HOME={data_dir}"
    if Path("/proc/self/environ").exists():
        for p in Path("/proc").glob("[0-9]*"):
            try:
                if b"--session-daemon" in (p / "cmdline").read_bytes() and \
                        mark.encode() in (p / "environ").read_bytes().split(b"\0"):
                    yield int(p.name)
            except OSError:
                pass
        return
    ps = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    for line in ps.splitlines():
        if "--session-daemon" in line:
            pid = line.split()[0]
            own = subprocess.run(["ps", "eww", "-p", pid], capture_output=True, text=True).stdout
            if mark + " " in own or own.rstrip().endswith(mark):
                yield int(pid)

def died(what, log):
    # CI keeps no artifacts: the log tail is the only trace of the cause
    tail = "\n".join(Path(log).read_text(errors="replace").splitlines()[-20:])
    return f"{what}; {log}:\n{tail}"

def unicode_dump(path):
    # Rust Debug escapes combining marks; compare the actual fixture text.
    return re.sub(r"\\u\{([0-9a-fA-F]+)\}", lambda m: chr(int(m[1], 16)), Path(path).read_text())

def raw_input_fixture(base):
    """A real PTY program recording input bytes until its scratch stop file."""
    (base / "cfg/rift").mkdir(parents=True)
    (base / "data").mkdir()
    (base / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        'shell_completions = false\nsync_os_theme = false\n')
    (base / "record.py").write_text(
        "import os, pathlib, select, sys, termios, time, tty\n"
        "base = pathlib.Path(sys.argv[1]); flags = int(sys.argv[2])\n"
        "old = termios.tcgetattr(0); tty.setraw(0)\n"
        "try:\n"
        "  if flags: os.write(1, f'\\x1b[>{flags}u'.encode())\n"
        "  (base / 'bytes').write_bytes(b''); (base / 'ready').touch()\n"
        "  until = time.monotonic() + 30\n"
        "  while not (base / 'stop').exists() and time.monotonic() < until:\n"
        "    if select.select([0], [], [], 0.05)[0]:\n"
        "      chunk = os.read(0, 4096)\n"
        "      if not chunk: break\n"
        "      with (base / 'bytes').open('ab') as out: out.write(chunk)\n"
        "finally:\n"
        "  if flags: os.write(1, b'\\x1b[<u')\n"
        "  termios.tcsetattr(0, termios.TCSANOW, old)\n")
    return dict(env, XDG_CONFIG_HOME=str(base / "cfg"), XDG_DATA_HOME=str(base / "data"))

# === sections: the preamble above runs in every shard (scripts/ui_smoke_sections.py) ===

# --- first run: palette, viewer, config reload, completion, find, paste, focus ---
steps = [
    "wait_prompt",
    "key escape",
    "type echo rift-audit-ok\\n",
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
    # an assistive technology's Focus never presses a control; Click does
    "a11y focus Match case",
    f"dump {scratch}/find-a11y-focus.txt",
    "a11y click Match case",
    f"dump {scratch}/find-a11y-click.txt",
    "a11y click Match case",
    # ⌘A ⌘C copies the query; an unavailable clipboard says so
    "fault clipboard",
    "key cmd+a",
    "key cmd+c",
    f"dump {scratch}/fault-copy.txt",
    "fault none",
    "key escape",
    # Enter typed ahead of a confirmation does not accept it
    "type sleep 30\\n",
    "wait 400",
    "key cmd+w",
    "key enter",
    f"dump {scratch}/confirm.txt",
    # Focus on the destructive button keeps the dialog; Click on Cancel closes it
    "a11y focus Close",
    "wait 500",
    f"dump {scratch}/confirm-a11y-focus.txt",
    "a11y click Cancel",
    f"dump {scratch}/confirm-a11y-cancel.txt",
    "key escape",
    "key ctrl+c",
    "wait 300",
    # Settings controls work from the keyboard (it opens on General; ↓ ↓ to
    # Appearance, past Profiles)
    "key cmd+,",
    "wait 200",
    "key down",
    "key down",
    "wait 100",
    # font size, not font family: a machine with one monospace font has
    # nothing to cycle to
    "key tab",
    "key tab",
    f"dump {scratch}/settings-focus.txt",
    cp(cfg / "config.toml", scratch / "settings-before.toml"),
    "key space",
    "wait 300",
    cp(cfg / "config.toml", scratch / "settings-after.toml"),
    # About Rift opens Settings › About; its copy button copies the version
    "key cmd+p",
    "type About Rift",
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
    # an empty paste (image-only clipboard) reaches bracketed programs only:
    # Claude Code reads the clipboard image on it
    "paste",
    "wait 400",
    f"dump {scratch}/sync-empty.txt",
    # files copied in the file manager paste as their quoted paths, images
    # too (Claude Code and Codex attach an image from its path)
    f"copyfiles {work}/my file.txt|{work}/hello.rs",
    "key cmd+v",
    "wait 400",
    f"dump {scratch}/sync-files.txt",
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
script = scratch / "smoke.rift"
script.write_text("\n".join(steps) + "\n")
env = dict(os.environ, XDG_CONFIG_HOME=str(scratch / "cfg"),
           XDG_DATA_HOME=str(scratch / "data"), RIFT_NO_NOTIFY="1")
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
    # Linux keeps a socket with automation off, for rift:// links only
    # (control.rs Level::Links); macOS has none.
    if sys.platform == "darwin":
        assert not (data / f"control/{app.pid}.sock").exists(), "disabled automation kept its socket"
    assert app.wait(timeout=60) == 0, died("app failed", scratch / "app.log")

def state(name):
    return (scratch / f"{name}.txt").read_text()

assert "editor_active=true" in state("prompt"), state("prompt")
assert "headless=true" in state("prompt") and "window_visible=Some(true)" not in state("prompt"), \
    state("prompt")
assert "rift-audit-ok" in state("prompt"), state("prompt")
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
assert 'editor="cat ./my\\\\ file.txt "' in state("complete"), state("complete")
assert "one-arg-ok" in state("complete-run"), state("complete-run")
# the closing quote belongs to the word, which ends the line: the one match
# gets a space after it (Warp); mid-line it does not
assert 'editor="cat ./my\\\\ file.txt " cursor=19' in state("complete-quoted"), state("complete-quoted")
assert 'editor="cat ./my\\\\ file.txt --other" cursor=18' in state("complete-mid"), state("complete-mid")
assert 'editor="a\\u{3000}b" cursor=4' in state("word-left"), state("word-left")
assert f'{scratch}/a/src/only_a.rs"' in state("hints"), state("hints")
found = re.search(r"matches=(\d+) total=(\d+)", state("find"))
assert found and found[1] == "500" and int(found[2]) >= 600, state("find")
assert "confirm=Some" in state("confirm") and "tabs=1" in state("confirm"), state("confirm")
assert 'Button "Match case" toggled=False' in state("find-a11y-focus"), state("find-a11y-focus")
assert 'Button "Match case" toggled=True' in state("find-a11y-click"), state("find-a11y-click")
assert 'Clipboard unavailable' in state("fault-copy"), state("fault-copy")
assert "confirm=Some" in state("confirm-a11y-focus") and "tabs=1" in state("confirm-a11y-focus"), \
    state("confirm-a11y-focus")
assert "confirm=None" in state("confirm-a11y-cancel") and "tabs=1" in state("confirm-a11y-cancel"), \
    state("confirm-a11y-cancel")
assert 'settings_focus=Some(SettingsStep("font_size"' in state("settings-focus"), state("settings-focus")
assert (scratch / "settings-before.toml").read_text() != (scratch / "settings-after.toml").read_text(), \
    "Space did not press the focused Settings control"
about = state("about")
assert "settings=true page=About" in about, about
assert "settings_focus=Some(SettingsCopyVersion)" in about, about
assert 'toast=Some("Copied version")' in about, about
# a manual check is offered even before any check ran
assert 'Button "Check for updates now"' in about, about
assert re.fullmatch(r"v\d+\.\d+\.\d+.*", json.loads(next(
    l for l in about.splitlines() if l.startswith("clipboard="))[len("clipboard="):])), about
settings_after = (scratch / "settings-after.toml").read_text()
assert settings_after.startswith("# keep this comment\n"), settings_after
assert "future_option = 1" in settings_after, settings_after
assert "panes=2" in state("split"), state("split")
sync = state("sync").split("\n  pane ")
assert "| zz" in sync[1] and "200~zz" not in sync[1], "plain pane got a bracketed paste"
assert "| ^[[200~zz^[[201~" in sync[2], "bracketed pane lost its paste framing"
empty = state("sync-empty").split("\n  pane ")
assert "| zz" in empty[1] and "^[[200~^[[201~" not in empty[1], "plain pane got an empty paste"
assert "| ^[[200~zz^[[201~^[[200~^[[201~" in empty[2], "bracketed pane missed the empty paste"
files_line = f"^[[200~'{work}/my file.txt' {work}/hello.rs ^[[201~"
assert files_line in state("sync-files").replace("\n    | ", ""), state("sync-files")[-2000:]
assert "^[[O^[[I" in state("focus-tabs"), state("focus-tabs")
wide = re.search(r"matches=(\d+) total=(\d+)", state("find-wide"))
assert wide and int(wide[2]) >= 1, state("find-wide")
assert "wait_prompt timed out" not in (scratch / "app.log").read_text()
# --- second run: fresh state, fixed 1000x700 window, mouse and hint geometry ---
geo = scratch / "geo"
(geo / "cfg/rift").mkdir(parents=True)
(geo / "data/rift").mkdir(parents=True)
(geo / "cfg/rift/config.toml").write_text(
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
    # What's New is reachable from Settings (General › Updates)
    "key backspace",
    "key backspace",
    "key backspace",
    "key backspace",
    "type what",
    "wait 100",
    f"dump {geo}/settings-whatsnew.txt",
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
    "wait_prompt",
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
(geo / "geo.rift").write_text("\n".join(geo_steps) + "\n")
geo_env = dict(env, XDG_CONFIG_HOME=str(geo / "cfg"), XDG_DATA_HOME=str(geo / "data"))
with (geo / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(geo / "geo.rift"), "--headless"],
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
    # 40 identical full rows fill the pane, so the drag's row is output on
    # every platform's metrics
    f"type s=$(printf '%0{cols - 1}d9' 0); for i in $(seq 40); do echo $s; done\\n",
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
(geo / "edge.rift").write_text("\n".join(edge_steps) + "\n")
with (geo / "edge.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(geo / "edge.rift"), "--headless"],
                        env=geo_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, f"right-edge run failed; see {geo}/edge.log"
# --- pane sizing like Warp: shares, 10 px key steps, 50 px minimum ---
sizing = scratch / "sizing"
(sizing / "cfg/rift").mkdir(parents=True)
(sizing / "data/rift").mkdir(parents=True)
(sizing / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(work))}\n'
    '[keybindings]\nequalize_panes = "ctrl+cmd+e"\n')
sizing_steps = [
    "wait_prompt",
    "focus on",
    "resize 1000 700",
    "wait 300",
    # ⌘D twice: one row of three equal panes (not a half and two quarters)
    "key cmd+d",
    "wait 1500",
    "key cmd+d",
    "wait 1500",
    f"dump {sizing}/three.txt",
    # ⌃⌘← twice on the last pane: the divider before it moves 2 × 10 px
    "key ctrl+cmd+left",
    "key ctrl+cmd+left",
    "wait 100",
    f"dump {sizing}/keys.txt",
    # double-clicking that divider gives the row equal shares again
    "dblclick 646 300",
    "wait 100",
    f"dump {sizing}/dblclick.txt",
    # drag the first divider to x=200, then far past the left edge: it stops
    # 50 px in, and only the two panes beside it change
    "press 333 300",
    "move 340 300",
    "move 200 300",
    "release 200 300",
    "wait 100",
    f"dump {sizing}/drag.txt",
    "press 200 300",
    "move 190 300",
    "move 5 300",
    "release 5 300",
    "wait 100",
    f"dump {sizing}/drag-min.txt",
    # closing the middle pane shares its space among the other two
    "key alt+cmd+left",
    "key cmd+w",
    "wait 500",
    f"dump {sizing}/closed.txt",
    # a wider window keeps each pane's share
    "resize 1400 700",
    "wait 300",
    f"dump {sizing}/wide.txt",
    # equalize_panes (bound here; Warp has no default key either)
    "key ctrl+cmd+e",
    "wait 100",
    f"dump {sizing}/equalized.txt",
    # ⇧⌘D across the row: the pane's column splits in halves
    "key shift+cmd+d",
    "wait 1500",
    f"dump {sizing}/cross.txt",
    "quit",
]
(sizing / "sizing.rift").write_text("\n".join(sizing_steps) + "\n")
sizing_env = dict(env, XDG_CONFIG_HOME=str(sizing / "cfg"), XDG_DATA_HOME=str(sizing / "data"))
with (sizing / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sizing / "sizing.rift"), "--headless"],
                        env=sizing_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("pane sizing run failed", sizing / "app.log")

def pane_rects(name):
    """id → (x, y, w, h) of the active tab's panes, in layout order."""
    text = (sizing / f"{name}.txt").read_text()
    return {int(m[0]): tuple(float(v) for v in m[1:]) for m in re.findall(
        r"^  pane_rect (\d+) x=([\d.]+) y=([\d.]+) w=([\d.]+) h=([\d.]+)$", text, re.M)}

def widths(name):
    return [r[2] for r in pane_rects(name).values()]

three = widths("three")
assert len(three) == 3 and max(three) - min(three) <= 1 and sum(three) == 998, three
keys = widths("keys")
assert keys == [three[0], three[1] - 20, three[2] + 20], (three, keys)
assert max(widths("dblclick")) - min(widths("dblclick")) <= 1, widths("dblclick")
drag = widths("drag")
assert drag[0] == 200 and drag[2] == three[2] and sum(drag) == 998, drag
drag_min = widths("drag-min")
assert drag_min[0] == 50 and drag_min[2] == three[2], drag_min
closed = widths("closed")
assert len(closed) == 2 and sum(closed) == 999, closed
# the closed pane's space went to both: 50 : 333 stays 50 : 333
assert abs(closed[0] / closed[1] - drag_min[0] / drag_min[2]) < 0.01, (drag_min, closed)
wide = widths("wide")
assert sum(wide) == 1399 and abs(wide[0] / wide[1] - closed[0] / closed[1]) < 0.01, (closed, wide)
assert abs(widths("equalized")[0] - widths("equalized")[1]) <= 1, widths("equalized")
cross = pane_rects("cross")
assert len(cross) == 3, cross
column = [r for r in cross.values() if r[0] == 0]
assert len(column) == 2 and column[0][2] == column[1][2] and abs(column[0][3] - column[1][3]) <= 1, cross
assert re.search(r"layout=Split \{ axis: Horizontal, children: \[\(1\.0, Split \{ axis: Vertical, "
                 r"children: \[\(1\.0, Leaf\(\d+\)\), \(1\.0, Leaf\(\d+\)\)\] \}\), "
                 r"\(1\.0, Leaf\(\d+\)\)\] \}", (sizing / "cross.txt").read_text()), \
    (sizing / "cross.txt").read_text()[:900]
assert clipboard("right-edge").rstrip("\n") == full, clipboard("right-edge")
# the drag selected text, not the block; the plain click then selects it
assert "selected_block=None" in gstate("right-edge"), gstate("right-edge")[:900]
assert "selected_block=Some(" in gstate("output-click"), gstate("output-click")[:900]
assert re.search(r"^tab 0 .* running=true( |$)", gstate("tab-running"), re.M), gstate("tab-running")[:900]
assert re.search(r"^tab 0 .* running=false( |$)", gstate("tab-done"), re.M), gstate("tab-done")[:900]
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
split = re.search(r"layout=Split \{ axis: Horizontal, children: \[\([\d.]+, Leaf\((\d+)\)\), "
                  r"\([\d.]+, Leaf\((\d+)\)\)\] \}", gstate("pane-split"))
assert split, gstate("pane-split")[:900]
left, right = split[1], split[2]
assert "mouse_cursor=Grabbing" in gstate("pane-dragging"), gstate("pane-dragging")[:300]
assert re.search(rf"layout=Split \{{ axis: Vertical, children: \[\([\d.]+, Leaf\({right}\)\), "
                 rf"\([\d.]+, Leaf\({left}\)\)\] \}}", gstate("pane-below")), gstate("pane-below")[:900]
moved = gstate("pane-tab")
assert "tabs=2 active=1" in moved, moved[:300]
assert f"layout=Leaf({right})" in moved and f"layout=Leaf({left})" in moved, moved[:900]
def tab_leaves(name):
    return re.findall(r"^tab \d+ .*?\n  layout=(.*)$", gstate(name), re.M)

assert tab_leaves("tabs-dragging") != tab_leaves("pane-tab"), gstate("tabs-dragging")[:600]
assert tab_leaves("tabs-esc") == tab_leaves("pane-tab"), gstate("tabs-esc")[:600]
swap = re.search(r"layout=Split \{ axis: Horizontal, children: \[\([\d.]+, Leaf\((\d+)\)\), "
                 r"\([\d.]+, Leaf\((\d+)\)\)\] \}", gstate("swap-before"))
assert swap, gstate("swap-before")[:900]
assert re.search(rf"layout=Split \{{ axis: Horizontal, children: \[\([\d.]+, Leaf\({swap[2]}\)\), "
                 rf"\([\d.]+, Leaf\({swap[1]}\)\)\] \}}", gstate("swap-after")), gstate("swap-after")[:900]
history = gstate("history-open")
assert 'history=true scope=All' in history and 'history_selected=Some("echo hist1")' in history, \
    history[:1200]
assert "history=true scope=Failed" in gstate("history-scope"), gstate("history-scope")[:1200]
assert "palette=true mode=History" in gstate("palette-prefix"), gstate("palette-prefix")[:300]
assert "palette=true mode=Sessions" in gstate("palette-tab"), gstate("palette-tab")[:300]
assert 'settings=true' in gstate("settings-search") and \
    'settings_tab=true settings_query="curs"' in gstate("settings-search"), gstate("settings-search")[:1200]
assert 'Button "Open What\'s New"' in gstate("settings-whatsnew"), gstate("settings-whatsnew")[:2000]
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
for directory in ("cfg/rift", "data/rift", "bin", "home"):
    (edge / directory).mkdir(parents=True)
(edge / "cfg/rift/config.toml").write_text(
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
    "wait_prompt",
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
    # a line submitted before the shell started the previous one keeps
    # its own label, and does not relabel that one
    "type echo 'a  b'\\necho TYPED_AHEAD\\n",
    "wait 600",
    f"dump {edge}/typed-ahead.txt",
    # find counts only drawn rows (a prompt echo is drawn as the block
    # header) and scrolls by drawn rows: a collapsed block between the
    # match and the bottom used to scroll the match off screen
    "type seq 1 200\\n",
    "wait 400",
    "type echo JUMP_NEEDLE\\n",
    "wait 400",
    "type seq 1 400\\n",
    "wait 600",
    "key cmd+up",
    "key ctrl+m",
    "wait 200",
    *["key down"] * 9,
    "key enter",
    "wait 300",
    # a selected block scopes find to it: clear the selection so the search
    # covers the pane, as this check intends
    "key escape",
    "key cmd+f",
    "type JUMP_NEEDLE",
    "wait 400",
    f"dump {edge}/find-jump.txt",
    "key escape",
    # a child shell keeps the block integration
    "type exec bash\\n",
    "wait 800",
    "type echo INNER_OK\\n",
    "wait 500",
    f"dump {edge}/child.txt",
    "quit",
]
(edge / "edge.rift").write_text("\n".join(edge_steps) + "\n")
edge_env = dict(env, XDG_CONFIG_HOME=str(edge / "cfg"), XDG_DATA_HOME=str(edge / "data"),
                HOME=str(edge / "home"), BASH_SILENCE_DEPRECATION_WARNING="1",
                PATH=f"{edge / 'bin'}:{os.environ['PATH']}")
with (edge / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(edge / "edge.rift"), "--headless"],
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
ahead = estate("typed-ahead")
assert "cmd=\"echo 'a  b'\" exit=Some(0)" in ahead and 'cmd="echo TYPED_AHEAD" exit=Some(0)' in ahead, ahead
jump = estate("find-jump")
assert re.search(r'cmd="seq 1 400" exit=Some\(0\) collapsed=true', jump), jump
assert re.search(r"matches=1 total=1 ", jump), jump
needle_row = re.search(r'cmd="echo JUMP_NEEDLE" .*? output_start=Some\((\d+)\)', jump)
drawn = re.search(r"geometry .*? rows=\[([^\]]*)\]", jump)
assert needle_row and drawn and any(r.startswith(f"{needle_row[1]}:") for r in drawn[1].split(" ")), jump
child = estate("child")
assert 'cmd="echo INNER_OK" exit=Some(0)' in child and "editor_active=true" in child, child

# welcome lifetime: clearing command blocks must not revive onboarding
wl = scratch / "welcome-lifetime"
for directory in (wl / "cfg/rift", wl / "data"):
    directory.mkdir(parents=True)
(wl / "cfg/rift/config.toml").write_text(config.replace('theme = "audit"', 'theme = "dark-default"'))
(wl / "test.rift").write_text("\n".join([
    "resize 1000 700", "wait_prompt", "wait 300",
    f"dump {wl}/fresh.txt",
    "type echo WELCOME_LIFETIME\\n", "wait_prompt", "wait 500",
    f"dump {wl}/used.txt",
    "key cmd+k", "wait 300",
    f"dump {wl}/cleared.txt",
    "type printf AFTER_CLEAR\\n", "wait_prompt", "wait 500",
    f"dump {wl}/after.txt", "quit",
]) + "\n")
with (wl / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(wl / "test.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(wl / "cfg"),
                                 XDG_DATA_HOME=str(wl / "data")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("welcome run failed", wl / "app.log")
for name, count in (("fresh", 5), ("used", 0), ("cleared", 0), ("after", 0)):
    snapshot = (wl / f"{name}.txt").read_text()
    assert f"welcome_rows={count}" in snapshot, snapshot
assert "AFTER_CLEAR" in (wl / "after.txt").read_text()

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
(rv / "rv.rift").write_text("\n".join(rv_steps) + "\n")
with (rv / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rv / "rv.rift"), "--headless"],
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
assert '"stage-hunk@957,164,20,20", "revert-hunk@977,164,20,20", "copy-hunk@997,164,20,20"' \
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

# review against a branch (merge-base → working tree, read-only) and "Send
# to agent…": a hunk pasted into a fake `claude` that records its stdin,
# bracketed, no Enter after; a target that closed gets nothing
rb = scratch / "review-base"
for d in ("repo", "bin", "cfg/rift", "data", "home"):
    (rb / d).mkdir(parents=True)
# an empty home: Ubuntu's stock ~/.bashrc titles the window `user@host: cwd`
# through PS1, which would replace the cwd-named pane labels asserted below
for rc in (".bash_profile", ".bashrc"):
    (rb / "home" / rc).write_text("")
rb_git = ["git", "-C", str(rb / "repo"), "-c", "user.name=t", "-c", "user.email=t@e.x",
          "-c", "commit.gpgsign=false"]
def rb_lines(edits):
    return "".join(edits.get(n, f"line {n}") + "\n" for n in range(1, 31))
subprocess.run(["git", "init", "-q", "-b", "main", str(rb / "repo")], check=True)
(rb / "repo/a.rs").write_text(rb_lines({}))
subprocess.run([*rb_git, "add", "-A"], check=True)
subprocess.run([*rb_git, "commit", "-qm", "init"], check=True)
subprocess.run([*rb_git, "checkout", "-qb", "feature"], check=True)
(rb / "repo/b.rs").write_text("fn b() {}\n")
subprocess.run([*rb_git, "add", "-A"], check=True)
subprocess.run([*rb_git, "commit", "-qm", "add b"], check=True)
(rb / "repo/a.rs").write_text(rb_lines({2: "line two"}))
subprocess.run([*rb_git, "commit", "-qam", "edit a"], check=True)
(rb / "repo/a.rs").write_text(rb_lines({2: "line two", 28: "line twenty-eight"}))
(rb / "bin/claude").write_text(
    f"#!/bin/sh\necho $PPID > {rb}/agent.pid\nprintf '\\033[?2004h'\nstty raw -echo\n"
    f"exec cat > {rb}/agent.bin\n")
(rb / "bin/claude").chmod(0o755)
(rb / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'working_directory = {json.dumps(str(rb / "repo"))}\n')
rb_steps = [
    "resize 1400 900",
    "wait_prompt",
    "key shift+cmd+g",
    "wait 1000",
    f"dump {rb}/uncommitted.txt",
    # the base chip lists Uncommitted, the default branch, every branch
    "click 1149 53",
    "wait 300",
    f"dump {rb}/picker.txt",
    "key down",
    "key enter",
    "wait 1200",
    f"dump {rb}/branch.txt",
    "key alt+cmd+g",
    "key s",
    "wait 200",
    f"dump {rb}/readonly.txt",
    "key escape",
    "key cmd+d",
    "wait 1000",
    f"type cd {rb / 'repo'}; export PATH={rb / 'bin'}:$PATH; claude\\n",
    "wait 1500",
    # a.rs's first hunk: a, then Enter on the first row (the agent)
    "key alt+cmd+g",
    "key down",
    "key right",
    "key down",
    "key a",
    "wait 300",
    f"dump {rb}/targets.txt",
    "key enter",
    "wait 800",
    f"sh cp {rb}/agent.bin {rb}/sent.bin",
    # the target closes while the picker is open
    "key alt+cmd+g",
    "key a",
    "wait 300",
    f"sh kill -9 $(cat {rb}/agent.pid)",
    "wait 2000",
    "key enter",
    "wait 500",
    f"dump {rb}/closed.txt",
    f"sh cp {rb}/agent.bin {rb}/final.bin",
    # a shell at Rift's command editor: the code lands in the editor (no
    # confirm, nothing reaches the shell)
    "key alt+cmd+g",
    "key a",
    "wait 300",
    "key enter",
    "wait 500",
    f"dump {rb}/to-editor.txt",
    "quit",
]
(rb / "rb.rift").write_text("\n".join(rb_steps) + "\n")
with (rb / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rb / "rb.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rb / "cfg"),
                                 XDG_DATA_HOME=str(rb / "data"), HOME=str(rb / "home")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("review-base run failed", rb / "app.log")
uncommitted = (rb / "uncommitted.txt").read_text()
assert 'unstaged=Some(["Ma.rs"])' in uncommitted, uncommitted
assert "review_git=base:Uncommitted vs:None" in uncommitted, uncommitted
picker = (rb / "picker.txt").read_text()
assert 'picker_rows=["Uncommitted", "Branch: main", "feature", "main"]' in picker, picker
branch = (rb / "branch.txt").read_text()
rb_main = subprocess.run([*rb_git, "rev-parse", "--short=7", "main"], capture_output=True,
                         text=True, check=True).stdout.strip()
assert 'unstaged=Some(["Ma.rs", "Ab.rs"])' in branch, branch
assert f'vs:Some("main") merge_base:Some("{rb_main}") read_only:true' in branch, branch
# no stage, unstage or revert buttons against a branch; s says why
assert not re.search(r'"(stage|unstage|revert)@', branch), branch
assert 'toast=Some("Against a branch the changes include commits' in \
    (rb / "readonly.txt").read_text()
targets = (rb / "targets.txt").read_text()
assert ('picker_rows=["Claude Code — window 1 · tab 1 · pane 2 (this pane)", '
        '"repo — window 1 · tab 1 · pane 1"]') in targets, targets
sent = (rb / "sent.bin").read_bytes()
assert sent == (b"\x1b[200~a.rs:1-5\n```rust\n line 1\n-line 2\n+line two\n line 3\n"
                b" line 4\n line 5\n```\x1b[201~"), sent
closed = (rb / "closed.txt").read_text()
assert 'toast=Some("That pane closed: nothing was sent")' in closed, closed
assert (rb / "final.bin").read_bytes() == sent
to_editor = (rb / "to-editor.txt").read_text()
assert 'toast=Some("Sent a.rs to the pane\'s command line")' in to_editor, to_editor
assert "confirm=None" in to_editor, to_editor
assert re.search(r'pane \d+ cwd=\S+ editor="a\.rs:\d+-\d+\\n```rust\\n', to_editor), to_editor

# Send's "Paste and run?" confirm writes to the chosen pane only, never to
# its synchronized siblings (⌥⌘I)
rs = scratch / "review-sync"
for d in ("bin", "cfg/rift", "data"):
    (rs / d).mkdir(parents=True)
for name in ("rec1", "rec2"):
    (rs / f"bin/{name}").write_text(f"#!/bin/sh\nstty raw -echo\nexec cat > {rs}/{name}.bin\n")
    (rs / f"bin/{name}").chmod(0o755)
(rs / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'working_directory = {json.dumps(str(rb / "repo"))}\n')
rs_steps = [
    "resize 1400 900",
    "wait_prompt",
    f"type export PATH={rs / 'bin'}:$PATH; rec1\\n",
    "wait 1000",
    "key cmd+d",
    "wait 1500",
    f"type export PATH={rs / 'bin'}:$PATH; rec2\\n",
    "wait 1000",
    "key alt+cmd+i",
    "key shift+cmd+g",
    "wait 1500",
    "key alt+cmd+g",
    "key down",
    "key a",
    "wait 300",
    f"dump {rs}/targets.txt",
    "key enter",
    "wait 800",
    f"dump {rs}/confirm.txt",
    "key tab",
    "key enter",
    "wait 1000",
    "quit",
]
(rs / "rs.rift").write_text("\n".join(rs_steps) + "\n")
with (rs / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rs / "rs.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rs / "cfg"),
                                 XDG_DATA_HOME=str(rs / "data"), HOME=str(rb / "home")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("review-sync run failed", rs / "app.log")
rs_targets = (rs / "targets.txt").read_text()
assert '"repo — window 1 · tab 1 · pane 1", ' in rs_targets, rs_targets
assert "confirm=Some((Paste" in (rs / "confirm.txt").read_text()
assert (rs / "rec1.bin").read_bytes().startswith(b"a.rs:"), (rs / "rec1.bin").read_bytes()[:200]
assert (rs / "rec2.bin").read_bytes() == b"", "a synchronized sibling got the Send paste"

# a commit whose window closed before it finished: the answer is dropped,
# never shown (or its busy state cleared) in another window
rl = scratch / "review-late"
for d in ("cfg/rift", "data"):
    (rl / d).mkdir(parents=True)
rl_git = ["git", "-C", str(rl / "repo"), "-c", "user.name=t", "-c", "user.email=t@e.x"]
subprocess.run(["git", "init", "-q", "-b", "main", str(rl / "repo")], check=True)
for k, v in (("commit.gpgsign", "false"), ("core.hooksPath", ".git/hooks"),
             ("user.name", "t"), ("user.email", "t@e.x")):
    subprocess.run([*rl_git, "config", k, v], check=True)
(rl / "repo/a.txt").write_text("a\n")
subprocess.run([*rl_git, "add", "-A"], check=True)
subprocess.run([*rl_git, "commit", "-qm", "init"], check=True)
(rl / "repo/b.txt").write_text("b\n")
subprocess.run([*rl_git, "add", "b.txt"], check=True)
(rl / "repo/.git/hooks/pre-commit").write_text("#!/bin/sh\nsleep 3\n")
(rl / "repo/.git/hooks/pre-commit").chmod(0o755)
(rl / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'working_directory = {json.dumps(str(rl / "repo"))}\n')
rl_steps = [
    "resize 1400 900",
    "wait_prompt",
    "key shift+cmd+g",
    "wait 1200",
    "click 1170 848",
    "type late",
    "key cmd+enter",
    "wait 300",
    "new_window",
    "wait 1500",
    "window 0",
    "close_window",
    "wait 4000",
    f"dump {rl}/after.txt",
    "quit",
]
(rl / "rl.rift").write_text("\n".join(rl_steps) + "\n")
with (rl / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rl / "rl.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rl / "cfg"),
                                 XDG_DATA_HOME=str(rl / "data")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("review-late run failed", rl / "app.log")
rl_after = (rl / "after.txt").read_text()
assert "windows=1 " in rl_after and "toast=None" in rl_after, rl_after
rl_log = subprocess.run([*rl_git, "log", "-1", "--format=%s"], capture_output=True,
                        text=True, check=True).stdout.strip()
assert rl_log == "late", rl_log

# DEC double-height (ESC # 3/4) and double-width (ESC # 6) lines draw every
# cell two columns wide: a double click maps the pointer to the word drawn
# under it, not to the single-width column there
dec = scratch / "dec"
for sub in ("cfg/rift", "data"):
    (dec / sub).mkdir(parents=True, exist_ok=True)
# the pointer positions below assume blocks from the top of the pane
(dec / "cfg/rift/config.toml").write_text('blocks_start = "top"\n')
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
    # the click selected the block, and ⌘A with a block selected takes
    # every block: Esc drops it so ⌘A selects the command line
    "key escape",
    "key cmd+a",
    "key cmd+v",
    "wait 200",
    f"dump {dec}/tall.txt",
    "quit",
]
(dec / "dec.rift").write_text("\n".join(dec_steps) + "\n")
with (dec / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(dec / "dec.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(dec / "cfg"),
                                 XDG_DATA_HOME=str(dec / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("DEC line run failed", dec / "app.log")
wide = (dec / "wide.txt").read_text()
assert 'editor="width"' in wide, wide
tall = (dec / "tall.txt").read_text()
assert 'editor="Double"' in tall, tall

# smart double-click: a URL, path:line:col, email, IPv4/IPv6, number, SHA
# or UUID is selected whole, trailing punctuation and brackets left out; a
# plain word keeps word selection, which `word_chars` widens (hot reload)
sm = scratch / "smart-click"
for sub in ("cfg/rift", "data"):
    (sm / sub).mkdir(parents=True, exist_ok=True)
sm_config = 'blocks_start = "top"\nshell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
(sm / "cfg/rift/config.toml").write_text(sm_config)
sm_lines = ["see https://example.com/a?b=1. ok", "at src/main.rs:42:7: oops",
            "mail <dev.ops+x@mail.example.com>", "ip 192.168.0.12: fe80::1ff:fe23:4567:890a",
            "n 3.1415 0xFF00 4ff62eb", "id 123e4567-e89b-12d3-a456-426614174000",
            "plain foo:bar baz",
            # soft-wrapped over two rows: clicked on its second it is whole
            "https://example.com/" + "w" * 150 + "/end"]
(sm / "lines.txt").write_text("\n".join(sm_lines) + "\n")
def sm_run(name, steps):
    (sm / f"{name}.rift").write_text("\n".join(
        ["resize 900 500", "wait_prompt", f"type cat {sm / 'lines.txt'}\\n", "wait 800",
         *steps, "quit"]) + "\n")
    with (sm / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sm / f"{name}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(sm / "cfg"), XDG_DATA_HOME=str(sm / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"smart click {name} run failed", sm / f"{name}.log")
sm_run("probe", [f"dump {sm}/probe.txt"])
sm_probe = (sm / "probe.txt").read_text()
sm_geo = next(l for l in sm_probe.splitlines() if l.strip().startswith("geometry "))
sm_cell = float(re.search(r"cell_w=([\d.]+)", sm_geo)[1])
sm_line_h = float(re.search(r"line_h=([\d.]+)", sm_geo)[1])
sm_rows = {int(a): (float(y), float(x)) for a, y, x in re.findall(r"(\d+):([\d.]+)@([\d.]+)", sm_geo)}
sm_start = int(re.findall(r'cmd="cat [^"]*lines.txt".*?output_start=Some\((\d+)\)', sm_probe)[-1])
def sm_at(i, needle):
    y, x = sm_rows[sm_start + i]
    return f"{x + (sm_lines[i].index(needle) + 0.5) * sm_cell:.1f} {y + sm_line_h / 2:.1f}"
sm_cases = [(0, "example", "https://example.com/a?b=1"), (1, "main", "src/main.rs:42:7"),
            (2, "ops", "dev.ops+x@mail.example.com"), (3, "168", "192.168.0.12"),
            (3, "1ff", "fe80::1ff:fe23:4567:890a"), (4, "1415", "3.1415"), (4, "FF0", "0xFF00"),
            (4, "f62", "4ff62eb"), (5, "e89b", "123e4567-e89b-12d3-a456-426614174000"),
            (6, "bar", "bar")]
sm_steps = []
for k, (i, needle, _) in enumerate(sm_cases):
    sm_steps += [f"dblclick {sm_at(i, needle)}", "wait 150", "key cmd+c", "wait 100",
                 f"dump {sm}/c{k}.txt", "click 5 480", "wait 400"]
sm_wy, sm_wx = sm_rows[sm_start + 8]
sm_steps += [f"dblclick {sm_wx + 5.5 * sm_cell:.1f} {sm_wy + sm_line_h / 2:.1f}", "wait 150",
             "key cmd+c", "wait 100", f"dump {sm}/wrapped.txt", "click 5 480", "wait 400"]
# word_chars = ":" makes the colon a word character
sm_steps += [f"sh printf 'word_chars = \":\"\\n' >> {sm / 'cfg/rift/config.toml'}", "wait 1200",
             f"dblclick {sm_at(6, 'bar')}", "wait 150", "key cmd+c", "wait 100", f"dump {sm}/wc.txt"]
sm_run("clicks", sm_steps)
def sm_clip(name):
    line = next(l for l in (sm / f"{name}.txt").read_text().splitlines() if l.startswith("clipboard="))
    return json.loads(line[len("clipboard="):])
for k, (_, needle, want) in enumerate(sm_cases):
    assert sm_clip(f"c{k}") == want, (needle, sm_clip(f"c{k}"), want)
assert sm_clip("wc") == "foo:bar", sm_clip("wc")
assert sm_clip("wrapped") == sm_lines[7], sm_clip("wrapped")

# soft `clear`: the next command starts right above the command line
# (blocks_start = "bottom", the default), or at the top of the viewport with
# blocks_start = "top", never at the far end. The completion panel docked
# above the command line moves the output up out from under it (Warp): the
# newest line stays in view
clr = scratch / "clear"
for sub in ("cfg/rift", "data", "top/cfg/rift", "top/data"):
    (clr / sub).mkdir(parents=True, exist_ok=True)
clr_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
              'sync_os_theme = false\nwindow_opacity = 0.8\n'
              + f'working_directory = {json.dumps(str(work))}\n')
(clr / "cfg/rift/config.toml").write_text(clr_config)
(clr / "top/cfg/rift/config.toml").write_text(clr_config + 'blocks_start = "top"\n')
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
(clr / "top/clear.rift").write_text("\n".join([*clear_steps(clr / "top"), "quit"]) + "\n")
with (clr / "top/app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(clr / "top/clear.rift"), "--headless"],
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
(clr / "clear.rift").write_text("\n".join(clr_steps) + "\n")
with (clr / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(clr / "clear.rift"), "--headless"],
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
    (chips / f"{name}.rift").write_text("\n".join([
        # a loaded runner is slow to run the cd: wait for it, then for git
        "resize 1200 800", "wait_prompt",
        f"type cd {chips / 'repo'} && touch {chips / name}.ready\\n",
        f"wait_file {chips / name}.ready 30000", "wait 800",
        *steps, "quit"]) + "\n")
    with (chips / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(chips / f"{name}.rift"), "--headless"],
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
# switch this shell to another (`rift-use`); the PR chip shows the branch's
# pull request (`gh`) and opens it; the Docker chip counts the compose
# project's running containers and lists them. Fake gh, docker, Homebrew
# and pyenv trees keep it hermetic; HOME is scratch so no rc file or version
# manager of the runner's leaks in.
tl = scratch / "tools"
for sub in ("cfg/rift", "data", "home", "bin", "repo"):
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
(tl / "cfg/rift/config.toml").write_text(
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
    (tl / f"{name}.rift").write_text("\n".join(["resize 1200 800", "wait_prompt", "wait 1500",
                                                 *steps, "quit"]) + "\n")
    with (tl / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tl / f"{name}.rift"), "--headless"],
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
assert f'cmd="rift-use php \'{brew}/opt/php@8.2/bin\'" exit=Some(0)' in php_used, php_used
assert '"Php:8.2.9"' in php_used, php_used
py_menu = (tl / "py-menu.txt").read_text()
assert 'chip_menu=Some((Python, "", ["3.12.2", "3.11.1", "system"], 0))' in py_menu, py_menu
py_used = (tl / "py-used.txt").read_text()
assert 'cmd="rift-use pyenv \'3.11.1\'" exit=Some(0)' in py_used, py_used
assert '"Python:3.11.1"' in py_used, py_used
docker_menu = (tl / "docker-menu.txt").read_text()
assert 'chip_menu=Some((Docker, "", ["web", "db"], 0))' in docker_menu, docker_menu
assert 'toast=Some("Headless: not opening https://github.com/o/r/pull/42")' \
    in (tl / "pr.txt").read_text()

# a chip turned on while tabs are open shows in each at once (no command,
# no new tab): config.toml rewritten under the running app, as Settings or
# another window would
config_with = (tl / "cfg/rift/config.toml").read_text()
(tl / "cfg/rift/config.toml").write_text(
    config_with.replace('["github_pr", "python", "php", "docker"]', '["cwd"]'))
(tl / "docker.toml").write_text(config_with.replace(
    '["github_pr", "python", "php", "docker"]', '["cwd", "docker"]'))
run_tools("reload", [
    "key cmd+t", "wait_prompt", "wait 1000", f"dump {tl}/reload-before.txt",
    f"sh cp {tl}/docker.toml {tl}/cfg/rift/config.toml", "wait 1500",
    f"dump {tl}/reload-tab2.txt", "key cmd+1", "wait 300", f"dump {tl}/reload-tab1.txt",
])
(tl / "cfg/rift/config.toml").write_text(config_with)
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
for sub in ("cfg/rift", "data"):
    (pc / sub).mkdir(parents=True, exist_ok=True)
(pc / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nprompt_chips = ["python", "cwd", "git_branch", "time24"]\n'
    + f'working_directory = {json.dumps(str(work))}\n')

def run_prompt(name, steps):
    (pc / f"{name}.rift").write_text("\n".join(["resize 1200 800", "wait_prompt", *steps,
                                                  "quit"]) + "\n")
    # the runner's own Python environment stays out of the shell
    clean = {k: v for k, v in env.items()
             if k not in ("VIRTUAL_ENV", "PYENV_VERSION")}
    with (pc / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(pc / f"{name}.rift"), "--headless"],
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
                     f"move {rect_center(editor_dump, 'Item(0, 0)')}", "wait 900",
                     f"dump {pc}/card-box.txt"])
assert 'tooltip=Some("Host | This machine\'s host name.")' in (pc / "card-palette.txt").read_text()
assert 'tooltip=Some("Python environment | The active virtualenv' in (pc / "card-box.txt").read_text()
# the keyboard: Tab focuses the first chip, → moves it a place, Delete
# removes it; Esc leaves the saved chips alone
run_prompt("keys", [*open_editor, "key tab", "key right", f"dump {pc}/keys-moved.txt",
                    "key delete", f"dump {pc}/keys-deleted.txt", "key escape", "wait 200",
                    f"dump {pc}/keys-cancelled.txt"])
assert "prompt_editor=Some(([Cwd, Python, GitBranch, Time24], None))" in \
    (pc / "keys-moved.txt").read_text(), (pc / "keys-moved.txt").read_text()[:400]
assert "prompt_editor=Some(([Cwd, GitBranch, Time24], None))" in (pc / "keys-deleted.txt").read_text()
assert "prompt_chips=[Python, Cwd, GitBranch, Time24] prompt_editor=None" in \
    (pc / "keys-cancelled.txt").read_text()
first = rect_center(editor_dump, "Item(0, 0)")
last = re.search(r"Item\(0, 3\)@(\d+),(\d+),(\d+),(\d+)", editor_dump).groups()
past_last = f"{int(last[0]) + int(last[2]) + 4} {int(last[1]) + int(last[3]) // 2}"
# drag first: adding a chip can reflow the palette and move the box
run_prompt("edit", [*open_editor, f"drag {first} {past_last}", "wait 100", f"click {host}",
                    "wait 100", f"dump {pc}/edited.txt", "key Enter", "wait 300",
                    f"dump {pc}/saved.txt"])
edited = (pc / "edited.txt").read_text()
assert "prompt_editor=Some(([Cwd, GitBranch, Time24, Python, Host], None))" in edited, edited
assert "prompt_chips=[Cwd, GitBranch, Time24, Python, Host] prompt_editor=None" in \
    (pc / "saved.txt").read_text()
saved = (pc / "cfg/rift/config.toml").read_text()
assert re.search(r'prompt_chips = \[\s*"cwd",\s*"git_branch",\s*"time24",\s*"python",\s*"host",?\s*\]',
                 saved), saved

# the elapsed chip is hidden until the tab's first command, then counts from
# it; a click on a block's output selects it and a second click deselects
(pc / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nprompt_chips = ["cwd", "elapsed"]\n'
    + f'working_directory = {json.dumps(str(work))}\n')
run_prompt("elapsed", [f"dump {pc}/elapsed-before.txt", "type ls /nonexistent-dir\\n",
                       "wait 800", f"dump {pc}/elapsed-after.txt"])
assert 'prompt_chips_shown=["Cwd:' in (pc / "elapsed-before.txt").read_text()
assert '"Elapsed:' not in (pc / "elapsed-before.txt").read_text()
after = (pc / "elapsed-after.txt").read_text()
assert '"Elapsed:0m"]' in after, after
line_h, shown = grid_rows(after)
start = int(re.search(r'cmd="ls /nonexistent-dir".*?output_start=Some\((\d+)\)', after)[1])
out_at = f"300 {next(y for a, y in shown if a == start) + line_h / 2:.0f}"
# (the second click waits out the 400 ms double-click window)
run_prompt("select", ["type ls /nonexistent-dir\\n", "wait 800", f"click {out_at}", "wait 600",
                      f"dump {pc}/select-on.txt", f"click {out_at}", "wait 200",
                      f"dump {pc}/select-off.txt"])
assert "selected_block=Some(" in (pc / "select-on.txt").read_text()
assert "selected_block=None" in (pc / "select-off.txt").read_text()

# Settings › Notifications › Try it: a sample toast shows over the Settings
# tab (they were hidden there, and ran out unseen); pressed by keyboard
# focused, as a real window is (a read entry's toast was dropped then)
run_prompt("try-toast", ["focus on", "key cmd+,", "wait 400", "type card for a pane", "wait 300",
                         "key tab", "wait 100", "key space", "wait 300",
                         f"dump {pc}/try-toast.txt"])
tried = (pc / "try-toast.txt").read_text()
assert "settings_focus=Some(SettingsTestAlert(" in tried, tried[:3000]
assert re.search(r"note_toasts=\[\d+\]", tried), tried[:3000]
# drawn, not only queued: the card's own dismiss button is on screen
assert 'Button "Dismiss: cargo build --release"' in tried, tried[-3000:]
assert "test alert Finished (toast)" in (pc / "try-toast.log").read_text()
# ▷ plays the row's own sound without opening the list
run_prompt("play-probe", ["key cmd+,", "wait 400", "type attention", "wait 300",
                          f"dump {pc}/play-probe.txt"])
play = rect_center((pc / "play-probe.txt").read_text(), "Play this sound")
run_prompt("play", ["key cmd+,", "wait 400", "type attention", "wait 300", f"click {play}",
                    "wait 300", f"dump {pc}/play.txt"])
played = (pc / "play.log").read_text()
# the platform's attention sound (Ping on macOS), or, where it is not
# installed, a toast that says so
assert re.search(r"sound \(suppressed\): \S*(Ping|message-new-instant)", played) or \
    'toast=Some("Sound not found: Default")' in (pc / "play.txt").read_text(), played[-2000:]
assert "ListBoxOption" not in (pc / "play.txt").read_text(), "▷ opened the list"

# a restored tab gets a new shell, so its elapsed clock starts over: the
# saved start of a shell that is gone is not shown
rs = scratch / "restore-elapsed"
for sub in ("cfg/rift", "data/rift"):
    (rs / sub).mkdir(parents=True, exist_ok=True)
(rs / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
    'sync_os_theme = false\nprompt_chips = ["cwd", "elapsed"]\n')
(rs / "data/rift/session.toml").write_text(
    f'version = 3\nactive = 0\n\n[[tabs]]\ncwd = {json.dumps(str(work))}\n'
    f'started = {int(time.time()) - 2 * 86400}\n')
(rs / "t.rift").write_text(f"wait_prompt\ndump {rs}/restored.txt\nquit\n")
with (rs / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rs / "t.rift"), "--headless"],
                        # restores the saved tabs; durable_sessions = false
                        # starts no session service
                        env=dict(env, XDG_CONFIG_HOME=str(rs / "cfg"),
                                 XDG_DATA_HOME=str(rs / "data"), RIFT_PERSIST_SESSION="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("restore-elapsed run failed", rs / "app.log")
restored = (rs / "restored.txt").read_text()
assert 'prompt_chips_shown=["Cwd:' in restored and '"Elapsed:' not in restored, restored

# live git status: two repositories in one tab, the chips and the review
# model follow the focused pane, and a change made while its pane is busy
# (an agent's edit; here a `sleep` runs while the file changes) shows
# without the command ending (the focused window's poll)
lg = scratch / "live-git"
for sub in ("cfg/rift", "data"):
    (lg / sub).mkdir(parents=True, exist_ok=True)
for r in ("a", "b"):
    (lg / r).mkdir(exist_ok=True)
    subprocess.run(["git", "-C", str(lg / r), "init", "-q"], check=True)
    (lg / r / "f.txt").write_text("one\n")
    subprocess.run(["git", "-C", str(lg / r), "add", "."], check=True)
    subprocess.run(["git", "-C", str(lg / r), "-c", "user.email=t@t", "-c", "user.name=t",
                    "commit", "-qm", "init"], check=True)
(lg / "a/f.txt").write_text("one\ntwo\nthree\n")
(lg / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nprompt_chips = ["cwd", "git_branch", "git_diff"]\n')
(lg / "t.rift").write_text("\n".join([
    "resize 1400 800", "focus on", "wait_prompt",
    f"type cd {lg / 'a'}\\n", "wait 1200", "key cmd+d", "wait_prompt",
    f"type cd {lg / 'b'}\\n", "wait 1500", f"dump {lg}/b.txt",
    "key cmd+alt+left", "wait 1500", f"dump {lg}/a.txt",
    "key cmd+alt+right", "wait 1500",
    "type sleep 8\\n", "wait 500",
    f"sh printf 'x\\\\ny\\\\n' >> {lg / 'b/f.txt'}",
    "wait 4500", f"dump {lg}/busy.txt", "quit"]) + "\n")
with (lg / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(lg / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(lg / "cfg"), XDG_DATA_HOME=str(lg / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("live-git run failed", lg / "app.log")
def lg_state(name):
    d = (lg / f"{name}.txt").read_text()
    review = re.search(r"^review=\S+ load=\S+ staged=\S+ unstaged=(\S+)", d, re.M)[1]
    diff = re.search(r'"GitDiff:[^"]*"', d)
    return review, diff[0] if diff else None
assert lg_state("b") == ("Some([])", None), lg_state("b")
assert lg_state("a") == ('Some(["Mf.txt"])', '"GitDiff:1 • +2 −0"'), lg_state("a")
busy = (lg / "busy.txt").read_text()
assert "any_running=true" in busy, busy[:600]
# the busy pane's own diff, while `sleep` still runs
assert re.search(r"^review=\S+ load=ready staged=\S+ unstaged=Some\(\[\"Mf.txt\"\]\)", busy, re.M), \
    busy[-3000:]

# ⌘ arrows in a multi-line command: ⌘←/⌘→ the cursor's line, ⌘↑/⌘↓ the
# command's start/end (⇧ selects); on its first line ⌘↑ selects the block
# above again; held keys (auto-repeat) move once per repeat
ca = scratch / "cmd-arrows"
for sub in ("cfg/rift", "data"):
    (ca / sub).mkdir(parents=True, exist_ok=True)
(ca / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n')
ca_keys = ["cmd+left", "cmd+up", "cmd+right", "cmd+down", "R7 left", "R3 alt+left",
           "shift+cmd+up", "cmd+up"]
ca_steps = ["resize 1200 800", "wait_prompt", "type echo a\\n", "wait 400", "type echo b\\n",
            "wait 400", "type echo one", "key shift+enter", "type echo two",
            "key shift+enter", "type echo three", f"dump {ca}/0.txt"]
for i, k in enumerate(ca_keys, 1):
    ca_steps += [f"keyrepeat {k[1:]}" if k.startswith("R") else f"key {k}",
                 f"dump {ca}/{i}.txt"]
# walking the blocks (one selected) from the first line of a multi-line
# command: ⌘↓ steps to the next block, never into the command
ca_steps += ["key cmd+up", f"dump {ca}/10.txt", "key cmd+down", f"dump {ca}/11.txt"]
ca_steps += ["key ctrl+u", "type ab", "keyrepeat 2 cmd+up", f"dump {ca}/9.txt", "quit"]
(ca / "t.rift").write_text("\n".join(ca_steps) + "\n")
with (ca / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(ca / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(ca / "cfg"), XDG_DATA_HOME=str(ca / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("cmd-arrows run failed", ca / "app.log")
def ca_state(i):
    d = (ca / f"{i}.txt").read_text()
    cur = int(re.search(r'editor="[^"]*" cursor=(\d+)', d)[1])
    blk = re.search(r"selected_block=(\S+)", d)[1]
    return cur, blk, "editor_selection=Some(" in d
# "echo one\necho two\necho three": lines start at 0, 9, 18; 28 chars
assert [ca_state(i)[0] for i in range(8)] == [28, 18, 0, 8, 28, 21, 9, 0], \
    [ca_state(i) for i in range(8)]
assert ca_state(7)[2], "⇧⌘↑ did not select to the command start"
assert ca_state(8)[1] == "Some(1)", ca_state(8)
# a one-line command: a held ⌘↑ walks up the blocks, one per repeat
assert ca_state(9)[1] == "Some(0)", ca_state(9)
assert ca_state(10)[:2] == (0, "Some(0)"), ca_state(10)
assert ca_state(11)[:2] == (0, "Some(1)"), ca_state(11)

# block tools: the block filter's toggles (case, invert, context with a
# separator row, an invalid regex keeping the last valid filter), a filter
# kept off and back on; ⌘F limited to the selected block and widened; a
# multi-block selection (⇧↑, ⌘A) and its menu (copy, compare); Copy as
# HTML (escaped, coloured) and Markdown; Save output writes the block text
bt2 = scratch / "block-tools"
for sub in ("cfg/rift", "data"):
    (bt2 / sub).mkdir(parents=True, exist_ok=True)
(bt2 / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n')
bt2_steps = [
    "resize 1100 700", "wait_prompt",
    "type for w in alpha 'beta ERROR one' gamma delta epsilon 'zeta error two' eta; do echo $w; done\\n",
    "wait 600", "key alt+shift+f", "type error", f"dump {bt2}/plain.txt",
    "key alt+c", f"dump {bt2}/case.txt", "key alt+c",
    "key alt+i", f"dump {bt2}/invert.txt", "key alt+i",
    "key alt+up", f"dump {bt2}/context.txt",
    "key alt+r", "type (", f"dump {bt2}/invalid.txt", "key backspace",
    "key enter", "key alt+shift+f", f"dump {bt2}/off.txt",
    "key alt+shift+f", f"dump {bt2}/on.txt", "key escape", "key escape",
    # coloured by the printf builtin: a forked tput sometimes adds bash
    # 3.2's "child setpgid ... Operation not permitted" to the output
    "type printf '\\033[31m'; echo 'NEEDLE <b>&'; printf '\\033[0m'\\n", "wait 400",
    "type echo NEEDLE two\\n", "wait 400",
    "key cmd+up", "key cmd+f", "type NEEDLE", "wait 200", f"dump {bt2}/scoped.txt",
    "a11y click Search the whole pane", "wait 200", f"dump {bt2}/widened.txt",
    "key escape", "key escape",
    "key cmd+up", "key shift+up", "wait 200", f"dump {bt2}/multi.txt",
    "key ctrl+m", "wait 300", f"dump {bt2}/menu.txt",
    "a11y click Copy 2 commands", "wait 200", f"dump {bt2}/copied.txt",
    "key ctrl+m", "wait 300", "a11y click Compare outputs", "wait 1000",
    f"dump {bt2}/compare.txt", "key escape",
    "key cmd+a", "wait 200", f"dump {bt2}/all.txt", "key escape",
    "key cmd+up", "key cmd+up", "key ctrl+m", "wait 300", "a11y click Copy as HTML", "wait 200",
    f"dump {bt2}/html.txt", "key ctrl+m", "wait 300", "a11y click Copy as Markdown", "wait 200",
    f"dump {bt2}/md.txt", "key ctrl+m", "wait 300", "a11y click Save output…", "wait 300",
    f"type {bt2}/out.txt", "wait 200", "key enter", "wait 400", f"dump {bt2}/saved.txt",
    # over an existing file: asked first; Esc keeps it, Replace writes
    f"sh printf OLD > {bt2}/out.txt",
    "key ctrl+m", "wait 300", "a11y click Save output…", "wait 300",
    f"type {bt2}/out.txt", "wait 200", "key enter", "wait 600", f"dump {bt2}/replace.txt",
    "key escape", "wait 300", f"sh cp {bt2}/out.txt {bt2}/kept.txt",
    "key ctrl+m", "wait 300", "a11y click Save output…", "wait 300",
    f"type {bt2}/out.txt", "wait 200", "key enter", "wait 700", "a11y click Replace", "wait 400",
    # find in a filtered block: a match in the lines its filter hides is
    # said, not silently missing
    "key escape", "type echo keep; echo alpha\\n", "wait 400",
    "key alt+shift+f", "type keep", "key enter", "wait 200",
    "key cmd+f", "type alpha", "wait 300", f"dump {bt2}/hidden.txt",
    "quit",
]
(bt2 / "t.rift").write_text("\n".join(bt2_steps) + "\n")
with (bt2 / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(bt2 / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(bt2 / "cfg"), XDG_DATA_HOME=str(bt2 / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("block-tools run failed", bt2 / "app.log")
def bt2_state(name):
    return (bt2 / f"{name}.txt").read_text()
def bt2_rows(name):
    # from the first header: the shell's banner rows come before it
    rows = re.search(r"row_kinds=(\S*)", bt2_state(name))[1]
    return rows[rows.index("H"):]
# the filtered block's header, then the kept lines (the live prompt is
# the editor's, not a row)
assert bt2_rows("plain") == "HGG", bt2_rows("plain")
assert bt2_rows("case") == "HG", bt2_rows("case")
assert bt2_rows("invert") == "HGGGGG", bt2_rows("invert")
assert bt2_rows("context") == "HGGGSGGG", bt2_rows("context")
inv = bt2_state("invalid")
assert 'query="error(" ' in inv and "error=Some(" in inv and "applied=true" in inv, inv[:3000]
assert bt2_rows("invalid") == "HGGGSGGG", "an invalid regex dropped the last valid filter"
assert "enabled=false" in bt2_state("off") and bt2_rows("off") == "HGGGGGGG"
on = bt2_state("on")
assert 'query="error" ' in on and "enabled=true" in on and "context=1" in on, on[:3000]
scoped = bt2_state("scoped")
assert 'Dialog "Find in block: echo NEEDLE two"' in scoped and " matches=1 " in scoped, scoped[:3000]
widened = bt2_state("widened")
assert 'Dialog "Find"' in widened and " matches=2 " in widened, widened[:3000]
assert "selected_blocks=[1, 2]" in bt2_state("multi"), bt2_state("multi")[:3000]
assert '"Copy 2 commands"' in bt2_state("menu") and '"Compare outputs"' in bt2_state("menu")
assert ('clipboard="printf \'\\\\033[31m\'; echo \'NEEDLE <b>&\'; printf \'\\\\033[0m\'\\necho NEEDLE two"'
        in bt2_state("copied")), bt2_state("copied")[-1500:]
cmp_line = next(l for l in bt2_state("compare").splitlines() if l.startswith("compare="))
assert ("compare=true blocks=(1, 2) phase=ready" in cmp_line and "added=1 removed=1" in cmp_line), cmp_line
assert "selected_blocks=[0, 1, 2]" in bt2_state("all"), bt2_state("all")[:3000]
html = next(l for l in bt2_state("html").splitlines() if l.startswith("clipboard_html="))
assert "&lt;b&gt;&amp;" in html and "<b>&" not in html, html
assert '<span style=\\"color: #' in html, "Copy as HTML lost the output's colour: " + html
md = bt2_state("md")
assert "clipboard=\"```sh\\nprintf '\\\\033[31m';" in md and "```\\nNEEDLE <b>&\\n```" in md, md[-1500:]
assert (bt2 / "out.txt").read_text() == "NEEDLE <b>&\n", (bt2 / "out.txt").read_text()
assert 'AlertDialog "Replace the file?"' in bt2_state("replace"), bt2_state("replace")[-3000:]
assert (bt2 / "kept.txt").read_text() == "OLD", "Save output replaced a file without asking"
hidden = bt2_state("hidden")
assert 'query="alpha"' in hidden and " hidden=1 " in hidden, hidden[:3000]

# ⇧⌘T of a tab whose shell reported a directory on another host opens in
# the configured start directory, never the app process's own cwd
rr = scratch / "reopen-remote"
for sub in ("cfg/rift", "data"):
    (rr / sub).mkdir(parents=True, exist_ok=True)
(rr / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'sync_os_theme = false\nworking_directory = {json.dumps(str(work))}\n')
(rr / "t.rift").write_text("\n".join([
    "wait_prompt",
    "type __rift_report_cwd() { printf '\\\\e]7;file://remotebox/srv/app\\\\a'; }\\n",
    "wait 300", "type true\\n", "wait 1000", "key cmd+t", "wait 800", "key cmd+1",
    "wait 200", "key ctrl+shift+w", "wait 400", "key cmd+shift+t", "wait 1500",
    f"dump {rr}/reopened.txt", "quit"]) + "\n")
with (rr / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rr / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rr / "cfg"), XDG_DATA_HOME=str(rr / "data")),
                        cwd=str(scratch), stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("reopen-remote run failed", rr / "app.log")
reopened = (rr / "reopened.txt").read_text()
assert "tabs=2 active=1 " in reopened, reopened[:1500]
assert re.findall(r"^  pane \d+ cwd=(\S+)", reopened, re.M)[-1].endswith(
    f'{scratch.name}/work")'), reopened[:2000]

# a restored split keeps its focused pane even when a later subtree splits
# again (each split makes its new pane active)
rf = scratch / "restore-focus"
for sub in ("cfg/rift", "data/rift"):
    (rf / sub).mkdir(parents=True, exist_ok=True)
(rf / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
    'sync_os_theme = false\n')
leaf = f'kind = "Pane"\ncwd = {json.dumps(str(work))}\n'
(rf / "data/rift/session.toml").write_text(
    f'version = 3\nactive = 0\n\n[[tabs]]\ncwd = {json.dumps(str(work))}\n\n'
    '[tabs.layout]\nkind = "Split"\naxis = "Horizontal"\nratio = 0.5\n\n'
    f'[tabs.layout.first]\n{leaf}focused = true\n\n'
    '[tabs.layout.second]\nkind = "Split"\naxis = "Vertical"\nratio = 0.5\n\n'
    f'[tabs.layout.second.first]\n{leaf}\n[tabs.layout.second.second]\n{leaf}')
(rf / "t.rift").write_text(f"wait_prompt\nwait 500\ndump {rf}/restored.txt\nquit\n")
with (rf / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rf / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rf / "cfg"),
                                 XDG_DATA_HOME=str(rf / "data"), RIFT_PERSIST_SESSION="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("restore-focus run failed", rf / "app.log")
restored = (rf / "restored.txt").read_text()
first_pane = re.findall(r"^  pane (\d+) ", restored, re.M)[0]
assert f"panes=3 active_pane={first_pane} " in restored, restored

# a pane's own colour scheme: a profile's `theme` colours the pane it opens,
# "Pane theme: <name>" the active pane only, "Pane theme: default" goes back
# to the window theme over the profile's; the session keeps each pane's, and
# a restart restores them (the session file copied before the quit, which
# empties it). Restart pane keeps the pane's theme, and its colour queries
# (OSC 11) answer with it
pt = scratch / "pane-theme"
for sub in ("cfg/rift", "data/rift"):
    (pt / sub).mkdir(parents=True, exist_ok=True)
(pt / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
    'sync_os_theme = false\n\n[[profiles]]\nname = "prod"\ntheme = "light-default"\n')
pt_env = dict(env, XDG_CONFIG_HOME=str(pt / "cfg"), XDG_DATA_HOME=str(pt / "data"),
              RIFT_PERSIST_SESSION="1")
def pt_run(name, steps):
    (pt / f"{name}.rift").write_text("\n".join(["resize 1000 500", "wait_prompt", *steps, "quit"]) + "\n")
    with (pt / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(pt / f"{name}.rift"), "--headless"],
                            env=pt_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"pane-theme {name} run failed", pt / f"{name}.log")
# prints the terminal's answer to an OSC 11 (background colour) query
(pt / "bg-query.sh").write_text(
    "printf '\\033]11;?\\007'; IFS= read -rs -t 2 -d \"$(printf '\\007')\" r; echo \"BG=${r#*;}\"\n")
def pt_panes(name):
    text = (pt / f"{name}.txt").read_text()
    return re.findall(r'pane_theme=(None|Some\("[^"]*"\)) launch_theme=(None|Some\("[^"]*"\))', text)
pt_run("set", [
    "key cmd+p", "type Split right: prod", "wait 200", "key enter", "wait_prompt", "wait 300",
    f"dump {pt}/split.txt",
    "key alt+cmd+left", "wait 200", "key cmd+p", "type Pane theme: tokyo-night", "wait 200",
    "key enter", "wait 300", f"dump {pt}/tokyo.txt",
    "key alt+cmd+right", "wait 200", "key cmd+p", "type Pane theme: default", "wait 200",
    "key enter", "wait 300", f"dump {pt}/default.txt", f"shot {pt}/default.png",
    "key cmd+p", "type Pane theme: catppuccin-mocha", "wait 200", "key enter", "wait 300",
    f"sh cp {pt}/data/rift/session.toml {pt}/saved.toml",
    "key cmd+p", "type Restart pane", "wait 200", "key enter", "wait_prompt", "wait 500",
    f"type bash {pt}/bg-query.sh\\n", "wait 1000", f"dump {pt}/restarted.txt"])
assert pt_panes("split") == [("None", "None"), ('Some("light-default")', "None")], pt_panes("split")
assert pt_panes("tokyo")[0] == ('Some("tokyo-night")', 'Some("tokyo-night")'), pt_panes("tokyo")
# the profile names a theme: "default" is recorded to keep the window's
assert pt_panes("default")[1] == ("None", 'Some("default")'), pt_panes("default")
saved = (pt / "saved.toml").read_text()
assert 'theme = "tokyo-night"' in saved and 'theme = "catppuccin-mocha"' in saved, saved
assert pt_panes("restarted")[1][0] == 'Some("catppuccin-mocha")', pt_panes("restarted")
assert "BG=rgb:1e1e/1e1e/2e2e" in (pt / "restarted.txt").read_text(), (pt / "restarted.txt").read_text()[-3000:]
(pt / "data/rift/session.toml").write_text(saved)
pt_run("restored", ["wait 500", f"dump {pt}/restored.txt"])
assert pt_panes("restored") == [('Some("tokyo-night")', 'Some("tokyo-night")'),
                                ('Some("catppuccin-mocha")', 'Some("catppuccin-mocha")')], \
    pt_panes("restored")

# ⌘K Clear Blocks wipes the screen and the scrollback (not the shell's soft
# `clear`, which keeps the history scrollable): nothing of before is left
ck = scratch / "clear-blocks"
for sub in ("cfg/rift", "data"):
    (ck / sub).mkdir(parents=True, exist_ok=True)
(ck / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\nsync_os_theme = false\n')
(ck / "t.rift").write_text("\n".join([
    "wait_prompt", "type seq -f OLD%g 1 80\\n", "wait 800", "wait_prompt", "key cmd+k", "wait 400",
    # scrolled to the top: the history is gone too
    "move 500 300", "scroll 3000", "wait 300",
    f"dump {ck}/cleared.txt", "type echo after\\n", "wait 800", f"dump {ck}/after.txt", "quit"]) + "\n")
with (ck / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(ck / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(ck / "cfg"), XDG_DATA_HOME=str(ck / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("clear-blocks run failed", ck / "app.log")
ck_cleared = (ck / "cleared.txt").read_text()
# nothing above to scroll to (the accessibility value is cut, so it cannot
# show history lines far up)
assert re.search(r"^  pane \d+ .*view_offset=0 ", ck_cleared, re.M), ck_cleared[:3000]
assert "OLD" not in re.search(r'Terminal "Terminal output" value="([^"]*)"', ck_cleared)[1], ck_cleared[-2000:]
ck_after = (ck / "after.txt").read_text()
assert re.findall(r'^    block \d+ state=done .*? cmd="([^"]*)"', ck_after, re.M) == ["echo after"], ck_after[-2000:]

# durable sessions: a window that dies (a crash) leaves its shell in the
# session service, and the relaunch reattaches it and replays its output at
# the new window's width; restored headers keep the exact commands (the
# shell's copy in 133;C), not the echo readline placed for the old width
ra = scratch / "reattach"
for sub in ("cfg/rift", "data/rift"):
    (ra / sub).mkdir(parents=True, exist_ok=True)
(ra / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = true\nsync_os_theme = false\n')
ra_env = dict(env, XDG_CONFIG_HOME=str(ra / "cfg"), XDG_DATA_HOME=str(ra / "data"),
              RIFT_PERSIST_SESSION="1")
ra_long, ra_wide = "echo " + "LONG_" * 40, "echo " + "漢字" * 42
(ra / "a.rift").write_text("\n".join([
    "resize 1000 700", "wait_prompt", f"type {ra_long}\\n", "wait 800", "wait_prompt",
    f"type {ra_wide}\\n", "wait 800", "wait_prompt", f"sh touch {ra}/ready", "wait 60000",
    "quit"]) + "\n")
with (ra / "a.log").open("w") as log:
    first = subprocess.Popen([str(binary), "--script", str(ra / "a.rift"), "--headless"],
                             env=ra_env, stdout=log, stderr=log)
    for _ in range(300):
        if (ra / "ready").exists() or first.poll() is not None:
            break
        time.sleep(0.1)
    time.sleep(0.5)
    first.kill()
    first.wait()
assert (ra / "ready").exists(), died("reattach first run failed", ra / "a.log")
# the relaunch's window starts at its default size, not 1000x700
(ra / "b.rift").write_text(f"wait_prompt\nwait 1500\ndump {ra}/restored.txt\nquit\n")
try:
    with (ra / "b.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(ra / "b.rift"), "--headless"],
                            env=ra_env, stdout=log, stderr=log, timeout=90).returncode
finally:
    # the session service this test started (matched by its data dir)
    for pid in session_daemons(ra / "data"):
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
assert rc == 0, died("reattach run failed", ra / "b.log")
restored = (ra / "restored.txt").read_text()
ra_cmds = re.findall(r'^    block \d+ state=done .*? cmd="([^"]*)"', restored, re.M)
assert ra_cmds == [ra_long, ra_wide], (ra_cmds, restored[-3000:])
# a restored pane whose shell is gone (the service ended) and whose profile
# is no longer in config.toml: the fresh shell is the global one, and a
# toast says so
rp = scratch / "reattach-profile"
for sub in ("cfg/rift", "data/rift"):
    (rp / sub).mkdir(parents=True, exist_ok=True)
rp_base = ('shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = true\nsync_os_theme = false\n'
           'discover_shells = false\ndefault_profile = "p"\n')
(rp / "cfg/rift/config.toml").write_text(rp_base + '[[profiles]]\nname = "p"\ncommand = "/bin/sh"\n')
rp_env = dict(env, XDG_CONFIG_HOME=str(rp / "cfg"), XDG_DATA_HOME=str(rp / "data"),
              RIFT_PERSIST_SESSION="1")
(rp / "a.rift").write_text(f"wait 2500\nsh touch {rp}/ready\nwait 60000\nquit\n")
with (rp / "a.log").open("w") as log:
    first = subprocess.Popen([str(binary), "--script", str(rp / "a.rift"), "--headless"],
                             env=rp_env, stdout=log, stderr=log)
    for _ in range(300):
        if (rp / "ready").exists() or first.poll() is not None:
            break
        time.sleep(0.1)
    time.sleep(0.5)
    first.kill()
    first.wait()
assert (rp / "ready").exists(), died("reattach-profile first run failed", rp / "a.log")
for pid in session_daemons(rp / "data"):
    os.kill(pid, signal.SIGKILL)
time.sleep(0.5)
(rp / "cfg/rift/config.toml").write_text(rp_base)
(rp / "b.rift").write_text(f"wait 4000\ndump {rp}/b.txt\nquit\n")
try:
    with (rp / "b.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(rp / "b.rift"), "--headless"],
                            env=rp_env, stdout=log, stderr=log, timeout=90).returncode
finally:
    for pid in session_daemons(rp / "data"):
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
assert rc == 0, died("reattach-profile run failed", rp / "b.log")
rp_b = (rp / "b.txt").read_text()
assert 'No profile named \\"p\\"; this pane runs /bin/bash instead' in rp_b, rp_b[:3000]
# a durable shell's RIFT_PANE_URL names its service session, so the link
# still focuses its pane after the window died and a relaunch reattached it
rf2 = scratch / "reattach-focus"
for sub in ("cfg/rift", "data/rift"):
    (rf2 / sub).mkdir(parents=True, exist_ok=True)
(rf2 / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = true\nsync_os_theme = false\n')
rf2_env = dict(env, XDG_CONFIG_HOME=str(rf2 / "cfg"), XDG_DATA_HOME=str(rf2 / "data"),
               RIFT_PERSIST_SESSION="1")
(rf2 / "a.rift").write_text("\n".join([
    "wait_prompt", "wait 1000", f'type printf %s "$RIFT_PANE_URL" > {rf2}/url\\n',
    f"wait_file {rf2}/url 10000", "wait 500", f"sh touch {rf2}/ready", "wait 60000", "quit"]) + "\n")
try:
    with (rf2 / "a.log").open("w") as log:
        first = subprocess.Popen([str(binary), "--script", str(rf2 / "a.rift"), "--headless"],
                                 env=rf2_env, stdout=log, stderr=log)
        for _ in range(300):
            if (rf2 / "ready").exists() or first.poll() is not None:
                break
            time.sleep(0.1)
        time.sleep(0.5)
        first.kill()
        first.wait()
    assert (rf2 / "ready").exists(), died("reattach-focus first run failed", rf2 / "a.log")
    rf2_url = (rf2 / "url").read_text()
    assert re.fullmatch(r"rift://action/focus\?pane=service\.\d+", rf2_url), rf2_url
    (rf2 / "b.rift").write_text("\n".join([
        "wait_prompt", "wait 1500", "key cmd+t", "wait 1000", f"dump {rf2}/before.txt",
        f"link {rf2_url}", "wait 500", f"dump {rf2}/after.txt", "quit"]) + "\n")
    with (rf2 / "b.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(rf2 / "b.rift"), "--headless"],
                            env=rf2_env, stdout=log, stderr=log, timeout=90).returncode
finally:
    for pid in session_daemons(rf2 / "data"):
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
assert rc == 0, died("reattach-focus run failed", rf2 / "b.log")
rf2_sid = rf2_url.rsplit(".", 1)[1]
rf2_before, rf2_after = ((rf2 / f"{n}.txt").read_text() for n in ("before", "after"))
# the restored tab reattached that shell; the new tab is focused, then the link
assert re.search(r"^window 0: tabs=2 active=1 ", rf2_before, re.M), rf2_before[:600]
assert f"service=Some({rf2_sid})" in rf2_before, rf2_before[:3000]
assert re.search(r"^window 0: tabs=2 active=0 ", rf2_after, re.M), rf2_after[:600]
assert "toasts=[]" in rf2_after, rf2_after[:3000]

# ⌃R Results: past runs by their output, across windows; a repeated command
# stays two runs; query words filter; the preview marks the match and runs
# nothing (⇥ puts the command on the line, ⏎ there runs it once more)
rs = scratch / "results"
(rs / "cfg/rift").mkdir(parents=True)
rs_base = 'shell = "/bin/bash"\ndurable_sessions = false\nsync_os_theme = false\n'
(rs / "cfg/rift/config.toml").write_text(rs_base + "restore_session = false\n")
rs_steps = [
    "resize 1100 760", "wait_prompt",
    "type echo applying 0003; echo ERROR: relation users exists; false\\n", "wait 600", "wait_prompt",
    f"type echo once >> {rs}/ran.log; echo RUNMARK\\n", "wait 600", "wait_prompt",
    "new_window", "wait_prompt",
    "type echo applying 0003; echo ok\\n", "wait 600", "wait_prompt",
    "type echo applying 0003; echo ok\\n", "wait 600", "wait_prompt",
    "key ctrl+r", "wait 200", "key ctrl+r", "wait 200", "key ctrl+r", "wait 800",
    f"dump {rs}/all.txt",
    "type relation", "wait 800", f"dump {rs}/relation.txt", f"shot {rs}/relation.png",
    "key ctrl+u", "type applying is:ok", "wait 800", f"dump {rs}/ok.txt",
    # (one row: ↓ past it would close the pane)
    "key ctrl+u", "type RUNMARK", "wait 800", "key up", "wait 200",
    f"dump {rs}/preview.txt",
    "key tab", "wait 300", f"dump {rs}/edit.txt",
    "key enter", "wait 800", f"dump {rs}/rerun.txt", "quit"]
(rs / "a.rift").write_text("\n".join(rs_steps) + "\n")
with (rs / "a.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rs / "a.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rs / "cfg"), XDG_DATA_HOME=str(rs / "data")),
                        stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("results run failed", rs / "a.log")
rst = lambda name: (rs / f"{name}.txt").read_text()
def found(name):
    line = re.search(r"^history_results=.*$", rst(name), re.M)
    assert line, rst(name)[:3000]
    return line[0]
assert "scope=Results" in rst("all"), rst("all")[:1500]
assert found("all").startswith("history_results=4 "), found("all")
# the two identical runs of the second window are two results
ids = re.search(r"found_ids=\[(.*)\]", found("all"))[1].split(", ")
assert len(ids) == len(set(ids)) == 4, ids
assert found("relation").startswith("history_results=1 "), found("relation")
assert 'found_hit=Some("relation")' in found("relation"), found("relation")
assert "found_exit=Some(1)" in found("relation") and 'found_context=Some("local · ' in found("relation")
assert "match on line 2" in found("relation"), found("relation")
assert found("ok").startswith("history_results=2 "), found("ok")
assert 'found_preview=Some("once' not in found("preview")
assert 'found_hit=Some("RUNMARK")' in found("preview"), found("preview")
# previewing ran nothing; ⇥ put the command on the line; ⏎ ran it once more
assert re.search(r'^\s*pane \d+ .*editor="echo once >>', rst("edit"), re.M), rst("edit")[-3000:]
assert "history=false" in rst("edit"), rst("edit")[:1500]
assert (rs / "ran.log").read_text() == "once\nonce\n", (rs / "ran.log").read_text()

# retained results: with save_transcripts on, a relaunch finds the earlier
# run (restored, its time kept); with saving off, or after "Clear saved
# block transcripts", it is no longer searchable
(rs / "cfg/rift/config.toml").write_text(rs_base + "restore_session = true\nsave_transcripts = true\n")
rt_env = dict(env, XDG_CONFIG_HOME=str(rs / "cfg"), XDG_DATA_HOME=str(rs / "data-r"),
              RIFT_PERSIST_SESSION="1")
(rs / "r1.rift").write_text("\n".join([
    "wait_prompt", "type echo SAVEDMARK-1; false\\n", "wait 800", "wait_prompt",
    f"sh touch {rs}/r1-ready", "wait 60000", "quit"]) + "\n")
with (rs / "r1.log").open("w") as log:
    first = subprocess.Popen([str(binary), "--script", str(rs / "r1.rift"), "--headless"],
                             env=rt_env, stdout=log, stderr=log)
    for _ in range(300):
        if (rs / "r1-ready").exists() or first.poll() is not None:
            break
        time.sleep(0.1)
    time.sleep(1.5)
    first.kill()
    first.wait()
assert (rs / "r1-ready").exists(), died("results first run failed", rs / "r1.log")
# the last command before a kill is in the saved transcript: a block that
# ended inside a coalesced wakeup is saved on the next frame (it used to be
# missing about one run in four); five killed runs
for k in range(5):
    kd = rs / f"data-k{k}"
    (rs / f"k{k}.rift").write_text("\n".join([
        "wait_prompt", f"type cd {rs}\\n", "wait 300",
        "type seq 1 3 | sed s/^/KILLMARK-/\\n", "wait 800", "wait_prompt",
        f"sh touch {rs}/k{k}-ready", "wait 60000", "quit"]) + "\n")
    with (rs / f"k{k}.log").open("w") as log:
        first = subprocess.Popen([str(binary), "--script", str(rs / f"k{k}.rift"), "--headless"],
                                 env=dict(rt_env, XDG_DATA_HOME=str(kd)), stdout=log, stderr=log)
        for _ in range(300):
            if (rs / f"k{k}-ready").exists() or first.poll() is not None:
                break
            time.sleep(0.1)
        time.sleep(1.5)
        first.kill()
        first.wait()
    assert (rs / f"k{k}-ready").exists(), died("killed run failed", rs / f"k{k}.log")
    saved = "".join(p.read_text() for p in (kd / "rift/transcripts").glob("*.json"))
    assert "KILLMARK" in saved, f"run {k}: the last command is not in the transcript"
on_cfg = rs_base + "restore_session = true\nsave_transcripts = true\n"
off_cfg = rs_base + "restore_session = true\nsave_transcripts = false\n"
requery = ["key ctrl+u", "wait 300", "type SAVEDMARK", "wait 1000"]
(rs / "on.toml").write_text(on_cfg)
(rs / "off.toml").write_text(off_cfg)
(rs / "r2.rift").write_text("\n".join([
    "wait_prompt", "wait 1000", "key ctrl+r", "wait 200", "key ctrl+r", "wait 200", "key ctrl+r",
    "type SAVEDMARK", "wait 1000", f"dump {rs}/r-found.txt",
    cp(rs / "off.toml", rs / "cfg/rift/config.toml"), "wait 1500", *requery, f"dump {rs}/r-off.txt",
    cp(rs / "on.toml", rs / "cfg/rift/config.toml"), "wait 1500", *requery, f"dump {rs}/r-on.txt",
    "key escape", "key ctrl+u", "key cmd+p", "wait 300", "type Clear saved block transcripts",
    "wait 300", "key enter", "wait 500",
    "key ctrl+r", "wait 300", *requery, f"dump {rs}/r-cleared.txt", "quit"]) + "\n")
with (rs / "r2.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rs / "r2.rift"), "--headless"],
                        env=rt_env, stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("results relaunch failed", rs / "r2.log")
assert found("r-found").startswith("history_results=1 "), found("r-found")
assert "restored from a previous session" in found("r-found"), found("r-found")
assert "found_when=Some(true)" in found("r-found") and "found_exit=Some(1)" in found("r-found")
assert found("r-off").startswith("history_results=0 ") and " saved=false " in found("r-off"), found("r-off")
assert found("r-on").startswith("history_results=1 "), found("r-on")
assert found("r-cleared").startswith("history_results=0 "), found("r-cleared")
assert "scope=Results" in rst("r-cleared"), rst("r-cleared")[:1500]

# a durable pane reattached after its window died: its runs come back once
# (the service's replay), not again from its saved transcript
rd = scratch / "results-durable"
(rd / "cfg/rift").mkdir(parents=True)
(rd / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = true\nsync_os_theme = false\n'
    "save_transcripts = true\n")
rd_env = dict(env, XDG_CONFIG_HOME=str(rd / "cfg"), XDG_DATA_HOME=str(rd / "data"),
              RIFT_PERSIST_SESSION="1")
(rd / "a.rift").write_text("\n".join([
    "wait_prompt", "wait 1000", "type echo DURMARK\\n", "wait 800", "wait_prompt",
    "type echo DURMARK\\n", "wait 800", "wait_prompt", "key cmd+t", "wait 1000",
    f"sh touch {rd}/ready", "wait 60000", "quit"]) + "\n")
try:
    with (rd / "a.log").open("w") as log:
        first = subprocess.Popen([str(binary), "--script", str(rd / "a.rift"), "--headless"],
                                 env=rd_env, stdout=log, stderr=log)
        for _ in range(300):
            if (rd / "ready").exists() or first.poll() is not None:
                break
            time.sleep(0.1)
        time.sleep(1.5)
        first.kill()
        first.wait()
    assert (rd / "ready").exists(), died("results-durable first run failed", rd / "a.log")
    (rd / "b.rift").write_text("\n".join([
        "wait_prompt", "wait 2000", "key ctrl+r", "wait 200", "key ctrl+r", "wait 200",
        "key ctrl+r", "type DURMARK", "wait 1000", f"dump {rd}/b.txt",
        f"sh ls {rd}/data/rift/transcripts > {rd}/files.txt", "quit"]) + "\n")
    with (rd / "b.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(rd / "b.rift"), "--headless"],
                            env=rd_env, stdout=log, stderr=log, timeout=90).returncode
finally:
    for pid in session_daemons(rd / "data"):
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
assert rc == 0, died("results-durable run failed", rd / "b.log")
rd_b = (rd / "b.txt").read_text()
rd_line = re.search(r"^history_results=.*$", rd_b, re.M)[0]
# the pane's saved transcript is there too (and holds both runs)
assert ".json" in (rd / "files.txt").read_text(), (rd / "files.txt").read_text()
# two actual runs, each once, both from the reattached service session
assert rd_line.startswith("history_results=2 "), rd_line
rd_ids = re.search(r"found_ids=\[(.*)\]", rd_line)[1].split(", ")
assert all(i.startswith('"service.') for i in rd_ids) and len(set(rd_ids)) == 2, rd_ids

# a session service that accepts and never answers holds no window: the
# shell is started on the pane's reader thread, so ⌘T, typing and a split
# come back at once (timestamped from the app's own thread); each pane then
# falls back to a local shell, which runs what was typed meanwhile
fz = scratch / "frozen"
for sub in ("cfg/rift", "data/rift/sessiond", "work"):
    (fz / sub).mkdir(parents=True, exist_ok=True)
(fz / "data/rift/sessiond").chmod(0o700)
(fz / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\ndurable_sessions = true\nrestore_session = false\n'
    f'sync_os_theme = false\nworking_directory = {json.dumps(str(fz / "work"))}\n')
fz_listener = socket.socket(socket.AF_UNIX)
fz_listener.bind(str(fz / "data/rift/sessiond/sock"))
fz_listener.listen(16)
fz_held = []
def fz_accept():
    while True:
        try:
            fz_held.append(fz_listener.accept()[0])
        except OSError:
            return
threading.Thread(target=fz_accept, daemon=True).start()
def fz_stamp(name):
    return f"sh python3 -c 'import time; print(time.time())' > {fz}/{name}"
(fz / "t.rift").write_text("\n".join([
    "wait 300", fz_stamp("t0"), "key cmd+t", fz_stamp("t1"),
    "type echo HELD_OK\\n", fz_stamp("t2"), "key cmd+d", fz_stamp("t3"),
    "wait 12000", f"dump {fz}/after.txt", "quit"]) + "\n")
with (fz / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(fz / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(fz / "cfg"), XDG_DATA_HOME=str(fz / "data"),
                                 RIFT_PERSIST_SESSION="1"),
                        stdout=log, stderr=log, timeout=60).returncode
fz_listener.close()
assert rc == 0, died("frozen-service run failed", fz / "app.log")
fz_t = [float((fz / f"t{i}").read_text()) for i in range(4)]
assert all(b - a < 1.5 for a, b in zip(fz_t, fz_t[1:])), ("the window waited on the service", fz_t)
fz_after = (fz / "after.txt").read_text()
assert re.search(r"tabs=2 .*panes=\[\d+, \d+, \d+\]", fz_after), fz_after[:600]
assert 'cmd="echo HELD_OK"' in fz_after, fz_after[-3000:]
assert "Session service unavailable" in (fz / "app.log").read_text() + fz_after

# a focus change alone reaches the session file (no quit, no other edit):
# a restart then focuses the pane last used
fs = scratch / "focus-save"
for sub in ("cfg/rift", "data/rift"):
    (fs / sub).mkdir(parents=True, exist_ok=True)
(fs / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
    'sync_os_theme = false\n')
(fs / "t.rift").write_text("\n".join([
    "wait_prompt", "key cmd+d", "wait 500", cp(fs / "data/rift/session.toml", fs / "split.toml"),
    "key alt+cmd+left", "wait 500", cp(fs / "data/rift/session.toml", fs / "focused.toml"),
    "quit"]) + "\n")
with (fs / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(fs / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(fs / "cfg"),
                                 XDG_DATA_HOME=str(fs / "data"), RIFT_PERSIST_SESSION="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("focus-save run failed", fs / "app.log")
def focused_leaf(name):
    text = (fs / name).read_text()
    leaves = re.findall(r"^\[tabs\.layout\.(first|second)\]\n(?:(?!^\[).*\n)*?focused = true",
                        text, re.M)
    return leaves, text
split_focus, split_text = focused_leaf("split.toml")
assert split_focus == ["second"], split_text
moved_focus, moved_text = focused_leaf("focused.toml")
assert moved_focus == ["first"], moved_text

# a pane whose shell never reports its directory (no integration: plain sh)
# is named by its title in the accessibility tree, never an empty "Pane "
an = scratch / "a11y-no-cwd"
for sub in ("cfg/rift", "data"):
    (an / sub).mkdir(parents=True, exist_ok=True)
(an / "cfg/rift/config.toml").write_text(
    'shell = "/bin/sh"\nrestore_session = false\ndurable_sessions = false\nsync_os_theme = false\n')
(an / "t.rift").write_text(f"wait 1000\nkey cmd+d\nwait 500\ndump {an}/a.txt\nquit\n")
with (an / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(an / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(an / "cfg"), XDG_DATA_HOME=str(an / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("a11y-no-cwd run failed", an / "app.log")
a11y = (an / "a.txt").read_text()
assert "panes=2" in a11y and re.search(r"^  pane \d+ cwd=None", a11y, re.M), a11y[:1500]
groups = re.findall(r"^\s*Group \"(Pane [^\"]*)\"", a11y, re.M)
assert len(groups) == 2 and all(g != "Pane " and not g.startswith("Pane ,") for g in groups), groups

# OSC 9;4 progress (ConEmu / Windows Terminal): a running command's report
# shows on its tab (dump, the tab's accessible label, the Dock badge) and
# ends with the command; in a split, an error beats a newer normal report,
# and closing the pane that reports it leaves the other's; show_progress =
# false hides it (hot reload)
pg = scratch / "progress"
for sub in ("cfg/rift", "data"):
    (pg / sub).mkdir(parents=True, exist_ok=True)
pg_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
             'sync_os_theme = false\nconfirm_close_running = false\n')
(pg / "cfg/rift/config.toml").write_text(pg_config)
(pg / "off.toml").write_text(pg_config + "show_progress = false\n")
(pg / "on.toml").write_text(pg_config)
pg_steps = [
    "resize 1200 800", "wait_prompt", "wait 500",
    # ST-terminated, then the command ends: cleared
    "type printf '\\\\e]9;4;1;42\\\\e\\\\\\\\'; sleep 3\\n", "wait 1200", f"dump {pg}/set.txt",
    "wait 3000", f"dump {pg}/ended.txt",
    # BEL-terminated error in the left pane, a newer normal one in the right
    "type printf '\\\\e]9;4;2;70\\\\a'; sleep 30\\n", "wait 800",
    "key cmd+d", "wait_prompt", "wait 300",
    "type printf '\\\\e]9;4;1;20\\\\a'; sleep 30\\n", "wait 1200", f"dump {pg}/split.txt",
    f"sh cp {pg}/off.toml {pg}/cfg/rift/config.toml", "wait 2000", f"dump {pg}/off.txt",
    f"sh cp {pg}/on.toml {pg}/cfg/rift/config.toml", "wait 2000", f"dump {pg}/on.txt",
    "key alt+cmd+left", "wait 300", "key cmd+w", "wait 800", f"dump {pg}/closed.txt",
    # a quiet pane beside it, then the reporting one closed: nothing left
    "key cmd+d", "wait_prompt", "wait 300", "key alt+cmd+left", "wait 300", "key cmd+w",
    "wait 800", f"dump {pg}/cleared.txt",
    "quit",
]
(pg / "t.rift").write_text("\n".join(pg_steps) + "\n")
with (pg / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(pg / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(pg / "cfg"), XDG_DATA_HOME=str(pg / "data")),
                        stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("progress run failed", pg / "app.log")
def pg_state(name):
    text = (pg / f"{name}.txt").read_text()
    tab = re.search(r"^tab 0 .* progress=(\S+)", text, re.M)[1]
    return tab, re.findall(r"^    progress=(\S+)$", text, re.M), text
tab, panes, text = pg_state("set")
assert tab == "normal:42" and panes == ["normal:42"], (tab, panes)
assert 'Tab "' in text and ', 42% done"' in text, text[:2000]
if sys.platform == "darwin":
    assert 'dock_badge=Some("42%")' in text, text[:300]
tab, panes, text = pg_state("ended")
assert tab == "none:-" and panes == ["none:-"] and "dock_badge=None" in text, (tab, panes)
tab, panes, text = pg_state("split")
assert tab == "error:70" and sorted(panes) == ["error:70", "normal:20"], (tab, panes)
assert ', progress error"' in text, text[:2000]
# off: the tab shows nothing; the panes still keep their reports
tab, panes, text = pg_state("off")
assert tab == "none:-" and sorted(panes) == ["error:70", "normal:20"], (tab, panes)
assert ', progress error"' not in text and "dock_badge=None" in text, text[:2000]
tab, panes, text = pg_state("on")
assert tab == "error:70", (tab, panes)
# the error's (left) pane closed: the tab shows the right pane's report
tab, panes, text = pg_state("closed")
assert "panes=1" in text and panes == ["normal:20"] and tab == "normal:20", (tab, panes)
tab, panes, text = pg_state("cleared")
assert "panes=1" in text and panes == ["none:-"] and tab == "none:-", (tab, panes)
if sys.platform == "darwin":
    assert "dock_badge=None" in text, text[:300]

# elapsed_counts = "running" (time logging): the chip counts only while a
# command runs, keeps its total across a restart, and its hover card (which
# stays open from the chip into it) resets it with the Reset button
tc = scratch / "time-counted"
for sub in ("cfg/rift", "data/rift"):
    (tc / sub).mkdir(parents=True, exist_ok=True)
(tc / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
    'sync_os_theme = false\nelapsed_counts = "running"\nprompt_chips = ["cwd", "elapsed"]\n')
tc_env = dict(env, XDG_CONFIG_HOME=str(tc / "cfg"), XDG_DATA_HOME=str(tc / "data"),
              RIFT_PERSIST_SESSION="1")
def run_counted(name, steps, counted):
    (tc / "data/rift/session.toml").write_text(
        f'version = 3\nactive = 0\n\n[[tabs]]\ncwd = {json.dumps(str(work))}\n'
        + (f'counted = {counted}\n' if counted is not None else ''))
    (tc / f"{name}.rift").write_text("\n".join(["resize 1200 800", "wait_prompt", "wait 500",
                                                *steps, "quit"]) + "\n")
    with (tc / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tc / f"{name}.rift"), "--headless"],
                            env=tc_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"time-counted {name} run failed", tc / f"{name}.log")
    return [(tc / f"{name}-{i}.txt").read_text() for i in range(len(re.findall("dump", "\n".join(steps))))]
def elapsed_of(dump):
    return re.search(r"elapsed_secs=(\S+) elapsed_counting=(\S+) elapsed_button=(\S+)", dump).groups()
# no saved total and no command yet: nothing to show
idle, = run_counted("idle", [f"dump {tc}/idle-0.txt"], None)
assert elapsed_of(idle)[0] == "None" and '"Elapsed:' not in idle, idle
# a command counts while it runs, then the clock pauses
live = run_counted("live", ["type sleep 3\\n", "wait 1500", f"dump {tc}/live-0.txt", "wait 3500",
                            f"dump {tc}/live-1.txt", "wait 2500", f"dump {tc}/live-2.txt"], None)
running, paused, later = (elapsed_of(d) for d in live)
assert running[1] == "Some(true)" and running[0] in ("Some(1)", "Some(2)"), running
# the clock follows the panes at frame edges: a slow software renderer sees a
# 3 s command a moment late, so it can total just under 3
assert paused[1] == "Some(false)" and paused[0] in ("Some(2)", "Some(3)", "Some(4)"), paused
assert later[0] == paused[0] and later[1] == "Some(false)", (paused, later)
# a saved total is restored (older files, without one, start at 0: idle above)
(tc / "probe.rift").write_text("resize 1200 800\nwait_prompt\nwait 500\n"
                               f"dump {tc}/probe.txt\nquit\n")
(tc / "data/rift/session.toml").write_text(
    f'version = 3\nactive = 0\n\n[[tabs]]\ncwd = {json.dumps(str(work))}\ncounted = 7200\n')
subprocess.run([str(binary), "--script", str(tc / "probe.rift"), "--headless"], env=tc_env,
               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
probe = (tc / "probe.txt").read_text()
assert '"Elapsed:2h 00m"' in probe and elapsed_of(probe)[:2] == ("Some(7200)", "Some(false)"), probe
cx, cy, cw, ch = map(int, re.search(r"Time on this tab@(\d+),(\d+),(\d+),(\d+)", probe).groups())
chip_at = f"{cx + cw // 2} {cy + ch // 2}"
card, = run_counted("reset", [
    f"move {chip_at}", "wait 900", f"dump {tc}/reset-0.txt", f"shot {tc}/card.png"], 7200)
assert 'elapsed_tip=Some("Counted 2h 00m · paused")' in card, card
x, y, w, h = map(float, re.search(r"elapsed_button=Some\(\[([\d.]+), ([\d.]+), ([\d.]+), ([\d.]+)\]\)",
                                  card).groups())
button_at = f"{int(x + w / 2)} {int(y + h / 2)}"
# away from the chip the card closes; reaching the button from the chip keeps it
gone, kept, zeroed = run_counted("reset2", [
    "move 1000 300", "wait 900", f"dump {tc}/reset2-0.txt",
    f"move {chip_at}", "wait 900", f"move {button_at}", "wait 150", f"dump {tc}/reset2-1.txt",
    f"click {button_at}", "wait 300", f"dump {tc}/reset2-2.txt",
], 7200)
assert elapsed_of(gone)[2] == "None", gone
assert elapsed_of(kept)[2] != "None", kept
assert elapsed_of(zeroed)[0] == "Some(0)" and '"Elapsed:0m"' in zeroed, zeroed
assert 'elapsed_tip=Some("Counted 0m · paused")' in zeroed, zeroed
# panes that run at the same time count once (6 s + 6 s, 2.5 s apart: about 8
# s, never 12), the palette action zeroes the tab, and the session file takes
# the total when the command ends (a crash must not lose it)
once, after_reset = run_counted("union", [
    "type sleep 6\\n", "wait 1500", "key cmd+d", "wait 1000", "type sleep 6\\n", "wait 8500",
    f"sh cp {tc}/data/rift/session.toml {tc}/union-session.toml", f"dump {tc}/union-0.txt",
    "key cmd+p", "wait 400", "type reset time on", "wait 400", "key enter", "wait 500",
    f"dump {tc}/union-1.txt"], None)
union = elapsed_of(once)
assert union[1] == "Some(false)" and union[0] in ("Some(8)", "Some(9)"), union
saved_total = int(re.search(r"^counted = (\d+)$", (tc / "union-session.toml").read_text(), re.M)[1])
assert saved_total in (8, 9), saved_total
assert elapsed_of(after_reset)[0] == "Some(0)", after_reset

# the sticky header's buttons act on its block and do not scroll to it: copy
# copies (toast, a check for a moment), ⋮ opens the block menu; the rest of
# the sticky header still jumps back to the block
sb = scratch / "sticky-buttons"
for sub in ("cfg/rift", "data"):
    (sb / sub).mkdir(parents=True, exist_ok=True)
(sb / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\nsync_os_theme = false\n')
sb_env = dict(env, XDG_CONFIG_HOME=str(sb / "cfg"), XDG_DATA_HOME=str(sb / "data"))
sb_start = ["resize 1000 500", "wait_prompt", "type seq 1 300\\n", "wait 800", "scroll 40", "wait 300"]
(sb / "rects.rift").write_text("\n".join([*sb_start, f"dump {sb}/rects.txt", "quit"]) + "\n")
subprocess.run([str(binary), "--script", str(sb / "rects.rift"), "--headless"], env=sb_env,
               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
def sb_center(label):
    x, y, w, h = map(int, re.search(rf'"{label}@(\d+),(\d+),(\d+),(\d+)"',
                                    (sb / "rects.txt").read_text()).groups())
    return f"{x + w // 2} {y + h // 2}"
copy_at, menu_at = sb_center("Copy output"), sb_center("More actions")
(sb / "click.rift").write_text("\n".join([
    *sb_start, f"click {copy_at}", "wait 150", f"dump {sb}/copied.txt",
    f"click {menu_at}", "wait 300", f"dump {sb}/menu.txt", "key escape", "wait 200",
    f"click 300 {copy_at.split()[1]}", "wait 300", f"dump {sb}/jump.txt", "quit"]) + "\n")
with (sb / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sb / "click.rift"), "--headless"], env=sb_env,
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("sticky-buttons run failed", sb / "log.txt")
copied = (sb / "copied.txt").read_text()
assert "view_offset=40" in copied and 'toast=Some("Copied output")' in copied, copied[:800]
assert 'clipboard="1\\n2\\n3' in copied, "the copy button did not copy the block's output"
menu = (sb / "menu.txt").read_text()
assert "menu=true" in menu and "view_offset=40" in menu, menu[:800]
assert "view_offset=40" not in (sb / "jump.txt").read_text(), "the sticky header no longer jumps"

# a selection dragged below the pane's rows scrolls it down, above them up
(sb / "edge.rift").write_text("\n".join([
    *sb_start, "press 400 250", "move 420 270", "move 420 495", "wait 1200",
    f"dump {sb}/edge-down.txt", "move 420 5", "wait 1200", f"dump {sb}/edge-up.txt",
    "release 420 5", "quit"]) + "\n")
with (sb / "edge-log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sb / "edge.rift"), "--headless"], env=sb_env,
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("selection edge-scroll run failed", sb / "edge-log.txt")
down = (sb / "edge-down.txt").read_text()
assert "view_offset=0 " in down and "selection=true" in down, "a selection held below the pane did not scroll down"
up_off = int(re.search(r"view_offset=(\d+)", (sb / "edge-up.txt").read_text())[1])
assert up_off > 40, f"a selection held above the pane did not scroll up (view_offset={up_off})"

# file links: ⌘-click on each compiler / traceback format opens the file at
# its line and column, relative to the command's directory (a fake editor
# on PATH records its arguments); a path that does not exist is no link;
# right-click on a path or URL adds Open / Copy path / Reveal or Copy URL
fl = scratch / "file-links"
for sub in ("cfg/rift", "data", "bin", "w/src", "w/app"):
    (fl / sub).mkdir(parents=True, exist_ok=True)
(fl / "cfg/rift/config.toml").write_text(
    'blocks_start = "top"\nshell = "/bin/bash"\nrestore_session = false\n'
    'durable_sessions = false\nlink_editor = "windsurf"\n')
# one write per call: editors opened by clicks 300 ms apart can run at
# once on a loaded machine, and piecewise appends interleaved
(fl / "bin/windsurf").write_text(
    f'#!/bin/sh\nl=""\nfor a in "$@"; do l="$l$a|"; done\nprintf "%s\\n" "$l" >> {fl}/argv.txt\n')
(fl / "bin/windsurf").chmod(0o755)
for f in ("src/main.rs", "src/App.cs", "src/x.ts", "app/x.py", "app/m.py", "lib.pl"):
    (fl / "w" / f).write_text("x\n")
fl_lines = ["src/main.rs:42:7: error", "src/App.cs(12,3): error CS1002", "src/App.cs(13, 4): warning",
            "src/x.ts[5, 6] oops", "app/x.py, line 7, column 2: E501",
            '  File "app/m.py", line 9, in <module>', "died at lib.pl line 31.",
            "see https://example.com/doc ok", "missing/nope.rs:3:1 here"]
(fl / "w/links.txt").write_text("\n".join(fl_lines) + "\n")
fl_env = dict(env, XDG_CONFIG_HOME=str(fl / "cfg"), XDG_DATA_HOME=str(fl / "data"),
              BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_TEST_HEADLESS_EDITOR="1",
              PATH=f"{fl / 'bin'}:{os.environ['PATH']}")
fl_base = ["resize 1000 520", "wait_prompt", f"type cd {fl / 'w'}\\n", "wait 500",
           "type cat links.txt\\n", "wait 800"]
def fl_run(name, steps):
    (fl / f"{name}.rift").write_text("\n".join(fl_base + steps + ["quit"]) + "\n")
    with (fl / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(fl / f"{name}.rift"), "--headless"],
                            env=fl_env, stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"file link {name} run failed", fl / f"{name}.log")
fl_run("probe", [f"dump {fl}/probe.txt"])
fl_probe = (fl / "probe.txt").read_text()
fl_geo = next(l for l in fl_probe.splitlines() if l.strip().startswith("geometry "))
fl_cell = float(re.search(r"cell_w=([\d.]+)", fl_geo)[1])
fl_line_h = float(re.search(r"line_h=([\d.]+)", fl_geo)[1])
fl_rows = {int(a): (float(y), float(x)) for a, y, x in re.findall(r"(\d+):([\d.]+)@([\d.]+)", fl_geo)}
fl_start = int(re.findall(r'cmd="cat links.txt".*?output_start=Some\((\d+)\)', fl_probe)[-1])
def fl_at(i, needle):
    y, x = fl_rows[fl_start + i]
    return f"{x + (fl_lines[i].index(needle) + 0.5) * fl_cell:.1f} {y + fl_line_h / 2:.1f}"
fl_steps = []
for i, needle in [(0, "main"), (1, "App"), (2, "App"), (3, "x.ts"), (4, "x.py"), (5, "m.py"),
                  (6, "lib"), (8, "nope")]:
    fl_steps += ["mods cmd", f"click {fl_at(i, needle)}", "mods none", "wait 300"]
fl_steps += [f"rclick {fl_at(1, 'App')}", "wait 200", f"dump {fl}/rpath.txt",
             "key down", "key down", "key enter", "wait 200", f"dump {fl}/rpath-copied.txt",
             f"rclick {fl_at(7, 'example')}", "wait 200", f"dump {fl}/rurl.txt",
             "key down", "key down", "key enter", "wait 200", f"dump {fl}/rurl-copied.txt",
             f"rclick {fl_at(8, 'nope')}", "wait 200", f"dump {fl}/rmissing.txt", "key escape"]
fl_run("clicks", fl_steps)
fl_w = fl / "w"
# sorted: the editor processes may finish out of order
assert sorted((fl / "argv.txt").read_text().splitlines()) == sorted([
    f"--goto|{fl_w}/src/main.rs:42:7|", f"--goto|{fl_w}/src/App.cs:12:3|", f"--goto|{fl_w}/src/App.cs:13:4|",
    f"--goto|{fl_w}/src/x.ts:5:6|", f"--goto|{fl_w}/app/x.py:7:2|", f"--goto|{fl_w}/app/m.py:9|",
    f"--goto|{fl_w}/lib.pl:31|"]), (fl / "argv.txt").read_text()
def fl_dump(name):
    return (fl / f"{name}.txt").read_text()
assert 'menu_items=["Open", "Copy path", ' in fl_dump("rpath"), fl_dump("rpath")[:2000]
assert f'clipboard="{fl_w}/src/App.cs"' in fl_dump("rpath-copied"), fl_dump("rpath-copied")[:2000]
assert 'menu_items=["Open link", "Copy URL", "Copy command"' in fl_dump("rurl"), fl_dump("rurl")[:2000]
assert 'clipboard="https://example.com/doc"' in fl_dump("rurl-copied"), fl_dump("rurl-copied")[:2000]
assert 'menu_items=["Copy command"' in fl_dump("rmissing"), fl_dump("rmissing")[:2000]

# the overlay scrollbar (scrollbar = "always"): its thumb is the app's own
# view over the display list (top, middle, bottom; collapsed blocks and a
# block filter shrink the list), it sits in the pane's right padding beside
# the text, ticks mark the prompts and the failed command; dragging the
# thumb scrolls to the top, a click on the track pages (down below the
# thumb, up above it). `never` hides it.
sbar = scratch / "scrollbar"
(sbar / "cfg/rift").mkdir(parents=True, exist_ok=True)
sbar_cfg = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
            'sync_os_theme = false\nscrollbar = "always"\n')
(sbar / "cfg/rift/config.toml").write_text(sbar_cfg)
sbar_start = ["resize 1000 500", "wait_prompt", "type seq 1 200\\n", "wait 600",
              "type false\\n", "wait 400", "type seq 1 200\\n", "wait 600"]
def sbar_run(name, steps):
    (sbar / f"{name}.rift").write_text("\n".join([*sbar_start, *steps, "quit"]) + "\n")
    with (sbar / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sbar / f"{name}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(sbar / "cfg"),
                                     XDG_DATA_HOME=str(sbar / f"data-{name}")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"scrollbar {name} run failed", sbar / f"{name}.log")
def sbar_state(name):
    text = (sbar / f"{name}.txt").read_text()
    m = re.search(r"scrollbar=(\w+) len=(\d+) rows=(\d+) start=(\d+) "
                  r"track=([\d.]+),([\d.]+),([\d.]+),([\d.]+) thumb=([\d.]+),([\d.]+) "
                  r"ticks=(\d+),(\d+),(\d+),(\d+)", text)
    assert m, text[:2000]
    shown, (n, rows, start), track, thumb, ticks = (
        m[1], map(int, m.group(2, 3, 4)), [float(v) for v in m.group(5, 6, 7, 8)],
        [float(v) for v in m.group(9, 10)], [int(v) for v in m.group(11, 12, 13, 14)])
    # the thumb is the view: its share of the track, at its place
    th = min(max(rows / n * track[3], 16.0), track[3])
    want_y = track[1] + start / (n - rows) * (track[3] - th)
    assert shown == "shown" and abs(thumb[0] - want_y) < 0.6 and abs(thumb[1] - th) < 0.6, \
        (name, m[0])
    offset = int(re.search(r"view_offset=(\d+)", text)[1])
    assert start == n - rows - offset, (name, m[0], offset)
    return n, rows, start, track, thumb, ticks, text
sbar_run("probe", [f"dump {sbar}/bottom.txt", "scroll 1000", "wait 200", f"dump {sbar}/top.txt",
                   "scroll -150", "wait 200", f"dump {sbar}/middle.txt"])
n, rows, start, track, thumb, ticks, bottom = sbar_state("bottom")
assert start == n - rows and ticks[0] == 2 and ticks[2] == 1, ticks  # two prompts, one failure
# never over the text: the bar is right of every row's last column
bgeo = next(l for l in bottom.splitlines() if l.strip().startswith("geometry "))
cell_w, cols = float(re.search(r"cell_w=([\d.]+)", bgeo)[1]), int(re.search(r" cols=(\d+)", bgeo)[1])
text_x = min(float(x) for x in re.findall(r"@([\d.]+)", bgeo))
assert text_x + cols * cell_w <= track[0], (text_x, cols, cell_w, track)
assert track[0] + track[2] <= 1000 and track[2] <= 8, track
assert sbar_state("top")[2] == 0
_, _, mid_start, *_ = sbar_state("middle")
assert 0 < mid_start < n - rows
# collapsed: the first block's output leaves the list; a filter shrinks the
# list to the matching lines
tips = (sbar / "bottom.txt").read_text()
cx, cy, cw, ch = map(int, re.search(r'"Collapse output@(\d+),(\d+),(\d+),(\d+)"', tips).groups())
sbar_run("collapsed", [f"click {cx + cw // 2} {cy + ch // 2}", "wait 300",
                       f"dump {sbar}/collapsed.txt", "scroll 1000", "wait 200",
                       f"dump {sbar}/collapsed-top.txt"])
c_n, *_ = sbar_state("collapsed")
assert c_n <= n - 200, (c_n, n)
assert sbar_state("collapsed-top")[2] == 0
sbar_run("filtered", ["key cmd+up", "wait 200", "key alt+shift+f", "wait 200", "type 19",
                      "wait 400", f"dump {sbar}/filtered.txt"])
f_n, *_ = sbar_state("filtered")
assert "filter=BlockFilters { list: [(" in (sbar / "filtered.txt").read_text() and f_n < n - 150, (f_n, n)
# drag the thumb from the bottom to the top; a click on the track under the
# thumb pages down one view, above it up one
tx = track[0] + track[2] / 2
sbar_run("drag", [f"drag {tx} {thumb[0] + thumb[1] / 2} {tx} {track[1] - 50}", "wait 200",
                  f"dump {sbar}/dragged.txt", f"click {tx} {track[1] + track[3] - 3}",
                  "wait 200", f"dump {sbar}/paged.txt", f"click {tx} {track[1] + 3}",
                  "wait 200", f"dump {sbar}/paged-up.txt"])
assert sbar_state("dragged")[2] == 0
assert sbar_state("paged")[2] == rows
assert sbar_state("paged-up")[2] == 0
# beside a split divider (its grab reaches a few px into the left pane, over
# the bar): a click on the drawn bar pages that pane, the split stays
sbar_split = ["key cmd+d", "wait_prompt", "type seq 1 200\\n", "wait 600"]
sbar_run("split-probe", [*sbar_split, f"dump {sbar}/split-probe.txt"])
lt = sbar_state("split-probe")[3]
sbar_run("split", [*sbar_split, f"click {lt[0] + lt[2] - 2.5} {lt[1] + 3}", "wait 200",
                   f"dump {sbar}/split.txt"])
sb_split = (sbar / "split.txt").read_text()
assert re.search(r"^  pane 1 .* view_offset=[1-9]", sb_split, re.M), sb_split[:3000]
assert "children: [(1.0, Leaf(1)), (1.0, Leaf(2))]" in sb_split, sb_split[:3000]
(sbar / "cfg/rift/config.toml").write_text(sbar_cfg.replace('"always"', '"never"'))
sbar_run("never", [f"dump {sbar}/never.txt", "scroll 5", "wait 200", f"dump {sbar}/never-scrolled.txt"])
assert " scrollbar=none " in (sbar / "never-scrolled.txt").read_text()

# mask_secrets: a token shows as bullets (same width) and a git SHA does not;
# copies are masked (⌘C, a selection cutting the token, the transcript) while
# the block menu's "Copy output unmasked" copies it as printed; a click on
# the bullets shows the token for a moment. A probe run reads the geometry.
ms = scratch / "mask-secrets"
(ms / "cfg/rift").mkdir(parents=True, exist_ok=True)
(ms / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
    'sync_os_theme = false\nmask_secrets = true\nsave_transcripts = true\n')
ms_token = "ghp_" + "FAKE0123" * 5  # fake: never issued
ms_start = ["resize 1000 500", "wait_prompt",
            "type printf 'GH=ghp_%s%s%s%s%s\\\\nsha 3f2c9a1b7d4e5f60718293a4b5c6d7e8f9012345\\\\n'"
            " FAKE0123 FAKE0123 FAKE0123 FAKE0123 FAKE0123\\n", "wait 800"]
def ms_run(name, steps):
    (ms / f"{name}.rift").write_text("\n".join([*ms_start, *steps, "quit"]) + "\n")
    # each run its own data dir; the transcript is written with the session
    # (durable_sessions = false starts no session service)
    run_env = dict(env, XDG_CONFIG_HOME=str(ms / "cfg"), XDG_DATA_HOME=str(ms / f"data-{name}"),
                   RIFT_PERSIST_SESSION="1")
    with (ms / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(ms / f"{name}.rift"), "--headless"],
                            env=run_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"mask-secrets {name} run failed", ms / f"{name}.log")
    return (ms / f"{name}.txt").read_text() if (ms / f"{name}.txt").exists() else ""
probe = ms_run("probe", [f"dump {ms}/probe.txt"])
assert "masked=1 revealed=0" in probe, probe[:1500]
ms_geo = next(l for l in probe.splitlines() if l.strip().startswith("geometry "))
ms_cell = float(re.search(r"cell_w=([\d.]+)", ms_geo)[1])
ms_line = float(re.search(r"line_h=([\d.]+)", ms_geo)[1])
ms_out = int(re.findall(r'cmd="printf .*?output_start=Some\((\d+)\)', probe)[-1])
ms_y, ms_x = next((float(y), float(x)) for a, y, x in
                  re.findall(r"(\d+):([\d.]+)@([\d.]+)", ms_geo) if int(a) == ms_out)
ms_mid = int(ms_y + ms_line / 2)
at = lambda col: int(ms_x + (col + 0.5) * ms_cell)
ms_run("copies", [
    "key cmd+c", "wait 200", f"dump {ms}/copied.txt",
    # "GH=ghp_FAK" selected: the token's start only
    f"drag {at(0)} {ms_mid} {at(9)} {ms_mid}", "wait 200", "key cmd+c", "wait 200",
    f"dump {ms}/selected.txt", f"click {at(30)} {ms_mid + int(ms_line) * 3}", "wait 450",
    f"rclick {at(20)} {ms_mid}", "wait 300", f"dump {ms}/menu.txt",
    "key down", "key down", "key down", "key enter", "wait 200", f"dump {ms}/unmasked.txt",
    "wait 450", f"click {at(20)} {ms_mid}", "wait 200", f"dump {ms}/revealed.txt",
    f"shot {ms}/revealed.png",
    # the pane's transcript as saved after the block (a quit clears it)
    f"sh cp -R {ms}/data-copies/rift/transcripts {ms}/saved"])
def ms_clip(name):
    line = next(l for l in (ms / f"{name}.txt").read_text().splitlines()
                if l.startswith("clipboard="))
    return json.loads(line[len("clipboard="):])
bullets = "•" * len(ms_token)
assert ms_clip("copied") == f"GH={bullets}\nsha 3f2c9a1b7d4e5f60718293a4b5c6d7e8f9012345\n", \
    ms_clip("copied")
assert re.fullmatch(r"GH=•{5,8}", ms_clip("selected")), ms_clip("selected")
menu = (ms / "menu.txt").read_text()
assert 'menu_items=["Copy command", "Copy output", "Copy output unmasked",' in menu, menu[:1500]
assert ms_clip("unmasked").startswith(f"GH={ms_token}\n"), ms_clip("unmasked")
assert "masked=0 revealed=1" in (ms / "revealed.txt").read_text(), \
    (ms / "revealed.txt").read_text()[:1500]
# the transcript keeps the bullets only
saved = [p.read_text() for p in (ms / "saved").glob("*.json")]
assert saved and all(ms_token not in t for t in saved) and any(bullets in t for t in saved), saved

# a link takes a plain click: "prompt" shows its tooltip at once (no hover
# wait), "open" opens it; a drag across it selects and does neither
lk = scratch / "link-click"
for sub in ("cfg/rift", "data"):
    (lk / sub).mkdir(parents=True, exist_ok=True)
def run_link(mode):
    (lk / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        f'sync_os_theme = false\nlink_click = "{mode}"\n')
    (lk / f"{mode}.rift").write_text("\n".join([
        "resize 1000 400", "wait_prompt",
        'type printf "see https://example.com/x ok\\\\n"\\n', "wait 800",
        f"dump {lk}/{mode}-at.txt", "quit"]) + "\n")
    subprocess.run([str(binary), "--script", str(lk / f"{mode}.rift"), "--headless"],
                   env=dict(env, XDG_CONFIG_HOME=str(lk / "cfg"), XDG_DATA_HOME=str(lk / "data")),
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
    x, y, w, h = map(int, re.search(r'"https://example\.com/x@(\d+),(\d+),(\d+),(\d+)"',
                                    (lk / f"{mode}-at.txt").read_text()).groups())
    at = f"{x + w // 2} {y + h // 2}"
    (lk / f"{mode}.rift").write_text("\n".join([
        "resize 1000 400", "wait_prompt",
        'type printf "see https://example.com/x ok\\\\n"\\n', "wait 800",
        f"move {at}", "wait 150", f"dump {lk}/{mode}-hover.txt",
        f"click {at}", "wait 150", f"dump {lk}/{mode}-click.txt",
        f"drag {x - 20} {y + h // 2} {x + w - 4} {y + h // 2}", "wait 300",
        f"dump {lk}/{mode}-drag.txt", "quit"]) + "\n")
    with (lk / f"{mode}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(lk / f"{mode}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(lk / "cfg"),
                                     XDG_DATA_HOME=str(lk / "data")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"link-click {mode} run failed", lk / f"{mode}.log")
    return [(lk / f"{mode}-{s}.txt").read_text() for s in ("hover", "click", "drag")], \
        (lk / f"{mode}.log").read_text().count("headless: not opening https://example.com/x")
(hover, click, drag), opened = run_link("prompt")
assert "tooltip=Some(" not in hover, hover
assert 'tooltip=Some("https://example.com/x ⌘ Click")' in click, click
assert "selection=true" in drag and opened == 0, (drag, opened)
(hover, click, drag), opened = run_link("open")
assert "Headless: not opening https://example.com/x" in click and opened == 1, (click, opened)
assert "selection=true" in drag, drag

# a link on the alternate screen of a program that tracks the mouse (Claude
# Code, Codex) is a link too: the hand, the tooltip after the rest time, and
# the left click is Rift's ("prompt" shows, "open" opens) instead of the
# program's; a click beside it still reaches the program
ALT = ("type printf '\\e[?1049h\\e[?1000h\\e[?1006h\\e[2J\\e[3;1Hlink "
       "https://example.com/alt ok'; sleep 60\\n")
def run_alt_link(mode, steps):
    (lk / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        f'sync_os_theme = false\nlink_click = "{mode}"\n')
    (lk / f"alt-{mode}.rift").write_text("\n".join(
        ["resize 1000 400", "wait_prompt", ALT, "wait 1200"] + steps + ["quit"]) + "\n")
    with (lk / f"alt-{mode}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(lk / f"alt-{mode}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(lk / "cfg"),
                                     XDG_DATA_HOME=str(lk / "data")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"alt link {mode} run failed", lk / f"alt-{mode}.log")
    return (lk / f"alt-{mode}.log").read_text().count("headless: not opening https://example.com/alt")
run_alt_link("prompt", [f"dump {lk}/alt-at.txt"])
x, y, w, h = map(int, re.search(r'"https://example\.com/alt@(\d+),(\d+),(\d+),(\d+)"',
                                (lk / "alt-at.txt").read_text()).groups())
at = f"{x + w // 2} {y + h // 2}"
opened = run_alt_link("prompt", [
    f"move {at}", "wait 1200", f"dump {lk}/alt-hover.txt", "move 900 300", "wait 200",
    f"click {at}", "wait 150", f"dump {lk}/alt-click.txt", "click 900 300", "wait 400",
    f"dump {lk}/alt-beside.txt"])
hover = (lk / "alt-hover.txt").read_text()
assert "alt=true" in hover and "mouse_cursor=Pointer" in hover, hover
assert 'tooltip=Some("https://example.com/alt ⌘ Click")' in hover, hover
assert 'tooltip=Some("https://example.com/alt ⌘ Click")' in (lk / "alt-click.txt").read_text()
# only the click beside the link was reported (the program echoes it)
beside = (lk / "alt-beside.txt").read_text()
echo = re.search(r"^ +\| link https://example\.com/alt ok(.*)$", beside, re.M)[1]
assert echo.count("[<0;") == 2 and opened == 0, (echo, opened)
assert run_alt_link("open", [f"click {at}", "wait 300"]) == 1

# tooltips: none before the rest time, then the control's action and
# shortcut; a neighbour shows at once; a key hides it. On a fresh session the
# welcome list runs under the history pane: its scope chip still gets the
# tooltip and the click. A fixed history (10 commands) lays the pane out the
# same everywhere; a first run reads the controls' rects from the dump.
tip = scratch / "tip"
for sub in ("cfg/rift", "data", "home"):
    (tip / sub).mkdir(parents=True, exist_ok=True)
(tip / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(work))}\n')
(tip / "home/.bash_history").write_text("".join(f"echo past{i}\n" for i in range(10)))
tip_env = dict(env, XDG_CONFIG_HOME=str(tip / "cfg"), XDG_DATA_HOME=str(tip / "data"),
               HOME=str(tip / "home"), HISTFILE=str(tip / "home/.bash_history"),
               BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_SCRIPT_HISTORY="1")

def tip_run(name, steps):
    (tip / f"{name}.rift").write_text("\n".join(["resize 1000 700", "wait_prompt", *steps, "quit"]) + "\n")
    with (tip / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tip / f"{name}.rift"), "--headless"],
                            env=tip_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"tooltip {name} run failed", tip / f"{name}.log")

# (over the tab: its × shows only while it is hovered)
tip_run("probe", ["wait 300", "move 150 17", "wait 300", f"dump {tip}/probe-bar.txt",
                  "key ctrl+r", "wait 300", f"dump {tip}/probe-history.txt"])

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

# Settings › Files: link_editor = "rift" opens a hinted path in the viewer;
# an editor that is not installed falls back to the viewer with a toast;
# dotfiles stay out of the explorer until Show hidden files is pressed
fl = scratch / "files"
for sub in ("cfg/rift", "data", "work"):
    (fl / sub).mkdir(parents=True, exist_ok=True)
(fl / "work/.env").write_text("SECRET=1\n")
(fl / "work/visible.txt").write_text("hi\n")
(fl / "work/other.txt").write_text("x\n")
(fl / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nexplorer_hidden_files = false\nfile_editor = "nope"\n'
    'link_editor = "rift"\n' + f'working_directory = {json.dumps(str(fl / "work"))}\n')
(fl / "files.rift").write_text("\n".join([
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
    rc = subprocess.run([str(binary), "--script", str(fl / "files.rift"), "--headless"],
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
assert "explorer_hidden_files = true" in (fl / "cfg/rift/config.toml").read_text()

# A 2,001-entry folder: the explorer lists 2,000, a "Show more" row closes
# the list and ⏎ on it (keyboard) reaches entry 2,001
big = scratch / "bigdir"
for sub in ("cfg/rift", "data", "work"):
    (big / sub).mkdir(parents=True, exist_ok=True)
for n in range(2001):
    (big / f"work/f{n:05d}").write_text("")
(big / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    + f'working_directory = {json.dumps(str(big / "work"))}\n')
(big / "big.rift").write_text("\n".join([
    "wait_prompt", "key shift+cmd+e", "wait 500", f"dump {big}/open.txt"]
    + ["key down"] * 2001
    + ["key enter", "wait 400", f"dump {big}/more.txt", "quit"]) + "\n")
with (big / "big.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(big / "big.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(big / "cfg"),
                                 XDG_DATA_HOME=str(big / "data")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("big folder run failed", big / "big.log")
assert "rows=2001 selected=None" in (big / "open.txt").read_text()  # 2,000 + Show more
more = (big / "more.txt").read_text()
assert "rows=2001 selected=Some(\"f02000\")" in more, more[:1500]

# Shell history follows the configured shell: a multiline bash entry
# (HISTTIMEFORMAT) is one entry, and switching `shell` in config.toml while
# running replaces the imported history with the new shell's
hs = scratch / "histswap"
for sub in ("cfg/rift", "data/fish", "home", "bin", "work"):
    (hs / sub).mkdir(parents=True, exist_ok=True)
(hs / "home/.bash_history").write_text(
    "#1719400000\nbashonly_one\n#1719400001\nfor i in 1 2; do\necho $i\ndone\n")
(hs / "data/fish/fish_history").write_text("- cmd: fishonly_one\n  when: 1\n")
(hs / "bin/fish").write_text('#!/bin/sh\nexec /bin/bash "$@"\n')
(hs / "bin/fish").chmod(0o755)
hs_conf = lambda shell: (
    f'shell = {json.dumps(shell)}\nrestore_session = false\ndurable_sessions = false\n'
    f'working_directory = {json.dumps(str(hs / "work"))}\n')
(hs / "cfg/rift/config.toml").write_text(hs_conf("/bin/bash"))
(hs / "fish.toml").write_text(hs_conf(str(hs / "bin/fish")))
(hs / "hist.rift").write_text("\n".join([
    "wait_prompt", "wait 800",
    "key ctrl+r", "type for i", "wait 300", f"dump {hs}/bash.txt", "key escape",
    "key cmd+a", "key backspace",
    cp(hs / "fish.toml", hs / "cfg/rift/config.toml"), "wait 2000",
    "key ctrl+r", "type fishonly", "wait 300", f"dump {hs}/fish.txt", "key escape",
    "key cmd+a", "key backspace",
    "key ctrl+r", "type bashonly", "wait 300", f"dump {hs}/fish-bash.txt",
    "quit"]) + "\n")
hs_env = {k: v for k, v in env.items() if k != "HISTFILE"}
hs_env.update(XDG_CONFIG_HOME=str(hs / "cfg"), XDG_DATA_HOME=str(hs / "data"),
              HOME=str(hs / "home"), BASH_SILENCE_DEPRECATION_WARNING="1",
              RIFT_SCRIPT_HISTORY="1")
with (hs / "hist.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(hs / "hist.rift"), "--headless"],
                        env=hs_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("history swap run failed", hs / "hist.log")
assert 'history_selected=Some("for i in 1 2; do\\necho $i\\ndone")' in (hs / "bash.txt").read_text()
assert 'history_selected=Some("fishonly_one")' in (hs / "fish.txt").read_text()
assert "history_rows=0" in (hs / "fish-bash.txt").read_text()

# History menu (↑, Warp): prefix matches of the typed text, the pane's lines
# then the shell's, newest first and once; the selection previews in the
# line, ↓ past the newest and Esc restore what was typed, ⏎ runs the
# preview, typing keeps it; ↓ past the ⌃R pane's last row closes the pane
hm = scratch / "history-menu"
for sub in ("cfg/rift", "data", "home", "work"):
    (hm / sub).mkdir(parents=True, exist_ok=True)
(hm / "home/.bash_history").write_text("echo old-shell\necho beta\nls d1/x\n")
for name in ("d1/f1", "d1/f2", "d2/f3"):
    (hm / "work" / name).parent.mkdir(parents=True, exist_ok=True)
    (hm / "work" / name).write_text("")
(hm / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'completions_open_while_typing = true\n'
    f'working_directory = {json.dumps(str(hm / "work"))}\n')
(hm / "m.rift").write_text("\n".join([
    "wait_prompt",
    *[s for w in ("alpha", "beta", "gamma")
      for s in (f"type echo {w}\\n", "wait 300", "wait_prompt")],
    "type echo", "wait 300",
    "key up", "wait 100", f"dump {hm}/up1.txt",
    "key up", "wait 100", f"dump {hm}/up2.txt",
    "key down", "key down", "wait 100", f"dump {hm}/restored.txt",
    "key down", "wait 100", f"dump {hm}/down3.txt",
    "key up", "key up", "key up", "key escape", "wait 100", f"dump {hm}/esc.txt",
    "key ctrl+u", "type echo a", "key up", "type !", "wait 100", f"dump {hm}/typed.txt",
    "key ctrl+u", "type echo o", "key up", "wait 100", f"dump {hm}/shell.txt",
    "key ctrl+u", "type echo gam", "key ctrl+r", "wait 300", f"dump {hm}/pane.txt",
    "key down", "wait 100", f"dump {hm}/pane-closed.txt",
    # the panel opened as you type gives ↑ to the menu; one opened by a Tab
    # keeps ↑ / ↓
    "key ctrl+u", "type ls d", "wait 800", f"dump {hm}/as-typed.txt",
    "key up", "wait 100", f"dump {hm}/as-typed-up.txt", "key down", "wait 800",
    f"dump {hm}/as-typed-back.txt", "key escape", "key tab", "wait 800", "key down", "key up",
    "wait 100", f"dump {hm}/tabbed.txt",
    # previews are no edits: ⌘Z after a kept preview goes back to the typed
    # text, and after Esc it never brings a preview back; leaving the tab
    # closes the menu and puts the typed text back
    "key ctrl+u", "type echo", "key up", "key up", "wait 100", "key cmd+z", "wait 100",
    f"dump {hm}/undo-preview.txt",
    "key up", "key escape", "key cmd+z", "wait 100", f"dump {hm}/undo-esc.txt",
    "key ctrl+u", "type echo", "key up", "key cmd+t", "wait_prompt", "wait 300",
    f"dump {hm}/newtab.txt", "key cmd+1", "wait 300",
    # a paste under the menu makes the line the user's: the menu goes and
    # Esc no longer throws the paste away
    "key ctrl+u", "type echo", "key up", "paste  --x", "wait 100", "key escape", "wait 100",
    f"dump {hm}/pasted.txt",
    "key ctrl+u", "type echo", "key up", "key up", "key enter", "wait 500", "wait_prompt",
    f"dump {hm}/ran.txt",
    "quit"]) + "\n")
hm_env = {k: v for k, v in env.items() if k != "HISTFILE"}
hm_env.update(XDG_CONFIG_HOME=str(hm / "cfg"), XDG_DATA_HOME=str(hm / "data"),
              HOME=str(hm / "home"), BASH_SILENCE_DEPRECATION_WARNING="1",
              RIFT_SCRIPT_HISTORY="1")
with (hm / "m.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(hm / "m.rift"), "--headless"],
                        env=hm_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("history menu run failed", hm / "m.log")
hmt = lambda name: (hm / f"{name}.txt").read_text()
assert 'history_menu=true history_menu_selected=0 history_menu_typed=Some("echo")' \
    in hmt("up1") and 'editor="echo gamma"' in hmt("up1"), hmt("up1")[:3000]
assert 'completion_items="echo gamma"(history) "echo beta"(history) "echo alpha"(history) ' \
    '"echo old-shell"(history)' in hmt("up1"), hmt("up1")[:3000]
assert 'editor="echo beta" cursor=9' in hmt("up2"), hmt("up2")[:3000]
for name in ("restored", "down3", "esc"):
    assert "history_menu=false" in hmt(name) and 'editor="echo" cursor=4' in hmt(name), \
        (name, hmt(name)[:3000])
assert "history_menu=false" in hmt("typed") and 'editor="echo alpha!"' in hmt("typed"), \
    hmt("typed")[:3000]
assert 'editor="echo old-shell"' in hmt("shell"), hmt("shell")[:3000]
assert "history=true" in hmt("pane") and "history_rows=1" in hmt("pane"), hmt("pane")[:3000]
assert "history=false" in hmt("pane-closed"), hmt("pane-closed")[:3000]
assert 'completion_items="d1/"(path) "d2/"(path)' in hmt("as-typed") \
    and "completion_asked=false" in hmt("as-typed"), hmt("as-typed")[:3000]
assert "history_menu=true" in hmt("as-typed-up") and 'editor="ls d1/x"' in hmt("as-typed-up"), \
    hmt("as-typed-up")[:3000]
# ↓ past the newest is back to typing: the panel returns
assert 'completion_items="d1/"(path) "d2/"(path)' in hmt("as-typed-back") \
    and 'editor="ls d" cursor=4' in hmt("as-typed-back"), hmt("as-typed-back")[:3000]
assert 'completion_items="d1/"(path) "d2/"(path)' in hmt("tabbed") \
    and "history_menu=false history_menu_selected=0" in hmt("tabbed") \
    and "completion_asked=true" in hmt("tabbed") and "completion_tab=true" in hmt("tabbed") \
    and 'editor="ls d" cursor=4' in hmt("tabbed"), hmt("tabbed")[:3000]
assert "history_menu=false" in hmt("undo-preview") \
    and re.search(r'pane 1 .*editor="echo" cursor=4', hmt("undo-preview")), hmt("undo-preview")[:3000]
assert "history_menu=false" in hmt("undo-esc") \
    and not re.search(r'pane 1 .*editor="echo (alpha|beta|gamma)"', hmt("undo-esc")), \
    hmt("undo-esc")[:3000]
assert "history_menu=false" in hmt("newtab") \
    and re.search(r'pane 1 .*editor="echo" cursor=4', hmt("newtab")), hmt("newtab")[:3000]
assert "history_menu=false" in hmt("pasted") \
    and re.search(r'pane 1 .*editor="echo gamma ?--x"', hmt("pasted")), hmt("pasted")[:3000]
# a scripted run that did not opt in (RIFT_SCRIPT_RECORD_HISTORY) records
# nothing, even under its own XDG_DATA_HOME
assert not (hm / "data/rift/history.json").exists()
hm_ran = hmt("ran")
assert hm_ran.count('cmd="echo beta" exit=Some(0)') == 2 and 'editor="" ' in hm_ran, \
    hm_ran[:3000]

# Rift's own history ranks by directory across restarts: run 1 runs
# `pnpm dev` in a, then the colliding (newer) `pnpm dev:web` in b, plus a
# leading-space command and a secret, which are never saved; run 2, a fresh
# session over the same data dir, types `pnpm de`: in a the ghost text and
# the first history row are a's command, in b b's. Run 1 does not read the
# store (no RIFT_SCRIPT_HISTORY); run 2 opts in.
dh = scratch / "dir-history"
for sub in ("cfg/rift", "data", "home", "a", "b"):
    (dh / sub).mkdir(parents=True, exist_ok=True)
for rc_file in (".bash_profile", ".bashrc"):  # macOS login / Linux shim
    (dh / "home" / rc_file).write_text("PS1='$ '\npnpm() { :; }\n")
(dh / "home/.bash_history").write_text("pnpm dedupe\n")
(dh / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'working_directory = {json.dumps(str(dh / "a"))}\n')
dh_env = dict(env, XDG_CONFIG_HOME=str(dh / "cfg"), XDG_DATA_HOME=str(dh / "data"),
              HOME=str(dh / "home"), HISTFILE=str(dh / "home/.bash_history"),
              BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_SCRIPT_RECORD_HISTORY="1")
dh_env.pop("RIFT_SCRIPT_HISTORY", None)
# `cat ./deploy.txt…` collides the same way: the ↑ menu for `cat ./de`
# lists its lines in the same order (the completion menu lists word
# candidates only)
for where in ("a", "b"):
    (dh / where / "deploy.txt").write_text("")
def dh_run(name, steps, extra=None):
    (dh / f"{name}.rift").write_text("\n".join(["wait_prompt", "wait 500", *steps, "quit"]) + "\n")
    with (dh / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(dh / f"{name}.rift"), "--headless"],
                            env=dict(dh_env, **(extra or {})), stdout=log, stderr=log,
                            timeout=120).returncode
    assert rc == 0, died(f"directory history {name} run failed", dh / f"{name}.log")
dh_run("record", [
    "type pnpm dev\\n", "wait 800",
    "type cat ./deploy.txt\\n", "wait 500",
    f"type cd {dh / 'b'}\\n", "wait 500",
    "type pnpm dev:web\\n", "wait 800",
    "type cat ./deploy.txt ./deploy.txt\\n", "wait 500",
    "key space", "type pnpm dev:hidden\\n", "wait 500",
    "type export TOKEN=abcd1234efgh\\n", "wait 800",
])
saved = json.loads((dh / "data/rift/history.json").read_text())
commands = [e["command"] for e in saved["entries"]]
assert "pnpm dev" in commands and "pnpm dev:web" in commands, commands
assert not any("hidden" in c or "TOKEN" in c for c in commands), commands
# each place: the ghost text as typed, the ↑ menu's first line, the ⌃R
# pane's first row for that query, and the ↑ menu's lines for `cat ./de`
dh_probe = lambda where: [
    "type pnpm de", "wait 500", f"dump {dh}/{where}-typed.txt",
    "key up", "wait 100", f"dump {dh}/{where}-up.txt", "key escape",
    "key ctrl+r", "wait 300", f"dump {dh}/{where}-search.txt", "key escape",
    "key cmd+a", "key backspace",
    "type cat ./de", "wait 500", "key up", "wait 100", f"dump {dh}/{where}-panel.txt",
    "key escape",
    "key cmd+a", "key backspace"]
dh_run("rank", [
    *dh_probe("a"),
    f"type cd {dh / 'b'}\\n", "wait 500",
    *dh_probe("b"),
], {"RIFT_SCRIPT_HISTORY": "1"})

def dh_first(where):
    read = lambda kind: (dh / f"{where}-{kind}.txt").read_text()
    ghost = re.search(r" ghost=(\"[^\"]*\")", read("typed"))[1]
    picked = re.search(r"history_selected=Some\(\"([^\"]*)\"\)", read("search"))
    rows = re.findall(r'"([^"]*)"\(history\)', re.search(r"^completion_items=(.*)$",
                                                        read("panel"), re.M)[1])
    return ghost, picked and picked[1], rows

assert dh_first("a") == ('"v"', "pnpm dev", ["cat ./deploy.txt", "cat ./deploy.txt ./deploy.txt"]), \
    dh_first("a")
assert dh_first("b") == ('"v:web"', "pnpm dev:web", ["cat ./deploy.txt ./deploy.txt",
                                                     "cat ./deploy.txt"]), dh_first("b")
for where, line in (("a", "pnpm dev"), ("b", "pnpm dev:web")):
    dh_up = (dh / f"{where}-up.txt").read_text()
    assert "history_menu=true" in dh_up and f'editor="{line}"' in dh_up, (where, dh_up[:3000])

# Next-command history: the ghost text suggests what usually follows the
# pane's previous command (Warp). Tab 1 runs `cd project1`, the backup, then
# the restore; tab 2 (another shell session) types the script's prefix:
# after another command the newest in the directory (the restore), after
# `cd project1` in the same directory with the same exit code the backup.
nc = scratch / "next-command"
for sub in ("cfg/rift", "data", "home", "x/project1/scripts"):
    (nc / sub).mkdir(parents=True, exist_ok=True)
for rc_file in (".bash_profile", ".bashrc"):  # macOS login / Linux shim
    (nc / "home" / rc_file).write_text("PS1='$ '\n")
(nc / "x/project1/scripts/db.sh").write_text("exit 0\n")
(nc / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'working_directory = {json.dumps(str(nc / "x"))}\n')
nc_env = dict(env, XDG_CONFIG_HOME=str(nc / "cfg"), XDG_DATA_HOME=str(nc / "data"),
              HOME=str(nc / "home"), HISTFILE=str(nc / "home/.bash_history"),
              BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_SCRIPT_RECORD_HISTORY="1")
nc_env.pop("RIFT_SCRIPT_HISTORY", None)
# the directory as the first tab's shell has it (/tmp is /private/tmp on
# macOS): the previous command matches by its directory string
nc_x = (nc / "x").resolve()
nc_steps = [
    "wait_prompt", "wait 500",
    "type cd project1\\n", "wait 500",
    "type bash scripts/db.sh backup\\n", "wait 800",
    "type bash scripts/db.sh restore --local --force\\n", "wait 800",
    "key cmd+t", "wait_prompt", "wait 800",
    f"type cd {nc_x}\\n", "wait 500",
    "type true\\n", "wait 500",
    f"type cd {nc_x / 'project1'}\\n", "wait 500",
    "type bash scripts/db.sh", "wait 800", f"dump {nc}/other.txt",
    "key cmd+a", "key backspace",
    f"type cd {nc_x}\\n", "wait 500",
    "type cd project1\\n", "wait 500",
    "type bash scripts/db.sh", "wait 800", f"dump {nc}/after-prev.txt",
    "quit",
]
(nc / "t.rift").write_text("\n".join(nc_steps) + "\n")
with (nc / "run.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(nc / "t.rift"), "--headless"],
                        env=nc_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("next-command run failed", nc / "run.log")
nc_saved = json.loads((nc / "data/rift/history.json").read_text())["entries"]
assert len({e.get("session") for e in nc_saved}) == 2 and all("seq" in e for e in nc_saved), \
    nc_saved
nc_ghost = lambda name: re.search(r" ghost=(\"[^\"]*\")", (nc / f"{name}.txt").read_text())[1]
assert nc_ghost("other") == '" restore --local --force"', nc_ghost("other")
assert nc_ghost("after-prev") == '" backup"', nc_ghost("after-prev")

# Tab and the ghost text (Warp): Tab completes the word — one match goes in
# with a space after it (a directory: none), several insert their common
# prefix and open the menu, Tab / ⇧Tab cycle it (wrapping), typing narrows
# it, ⏎ accepts; Tab never takes the ghost text; → ⌃F ⌃E ⌘→ End take it at
# the end of the line and only move elsewhere, ⌥→ / ⌃⌥→ one word / word
# part; Esc closes the menu and clears it. The menu opens as you type only
# with completions_open_while_typing (or the old completions_menu): ⏎ then
# accepts unless the word already is the row, then runs it, no space added.
# tab_behavior = "autosuggestion" gives Tab the ghost text and ⌃Space the
# completions; vim Normal's l $ take it, w e one word, its Esc clears it
wt = scratch / "warp-tab"
for sub in ("cfg/rift", "data", "home", "work/src/lib"):
    (wt / sub).mkdir(parents=True, exist_ok=True)
for name in ("alpha.txt", "alpine.txt", "beta.txt"):
    (wt / "work" / name).write_text("")
for rc_file in (".bash_profile", ".bashrc"):  # macOS login / Linux shim
    (wt / "home" / rc_file).write_text("PS1='$ '\n")
(wt / "home/.bash_history").write_text("echo hello world\ngit checkout main\n")
wt_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
             f'working_directory = {json.dumps(str(wt / "work"))}\n')
wt_env = dict(env, XDG_CONFIG_HOME=str(wt / "cfg"), XDG_DATA_HOME=str(wt / "data"),
              HOME=str(wt / "home"), HISTFILE=str(wt / "home/.bash_history"),
              BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_SCRIPT_HISTORY="1")
def wt_run(name, steps, extra=""):
    (wt / "cfg/rift/config.toml").write_text(wt_config + extra)
    (wt / f"{name}.rift").write_text("\n".join(["wait_prompt", "wait 500", *steps, "quit"]) + "\n")
    with (wt / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(wt / f"{name}.rift"), "--headless"],
                            env=wt_env, stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"warp tab {name} run failed", wt / f"{name}.log")
wt_fresh = lambda line: ["key ctrl+u", f"type {line}", "wait 500"]
wt_run("tab", [
    "type cat al", "wait 500", f"dump {wt}/typed.txt",
    "key tab", "wait 500", f"dump {wt}/lcp.txt",
    "key tab", "wait 100", f"dump {wt}/next.txt",
    "key tab", "wait 100", f"dump {wt}/wrap.txt",
    "key shift+tab", "wait 100", f"dump {wt}/prev.txt",
    "type i", "wait 300", f"dump {wt}/narrowed.txt",
    "key enter", "wait 300", f"dump {wt}/accepted.txt",
    *wt_fresh("cat be"), "key tab", "wait 400", f"dump {wt}/single.txt",
    *wt_fresh("cat sr"), "key tab", "wait 400", f"dump {wt}/dir.txt",
    *wt_fresh("git ch"), f"dump {wt}/ghost.txt",
    "key tab", "wait 500", f"dump {wt}/tab-ghost.txt",
    "key escape", "wait 100", f"dump {wt}/esc-menu.txt",
    *wt_fresh("echo h"), "key escape", "wait 100", f"dump {wt}/esc-ghost.txt",
    *[s for key in ("right", "ctrl+f", "ctrl+e", "cmd+right", "end")
      for s in (*wt_fresh("echo h"), f"key {key}", "wait 200", f"dump {wt}/at-end-{key}.txt")],
    *[s for key in ("right", "ctrl+e", "cmd+right", "end")
      for s in (*wt_fresh("echo h"), "key left", f"key {key}", "wait 200",
                f"dump {wt}/mid-{key}.txt")],
    *wt_fresh("echo h"), "key alt+right", "wait 200", f"dump {wt}/word.txt",
    *wt_fresh("echo h"), "key ctrl+alt+right", "wait 200", f"dump {wt}/subword.txt",
    # ⌃Space opens the completions with the default Tab too
    *wt_fresh("cat al"), "key escape", "key ctrl+space", "wait 400",
    f"dump {wt}/ctrl-space.txt",
])
wt_run("ayt", ["type cat al", "wait 600", f"dump {wt}/ayt-open.txt",
               "key enter", "wait 300", f"dump {wt}/ayt-enter.txt",
               *wt_fresh("cat alpha.txt"), "wait 200", f"dump {wt}/ayt-full.txt",
               "key enter", "wait 600", f"dump {wt}/ayt-ran.txt"],
       "completions_open_while_typing = true\n")
wt_run("legacy", ["type cat al", "wait 600", f"dump {wt}/legacy-open.txt"],
       "completions_menu = true\n")
wt_run("auto", ["type echo h", "wait 500", "key tab", "wait 200", f"dump {wt}/auto-tab.txt",
                *wt_fresh("cat be"), "key escape", "key tab", "wait 400", f"dump {wt}/auto-none.txt",
                *wt_fresh("cat al"), "key tab", "wait 400", f"dump {wt}/auto-ghost-al.txt",
                *wt_fresh("cat al"), "key escape", "key ctrl+space", "wait 400",
                f"dump {wt}/auto-ctrl-space.txt"],
       'tab_behavior = "autosuggestion"\n')
wt_run("vim", ["type echo h", "wait 500", "key escape", "wait 100", f"dump {wt}/vim-normal.txt",
               "type w", "wait 100", f"dump {wt}/vim-w.txt",
               "type l", "wait 100", f"dump {wt}/vim-l.txt",
               *wt_fresh("echo h"), "key escape", "type 0", "type l", "wait 100",
               f"dump {wt}/vim-mid.txt",
               "type $", "type $", "wait 100", f"dump {wt}/vim-dollar.txt",
               *wt_fresh("echo h"), "key escape", "type e", "wait 100", f"dump {wt}/vim-e.txt",
               *wt_fresh("echo h"), "key escape", "key escape", "wait 100",
               f"dump {wt}/vim-esc.txt"],
       "vim_mode = true\n")
wtt = lambda name: (wt / f"{name}.txt").read_text()
def wt_state(name):
    text = wtt(name)
    editor = re.search(r'editor="((?:[^"\\]|\\.)*)" cursor=(\d+)', text)
    menu = re.search(r' completion=(\w+) ghost="((?:[^"\\]|\\.)*)" items=(\d+)', text)
    rows = re.findall(r'"([^"]*)"\(\w+\)', re.search(r"^completion_items=(.*)$", text, re.M)[1])
    tab = re.search(r"completion_tab=(\w+) completion_selected=(\d+)", text)
    return (editor[1], int(editor[2]), menu[1] == "true", menu[2], rows,
            tab[1] == "true", int(tab[2]))
# typing opens no menu (completions_open_while_typing is off by default)
assert wt_state("typed")[:4] == ("cat al", 6, False, "pha.txt"), wt_state("typed")
# two matches: their common prefix, the menu, the first row; no ghost beside it
assert wt_state("lcp") == ("cat alp", 7, True, "", ["alpha.txt", "alpine.txt"], True, 0), \
    wt_state("lcp")
assert wt_state("next")[6] == 1 and wt_state("wrap")[6] == 0 and wt_state("prev")[6] == 1, \
    (wt_state("next"), wt_state("wrap"), wt_state("prev"))
assert wt_state("narrowed")[:5] == ("cat alpi", 8, True, "", ["alpine.txt"]), wt_state("narrowed")
assert wt_state("accepted")[:3] == ("cat alpine.txt ", 15, False), wt_state("accepted")
# one match: inserted, a space after a file, none after a directory
assert wt_state("single")[:3] == ("cat beta.txt ", 13, False), wt_state("single")
assert wt_state("dir")[:3] == ("cat src/", 8, False), wt_state("dir")
# Tab with ghost text completes the word, never takes the ghost
assert wt_state("ghost")[3] == "eckout main", wt_state("ghost")
assert wt_state("tab-ghost")[:4] == ("git che", 7, True, ""), wt_state("tab-ghost")
assert "checkout" in wt_state("tab-ghost")[4], wt_state("tab-ghost")
assert wt_state("esc-menu")[:3] == ("git che", 7, False), wt_state("esc-menu")
assert wt_state("esc-ghost")[:4] == ("echo h", 6, False, ""), wt_state("esc-ghost")
for key in ("right", "ctrl+f", "ctrl+e", "cmd+right", "end"):
    assert wt_state(f"at-end-{key}")[:4] == ("echo hello world", 16, False, ""), \
        (key, wt_state(f"at-end-{key}"))
# mid-line they only move (End scrolls and never moves the cursor)
for key, cursor in (("right", 6), ("ctrl+e", 6), ("cmd+right", 6), ("end", 5)):
    assert wt_state(f"mid-{key}")[:4] == ("echo h", cursor, False, "ello world"), \
        (key, wt_state(f"mid-{key}"))
assert wt_state("word")[:4] == ("echo hello", 10, False, " world"), wt_state("word")
assert wt_state("subword")[:4] == ("echo hello", 10, False, " world"), wt_state("subword")
# as you type: the menu opens; ⏎ accepts (no space), then runs what already is the row
for name in ("ayt-open", "legacy-open"):
    assert wt_state(name)[:5] == ("cat al", 6, True, "pha.txt", ["alpha.txt", "alpine.txt"]), \
        (name, wt_state(name))
assert wt_state("ayt-enter")[:3] == ("cat alpha.txt", 13, False), wt_state("ayt-enter")
assert wt_state("ayt-full")[:3] == ("cat alpha.txt", 13, True), wt_state("ayt-full")
assert wt_state("ayt-ran")[:2] == ("", 0) and 'cmd="cat alpha.txt"' in wtt("ayt-ran"), \
    wtt("ayt-ran")[:2000]
# autosuggestion: Tab takes the ghost; with none it completes; ⌃Space opens the menu
assert wt_state("auto-tab")[:2] == ("echo hello world", 16), wt_state("auto-tab")
assert wt_state("auto-none")[:3] == ("cat beta.txt ", 13, False), wt_state("auto-none")
assert wt_state("auto-ghost-al")[:2] == ("cat alpha.txt", 13), wt_state("auto-ghost-al")
assert wt_state("auto-ctrl-space")[:3] == ("cat alp", 7, True), wt_state("auto-ctrl-space")
assert wt_state("ctrl-space")[:3] == ("cat alp", 7, True), wt_state("ctrl-space")
# vim: Insert's Esc keeps the ghost; Normal's w / e take a word, l / $ the
# rest, on the last character only; Normal's Esc clears it
assert wt_state("vim-normal")[:4] == ("echo h", 5, False, "ello world") \
    and "vim=Some(Normal)" in wtt("vim-normal"), wtt("vim-normal")[:1500]
# ... and shows it after the block cursor on the last character, its chip too
# (`l` takes what is on screen)
assert 'Button "Accept suggestion key: →"' in wtt("vim-normal"), wtt("vim-normal")[-3000:]
assert wt_state("vim-w")[:4] == ("echo hello", 9, False, " world"), wt_state("vim-w")
assert wt_state("vim-l")[:4] == ("echo hello world", 15, False, ""), wt_state("vim-l")
assert wt_state("vim-mid")[:4] == ("echo h", 1, False, "ello world"), wt_state("vim-mid")
assert wt_state("vim-dollar")[:2] == ("echo hello world", 15), wt_state("vim-dollar")
assert wt_state("vim-e")[:4] == ("echo hello", 9, False, " world"), wt_state("vim-e")
assert wt_state("vim-esc")[:4] == ("echo h", 5, False, "") \
    and "vim=Some(Normal)" in wtt("vim-esc"), wtt("vim-esc")[:1500]

# Ghost text chips (Warp): the accept-key chip shows by default
# (autosuggestion_hint) and names the key; a click opens its menu (→, Tab,
# ⇧→, Custom…, the current one checked; Esc closes it). Picking Tab writes
# tab_behavior = "autosuggestion" (Tab then takes the ghost), ⇧→ binds
# accept_autosuggestion (⇧→ still selects with no ghost to take). The ✕
# shows only with autosuggestion_ignore_button: a click ignores the whole
# suggested line, the next suggestion takes its place, and it stays ignored
# after a relaunch over the same data dir (RIFT_SCRIPT_RECORD_IGNORED);
# running the line or the palette's "Clear ignored suggestions" suggests it
# again.
import tomllib
gc = scratch / "ghost-chips"
for sub in ("cfg/rift", "data", "home", "work"):
    (gc / sub).mkdir(parents=True, exist_ok=True)
for rc_file in (".bash_profile", ".bashrc"):  # macOS login / Linux shim
    (gc / "home" / rc_file).write_text("PS1='$ '\n")
gc_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
             f'working_directory = {json.dumps(str(gc / "work"))}\n')
gc_env = dict(env, XDG_CONFIG_HOME=str(gc / "cfg"), XDG_DATA_HOME=str(gc / "data"),
              HOME=str(gc / "home"), HISTFILE=str(gc / "home/.bash_history"),
              BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_SCRIPT_HISTORY="1",
              RIFT_SCRIPT_RECORD_IGNORED="1")
def gc_run(name, steps, extra=""):
    # the shell history is the fixture again each run (a run appends to it)
    (gc / "home/.bash_history").write_text("git status --short\ngit status\n")
    if extra is not None:
        (gc / "cfg/rift/config.toml").write_text(gc_config + extra)
    (gc / f"{name}.rift").write_text("\n".join(["wait_prompt", "wait 500", *steps, "quit"]) + "\n")
    with (gc / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(gc / f"{name}.rift"), "--headless"],
                            env=gc_env, stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"ghost chips {name} run failed", gc / f"{name}.log")
gct = lambda name: (gc / f"{name}.txt").read_text()
gc_ghost = lambda name: re.search(r' ghost="((?:[^"\\]|\\.)*)"', gct(name))[1]
gc_editor = lambda name: re.search(r' editor="((?:[^"\\]|\\.)*)" cursor=', gct(name))[1]
gc_toml = lambda: tomllib.loads((gc / "cfg/rift/config.toml").read_text())
gc_run("hint", [
    "type git st", "wait 600", f"dump {gc}/hint-typed.txt",
    "a11y click Accept suggestion key: →", "wait 200", f"dump {gc}/hint-menu.txt",
    "key escape", "wait 100", f"dump {gc}/hint-esc.txt",
    "a11y click Accept suggestion key: →", "wait 200", "a11y click Tab", "wait 600",
    f"dump {gc}/hint-picked-tab.txt",
    "key tab", "wait 300", f"dump {gc}/hint-tab-accepts.txt",
])
assert gc_ghost("hint-typed") == "atus", gc_ghost("hint-typed")
assert 'Button "Accept suggestion key: →"' in gct("hint-typed"), gct("hint-typed")[-3000:]
# the ✕ is off by default
assert "Ignore this suggestion" not in gct("hint-typed")
assert 'menu_items=["→", "Tab", "⇧→", "Custom…"] menu_selected=Some(0)' in gct("hint-menu"), \
    re.search(r"^menu_items=.*$", gct("hint-menu"), re.M)
assert 'Menu "Accept suggestion key"' in gct("hint-menu") \
    and 'MenuItemRadio "→" toggled=True' in gct("hint-menu") \
    and 'MenuItemRadio "Tab" toggled=False' in gct("hint-menu"), gct("hint-menu")[-2000:]
assert " menu=false " in gct("hint-esc") and gc_ghost("hint-esc") == "atus", gct("hint-esc")[:3000]
assert gc_toml().get("tab_behavior") == "autosuggestion", gc_toml()
assert 'Button "Accept suggestion key: Tab"' in gct("hint-picked-tab"), gct("hint-picked-tab")[-3000:]
assert gc_editor("hint-tab-accepts") == "git status", gc_editor("hint-tab-accepts")
# ⇧→: written as the binding; it takes the ghost, and selects with none to take
gc_run("shift", [
    "type git st", "wait 600",
    "a11y click Accept suggestion key: →", "wait 200", "a11y click ⇧→", "wait 600",
    f"dump {gc}/shift-picked.txt",
    "key shift+right", "wait 300", f"dump {gc}/shift-accepts.txt",
    "key ctrl+u", "type abc", "wait 300", "key left", "key left", "key shift+right", "type X",
    "wait 200", f"dump {gc}/shift-selects.txt",
])
assert gc_toml().get("keybindings", {}).get("accept_autosuggestion") == "shift+right", gc_toml()
assert gc_toml().get("tab_behavior", "completions") == "completions", gc_toml()
assert 'Button "Accept suggestion key: ⇧→"' in gct("shift-picked"), gct("shift-picked")[-3000:]
assert gc_editor("shift-accepts") == "git status", gc_editor("shift-accepts")
assert gc_editor("shift-selects") == "aXc", gc_editor("shift-selects")
# the hint off: no chip
gc_run("nohint", ["type git st", "wait 600", f"dump {gc}/nohint.txt"],
       "autosuggestion_hint = false\n")
assert gc_ghost("nohint") == "atus" and "Accept suggestion key" not in gct("nohint"), \
    gct("nohint")[-3000:]
# the ✕: ignore, the next suggestion; still ignored after a relaunch
ig_file = gc / "data/rift/ignored-suggestions.json"
gc_run("ignore", [
    "type git st", "wait 600", f"dump {gc}/ignore-typed.txt",
    "a11y click Ignore this suggestion", "wait 600", f"dump {gc}/ignore-next.txt",
], "autosuggestion_ignore_button = true\n")
assert 'Button "Ignore this suggestion"' in gct("ignore-typed"), gct("ignore-typed")[-3000:]
# their tooltips (Warp's words)
assert '"Change keybinding@' in gct("ignore-typed") and '"Ignore this suggestion@' in gct("ignore-typed"), \
    re.search(r"^tips=.*$", gct("ignore-typed"), re.M)
assert gc_ghost("ignore-typed") == "atus", gc_ghost("ignore-typed")
assert gc_ghost("ignore-next") == "atus --short", gc_ghost("ignore-next")
assert json.loads(ig_file.read_text()) == {"version": 1, "commands": ["git status"]}, \
    ig_file.read_text()
if sys.platform != "win32":
    assert ig_file.stat().st_mode & 0o777 == 0o600, oct(ig_file.stat().st_mode)
gc_run("relaunch", [
    "type git st", "wait 600", f"dump {gc}/relaunch.txt",
    # the palette's action suggests it again
    "key ctrl+u", "key ctrl+shift+p", "wait 200", "type Clear ignored suggestions", "wait 200",
    "key enter", "wait 300", "type git st", "wait 600", f"dump {gc}/cleared.txt",
], None)
assert gc_ghost("relaunch") == "atus --short", gc_ghost("relaunch")
assert gc_ghost("cleared") == "atus", gc_ghost("cleared")
assert json.loads(ig_file.read_text())["commands"] == [], ig_file.read_text()
# running an ignored line suggests it again (Warp)
gc_run("rerun", [
    "type git st", "wait 600", "a11y click Ignore this suggestion", "wait 600",
    "key ctrl+u", "type git status\\n", "wait 800",
    "type git st", "wait 600", f"dump {gc}/rerun.txt",
], None)
assert gc_ghost("rerun") == "atus", gc_ghost("rerun")
assert json.loads(ig_file.read_text())["commands"] == [], ig_file.read_text()
# a scripted run that did not opt in neither reads nor writes the file
ig_file.write_text('{"version":1,"commands":["git status"]}')
gc_env.pop("RIFT_SCRIPT_RECORD_IGNORED")
gc_run("blind", [
    "type git st", "wait 600", f"dump {gc}/blind.txt",
    "a11y click Ignore this suggestion", "wait 600",
], None)
assert gc_ghost("blind") == "atus", gc_ghost("blind")
assert json.loads(ig_file.read_text())["commands"] == ["git status"], ig_file.read_text()

# Shell widgets: ⌃R runs the shell's own binding (a fake widget, no UI)
# on the command line, which then holds its pick, nothing run; with
# `shell_widgets = "rift"`, or once the shell unbinds ⌃R, the history pane
# opens. Needs bash 4.3 (READLINE_LINE, bind -X): macOS's /bin/bash is 3.2.
sw = scratch / "widgets"
for sub in ("cfg/rift", "data", "home", "work"):
    (sw / sub).mkdir(parents=True, exist_ok=True)
sw_bash = next((b for b in ("/opt/homebrew/bin/bash", "/usr/local/bin/bash", "/bin/bash")
                if Path(b).exists() and subprocess.run(
                    [b, "-c", "((BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] >= 403))"]
                ).returncode == 0), None)
if sw_bash is None:
    print("SKIP shell widgets: no bash >= 4.3", flush=True)
else:
    fixture = ("PS1='$ '\nfake_hist() { READLINE_LINE='picked cmd'; "
               "READLINE_POINT=${#READLINE_LINE}; }\nbind -x '\"\\C-r\": fake_hist'\n")
    for rc_file in (".bash_profile", ".bashrc"):  # macOS login / Linux shim
        (sw / "home" / rc_file).write_text(fixture)
    sw_conf = lambda mode: (
        f'shell = {json.dumps(sw_bash)}\nrestore_session = false\ndurable_sessions = false\n'
        f'shell_widgets = "{mode}"\nworking_directory = {json.dumps(str(sw / "work"))}\n')
    (sw / "cfg/rift/config.toml").write_text(sw_conf("auto"))
    (sw / "auto.toml").write_text(sw_conf("auto"))
    (sw / "rift.toml").write_text(sw_conf("rift"))
    (sw / "w.rift").write_text("\n".join([
        "wait_prompt", "wait 500",
        "type echo hi", "key ctrl+r", "wait 800", f"dump {sw}/picked.txt",
        "key ctrl+u", cp(sw / "rift.toml", sw / "cfg/rift/config.toml"), "wait 2000",
        "key ctrl+r", "wait 300", f"dump {sw}/rift.txt", "key escape",
        cp(sw / "auto.toml", sw / "cfg/rift/config.toml"), "wait 2000",
        "key ctrl+u", "type bind -r '\\C-r'", "key enter", "wait 800",
        "key ctrl+r", "wait 300", f"dump {sw}/unbound.txt",
        "quit"]) + "\n")
    sw_env = dict(env, XDG_CONFIG_HOME=str(sw / "cfg"), XDG_DATA_HOME=str(sw / "data"),
                  HOME=str(sw / "home"), BASH_SILENCE_DEPRECATION_WARNING="1")
    with (sw / "w.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sw / "w.rift"), "--headless"],
                            env=sw_env, stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died("shell widgets run failed", sw / "w.log")
    picked = (sw / "picked.txt").read_text()
    assert 'editor="picked cmd" cursor=10' in picked, picked[:3000]
    assert "history=false" in picked and "    block 1 " not in picked, picked[:3000]
    rift_mode = (sw / "rift.txt").read_text()
    assert "history=true" in rift_mode, rift_mode[:1500]
    unbound = (sw / "unbound.txt").read_text()
    assert "history=true" in unbound, unbound[:1500]

# a widget still running (it waits for a key) owns the keys: ⌃⇧G opens no
# composer over it, the key it waits for reaches it
if sw_bash is not None:
    swb = scratch / "widgets-busy"
    for sub in ("cfg/rift", "data", "home", "work"):
        (swb / sub).mkdir(parents=True, exist_ok=True)
    for rc_file in (".bash_profile", ".bashrc"):
        (swb / "home" / rc_file).write_text(
            "PS1='$ '\n__slow() { printf 'PICKER> '; read -rsn1 k; READLINE_LINE=\"got-$k\"; "
            "READLINE_POINT=${#READLINE_LINE}; }\nbind -x '\"\\C-r\": __slow'\n")
    (swb / "cfg/rift/config.toml").write_text(
        f'shell = {json.dumps(sw_bash)}\nrestore_session = false\ndurable_sessions = false\n'
        f'working_directory = {json.dumps(str(swb / "work"))}\n')
    (swb / "w.rift").write_text("\n".join([
        "wait_prompt", "wait 500", "type abc", "key ctrl+r", "wait 800",
        "key ctrl+shift+g", "wait 300", f"dump {swb}/busy.txt", "type x", "wait 1000",
        f"dump {swb}/after.txt", "quit"]) + "\n")
    with (swb / "w.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(swb / "w.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(swb / "cfg"),
                                     XDG_DATA_HOME=str(swb / "data"), HOME=str(swb / "home"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died("busy shell widget run failed", swb / "w.log")
    sw_busy = (swb / "busy.txt").read_text()
    assert "composer=closed" in sw_busy, sw_busy[:3000]
    sw_after = (swb / "after.txt").read_text()
    assert 'editor="got-x" cursor=5' in sw_after, sw_after[:3000]

# zsh and fish: ⌃R and ⌃T run the shell's own (fake) widget on the line, the
# cursor where it left it; ⌥C's widget runs a command itself (zsh: zle
# accept-line; fish: `commandline -f execute`, which fish runs only after the
# whole binding), so the command runs and Rift's line keeps what it held
sw_fixtures = {
    "zsh": ("home/.zshrc",
            "PROMPT='$ '\n"
            "fake-r() { BUFFER=\"picked-$BUFFER\"; CURSOR=${#BUFFER}; }\n"
            "zle -N fake-r; bindkey '^R' fake-r\n"
            "fake-t() { LBUFFER+=FILE; }\nzle -N fake-t; bindkey '^T' fake-t\n"
            "fake-c() { BUFFER='echo ran-by-widget'; zle accept-line; }\n"
            "zle -N fake-c; bindkey '^[c' fake-c\n"),
    "fish": ("cfg/fish/config.fish",
             "bind \\cr 'commandline -r \"picked-\"(commandline); commandline -f end-of-line'\n"
             "bind \\ct 'commandline -i FILE'\n"
             "bind \\ec 'commandline -r \"echo ran-by-widget\"; commandline -f execute'\n"),
}
for sw_name, (sw_rc, sw_body) in sw_fixtures.items():
    sw_shell = shutil.which(sw_name)
    if sw_shell is None:
        print(f"SKIP shell widgets: no {sw_name}", flush=True)
        continue
    swn = scratch / f"widgets-{sw_name}"
    for sub in ("cfg/rift", "data", "home", "work"):
        (swn / sub).mkdir(parents=True, exist_ok=True)
    (swn / sw_rc).parent.mkdir(parents=True, exist_ok=True)
    (swn / sw_rc).write_text(sw_body)
    (swn / "cfg/rift/config.toml").write_text(
        f'shell = {json.dumps(sw_shell)}\nrestore_session = false\ndurable_sessions = false\n'
        f'option_as_meta = true\nworking_directory = {json.dumps(str(swn / "work"))}\n')
    (swn / "w.rift").write_text("\n".join([
        "wait_prompt", "wait_widget r",
        "type abc", "key ctrl+r", "wait 1000", f"dump {swn}/r.txt",
        "key ctrl+u", "type a b", "key left", "key left", "key ctrl+t", "wait 1000",
        f"dump {swn}/t.txt",
        "key ctrl+u", "type keep", "key alt+c", "wait 1500", f"dump {swn}/c.txt",
        "quit"]) + "\n")
    with (swn / "w.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(swn / "w.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(swn / "cfg"),
                                     XDG_DATA_HOME=str(swn / "data"), HOME=str(swn / "home")),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"{sw_name} shell widgets run failed", swn / "w.log")
    sw_r = (swn / "r.txt").read_text()
    assert 'editor="picked-abc" cursor=10' in sw_r and "    block 1 " not in sw_r, \
        (sw_name, sw_r[:3000])
    sw_t = (swn / "t.txt").read_text()
    assert 'editor="aFILE b" cursor=5' in sw_t, (sw_name, sw_t[:3000])
    sw_c = (swn / "c.txt").read_text()
    assert 'cmd="echo ran-by-widget" exit=Some(0)' in sw_c, (sw_name, sw_c[:3000])
    assert 'editor="keep" cursor=4' in sw_c, (sw_name, sw_c[:3000])

# The user's own iTerm2 integration next to ours: its PS1 carries 133;A and
# 133;B, so accepting a line redraws the prompt on its row before the echo's
# line break. The command keeps its block and its output (no grey ghost line
# of the command above an empty block)
it_zsh = shutil.which("zsh")
if it_zsh is None:
    print("SKIP iTerm2 markers: no zsh", flush=True)
else:
    it = scratch / "iterm2-markers"
    for sub in ("cfg/rift", "data", "home", "work"):
        (it / sub).mkdir(parents=True, exist_ok=True)
    (it / "home/.zshrc").write_text(
        "PS1=$'%{\\e]133;A\\a%}$ %{\\e]133;B\\a%}'\n"
        "it_precmd() { printf '\\e]133;D;%s\\a' $?; }\n"
        "it_preexec() { printf '\\e]133;C;\\a'; }\n"
        "precmd_functions+=(it_precmd); preexec_functions+=(it_preexec)\n")
    (it / "cfg/rift/config.toml").write_text(
        f'shell = {json.dumps(it_zsh)}\nrestore_session = false\ndurable_sessions = false\n'
        f'working_directory = {json.dumps(str(it / "work"))}\n')
    (it / "m.rift").write_text("\n".join([
        "wait_prompt", "wait 500",
        "type echo one-out\\n", "wait 500", "wait_prompt",
        "type echo two-out\\n", "wait 500", "wait_prompt",
        f"dump {it}/m.txt", "quit"]) + "\n")
    with (it / "m.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(it / "m.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(it / "cfg"),
                                     XDG_DATA_HOME=str(it / "data"), HOME=str(it / "home")),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died("iTerm2 markers run failed", it / "m.log")
    it_dump = (it / "m.txt").read_text()
    it_blocks = [l for l in it_dump.splitlines() if l.startswith("    block ")]
    assert not any("background=true" in l for l in it_blocks), it_dump[:4000]
    for cmd in ("echo one-out", "echo two-out"):
        row = next((l for l in it_blocks if f'cmd="{cmd}"' in l), "")
        m = re.search(r"prompt_start=(\d+) output_start=Some\((\d+)\) end=Some\((\d+)\)", row)
        # its output row is its own: after the prompt row, before the end
        assert m and int(m[1]) < int(m[2]) < int(m[3]), (cmd, it_dump[:4000])

# Settings › Terminal: an OSC 52 read is answered only with "read_write";
# middle-click pastes the clipboard (an OSC 52 copy) into the pane
tm = scratch / "termpage"
for sub in ("cfg/rift", "data", "work"):
    (tm / sub).mkdir(parents=True, exist_ok=True)

def term_run(name, access, steps):
    (tm / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        f'sync_os_theme = false\nclipboard_access = "{access}"\n'
        + f'working_directory = {json.dumps(str(tm / "work"))}\n')
    (tm / f"{name}.rift").write_text("\n".join(["resize 1000 700", "wait_prompt", *steps,
                                                  "quit"]) + "\n")
    with (tm / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tm / f"{name}.rift"), "--headless"],
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
for sub in ("cfg/rift/themes", "data"):
    (sw / sub).mkdir(parents=True, exist_ok=True)
(sw / "cfg/rift/themes/broken.toml").write_text('bg = "not a color"\n')
(sw / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\nvim_mode = true\ntheme = "broken"\n'
    + f'working_directory = {json.dumps(str(work))}\n')
(sw / "sweep.rift").write_text("\n".join([
    "resize 1180 760", "wait_prompt", f"dump {sw}/first.txt",
    "key cmd+t", "wait_prompt", f"dump {sw}/second.txt",
    # the theme list runs below the fold: scroll to the user's themes
    "key cmd+,", "wait 300", "key end", "wait 300", "move 700 400", "scroll -20",
    "wait 300", f"dump {sw}/themes.txt",
    "key home", "key down", "key down", "key down", "wait 300",
    # (the vim register, shell widgets and Tab key segments come first: 2
    # stops each; then the accept-key hint and ignore button toggles)
    *["key tab"] * 31, "wait 300", f"dump {sw}/tabbed.txt",
    "quit"]) + "\n")
with (sw / "sweep.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "sweep.rift"), "--headless"],
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

# vim visual mode (same vim_mode config): `v` selects on the editor's own
# selection, a motion extends it inclusively, `d` deletes it, `.` repeats it
# and one `u` undoes the repeat
(sw / "vim.rift").write_text("\n".join([
    "resize 1180 760", "wait_prompt", "type echo hello world", "key escape",
    "type 0wv", "wait 100", f"dump {sw}/vim-v.txt",
    "type e", "wait 100", f"dump {sw}/vim-ve.txt", f"shot {sw}/vim-ve.png",
    "type d", "wait 100", f"dump {sw}/vim-d.txt",
    "type V", "wait 100", f"dump {sw}/vim-line.txt",
    "key escape", "type 0.", "wait 100", f"dump {sw}/vim-dot.txt",
    "type u", "wait 100", f"dump {sw}/vim-undo.txt",
    "quit"]) + "\n")
with (sw / "vim.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "vim.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("vim run failed", sw / "vim.log")
vim_v, vim_ve = (sw / "vim-v.txt").read_text(), (sw / "vim-ve.txt").read_text()
assert 'editor="echo hello world" cursor=5' in vim_v and "vim=Some(Visual)" in vim_v, vim_v[:1500]
assert 'editor_selection=Some("h")' in vim_v, vim_v[:1500]
assert "vim=Some(Visual)" in vim_ve and 'editor_selection=Some("hello")' in vim_ve, vim_ve[:1500]
vim_d = (sw / "vim-d.txt").read_text()
assert 'editor="echo  world"' in vim_d and "vim=Some(Normal)" in vim_d, vim_d[:1500]
assert "editor_selection=None" in vim_d, vim_d[:1500]
vim_line = (sw / "vim-line.txt").read_text()
assert "vim=Some(VisualLine)" in vim_line, vim_line[:1500]
assert 'editor_selection=Some("echo  world")' in vim_line, vim_line[:1500]
vim_dot = (sw / "vim-dot.txt").read_text()
assert 'editor="  world"' in vim_dot and "vim=Some(Normal)" in vim_dot, vim_dot[:1500]
vim_undo = (sw / "vim-undo.txt").read_text()
assert 'editor="echo  world" cursor=0' in vim_undo, vim_undo[:1500]

# vim Replace/Normal keys open no completion menu (it would take the Esc
# meant for the mode): one Esc leaves Replace
(sw / "vim-r.rift").write_text("\n".join([
    "resize 1180 760", "wait_prompt", "type x", "key escape",
    "type 0Rgi", "wait 400", f"dump {sw}/vim-r.txt",
    "key escape", "wait 100", f"dump {sw}/vim-r-esc.txt",
    "quit"]) + "\n")
with (sw / "vim-r.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "vim-r.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("vim replace run failed", sw / "vim-r.log")
vim_r = (sw / "vim-r.txt").read_text()
assert 'editor="gi"' in vim_r and "vim=Some(Replace)" in vim_r, vim_r[:1500]
assert "completion=false" in vim_r, vim_r[:1500]
vim_r_esc = (sw / "vim-r-esc.txt").read_text()
assert "vim=Some(Normal)" in vim_r_esc, vim_r_esc[:1500]

# vim registers: "+ yanks to the (private headless) clipboard and "+P puts
# it back, "a holds its own text past later yanks, ⌃R redoes what u undid
(sw / "vim-reg.rift").write_text("\n".join([
    "resize 1180 760", "wait_prompt", "type echo hello world", "key escape",
    'type 0"+yiw', "wait 100", f"dump {sw}/vreg-yank.txt",
    'type w"ayiwwyiw$"ap', "wait 100", f"dump {sw}/vreg-put.txt",
    "type u", "wait 100", f"dump {sw}/vreg-undo.txt",
    "key ctrl+r", "wait 100", f"dump {sw}/vreg-redo.txt",
    'type 0"+P', "wait 100", f"dump {sw}/vreg-clip.txt",
    "quit"]) + "\n")
with (sw / "vim-reg.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "vim-reg.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("vim register run failed", sw / "vim-reg.log")
def vreg(name):
    text = (sw / f"vreg-{name}.txt").read_text()
    clip = next(l for l in text.splitlines() if l.startswith("clipboard="))
    return text, json.loads(clip[len("clipboard="):])
text, clip = vreg("yank")
assert clip == "echo" and 'editor="echo hello world"' in text, (clip, text[:1500])
text, _ = vreg("put")
assert 'editor="echo hello worldhello"' in text, text[:1500]
text, _ = vreg("undo")
assert 'editor="echo hello world"' in text, text[:1500]
text, _ = vreg("redo")
assert 'editor="echo hello worldhello"' in text, text[:1500]
text, clip = vreg("clip")
assert clip == "echo" and 'editor="echoecho hello worldhello"' in text, text[:1500]

# command corrections from a failed command's output (bash's messages): a
# mistyped cd, a script without its execute bit, a permission denial, git's
# own suggestion; → accepts one into the editor, never runs it
(sw / "corr/projects").mkdir(parents=True, exist_ok=True)
(sw / "corr/run.sh").write_text("#!/bin/sh\necho ran\n")
(sw / "corr/run.sh").chmod(0o644)
(sw / "corr.rift").write_text("\n".join([
    "resize 1180 760", "wait_prompt",
    f"type cd {sw / 'corr'}\\n", "wait 600",
    "type cd projcts\\n", "wait 1000", f"dump {sw}/corr-cd.txt",
    "key right", "wait 200", f"dump {sw}/corr-cd-accept.txt",
    "key cmd+a", "key backspace",
    "type ./run.sh\\n", "wait 1000", f"dump {sw}/corr-chmod.txt",
    "type mkdir /etc/rift-smoke-denied\\n", "wait 1000", f"dump {sw}/corr-sudo.txt",
    "type git stauts\\n", "wait 1500", f"dump {sw}/corr-git.txt",
    "quit"]) + "\n")
with (sw / "corr.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "corr.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("corrections run failed", sw / "corr.log")
corr = {n: (sw / f"corr-{n}.txt").read_text() for n in ("cd", "cd-accept", "chmod", "sudo", "git")}
assert 'correction=Some("cd projects")' in corr["cd"], corr["cd"][:2000]
assert 'editor="cd projects"' in corr["cd-accept"], corr["cd-accept"][:2000]
assert 'correction=Some("chmod +x ./run.sh && ./run.sh")' in corr["chmod"], corr["chmod"][:2000]
assert not os.access(sw / "corr/run.sh", os.X_OK), "a correction ran"
assert 'correction=Some("sudo mkdir /etc/rift-smoke-denied")' in corr["sudo"], corr["sudo"][:2000]
assert 'correction=Some("git status")' in corr["git"], corr["git"][:2000]

# an agent idle at its own prompt (Claude Code's "✳" title) does not spin;
# one working (a spinner frame in its title) does
def agent_run(name, title_octal, command=None):
    typed = command or f"printf '\\\\033]0;{title_octal} Claude Code\\\\007'; sleep 3"
    (sw / f"{name}.rift").write_text("\n".join([
        "resize 1000 500", "wait_prompt",
        f"type {typed}\\n", "wait 800",
        f"dump {sw}/{name}.txt", "quit"]) + "\n")
    with (sw / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sw / f"{name}.rift"), "--headless"],
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
# the tab's agent state agrees (it kept spinning on "working")
assert "agent=Codex agent_state=done" in codex_idle, codex_idle[:2000]
codex_busy = agent_run("codex-busy", "", f"{fake_codex} '\\\\342\\\\240\\\\213 shift'")
assert 'title="⠋ shift"' in codex_busy and "running=true" in codex_busy, codex_busy[:2000]
assert "agent=Codex agent_state=working" in codex_busy, codex_busy[:2000]
# no standard says a program is busy: OpenCode (its title never says it
# works) and a full-screen program spin only on a working signal (a working
# title or OSC 9;4 progress); a plain command spins
fake_opencode = sw / "agent-bin" / "opencode"
fake_opencode.write_text("#!/bin/sh\nprintf \"\\033]0;OC | Project\\007\"\nsleep 3\n")
fake_opencode.chmod(0o755)
oc_idle = agent_run("opencode-idle", "", str(fake_opencode))
assert ("running=false" in oc_idle and "agent=OpenCode agent_state=done" in oc_idle), oc_idle[:2000]
full = agent_run("fullscreen-idle", "", "printf '\\\\033[?1049h'; sleep 3; printf '\\\\033[?1049l'")
assert "alt=true" in full and "running=false" in full, full[:2000]
full_busy = agent_run("fullscreen-busy", "",
                      "printf '\\\\033[?1049h\\\\033]9;4;3\\\\007'; sleep 3; printf '\\\\033[?1049l'")
assert "alt=true" in full_busy and "running=true" in full_busy, full_busy[:2000]
plain = agent_run("plain-busy", "", "sleep 3")
assert "running=true" in plain, plain[:2000]

# agent tabs: a (fake) `claude` on PATH works, then sends Claude Code's
# permission notification (OSC 777) and waits; it finishes in a background
# tab (done until viewed; viewing clears it), while a plain `sleep` keeps
# the spinner. Linked worktrees of one repository show their own name, a
# branch switch keeps the identity; "New worktree tab…" adds one and opens a
# tab there, an existing branch toasts git's message and creates nothing.
at = sw / "agenttabs"
for sub in ("cfg/rift", "data", "bin", "work"):
    (at / sub).mkdir(parents=True, exist_ok=True)
(at / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\ntab_layout = "vertical"\n'
    + f'working_directory = {json.dumps(str(at / "work"))}\n')
fake_claude = at / "bin" / "claude"
fake_claude.write_text(
    # Claude Code's working title (◐): an agent works only while it says so
    "#!/bin/sh\nprintf '\\033]0;\\342\\227\\220 work\\007'\necho working\nsleep 2\n"
    "printf '\\033]777;notify;Claude Code;Claude needs your permission to use Bash\\007'\n"
    "sleep \"$2\"\nexit \"$1\"\n")
fake_claude.chmod(0o755)
wt = at / "work" / "repo.worktrees"
(at / "t.rift").write_text("\n".join([
    "resize 1100 560", "focus on", "wait_prompt",
    "type claude 0 3\\n", "wait 1000", f"dump {at}/working.txt",
    "wait 2200", f"dump {at}/waiting.txt",
    "key cmd+t", "wait_prompt", "type sleep 6\\n", "wait 3000", f"dump {at}/done.txt",
    "key cmd+1", "wait 400", f"dump {at}/seen.txt",
    "type mkdir repo && cd repo && git init -q -b main && "
    "git -c user.name=t -c user.email=t@x commit -q --allow-empty -m init && "
    "git worktree add -q -b wa ../repo.worktrees/wa && "
    "git worktree add -q -b wb ../repo.worktrees/wb\\n", "wait 2500",
    "key cmd+t", "wait_prompt", f"type cd {wt}/wa\\n", "wait 1500",
    "key cmd+t", "wait_prompt", f"type cd {wt}/wb\\n", "wait 1500",
    f"dump {at}/worktrees.txt",
    "type git switch -q -c wb2\\n", "wait 1500", f"dump {at}/switched.txt",
    "key cmd+1", "wait 400", "key cmd+p", "type new worktree", "wait 300", "key enter",
    "wait 400", f"dump {at}/ask.txt", "key enter", "wait 2500", f"dump {at}/created.txt",
    "key cmd+1", "wait 400", "key cmd+p", "type new worktree", "wait 300", "key enter",
    # the prefill `wt-2` is selected: typing replaces it
    "wait 400", "type wa", "key enter", "wait 2500",
    f"dump {at}/exists.txt", "quit"]) + "\n")
with (at / "t.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(at / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(at / "cfg"),
                                 XDG_DATA_HOME=str(at / "data"),
                                 PATH=f"{at / 'bin'}:{env['PATH']}",
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("agent tabs run failed", at / "t.log")
def at_tabs(name):
    return re.findall(r"^tab \d+ .*$", (at / f"{name}.txt").read_text(), re.M)
def at_tab(name, i):
    return at_tabs(name)[i]
assert "agent=Claude_Code agent_state=working" in at_tab("working", 0), at_tabs("working")
assert "agent=Claude_Code agent_state=waiting" in at_tab("waiting", 0), at_tabs("waiting")
assert "agent=Claude_Code agent_state=done" in at_tab("done", 0), at_tabs("done")
assert "running=true" in at_tab("done", 1) and "agent=- agent_state=-" in at_tab("done", 1), \
    at_tabs("done")
assert "agent=- agent_state=-" in at_tab("seen", 0), at_tabs("seen")
# tabs: 0 the main checkout, 1 the sleep, 2 and 3 the linked worktrees
worktrees = at_tabs("worktrees")
assert worktrees[0].endswith("worktree=-") and worktrees[2].endswith("worktree=wa") \
    and worktrees[3].endswith("worktree=wb"), worktrees
switched = (at / "switched.txt").read_text()
assert at_tab("switched", 3).endswith("worktree=wb") and 'git=Some("wb2")' in switched, switched[:3000]
assert re.search(r"active_pane=(\d+)", worktrees[3])[1] == \
    re.search(r"active_pane=(\d+)", at_tab("switched", 3))[1], "the pane survived the switch"
assert 'mode=NewWorktree query="wt-1"' in (at / "ask.txt").read_text()
created = at_tabs("created")
assert len(created) == 5 and created[4].endswith("worktree=wt-1"), created
assert (wt / "wt-1").is_dir()
exists = (at / "exists.txt").read_text()
assert "a branch named 'wa' already exists" in exists and len(at_tabs("exists")) == 5, exists[:3000]

# ⌘⌫ in a program that owns the keys (Claude Code) sends ctrl+u, as in
# Warp: a raw-mode reader gets 0x15
(sw / "cmdbs.sh").write_text(
    "stty raw -echo\nb=$(dd bs=1 count=1 2>/dev/null | od -An -tx1)\n"
    "stty sane\necho \"KEY[$(echo $b)]\"\n")
(sw / "cmdbs.rift").write_text("\n".join([
    "wait_prompt", f"type sh {sw}/cmdbs.sh\\n", "wait 800", "key cmd+backspace",
    "wait 800", f"dump {sw}/cmdbs.txt", "quit"]) + "\n")
with (sw / "cmdbs.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "cmdbs.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("cmd+backspace run failed", sw / "cmdbs.log")
cmdbs = (sw / "cmdbs.txt").read_text()
assert "KEY[15]" in cmdbs, cmdbs[:2000]

# rich input: ⌃⇧G opens the composer over a program that reads raw keys and
# enabled bracketed paste; ⇧⏎ breaks the line, ⏎ sends the text once as one
# bracketed paste and CR. Esc keeps the draft, reopening restores it, an
# empty ⏎ sends nothing; at a shell prompt the composer never opens.
ri = scratch / "rich-input"
for sub in ("cfg/rift", "data"):
    (ri / sub).mkdir(parents=True, exist_ok=True)
(ri / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(work))}\n')
(ri / "agent.py").write_text(
    "import os, select, sys, termios, tty\n"
    "out, stop = sys.argv[1], sys.argv[2]\n"
    "fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); tty.setraw(fd)\n"
    "open(out, 'wb').close()\n"
    "sys.stdout.write('\\033[?2004hagent ready\\r\\n> '); sys.stdout.flush()\n"
    "while not os.path.exists(stop):\n"
    "    if select.select([fd], [], [], 0.1)[0]:\n"
    "        data = os.read(fd, 4096)\n"
    "        with open(out, 'ab') as f: f.write(data)\n"
    "sys.stdout.write('\\033[?2004l'); termios.tcsetattr(fd, termios.TCSADRAIN, old)\n")
(ri / "ri.rift").write_text("\n".join([
    "resize 1000 600", "wait_prompt",
    f"type python3 {ri}/agent.py {ri}/got.bin {ri}/stop\\n", "wait 1500",
    "key ctrl+shift+g", "wait 200", "type line1", "key shift+enter", "type line2",
    "wait 200", f"dump {ri}/open.txt", "key enter", "wait 700", f"dump {ri}/sent.txt",
    "key ctrl+shift+g", "wait 200", "type kept draft", "key escape", "wait 200",
    f"dump {ri}/closed.txt", "key ctrl+shift+g", "wait 200", f"dump {ri}/reopened.txt",
    "key cmd+a", "key backspace", "key enter", "wait 500", f"dump {ri}/empty.txt",
    "key escape", f"sh touch {ri}/stop", "wait 1000",
    "key ctrl+shift+g", "wait 200", f"dump {ri}/prompt.txt", "quit"]) + "\n")
with (ri / "ri.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(ri / "ri.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(ri / "cfg"),
                                 XDG_DATA_HOME=str(ri / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("rich input run failed", ri / "ri.log")
ri_open = (ri / "open.txt").read_text()
assert "composer=open draft_len=11" in ri_open, ri_open[:2000]
assert 'TextInput "Compose for python3' in ri_open, ri_open[:4000]
ri_sent = (ri / "sent.txt").read_text()
assert "composer=closed draft_len=0" in ri_sent, ri_sent[:2000]
ri_closed = (ri / "closed.txt").read_text()
assert "composer=closed draft_len=10" in ri_closed, ri_closed[:2000]
ri_reopened = (ri / "reopened.txt").read_text()
assert "composer=open draft_len=10" in ri_reopened and 'editor="kept draft"' in ri_reopened, \
    ri_reopened[:2000]
ri_empty = (ri / "empty.txt").read_text()
assert "composer=open draft_len=0" in ri_empty, ri_empty[:2000]
# exactly one send: the bracketed paste and its Enter, nothing else
got = (ri / "got.bin").read_bytes()
assert got == b"\x1b[200~line1\nline2\x1b[201~\r", got
ri_prompt = (ri / "prompt.txt").read_text()
assert "composer=closed" in ri_prompt and "editor_active=true" in ri_prompt, ri_prompt[:2000]

# rich input over a shell that binds ⌃R to a widget of its own: ⌃R in the
# composer is not the shell's (no hand-off reaches the program), and the
# draft still sends as one paste
if sw_bash is None:
    print("SKIP rich input ⌃R: no bash >= 4.3", flush=True)
else:
    (ri / "cfg/rift/config.toml").write_text(
        f'shell = {json.dumps(sw_bash)}\nrestore_session = false\ndurable_sessions = false\n'
        f'shell_widgets = "auto"\nworking_directory = {json.dumps(str(work))}\n')
    (ri / "ctrlr.rift").write_text("\n".join([
        "wait_prompt", "wait 500",
        f"type python3 {ri}/agent.py {ri}/got-r.bin {ri}/stop-r\\n", "wait 1500",
        "key ctrl+shift+g", "wait 200", "type hello", "key ctrl+r", "wait 800",
        "type zz", f"dump {ri}/ctrlr.txt", "key enter", "wait 700",
        f"sh touch {ri}/stop-r", "wait 800", "quit"]) + "\n")
    with (ri / "ctrlr.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(ri / "ctrlr.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(ri / "cfg"),
                                     XDG_DATA_HOME=str(ri / "data"), HOME=str(sw / "home"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died("rich input ⌃R run failed", ri / "ctrlr.log")
    ri_r = (ri / "ctrlr.txt").read_text()
    assert "composer=open draft_len=7" in ri_r and 'editor="hellozz"' in ri_r, ri_r[:2000]
    got = (ri / "got-r.bin").read_bytes()
    assert got == b"\x1b[200~hellozz\x1b[201~\r", got

# rich_input_auto: a coding agent quiet at its prompt gets the composer; after
# Esc, the echo of what the user types into the agent never reopens it (the
# keys keep reaching the agent); the agent asking again (its notification)
# does
(ri / "bin").mkdir(exist_ok=True)
(ri / "bin/claude").write_text(
    "#!/usr/bin/env python3\n"
    "import os, select, sys, termios, tty\n"
    f"out, stop = {str(ri / 'auto.bin')!r}, {str(ri / 'stop-a')!r}\n"
    "fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); tty.setraw(fd)\n"
    "open(out, 'wb').close(); got = b''\n"
    "sys.stdout.write('agent ready\\r\\n> '); sys.stdout.flush()\n"
    "while not os.path.exists(stop):\n"
    "    if select.select([fd], [], [], 0.1)[0]:\n"
    "        data = os.read(fd, 4096); got += data\n"
    "        with open(out, 'ab') as f: f.write(data)\n"
    "        sys.stdout.write(data.decode(errors='replace'))\n"
    "        if got.endswith(b'go'):\n"
    "            sys.stdout.write('\\033]9;Claude needs your permission to use Bash\\a')\n"
    "        sys.stdout.flush()\n"
    "termios.tcsetattr(fd, termios.TCSADRAIN, old)\n")
(ri / "bin/claude").chmod(0o755)
(ri / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    f'rich_input_auto = true\nworking_directory = {json.dumps(str(work))}\n')
(ri / "auto.rift").write_text("\n".join([
    "wait_prompt", "wait 500", f"type {ri}/bin/claude\\n", "wait 2500",
    f"dump {ri}/auto-open.txt", "key escape", "wait 200", "type ab", "wait 1800",
    f"dump {ri}/auto-typed.txt", "type cd", "wait 300", "type go", "wait 1500",
    f"dump {ri}/auto-rearmed.txt", "key escape", f"sh touch {ri}/stop-a", "wait 800",
    "quit"]) + "\n")
with (ri / "auto.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(ri / "auto.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(ri / "cfg"),
                                 XDG_DATA_HOME=str(ri / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("rich input auto-open run failed", ri / "auto.log")
ri_auto = (ri / "auto-open.txt").read_text()
assert "composer=open" in ri_auto, ri_auto[:2000]
ri_typed = (ri / "auto-typed.txt").read_text()
assert "composer=closed" in ri_typed, ri_typed[:2000]
ri_rearmed = (ri / "auto-rearmed.txt").read_text()
assert "composer=open draft_len=0" in ri_rearmed, ri_rearmed[:2000]
got = (ri / "auto.bin").read_bytes()
assert got == b"abcdgo", got

# the font picker lists the embedded Hack after the default and no
# bitmap-only faces; a typed space reaches the settings search
(sw / "fonts.rift").write_text("\n".join([
    "wait_prompt", "key cmd+,", "wait 300", "type font family", "wait 200",
    "key tab", "wait 100", "key enter", "wait 300", f"dump {sw}/fonts.txt", "quit"]) + "\n")
with (sw / "fonts.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sw / "fonts.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sw / "cfg"),
                                 XDG_DATA_HOME=str(sw / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("font picker run failed", sw / "fonts.log")
fonts = (sw / "fonts.txt").read_text()
assert 'settings_query="font family"' in fonts, fonts[:2000]
font_opts = re.findall(r'ListBoxOption "([^"]+)"', fonts)
assert font_opts[:1] == ["JetBrains Mono"] and "Hack" in font_opts, font_opts
assert not any("Bitmap" in f for f in font_opts), font_opts

# Updates: a file:// manifest (RIFT_UPDATE_URL) advertising a newer
# version shows the tab bar's chip; a click opens the update dialog (a
# development build cannot update itself: "Open download page"); Later
# dismisses that version, also after a relaunch. The running version or a
# malformed manifest shows no chip; About says why. Every manifest needs
# latest.json.sig, an Ed25519 signature: these are signed with a throwaway
# test key (RIFT_UPDATE_TEST_KEY, honoured for file:// manifests only); an
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
(up / "cfg/rift").mkdir(parents=True, exist_ok=True)
(up / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
    + f'working_directory = {json.dumps(str(work))}\n')
manifest = {"version": "99.0.0", "date": "2026-10-12", "protocol": 1, "notes": "Newer.",
            "url": "https://github.com/botjaeger/rift-releases/releases/tag/v99.0.0",
            "assets": {}}
newer = json.dumps(manifest).encode()
write_signed(up / "newer.json", newer)
write_signed(up / "same.json", json.dumps(dict(manifest, version=current)).encode())
write_signed(up / "bad.json", b"{not json")
(up / "unsigned.json").write_bytes(newer)
# signed, then changed (98 -> 99): the signature no longer matches
write_signed(up / "forged.json", newer, newer.replace(b"99.0.0", b"98.0.0"))
about_steps = ["key cmd+p", "type About Rift", "wait 200", "key enter", "wait 400"]

def up_run(name, data, url, steps):
    (up / f"{name}.rift").write_text(
        "\n".join(["resize 1000 700", "wait_prompt", "wait 800", *steps, "quit"]) + "\n")
    with (up / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(up / f"{name}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(up / "cfg"),
                                     XDG_DATA_HOME=str(up / data),
                                     RIFT_UPDATE_URL=f"file://{up / url}",
                                     RIFT_UPDATE_TEST_KEY=up_key,
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"update {name} run failed", up / f"{name}.log")

def up_state(name):
    return (up / f"{name}.txt").read_text()

up_run("probe", "data-new", "newer.json", [f"dump {up}/probe.txt"])
probe = up_state("probe")
assert 'update=available:99.0.0 chip=Some("99.0.0")' in probe, probe[:3000]
# offered as a notice, never a toast (a development build has nothing to
# download, so it is offered at once)
assert ('update_ready=Some("99.0.0") update_fetching=None '
        'app_notes=["Rift v99.0.0 is ready · Click to see how to update"]') in probe, probe[:3000]
assert "toast=None" in probe, probe[:3000]
assert 'Button "Update Rift, v99.0.0 available"' in probe, probe[-3000:]
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
assert 'AlertDialog "Update to Rift v99.0.0?"' in dialog, dialog[-3000:]
assert 'Button "Later"' in dialog and 'Button "Open download page"' in dialog, dialog[-3000:]
later = up_state("later")
assert 'confirm=None' in later and 'chip=None dismissed=Some("99.0.0")' in later, later[:3000]
about_new = up_state("about-new")
assert 'Button "Update to v99.0.0 available"' in about_new, about_new[-3000:]
# dismissed stays dismissed across a relaunch (update.json)
up_run("relaunch", "data-new", "newer.json", [f"dump {up}/relaunch.txt"])
assert 'update=available:99.0.0 chip=None dismissed=Some("99.0.0")' in up_state("relaunch")
assert "app_notes=[]" in up_state("relaunch"), "a dismissed version notified again"
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

# a check asked for answers with a notice (the palette's Check for updates)
up_run("same-check", "data-same-check", "same.json", [
    "key cmd+p", "type Check for updates", "wait 200", "key enter", "wait 1500",
    f"dump {up}/same-check.txt"])
same_check = up_state("same-check")
assert 'app_notes=["Rift is up to date · v' in same_check and "toast=None" in same_check, \
    same_check[:3000]

# An update's relaunch that is quit before its start is confirmed
# (CONFIRM_AFTER) keeps the new version: the real relaunch helper (HELPER in
# update.rs) starts this binary as the version it installed, which quits 2 s
# in; it must not roll back to the old copy nor report "Couldn't update"
uq = scratch / "update-quit"
(uq / "cfg/rift").mkdir(parents=True)
(uq / "data/rift/updates").mkdir(parents=True)
(uq / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n')
uq_helper = re.search(r'pub const HELPER: &str = r#"(.*?)"#;',
                      (Path(__file__).resolve().parents[1] / "crates/rift-core/src/update.rs").read_text(), re.S)[1]
for side in ("app", "app.old"):
    (uq / side).mkdir()
    (uq / side / "which").write_text(side)
uq_version = subprocess.run([str(binary), "--version"], capture_output=True,
                            text=True, check=True).stdout.split()[-1]
uq_marker = uq / "data/rift/update-pending.json"
uq_failed = uq / "data/rift/update-failed.json"
uq_marker.write_text(json.dumps({"from": "0.0.1", "to": uq_version,
                                 "old_path": str(uq / "app.old"), "new_path": str(uq / "app")}))
(uq / "t.rift").write_text("wait_prompt\nwait 2000\nquit\n")
uq_gone = subprocess.Popen(["true"])
uq_gone.wait()
rc = subprocess.run(
    ["/bin/sh", "-c", uq_helper, "sh", str(uq_gone.pid), str(uq / "app"), str(uq / "app.old"),
     str(binary), str(uq_marker), str(uq_failed), str(uq / "relaunch.log"),
     "--script", str(uq / "t.rift"), "--headless"],
    env=dict(env, XDG_CONFIG_HOME=str(uq / "cfg"), XDG_DATA_HOME=str(uq / "data"),
             RIFT_UPDATE_RELAUNCH="1", BASH_SILENCE_DEPRECATION_WARNING="1"),
    timeout=90).returncode
uq_log = (uq / "relaunch.log").read_text()
assert rc == 0, uq_log[-3000:]
assert (uq / "app/which").read_text() == "app", "a quit before confirming rolled back: " + uq_log[-3000:]
assert not uq_marker.exists() and not uq_failed.exists(), uq_log[-3000:]
assert "quit before the start was confirmed" in uq_log and "Couldn't update" not in uq_log, \
    uq_log[-3000:]

# Settings › Remote access › Recent connections shows five rows; the wheel
# over it scrolls a row at a time and past its end the page scrolls on.
# leaving the Settings tab for a shell tab and coming back (⌘, twice) keeps
# its page; closing it (⌘W) and opening it again starts on General
rh = scratch / "recent"
(rh / "cfg/rift").mkdir(parents=True)
# Recent connections is folded away at the end of the page while remote
# access is on (the gateway this starts stops again before the quit)
recent_base = ('shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
               'durable_sessions = false\nremote_listen = "127.0.0.1:0"\ncontrol_api = "off"\n')
(rh / "cfg/rift/config.toml").write_text(recent_base + "remote_access = true\n")
(rh / "off.toml").write_text(recent_base)
(rh / "data/rift/remote").mkdir(parents=True, mode=0o700)
(rh / "data/rift/remote/history.jsonl").write_text("".join(
    json.dumps({"device": f"d{i}", "name": f"Phone {i}", "via": "lan",
                "address": f"10.0.0.{i}:5000", "start": f"2026-10-16T10:0{i}:00Z",
                "end": f"2026-10-16T10:0{i}:30Z", "reason": "closed"}) + "\n"
    for i in range(1, 9)))
(rh / "t.rift").write_text("\n".join([
    "resize 1100 700", "wait_prompt",
    f"wait_file {rh}/data/rift/remote/status.json 20000",
    "key cmd+,", "wait 300", "key end", "key down", "wait 1500",
    f"dump {rh}/folded.txt",
    "move 550 300", "scroll -40", "wait 300", "a11y click Recent connections (8), collapsed",
    "wait 300", "scroll -40", "wait 300", f"dump {rh}/top.txt",
    "move 600 550", *["scroll -1", "wait 100"] * 2, f"dump {rh}/two.txt",
    *["scroll -1", "wait 100"] * 2, f"dump {rh}/end.txt",
    "key cmd+,", "wait 200", "key cmd+,", "wait 300", f"dump {rh}/back.txt",
    "key cmd+w", "wait 200", "key cmd+,", "wait 300", f"dump {rh}/reopen.txt",
    f"sh cp {shlex.quote(str(rh / 'off.toml'))} {shlex.quote(str(rh / 'cfg/rift/config.toml'))}",
    "wait 3500", "quit"]) + "\n")
with (rh / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(rh / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(rh / "cfg"),
                                 XDG_DATA_HOME=str(rh / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("recent connections run failed", rh / "log.txt")

def shown_phones(name):
    return re.findall(r'Label "Phone (\d), ', (rh / f"{name}.txt").read_text())

assert shown_phones("folded") == [], shown_phones("folded")
assert shown_phones("top") == ["8", "7", "6", "5", "4"], shown_phones("top")
assert shown_phones("two") == ["6", "5", "4", "3", "2"], shown_phones("two")
# three rows down is the end: the fourth notch scrolled the page instead
assert shown_phones("end") == ["5", "4", "3", "2", "1"], shown_phones("end")
assert "settings=true page=Remote" in (rh / "back.txt").read_text(), \
    (rh / "back.txt").read_text()[:1500]
assert "settings=true page=General" in (rh / "reopen.txt").read_text(), \
    (rh / "reopen.txt").read_text()[:1500]

# Stall log: a main thread blocked over 2 s (here the script's `sh`) leaves
# a report in <data>/stalls/ with how long it lasted and, on macOS, the
# stacks that name the blocking call; a run that never blocks leaves none
assert not (rh / "data/rift/stalls").exists(), list((rh / "data/rift/stalls").iterdir())
sl = scratch / "stall"
(sl / "cfg/rift").mkdir(parents=True)
(sl / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n')
(sl / "t.rift").write_text("wait_prompt\nsh sleep 5\nwait 6000\nquit\n")
with (sl / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sl / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sl / "cfg"),
                                 XDG_DATA_HOME=str(sl / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("stall run failed", sl / "log.txt")
stalls = list((sl / "data/rift/stalls").glob("stall-*.txt"))
assert len(stalls) == 1, stalls
stall = stalls[0].read_text()
assert "the main thread is blocked: one pass of the event loop" in stall, stall[:500]
lasted = int(re.search(r"^lasted about (\d+) ms$", stall, re.M)[1])
assert 4500 <= lasted <= 7000, lasted
if sys.platform == "darwin":
    assert "drive_script" in stall and "wait4" in stall, stall[:3000]

# Status toasts are chips in the pane they are about: a config.toml that
# was not applied shows in the focused (right) pane with "Open config.toml"
# and stays until its ✕; the button opens the file beside Settings
ts = scratch / "toasts"
(ts / "cfg/rift").mkdir(parents=True)
good_cfg = 'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
(ts / "good.toml").write_text(good_cfg)
(ts / "bad.toml").write_text("font_size = [\n")

def toast_run(name, steps):
    (ts / "cfg/rift/config.toml").write_text(good_cfg)
    (ts / f"{name}.rift").write_text("\n".join([
        "resize 1100 700", "wait_prompt", "key cmd+d", "wait_prompt", "wait 400",
        cp(ts / "bad.toml", ts / "cfg/rift/config.toml"), "wait 1500", *steps, "quit"]) + "\n")
    with (ts / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(ts / f"{name}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(ts / "cfg"),
                                     XDG_DATA_HOME=str(ts / f"data-{name}"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"toast {name} run failed", ts / f"{name}.log")

toast_run("probe", ["wait 3500", f"dump {ts}/probe.txt"])
probe = (ts / "probe.txt").read_text()
line = re.search(r"^toasts=.*$", probe, re.M)[0]
# still there after the 2.6 s a done toast gets: a failure stays
assert re.match(r'toasts=\[\(Error, Some\(\d+\), Some\(OpenConfig\), "config\.toml not applied', line), line
hits = re.search(r"toast_hits=\[\(\[([\d., -]+)\], Some\(\[([\d., -]+)\]\), Some\(\[([\d., -]+)\]\)\)\]",
                 line)
assert hits, line
chip, button, close = ([float(v) for v in g.split(",")] for g in hits.groups())
# right-aligned in the right pane (the window is 1100 wide, split in two)
assert chip[0] > 550 and abs(chip[0] + chip[2] - (1100 - 14)) < 2, chip
centre = lambda r: f"{r[0] + r[2] / 2:.0f} {r[1] + r[3] / 2:.0f}"
toast_run("close", [f"click {centre(close)}", "wait 300", f"dump {ts}/closed.txt"])
assert "toasts=[] " in (ts / "closed.txt").read_text(), (ts / "closed.txt").read_text()[:2000]
# fixing config.toml takes its error away by itself
toast_run("fixed", [cp(ts / "good.toml", ts / "cfg/rift/config.toml"), "wait 1500",
                    f"dump {ts}/fixed.txt"])
assert "toasts=[] " in (ts / "fixed.txt").read_text(), (ts / "fixed.txt").read_text()[:2000]
toast_run("action", [f"click {centre(button)}", "wait 600", f"dump {ts}/opened.txt"])
opened = (ts / "opened.txt").read_text()
assert "toasts=[] " in opened and "settings=true" in opened, opened[:2000]
assert re.search(r'viewer=Some\("[^"]*toasts/cfg/rift/config\.toml"\)', opened), opened[:2000]

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
                                 (up / "rawctl.py", up / "data-rr/rift/control", request, out))
        + " >/dev/null 2>&1 &",
        f"wait_file {out}.done 15000", "wait 500"]
up_run("restart", "data-rr", "same.json", [*restart_steps, f"dump {up}/restart.txt"])
for i in range(2):
    answer = (up / f"restart-{i}.json").read_text()
    assert answer.startswith("{"), answer
    refused = json.loads(answer)
    assert refused.get("ok") is False and "no update in progress" in refused.get("error", ""), refused
assert "update=" in up_state("restart"), "the window quit on a refused prepare-restart"
assert not list((up / "data-rr/rift").glob("restart/*.toml")), "a refused request saved a layout"

# a window an interrupted update saved (restart/<pid>.toml, a process per
# window before) and never reopened comes back on the next plain launch, as
# a window of that process, and its file is deleted
left = up / "data-left/rift/restart"
left.mkdir(parents=True)
(left / "99999.toml").write_text('[[tabs]]\ncwd = "/tmp"\ntitle = "LEFTOVER"\n')
(up / "cfg-left").mkdir()
(up / "cfg-left/rift").mkdir()
(up / "cfg-left/rift/config.toml").write_text("durable_sessions = false\n")
(up / "left-main.rift").write_text(
    f"wait_prompt\nwait 1500\nwindow 1\nwait_prompt\nwait 300\ndump {up}/left-window.txt\nquit\n")
with (up / "left-main.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(up / "left-main.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(up / "cfg-left"),
                                 XDG_DATA_HOME=str(up / "data-left"),
                                 RIFT_PERSIST_SESSION="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("leftover-window run failed", up / "left-main.log")
left_window = (up / "left-window.txt").read_text()
assert left_window.startswith("windows=2 window=1 "), left_window[:400]
assert 'title="LEFTOVER"' in left_window, left_window[:2000]
assert not list(left.glob("*.toml")), "the reopened window kept its restart file"

# --- remote access (mobile companion, window side) ---
# The gateway's `remote-input` request reaches the window only while
# remote_access is on (independent of control_api); a command runs only at a
# prompt; keys go to the running program; the managed gateway follows the
# setting; a connected phone shows a tab-bar chip. Each run has its own
# scratch XDG dirs, and the gateway listens on 127.0.0.1 only.
rm = scratch / "remote"
rm.mkdir()
(rm / "rin.sh").write_text(
    "#!/bin/sh\n"
    "# rin.sh BIN OUT JSON: send JSON (PANE = the first pane's id) to the window\n"
    "pane=$(\"$1\" ctl request '{\"cmd\":\"status\"}' | python3 -I -c "
    "'import json,sys; print(json.load(sys.stdin)[\"tabs\"][0][\"panes\"][0][\"id\"])')\n"
    "\"$1\" ctl request \"$(printf '%s' \"$3\" | sed \"s/PANE/$pane/\")\" > \"$2\" 2>&1\n"
    "touch \"$2.done\"\n")
(rm / "since.sh").write_text(
    "#!/bin/sh\n"
    "# since.sh BIN OUT SCREEN_JSON: ask for the screen again with the generation\n"
    "# SCREEN_JSON answered (the window builds none while it is current)\n"
    "pane=$(\"$1\" ctl request '{\"cmd\":\"status\"}' | python3 -I -c "
    "'import json,sys; print(json.load(sys.stdin)[\"tabs\"][0][\"panes\"][0][\"id\"])')\n"
    "gen=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"generation\"])' \"$3\")\n"
    "\"$1\" ctl request \"{\\\"cmd\\\":\\\"screen\\\",\\\"pane\\\":$pane,\\\"force\\\":true,"
    "\\\"since\\\":$gen}\" > \"$2\" 2>&1\n"
    "touch \"$2.done\"\n")
remote_base = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
               'remote_listen = "127.0.0.1:0"\n')

def rin(run, out, request):
    path = rm / run / out
    return [f"sh sh {shlex.quote(str(rm / 'rin.sh'))} {shlex.quote(str(binary))} "
            f"{shlex.quote(str(path))} {shlex.quote(request)} >/dev/null 2>&1 &",
            f"wait_file {path}.done 15000"]

def remote_run(name, config_text, steps, timeout=90, extra_env=None):
    run = rm / name
    (run / "cfg/rift").mkdir(parents=True)
    (run / "data/rift").mkdir(parents=True, exist_ok=True)
    (run / "cfg/rift/config.toml").write_text(config_text)
    (run / "t.rift").write_text(
        "\n".join(["resize 1100 800", "wait_prompt", *steps, "quit"]) + "\n")
    with (run / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(run / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(run / "cfg"),
                                     XDG_DATA_HOME=str(run / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1",
                                     **(extra_env or {})),
                            stdout=log, stderr=log, timeout=timeout).returncode
    assert rc == 0, died(f"remote {name} run failed", run / "log.txt")
    return run

def answer(run, name):
    return json.loads((run / name).read_text())

def remote_line(text):
    return next(l for l in text.splitlines() if l.startswith("remote_access="))

input_cmd = '{"cmd":"remote-input","pane":PANE,"device":"Test Phone","command":"echo remote-ok"}'
text_cmd = '{"cmd":"remote-input","pane":PANE,"device":"Test Phone","text":"typed-secret"}'

def keys_cmd(*keys):
    return json.dumps({"cmd": "remote-input", "pane": 0, "device": "Test Phone",
                       "keys": list(keys)}).replace('"pane": 0', '"pane":PANE')
settings_remote = ["key cmd+,", "wait 300", *["key down"] * 9, "wait 600"]
# the page scrolled to its end (the pointer over the page, not a list)
page_bottom = ["move 700 300", "scroll -80", "wait 300"]

# (a) remote_access off: refused, though the socket serves navigate requests
off = remote_run("off", remote_base, [
    "wait 500", *rin("off", "cmd.json", input_cmd), "wait 300",
    *rin("off", "meta.json", '{"cmd":"blocks","pane":PANE,"meta":true}'),
    *rin("off", "fit.json", '{"cmd":"remote-fit","pane":PANE,"cols":40,"rows":12}'),
    # the status a phone's topology reads carries the prompt chips the window
    # draws (it once deadlocked: the chips locked the cwd the status held)
    *rin("off", "status.json", '{"cmd":"status"}'),
    f"dump {rm}/off/off.txt"])
refused = answer(off, "cmd.json")
assert refused == {"ok": False, "error": "remote access is off"}, refused
assert "remote-ok" not in (off / "off.txt").read_text()
assert answer(off, "meta.json")["ok"] is True
chips = answer(off, "status.json")["tabs"][0]["panes"][0]["chips"]
assert chips and chips[0]["kind"] == "cwd", chips
assert chips[0]["short"][0]["tone"] == "bright" and chips[0]["full"][0]["text"], chips
assert answer(off, "fit.json") == {"ok": False, "error": "remote access is off"}, \
    answer(off, "fit.json")
assert "remote_access=false control_level=Navigate" in (off / "off.txt").read_text()
assert not (off / "data/rift/remote").exists(), "a gateway started with remote access off"

# `remote-open` (the phone's new tab / new pane keys): refused while remote
# access is off; with it on a tab opens in the window and a split right of
# the pane, each answering the new pane with a toast naming the device
open_tab = '{"cmd":"remote-open","pane":PANE,"device":"Test Phone","kind":"tab"}'
open_pane = '{"cmd":"remote-open","pane":PANE,"device":"Test Phone","kind":"pane"}'
open_off = remote_run("open-off", remote_base, [
    "wait 500", *rin("open-off", "tab.json", open_tab), "wait 300",
    f"dump {rm}/open-off/after.txt"])
assert answer(open_off, "tab.json") == {"ok": False, "error": "remote access is off"}, \
    answer(open_off, "tab.json")
assert "tabs=1 " in (open_off / "after.txt").read_text()
opened = remote_run("open", remote_base + 'control_api = "off"\nremote_access = true\n', [
    "wait 500", *rin("open", "tab.json", open_tab), "wait 600", f"dump {rm}/open/tab.txt",
    *rin("open", "pane.json", open_pane), "wait 600", f"dump {rm}/open/pane.txt",
    *rin("open", "gone.json", open_tab.replace("PANE", "PANE999")), "wait 300"])
assert answer(opened, "tab.json") == {"ok": True, "pane": 2}, answer(opened, "tab.json")
tab_dump = (opened / "tab.txt").read_text()
assert "tabs=2 active=1" in tab_dump, tab_dump[:400]
assert 'toast=Some("Test Phone opened a tab")' in tab_dump, tab_dump
assert answer(opened, "pane.json") == {"ok": True, "pane": 3}, answer(opened, "pane.json")
pane_dump = (opened / "pane.txt").read_text()
assert "tabs=2 active=0" in pane_dump, pane_dump[:400]
assert re.search(r"tab 0 title=.* panes=2 active_pane=3 ", pane_dump), pane_dump
assert 'toast=Some("Test Phone opened a pane")' in pane_dump, pane_dump
assert answer(opened, "gone.json") == {"ok": False, "error": "no such pane"}, \
    answer(opened, "gone.json")

# `remote-close` (the phone's close pane / close tab): refused while remote
# access is off; with it on it ends a pane or a whole tab without the
# running-command confirmation, naming the device in a toast
close_off = remote_run("close-off", remote_base, [
    "wait 500", *rin("close-off", "pane.json",
                     '{"cmd":"remote-close","pane":PANE,"device":"Test Phone","kind":"pane"}'),
    "wait 300", f"dump {rm}/close-off/after.txt"])
assert answer(close_off, "pane.json") == {"ok": False, "error": "remote access is off"}, \
    answer(close_off, "pane.json")
assert "tabs=1 " in (close_off / "after.txt").read_text()
close_pane = '{"cmd":"remote-close","pane":2,"device":"Test Phone","kind":"pane"}'
close_tab = '{"cmd":"remote-close","pane":4,"device":"Test Phone","kind":"tab"}'
closed = remote_run("close", remote_base + 'control_api = "off"\nremote_access = true\n', [
    "wait 500", *rin("close", "o1.json", open_tab), "wait 600",
    "type sleep 30\\n", "wait 300",
    *rin("close", "pane.json", close_pane), "wait 600", f"dump {rm}/close/pane.txt",
    *rin("close", "o2.json", open_tab), "wait 600",
    *rin("close", "o3.json", open_pane.replace("PANE", "3")), "wait 600",
    *rin("close", "tab.json", close_tab), "wait 600", f"dump {rm}/close/tab.txt",
    *rin("close", "gone.json", close_pane), "wait 300",
    # the window's last tab: a phone never quits the window, a fresh tab stays
    *rin("close", "last.json", close_tab.replace('"pane":4', '"pane":1')), "wait 800",
    f"dump {rm}/close/last.txt"])
assert answer(closed, "o1.json") == {"ok": True, "pane": 2}, answer(closed, "o1.json")
assert answer(closed, "pane.json") == {"ok": True}, answer(closed, "pane.json")
pane_dump = (closed / "pane.txt").read_text()
assert "tabs=1 " in pane_dump and 'toast=Some("Test Phone closed a pane")' in pane_dump, pane_dump[:600]
assert answer(closed, "o2.json") == {"ok": True, "pane": 3}, answer(closed, "o2.json")
assert answer(closed, "o3.json") == {"ok": True, "pane": 4}, answer(closed, "o3.json")
assert answer(closed, "tab.json") == {"ok": True}, answer(closed, "tab.json")
tab_dump = (closed / "tab.txt").read_text()
assert "tabs=1 " in tab_dump and 'toast=Some("Test Phone closed a tab")' in tab_dump, tab_dump[:600]
assert answer(closed, "gone.json") == {"ok": False, "error": "no such pane"}, answer(closed, "gone.json")
assert answer(closed, "last.json") == {"ok": True}, answer(closed, "last.json")
last_dump = (closed / "last.txt").read_text()
assert "tabs=1 " in last_dump and not re.search(r"tab 0 title=.* panes=1 active_pane=1 ", last_dump), \
    last_dump[:600]

# (b)-(d) remote access on with control_api off: the socket serves the
# gateway's requests only; a command at the prompt runs as a block with a
# toast; one while a command runs is busy; ctrl+c interrupts; turning
# remote access off stops the managed gateway within 2 s
on_config = remote_base + 'control_api = "off"\nremote_access = true\n'
(rm / "off-config.toml").write_text(remote_base + 'control_api = "off"\n')
on = rm / "on"
status_file = on / "data/rift/remote/status.json"
on_run = remote_run("on", on_config, [
    f"wait_file {status_file} 20000", "wait 1500",
    *rin("on", "cmd.json", input_cmd), "wait 800", f"dump {on}/cmd.txt",
    *rin("on", "send.json", '{"cmd":"send-text","pane":PANE,"text":"echo no\\n"}'),
    *rin("on", "focus.json", '{"cmd":"focus","pane":PANE}'),
    *rin("on", "text-busy.json", text_cmd),
    # keys at the prompt never build or submit a command line: Enter with
    # text on the line is refused; history, completion and the ghost
    # suggestion do nothing; an empty line's Enter passes
    "type echo draft", "wait 300",
    *rin("on", "enter-draft.json", keys_cmd("enter")),
    *rin("on", "recall.json", keys_cmd("up", "tab", "ctrl+right", "ctrl+r")),
    "wait 300", f"dump {on}/draft.txt",
    *rin("on", "clear.json", keys_cmd("ctrl+u")),
    *rin("on", "empty-enter.json", keys_cmd("enter")), "wait 600",
    "type sleep 5\\n", "wait 600",
    *rin("on", "busy.json", input_cmd.replace("echo remote-ok", "echo too-early")),
    "wait 5000",
    "type sleep 30\\n", "wait 800",
    *rin("on", "text.json", text_cmd), "wait 600", f"dump {on}/text.txt",
    *rin("on", "alt.json",
         '{"cmd":"remote-input","pane":PANE,"device":"Test Phone","keys":["alt+b"]}'),
    "wait 600",
    *rin("on", "screen.json", '{"cmd":"screen","pane":PANE,"force":true}'),
    f"sh sh {shlex.quote(str(rm / 'since.sh'))} {shlex.quote(str(binary))} "
    f"{shlex.quote(str(on / 'since.json'))} {shlex.quote(str(on / 'screen.json'))} >/dev/null 2>&1 &",
    f"wait_file {on / 'since.json'}.done 15000",
    *rin("on", "noscreen.json", '{"cmd":"screen","pane":PANE}'),
    # a phone's fit: the shell is sized to the smaller of the window's grid
    # and the phone's; half a size is refused; the release gives it back
    *rin("on", "fit.json", '{"cmd":"remote-fit","pane":PANE,"cols":40,"rows":12}'), "wait 700",
    *rin("on", "fit-screen.json", '{"cmd":"screen","pane":PANE,"force":true}'),
    *rin("on", "fit-bad.json", '{"cmd":"remote-fit","pane":PANE,"cols":40}'),
    *rin("on", "fit-off.json", '{"cmd":"remote-fit","pane":PANE,"cols":null,"rows":null}'),
    "wait 700",
    *rin("on", "fit-back.json", '{"cmd":"screen","pane":PANE,"force":true}'),
    *rin("on", "noblock.json", '{"cmd":"output","pane":PANE,"block":999999,"from":0,"limit":5}'),
    *rin("on", "keys.json",
         '{"cmd":"remote-input","pane":PANE,"device":"Test Phone","keys":["ctrl+c"]}'),
    "wait 1200", f"dump {on}/keys.txt",
    *rin("on", "meta.json", '{"cmd":"blocks","pane":PANE,"meta":true}'),
    *rin("on", "full.json", '{"cmd":"blocks","pane":PANE}'),
    *settings_remote, f"shot {on}/settings.png", f"dump {on}/settings.txt",
    cp(rm / "off-config.toml", on / "cfg/rift/config.toml"), "wait 3500",
    f"dump {on}/stopped.txt"], timeout=120)

# The Cloudflare tunnel (remote_tunnel) without cloudflared: the checklist
# offers the install, the tunnel and relay wait, and the gateway says so in
# tunnel.json; turning remote access off removes it
tun = rm / "tunnel"
tunnel_file = tun / "data/rift/remote/tunnel.json"
remote_run("tunnel", remote_base + 'control_api = "off"\nremote_access = true\n'
           'remote_tunnel = true\n', [
    f"wait_file {tunnel_file} 20000", "wait 1500",
    *settings_remote, f"dump {tun}/top.txt",
    # the tunnel lives under "Connection settings" at the page's end
    *page_bottom, "a11y click Connection settings, collapsed", "wait 300", *page_bottom,
    "wait 500", f"dump {tun}/settings.txt",
    cp(rm / "off-config.toml", tun / "cfg/rift/config.toml"), "wait 3500"],
    extra_env={"RIFT_CLOUDFLARED": ""})
tunnel_page = (tun / "settings.txt").read_text()
for row in ['Switch "Use a Cloudflare tunnel instead" toggled=True', 'cloudflared"',
            'Label "Tunnel: Waiting for cloudflared"']:
    assert row in tunnel_page, (row, tunnel_page)
assert 'Label "Away from home: Waiting for the tunnel"' in (tun / "top.txt").read_text(), \
    (tun / "top.txt").read_text()[-3000:]
assert re.search(r'Button "(Install|Download) cloudflared"', tunnel_page), tunnel_page
assert not tunnel_file.exists(), tunnel_file.read_text()

gateway_line = remote_line((on / "settings.txt").read_text())
assert "remote_access=true control_level=Remote" in gateway_line, gateway_line
assert re.search(r'gateway="Listening on 127\.0\.0\.1:\d+" gateway_pid=Some\(\d+\)', gateway_line), \
    gateway_line
assert "settings=true page=Remote" in (on / "settings.txt").read_text()
assert 'Switch "Allow remote access"' in (on / "settings.txt").read_text()
assert (on / "settings.png").read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
assert answer(on, "cmd.json") == {"ok": True, "pane": 1}, answer(on, "cmd.json")
cmd_dump = (on / "cmd.txt").read_text()
assert re.search(r'block \d+ state=done .*cmd="echo remote-ok" exit=Some\(0\)', cmd_dump), cmd_dump
assert 'toast=Some("Test Phone ran echo remote-ok")' in cmd_dump, cmd_dump
assert "not allowed" in answer(on, "send.json")["error"], answer(on, "send.json")
assert "not allowed" in answer(on, "focus.json")["error"], answer(on, "focus.json")
assert answer(on, "busy.json") == {"ok": False, "error": "busy"}, answer(on, "busy.json")
# text: busy at the prompt (a command line goes through `run`); typed into a
# running program with a toast that never shows it; alt is Meta (ESC b, which
# the tty echoes as ^[b); the gateway's screen and output reads
assert answer(on, "text-busy.json") == {"ok": False, "error": "busy"}, answer(on, "text-busy.json")
assert answer(on, "enter-draft.json") == \
    {"ok": False, "error": "the command line has text; send the command with run"}, \
    answer(on, "enter-draft.json")
assert answer(on, "recall.json") == {"ok": True, "pane": 1}, answer(on, "recall.json")
draft_dump = (on / "draft.txt").read_text()
assert 'editor="echo draft"' in draft_dump, draft_dump
assert answer(on, "clear.json") == {"ok": True, "pane": 1}, answer(on, "clear.json")
assert answer(on, "empty-enter.json") == {"ok": True, "pane": 1}, answer(on, "empty-enter.json")
assert answer(on, "text.json") == {"ok": True, "pane": 1}, answer(on, "text.json")
text_dump = (on / "text.txt").read_text()
assert 'toast=Some("Test Phone typed into a program")' in text_dump, text_dump
assert answer(on, "alt.json") == {"ok": True, "pane": 1}, answer(on, "alt.json")
shown = answer(on, "screen.json")
assert shown["ok"] and shown["screen"]["alt"] is False, shown
# asked again with its generation: the screen did not change, none is built
assert answer(on, "since.json") == {"ok": True, "pane": 1, "generation": shown["generation"],
                                    "unchanged": True}, answer(on, "since.json")
screen_text = "\n".join("".join(r[0] for r in row) for row in shown["screen"]["lines"])
assert "typed-secret^[b" in screen_text, screen_text
# the forced screen is Rift's own view: a header row for the running command,
# not the shell's prompt echo
shown_rows = ["".join(r[0] for r in row) for row in shown["screen"]["lines"]]
header_rows = [t for t in shown_rows if t.startswith("▾ sleep 30")]
assert len(header_rows) == 1 and header_rows[0].rstrip().endswith("running"), shown_rows
assert sum("sleep 30" in t for t in shown_rows) == 1, shown_rows
fit_screen = answer(on, "fit-screen.json")["screen"]
assert (fit_screen["cols"], fit_screen["rows"]) == (40, 12), fit_screen
assert all(len("".join(r[0] for r in row)) <= 40 for row in fit_screen["lines"]), fit_screen
assert answer(on, "fit-bad.json")["ok"] is False, answer(on, "fit-bad.json")
assert answer(on, "fit-off.json")["ok"] is True, answer(on, "fit-off.json")
fit_back = answer(on, "fit-back.json")["screen"]
assert fit_back["cols"] > 40 and fit_back["rows"] > 12, fit_back
noscreen = answer(on, "noscreen.json")
assert noscreen == {"ok": True, "pane": 1, "generation": noscreen.get("generation")} \
    and "screen" not in noscreen, noscreen
assert answer(on, "noblock.json") == {"ok": False, "error": "no such block"}, \
    answer(on, "noblock.json")
keys_dump = (on / "keys.txt").read_text()
assert "too-early" not in keys_dump, "a busy command reached the shell"
assert 'cmd="echo draft"' not in keys_dump, "a phone's Enter submitted the desktop's draft"
assert re.search(r'cmd="sleep 30" exit=Some\([1-9]\d*\)', keys_dump), keys_dump
assert 'toast=Some("Test Phone sent ctrl+c")' in keys_dump, keys_dump
meta = answer(on, "meta.json")["blocks"]
full = answer(on, "full.json")["blocks"]
assert [b["id"] for b in meta] == [b["id"] for b in full] and len(meta) >= 3, (meta, full)
assert all(b["output"] == "" and b["dropped"] == 0 for b in meta), meta
assert any(b["output"] == "remote-ok\n" for b in full), full
stopped = remote_line((on / "stopped.txt").read_text())
# Linux keeps the rift:// links level with automation off (control.rs)
off_level = "Off" if sys.platform == "darwin" else "Links"
assert f'remote_access=false control_level={off_level} gateway="Off" gateway_pid=None' in stopped, stopped
assert not status_file.exists(), "the managed gateway outlived remote_access = false"
# only this test's gateway, found through its scratch status, is ever stopped
left = re.search(r"gateway_pid=Some\((\d+)\)", gateway_line)
if left and subprocess.run(["ps", "-p", left[1]], capture_output=True).returncode == 0:
    own = subprocess.run(["ps", "eww", "-p", left[1]], capture_output=True, text=True).stdout
    if str(on / "data") in own:
        os.kill(int(left[1]), 15)
    raise AssertionError("the managed gateway did not exit after remote_access = false")

# (e) an idle window with remote_access off; a gateway started by hand and
# a real phone (examples/phone) pairing through it: the gateway's
# `remote-status` notices make the chip appear within 2 s of the phone
# connecting and go within 2 s of it leaving, while the window sleeps in
# one long wait (chip_age_ms dates the change; no wake of its own)
phone_bin = os.environ.get("RIFT_PHONE_BIN")
assert phone_bin, "RIFT_PHONE_BIN is unset (scripts/ui-smoke.sh builds examples/phone)"
idle = rm / "idle"
for sub in ("cfg/rift", "data/rift"):
    (idle / sub).mkdir(parents=True)
(idle / "cfg/rift/config.toml").write_text(remote_base)
(idle / "t.rift").write_text("\n".join([
    "resize 1100 800", "wait_prompt", "wait 500", f"dump {idle}/ready.txt",
    "wait 9000", f"dump {idle}/connected.txt", f"shot {idle}/chip.png",
    "wait 9000", f"dump {idle}/left.txt", "quit"]) + "\n")
idle_env = dict(env, XDG_CONFIG_HOME=str(idle / "cfg"), XDG_DATA_HOME=str(idle / "data"),
                BASH_SILENCE_DEPRECATION_WARNING="1")
gateway = None
with (idle / "log.txt").open("w") as log, (idle / "gateway.txt").open("w") as glog, \
        (idle / "phone.txt").open("w") as plog:
    window = subprocess.Popen([str(binary), "--script", str(idle / "t.rift"), "--headless"],
                              env=idle_env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 40
        while not (idle / "ready.txt").exists():
            assert window.poll() is None, died("idle window exited", idle / "log.txt")
            assert time.monotonic() < deadline, "idle window never got ready"
            time.sleep(0.05)
        gateway = subprocess.Popen([str(binary), "remote", "--listen", "127.0.0.1:0"],
                                   env=idle_env, stdout=glog, stderr=glog)
        status = idle / "data/rift/remote/status.json"
        while not status.exists():
            assert gateway.poll() is None, died("hand-started gateway exited", idle / "gateway.txt")
            time.sleep(0.05)
        link = next(l for l in subprocess.run(
            [str(binary), "remote", "pair"], env=idle_env, capture_output=True, text=True,
            timeout=20).stdout.splitlines() if l.startswith("rift-remote://"))
        acked = idle / "phone-acked"
        phone_start = time.time()
        phone = subprocess.Popen([phone_bin, str(idle / "phone.key"), link, "true", "12",
                                  str(acked)], stdout=plog, stderr=plog)
        while not acked.exists():
            assert phone.poll() is None, died("the phone gave up", idle / "phone.txt")
            time.sleep(0.02)
        phone_acked = time.time()
        phone.wait(timeout=30)
        phone_left = time.time()
        assert window.wait(timeout=60) == 0, died("idle window failed", idle / "log.txt")
    finally:
        # only the gateway this test started
        if gateway is not None and gateway.poll() is None:
            gateway.terminate()
            gateway.wait(timeout=10)
        if window.poll() is None:
            window.kill()

def changed_at(name):
    path = idle / f"{name}.txt"
    line = remote_line(path.read_text())
    age = int(re.search(r"chip_age_ms=Some\((\d+)\)", line)[1])
    return line, path.stat().st_mtime - age / 1000

line, appeared = changed_at("connected")
assert 'remote_access=false' in line and 'remote_chip=Some("Remote · Live Check Phone")' in line, line
notices = int(re.search(r"remote_notices=(\d+)", line)[1])
assert notices >= 2, f"no remote-status notices reached the window: {line}"
assert phone_start <= appeared <= phone_acked + 2, (phone_start, appeared, phone_acked)
line, went = changed_at("left")
assert "clients=[] remote_chip=None" in line, line
assert phone_acked <= went <= phone_left + 2, (phone_acked, went, phone_left)

# (e2) a real phone closes a pane through the gateway: the window has two
# tabs, the phone's `close` (PHONE_SEND, after its `run`) ends the last
# one; its ack is ok and the window is left with one tab and a toast
closer = rm / "closer"
for sub in ("cfg/rift", "data/rift"):
    (closer / sub).mkdir(parents=True)
(closer / "cfg/rift/config.toml").write_text(remote_base + "remote_access = true\n")
(closer / "t.rift").write_text("\n".join([
    "resize 1100 800", "wait_prompt", "key cmd+t", "wait_prompt", "wait 800",
    f"dump {closer}/ready.txt", f"wait_file {closer}/phone-acked 40000", "wait 800",
    f"dump {closer}/after.txt", "quit"]) + "\n")
closer_env = dict(env, XDG_CONFIG_HOME=str(closer / "cfg"), XDG_DATA_HOME=str(closer / "data"),
                  BASH_SILENCE_DEPRECATION_WARNING="1")
with (closer / "log.txt").open("w") as log, (closer / "phone.txt").open("w") as plog:
    window = subprocess.Popen([str(binary), "--script", str(closer / "t.rift"), "--headless"],
                              env=closer_env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 40
        while not (closer / "ready.txt").exists():
            assert window.poll() is None, died("closer window exited", closer / "log.txt")
            assert time.monotonic() < deadline, "closer window never got ready"
            time.sleep(0.05)
        # the window's own gateway (remote_access = true)
        while not (closer / "data/rift/remote/status.json").exists():
            assert window.poll() is None, died("closer window exited", closer / "log.txt")
            time.sleep(0.05)
        link = next(l for l in subprocess.run(
            [str(binary), "remote", "pair"], env=closer_env, capture_output=True, text=True,
            timeout=20).stdout.splitlines() if l.startswith("rift-remote://"))
        phone = subprocess.run(
            [phone_bin, str(closer / "phone.key"), link, "true", "3", str(closer / "phone-acked")],
            env=dict(os.environ, PHONE_SEND='{"t":"close","id":"c1","pane":LAST,"kind":"pane"}'),
            stdout=plog, stderr=plog, timeout=60)
        assert phone.returncode == 0, died("the closing phone", closer / "phone.txt")
        assert window.wait(timeout=60) == 0, died("closer window failed", closer / "log.txt")
    finally:
        if window.poll() is None:
            window.kill()
transcript = (closer / "phone.txt").read_text()
welcome_line = next(l for l in transcript.splitlines() if '"t":"welcome"' in l)
assert '"close"' in welcome_line, welcome_line
assert re.search(r'← \{"id":"c1","ok":true,"t":"ack"\}', transcript), transcript[-2000:]
assert "tabs=2 " in (closer / "ready.txt").read_text()
closed_dump = (closer / "after.txt").read_text()
assert "tabs=1 " in closed_dump and 'closed a pane")' in closed_dump, closed_dump[:600]

# phone scroll and image paste: a real phone (examples/phone) through the
# window's own gateway. Tab 1 holds 400 lines at a prompt, tab 2 a program
# on the alternate screen with SGR mouse tracking that records what it
# reads. The phone's `scroll` of tab 1 moves the phone's own view of it (the
# styled screen it is sent), never the desktop's (view_offset stays 0); its
# `scroll` of tab 2 reaches the program as SGR wheel reports (the wheel's
# rule, mouse::wheel); its `upload` is saved by the gateway (0600, a name it
# made) and the path pasted into the program
pin = scratch / "phone-input"
for sub in ("cfg/rift", "data/rift"):
    (pin / sub).mkdir(parents=True)
(pin / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'remote_listen = "127.0.0.1:0"\nremote_relay = ""\nremote_access = true\n')
(pin / "scr.sh").write_text(
    "#!/bin/sh\n"
    "# scr.sh BIN OUT: the styled screen a phone is sent of the first tab's pane\n"
    "pane=$(\"$1\" ctl request '{\"cmd\":\"status\"}' | python3 -I -c "
    "'import json,sys; print(json.load(sys.stdin)[\"tabs\"][0][\"panes\"][0][\"id\"])')\n"
    "\"$1\" ctl request \"{\\\"cmd\\\":\\\"screen\\\",\\\"pane\\\":$pane,\\\"force\\\":true}\""
    " > \"$2\" 2>&1\n"
    "touch \"$2.done\"\n")
def pin_screen(out):
    return [f"sh sh {shlex.quote(str(pin / 'scr.sh'))} {shlex.quote(str(binary))} "
            f"{shlex.quote(str(pin / out))} >/dev/null 2>&1 &",
            f"wait_file {pin / out}.done 15000"]
(pin / "wheel.py").write_text(
    "import os, select, sys, termios, time, tty\n"
    "out, ready = sys.argv[1], sys.argv[2]\n"
    "fd = sys.stdin.fileno()\n"
    "old = termios.tcgetattr(fd)\n"
    "tty.setraw(fd)\n"
    "# the alternate screen, button tracking, SGR reports\n"
    "sys.stdout.write('\\x1b[?1049h\\x1b[?1000h\\x1b[?1006hwheel'); sys.stdout.flush()\n"
    "open(ready, 'w').close()\n"
    "got = b''\n"
    "end = time.time() + 40\n"
    "while time.time() < end and b'.png' not in got:\n"
    "    if select.select([fd], [], [], 0.2)[0]:\n"
    "        got += os.read(fd, 4096)\n"
    "        open(out, 'wb').write(got)\n"
    "sys.stdout.write('\\x1b[?1006l\\x1b[?1000l\\x1b[?1049l'); sys.stdout.flush()\n"
    "termios.tcsetattr(fd, termios.TCSADRAIN, old)\n")
(pin / "t.rift").write_text("\n".join([
    "resize 1100 800", "wait_prompt", "type seq 1 400\\n", "wait 1500", "key cmd+t", "wait_prompt",
    f"type python3 -I {pin}/wheel.py {pin}/read.bin {pin}/tracking\\n",
    f"wait_file {pin}/tracking 20000", "wait 500", *pin_screen("before.json"),
    f"dump {pin}/ready.txt", f"wait_file {pin}/phone-acked 60000", "wait 1000",
    *pin_screen("after.json"), f"dump {pin}/after.txt", "quit"]) + "\n")
pin_env = dict(env, XDG_CONFIG_HOME=str(pin / "cfg"), XDG_DATA_HOME=str(pin / "data"),
               BASH_SILENCE_DEPRECATION_WARNING="1", RIFT_NO_DEFAULT_RELAY="1")
pin_phone = os.environ.get("RIFT_PHONE_BIN")
assert pin_phone, "RIFT_PHONE_BIN is unset (scripts/ui-smoke.sh builds examples/phone)"
pin_png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=")
pin_send = "\n".join(json.dumps(m) for m in [
    {"t": "scroll", "id": "s1", "pane": "FIRST", "lines": 10},
    {"t": "scroll", "id": "s2", "pane": "LAST", "lines": 2},
    {"t": "scroll", "id": "s3", "pane": "LAST", "lines": -1},
    {"t": "upload", "id": "u1", "pane": "LAST", "size": len(pin_png), "offset": 0,
     "data": base64.b64encode(pin_png).decode()},
]).replace('"FIRST"', "FIRST").replace('"LAST"', "LAST")
with (pin / "log.txt").open("w") as log, (pin / "phone.txt").open("w") as plog:
    window = subprocess.Popen([str(binary), "--script", str(pin / "t.rift"), "--headless"],
                              env=pin_env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 60
        while not (pin / "ready.txt").exists():
            assert window.poll() is None, died("phone-input window exited", pin / "log.txt")
            assert time.monotonic() < deadline, "phone-input window never got ready"
            time.sleep(0.05)
        while not (pin / "data/rift/remote/status.json").exists():
            assert window.poll() is None, died("phone-input window exited", pin / "log.txt")
            time.sleep(0.05)
        link = next(l for l in subprocess.run(
            [str(binary), "remote", "pair"], env=pin_env, capture_output=True, text=True,
            timeout=20).stdout.splitlines() if l.startswith("rift-remote://"))
        assert "&r=" not in link, f"a relay in a local test's link: {link}"
        phone = subprocess.run(
            [pin_phone, str(pin / "phone.key"), link, "true", "3", str(pin / "phone-acked")],
            env=dict(os.environ, PHONE_SEND=pin_send), stdout=plog, stderr=plog, timeout=90)
        assert phone.returncode == 0, died("the scrolling phone", pin / "phone.txt")
        assert window.wait(timeout=60) == 0, died("phone-input window failed", pin / "log.txt")
    finally:
        if window.poll() is None:
            window.kill()
transcript = (pin / "phone.txt").read_text()
welcome_line = next(l for l in transcript.splitlines() if '"t":"welcome"' in l)
assert '"scroll"' in welcome_line and '"upload"' in welcome_line, welcome_line
for rid in ("s1", "s2", "s3", "u1"):
    assert re.search(rf'← \{{"id":"{rid}","ok":true,"t":"ack"\}}', transcript), (rid, transcript[-3000:])
def pane_lines(dump):
    return [l for l in dump.splitlines() if l.startswith("  pane ") and " view_offset=" in l]
ready_panes = pane_lines((pin / "ready.txt").read_text())
after_dump = (pin / "after.txt").read_text()
after_panes = pane_lines(after_dump)
assert len(ready_panes) == 2 and len(after_panes) == 2, (ready_panes, after_panes)
# tab 1 (a prompt over 400 lines): the phone's view moved 10 lines up, the
# desktop's not at all; tab 2's (the program's alternate screen) never did
assert " view_offset=0 " in ready_panes[0] and " alt=true " in ready_panes[1], ready_panes
assert " view_offset=0 " in after_panes[0] and " alt=false " in after_panes[0], after_panes
def pin_rows(name):
    reply = json.loads((pin / name).read_text())
    assert reply.get("ok"), reply
    return ["".join(r[0] if isinstance(r, list) else r["text"] for r in line).strip()
            for line in reply["screen"]["lines"]]
before_rows, after_rows = pin_rows("before.json"), pin_rows("after.json")
# the phone's screen ends 10 lines further up (the output's last line, 400,
# ends the unscrolled one)
last_number = lambda rows: next(l for l in reversed(rows) if l.isdigit())
assert (last_number(before_rows), last_number(after_rows)) == ("400", "390"), (
    before_rows[-12:], after_rows[-12:])
# (the program left the alternate screen once the pasted path arrived)
assert " view_offset=0 " in after_panes[1], after_panes
read = (pin / "read.bin").read_bytes()
# two reports up, one down, at the middle of the screen, no modifiers
ups, downs = re.findall(rb"\x1b\[<64;\d+;\d+M", read), re.findall(rb"\x1b\[<65;\d+;\d+M", read)
assert (len(ups), len(downs)) == (2, 1), read
pasted = re.search(rb"(/[^\s'\x1b]*/rift-paste-\d+-[0-9a-f]{8}\.png)'? $", read)
assert pasted, read
saved = Path(pasted[1].decode())
assert saved.parent == pin / "data/rift/remote/uploads", saved
assert saved.read_bytes() == pin_png
assert saved.stat().st_mode & 0o777 == 0o600 and saved.parent.stat().st_mode & 0o777 == 0o700
assert "Live Check Phone pasted an image" in after_dump, after_dump[:3000]

# (f) the chip on every window comes from status.json of a gateway holding
# gateway.lock; a stale status (lock not held) shows none
import fcntl
chip = rm / "chip"
chip_remote = chip / "data/rift/remote"
chip_remote.mkdir(parents=True)
(chip_remote / "status.json").write_text(json.dumps({
    "pid": os.getpid(), "addresses": ["127.0.0.1:9"],
    "clients": [{"device": "d3f1", "name": "Test Phone", "since": "2026-10-12T10:00:00Z"}]}))
lock = open(chip_remote / "gateway.lock", "w")
fcntl.flock(lock, fcntl.LOCK_EX)
(rm / "release.sh").write_text(f"rm -f {shlex.quote(str(chip / 'held'))}\n")
release = threading.Event()
def unlock_when_asked():
    while (chip / "held").exists():
        time.sleep(0.05)
    fcntl.flock(lock, fcntl.LOCK_UN)
    lock.close()
    release.set()
(chip / "held").parent.mkdir(parents=True, exist_ok=True)
(chip / "held").touch()
threading.Thread(target=unlock_when_asked, daemon=True).start()
chip_run = remote_run("chip", remote_base, [
    "wait 1500", f"dump {chip}/shown.txt", f"shot {chip}/chip.png",
    f"sh sh {shlex.quote(str(rm / 'release.sh'))}", "wait 2500", f"dump {chip}/gone.txt"])
shown = (chip / "shown.txt").read_text()
assert 'remote_chip=Some("Remote · Test Phone")' in remote_line(shown), remote_line(shown)
assert 'Button "Remote · Test Phone, connected: Test Phone"' in shown, shown[-3000:]
assert "Connected: Test Phone@" in shown, "the chip has no tooltip"
gone = remote_line((chip / "gone.txt").read_text())
assert release.is_set()
assert "gateway_pid=None clients=[] remote_chip=None" in gone, gone

# (f2) quitting is intentional: every tab ends and the session file keeps
# only the window's frame, so the next launch starts fresh. While a phone is
# connected, ⌘Q asks first (Esc keeps the window); without one it quits at
# once, even with no command running.
def quit_run(name, phone):
    run = rm / name
    held = None
    if phone:
        (run / "data/rift/remote").mkdir(parents=True)
        (run / "data/rift/remote/status.json").write_text(json.dumps({
            "pid": os.getpid(), "addresses": ["127.0.0.1:9"],
            "clients": [{"device": "d3f2", "name": "Quit Phone", "since": "2026-10-16T10:00:00Z"}]}))
        held = open(run / "data/rift/remote/gateway.lock", "w")
        fcntl.flock(held, fcntl.LOCK_EX)
    steps = ["key cmd+t", "wait_prompt", "wait 1500"]
    if phone:
        steps += ["key cmd+q", "wait 400", f"dump {run}/asked.txt", "key escape",
                  "wait 100", f"dump {run}/kept.txt", "key cmd+q", "wait 400", "key enter"]
    else:
        steps += ["key cmd+q"]
    # the script's own trailing quit is never reached: ⌘Q ends the run
    steps += ["wait 3000", f"dump {run}/not-quit.txt"]
    try:
        remote_run(name, remote_base, steps, extra_env={"RIFT_PERSIST_SESSION": "1"})
    finally:
        if held is not None:
            held.close()
    assert not (run / "not-quit.txt").exists(), f"{name}: ⌘Q did not quit"
    saved = (run / "data/rift/session.toml").read_text()
    assert "[[tabs]]" not in saved and "[window]" in saved, f"{name}: {saved}"
    return run
asked_run = quit_run("quit-phone", True)
asked = (asked_run / "asked.txt").read_text()
# ⌘Q quits every window (CloseScope::App); closing the last window would ask
# the same
assert "confirm=Some((Close(App), []))" in asked, asked[:2000]
assert 'Quit Rift?' in asked and "Quit Phone is connected and loses its tabs." in asked, asked[-3000:]
assert "confirm=None" in (asked_run / "kept.txt").read_text()
quit_run("quit-alone", False)

# (f3) Settings › General opens with the App group (Launch at login on macOS;
# no background setting: remote access decides); on macOS the menu bar icon's Quit Rift Completely quits like
# ⌘Q and leaves `remote/stop` for the running gateway; a plain ⌘Q does not
if sys.platform == "darwin":
    def full_quit_run(name, quit_step):
        run = rm / name
        (run / "data/rift/remote").mkdir(parents=True)
        # a gateway with no phone: its lock held, its status naming a live pid
        (run / "data/rift/remote/status.json").write_text(json.dumps({
            "pid": os.getpid(), "addresses": ["127.0.0.1:9"], "clients": []}))
        with open(run / "data/rift/remote/gateway.lock", "w") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            remote_run(name, remote_base, [
                "key cmd+,", "wait 600", f"dump {run}/settings.txt", "key escape", "wait 200",
                quit_step, "wait 3000", f"dump {run}/not-quit.txt"])
        assert not (run / "not-quit.txt").exists(), f"{name}: did not quit"
        return run
    full = full_quit_run("quit-completely", "menu status:quit_completely")
    general = (full / "settings.txt").read_text()
    assert "Launch at login" in general and "Allow background processes" not in general, general[-3000:]
    assert (full / "data/rift/remote/stop").exists(), "Quit Rift Completely left the gateway running"
    plain = full_quit_run("quit-plain", "key cmd+q")
    assert not (plain / "data/rift/remote/stop").exists(), "⌘Q stopped the gateway"

# (f4) Remote access on (macOS): a quit keeps Rift running. ⌘Q asks "Keep
# running?" (with a Don't-ask-again box: Space ticks it, Keep running stores
# it in config.toml), the shells stay in the session service, the session file
# is saved whole and flagged `kept`, the next launch restores the tab and
# reattaches the same running shell (restore_session is off: a kept session
# restores anyway) and a second ⌘Q asks nothing; Quit Rift Completely ends the
# shells and the kept tabs go with them. Without the setting a quit ends them.
if sys.platform == "darwin":  # the kept background is macOS only (the menu bar icon ends it)
    kq = scratch / "keep-quit"
    for sub in ("cfg/rift", "data/rift"):
        (kq / sub).mkdir(parents=True, exist_ok=True)
    (kq / "cfg/rift/config.toml").write_text(
        'remote_access = true\ndurable_sessions = true\nrestore_session = false\n'
        'check_for_updates = false\nshell = "/bin/bash"\n'
        'remote_listen = "127.0.0.1:0"\n')
    kq_env = dict(env, XDG_CONFIG_HOME=str(kq / "cfg"), XDG_DATA_HOME=str(kq / "data"),
                  RIFT_PERSIST_SESSION="1")
    def kq_run(name, steps):
        (kq / f"{name}.rift").write_text("\n".join(steps) + "\n")
        with (kq / f"{name}.log").open("w") as log:
            rc = subprocess.run([str(binary), "--script", str(kq / f"{name}.rift"), "--headless"],
                                env=kq_env, stdout=log, stderr=log, timeout=90).returncode
        assert rc == 0, died(f"keep-quit {name} run failed", kq / f"{name}.log")
    def kq_text(name):
        path = kq / f"{name}.txt"
        return path.read_text() if path.exists() else ""
    try:
        kq_run("one", [
            "wait_prompt", "type sleep 600\\n", "wait 800", "key cmd+q", "wait 600",
            f"dump {kq}/dialog.txt", "key space", "wait 300", "key enter", "wait 2500",
            f"dump {kq}/not-quit.txt"])
        dialog = kq_text("dialog")
        assert 'AlertDialog "Quit Rift?"' in dialog and "keeps running in the menu bar" in dialog \
            and 'Button "Keep running"' in dialog and "Don’t ask again" in dialog, dialog[-1500:]
        assert "terminated" not in dialog, dialog[-1500:]
        assert not (kq / "not-quit.txt").exists(), "Keep running did not quit"
        saved = (kq / "data/rift/session.toml").read_text()
        assert "kept = true" in saved and "[[tabs]]" in saved, saved
        assert "confirm_background_quit = false" in (kq / "cfg/rift/config.toml").read_text()
        assert len(list(session_daemons(kq / "data"))) == 1, "the shell did not stay in the session service"
        kq_run("two", ["wait 2500", f"dump {kq}/restored.txt", "quit"])
        restored = kq_text("restored")
        assert "sleep 600" in restored and re.search(r"service=Some\(\d+\)", restored), restored[-1500:]
        # "Don't ask again" was ticked: this quit asks nothing, and keeps running again
        kq_run("three", ["wait 1500", "key cmd+q", "wait 2500", f"dump {kq}/not-quit3.txt"])
        assert not (kq / "not-quit3.txt").exists(), "the second quit asked a question"
        assert "kept = true" in (kq / "data/rift/session.toml").read_text()
        # Quit Rift Completely (macOS menu): ends the shells; nothing is kept
        if sys.platform == "darwin":
            kq_run("four", [
                "wait 2500", "menu status:quit_completely", "wait 900", f"dump {kq}/qc.txt",
                "key enter", "wait 3000", f"dump {kq}/not-quit4.txt"])
            qc = kq_text("qc")
            assert "Keep running" not in qc and "terminated" in qc, qc[-1500:]
            assert not (kq / "not-quit4.txt").exists(), "Quit Rift Completely did not quit"
            after = (kq / "data/rift/session.toml").read_text()
            assert "kept = true" not in after and "[[tabs]]" not in after, after
        # remote access off: nothing shows or ends a kept Rift, so ⌘Q is the plain quit
        off = scratch / "keep-quit-off"
        for sub in ("cfg/rift", "data/rift"):
            (off / sub).mkdir(parents=True, exist_ok=True)
        (off / "cfg/rift/config.toml").write_text(
            'remote_access = false\ndurable_sessions = true\nrestore_session = false\n'
            'check_for_updates = false\nshell = "/bin/bash"\n')
        (off / "q.rift").write_text("\n".join([
            "wait_prompt", "type sleep 600\\n", "wait 800", "key cmd+q", "wait 600",
            f"dump {off}/dialog.txt", "key enter", "wait 2500", f"dump {off}/not-quit.txt"]) + "\n")
        with (off / "q.log").open("w") as log:
            subprocess.run([str(binary), "--script", str(off / "q.rift"), "--headless"],
                           env=dict(kq_env, XDG_CONFIG_HOME=str(off / "cfg"),
                                    XDG_DATA_HOME=str(off / "data")),
                           stdout=log, stderr=log, timeout=90)
        plain_q = (off / "dialog.txt").read_text() if (off / "dialog.txt").exists() else ""
        assert "Keep running" not in plain_q and "terminated" in plain_q, plain_q[-1500:]
        assert not (off / "not-quit.txt").exists(), "the plain quit did not quit"
        assert "kept = true" not in (off / "data/rift/session.toml").read_text()
    finally:
        for pid in list(session_daemons(kq / "data")) + list(session_daemons(scratch / "keep-quit-off/data")):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

# (f5) crash recovery: every ordinary way out removes <data dir>/running; a
# leftover one whose process is gone says the last run crashed, and the next
# launch restores the tabs and reattaches their shells (restore_session is off)
# with a notice. RIFT_CRASH_CHECK asks a scripted run to do it.
cr = scratch / "crash-recovery"
for sub in ("cfg/rift", "data/rift"):
    (cr / sub).mkdir(parents=True, exist_ok=True)
(cr / "cfg/rift/config.toml").write_text(
    'durable_sessions = true\nrestore_session = false\nremote_access = false\n'
    'check_for_updates = false\nshell = "/bin/bash"\n')
cr_env = dict(env, XDG_CONFIG_HOME=str(cr / "cfg"), XDG_DATA_HOME=str(cr / "data"),
              RIFT_PERSIST_SESSION="1", RIFT_CRASH_CHECK="1")
def cr_run(name, steps, wait=True):
    (cr / f"{name}.rift").write_text("\n".join(steps) + "\n")
    log = (cr / f"{name}.log").open("w")
    proc = subprocess.Popen([str(binary), "--script", str(cr / f"{name}.rift"), "--headless"],
                            env=cr_env, stdout=log, stderr=log)
    if wait:
        assert proc.wait(timeout=90) == 0, died(f"crash-recovery {name} run failed", cr / f"{name}.log")
    return proc
try:
    proc = cr_run("a", ["wait_prompt", "type sleep 600\\n", "wait 600", f"dump {cr}/a.txt", "wait 60000"], wait=False)
    deadline = time.monotonic() + 30
    while not (cr / "a.txt").exists() and time.monotonic() < deadline:
        time.sleep(0.2)
    marker = cr / "data/rift/running"
    assert marker.read_text().strip() == str(proc.pid), "the run did not mark itself running"
    proc.kill()  # SIGKILL: no exit hook runs
    proc.wait()
    time.sleep(1)
    assert marker.exists() and len(list(session_daemons(cr / "data"))) == 1
    cr_run("b", ["wait 2500", f"dump {cr}/b.txt", "quit"])
    b = (cr / "b.txt").read_text()
    assert "sleep 600" in b and re.search(r"service=Some\(\d+\)", b), b[-1500:]
    assert "quit normally" in b, b[-1500:]
    assert not marker.exists(), "a clean quit left the marker"
    for pid in session_daemons(cr / "data"):
        os.kill(pid, signal.SIGTERM)
    time.sleep(1)
    cr_run("c", ["wait_prompt", "type echo hi\\n", "wait 500", "quit"])
    cr_run("d", ["wait_prompt", f"dump {cr}/d.txt", "quit"])
    d = (cr / "d.txt").read_text()
    assert "quit normally" not in d and "echo hi" not in d, d[-800:]
finally:
    for pid in session_daemons(cr / "data"):
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass

# (g) a relay (Phase 2): the managed gateway registers at a local relay
# (rift-relay) and Settings shows it connected; a phone forced through the
# relay shows "via relay"; Operate -> View only on the page closes it within
# a second and its next handshake gets view (run refused); Recent
# connections lists it via Relay. Dark and light screenshots of the page.
relay_bin = os.environ.get("RIFT_RELAY_BIN")
assert relay_bin, "RIFT_RELAY_BIN is unset (scripts/ui-smoke.sh builds rift-relay)"
rl = rm / "relay"
for sub in ("cfg/rift", "data/rift"):
    (rl / sub).mkdir(parents=True)
with socket.socket() as s:
    s.bind(("127.0.0.1", 0))
    relay_port = s.getsockname()[1]
relay_config = (remote_base + "remote_access = true\n"
                f'remote_relay = "ws://127.0.0.1:{relay_port}"\n')
(rl / "cfg/rift/config.toml").write_text(relay_config)
(rl / "light.toml").write_text(relay_config + 'theme = "light-default"\n')
(rl / "off.toml").write_text(remote_base)
(rl / "t.rift").write_text("\n".join([
    "resize 1100 1400", "wait_prompt",
    f"wait_file {rl}/go-settings 60000", *settings_remote, "wait 1200",
    f"dump {rl}/connected.txt", f"shot {rl}/relay-dark.png",
    # reading order: ↺ and switch, Pair a phone, The new phone can only
    # watch, then the device's Can run commands / View only
    *["key tab"] * 6, "wait 200", f"dump {rl}/focus.txt",
    "key space", "wait 2500", f"dump {rl}/changed.txt",
    f"wait_file {rl}/go-light 60000", cp(rl / "light.toml", rl / "cfg/rift/config.toml"),
    "wait 2000", f"dump {rl}/history.txt", f"shot {rl}/relay-light.png",
    f"wait_file {rl}/go-quit 60000", "quit"]) + "\n")
rl_env = dict(env, XDG_CONFIG_HOME=str(rl / "cfg"), XDG_DATA_HOME=str(rl / "data"),
              BASH_SILENCE_DEPRECATION_WARNING="1")
rl_status = rl / "data/rift/remote/status.json"

def relay_status():
    try:
        return json.loads(rl_status.read_text())
    except (OSError, ValueError):
        return {}

def wait_path(path, what, proc, log, seconds=40):
    deadline = time.monotonic() + seconds
    while not path.exists():
        assert proc.poll() is None, died(f"{what}: window exited", log)
        assert time.monotonic() < deadline, f"timed out waiting for {what}"
        time.sleep(0.05)

relay = window = None
with (rl / "relay.txt").open("w") as relay_log, (rl / "log.txt").open("w") as log, \
        (rl / "phone-1.txt").open("w") as p1log, (rl / "phone-2.txt").open("w") as p2log:
    relay = subprocess.Popen([relay_bin, "--listen", f"127.0.0.1:{relay_port}"],
                             stdout=relay_log, stderr=relay_log)
    window = subprocess.Popen([str(binary), "--script", str(rl / "t.rift"), "--headless"],
                              env=rl_env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 40
        while (relay_status().get("relay") or {}).get("state") != "connected":
            assert window.poll() is None, died("relay window exited", rl / "log.txt")
            assert relay.poll() is None, died("the relay exited", rl / "relay.txt")
            assert time.monotonic() < deadline, f"never registered at the relay: {relay_status()}"
            time.sleep(0.05)
        link = next(l for l in subprocess.run(
            [str(binary), "remote", "pair"], env=rl_env, capture_output=True, text=True,
            timeout=20).stdout.splitlines() if l.startswith("rift-remote://"))
        assert link.endswith(f"&r=ws%3A%2F%2F127.0.0.1%3A{relay_port}"), link
        acked = rl / "phone-acked"
        phone1 = subprocess.Popen([phone_bin, "--relay-only", str(rl / "phone.key"), link,
                                   "true", "30", str(acked)], stdout=p1log, stderr=p1log)
        while not acked.exists():
            assert phone1.poll() is None, died("the relay phone gave up", rl / "phone-1.txt")
            time.sleep(0.02)
        assert [c["via"] for c in relay_status()["clients"]] == ["relay"], relay_status()
        (rl / "go-settings").touch()
        wait_path(rl / "changed.txt", "the permission change", window, rl / "log.txt")
        changed_at = time.time()
        # closed by the gateway (not by its own 30 s timer)
        assert phone1.wait(timeout=10) == 1, died("phone 1 was not closed", rl / "phone-1.txt")
        closed_at = time.time()
        phone2 = subprocess.run([phone_bin, "--relay-only", str(rl / "phone.key"), link,
                                 "echo must-not-run", "1"], stdout=p2log, stderr=p2log,
                                timeout=30)
        assert phone2.returncode == 3, died("phone 2 was not refused", rl / "phone-2.txt")
        (rl / "go-light").touch()
        wait_path(rl / "history.txt", "the light dump", window, rl / "log.txt")
        (rl / "go-quit").touch()
        assert window.wait(timeout=60) == 0, died("relay window failed", rl / "log.txt")
    finally:
        if window.poll() is None:
            window.kill()
        if relay.poll() is None:
            relay.terminate()
            relay.wait(timeout=10)
# the managed gateway follows remote_access = false
gw_pid = relay_status().get("pid")
(rl / "cfg/rift/config.toml").write_text(remote_base)
deadline = time.monotonic() + 10
while rl_status.exists():
    if time.monotonic() > deadline:
        own = subprocess.run(["ps", "eww", "-p", str(gw_pid)], capture_output=True, text=True).stdout
        if gw_pid and str(rl / "data") in own:
            os.kill(int(gw_pid), 15)
        raise AssertionError("the relay test's managed gateway did not exit")
    time.sleep(0.1)

def relay_line(name):
    text = (rl / name).read_text()
    return next(l for l in text.splitlines() if l.startswith("remote_relay=")), text

line, text = relay_line("connected.txt")
assert 'relay_line="Phones away from home connect through it' in line, line
assert 'clients_via=["Live Check Phone:relay"]' in line, line
assert 'chip_tip=Some("Connected: Live Check Phone via relay")' in line, line
assert 'device_access=["Live Check Phone=operate"]' in line, line
assert "Connected through the relay" in remote_line(text), remote_line(text)
# the card's tiles: Away from home (the relay's host, its round trip) and
# On this Wi-Fi; the relay field sits under the collapsed "Connection
# settings"
assert re.search(r'Label "Away from home \(127\.0\.0\.1\): Phones away from home connect '
                 r'through it( · \d+[^"]*ms)?"', text), text[-4000:]
assert 'Label "On this Wi-Fi (127.0.0.1:' in text, text[-4000:]
assert 'Button "Connection settings, collapsed"' in text, text[-4000:]
assert "TextInput" not in text, text[-4000:]
assert 'RadioButton "Live Check Phone access: Can run commands, selected"' in text, text[-4000:]
focus = (rl / "focus.txt").read_text()
assert 'settings_focus=Some(RemotePermission(' in focus and ', true))' in focus, \
    re.search(r"settings_focus=.*", focus)[0]
line, text = relay_line("changed.txt")
assert 'device_access=["Live Check Phone=view"]' in line, line
assert 'toast=Some("Live Check Phone can now only watch")' in text, text[-3000:]
assert closed_at - changed_at < 2.5, (changed_at, closed_at)
p2 = (rl / "phone-2.txt").read_text()
assert '"permission":"view"' in p2 and '"error":"forbidden","id":"live1"' in p2, p2
line, text = relay_line("history.txt")
assert re.search(r'history=\["Live Check Phone \(Relay · 127\.0\.0\.1:%d · [^)]* · \d+s\)'
                 % relay_port, line), line
assert "Live Check Phone (Relay" in line and line.count("Relay · 127.0.0.1") >= 2, line
for shot in ("relay-dark.png", "relay-light.png"):
    assert (rl / shot).read_bytes().startswith(b"\x89PNG\r\n\x1a\n"), shot
assert not re.search(r'cmd="echo must-not-run"', (rl / "history.txt").read_text()), \
    "a view-only phone ran a command"

# (h) a relay that cannot be reached: Connection shows the WHOLE error,
# wrapped, never cut, with Try again; "Relay settings…" opens the relay
# address field
bad = remote_run("relay-error", remote_base + "remote_access = true\n"
                 'remote_relay = "wss://relay.invalid"\ncontrol_api = "off"\n', [
    "resize 1100 1500", "wait 7000", *settings_remote, "wait 800",
    f"shot {rm}/relay-error/error.png", f"dump {rm}/relay-error/error.txt",
    "a11y click Connection settings, collapsed", "wait 400",
    f"shot {rm}/relay-error/error-open.png", f"dump {rm}/relay-error/open.txt"], timeout=120)
error_page = (bad / "error.txt").read_text()
reason = re.search(r'relay_reason=Some\("(.*?)"\) relay_open=', error_page)
assert reason and len(reason[1]) > 20, error_page[:1500]
assert f'. {reason[1]}"' in error_page, ("the error text is cut", reason[1], error_page[-3000:])
assert 'Label "Away from home (relay.invalid): Can’t reach the relay, so phones away from home ' \
    'can’t connect.' in error_page, error_page[-3000:]
assert 'Button "Try again: retry the relay now"' in error_page, error_page[-3000:]
assert 'Button "Connection settings, collapsed"' in error_page, error_page[-3000:]
open_page = (bad / "open.txt").read_text()
assert "relay_open=true" in open_page and 'TextInput "Relay URL: wss://relay.invalid.' in open_page, \
    open_page[-3000:]
assert 'Button "Connection settings, expanded"' in open_page, open_page[-3000:]
for shot in ("error.png", "error-open.png"):
    assert (bad / shot).read_bytes().startswith(b"\x89PNG\r\n\x1a\n"), shot

# block, tab and pane settings, each applied by a hot reload of config.toml:
# the lone tab hides, close buttons follow tab_close_button, ⌘T lands after
# the current tab, the sticky header and the inactive-pane wash switch off,
# and focus follows the pointer only while focus_follows_mouse is on
bt = scratch / "block-tab"
for sub in ("cfg/rift", "data"):
    (bt / sub).mkdir(parents=True, exist_ok=True)
bt_base = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
           f'sync_os_theme = false\nworking_directory = {json.dumps(str(work))}\n')
bt_configs = {
    "one-tab": "tab_bar_with_one_tab = false\n",
    "placement": "tab_bar_with_one_tab = false\nnew_tab_placement = \"after_current\"\n"
                 "tab_close_button = \"always\"\n",
    "never": "new_tab_placement = \"after_current\"\ntab_close_button = \"never\"\n",
    "no-sticky": "tab_close_button = \"never\"\nsticky_command_header = false\n",
    "no-dim": "sticky_command_header = false\ndim_inactive_panes = false\n",
    "ffm": "dim_inactive_panes = false\nfocus_follows_mouse = true\n",
}
for name, extra in bt_configs.items():
    (bt / f"{name}.toml").write_text(bt_base + extra)
(bt / "cfg/rift/config.toml").write_text(bt_base)
bt_cfg = bt / "cfg/rift/config.toml"

# polled: the file watcher's latency grows under load; 6 s before moving on
def bt_load(name):
    return [cp(bt / f"{name}.toml", bt_cfg),
            *[step for i in range(12) for step in ("wait 500", f"dump {bt}/{name}-{i}.txt")]]

bt_steps = [
    "resize 1200 800", "wait_prompt", "move 600 600",
    # a long block scrolled past the top pins its header
    "type seq 1 300\\n", "wait 1500", f"dump {bt}/sticky-on.txt",
    f"dump {bt}/default.txt",
    *bt_load("one-tab"),
    "key cmd+t", "wait 1500", f"dump {bt}/two-tabs.txt",
    # back on the first tab: the idle second one has no badge or dot, so
    # "always" gives it a × (the first tab's finished seq may leave a badge)
    "key cmd+1", "wait 300",
    *bt_load("placement"),
    # from the first tab, ⌘T opens right after it
    "key cmd+t", "wait 1500", f"dump {bt}/after-current.txt",
    *bt_load("never"),
    "key cmd+1", "wait 300", f"dump {bt}/sticky-before.txt",
    *bt_load("no-sticky"),
    "key cmd+d", "wait 1500", f"dump {bt}/split-dim.txt",
    # pointer over the left pane with focus_follows_mouse off: no change
    "move 300 400", "wait 300", f"dump {bt}/ffm-off.txt",
    *bt_load("no-dim"),
    *bt_load("ffm"), "move 301 401", "wait 300", f"dump {bt}/ffm-on.txt",
    "move 900 400", "wait 300", f"dump {bt}/ffm-back.txt",
    "quit",
]
(bt / "bt.rift").write_text("\n".join(bt_steps) + "\n")
bt_env = dict(env, XDG_CONFIG_HOME=str(bt / "cfg"), XDG_DATA_HOME=str(bt / "data"))
with (bt / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(bt / "bt.rift"), "--headless"],
                        env=bt_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("block/tab settings run failed", bt / "log.txt")

def bt_state(name):
    return (bt / f"{name}.txt").read_text()

def bt_slots(text):
    return re.search(r"^tab_slots=.*$", text, re.M)[0]

def bt_line(name):
    return bt_slots(bt_state(name))

def bt_first(name, ok):
    """The first of a load's polled dumps that passes `ok` (else the last)."""
    dumps = [bt_state(f"{name}-{i}") for i in range(12)]
    return next((d for d in dumps if ok(d)), dumps[-1])

def bt_polled(name, ok):
    return bt_slots(bt_first(name, ok))

def bt_active_pane(name, tab):
    return re.search(rf"^tab {tab} .*active_pane=(\d+)", bt_state(name), re.M)[1]

assert re.search(r"sticky=Some\(", bt_state("sticky-on")), bt_state("sticky-on")[-2000:]
# hover (the default): × only on the tab under the pointer, which is over
# the pane here, so on none, the active one included
assert bt_line("default").startswith("tab_slots=1 tab_close=[]"), bt_line("default")
# one tab, tab_bar_with_one_tab off: no tab in the strip
one = bt_polled("one-tab", lambda d: bt_slots(d).startswith("tab_slots=0 "))
assert one.startswith("tab_slots=0 tab_close=[]"), one
assert bt_line("two-tabs").startswith('tab_slots=2 tab_close=[]'), bt_line("two-tabs")
always = bt_polled("placement", lambda d: 'tab_close=["0", "1"]' in bt_slots(d))
assert 'tab_slots=2 tab_close=["0", "1"]' in always, always
after = bt_state("after-current")
assert "tabs=3 active=1 " in after, after[:1500]
never = bt_polled("never", lambda d: "tab_close=[]" in bt_slots(d))
assert never.startswith("tab_slots=3 tab_close=[]"), never
assert re.search(r"sticky=Some\(", bt_state("sticky-before")), bt_state("sticky-before")[-2000:]
off = bt_first("no-sticky", lambda d: "sticky=Some" not in d)
assert "sticky=Some" not in off and "sticky=None" in off, off[-2000:]
assert bt_line("split-dim").endswith("dimmed_panes=1"), bt_line("split-dim")
nodim = bt_polled("no-dim", lambda d: bt_slots(d).endswith("dimmed_panes=0"))
assert nodim.endswith("dimmed_panes=0"), nodim
right = bt_active_pane("split-dim", 0)
assert bt_active_pane("ffm-off", 0) == right, "focus moved without focus_follows_mouse"
left = bt_active_pane("ffm-on", 0)
assert left != right, ("focus_follows_mouse did not focus the hovered pane", left, right)
assert bt_active_pane("ffm-back", 0) == right, "focus did not follow back"

# Settings rows: Appearance › Tabs and Panes, Input › Sticky command header
bt_shots = [
    # (tall: Appearance runs to its Tabs group well below 900 px)
    "resize 1200 2400", "wait_prompt", "key cmd+,", "wait 300", "key down", "key down", "wait 400",
    f"dump {bt}/appearance-top.txt", "move 700 500", "scroll -12", "wait 300",
    f"dump {bt}/appearance-mid.txt",
    "move 700 500", "scroll -200", "wait 400", f"shot {bt}/appearance.png",
    f"dump {bt}/appearance.txt",
    "key down", "wait 400", f"shot {bt}/input.png",
    f"dump {bt}/input.txt", "quit",
]
(bt / "cfg/rift/config.toml").write_text(bt_base)
(bt / "shots.rift").write_text("\n".join(bt_shots) + "\n")
with (bt / "shots-log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(bt / "shots.rift"), "--headless"],
                        env=bt_env, stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("block/tab settings shots failed", bt / "shots-log.txt")
# the page runs past the window: its top, then scrolled to its end
appearance = bt_state("appearance-top") + bt_state("appearance-mid") + bt_state("appearance")
for label in ("Tab position", "Close buttons", "New tabs open", "Show tabs with only one tab open",
              "Dim inactive panes", "Dimming strength", "Focus follows mouse"):
    assert label in appearance, (label, appearance[-3000:])
assert "Sticky command header" in bt_state("input"), bt_state("input")[-3000:]

# Warp TOML Tab Configs: a .toml in launch_configs/ is listed in the palette
# (a malformed neighbour is skipped), runs, and opens its tab with the title,
# color, split tree and focused pane; a layout saved from the palette is TOML
lt = scratch / "tab-config"
for sub in ("cfg/rift/launch_configs", "data/rift"):
    (lt / sub).mkdir(parents=True, exist_ok=True)
(lt / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\nsync_os_theme = false\n')
(lt / "cfg/rift/launch_configs/dev_logs.toml").write_text(
    'name = "Dev Logs"\ntitle = "devlogs"\ncolor = "magenta"\nfuture_key = 1\n'
    '[[panes]]\nid = "root"\nsplit = "horizontal"\nchildren = ["server", "right"]\n'
    f'[[panes]]\nid = "server"\ntype = "terminal"\ndirectory = {json.dumps(str(work))}\nis_focused = true\n'
    '[[panes]]\nid = "right"\nsplit = "vertical"\nchildren = ["a", "b"]\n'
    '[[panes]]\nid = "a"\ntype = "terminal"\n[[panes]]\nid = "b"\ntype = "agent"\n')
(lt / "cfg/rift/launch_configs/broken.toml").write_text("name = [\n")
(lt / "t.rift").write_text("\n".join([
    "wait_prompt", "key cmd+p", "type Launch configuration:", "wait 200",
    f"dump {lt}/palette.txt",
    "type  Dev Logs", "wait 200", "key enter", "wait 800", f"dump {lt}/opened.txt",
    "key cmd+p", "type Save current layout as launch", "wait 200", "key enter", "wait 500",
    "quit"]) + "\n")
with (lt / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(lt / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(lt / "cfg"), XDG_DATA_HOME=str(lt / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("tab-config run failed", lt / "app.log")
listed = (lt / "palette.txt").read_text()
assert 'ListBoxOption "Launch configuration: Dev Logs"' in listed, listed[-3000:]
assert "Launch configuration: broken" not in listed, "a malformed tab config was listed"
# ...and named once in a toast with its reason
assert re.search(r"toast=Some\(\"Tab config skipped: broken\.toml \([^)\n]+\)", listed), listed[:1500]
assert (lt / "app.log").read_text().count("Tab config skipped") == 1, "the same broken set toasted again"
opened = (lt / "opened.txt").read_text()
assert "tabs=2 active=1" in opened, opened[:400]
assert 'custom_title=Some("devlogs")' in opened, opened[:900]
assert "panes=3" in opened and "color=Some(4)" in opened, opened[:900]
assert re.search(r"layout=Split \{ axis: Horizontal, children: \[\([\d.]+, Leaf\(2\)\), "
                 r"\([\d.]+, Split \{ axis: Vertical", opened), opened[:900]
assert "active_pane=2" in opened, "is_focused pane not active: " + opened[:900]
saved = sorted((lt / "cfg/rift/launch_configs").glob("layout-*.toml"))
assert len(saved) == 2, saved  # one Tab Config per open tab
assert all(f.read_text().startswith('name = "Layout ') for f in saved), saved
assert 'split = "horizontal"' in saved[1].read_text() and 'color = "magenta"' in saved[1].read_text(), \
    saved[1].read_text()

# Launch profiles (`[[profiles]]`) and a shell per pane: one Tab Config opens
# panes with different shells (a Warp per-pane `shell`, a profile) in
# different directories, each with its own shell integration (OSC 133
# blocks) and the profile's environment in that pane only (not its sibling,
# not the app). The ⌄ beside + (and right-click on +) lists the profiles,
# ↓/Enter opens one, ⌃⇧N opens the Nth; an unknown profile or a missing
# program is a toast and opens nothing.
pf = scratch / "profiles"
for sub in ("cfg/rift/launch_configs", "data/rift", "dir a", 'dir "b"', "dir c"):
    (pf / sub).mkdir(parents=True, exist_ok=True)
pf_shells = [s for s in ("zsh", "bash", "fish") if shutil.which(s)]
assert len(pf_shells) >= 2, f"profiles: need two of zsh/bash/fish, found {pf_shells}"
pf_main, pf_other = pf_shells[0], pf_shells[-1]
(pf / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\nsync_os_theme = false\n'
    'discover_shells = false\n'
    f'[[profiles]]\nname = "smoke"\ncommand = "{pf_other}"\ncwd = {json.dumps(str(pf / "dir c"))}\n'
    'title = "Smoke tab"\ncolor = "green"\n[profiles.env]\nRIFT_SMOKE_X = "only-here"\n'
    '[[profiles]]\nname = "gone"\ncommand = "/nonexistent/rift-shell"\n')
pf_cmd = lambda name: [f'echo "[$RIFT_SMOKE_X]" > {shlex.quote(str(pf / name))}']
(pf / "cfg/rift/launch_configs/shells.toml").write_text(
    'name = "Shells"\n'
    '[[panes]]\nid = "root"\nsplit = "horizontal"\nchildren = ["a", "b"]\n'
    f'[[panes]]\nid = "a"\ntype = "terminal"\ndirectory = {json.dumps(str(pf / "dir a"))}\n'
    f'shell = "{pf_main}"\ncommands = {json.dumps(pf_cmd("a.txt"))}\n'
    f'[[panes]]\nid = "b"\ntype = "terminal"\nprofile = "smoke"\nis_focused = true\n'
    f'commands = {json.dumps(pf_cmd("b.txt"))}\n')
(pf / "cfg/rift/launch_configs/unknown.toml").write_text(
    'name = "Unknown"\n[[panes]]\nid = "a"\ntype = "terminal"\nprofile = "nope"\n')
pf_env = dict(env, XDG_CONFIG_HOME=str(pf / "cfg"), XDG_DATA_HOME=str(pf / "data"))
pf_env.pop("RIFT_SMOKE_X", None)

def pf_run(name, steps):
    (pf / f"{name}.rift").write_text("\n".join(["resize 1200 800", "wait_prompt", *steps, "quit"]) + "\n")
    with (pf / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(pf / f"{name}.rift"), "--headless"],
                            env=pf_env, stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"profiles ({name}) run failed", pf / f"{name}.log")

def pf_state(name):
    return (pf / f"{name}.txt").read_text()

pf_run("shells", [
    "key cmd+p", "type Launch configuration: Shells", "wait 300", "key enter",
    f"wait_file {pf / 'b.txt'} 20000", f"wait_file {pf / 'a.txt'} 20000", "wait 500",
    f"dump {pf}/shells.txt",
    f'sh echo "[$RIFT_SMOKE_X]" > {shlex.quote(str(pf / "app.txt"))}',
    "key cmd+p", "type Launch configuration: Unknown", "wait 300", "key enter", "wait 300",
    f"dump {pf}/unknown.txt"])
shells = pf_state("shells")
pf_tab = shells[shells.index("\ntab 1 "):]
pf_panes = re.findall(r'\n  pane \d+ cwd=Some\("([^"\n]*(?:\\"[^"\n]*)*)"\).*\n'
                      r'    launch profile=(\S+) shell=(\S+) args=\[\] program="([^"]+)"', pf_tab)
assert len(pf_panes) == 2, pf_tab[:1500]
(a_cwd, a_prof, a_shell, a_prog), (b_cwd, b_prof, b_shell, b_prog) = pf_panes
assert a_cwd.endswith("dir a") and a_shell == f'Some("{pf_main}")' and a_prog.endswith("/" + pf_main), pf_panes
assert b_cwd.endswith("dir c") and b_prof == 'Some("smoke")' and b_prog.endswith("/" + pf_other), pf_panes
# each pane's own hooks report blocks (OSC 133): its launch command finished
# with an exit status, and a new prompt followed
for pane in re.split(r"\n  pane \d+ ", pf_tab)[1:]:
    assert re.search(r"block \d+ state=done .* exit=Some\(0\)", pane), pane[:2500]
    assert re.search(r"block \d+ state=prompt", pane), pane[:2500]
assert (pf / "b.txt").read_text() == "[only-here]\n", "the profile's env did not reach its pane"
assert (pf / "a.txt").read_text() == "[]\n", "the profile's env leaked into a sibling pane"
assert (pf / "app.txt").read_text() == "[]\n", "the profile's env leaked into the app process"
unknown = pf_state("unknown")
assert 'toast=Some("Launch configuration Unknown: No profile named \\"nope\\"")' in unknown, unknown[:800]
assert "tabs=2 " in unknown, "a launch configuration with an unknown profile opened a tab"

# the menu: ⌄ opens it under the button, right-click + too; ↓ Enter opens the
# first profile, ⌃⇧2 the second (a missing program: a toast, no tab)
pf_run("probe", [f"dump {pf}/probe.txt"])
pf_tips = dict((m[0], [float(v) for v in m[1:]]) for m in
               re.findall(r'"([^"@]+)@([\d.]+),([\d.]+),([\d.]+),([\d.]+)"', pf_state("probe")))
pf_plus = next(r for k, r in pf_tips.items() if k == "New tab")
pf_chev = pf_tips["Open a profile"]
pf_at = lambda r: f"{r[0] + r[2] / 2:.0f} {r[1] + r[3] / 2:.0f}"
pf_run("menu", [
    f"rclick {pf_at(pf_plus)}", "wait 300", f"dump {pf}/rclick.txt", "key escape", "wait 200",
    f"click {pf_at(pf_chev)}", "wait 300", f"shot {pf}/menu.png", f"dump {pf}/menu.txt",
    "key down", "key enter", "wait_prompt", "wait 300", f"dump {pf}/opened.txt",
    "key ctrl+shift+2", "wait 300", f"dump {pf}/gone.txt"])
menu = pf_state("menu")
assert 'menu_items=["smoke", "gone", "Settings", "Command Palette", "About Rift"]' in menu, menu[:1500]
for row in ("smoke", "gone", "Settings", "Command Palette", "About Rift"):
    assert f'MenuItem "{row}"' in menu, (row, menu[-2500:])
opened = pf_state("opened")
assert "tabs=2 active=1" in opened and 'custom_title=Some("Smoke tab")' in opened, opened[:1200]
assert 'launch profile=Some("smoke")' in opened and "color=Some(1)" in opened, opened[:1500]
gone = pf_state("gone")
assert 'toast=Some("Profile \\"gone\\": /nonexistent/rift-shell is not an executable")' in gone, gone[:800]
assert "tabs=2 " in gone, "a profile whose program is missing opened a tab"
assert 'menu_items=["smoke", "gone",' in pf_state("rclick"), pf_state("rclick")[:1500]

# default_profile: every tab that opens without a profile picker takes the
# profile's directory, title and color (the first tab, a link, a new window);
# a link's folder still wins, and Duplicate tab keeps its source's folder
pfd = pf / "default"
(pfd / "cfg/rift").mkdir(parents=True)
(pfd / "cfg/rift/config.toml").write_text(
    (pf / "cfg/rift/config.toml").read_text().replace(
        'discover_shells = false\n', 'discover_shells = false\ndefault_profile = "smoke"\n'))
(pfd / "t.rift").write_text("\n".join([
    "resize 1200 800", "wait_prompt", f"dump {pfd}/first.txt",
    "link rift://action/new_tab", "wait_prompt",
    f"link rift://action/new_tab?path={pf / 'dir a'}", "wait_prompt",
    # the third tab (dir a): right-click → Duplicate tab
    "rclick 600 17", "wait 300", "key down", "key down", "key enter", "wait_prompt", "wait 300",
    f"dump {pfd}/tabs.txt",
    # the palette's profile rows show ⌃⇧N as the ⌄ menu does (tab rows only)
    "key cmd+p", "type smoke", "wait 300", f"dump {pfd}/palette.txt", "key escape", "wait 200",
    "link rift://action/new_window", "wait_prompt", "wait 300", f"dump {pfd}/window.txt", "quit"]) + "\n")
with (pfd / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(pfd / "t.rift"), "--headless"],
                        env=dict(pf_env, XDG_CONFIG_HOME=str(pfd / "cfg"),
                                 XDG_DATA_HOME=str(pfd / "data")),
                        stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("default_profile run failed", pfd / "log.txt")
pfd_tabs = lambda name: re.findall(
    r'\ntab \d+ .*color=(\S+) .*custom_title=(Some\("[^"]*"\)|None).*\n(?:.*\n)*?  pane \d+ cwd=Some\("([^"]*)"\)',
    (pfd / f"{name}.txt").read_text())
smoke_tab = ("Some(1)", 'Some("Smoke tab")')
first = pfd_tabs("first")
assert len(first) == 1 and first[0][:2] == smoke_tab and first[0][2].endswith("/dir c"), first
tabs = pfd_tabs("tabs")
assert [t[:2] for t in tabs] == [smoke_tab] * 4, tabs
assert [t[2].rsplit("/", 1)[-1] for t in tabs] == ["dir c", "dir c", "dir a", "dir a"], tabs
pfd_pal = (pfd / "palette.txt").read_text()
assert 'ListBoxOption "New tab: smoke, ⌃ ⇧ 1"' in pfd_pal, pfd_pal[-2500:]
assert re.search(r'ListBoxOption "Split right: smoke"( |$)', pfd_pal, re.M), pfd_pal[-2500:]
win = (pfd / "window.txt").read_text()
assert re.search(r"^windows=2 ", win, re.M), win[:600]
assert pfd_tabs("window")[-1][:2] == smoke_tab and pfd_tabs("window")[-1][2].endswith("/dir c"), \
    pfd_tabs("window")

# completions from the user's own shell (`shell_completions`): a completion
# function in a scratch ~/.bashrc answers for a tool Rift has no spec for;
# one that hangs (for git) times out and Rift's own git candidates still
# show, first while the shell is asked and after it gave up (the menu opens
# as you type here, so the first answer shows); with the setting off the
# shell is never asked
sc = scratch / "shell-completions"
(sc / "home").mkdir(parents=True, exist_ok=True)
(sc / "home/.bashrc").write_text(
    '_mycmd() { COMPREPLY=($(compgen -W "deploy destroy status" -- "$2")); }\n'
    'complete -F _mycmd mycmd\n'
    '_hang() { sleep 30; COMPREPLY=(never); }\ncomplete -F _hang git\n')
(sc / "home/.bash_profile").write_text(". ~/.bashrc\n")

def run_shell_completions(name, extra):
    (sc / name / "cfg/rift").mkdir(parents=True, exist_ok=True)
    (sc / name / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        'completions_open_while_typing = true\n'
        f'working_directory = {json.dumps(str(work))}\n' + extra)
    out = sc / name
    (out / "t.rift").write_text("\n".join([
        "wait_prompt", "type mycmd de", "wait 2000", f"dump {out}/mycmd.txt",
        "key cmd+t", "wait_prompt", "type git chec", "wait 100", f"dump {out}/git-early.txt",
        "wait 1500", f"dump {out}/git.txt", "quit"]) + "\n")
    with (out / "app.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(out / "t.rift"), "--headless"],
                            env=dict(env, HOME=str(sc / "home"), XDG_CONFIG_HOME=str(out / "cfg"),
                                     XDG_DATA_HOME=str(out / "data")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"shell completions ({name}) run failed", out / "app.log")
    items = lambda f: next(l for l in (out / f).read_text().splitlines()
                           if l.startswith("completion_items="))
    return items("mycmd.txt"), items("git-early.txt"), items("git.txt")

mycmd, git_early, git_late = run_shell_completions("on", "")
assert '"deploy"(shell) "destroy"(shell)' in mycmd, mycmd
for got in (git_early, git_late):
    assert '"checkout"' in got and "never" not in got, got
mycmd, _, git_late = run_shell_completions("off", "shell_completions = false\n")
assert "deploy" not in mycmd, mycmd
assert '"checkout"' in git_late, git_late

# One process, many windows: ⌘N opens a window in this process, its tab in
# the current pane's folder; each window takes its own typing; the control
# API lists both (tabs numbered across windows) and `focus --tab` raises the
# other window; the switcher lists the other window's tab; the session file
# holds both and a launch restores both; closing one leaves the other.
mw = scratch / "windows"
for sub in ("cfg/rift", "data/rift", "b"):
    (mw / sub).mkdir(parents=True, exist_ok=True)
(mw / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\nsync_os_theme = false\n')
mw_env = dict(env, XDG_CONFIG_HOME=str(mw / "cfg"), XDG_DATA_HOME=str(mw / "data"),
              RIFT_PERSIST_SESSION="1", BASH_SILENCE_DEPRECATION_WARNING="1")
def ctl_bg(name, request):
    # answered on the UI thread, which runs the script: in the background
    out = mw / f"{name}.json"
    return [f"sh ({shlex.quote(str(binary))} ctl request {shlex.quote(request)} > {shlex.quote(str(out))} "
            f"2>&1; touch {shlex.quote(str(out))}.done) &", f"wait_file {out}.done 15000", "wait 300"]
(mw / "t.rift").write_text("\n".join([
    "wait_prompt", f"type cd {shlex.quote(str(mw / 'b'))}\\n", "wait 500",
    "key cmd+n", "wait_prompt", "wait 300", "type echo second-window\\n", "wait 500",
    "window 0", "type echo first-window\\n", "wait 500", f"dump {mw}/w0.txt",
    "window 1", f"dump {mw}/w1.txt",
    *ctl_bg("status", '{"cmd":"status"}'),
    "window 0", *ctl_bg("focus", '{"cmd":"focus","tab":1}'), f"dump {mw}/focused.txt",
    "key shift+cmd+o", "wait 400", f"dump {mw}/switcher.txt", "key escape",
    f"sh cp {shlex.quote(str(mw / 'data/rift/session.toml'))} {shlex.quote(str(mw / 'both.toml'))}",
    "window 1", "close_window", "wait 400", f"dump {mw}/closed.txt", "quit"]) + "\n")
with (mw / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(mw / "t.rift"), "--headless"],
                        env=mw_env, stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("two-window run failed", mw / "app.log")
w0, w1 = ((mw / f"w{i}.txt").read_text() for i in range(2))
for d in (w0, w1):
    assert d.startswith("windows=2 "), d[:400]
    assert re.search(r"^window 0: tabs=1 .*\n^window 1: tabs=1 ", d, re.M), d[:400]
assert 'cmd="echo first-window"' in w0 and 'cmd="echo second-window"' not in w0, w0[:3000]
assert 'cmd="echo second-window"' in w1 and 'cmd="echo first-window"' not in w1, w1[:3000]
# the new window's tab starts where the pane it was opened from was
assert re.search(r"^  pane \d+ cwd=Some\(\"[^\"]*windows/b\"\)", w1, re.M), w1[:3000]
status = json.loads((mw / "status.json").read_text())
assert [(t["index"], t.get("window", 0)) for t in status["tabs"]] == [(0, 0), (1, 1)], status
assert (mw / "focused.txt").read_text().startswith("windows=2 window=1 focused_window=Some(1)"), \
    (mw / "focused.txt").read_text()[:300]
# (opened in the second window, which `focus` raised: it lists the first,
# "window 1" to a person)
switcher = (mw / "switcher.txt").read_text()
# (the tab title is the folder name, or the shell's own `user@host: path`
# title where bash's default prompt sets one, as on the Linux CI image)
assert re.search(r'ListBoxOption "(?:[^"]*[ /:])?b — window 1"', switcher), switcher[-3000:]
both = (mw / "both.toml").read_text()
# windows are saved from schema version 4 on (later versions add fields)
both_ver = re.match(r"version = (\d+)\n", both)
assert both_ver and int(both_ver[1]) >= 4 and both.count("[[other_windows]]") == 1, both
closed = (mw / "closed.txt").read_text()
assert closed.startswith("windows=1 window=0 "), closed[:400]
assert 'cmd="echo first-window"' in closed, closed[:3000]
# a launch restores both windows from that file
(mw / "data/rift/session.toml").write_text(both)
(mw / "restore.rift").write_text("\n".join([
    "wait_prompt", "wait 300", f"dump {mw}/r0.txt", "window 1", "wait_prompt", f"dump {mw}/r1.txt",
    "quit"]) + "\n")
with (mw / "restore.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(mw / "restore.rift"), "--headless"],
                        env=mw_env, stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("two-window restore failed", mw / "restore.log")
r1 = (mw / "r1.txt").read_text()
assert r1.startswith("windows=2 window=1 "), r1[:400]
assert re.search(r"^  pane \d+ cwd=Some\(\"[^\"]*windows/b\"\)", r1, re.M), r1[:3000]
# an update's restart: this process's own restart file (both windows) and
# another process's (a version that ran a process per window) reopen as
# three windows of the one new process, and both files go
rs = mw / "data/rift/restart"
rs.mkdir(parents=True, exist_ok=True)
(rs / "main-4242.toml").write_text(both)
(rs / "4343.toml").write_text('[[tabs]]\ncwd = "/tmp"\ntitle = "OTHER-PROCESS"\n')
(mw / "restart.rift").write_text("\n".join([
    "wait_prompt", "wait 1500", "window 2", "wait_prompt", f"dump {mw}/restarted.txt", "quit"]) + "\n")
with (mw / "restart.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(mw / "restart.rift"), "--headless"],
                        env=dict(mw_env, RIFT_RESTORE_SESSION=str(rs / "main-4242.toml"),
                                 RIFT_RESTART_OTHERS="1"),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("restart reopen failed", mw / "restart.log")
restarted = (mw / "restarted.txt").read_text()
assert restarted.startswith("windows=3 window=2 "), restarted[:500]
assert 'titles=["OTHER-PROCESS"]' in restarted, restarted[:500]
assert not list(rs.glob("*.toml")), list(rs.glob("*"))

# Pinned tabs and tab groups: a pinned tab moves first and ⌘W asks before
# closing it; ⌘-click picks tabs and "Group picked tabs" groups them (never
# the pinned one); a click on the group's pill folds it (⌘1–9 count the
# shown tabs), a double-click renames it; the session file keeps pins and
# groups and a launch restores them; a tab moved to another window leaves
# its group and a pinned one stays pinned; a new tab from a grouped one
# joins the group. The vertical rail: its search filters (a match, no match,
# Esc), its panes view lists a split tab's panes with their repository's
# `+added −removed` (as `git diff --shortstat` counts them) and a pane row
# click focuses that pane; Enter in an empty search only lets go. A screen
# reader gets the search field (focused while it has the keys), the panes
# toggle and the pane rows, and the tab list follows the search; tab labels
# say pinned and read-only. With mask_secrets, a secret in a pane's title is
# masked in its row and a search does not find it.
tg = scratch / "tabgroups"
for sub in ("cfg/rift", "data/rift", "repo", "home"):
    (tg / sub).mkdir(parents=True, exist_ok=True)
# an empty home: Ubuntu's stock ~/.bashrc retitles the window `user@host: cwd`
# at every prompt (PS1), over the title the rail's secret test sets
for rc in (".bash_profile", ".bashrc"):
    (tg / "home" / rc).write_text("")
tg_config = ('shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
             'sync_os_theme = false\n')
(tg / "cfg/rift/config.toml").write_text(tg_config)
tg_env = dict(env, XDG_CONFIG_HOME=str(tg / "cfg"), XDG_DATA_HOME=str(tg / "data"),
              RIFT_PERSIST_SESSION="1", BASH_SILENCE_DEPRECATION_WARNING="1",
              HOME=str(tg / "home"))
def tg_run(name, steps):
    (tg / f"{name}.rift").write_text("\n".join(steps) + "\n")
    with (tg / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(tg / f"{name}.rift"), "--headless"],
                            env=tg_env, stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"tab groups {name} run failed", tg / f"{name}.log")
def tg_state(name):
    return (tg / f"{name}.txt").read_text()
def tg_marks(name):
    return re.findall(r"^  tab_marks pinned=(\w+) group=(.*?) picked=(\w+)$", tg_state(name), re.M)
tg_run("groups", [
    "wait_prompt", "key cmd+t", "wait_prompt", "key cmd+t", "wait_prompt", "key cmd+t",
    "wait_prompt", "key cmd+p", "type pin or unpin", "wait 200", "key enter", "wait 300",
    f"dump {tg}/pinned.txt",
    "key cmd+w", "wait 300", f"dump {tg}/pin-confirm.txt", "key escape", "wait 200",
    # pinned 114..158, then 200 px tabs
    "mods cmd", "click 250 17", "click 450 17", "mods none", "wait 200",
    f"dump {tg}/picked.txt",
    "rclick 450 17", "wait 200", "a11y click Group picked tabs", "wait 300",
    f"dump {tg}/grouped.txt",
    "click 190 17", "wait 300", f"dump {tg}/collapsed.txt",
    "key cmd+2", "wait 200", f"dump {tg}/cmd2.txt",
    "key cmd+1", "dblclick 190 17", "wait 200", "type Servers", "key enter", "wait 300",
    f"dump {tg}/renamed.txt",
    f"sh cp {shlex.quote(str(tg / 'data/rift/session.toml'))} {shlex.quote(str(tg / 'saved.toml'))}",
    "quit"])
assert tg_marks("pinned")[0] == ("true", "None", "false"), tg_state("pinned")
assert re.search(r'^    Tab "[^"]*, pinned"', tg_state("pinned"), re.M), tg_state("pinned")[-3000:]
assert "confirm=Some((ClosePinned(" in tg_state("pin-confirm"), tg_state("pin-confirm")
assert [m[2] for m in tg_marks("picked")] == ["true", "true", "true", "false"], tg_state("picked")
assert 'groups=["Group 1:red:open:2"]' in tg_state("grouped"), tg_state("grouped")
assert [m[1] for m in tg_marks("grouped")] == ["None", 'Some("Group 1")', 'Some("Group 1")', "None"], \
    tg_state("grouped")
assert 'groups=["Group 1:red:collapsed:2"]' in tg_state("collapsed") \
    and "shown_tabs=[0, 3]" in tg_state("collapsed"), tg_state("collapsed")
# ⌘2: the second shown tab (the folded ones skipped)
assert re.search(r"^window 0: tabs=4 active=3 ", tg_state("cmd2"), re.M), tg_state("cmd2")
renamed = tg_state("renamed")
assert 'groups=["Servers:red:collapsed:2"]' in renamed, renamed
assert 'Group "Servers group, 2 tabs, collapsed" expanded=false' in renamed, renamed
saved = (tg / "saved.toml").read_text()
assert "pinned = true" in saved and 'name = "Servers"' in saved and "collapsed = true" in saved, saved
(tg / "data/rift/session.toml").write_text(saved)
tg_run("restore", ["wait_prompt", "wait 800", f"dump {tg}/restored.txt", "quit"])
restored = tg_state("restored")
assert 'groups=["Servers:red:collapsed:2"]' in restored and "shown_tabs=[0, 3]" in restored, restored
assert tg_marks("restored")[0][0] == "true", restored
(tg / "cfg/rift/config.toml").write_text(tg_config.replace("restore_session = true",
                                                           "restore_session = false"))
tg_run("move", [
    "wait_prompt", "key cmd+t", "wait_prompt", "key cmd+t", "wait_prompt",
    "rclick 450 17", "wait 200", "a11y click Group tab", "wait 200", "click 400 17",
    "wait 300", "key cmd+t", "wait_prompt", "wait 300", f"dump {tg}/joined.txt",
    "key cmd+1", "key cmd+p", "type pin or unpin", "wait 200", "key enter", "wait 300",
    "drag_tab 0 0 new", "wait 600", f"dump {tg}/pin-moved.txt",
    "window 0", "drag_tab 0 0 new", "wait 600", f"dump {tg}/group-moved.txt", "quit"])
assert [m[1] for m in tg_marks("joined")] == ["None", 'Some("Group 1")', 'Some("Group 1")', "None"], \
    tg_state("joined")
assert tg_marks("pin-moved") == [("true", "None", "false")], tg_state("pin-moved")
assert tg_marks("group-moved") == [("false", "None", "false")], tg_state("group-moved")
# the rail: search, no match, Esc, panes view with diff stats, a pane row
repo = tg / "repo"
subprocess.run(["git", "init", "-q", str(repo)], check=True)
(repo / "f.txt").write_text("a\nb\nc\n")
subprocess.run(["git", "-C", str(repo), "add", "f.txt"], check=True)
subprocess.run(["git", "-C", str(repo), "-c", "user.email=t@t", "-c", "user.name=t",
                "commit", "-qm", "init"], check=True)
(repo / "f.txt").write_text("a\nB\nc\nd\n")
short = subprocess.run(["git", "-C", str(repo), "diff", "--shortstat"], capture_output=True,
                       text=True, check=True).stdout
added, removed = re.search(r"(\d+) insertions?\(\+\), (\d+) deletions?", short).groups()
(tg / "cfg/rift/config.toml").write_text(
    tg_config.replace("restore_session = true", "restore_session = false")
    + 'tab_layout = "vertical"\nmask_secrets = true\n')
tg_run("rail", [
    "wait_prompt", f"type cd {shlex.quote(str(repo))}\\n", "wait 1200",
    "key cmd+t", "wait_prompt", "type echo zebra-needle\\n", "wait 400",
    "key cmd+d", "wait_prompt", "type cd /tmp\\n", "wait 600",
    # a fake token (never issued) in the pane's title, built by printf
    "type printf '\\033]2;GH=ghp_%s%s%s%s%s\\007' FAKE0123 FAKE0123 FAKE0123 FAKE0123 FAKE0123\\n",
    "wait 400",
    "click 80 55", "type zebra", "wait 200", f"dump {tg}/rail-match.txt",
    *["key backspace"] * 5, "type qqqxx", "wait 200", f"dump {tg}/rail-none.txt",
    "key escape", "wait 200", f"dump {tg}/rail-clear.txt",
    "click 80 55", "key enter", "wait 200", f"dump {tg}/rail-empty-enter.txt",
    "click 199 55", "wait 300", f"dump {tg}/rail-panes.txt",
    "click 100 196", "wait 200", f"dump {tg}/rail-pane-click.txt",
    "click 80 55", "type 0123FAKE", "wait 200", f"dump {tg}/rail-secret.txt", "quit"])
rail_line = lambda name: re.search(r"^rail (.*)$", tg_state(name), re.M).group(1)
assert rail_line("rail-match") == 'query="zebra" focused=true panes=false rows=["tab:1"]', \
    rail_line("rail-match")
assert rail_line("rail-none").endswith('rows=["nomatch"]'), rail_line("rail-none")
assert rail_line("rail-clear") == 'query="" focused=false panes=false rows=["tab:0", "tab:1"]', \
    rail_line("rail-clear")
assert rail_line("rail-panes").endswith(
    f'rows=["tab:0", "tab:1", "pane:2:+{added}-{removed}", "pane:3:active"]'), rail_line("rail-panes")
assert re.search(r"^tab 1 .* active_pane=2 ", tg_state("rail-pane-click"), re.M), \
    tg_state("rail-pane-click")
assert rail_line("rail-empty-enter") == 'query="" focused=false panes=false rows=["tab:0", "tab:1"]' \
    and re.search(r"^window 0: tabs=2 active=1 ", tg_state("rail-empty-enter"), re.M), \
    tg_state("rail-empty-enter")[:1500]
def tg_tablist(name):
    text = tg_state(name)
    return re.findall(r"^    Tab (.*)$", text.split('  TabList "Tabs"\n', 1)[1].split("\n  Group", 1)[0], re.M)
assert len(tg_tablist("rail-match")) == 1 and 'SearchInput "Search tabs" value="zebra" [focus]' \
    in tg_state("rail-match"), tg_state("rail-match")[-4000:]
assert '[focus]' not in tg_state("rail-match").split('TextInput "Command"', 1)[-1].split("\n", 1)[0]
assert len(tg_tablist("rail-clear")) == 2, tg_tablist("rail-clear")
rp = tg_state("rail-panes")
assert re.search(r'Button "Show panes under their tabs" toggled=True', rp), rp[-4000:]
assert re.search(rf'ListItem ".*, pane, {added} lines added, {removed} removed"', rp), rp[-4000:]
assert re.search(r'ListItem "GH=[^"]*, pane, active" selected', rp), rp[-4000:]
# the tab is named for the focused pane's folder, never the program's title
# (that is the tooltip's, masked there too)
assert re.search(r'^    Tab "tmp(, [^"]*)?" selected', rp, re.M) and not re.search(r'(Tab|ListItem) "GH=ghp_', rp), \
    rp[-4000:]
assert rail_line("rail-secret").endswith('rows=["nomatch"]'), rail_line("rail-secret")

# Read-only panes refuse typing, pastes, drops, `ctl send-text` and a
# synchronized sibling's input, with a toast; toggling it off lets input in
# again. close_on_exit = "graceful": a failed exit keeps the pane with its
# Restart bar (exited=Some(3)), Enter restarts it in place (same pane id and
# layout), restarting over a running job asks first, and exit 0 closes it.
pl = scratch / "panelife"
for sub in ("cfg/rift", "data/rift"):
    (pl / sub).mkdir(parents=True, exist_ok=True)
(pl / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\ncontrol_api = "full"\nclose_on_exit = "graceful"\n')
pl_env = dict(env, XDG_CONFIG_HOME=str(pl / "cfg"), XDG_DATA_HOME=str(pl / "data"),
              BASH_SILENCE_DEPRECATION_WARNING="1")
pl_ctl = pl / "ctl.json"
(pl / "t.rift").write_text("\n".join([
    "wait_prompt", "key cmd+p", "type toggle read-only", "wait 200", "key enter", "wait 300",
    "type echo typed", "paste pasted", f"drop {pl}/t.rift", "wait 300", f"dump {pl}/ro.txt",
    f"sh ({shlex.quote(str(binary))} ctl request "
    + shlex.quote('{"cmd":"send-text","pane":1,"text":"echo ctl\\n"}')
    + f" > {shlex.quote(str(pl_ctl))} 2>&1; touch {shlex.quote(str(pl_ctl))}.done) &",
    f"wait_file {pl_ctl}.done 15000", "wait 300",
    "key cmd+d", "wait_prompt", "wait 400", "key alt+cmd+i", "type echo synced\\n", "wait 800",
    f"dump {pl}/ro-sync.txt", "key alt+cmd+i", "key alt+cmd+left", "wait 200",
    "key cmd+p", "type toggle read-only", "wait 200", "key enter", "wait 300",
    "type echo free\\n", "wait 600", f"dump {pl}/ro-off.txt",
    "key alt+cmd+right", "wait 200", "type exit 3\\n", "wait 1200", f"dump {pl}/kept.txt",
    "type abc", "key enter", "wait_prompt", "wait 800", f"dump {pl}/restarted.txt",
    "type sleep 30\\n", "wait 500", "key cmd+p", "type restart pane", "wait 200",
    "key enter", "wait 600", f"dump {pl}/restart-confirm.txt", "key enter", "wait 1500",
    "type exit 0\\n", "wait 1200", f"dump {pl}/exit0.txt", "quit"]) + "\n")
with (pl / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(pl / "t.rift"), "--headless"],
                        env=pl_env, stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("pane life run failed", pl / "app.log")
def pl_state(name):
    return (pl / f"{name}.txt").read_text()
def pl_panes(name):
    # (id, editor, last_exit, exited, read_only) per pane
    return re.findall(r'^  pane (\d+) .*? editor="([^"]*)" .*? last_exit=(\S+) .*\n'
                      r'(?:    .*\n)*?    exited=(\S+) read_only=(\w+)$', pl_state(name), re.M)
ro = pl_state("ro")
assert pl_panes("ro") == [("1", "", "None", "None", "true")], ro
assert re.search(r'^    Tab "[^"]*, read-only" selected', ro, re.M), ro[-3000:]
assert '"Pane is read-only"' in ro, ro
assert json.loads(pl_ctl.read_text()) == {"ok": False, "error": "pane is read-only"}, pl_ctl.read_text()
assert pl_panes("ro-sync") == [("1", "", "None", "None", "true"), ("2", "", "Some(0)", "None", "false")], \
    pl_state("ro-sync")
assert pl_panes("ro-off")[0] == ("1", "", "Some(0)", "None", "false"), pl_state("ro-off")
assert pl_panes("kept")[1][3] == "Some(3)" and "running=false" in pl_state("kept"), pl_state("kept")
assert pl_panes("restarted")[1][0] == "2" and pl_panes("restarted")[1][3] == "None" \
    and "Leaf(1)), (1.0, Leaf(2))" in pl_state("restarted"), pl_state("restarted")
assert 'confirm=Some((Restart(2), ["sleep 30"]))' in pl_state("restart-confirm"), \
    pl_state("restart-confirm")
assert [p[0] for p in pl_panes("exit0")] == ["1"], pl_state("exit0")

# Read-only command recall refuses every insertion path without replacing
# the draft: the shortcut, the history palette and a multi-block menu.
ro_recall = scratch / "read-only-recall"
(ro_recall / "cfg/rift").mkdir(parents=True)
(ro_recall / "data").mkdir()
(ro_recall / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'shell_completions = false\nsync_os_theme = false\n')
(ro_recall / "t.rift").write_text("\n".join([
    "wait_prompt", "type echo recall-one\\n", "wait_prompt",
    "type echo recall-two\\n", "wait_prompt", "type keep draft",
    "key cmd+p", "type toggle read-only", "key enter", "wait 200",
    "key cmd+up", "key cmd+i", f"dump {ro_recall}/shortcut.txt",
    "key cmd+p", "type #echo recall-one", "key enter", "wait 200",
    f"dump {ro_recall}/palette.txt",
    "key escape", "key cmd+up", "key cmd+a", "key ctrl+m", "wait 200",
    "a11y click Insert 2 commands into the input", f"dump {ro_recall}/multi.txt",
    "key escape", "key cmd+up", "key shift+up", f"dump {ro_recall}/selected.txt",
    "key escape", "key cmd+p", "type toggle read-only", "key enter", "wait 200",
    "key cmd+up", "key cmd+i", f"dump {ro_recall}/unlocked.txt", "quit",
]) + "\n")
with (ro_recall / "app.log").open("w") as log:
    ro_recall_rc = subprocess.run(
        [str(binary), "--script", str(ro_recall / "t.rift"), "--headless"],
        env=dict(env, XDG_CONFIG_HOME=str(ro_recall / "cfg"),
                 XDG_DATA_HOME=str(ro_recall / "data")),
        stdout=log, stderr=log, timeout=90).returncode
assert ro_recall_rc == 0, died("read-only recall run failed", ro_recall / "app.log")
ro_recall_bad = []
for ro_recall_path in ("shortcut", "palette", "multi"):
    ro_recall_state = (ro_recall / f"{ro_recall_path}.txt").read_text()
    if 'editor="keep draft"' not in ro_recall_state:
        ro_recall_bad.append(ro_recall_path + ": " + next(
            line for line in ro_recall_state.splitlines() if line.startswith("  pane ")))
assert not ro_recall_bad, "read-only command recall replaced the draft: " + repr(ro_recall_bad)
assert 'editor="echo recall-two"' in (ro_recall / "unlocked.txt").read_text()
assert "selected_blocks=[0, 1]" in (ro_recall / "selected.txt").read_text(), \
    (ro_recall / "selected.txt").read_text()[-4000:]

# A read-only prompt keeps its draft when End or a rebound accept key
# would take ghost text. Unlocking permits the same suggestion again.
ro_ghost = scratch / "read-only-ghost"
ro_ghost_env = raw_input_fixture(ro_ghost)
with (ro_ghost / "cfg/rift/config.toml").open("a") as ro_ghost_config:
    ro_ghost_config.write('[keybindings]\naccept_autosuggestion = "shift+right"\n')
(ro_ghost / "t.rift").write_text("\n".join([
    "wait_prompt", "type echo locked-ghost\\n", "wait_prompt", "type echo locked",
    "wait 700", f"dump {ro_ghost}/before.txt",
    "key cmd+p", "type toggle read-only", "key enter", "wait 200",
    "key end", f"dump {ro_ghost}/end.txt", "key shift+right", f"dump {ro_ghost}/bound.txt",
    "key cmd+p", "type toggle read-only", "key enter", "wait 300",
    "key end", f"dump {ro_ghost}/unlocked.txt", "quit",
]) + "\n")
with (ro_ghost / "app.log").open("w") as log:
    ro_ghost_rc = subprocess.run(
        [str(binary), "--script", str(ro_ghost / "t.rift"), "--headless"],
        env=ro_ghost_env, stdout=log, stderr=log, timeout=60).returncode
assert ro_ghost_rc == 0, died("read-only ghost run failed", ro_ghost / "app.log")
assert 'ghost="-ghost"' in (ro_ghost / "before.txt").read_text()
for ro_ghost_stage in ("end", "bound"):
    assert 'editor="echo locked"' in (ro_ghost / f"{ro_ghost_stage}.txt").read_text(), \
        ro_ghost_stage + ": " + (ro_ghost / f"{ro_ghost_stage}.txt").read_text()[-3000:]
assert 'editor="echo locked-ghost"' in (ro_ghost / "unlocked.txt").read_text()

# Mouse placement measures whole graphemes: either half of a combining
# character chooses a boundary, and Backspace removes the complete cluster.
mouse_grapheme = scratch / "mouse-grapheme"
mouse_grapheme_env = raw_input_fixture(mouse_grapheme)
def mouse_grapheme_run(name, steps):
    (mouse_grapheme / f"{name}.rift").write_text("\n".join(steps + ["quit"]) + "\n")
    with (mouse_grapheme / f"{name}.log").open("w") as log:
        rc = subprocess.run(
            [str(binary), "--script", str(mouse_grapheme / f"{name}.rift"), "--headless"],
            env=mouse_grapheme_env, stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died("mouse grapheme run failed", mouse_grapheme / f"{name}.log")
mouse_grapheme_run("geometry", ["resize 1000 700", "wait_prompt", "paste áb",
                                f"dump {mouse_grapheme}/geometry.txt"])
mouse_geometry = unicode_dump(mouse_grapheme / "geometry.txt")
mouse_anchor = re.search(r"editor_anchor=\(([^,]+), ([^)]+)\)", mouse_geometry)
assert mouse_anchor, mouse_geometry[:2000]
mouse_x, mouse_y = map(float, mouse_anchor.groups())
mouse_cell = float(re.search(r"geometry cell_w=([\d.]+)", mouse_geometry)[1])
mouse_steps = ["resize 1000 700", "wait_prompt", "paste áb"]
for mouse_i, mouse_frac in enumerate((0.25, 0.75, 1.25, 1.75)):
    mouse_steps += ["wait 600", f"click {mouse_x + mouse_cell * mouse_frac} {mouse_y + 5}",
                    f"dump {mouse_grapheme}/{mouse_i}.txt"]
mouse_steps += ["wait 600", f"click {mouse_x + mouse_cell * 0.75} {mouse_y + 5}",
                "key backspace", f"dump {mouse_grapheme}/deleted.txt",
                f"shot {mouse_grapheme}/deleted.png"]
mouse_grapheme_run("pointer", mouse_steps)
mouse_cursors = [int(re.search(r'editor="áb" cursor=(\d+)',
                              unicode_dump(mouse_grapheme / f"{i}.txt"))[1]) for i in range(4)]
assert mouse_cursors == [0, 3, 3, 4], (mouse_cursors, mouse_x, mouse_y, mouse_cell)
assert 'editor="b"' in (mouse_grapheme / "deleted.txt").read_text()

# Accepting ghost text in Vim Normal mode rests on the final whole
# grapheme, with a block cursor there instead of inside its combining marks.
vim_ghost = scratch / "vim-ghost-grapheme"
vim_ghost_env = raw_input_fixture(vim_ghost)
with (vim_ghost / "cfg/rift/config.toml").open("a") as vim_ghost_config:
    vim_ghost_config.write('vim_mode = true\n[keybindings]\naccept_autosuggestion = "shift+right"\n')
(vim_ghost / "t.rift").write_text("\n".join([
    "wait_prompt", "paste echo á", "key enter", "wait_prompt", "paste echo ",
    "wait 700", f"dump {vim_ghost}/before.txt", "key escape", "key shift+right",
    f"dump {vim_ghost}/accepted.txt", "quit",
]) + "\n")
with (vim_ghost / "app.log").open("w") as log:
    vim_ghost_rc = subprocess.run(
        [str(binary), "--script", str(vim_ghost / "t.rift"), "--headless"],
        env=vim_ghost_env, stdout=log, stderr=log, timeout=60).returncode
assert vim_ghost_rc == 0, died("Vim ghost run failed", vim_ghost / "app.log")
assert 'ghost=" á"' in unicode_dump(vim_ghost / "before.txt")
assert 'editor="echo á" cursor=5' in unicode_dump(vim_ghost / "accepted.txt"), \
    (vim_ghost / "accepted.txt").read_text()[-4000:]

# IME commits reach raw legacy and kitty programs in full, including
# multi-character text, cancelled composition and synthetic key releases.
ime_checks = []
for ime_flags in (0, 9, 11, 25):
    ime_base = scratch / f"ime-{ime_flags}"
    ime_env = raw_input_fixture(ime_base)
    (ime_base / "t.rift").write_text("\n".join([
        "wait_prompt", f"type python3 {ime_base}/record.py {ime_base} {ime_flags}\\n",
        f"wait_file {ime_base}/ready 10000", "wait 200",
        "preedit cancelled", "preedit", "preedit にほんご", "ime 日本語", "ime A",
        "wait 300", f"sh touch {ime_base}/stop", "wait_prompt", "quit",
    ]) + "\n")
    with (ime_base / "app.log").open("w") as log:
        ime_rc = subprocess.run([str(binary), "--script", str(ime_base / "t.rift"), "--headless"],
                                env=ime_env, stdout=log, stderr=log, timeout=60).returncode
    assert ime_rc == 0, died("IME routing run failed", ime_base / "app.log")
    if ime_flags == 0:
        ime_want = "日本語A".encode()
    else:
        ime_want = b""
        for ime_code, ime_mods, ime_text in ((26085, 1, 26085), (26412, 1, 26412),
                                               (35486, 1, 35486), (97, 2, 65)):
            ime_params = f";{ime_mods};{ime_text}" if ime_flags & 16 else (
                f";{ime_mods}" if ime_mods != 1 else "")
            ime_want += f"\x1b[{ime_code}{ime_params}u".encode()
            if ime_flags & 2:
                ime_want += f"\x1b[{ime_code};{ime_mods}:3u".encode()
    ime_got = (ime_base / "bytes").read_bytes()
    if ime_got != ime_want:
        ime_checks.append((ime_flags, ime_got, ime_want))
assert not ime_checks, f"committed text lost or misencoded: {ime_checks!r}"

# Kitty key releases go back to the pane that received the press, even
# after another window takes focus (never to its shell's editor).
release_base = scratch / "key-release-window"
release_env = raw_input_fixture(release_base)
(release_base / "t.rift").write_text("\n".join([
    "wait_prompt", "new_window", "wait_prompt", "window 0",
    f"type python3 {release_base}/record.py {release_base} 11\\n",
    f"wait_file {release_base}/ready 10000", "wait 200", "key a", "window 1", "keyup a",
    "wait 300", f"dump {release_base}/other.txt", f"sh touch {release_base}/stop",
    "window 0", "wait_prompt", "quit",
]) + "\n")
with (release_base / "app.log").open("w") as log:
    release_rc = subprocess.run(
        [str(binary), "--script", str(release_base / "t.rift"), "--headless"],
        env=release_env, stdout=log, stderr=log, timeout=60).returncode
assert release_rc == 0, died("key-release routing run failed", release_base / "app.log")
assert (release_base / "bytes").read_bytes() == b"\x1b[97u\x1b[97;1:3u", \
    (release_base / "bytes").read_bytes()
assert 'editor=""' in (release_base / "other.txt").read_text()

# Pipe output only enters a local writable shell prompt: a program owning
# raw input must never receive the shell wrapper, and remote paths cannot
# consume a local temporary file. Cancellation creates no output file.
pipe_base = scratch / "pipe-output-guard"
pipe_env = raw_input_fixture(pipe_base)
(pipe_base / "t.rift").write_text("\n".join([
    "wait_prompt", "type echo PIPE_SOURCE\\n", "wait_prompt",
    "key cmd+p", "type Send block output to a command", "key enter", "key escape",
    f"sh ls {pipe_base}/data/rift/pipe > {pipe_base}/cancel-files 2>/dev/null || true",
    "key cmd+p", "type Send block output to a command", "key enter",
    f"type cat > {pipe_base}/result", "key enter", "wait_prompt",
    f"type python3 {pipe_base}/record.py {pipe_base} 0\\n",
    f"wait_file {pipe_base}/ready 10000", "wait 200",
    "key cmd+p", "type Send block output to a command", "key enter", "wait 200",
    f"dump {pipe_base}/busy.txt", "wait 300",
    f"sh touch {pipe_base}/stop", "wait_prompt", "quit",
]) + "\n")
with (pipe_base / "app.log").open("w") as log:
    pipe_rc = subprocess.run([str(binary), "--script", str(pipe_base / "t.rift"), "--headless"],
                             env=pipe_env, stdout=log, stderr=log, timeout=60).returncode
assert pipe_rc == 0, died("pipe output run failed", pipe_base / "app.log")
assert (pipe_base / "cancel-files").read_text() == ""
assert (pipe_base / "result").read_text().strip() == "PIPE_SOURCE"
assert (pipe_base / "bytes").read_bytes() == b"", (pipe_base / "bytes").read_bytes()
assert "palette=true mode=PipeOutput" not in (pipe_base / "busy.txt").read_text(), \
    (pipe_base / "busy.txt").read_text()[-3000:]

# Pipe output rechecks a prompt that changed while its dialog was open,
# and never creates a local output file for remote or read-only prompts.
pipe_recheck_bad = []
for pipe_case in ("changed", "remote", "locked"):
    pipe_check = scratch / f"pipe-check-{pipe_case}"
    pipe_check_env = raw_input_fixture(pipe_check)
    pipe_check_steps = ["wait_prompt", "type echo PIPE_RECHECK\\n", "wait_prompt"]
    if pipe_case == "changed":
        (pipe_check / "alternate.sh").write_text(
            f"sleep 1\nprintf '\\033[?1049h'\ntouch {pipe_check}/alternate\nsleep 5\n")
        pipe_check_steps += [f"type sh {pipe_check}/alternate.sh &\\n", "wait_prompt"]
    elif pipe_case == "remote":
        (pipe_check / "remote.sh").write_text(
            "__rift_report_cwd() { printf '\\033]7;file://remote.invalid/tmp\\007'; }\n")
        pipe_check_steps += [f"type source {pipe_check}/remote.sh\\n", "wait_prompt",
                             "type echo REMOTE_READY\\n", "wait_prompt"]
    else:
        pipe_check_steps += ["key cmd+p", "type toggle read-only", "key enter"]
    pipe_check_steps += ["key cmd+p", "type Send block output to a command", "key enter",
                         f"dump {pipe_check}/opened.txt"]
    if pipe_case == "changed":
        pipe_check_steps += [f"type cat > {pipe_check}/unexpected",
                             f"wait_file {pipe_check}/alternate 5000", "wait 200",
                             f"dump {pipe_check}/alternate.txt", "key enter"]
    pipe_check_steps += ["wait 300", f"dump {pipe_check}/after.txt", "quit"]
    (pipe_check / "t.rift").write_text("\n".join(pipe_check_steps) + "\n")
    with (pipe_check / "app.log").open("w") as log:
        pipe_check_rc = subprocess.run(
            [str(binary), "--script", str(pipe_check / "t.rift"), "--headless"],
            env=pipe_check_env, stdout=log, stderr=log, timeout=60).returncode
    assert pipe_check_rc == 0, died("pipe recheck run failed", pipe_check / "app.log")
    pipe_opened = (pipe_check / "opened.txt").read_text()
    if pipe_case == "changed":
        assert "palette=true mode=PipeOutput" in pipe_opened, pipe_opened[-3000:]
        assert "alt=true" in (pipe_check / "alternate.txt").read_text()
    else:
        if "palette=true mode=PipeOutput" in pipe_opened:
            pipe_recheck_bad.append(pipe_case)
        if pipe_case == "remote":
            assert "cwd_remote=true" in pipe_opened, pipe_opened[-3000:]
    if list((pipe_check / "data/rift/pipe").glob("*")) or (pipe_check / "unexpected").exists():
        pipe_recheck_bad.append(pipe_case + " wrote output")
assert not pipe_recheck_bad, f"pipe prompt checks failed: {pipe_recheck_bad}"

# Palette history runs only at shell prompts, including synchronized
# siblings: a running program must receive none of the recalled command.
history_run_bad = []
for history_synced in (False, True):
    history_run_base = scratch / f"history-run-{history_synced}"
    history_run_env = raw_input_fixture(history_run_base)
    history_run_steps = ["wait_prompt", "type echo HISTORY_SAFE\\n", "wait_prompt"]
    if history_synced:
        history_run_steps += ["key cmd+d", "wait_prompt"]
    history_run_steps += [
        f"type python3 {history_run_base}/record.py {history_run_base} 0\\n",
        f"wait_file {history_run_base}/ready 10000", "wait 200"]
    if history_synced:
        history_run_steps += ["key alt+cmd+left", "key alt+cmd+i"]
    history_run_steps += [
        "key cmd+p", "type echo HISTORY_SAFE", "key enter", "wait 300",
        f"dump {history_run_base}/after.txt", f"sh touch {history_run_base}/stop",
        "wait 300", "quit"]
    (history_run_base / "t.rift").write_text("\n".join(history_run_steps) + "\n")
    with (history_run_base / "app.log").open("w") as log:
        history_run_rc = subprocess.run(
            [str(binary), "--script", str(history_run_base / "t.rift"), "--headless"],
            env=history_run_env, stdout=log, stderr=log, timeout=60).returncode
    assert history_run_rc == 0, died("history run failed", history_run_base / "app.log")
    history_run_bytes = (history_run_base / "bytes").read_bytes()
    if history_run_bytes:
        history_run_bad.append((history_synced, history_run_bytes))
    if history_synced:
        history_run_state = (history_run_base / "after.txt").read_text()
        assert len(re.findall(r'block \d+ state=done .*?cmd="echo HISTORY_SAFE"', history_run_state)) == 2, \
            history_run_state[-4000:]
assert not history_run_bad, f"palette history sent a shell command to raw input: {history_run_bad!r}"

# Tabs move between windows (`drag_tab` presses, pulls and releases a tab
# through the mouse handlers; headless windows sit apart on a pretend
# desktop). Out of every window: a new window holds it, with the same shell
# (its `sleep` still running, the same $$) and scrollback, and takes focus.
# Onto another window's strip at index 1: it lands there; its window closes
# when that was its last tab, its shell living on. A failed command in a
# moved pane is that window's notification.
xw = scratch / "xwindow"
for sub in ("cfg/rift", "data/rift"):
    (xw / sub).mkdir(parents=True, exist_ok=True)
xw_config = ('shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\n'
             'sync_os_theme = false\n')
# a profile whose program is missing: ⌃⇧1 puts an error toast on the pane
(xw / "cfg/rift/config.toml").write_text(
    xw_config + '[[profiles]]\nname = "bad"\ncommand = "/nonexistent/rift-shell"\n')
xw_env =dict(env, XDG_CONFIG_HOME=str(xw / "cfg"), XDG_DATA_HOME=str(xw / "data"),
              RIFT_PERSIST_SESSION="1", BASH_SILENCE_DEPRECATION_WARNING="1")
def xw_run(name, steps):
    (xw / f"{name}.rift").write_text("\n".join(steps) + "\n")
    with (xw / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(xw / f"{name}.rift"), "--headless"],
                            env=xw_env, stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"tab move {name} run failed", xw / f"{name}.log")
def xw_windows(name):
    return re.findall(r"^window (\d+): tabs=(\d+) active=(\d+) panes=\[([\d, ]*)\]",
                      (xw / f"{name}.txt").read_text(), re.M)
xw_run("drag", [
    "wait_prompt", "type echo xw-first\\n", "wait 300", "key cmd+t", "wait_prompt",
    "type echo xw-scrollback\\n", "wait 300",
    f"type echo $$ > {xw}/pid1; sleep 300\\n", "wait 800", "key ctrl+shift+1", "wait 200",
    f"dump {xw}/before.txt",
    "drag_tab 0 1 new", "wait 600", f"dump {xw}/new.txt",
    "window 0", "wait 200", f"dump {xw}/left.txt", "window 1", "wait 200",
    f"sh ps -p $(cat {xw}/pid1) -o pid= > {xw}/alive.txt",
    "key ctrl+c", "wait 300", f"type echo $$ > {xw}/pid2; false\\n", "wait 600",
    f"dump {xw}/note.txt",
    "key cmd+t", "wait_prompt", "wait 300", "drag_tab 0 0 1 1", "wait 600", f"dump {xw}/into.txt",
    f"sh cp {xw}/data/rift/session.toml {xw}/after-drag.toml", "quit"])
assert xw_windows("before") == [("0", "2", "1", "1, 2")], xw_windows("before")
new = (xw / "new.txt").read_text()
assert new.startswith("windows=2 window=1 focused_window=Some(1)"), new[:300]
assert xw_windows("new") == [("0", "1", "0", "1"), ("1", "1", "0", "2")], xw_windows("new")
assert re.search(r"^tab 0 .* running=true( |$)", new, re.M) and "xw-scrollback" in new, new[:3000]
# the pane's toast goes with it, and leaves the window it left
xw_toast = r'^toasts=\[\(Error, Some\(2\), None, "Profile \\"bad\\"'
assert re.search(xw_toast, (xw / "before.txt").read_text(), re.M), (xw / "before.txt").read_text()[:3000]
assert re.search(xw_toast, new, re.M), new[:3000]
assert re.search(r"^toasts=\[\] ", (xw / "left.txt").read_text(), re.M), (xw / "left.txt").read_text()[:3000]
assert (xw / "alive.txt").read_text().strip() == (xw / "pid1").read_text().strip(), \
    (xw / "alive.txt").read_text()
assert (xw / "pid1").read_text() == (xw / "pid2").read_text(), "the moved tab's shell changed"
assert re.search(r'^attention id=\d+ pane=2 kind=Failed .* window=Some\(1\)$',
                 (xw / "note.txt").read_text(), re.M), (xw / "note.txt").read_text()[:3000]
into = (xw / "into.txt").read_text()
assert into.startswith("windows=1 window=0 focused_window=Some(0)"), into[:300]
assert xw_windows("into") == [("0", "3", "1", "2, 1, 3")], xw_windows("into")
assert 'cmd="echo xw-first"' in into, into[:3000]
# The menus, with the tab rail: a tab's "Move to window 2: <its active tab>"
# and a dragged rail row move a tab; the pane menu's "Move to new window"
# moves a split pane; the drop-down window gives its last tab away (it
# hides, with a fresh tab) and takes one from the menu (it drops down).
# The session file then holds the ordinary windows, and a launch restores them.
(xw / "data/rift/session.toml").unlink(missing_ok=True)
(xw / "cfg/rift/config.toml").write_text(
    xw_config + 'tab_layout = "vertical"\nhotkey_window = true\nhotkey_window_height = 50\n'
    '[keybindings]\nrename_tab = "ctrl+cmd+y"\n')
# (the rail's rows start under its search field: y 161 is the second tab)
xw_run("menu", [
    "resize 1000 700", "wait_prompt", "key cmd+t", "wait_prompt", "key ctrl+cmd+y", "type mover\\n",
    "new_window", "wait_prompt", "resize 1000 700", "key ctrl+cmd+y", "type dest\\n", "wait 300",
    "window 0", "wait 300", "rclick 100 161", "wait 200", f"dump {xw}/tab-menu.txt",
    "a11y click Move to window 2: dest", "wait 500", f"dump {xw}/menu-moved.txt",
    "drag_tab 1 1 0 0", "wait 500", f"dump {xw}/rail-back.txt",
    "key cmd+d", "wait_prompt", "wait 300", "rclick 800 300", "wait 200",
    f"dump {xw}/pane-menu.txt", "a11y click Move to new window", "wait 500",
    f"dump {xw}/pane-moved.txt",
    "hotkey", "wait_prompt", "wait 300", "drag_tab 3 0 0", "wait_prompt", "wait 500",
    f"dump {xw}/dropdown-out.txt",
    "window 0", "rclick 100 161", "wait 200", "a11y click Move to hotkey window", "wait 500",
    f"dump {xw}/dropdown-in.txt",
    f"sh cp {xw}/data/rift/session.toml {xw}/after-menu.toml", "quit"])
tab_menu = (xw / "tab-menu.txt").read_text()
assert re.search(r'^menu_items=\[.*"Move to new window", "Move to window 2: dest", "Close tab"',
                 tab_menu, re.M), tab_menu[:3000]
assert xw_windows("menu-moved") == [("0", "1", "0", "1"), ("1", "2", "1", "3, 2")], \
    xw_windows("menu-moved")
assert xw_windows("rail-back") == [("0", "2", "0", "2, 1"), ("1", "1", "0", "3")], \
    xw_windows("rail-back")
assert '"Move to new tab", "Move to new window"' in (xw / "pane-menu.txt").read_text()
pane_moved = (xw / "pane-moved.txt").read_text()
assert pane_moved.startswith("windows=3 window=2 "), pane_moved[:300]
assert xw_windows("pane-moved")[2] == ("2", "1", "0", "4"), xw_windows("pane-moved")
out = (xw / "dropdown-out.txt").read_text()
assert xw_windows("dropdown-out")[0] == ("0", "3", "2", "2, 1, 5"), xw_windows("dropdown-out")
assert re.search(r"^window 3: tabs=1 active=0 panes=\[6\] .*kind=dropdown shown=false", out, re.M), \
    out[:600]
inn = (xw / "dropdown-in.txt").read_text()
assert inn.startswith("windows=4 window=3 "), inn[:300]
assert re.search(r"^window 3: tabs=2 active=1 panes=\[6, 1\] .*kind=dropdown shown=true", inn, re.M), \
    inn[:600]
# a launch restores the three ordinary windows as they were left
(xw / "data/rift/session.toml").write_text((xw / "after-menu.toml").read_text())
(xw / "cfg/rift/config.toml").write_text(xw_config + 'tab_layout = "vertical"\n')
xw_run("restore", ["wait_prompt", "wait 1000", f"dump {xw}/restored.txt", "quit"])
restored = [(t, a) for _, t, a, _ in xw_windows("restored")]
assert restored == [("2", "1"), ("1", "0"), ("1", "0")], xw_windows("restored")
assert re.search(r'^window 0: .*titles=\["mover", ', (xw / "restored.txt").read_text(), re.M)

# --- drop-down hotkey window (hotkey_window): the `hotkey` step runs the
# global hotkey's toggle. The window is created on first use, spans the
# pointer's screen at its top edge at the configured height, takes typing,
# hides on the next hotkey (its shell lives on), hides on a focus loss, and
# stays out of the session file; the ordinary window is untouched.
dd = scratch / "dropdown"
for sub in ("cfg/rift", "data/rift"):
    (dd / sub).mkdir(parents=True, exist_ok=True)
(dd / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = true\ndurable_sessions = false\nsync_os_theme = false\n'
    'hotkey_window = true\nhotkey_window_height = 50\n')
(dd / "t.rift").write_text("\n".join([
    "wait_prompt", "type echo ordinary-window\\n", "wait 400", f"dump {dd}/d0.txt",
    "hotkey", "wait_prompt", "wait 300", "type echo in-dropdown\\n", "wait 500", f"dump {dd}/d1.txt",
    "hotkey", "wait 300", "type echo after-hide\\n", "wait 400", f"dump {dd}/d2.txt",
    f"sh cp {shlex.quote(str(dd / 'data/rift/session.toml'))} {shlex.quote(str(dd / 'saved.toml'))}",
    "hotkey", "wait 300", "type echo again-$((40+2))\\n", "wait 500", f"dump {dd}/d3.txt",
    "focus on", "focus off", "wait 200", f"dump {dd}/d4.txt", "quit"]) + "\n")
with (dd / "app.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(dd / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(dd / "cfg"), XDG_DATA_HOME=str(dd / "data"),
                                 RIFT_PERSIST_SESSION="1", BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("drop-down window run failed", dd / "app.log")
def dd_windows(name):
    text = (dd / f"{name}.txt").read_text()
    return text, re.findall(r"^window (\d+): .*$", text, re.M), text.splitlines()[0]
d0, rows, head = dd_windows("d0")
assert head.startswith("windows=1 ") and "kind=normal" in d0, d0[:400]
d1, rows, head = dd_windows("d1")
assert head.startswith("windows=2 window=1 focused_window=Some(1)"), head
line = re.search(r"^window 1: .*kind=dropdown shown=true sliding=false .*$", d1, re.M)
assert line, d1[:600]
assert re.search(r"^window 0: tabs=1 .*kind=normal$", d1, re.M), d1[:600]
nums = lambda key: [float(v) for v in re.search(key + r"=Some\(\[([^\]]+)\]\)", line[0])[1].split(",")]
frame, screen, size = nums("frame"), nums("screen"), nums("size")
# full width at the screen's top edge, half its height; the window has that size
assert frame[:3] == screen[:3] and frame[3] == round(screen[3] * 0.5), line[0]
assert size == frame[2:], line[0]
# typed into the drop-down window, at its prompt
assert 'cmd="echo in-dropdown"' in d1 and 'cmd="echo ordinary-window"' not in d1, d1[:3000]
d2, rows, head = dd_windows("d2")
assert head.startswith("windows=2 window=0 focused_window=Some(0)"), head
assert re.search(r"^window 1: tabs=1 .*kind=dropdown shown=false ", d2, re.M), d2[:600]
# hidden: typing goes to the ordinary window, whose own output is intact
assert 'cmd="echo after-hide"' in d2 and 'cmd="echo ordinary-window"' in d2, d2[:3000]
saved = (dd / "saved.toml").read_text()
assert saved.count("[[tabs]]") == 1 and "other_windows" not in saved, saved
d3, rows, head = dd_windows("d3")
assert head.startswith("windows=2 window=1 "), head
# the same pane, its shell still alive after the hide
panes = lambda text, i: re.search(rf"^window {i}: .*panes=(\[[^\]]*\])", text, re.M)[1]
assert panes(d3, 1) == panes(d1, 1), (d1[:600], d3[:600])
assert re.search(r"^ +\| again-42$", d3, re.M), d3[:3000]
d4, rows, head = dd_windows("d4")
assert head.startswith("windows=2 window=0 focused_window=Some(0)"), head
assert re.search(r"^window 1: .*kind=dropdown shown=false ", d4, re.M), d4[:600]

# --- the drop-down window among ordinary windows: it does not keep the app
# open, so ⇧⌘W on the last ordinary window is a quit and asks about every
# window's running commands (here the drop-down's); a toast for a pane in a
# background window shows in the focused window; `ctl focus` on a pane of
# the hidden drop-down window drops it down; the switcher's answers reach
# the window that asked; the switcher and `ctl status` name the hotkey
# window; a phone closing the last ordinary tab never quits; turning
# hotkey_window off closes an idle drop-down window and keeps a busy one.
hw = scratch / "hotkey-windows"
hw_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
             'sync_os_theme = false\ncontrol_api = "full"\nhotkey_window = true\n')
def hw_run(name, steps, config=hw_config):
    run = hw / name
    for sub in ("cfg/rift", "data/rift/control"):
        (run / sub).mkdir(parents=True, exist_ok=True)
    (run / "cfg/rift/config.toml").write_text(config)
    (run / "t.rift").write_text("\n".join(steps) + "\n")
    with (run / "app.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(run / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(run / "cfg"),
                                     XDG_DATA_HOME=str(run / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"hotkey-window {name} run failed", run / "app.log")
    return run
def hw_ctl(run, name, request):
    out = hw / run / f"{name}.json"
    return [f"sh ({shlex.quote(str(binary))} ctl request {shlex.quote(request)} > {shlex.quote(str(out))} "
            f"2>&1; touch {shlex.quote(str(out))}.done) &", f"wait_file {out}.done 15000", "wait 300"]
# (1) ⇧⌘W on the only ordinary window with a command running in the drop-down
run = hw_run("quit", [
    "wait_prompt", "hotkey", "wait_prompt", "type sleep 301\\n", "wait 800",
    "window 0", "close_window", "wait 400", f"dump {hw}/quit/asked.txt",
    "key escape", "wait 200", "quit"])
asked = (run / "asked.txt").read_text() if (run / "asked.txt").exists() else ""
assert 'confirm=Some((Close(Window), ["sleep 301"]))' in asked, \
    died("⇧⌘W quit without asking about the drop-down's command", run / "app.log") + asked[:600]
assert "Quit Rift?" in asked, asked[-3000:]
# (2) panes 1 and 2 are the ordinary windows', 3 the drop-down's; a control
# socket that never answers holds the switcher's search for windows of
# other processes (6 s) until the asking window is no longer focused
def socket_dir(data_dir, name):
    # rift_pty::ipc::socket_dir_for: a path too long for a socket moves to /tmp
    path = data_dir / name
    if len(str(path)) + 20 < 100:
        return path
    h = 0xcbf29ce484222325
    for byte in str(data_dir).encode():
        h = ((h ^ byte) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return Path("/tmp") / f"rift-{os.getuid()}-{h:016x}" / name
hw_control = socket_dir(hw / "route/data/rift", "control")
(hw / "route").mkdir(parents=True, exist_ok=True)
for d in (hw_control.parent.parent, hw_control.parent, hw_control):
    d.mkdir(mode=0o700, exist_ok=True)
stall_hw = socket.socket(socket.AF_UNIX)
cwd = os.getcwd()
os.chdir(hw_control)
try:
    stall_hw.bind("999998.sock")
finally:
    os.chdir(cwd)
stall_hw.listen(4)
try:
    run = hw_run("route", [
        "wait_prompt", "new_window", "wait_prompt",
        "window 0", "focus on", "window 1", "focus off", f"type printf '\\\\e]9;hw-toast\\\\a'; touch {hw}/route/toast.ready\\n",
        f"wait_file {hw}/route/toast.ready 15000", "wait 800",
        "window 0", f"dump {hw}/route/toast.txt",
        "hotkey", "wait_prompt", "hotkey", "wait 300",
        *hw_ctl("route", "focus", '{"cmd":"focus","pane":3}'), f"dump {hw}/route/focus.txt",
        "hotkey", "wait 300", "window 0", *hw_ctl("route", "status", '{"cmd":"status"}'),
        "key shift+cmd+o", "window 1", "wait 6500", "window 0", f"dump {hw}/route/switcher.txt",
        "key escape", "quit"])
finally:
    stall_hw.close()
    (hw_control / "999998.sock").unlink(missing_ok=True)
toast = (run / "toast.txt").read_text()
assert re.search(r"^note_toasts=\[\d+\]", toast, re.M), toast[:400]
log_text = (run / "app.log").read_text()
assert "alert Attention: hw-toast" in log_text and "notify (suppressed): hw-toast" not in log_text, log_text[-2000:]
focus = (run / "focus.txt").read_text()
assert focus.startswith("windows=3 window=2 focused_window=Some(2)"), focus[:300]
assert re.search(r"^window 2: .*kind=dropdown shown=true ", focus, re.M), focus[:800]
status = json.loads((run / "status.json").read_text())
assert status.get("windows") == [{"window": 2, "kind": "dropdown"}], status.get("windows")
assert [t.get("window", 0) for t in status["tabs"]] == [0, 1, 2], status["tabs"]
switcher = (run / "switcher.txt").read_text()
assert "palette=true mode=Sessions" in switcher and " loading=false/false" in switcher, \
    re.search(r"^palette=.*$", switcher, re.M)
assert re.search(r'ListBoxOption "[^"]* — hotkey window"', switcher), switcher[-3000:]
assert re.search(r'ListBoxOption "[^"]* — window 2"', switcher), switcher[-3000:]
# (3) a phone closing the only ordinary window's last tab leaves a fresh tab
run = hw_run("phone", [
    "wait_prompt", "hotkey", "wait_prompt", "hotkey", "wait 300",
    *hw_ctl("phone", "close", '{"cmd":"remote-close","pane":1,"device":"Test Phone","kind":"tab"}'),
    "wait 500", f"dump {hw}/phone/after.txt", "quit"],
    hw_config + 'remote_access = true\nremote_listen = "127.0.0.1:0"\n')
closed = json.loads((run / "close.json").read_text())
assert closed.get("ok") is True, closed
after = (run / "after.txt").read_text() if (run / "after.txt").exists() else ""
assert after.startswith("windows=2 "), died("a phone's close quit Rift", run / "app.log") + after[:300]
assert re.search(r"^window 0: tabs=1 .*kind=normal$", after, re.M), after[:600]
assert not re.search(r"^window 0: .*panes=\[1\]", after, re.M), after[:600]
# (4) hotkey_window off: an idle drop-down window closes; one running a
# command stays and drops down
off = hw / "retire/off.toml"
(hw / "retire").mkdir(parents=True, exist_ok=True)
off.write_text(hw_config.replace("hotkey_window = true", "hotkey_window = false"))
(hw / "retire/on.toml").write_text(hw_config)
conf = hw / "retire/cfg/rift/config.toml"
run = hw_run("retire", [
    "wait_prompt", "hotkey", "wait_prompt", "hotkey", "wait 300",
    cp(off, conf), "wait 2000", f"dump {hw}/retire/idle.txt",
    cp(hw / "retire/on.toml", conf), "wait 2000",
    "hotkey", "wait_prompt", "type sleep 301\\n", "wait 600", "hotkey", "wait 300",
    cp(off, conf), "wait 2000", f"dump {hw}/retire/busy.txt", "quit"])
idle = (run / "idle.txt").read_text()
assert idle.startswith("windows=1 ") and "kind=dropdown" not in idle, idle[:400]
busy = (run / "busy.txt").read_text()
assert busy.startswith("windows=2 "), busy[:400]
assert re.search(r"^window 1: .*kind=dropdown shown=true ", busy, re.M), busy[:600]

# Output paths of a command that ran on another host (ssh) never resolve
# against the local disk, while blocks that ran here still do, whichever side
# the pane is on now; the shell fakes the remote with a prompt hook that
# reports the directory on "otherhost"
kp = scratch / "keys-paths"
(kp / "proj/src").mkdir(parents=True)
(kp / "proj/src/main.rs").write_text("fn main() {}\n")
(kp / "fake.bash").write_text(
    "fakeremote() { printf '\\033]7;file://otherhost%s\\007' \"$PWD\"; }\n"
    'fakeon() { PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND;}fakeremote"; }\n'
    'fakeoff() { PROMPT_COMMAND="${PROMPT_COMMAND//;fakeremote/}"; }\n')
(pc / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
    'sync_os_theme = false\n')
run_prompt("remote-paths", [
    f"type cd {kp / 'proj'}\\n", "wait 600",
    f"type source {kp / 'fake.bash'}\\n", "wait 600",
    "type echo src/main.rs\\n", "wait 600",
    "type fakeon\\n", "wait 600",
    "type echo src/main.rs\\n", "wait 600",
    "type fakeoff\\n", "wait 600",
    "type echo src/main.rs\\n", "wait 800",
    "key shift+cmd+h", "wait 300", f"dump {kp}/hints.txt"])
remote_dump = (kp / "hints.txt").read_text()
flags = re.findall(r'cmd="echo src/main.rs".*? cwd_remote=(\w+)', remote_dump)
assert flags == ["false", "true", "false"], (flags, remote_dump)
hint_list = re.search(r"hints=(\[[^\]]*\])", remote_dump)[1]
# the two local outputs are hint targets, the remote one is not
assert hint_list.count(f"{kp}/proj/src/main.rs") == 2, hint_list

# code review by keyboard alone: ⌥⌘G focuses the panel, s / u stage and
# unstage a hunk or file, r asks first (Tab moves to Cancel and Enter presses
# it; Revert, then Enter, discards the hunk), Esc hands the keys back
krepo = kp / "repo"
krepo.mkdir()
kgit = ["git", "-C", str(krepo), "-c", "user.name=t", "-c", "user.email=t@e.x"]
ksrc = [f"    let v{n} = compute({n});" for n in range(1, 81)]
(krepo / "main.rs").write_text("\n".join(["fn main() {", *ksrc, "}"]) + "\n")
(krepo / "lib.rs").write_text("pub fn a() {}\n")
subprocess.run(["git", "init", "-q", str(krepo)], check=True)
subprocess.run([*kgit, "add", "-A"], check=True)
subprocess.run([*kgit, "commit", "-qm", "init"], check=True)
ksrc[4] = "    let v5 = compute_fast(5);"
ksrc[59] = "    let v60 = compute_slow(60);"
(krepo / "main.rs").write_text("\n".join(["fn main() {", *ksrc, "}"]) + "\n")
(krepo / "lib.rs").write_text("pub fn a() {}\npub fn b() {}\n")

run_prompt("review-keys", [
    "resize 1400 900",
    f"type cd {krepo}\\n", "wait 1000",
    "key alt+cmd+g", "wait 1000", f"dump {kp}/rk-focus.txt",
    "key down", "key down", "key right", "key down", "wait 300", f"dump {kp}/rk-hunk.txt",
    "key s", "wait 1200", f"sh git -C {krepo} diff --cached --numstat > {kp}/rk-staged.txt",
    f"dump {kp}/rk-next.txt",
    "key home", "key down", "key u", "wait 1200",
    f"sh git -C {krepo} diff --cached --numstat > {kp}/rk-unstaged.txt",
    "key home", "key down", "key down", "key right", "key down", "key r", "wait 500",
    f"dump {kp}/rk-confirm.txt",
    "key tab", "wait 200", f"dump {kp}/rk-cancel.txt",
    "key enter", "wait 800", f"dump {kp}/rk-kept.txt",
    f"sh git -C {krepo} diff --numstat > {kp}/rk-before.txt",
    "key r", "wait 500", "key enter", "wait 1500",
    f"sh git -C {krepo} diff --numstat > {kp}/rk-reverted.txt",
    "key escape", "type echo typed_in_editor\\n", "wait 700", f"dump {kp}/rk-esc.txt"])
rk = lambda name: (kp / f"rk-{name}.txt").read_text()
assert "review_keys=true review_focus=Some(Section(false))" in rk("focus"), rk("focus")
assert 'review_focus=Some(Hunk(false, "main.rs"' in rk("hunk"), rk("hunk")
# staging the first hunk leaves the focus on the file's other hunk
assert rk("staged").split()[:2] == ["1", "1"], rk("staged")
assert 'review_focus=Some(Hunk(false, "main.rs"' in rk("next"), rk("next")
assert rk("unstaged") == "", rk("unstaged")
assert "confirm_focus=confirm" in rk("confirm") and "Revert" in rk("confirm"), rk("confirm")
assert "confirm_focus=cancel" in rk("cancel"), rk("cancel")
assert "confirm=None" in rk("kept") and "confirm_focus=none" in rk("kept"), rk("kept")
assert rk("before").split("\n")[1].split() == ["2", "2", "main.rs"], rk("before")
# one hunk of main.rs went, the other and lib.rs stay
assert rk("reverted").split("\n")[:2] == ["1\t0\tlib.rs", "1\t1\tmain.rs"], rk("reverted")
assert "review_keys=false" in rk("esc") and 'cmd="echo typed_in_editor"' in rk("esc"), rk("esc")

# rift:// links: a tab and a window in a folder, a launch configuration by
# name, Settings with its search; a file or an unknown action is refused
lk = scratch / "links"
(lk / "dir a").mkdir(parents=True)
(lk / "script.sh").write_text("#!/bin/sh\ntouch ran\n")
(lk / "script.sh").chmod(0o755)
(pc / "cfg/rift/launch_configs").mkdir(parents=True, exist_ok=True)
(pc / "cfg/rift/launch_configs/links-demo.yaml").write_text(
    f"name: Links Demo\nwindows:\n  - tabs:\n      - title: LinkTab\n"
    f"        layout:\n          cwd: {lk}\n")
(pc / "cfg/rift/launch_configs/links-run.yaml").write_text(
    f"name: Links Run\nwindows:\n  - tabs:\n      - title: RunTab\n"
    f"        layout:\n          cwd: {lk}\n          commands:\n"
    f"            - exec: touch {lk}/ran-from-link\n")
run_prompt("links", [
    f"link rift://action/new_tab?path={str(lk / 'dir a').replace(' ', '%20')}",
    "wait 1500", f"dump {lk}/tab.txt",
    "link rift://launch/LINKS%20DEMO", "wait 300", f"dump {lk}/launch.txt",
    f"link rift://action/new_tab?path={lk}/script.sh", "wait 300", f"dump {lk}/file.txt",
    "link rift://action/run?cmd=touch%20ran", "wait 300", f"dump {lk}/unknown.txt",
    f"link rift://action/new_window?path={lk}", "wait 300", f"dump {lk}/window.txt",
    "link rift://settings?q=font+size", "wait 300", f"dump {lk}/settings.txt",
    # a layout whose panes run commands asks first; Cancel runs nothing
    "key escape", "link rift://launch/links%20run", "wait 300", f"dump {lk}/asks.txt",
    "key escape", "wait 1500", f"dump {lk}/cancelled.txt"])
lt = lambda name: (lk / f"{name}.txt").read_text()
# (a shell's own title may carry user@host: before the folder)
assert re.search(r'^window 0: tabs=2 active=1 .*titles=\[[^\]]*dir a"\]', lt("tab"), re.M), lt("tab")
# (the shell may report /tmp as /private/tmp)
assert re.search(r'pane \d+ cwd=Some\("[^"]*/links/dir a"\)', lt("tab")), lt("tab")
assert re.search(r'^window 0: tabs=3 .*"LinkTab"\]', lt("launch"), re.M), lt("launch")
assert "not a folder" in lt("file") and "tabs=3 " in lt("file"), lt("file")
assert "unknown link" in lt("unknown"), lt("unknown")
assert not (lk / "ran").exists() and not (work / "ran").exists()
assert re.search(r"^windows=2 window=1 ", lt("window"), re.M), lt("window")
assert 'settings_tab=true settings_query="font size"' in lt("settings"), lt("settings")
assert re.search(r"confirm=Some\(.*LaunchLink", lt("asks")), lt("asks")
assert "confirm=None" in lt("cancelled") and '"RunTab"' not in lt("cancelled"), lt("cancelled")
assert not (lk / "ran-from-link").exists()

# Launch links must ask before per-pane or saved-profile arguments execute,
# even when the layout has no commands[]. Cancel preserves the empty marker;
# explicit approval starts exactly the saved fixture program.
link_launch_issues = []
for link_launch_kind in ("pane", "profile"):
    ll = scratch / f"link-launch-{link_launch_kind}"
    (ll / "cfg/rift/launch_configs").mkdir(parents=True)
    (ll / "home").mkdir()
    marker = ll / "approved-only"
    ll_config = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
                 'discover_shells = false\nsync_os_theme = false\n')
    if link_launch_kind == "profile":
        ll_config += ('[[profiles]]\nname = "fixture"\ncommand = "/bin/bash"\n'
                      f'args = ["-c", "touch {marker}"]\n')
        ll_launch = 'profile: fixture\n'
    else:
        ll_launch = f'shell: /bin/bash\n          args: ["-c", "touch {marker}"]\n'
    (ll / "cfg/rift/config.toml").write_text(ll_config)
    (ll / "cfg/rift/launch_configs/probe.yaml").write_text(
        'name: fixture\nwindows:\n  - tabs:\n      - title: fixture\n        layout:\n          ' + ll_launch)
    (ll / "t.rift").write_text("\n".join([
        "wait_prompt", "link rift://launch/fixture", "wait 300", f"dump {ll}/asks.txt",
        f"shot {ll}/asks.png",
        "key escape", "wait 300", f"sh test ! -e {marker} && touch {ll}/cancel-safe",
        "link rift://launch/fixture", "wait 300", "a11y click Open and run", "wait 600",
        f"dump {ll}/approved.txt", "quit",
    ]) + "\n")
    ll_env = {k: v for k, v in env.items() if not k.startswith("RIFT_")}
    ll_env.update(HOME=str(ll / "home"), HISTFILE=str(ll / "home/history"),
                  XDG_CONFIG_HOME=str(ll / "cfg"), XDG_DATA_HOME=str(ll / "data"),
                  RIFT_NO_NOTIFY="1")
    with (ll / "app.log").open("w") as log:
        ll_rc = subprocess.run([str(binary), "--script", str(ll / "t.rift"), "--headless"],
                               env=ll_env, stdout=log, stderr=log, timeout=60).returncode
    assert ll_rc == 0, died("launch-link arguments run failed", ll / "app.log")
    ll_asks = (ll / "asks.txt").read_text()
    if "confirm=Some" not in ll_asks or "LaunchLink" not in ll_asks:
        link_launch_issues.append(f"{link_launch_kind}: no launch confirmation")
    if not (ll / "cancel-safe").exists():
        link_launch_issues.append(f"{link_launch_kind}: link ran before approval")
    if not marker.exists():
        link_launch_issues.append(f"{link_launch_kind}: approved launch did not run")
assert not link_launch_issues, link_launch_issues

# `rift-terminal <path>` from another process (an editor's "external
# terminal", a script): the running Rift opens a tab in the folder, relative
# to the caller's directory, a file's folder for a file, a window with
# --new-window; a missing path exits 2. A second process without a running
# Rift (the cold start) opens its first tab there.
op = scratch / "open-path"
(op / "a dir/sub").mkdir(parents=True)
(op / "a dir/f.txt").write_text("x\n")
def open_cli(tag, cwd, *args):
    # never without the socket: the process would start a Rift of its own
    out = op / tag
    cmd = (f"cd {shlex.quote(str(cwd))} && if ls {shlex.quote(str(socket_dir(pc / 'data/rift', 'control')))}/*.sock "
           f">/dev/null 2>&1; then {shlex.quote(str(binary))} "
           + " ".join(shlex.quote(a) for a in args)
           + f" 2> {shlex.quote(str(out))}.err; echo $? > {shlex.quote(str(out))}.rc; "
           f"else echo nosock > {shlex.quote(str(out))}.rc; fi; touch {shlex.quote(str(out))}.done")
    return [f"sh ({cmd}) &", f"wait_file {out}.done 15000", "wait 1500", f"dump {op}/{tag}.txt"]
run_prompt("open-path", [
    *open_cli("abs", "/", str(op / "a dir")),
    *open_cli("rel", op / "a dir", "sub"),
    *open_cli("file", op, "a dir/f.txt"),
    *open_cli("win", "/", "--new-window", str(op / "a dir/sub")),
    *open_cli("nope", op, "nope")])
ot = lambda name: (op / f"{name}.txt").read_text()
orc = lambda name: (op / f"{name}.rc").read_text().strip()
assert [orc(t) for t in ("abs", "rel", "file", "win", "nope")] == ["0", "0", "0", "0", "2"], \
    died("rift-terminal <path> exit codes", pc / "open-path.log")
assert "no such file or directory" in (op / "nope.err").read_text()
cwds = lambda name: re.findall(r'pane \d+ cwd=Some\("([^"]*)"\)', ot(name))
assert re.search(r"^window 0: tabs=2 active=1 ", ot("abs"), re.M), ot("abs")[:600]
assert cwds("abs")[-1].endswith("/open-path/a dir"), cwds("abs")
assert re.search(r"^window 0: tabs=3 active=2 ", ot("rel"), re.M), ot("rel")[:600]
assert cwds("rel")[-1].endswith("/open-path/a dir/sub"), cwds("rel")
assert re.search(r"^window 0: tabs=4 active=3 ", ot("file"), re.M), ot("file")[:600]
assert cwds("file")[-1].endswith("/open-path/a dir"), cwds("file")
assert re.search(r"^windows=2 window=1 ", ot("win"), re.M), ot("win")[:600]
assert re.search(r"^window 1: tabs=1 ", ot("win"), re.M) and cwds("win")[-1].endswith("/a dir/sub"), \
    ot("win")[:600]
assert "tabs=4 " in ot("nope") and "windows=2 " in ot("nope"), ot("nope")[:600]
# the cold start: one tab, in the folder named (relative to its directory)
(op / "cold.rift").write_text(f"wait_prompt\nwait 500\ndump {op}/cold.txt\nquit\n")
with (op / "cold.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(op / "cold.rift"), "--headless", "sub"],
                        cwd=op / "a dir", env=dict(env, XDG_CONFIG_HOME=str(pc / "cfg"),
                                                   XDG_DATA_HOME=str(op / "data")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("cold start with a path failed", op / "cold.log")
assert re.search(r"^window 0: tabs=1 ", ot("cold"), re.M), ot("cold")[:600]
assert cwds("cold") and cwds("cold")[0].endswith("/open-path/a dir/sub"), cwds("cold")
# a cold `--new-window` without a path: its window is the first one, not a second
(op / "cold-win.rift").write_text(f"wait_prompt\nwait 800\ndump {op}/cold-win.txt\nquit\n")
with (op / "cold-win.log").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(op / "cold-win.rift"), "--headless", "--new-window"],
                        cwd=op, env=dict(env, XDG_CONFIG_HOME=str(pc / "cfg"),
                                         XDG_DATA_HOME=str(op / "data-win")),
                        stdout=log, stderr=log, timeout=60).returncode
assert rc == 0, died("cold start with --new-window failed", op / "cold-win.log")
assert re.search(r"^windows=1 .*\nwindow 0: tabs=1 ", ot("cold-win"), re.M), ot("cold-win")[:600]

# RIFT_PANE_URL: a shell's own focus link. Another process opening it (an
# agent hook, a notification's click URL) raises the window holding the pane
# and focuses its tab and pane; a pane of an earlier run, or one now closed,
# says so in a toast and focuses nothing
pf2 = scratch / "pane-focus"
pf2.mkdir()
run_prompt("pane-focus", [
    f'type printf %s "$RIFT_PANE_URL" > {pf2}/url\\n', f"wait_file {pf2}/url 10000",
    "key cmd+d", "wait 1000", "key cmd+t", "wait 1000", "new_window", "wait_prompt",
    f"dump {pf2}/before.txt",
    f'type {shlex.quote(str(binary))} "$(cat {pf2}/url)"; echo $? > {pf2}/rc\\n',
    f"wait_file {pf2}/rc 15000", "wait 800", f"dump {pf2}/after.txt",
    "link rift://action/focus?pane=abcdef.1", "wait 300", f"dump {pf2}/stale.txt",
    "link rift://action/focus?pane=service.999999", "wait 300", f"dump {pf2}/gone.txt"])
pft = lambda name: (pf2 / f"{name}.txt").read_text()
pane_url = (pf2 / "url").read_text()
assert re.fullmatch(r"rift://action/focus\?pane=[0-9a-f]+\.1", pane_url), pane_url
assert (pf2 / "rc").read_text().strip() == "0"
assert re.search(r"^windows=2 window=1 ", pft("before"), re.M), pft("before")[:600]
assert re.search(r"^windows=2 window=0 focused_window=Some\(0\)", pft("after"), re.M), pft("after")[:600]
assert re.search(r"^window 0: tabs=2 active=0 ", pft("after"), re.M), pft("after")[:600]
assert re.search(r"^tab 0 title=.* panes=2 active_pane=1 ", pft("after"), re.M), pft("after")[:2000]
assert "That pane belonged to an earlier Rift run" in pft("stale"), pft("stale")[:2000]
assert "That pane is no longer open" in pft("gone"), pft("gone")[:2000]
assert re.search(r"^window 0: tabs=2 active=0 ", pft("gone"), re.M), pft("gone")[:600]

# the configurable toolbar: buttons on the side and in the order the config
# lists. Its editor is "Edit toolbar" (right-click a toolbar button or the
# empty strip → "Rearrange toolbar items", or Settings › Toolbar buttons): a
# palette of the hidden buttons and two dashed buckets, Left and Right. A
# palette click adds at the right end, a drag moves between the buckets, ×
# and a drop outside the buckets remove, Restore default puts the defaults
# back; Esc / Cancel leave config.toml alone, Enter / Save write it.
tb = scratch / "toolbar"
TB_CONFIG = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
             'sync_os_theme = false\ncheck_for_updates = false\n'
             'toolbar_left = ["palette", "review"]\ntoolbar_right = ["settings"]\n')
TB_MENU = ["rclick 1150 17", "wait 200", "key down", "key enter", "wait 300"]
def tb_run(name, steps):
    d = tb / name
    for sub in ("cfg/rift", "data"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "cfg/rift/config.toml").write_text(TB_CONFIG)
    (d / "t.rift").write_text("\n".join(
        ["resize 1200 600", "wait_prompt", *(s.replace("{d}", str(d)) for s in steps), "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data")),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"toolbar {name} run failed", d / "log.txt")
    return d
def tb_line(d, name="a"):
    return re.search(r"^toolbar=.*$", (d / f"{name}.txt").read_text(), re.M).group(0)
probe = tb_run("probe", ["dump {d}/a.txt", *TB_MENU, "dump {d}/b.txt"])
tips = lambda text: re.findall(r'"(Command palette|Review changes|File explorer|Notifications'
                               r'|Settings)@(\d+),', text)
assert tips((probe / "a.txt").read_text()) == [("Command palette", "80"), ("Review changes", "110"),
                                               ("Settings", "1164")], tips((probe / "a.txt").read_text())
# a screen reader reaches the buttons, named as their tooltips
assert re.search(r'Group "Toolbar"\n    Button "Command palette', (probe / "a.txt").read_text())
opened = tb_line(probe, "b")
assert opened.startswith("toolbar=[Palette, Review]|[Settings] toolbar_editor=Some(((["
                         "Palette, Review], [Settings]), None, None, true, false))"), opened
def tb_center(text, label):
    x, y, w, h = map(int, re.search(re.escape(label) + r"@(\d+),(\d+),(\d+),(\d+)", text).groups())
    return f"{x + w // 2} {y + h // 2}"
center = lambda label: tb_center(opened, label)
# the dialog for a screen reader: one button per item and action, Save
# changes listed but disabled while nothing changed
b_text = (probe / "b.txt").read_text()
assert 'Dialog "Edit toolbar"' in b_text and 'Button "Add File explorer"' in b_text, b_text[-3000:]
assert 'Button "Code review, 2 in the left side"\n      Button "Remove Code review"' in b_text, b_text[-3000:]
assert re.search(r'Button "Save changes"[^\n]*disabled', b_text), b_text[-3000:]
# a palette click: the button joins the right end
d = tb_run("add", [*TB_MENU, f"click {center('Palette(Explorer)')}", "wait 200", "dump {d}/a.txt"])
assert "toolbar_editor=Some((([Palette, Review], [Settings, Explorer]), None, None, true, true))" in tb_line(d)
# a drag from Right into Left, before its first button (the left half of it)
px, py, pw, ph = map(int, re.search(r"Item\(0, 0\)@(\d+),(\d+),(\d+),(\d+)", opened).groups())
d = tb_run("drag", [*TB_MENU, f"drag {center('Item(1, 0)')} {px + 6} {py + ph // 2}", "wait 200",
                    "dump {d}/a.txt"])
assert "toolbar_editor=Some((([Settings, Palette, Review], []), None, Some((0, 0)), true, true))" in tb_line(d), tb_line(d)
# × removes; a bucket button dropped outside every bucket leaves too
d = tb_run("remove", [*TB_MENU, f"click {center('Remove(0, 1)')}", "wait 200", "dump {d}/a.txt"])
assert "toolbar_editor=Some((([Palette], [Settings]), None, None, true, true))" in tb_line(d), tb_line(d)
bx, by = map(int, re.search(r"Bucket\(0\)@(\d+),(\d+),", opened).groups())
d = tb_run("out", [*TB_MENU, f"drag {center('Item(0, 0)')} {bx + 10} {by - 14}", "wait 200",
                   "dump {d}/a.txt"])
assert "toolbar_editor=Some((([Review], [Settings]), None, None, true, true))" in tb_line(d), tb_line(d)
# Restore default puts the defaults back (and goes inert)
d = tb_run("restore", [*TB_MENU, f"click {center('Restore')}", "wait 200", "dump {d}/a.txt"])
assert "toolbar_editor=Some((([Explorer], [Notifications, Review]), None, None, false, true))" in tb_line(d), tb_line(d)
assert "Restore@" not in tb_line(d)
# Esc and Cancel leave config.toml alone
d = tb_run("esc", [*TB_MENU, f"click {center('Palette(Explorer)')}", "key escape", "wait 200",
                   "dump {d}/a.txt"])
assert tb_line(d).startswith("toolbar=[Palette, Review]|[Settings] toolbar_editor=None"), tb_line(d)
assert (d / "cfg/rift/config.toml").read_text() == TB_CONFIG
d = tb_run("cancel", [*TB_MENU, f"click {center('Palette(Explorer)')}", f"click {center('Cancel')}",
                      "wait 200", "dump {d}/a.txt"])
assert tb_line(d).startswith("toolbar=[Palette, Review]|[Settings] toolbar_editor=None"), tb_line(d)
assert (d / "cfg/rift/config.toml").read_text() == TB_CONFIG
# opened from Settings › Toolbar buttons; Enter saves, and the strip shows it
found = tb_run("find", ["key cmd+,", "wait 400", "type toolbar", "wait 300", "dump {d}/a.txt"])
preview = tb_center((found / "a.txt").read_text(), "Edit toolbar")
d = tb_run("save", ["key cmd+,", "wait 400", "type toolbar", "wait 300", f"click {preview}", "wait 300",
                    f"click {center('Palette(Explorer)')}", "key enter", "wait 400", "key cmd+,",
                    "wait 400", "dump {d}/a.txt"])
assert tb_line(d).startswith("toolbar=[Palette, Review]|[Settings, Explorer] toolbar_editor=None"), tb_line(d)
assert re.search(r'toolbar_right = \[\s*"settings",\s*"explorer",?\s*\]',
                 (d / "cfg/rift/config.toml").read_text()), (d / "cfg/rift/config.toml").read_text()
saved_tips = dict(tips((d / "a.txt").read_text()))
assert int(saved_tips["Settings"]) < int(saved_tips["File explorer"]) > 1000, saved_tips


def run_own(name, config, steps):
    """A headless run with its own config and data dirs (`scratch/<name>`);
    returns that directory."""
    d = scratch / name
    (d / "cfg/rift").mkdir(parents=True)
    (d / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n' + config)
    (d / "t.rift").write_text("\n".join(["resize 1200 600", "wait_prompt", *steps, "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"),
                                     XDG_DATA_HOME=str(d / "data")),
                            stdout=log, stderr=log, timeout=60).returncode
    assert rc == 0, died(f"{name} run failed", d / "log.txt")
    return d


# app icons: the picker writes `app_icon` and the app applies it; a config
# edit applies too
ai = scratch / "app-icon"
run_own("app-icon", "", [
    f"dump {ai}/a.txt", "key cmd+,", "wait 200", "type app icon", "wait 300",
    "click 960 158", "wait 200", "key down", "key down", "key enter", "wait 300",
    f"dump {ai}/b.txt",
    # (the picker wrote the key: replace it, GNU and BSD sed alike)
    f"sh sed -i.bak 's/^app_icon = .*/app_icon = \"neon\"/' {ai}/cfg/rift/config.toml",
    "wait 1500",
    f"dump {ai}/c.txt"])
at = lambda name: (ai / f"{name}.txt").read_text()
assert 'app_icon=Some("default")' in at("a"), at("a")
assert 'app_icon=Some("mono")' in at("b"), at("b")
assert 'app_icon=Some("neon")' in at("c"), at("c")

# block and tab settings: `block_spacing = "normal"` leaves a blank row
# above each later block header; the jump-to-bottom button shows over a
# block running past the pane and scrolls to its end; the × sits at the
# tab's left end; `tab_bar = "on_hover"` hides the strip until the pointer
# reaches the top edge, and hides it again once it leaves
g7 = scratch / "gap7"
run_own("gap7", 'block_spacing = "normal"\nblock_dividers = true\n'
        'tab_close_side = "left"\n', [
    "type echo one\\n", "wait 400", "type seq 1 80\\n", "wait 600", "type false\\n", "wait 400",
    f"dump {g7}/spaced.txt",
    "move 450 300", "scroll 8", "wait 200", "move 450 330", "wait 200", f"dump {g7}/over.txt",
    # the × shows on the hovered tab, at its left end
    "move 200 17", "wait 200", f"dump {g7}/tab-hover.txt"])
btt = lambda name: (g7 / f"{name}.txt").read_text()
kinds = re.search(r"row_kinds=(\S*)", btt("spaced")).group(1)
# the blocks below the first: a pad right above each header
assert "_H" in kinds, kinds
assert all(kinds[i - 1] == "_" for i, k in enumerate(kinds) if k == "H" and i > 0
           and "G" in kinds[:i]), kinds
jump = re.search(r'"Jump to the bottom of this block@(\d+),(\d+),(\d+),(\d+)"', btt("over"))
assert jump, btt("over")
jx, jy, jw, jh = map(int, jump.groups())
assert re.search(r'"Close tab@1\d\d,', btt("tab-hover")), btt("tab-hover")
# the same run again, pressing the button where the first one drew it
run_own("gap7-1", 'block_spacing = "normal"\nblock_dividers = true\n'
        'tab_close_side = "left"\n', [
    "type echo one\\n", "wait 400", "type seq 1 80\\n", "wait 600", "type false\\n", "wait 400",
    "move 450 300", "scroll 8", "wait 200", "move 450 330", "wait 200",
    f"dump {scratch}/gap7-1/before.txt",
    f"click {jx + jw // 2} {jy + jh // 2}", "wait 300", f"dump {scratch}/gap7-1/after.txt"])
bt1 = lambda name: (scratch / "gap7-1" / f"{name}.txt").read_text()
offset = lambda text: int(re.search(r"view_offset=(\d+)", text).group(1))
assert offset(bt1("before")) > 0 and offset(bt1("after")) < offset(bt1("before")), \
    (offset(bt1("before")), offset(bt1("after")))
assert "Jump to the bottom" not in bt1("after"), bt1("after")
run_own("gap7-2", 'tab_bar = "on_hover"\n', [
    "type echo hi\\n", "wait 400", "move 400 200", "wait 100",
    f"dump {scratch}/gap7-2/before.txt",
    "move 400 2", "wait 100", f"dump {scratch}/gap7-2/edge.txt",
    "move 400 20", "wait 50", f"dump {scratch}/gap7-2/strip.txt",
    "move 400 200", "wait 100", f"dump {scratch}/gap7-2/left.txt"])
bt2 = lambda name: (scratch / "gap7-2" / f"{name}.txt").read_text()
assert "strip=hidden" in bt2("before") and '"New tab@' not in bt2("before"), bt2("before")
assert "strip=slid_in" in bt2("edge") and '"New tab@' in bt2("edge"), bt2("edge")
assert "strip=slid_in" in bt2("strip"), bt2("strip")
assert "strip=hidden" in bt2("left"), bt2("left")

# the command inspector: ⇧⌘I shows the card for the part at the cursor at
# once (a subcommand, an option's value), the next key puts it away
ci = scratch / "inspector"
run_own("inspector", "", [
    "type git commit -m hi", "wait 200", "key shift+cmd+i", "wait 200", f"dump {ci}/value.txt",
    # into "commit" (Home scrolls the blocks)
    *["key left"] * 10, "wait 100",
    f"dump {ci}/gone.txt", "key shift+cmd+i", "wait 200", f"dump {ci}/sub.txt",
    "key shift+cmd+i", "wait 200", f"dump {ci}/toggled.txt"])
cit = lambda name: (ci / f"{name}.txt").read_text()
assert 'tooltip=Some("hi | Option value · The value of -m, --message' in cit("value"), cit("value")
assert "tooltip=None" in cit("gone"), cit("gone")
assert 'tooltip=Some("commit | Subcommand · ' in cit("sub"), cit("sub")
assert "tooltip=None" in cit("toggled"), cit("toggled")

# multi-cursor: ⌃G selects the word, then its next occurrence; typing
# renames both, Esc goes back to one caret, and ⌘Z undoes per keystroke;
# ⌃⇧↓ adds a caret on the next line of a multi-line command
mc = scratch / "multi-cursor"
run_own("multi-cursor", "", [
    "type cp foo.txt foo.bak", *["key left"] * 12, "key ctrl+g", "key ctrl+g", "wait 100",
    f"dump {mc}/selected.txt", "type bar", "wait 100", f"dump {mc}/typed.txt",
    "key escape", "wait 100", f"dump {mc}/one.txt",
    "key cmd+a", "key backspace", "type echo a", "key shift+enter", "type echo b",
    "key cmd+up", "key ctrl+shift+down", "type !", "wait 100", f"dump {mc}/lines.txt"])
mct = lambda name: (mc / f"{name}.txt").read_text()
assert 'editor_selection=Some("foo") carets=[(14, Some(11))]' in mct("selected"), mct("selected")
assert 'editor="cp bar.txt bar.bak"' in mct("typed") and "carets=[(14, None)]" in mct("typed"), \
    mct("typed")
assert "carets=[]" in mct("one"), mct("one")
assert 'editor="!echo a\\n!echo b"' in mct("lines"), mct("lines")

# Warp's editor keys: Meta+. inserts the last word of the previous command
# (again: of the one before, quotes kept), ⌃⌥←/→ step camelCase and
# snake_case parts, ⌃U copies what it clears
ek = scratch / "editor-keys"
run_own("editor-keys", "option_as_meta = true\n", [
    "type ls 'my file'\\n", "wait 500", "type echo one two\\n", "wait 500",
    "key alt+.", f"dump {ek}/1.txt", "key alt+.", f"dump {ek}/2.txt",
    "key ctrl+u", f"dump {ek}/3.txt",
    "type fooBar_baz", "key ctrl+alt+left", f"dump {ek}/4.txt",
    "key ctrl+alt+left", f"dump {ek}/5.txt", "key ctrl+alt+right", f"dump {ek}/6.txt"])
ekt = lambda name: (ek / f"{name}.txt").read_text()
ek_line = lambda name: re.search(r'editor="((?:[^"\\]|\\.)*)" cursor=(\d+)', ekt(name)).groups()
assert ek_line("1") == ("two", "3"), ek_line("1")
assert ek_line("2") == ("'my file'", "9"), ek_line("2")
assert ek_line("3") == ("", "0") and "clipboard=\"'my file'\"" in ekt("3"), ekt("3")[-2000:]
assert [ek_line(n)[1] for n in ("4", "5", "6")] == ["7", "3", "6"], [ek_line(n) for n in "456"]

# program-start-size: programs see their running viewport before their first instruction, not
# the shorter command-editor grid; every submission follows the same order
ps = scratch / "program-start-size"
ps_steps = []
for i in range(10):
    ps_steps += [
        f"type stty size > {ps}/{i}.size; touch {ps}/{i}.ready; while [ ! -f {ps}/{i}.go ]; do sleep 0.01; done\\n",
        f"wait_file {ps}/{i}.ready 10000", "wait 100", f"dump {ps}/{i}.txt",
        f"sh touch {ps}/{i}.go", "wait_prompt",
    ]
run_own("program-start-size", "", ps_steps)
for i in range(10):
    captured = (ps / f"{i}.txt").read_text()
    grid = re.search(r"grid x=.* cols=(\d+) rows=(\d+)", captured)
    assert grid, captured
    initial = [int(n) for n in (ps / f"{i}.size").read_text().split()]
    assert initial == [int(grid[2]), int(grid[1])], (i, initial, grid.groups())

# mark mode (⌃⇧M / ⇧⌘M): a cursor in the scrollback, ⇧ extends, ⏎ copies;
# g goes to the first row (the viewport follows), Esc leaves no selection;
# in the alternate screen too; another app shortcut (⌘T) leaves and runs
mk = scratch / "mark-mode"
(scratch / "mark-alt.txt").write_text("".join(f"alt line {i}\n" for i in range(1, 101)))
run_own("mark-mode", "", [
    "type seq 1 200\\n", "wait 1000", "key ctrl+shift+m", f"dump {mk}/on.txt",
    "key 0", "key shift+up", "key shift+k", f"dump {mk}/sel.txt", "key enter", f"dump {mk}/copied.txt",
    "key ctrl+shift+m", "key g", f"dump {mk}/top.txt", "key shift+l", "key escape", f"dump {mk}/esc.txt",
    f"type less {scratch}/mark-alt.txt\\n", "wait 1500",
    "key ctrl+shift+m", "key g", "key shift+j", "key shift+end", "key cmd+c", f"dump {mk}/alt.txt",
    "key q", "wait 500", "key ctrl+shift+m", "key shift+up", "key cmd+t", "wait_prompt",
    f"dump {mk}/shortcut.txt"])
mkt = lambda name: (mk / f"{name}.txt").read_text()
mk_clip = lambda name: json.loads(next(l for l in mkt(name).splitlines()
                                       if l.startswith("clipboard="))[len("clipboard="):])
assert "mark=on " in mkt("on") and "Mark mode: arrow keys move" in mkt("on"), mkt("on")[-3000:]
assert re.search(r"^mark=on .* anchor=Some", mkt("sel"), re.M), mkt("sel")[-3000:]
assert mk_clip("copied") == "198\n199\n2" and "mark=off" in mkt("copied"), mk_clip("copied")
assert "selection=false" in mkt("copied"), mkt("copied")[:3000]
assert not re.search(r"view_offset=0 ", mkt("top")), mkt("top")[:3000]
assert "mark=off" in mkt("esc") and "selection=true" not in mkt("esc"), mkt("esc")[:3000]
assert mk_clip("alt") == "alt line 1\nalt line 2", mk_clip("alt")
assert "mark=off" in mkt("shortcut") and re.search(r"^tabs=2 active=1 ", mkt("shortcut"), re.M) \
    and "selection=true" not in mkt("shortcut"), mkt("shortcut")[:3000]

# [[actions]]: a send runs at the prompt; split-then-send types into the NEW
# pane though focus moved back before its shell started; a failing step
# stops the rest with a toast; the palette lists each with its chord. A send
# into a read-only pane, or one kept after its shell exited, fails the step
# (the refused line never labels the pane's next block); ⌥. and ⌃U leave a
# read-only command line as it is, a subword move still moves
ua = scratch / "user-actions"
run_own("user-actions", '\n'.join([
    'close_on_exit = "never"',
    '[[actions]]', 'name = "Say"', 'key = "ctrl+alt+s"', 'steps = [{ send = "echo said-$((6*7))\\n" }]',
    '[[actions]]', 'name = "Split say"', 'key = "ctrl+alt+h"',
    'steps = [{ action = "split_vertical" }, { send = "echo in-new-pane\\n" }]',
    '[[actions]]', 'name = "Broken"', 'key = "ctrl+alt+b"',
    'steps = [{ open = "no such layout" }, { send = "echo never-ran\\n" }]', '']), [
    "key ctrl+alt+s", "wait 800", f"dump {ua}/send.txt",
    "key ctrl+alt+h", "key alt+cmd+left", "wait 4000", f"dump {ua}/split.txt",
    "key ctrl+alt+b", "wait 800", f"dump {ua}/broken.txt",
    "key cmd+p", "type Action:", f"dump {ua}/palette.txt", "key escape",
    "type keep-me", "key cmd+p", "type toggle read-only", "wait 200", "key enter", "wait 300",
    "key ctrl+alt+s", "wait 600", "key alt+.", "key ctrl+u", "key ctrl+alt+left", "wait 200",
    f"dump {ua}/ro.txt",
    "key cmd+p", "type toggle read-only", "wait 200", "key enter", "wait 300",
    "key ctrl+u", "type echo after-ro\\n", "wait 800", f"dump {ua}/ro-after.txt",
    "type exit 3\\n", "wait 1200", "key ctrl+alt+s", "wait 600", f"dump {ua}/exited.txt"])
uat = lambda name: (ua / f"{name}.txt").read_text()
assert re.search(r'cmd="echo said-\$\(\(6\*7\)\)" exit=Some\(0\)', uat("send")), uat("send")[-3000:]
assert "said-42" in uat("send"), uat("send")[-3000:]
split = uat("split")
assert re.search(r"active_pane=1 ", split), split[:3000]
pane2 = split.split("  pane 2 ", 1)[1]
assert re.search(r'^    block 0 state=done .*cmd="echo in-new-pane" exit=Some\(0\)', pane2, re.M), pane2[:3000]
assert 'stopped at step 1 (open \\"no such layout\\")' in uat("broken"), uat("broken")[-3000:]
assert "never-ran" not in uat("broken"), uat("broken")[-3000:]
assert 'ListBoxOption "Action: Say, ⌃ ⌥ S"' in uat("palette"), uat("palette")[-3000:]
assert "): the pane is read-only" in uat("ro"), uat("ro")[-3000:]
ua_pane1 = lambda name: uat(name).split("  pane 1 ", 1)[1].split("\n  pane ", 1)[0]
assert ' editor="keep-me" cursor=5 ' in ua_pane1("ro").split("\n", 1)[0], ua_pane1("ro")[:2000]
assert re.search(r'^    block 1 state=done .*cmd="echo after-ro" exit=Some\(0\)', ua_pane1("ro-after"), re.M), \
    ua_pane1("ro-after")[:3000]
assert "): the pane's program has exited" in uat("exited"), uat("exited")[-3000:]

# ⌃Tab in most-recently-used order: a tap toggles the two last tabs, ⌃ held
# shows the switcher (a11y dialog) and its release switches, Esc cancels;
# `in_order` (the default) steps the strip. The switcher lists the shown
# tabs only (not one folded in a collapsed group), and a tab closing while
# it shows does not shift the pick onto another tab
mru = scratch / "tab-mru"
mru_steps = ["key cmd+t", "wait_prompt", "key cmd+t", "wait_prompt", "key cmd+1", "wait 200",
             "key ctrl+tab", f"dump {{d}}/tap.txt", "mods ctrl", "key ctrl+tab", "key ctrl+tab",
             f"dump {{d}}/held.txt", "mods none", f"dump {{d}}/released.txt",
             "mods ctrl", "key ctrl+tab", "key escape", "mods none", f"dump {{d}}/esc.txt"]
run_own("tab-mru", 'tab_switch_order = "mru"\n', [s.format(d=mru) for s in mru_steps])
mrt = lambda name: (mru / f"{name}.txt").read_text()
mr_active = lambda name: re.search(r"^tabs=3 active=(\d)", mrt(name), re.M)[1]
assert mr_active("tap") == "2", mrt("tap")[:500]
assert 'Dialog "Recent tabs"' in mrt("held") and "3 of 3, release Control" in mrt("held"), \
    mrt("held")[-3000:]
assert mr_active("held") == "2" and mr_active("released") == "1", mrt("released")[:500]
assert mr_active("esc") == "1" and "tab_switcher=None" in mrt("esc"), mrt("esc")[:500]
mrg = run_own("tab-mru-groups", 'tab_switch_order = "mru"\n', [
    "key cmd+t", "wait_prompt", "key cmd+t", "wait_prompt", "rclick 450 17", "wait 200",
    "a11y click Group tab", "wait 300", "a11y click Group 1 group, 1 tabs", "wait 300",
    "mods ctrl", "key ctrl+tab", f"dump {scratch}/tab-mru-groups/held.txt", "key escape", "mods none"])
mrg_held = (mrg / "held.txt").read_text()
assert 'groups=["Group 1:red:collapsed:1"]' in mrg_held and "of 2, release Control" in mrg_held, \
    mrg_held[-3000:]
# tabs A B C; A's shell ends while the switcher (C, B, A) has B picked
mrc = run_own("tab-mru-close", 'tab_switch_order = "mru"\n', [
    "key cmd+t", "wait_prompt", "key cmd+t", "wait_prompt", "key cmd+1", "wait 200",
    "type sleep 2; exit\\n", "key cmd+2", "wait 200", "key cmd+3", "wait 200",
    "mods ctrl", "key ctrl+tab", "wait 3500", f"dump {scratch}/tab-mru-close/held.txt",
    "mods none", "wait 200", f"dump {scratch}/tab-mru-close/released.txt"])
mrc_held = (mrc / "held.txt").read_text()
assert re.search(r"^tabs=2 active=1 ", mrc_held, re.M) and "of 2, release Control" in mrc_held, \
    mrc_held[-3000:]
assert re.search(r"^tabs=2 active=0 ", (mrc / "released.txt").read_text(), re.M), \
    (mrc / "released.txt").read_text()[:1500]
io = scratch / "tab-in-order"
run_own("tab-in-order", "", [s.format(d=io) for s in mru_steps])
iot = lambda name: re.search(r"^tabs=3 active=(\d)", (io / f"{name}.txt").read_text(), re.M)[1]
assert [iot(n) for n in ("tap", "released", "esc")] == ["1", "0", "1"], \
    [iot(n) for n in ("tap", "released", "esc")]

# background blocks: `cmd &` is a block of its own (bash 3.2 never marks it
# started), and what the job prints while the prompt waits goes into a
# background block before the prompt, not under the next command's header
bgb = scratch / "background"
run_own("background", "", [
    "type (sleep 1; echo bg-out-1; echo bg-out-2) &\\n", "wait 2500", f"dump {bgb}/at-prompt.txt",
    "type echo next\\n", "wait 800", f"dump {bgb}/after.txt",
    # ^C at the prompt: the shell's echo and new prompt are its own
    "key ctrl+c", "wait 600", "key ctrl+c", "wait 600", f"dump {bgb}/interrupt.txt"])
bgt = lambda name: (bgb / f"{name}.txt").read_text()
blocks_of = lambda text: re.findall(r"^    block \d+ (.*)$", text, re.M)
at_prompt = blocks_of(bgt("at-prompt"))
assert any('cmd="(sleep 1; echo bg-out-1; echo bg-out-2) &"' in b and "state=done" in b
           for b in at_prompt), at_prompt
assert any(b.endswith("background=true") for b in at_prompt), at_prompt
assert "state=prompt" in at_prompt[-1], at_prompt
after = blocks_of(bgt("after"))
# the job's lines stay in their block: between it and the next command
kinds = [("bg" if b.endswith("background=true") else re.search(r'cmd="([^"]*)"', b).group(1))
         for b in after]
assert kinds[-3:-1] == ["bg", "echo next"], kinds
bg_line = next(b for b in after if b.endswith("background=true"))
start, end = re.search(r"output_start=Some\((\d+)\) end=Some\((\d+)\)", bg_line).groups()
assert int(end) - int(start) >= 2, bg_line
assert len([b for b in blocks_of(bgt("interrupt")) if b.endswith("background=true")]) == 1, \
    blocks_of(bgt("interrupt"))

# subshells: a command that starts a shell without Rift's hooks (here a
# `bash --norc` added to subshell_commands) shows the offer after it has run
# a moment; ⌘⏎ types the hooks in, the command's block folds, the subshell's
# commands get blocks with exit codes, and `exit` returns to the outer one
ss = scratch / "subshell"
# Enable's chord: ⌘⏎ on macOS, Ctrl+Shift+Enter elsewhere (app/subshell.rs)
enable_chord = "cmd+enter" if sys.platform == "darwin" else "ctrl+shift+enter"
run_own("subshell", 'subshell_commands = ["^bash --norc$"]\n', [
    "type echo before\\n", "wait 500",
    "type bash --norc\\n", "wait 3000", f"dump {ss}/offer.txt",
    # a full-screen program in the subshell hides the bar, and ⌘⏎ there
    # types nothing into it
    "type printf '\\\\033[?1049h'; sleep 2; printf '\\\\033[?1049l'\\n", "wait 700",
    f"dump {ss}/alt.txt", f"key {enable_chord}", "wait 300", f"dump {ss}/alt-enter.txt", "wait 2000",
    f"dump {ss}/back.txt",
    # the hooks are typed in, then the subshell's first prompt (a fixed wait
    # dumped mid-injection under the gates' parallel load)
    f"key {enable_chord}", "wait_prompt", "wait 300", f"dump {ss}/enabled.txt",
    "type false\\n", "wait 800", f"dump {ss}/inner.txt",
    "type exit\\n", "wait 1000", f"dump {ss}/outer.txt"])
sst = lambda name: (ss / f"{name}.txt").read_text()
assert 'subshell=Some(("bash --norc", true))' in sst("offer"), sst("offer")
assert '"Use Rift\'s blocks and editor in this subshell' in sst("offer") or \
    "Use Rift's blocks and editor in this subshell" in sst("offer"), sst("offer")
# (the pane's `subshell=` sits a few dump lines below its `alt=`)
assert re.search(r"alt=true .*\n(?:.*\n){0,12}?.*subshell=Some\(\(\"bash --norc\", false\)\)",
                 sst("alt")), sst("alt")
assert 'subshell=Some(("bash --norc", false))' in sst("alt-enter"), sst("alt-enter")
assert 'subshell=Some(("bash --norc", true))' in sst("back"), sst("back")
assert "subshell=None" in sst("enabled"), sst("enabled")
assert re.search(r'cmd="bash --norc" exit=None collapsed=true', sst("enabled")), sst("enabled")
assert re.search(r"^    block \d+ state=prompt", sst("enabled"), re.M), sst("enabled")
assert re.search(r'cmd="false" exit=Some\(1\)', sst("inner")), sst("inner")
assert re.search(r"^    block \d+ state=prompt", sst("outer"), re.M), sst("outer")

# terminal text: box drawing, Powerline and legacy-computing blocks drawn as
# geometry (art_cells); the text options reach the grid (grid_text); the
# double-underline cursor turns hollow while the window is unfocused; with
# ambiguous_width = "wide" ± and α take two cells and Powerline stays one.
# The default run is the control: options off, one-cell ±, a block cursor
# whatever the focus.
tr = scratch / "text-render"
tr.mkdir()
(tr / "art.sh").write_text(
    "printf '┌─┬┐\\n│▀│\\n└─┴┘\\n\\356\\202\\260\\360\\237\\254\\200\\n±|α|x\\n'\n")
tr_steps = lambda d: [
    f"type bash {tr}/art.sh\\n", "wait 700", "type cat\\n", "wait 300",
    "focus on", "wait 200", f"dump {d}/focused.txt",
    "focus off", "wait 200", f"dump {d}/unfocused.txt",
    "focus on", "key ctrl+c", "wait 200"]
run_own("text-render/on",
        'cursor_style = "double_underline"\ncursor_hollow_unfocused = true\n'
        'font_weight = 500\nfont_features = ["zero", "-calt", "bogus"]\n'
        'minimum_contrast = 4.5\nbold_is_bright = true\nambiguous_width = "wide"\n',
        tr_steps(tr / "on"))
run_own("text-render/off", "", tr_steps(tr / "off"))
trs = lambda run, name: (tr / run / f"{name}.txt").read_text()
for run in ("on", "off"):
    # ┌─┬┐ │▀│ └─┴┘, the Powerline arrow and a sextant: 13 cells
    assert "art_cells=13 " in trs(run, "focused"), trs(run, "focused")
assert "grid_cursor=Some(DoubleUnderline)" in trs("on", "focused"), trs("on", "focused")
assert "grid_cursor=Some(Hollow)" in trs("on", "unfocused"), trs("on", "unfocused")
assert 'grid_text=Some(((500, 700), ["zero=1", "calt=0"], 4.5, true))' in trs("on", "focused"), \
    trs("on", "focused")
assert re.search(r"^    \| ± \|α \|x$", trs("on", "focused"), re.M), trs("on", "focused")
for name in ("focused", "unfocused"):
    assert "grid_cursor=Some(Block)" in trs("off", name), trs("off", name)
assert "grid_text=Some(((400, 700), [], 1.0, false))" in trs("off", "focused"), trs("off", "focused")
assert re.search(r"^    \| ±\|α\|x$", trs("off", "focused"), re.M), trs("off", "focused")

# --- window and input options: padding per side and none around a
# full-screen program, the drop-down window's edge and width, Dock bounce /
# urgency on a bell while no window has focus, the large-paste question and
# trim, word delimiters, the resize overlay and snap to cells, ⌘ + wheel
# zoom and opacity, the opacity actions, always on top, launch mode, the
# default window size and named windows
def wo_run(name, config, steps, resize=True, **extra):
    """A headless run like run_own (`scratch/wo-<name>`), optionally without
    the first resize and with more environment; returns its directory."""
    d = scratch / f"wo-{name}"
    (d / "cfg/rift").mkdir(parents=True)
    # a key the config sets is not written twice (TOML refuses the file)
    base = [l for l in ('shell = "/bin/bash"', "restore_session = false", "durable_sessions = false")
            if l.split(" =")[0] + " =" not in config]
    (d / "cfg/rift/config.toml").write_text("\n".join(base) + "\n" + config)
    first = ["resize 1200 600"] if resize else []
    (d / "t.rift").write_text("\n".join([*first, "wait_prompt", *steps, "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"),
                                     XDG_DATA_HOME=str(d / "data"), **extra),
                            stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"wo-{name} run failed", d / "log.txt")
    return d
wo_dump = lambda d, name: (d / f"{name}.txt").read_text()
def wo_grid(text):
    rect = re.search(r"^  pane_rect 1 x=(\S+) y=(\S+) w=(\S+) h=(\S+)$", text, re.M)
    grid = re.search(r"^    grid x=(\S+) y=(\S+) right=(\S+) cols=(\d+) rows=(\d+)$", text, re.M)
    assert rect and grid, text[:3000]
    return [float(v) for v in rect.groups()], [float(v) for v in grid.groups()]
wo_line = lambda text: next(l for l in text.splitlines() if l.startswith("window_name="))
wo_steps = lambda d, steps: [s.format(d=d) for s in steps]
# Rust's round (half away from zero) for the non-negative sizes below
wo_round = lambda v: int(v + 0.5)

# padding per side; a full-screen program keeps it unless alt_screen_padding
# is off (then its grid fills the pane); window_padding_bottom is the gap
# under its last row
alt_cmd = "type printf '\\\\033[?1049h'; sleep 3; printf '\\\\033[?1049l'\\n"
pad_steps = ["wait 300", "dump {d}/prompt.txt", alt_cmd, "wait 1000", "dump {d}/alt.txt", "wait 2500"]
pd = wo_run("pad-default", "", wo_steps(scratch / "wo-pad-default", pad_steps))
ps = wo_run("pad-set", "window_padding_left = 30\nwindow_padding_right = 2\nwindow_padding_top = 40\n"
            "window_padding_bottom = 30\n", wo_steps(scratch / "wo-pad-set", pad_steps))
pa = wo_run("pad-alt", "alt_screen_padding = false\n", wo_steps(scratch / "wo-pad-alt", pad_steps))
(rx, ry, rw, rh), (gx, gy, gr, _, _) = wo_grid(wo_dump(pd, "prompt"))
assert (gx - rx, rx + rw - gr, gy - ry) == (8, 8, 14), wo_dump(pd, "prompt")[:3000]
(rx, ry, rw, rh), (gx, gy, gr, _, _) = wo_grid(wo_dump(ps, "prompt"))
assert (gx - rx, rx + rw - gr, gy - ry) == (30, 2, 40), wo_dump(ps, "prompt")[:3000]
# the alternate screen: padded by default, bare with alt_screen_padding off
for d in (pd, pa, ps):
    assert " alt=true " in wo_dump(d, "alt"), wo_dump(d, "alt")[:3000]
(rx, ry, rw, rh), (gx, gy, gr, _, rows_default) = wo_grid(wo_dump(pd, "alt"))
assert (gx - rx, rx + rw - gr) == (8, 8), wo_dump(pd, "alt")[:3000]
(rx, ry, rw, rh), (gx, gy, gr, _, rows_bare) = wo_grid(wo_dump(pa, "alt"))
assert (gx, gr) == (rx, rx + rw) and rows_bare > rows_default, wo_dump(pa, "alt")[:3000]
line_h = float(re.search(r"line_h=([\d.]+)", wo_dump(ps, "alt"))[1])
(rx, ry, rw, rh), (gx, gy, gr, _, rows_set) = wo_grid(wo_dump(ps, "alt"))
assert gy + rows_set * line_h <= ry + rh - 30 + 0.5 and rows_set < rows_default, wo_dump(ps, "alt")[:3000]

# the drop-down window at the bottom edge, half the screen wide and centred;
# at the right edge its depth is a width
for edge, screen_cfg in (("bottom", "pointer"), ("right", "primary")):
    hd = wo_run(f"hotkey-{edge}", f'hotkey_window = true\nhotkey_window_edge = "{edge}"\n'
                f'hotkey_window_width = 50\nhotkey_window_height = 40\n'
                f'hotkey_window_screen = "{screen_cfg}"\n',
                ["hotkey", "wait_prompt", "wait 300", f"dump {scratch}/wo-hotkey-{edge}/shown.txt"],
                resize=False)
    line = re.search(r"^window 1: .*kind=dropdown shown=true sliding=false .*$", wo_dump(hd, "shown"), re.M)
    assert line and line[0].endswith(f"edge={edge.capitalize()}"), wo_dump(hd, "shown")[:600]
    nums = lambda key: [float(v) for v in re.search(key + r"=Some\(\[([^\]]+)\]\)", line[0])[1].split(",")]
    (fx, fy, fw, fh), (sx, sy, sw, sh), size = nums("frame"), nums("screen"), nums("size")
    if edge == "bottom":
        want = [sx + wo_round((sw - wo_round(sw * 0.5)) / 2), sy + sh - wo_round(sh * 0.4),
                wo_round(sw * 0.5), wo_round(sh * 0.4)]
    else:
        want = [sx + sw - wo_round(sw * 0.4), sy + wo_round((sh - wo_round(sh * 0.5)) / 2),
                wo_round(sw * 0.4), wo_round(sh * 0.5)]
    assert [fx, fy, fw, fh] == want, (line[0], want)
    assert size == [fw, fh], line[0]

# a bell while no window has focus asks for attention; a focused window is
# never interrupted; off (the default) never asks
bell = "type printf '\\\\a'\\n"
at_steps = ["focus on", bell, "wait 500", "dump {d}/focused.txt", "focus off", bell, "wait 500",
            "dump {d}/unfocused.txt"]
ao = wo_run("attention", "request_attention = true\n", wo_steps(scratch / "wo-attention", at_steps))
an = wo_run("attention-off", "", wo_steps(scratch / "wo-attention-off", at_steps))
requests = lambda d, name: int(re.search(r"^attention_requests=(\d+)$", wo_dump(d, name), re.M)[1])
assert requests(ao, "focused") == 0 and requests(ao, "unfocused") >= 1, wo_dump(ao, "unfocused")[:600]
assert requests(an, "unfocused") == 0, wo_dump(an, "unfocused")[:600]

# a paste over large_paste_warning asks first; Paste goes on; a small one
# does not ask; a read-only pane refuses it before asking
big = "y" * 1500
lp = scratch / "wo-large-paste"
wo_run("large-paste", "large_paste_warning = 1\n", [
    f"paste {big}", f"dump {lp}/asked.txt", "a11y click Paste", "wait 200", f"dump {lp}/pasted.txt",
    "key cmd+a", "key backspace", "paste short", f"dump {lp}/small.txt",
    "key cmd+a", "key backspace",
    "key cmd+p", "type toggle read-only", "wait 200", "key enter", "wait 300",
    f"paste {big}", f"dump {lp}/read-only.txt"])
asked = wo_dump(lp, "asked")
assert "confirm=Some((LargePaste {" in asked and ' editor="" ' in asked, asked[:3000]
assert "Paste a large amount of text?" in asked, asked[-3000:]
assert f' editor="{big}" ' in wo_dump(lp, "pasted") and "confirm=None" in wo_dump(lp, "pasted"), \
    wo_dump(lp, "pasted")[:3000]
assert ' editor="short" ' in wo_dump(lp, "small") and "confirm=None" in wo_dump(lp, "small"), \
    wo_dump(lp, "small")[:3000]
ro = wo_dump(lp, "read-only")
assert "confirm=None" in ro and ' editor="" ' in ro and "read_only=true" in ro, ro[:3000]

# trim_paste drops trailing blanks (a copied line does not run); copied
# files' quoted paths keep the space after them; off pastes as is
tp_steps = ["paste echo trimmed  \\n\\n", "dump {d}/text.txt", "key cmd+a", "key backspace",
            f"copyfiles {work}/hello.rs", "key cmd+v", "dump {d}/files.txt"]
tp = wo_run("trim", "trim_paste = true\n", wo_steps(scratch / "wo-trim", tp_steps))
tn = wo_run("trim-off", "", wo_steps(scratch / "wo-trim-off", tp_steps))
assert ' editor="echo trimmed" ' in wo_dump(tp, "text"), wo_dump(tp, "text")[:3000]
assert ' editor="echo trimmed" ' not in wo_dump(tn, "text"), wo_dump(tn, "text")[:3000]
assert re.search(r' editor="[^"]*hello\.rs ', wo_dump(tp, "files")), wo_dump(tp, "files")[:3000]

# word_delimiters: ⌥← stops after a delimiter and a double-click selects
# between two; without, a blank is the only stop and the word is whole
wd_steps = ["type echo alpha=beta=gamma\\n", "wait 500", "dump {d}/out.txt",
            "paste cd src=app=main", "key alt+left", "dump {d}/word.txt"]
wd_cfg = 'word_delimiters = "="\n'
wd = wo_run("words", wd_cfg, wo_steps(scratch / "wo-words", wd_steps))
wn = wo_run("words-off", "", wo_steps(scratch / "wo-words-off", wd_steps))
assert ' editor="cd src=app=main" cursor=11 ' in wo_dump(wd, "word"), wo_dump(wd, "word")[:3000]
assert ' editor="cd src=app=main" cursor=3 ' in wo_dump(wn, "word"), wo_dump(wn, "word")[:3000]
def wo_dblclick(d, name, config):
    """Double-click "beta" in the echo's output row (where run `d` drew it)
    in a run of its own; what it copied."""
    text = wo_dump(d, "out")
    start = re.search(r'cmd="echo alpha=beta=gamma" .*?output_start=Some\((\d+)\)', text)[1]
    geometry = next(l for l in text.splitlines() if l.strip().startswith("geometry "))
    cw = float(re.search(r"cell_w=([\d.]+)", geometry)[1])
    y, x = re.search(rf"[\[ ]{start}:([\d.]+)@([\d.]+)", geometry).groups()
    point = f"{float(x) + cw * 7.5:.1f} {float(y) + 4:.1f}"
    c = wo_run(name, config, ["type echo alpha=beta=gamma\\n", "wait 500", f"dblclick {point}",
                              "wait 200", f"dump {scratch}/wo-{name}/clicked.txt"])
    line = next(l for l in wo_dump(c, "clicked").splitlines() if l.startswith("clipboard="))
    return json.loads(line[len("clipboard="):])
assert wo_dblclick(wd, "words-click", wd_cfg) == "beta"
assert wo_dblclick(wn, "words-off-click", "") == "alpha=beta=gamma"

# the resize overlay (the grid's cols × rows) while a resize lasts, gone a
# second later; snap_to_cells makes the window's resize step one cell; both
# off by default
ro_steps = ["wait 1500", "resize 900 500", "wait 300", "dump {d}/resized.txt", "wait 1500",
            "dump {d}/later.txt"]
rs = wo_run("resize", "resize_overlay = true\nsnap_to_cells = true\n",
            wo_steps(scratch / "wo-resize", ro_steps))
rn = wo_run("resize-off", "", wo_steps(scratch / "wo-resize-off", ro_steps))
resized = wo_dump(rs, "resized")
_, (_, _, _, cols, rows) = wo_grid(resized)
assert f'resize_overlay=Some("{cols:.0f} × {rows:.0f}")' in wo_line(resized), wo_line(resized)
assert "resize_overlay=None" in wo_line(wo_dump(rs, "later")), wo_line(wo_dump(rs, "later"))
geometry = next(l for l in resized.splitlines() if l.strip().startswith("geometry "))
cw, lh = (float(re.search(k + r"=([\d.]+)", geometry)[1]) for k in ("cell_w", "line_h"))
step = re.search(r'resize_increments=Some\("([\d.]+)x([\d.]+)"\)', wo_line(resized))
assert step and abs(float(step[1]) - cw) < 0.5 and abs(float(step[2]) - lh) < 0.5, (wo_line(resized), cw, lh)
assert "resize_overlay=None" in wo_line(wo_dump(rn, "resized")), wo_line(wo_dump(rn, "resized"))
assert "resize_increments=None" in wo_line(wo_dump(rn, "resized")), wo_line(wo_dump(rn, "resized"))

# ⌘ + wheel zooms the font (Ctrl + wheel off macOS), ⇧ added steps the
# opacity; the opacity actions by their keybinding names; off (the default)
# the wheel only scrolls
zoom_mod = "cmd" if sys.platform == "darwin" else "ctrl"
wz_steps = [f"mods {zoom_mod}", "scroll 3", "mods none", "dump {d}/zoomed.txt",
            f"mods {zoom_mod}+shift", "scroll -3", "scroll -3", "mods none", "dump {d}/dimmer.txt",
            "key ctrl+alt+o", "dump {d}/action.txt", "key ctrl+alt+r", "dump {d}/reset.txt"]
wz_keys = '\n[keybindings]\nopacity_increase = "ctrl+alt+o"\nopacity_reset = "ctrl+alt+r"\n'
wz = wo_run("zoom", "wheel_zoom = true\n" + wz_keys, wo_steps(scratch / "wo-zoom", wz_steps))
wzn = wo_run("zoom-off", wz_keys, wo_steps(scratch / "wo-zoom-off", wz_steps))
font = lambda d, name: float(re.search(r"^font_size=([\d.]+) ", wo_dump(d, name), re.M)[1])
opacity = lambda d, name: float(re.search(r" opacity=([\d.]+)$", wo_line(wo_dump(d, name)))[1])
assert font(wz, "zoomed") == 14 and opacity(wz, "dimmer") == 0.4, (font(wz, "zoomed"), opacity(wz, "dimmer"))
assert opacity(wz, "action") == 0.45 and opacity(wz, "reset") == 0.5, opacity(wz, "action")
assert re.search(r"^font_size = 14", (wz / "cfg/rift/config.toml").read_text(), re.M)
assert font(wzn, "zoomed") == 13 and opacity(wzn, "dimmer") == 0.5, (font(wzn, "zoomed"), opacity(wzn, "dimmer"))
assert opacity(wzn, "action") == 0.55, opacity(wzn, "action")

# always on top, launch mode and the size of a window without a saved frame;
# the palette toggles on top for the window; "Rename window" names it (its
# title), the session file keeps the name and a relaunch restores it
ws = scratch / "wo-window"
wo_run("window", 'always_on_top = true\nlaunch_mode = "maximized"\nwindow_width = 900\n'
       'window_height = 520\nrestore_session = true\n', [
           f"dump {ws}/opened.txt", "key cmd+p", "type toggle always on top", "wait 200", "key enter",
           f"dump {ws}/toggled.txt",
           "key cmd+p", "type rename window", "wait 200", "key enter", "wait 200", "type build logs",
           f"dump {ws}/typing.txt", "key enter", "wait 300", f"dump {ws}/named.txt",
           f"sh cp {ws}/data/rift/session.toml {ws}/session.toml"],
       resize=False, RIFT_PERSIST_SESSION="1")
opened = wo_line(wo_dump(ws, "opened"))
assert 'on_top=true launch=Maximized size=Some("900x520")' in opened, opened
assert "on_top=false" in wo_line(wo_dump(ws, "toggled")), wo_line(wo_dump(ws, "toggled"))
assert "Name this window “build logs”" in wo_dump(ws, "typing"), wo_dump(ws, "typing")[-3000:]
assert 'window_name=Some("build logs")' in wo_line(wo_dump(ws, "named")), wo_line(wo_dump(ws, "named"))
assert 'window_name = "build logs"' in (ws / "session.toml").read_text(), (ws / "session.toml").read_text()
wr = scratch / "wo-window-restore"
(wr / "data/rift").mkdir(parents=True)
shutil.copy(ws / "session.toml", wr / "data/rift/session.toml")
wo_run("window-restore", "restore_session = true\n", [f"dump {wr}/restored.txt"],
       resize=False, RIFT_PERSIST_SESSION="1")
assert 'window_name=Some("build logs") on_top=false launch=Default' in wo_line(wo_dump(wr, "restored")), \
    wo_line(wo_dump(wr, "restored"))

# mask_secrets: a window name's secret is masked in its title; the
# large-paste preview masks a private key's body lines (they match only
# with its header and footer); the multiline question masks its lines
ms = scratch / "wo-mask"
pem = "\\n".join(["-----BEGIN PRIVATE KEY-----", *["Q" * 64] * 18, "-----END PRIVATE KEY-----"])
wo_run("mask", "mask_secrets = true\nlarge_paste_warning = 1\n", [
    "key cmd+p", "type rename window", "wait 200", "key enter", "wait 200", "type token=abcd1234secret",
    "key enter", "wait 300", f"dump {ms}/named.txt",
    f"paste {pem}", "wait 300", f"dump {ms}/large.txt", "key escape", "wait 200",
    "type cat\\n", "wait 300", "paste password=hunter2xyz\\nls\\n", "wait 300", f"dump {ms}/multi.txt"])
assert 'title="token=••••••••••••••"' in wo_line(wo_dump(ms, "named")), wo_line(wo_dump(ms, "named"))
preview = re.search(r"^confirm=Some\(\(LargePaste \{.*?\}, (\[.*?\])\)\)", wo_dump(ms, "large"), re.M)
assert preview and "QQQQ" not in preview[1] and "PRIVATE" not in preview[1], (preview and preview[1])[:400]
assert '["password=••••••••••", "ls"]' in wo_dump(ms, "multi"), wo_dump(ms, "multi")[:3000]

# --- Tier 3 appearance: the theme picker previews live and reverts; a
# Warp gradient theme draws its ground and accent as gradients; tab colours
# take any hex, cycle, and follow [[tab_color_rules]]; the sticky header
# minimizes per pane and a click restores it; the explorer's right-click
# menu; the Markdown viewer's Run (asks, masks, refuses a read-only pane)
# and "Open in viewer" after `cat x.md` ---
t3 = scratch / "tier3"
def t3_run(name, config, steps, files=None):
    d = t3 / name
    for sub in ("cfg/rift/themes", "data", "work/sub"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    for rel, body in (files or {}).items():
        (d / rel).write_text(body)
    (d / "cfg/rift/config.toml").write_text(
        'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
        'sync_os_theme = false\n' + f'working_directory = {json.dumps(str(d / "work"))}\n'
        + config.replace("@WORK@", str(d / "work")))
    (d / "t.rift").write_text("\n".join(["resize 1200 600", "wait_prompt", *steps, "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"),
                                     XDG_DATA_HOME=str(d / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"tier3 {name} run failed", d / "log.txt")
    return d
t3_line = lambda d, name: re.search(r"^theme_preview=.*$", (d / f"{name}.txt").read_text(), re.M).group(0)
t3_text = lambda d, name: (d / f"{name}.txt").read_text()
t3_keys = ('\n[keybindings]\ntheme_picker = "ctrl+alt+t"\ncycle_tab_color = "ctrl+alt+c"\n'
           'set_tab_color = "ctrl+alt+h"\ntoggle_sticky_header = "ctrl+alt+s"\n'
           'toggle_read_only = "ctrl+alt+r"\n')
# (1) picker, tab colours, sticky header
d = t3 / "look"
d = t3_run("look", 'theme = "nord"\n\n[[tab_color_rules]]\ndir = "@WORK@"\ncolor = "#ff8800"\n' + t3_keys, [
    f"dump {d}/a0.txt",
    "key ctrl+alt+t", "wait 300", f"dump {d}/a1.txt",
    "type dracula", "wait 300", f"dump {d}/a2.txt",
    "key escape", "wait 300", f"dump {d}/a3.txt",
    "key ctrl+alt+t", "wait 200", "type gruvbox light", "key enter", "wait 300", f"dump {d}/a4.txt",
    cp(d / "cfg/rift/config.toml", d / "a4.toml"),
    "key ctrl+alt+c", "wait 200", f"dump {d}/a5.txt",
    "key ctrl+alt+h", "wait 200", "type #123abc", "key enter", "wait 200", f"dump {d}/a6.txt",
    "key ctrl+alt+h", "type no color", "key enter", "wait 200", f"dump {d}/a7.txt",
    "type seq 1 300\\n", "wait 1000", f"dump {d}/a8.txt",
    "key ctrl+alt+s", "wait 300", f"dump {d}/a9.txt",
    # the minimized header's tab (top right of the 1200 px grid)
    "click 1170 56", "wait 300", f"dump {d}/a10.txt"])
assert 'theme="nord"' in t3_text(d, "a0") and "tab_tints=[Some(\"#ff8800\")]" in t3_line(d, "a0"), t3_line(d, "a0")
assert "palette=true mode=Themes" in t3_text(d, "a1") and "theme_preview=true" in t3_line(d, "a1"), t3_line(d, "a1")
# the selection previews without saving; Esc puts the old theme back
assert 'theme="dracula"' in t3_text(d, "a2"), re.search(r"^font_size.*$", t3_text(d, "a2"), re.M)
assert 'theme="nord"' in t3_text(d, "a3") and "theme_preview=false" in t3_line(d, "a3"), t3_line(d, "a3")
assert 'theme="gruvbox-light"' in t3_text(d, "a4"), re.search(r"^font_size.*$", t3_text(d, "a4"), re.M)
assert 'theme = "gruvbox-light"' in (d / "a4.toml").read_text(), "Enter keeps (saves) the theme"
# own colours win over the directory rule; "No color" shows the rule again
assert 'tab_tints=[Some("red")]' in t3_line(d, "a5"), t3_line(d, "a5")
assert 'tab_tints=[Some("#123abc")]' in t3_line(d, "a6"), t3_line(d, "a6")
assert 'tab_tints=[Some("#ff8800")]' in t3_line(d, "a7"), t3_line(d, "a7")
assert "sticky=Some(" in t3_text(d, "a8"), "a finished long block keeps its sticky header"
assert re.search(r"sticky_minimized=\[\d+\] sticky_tab=\[\"1156,48,28,16\"\]", t3_line(d, "a9")), t3_line(d, "a9")
assert "sticky=None" in t3_text(d, "a9")
assert 'Button "Show sticky command header"' in t3_text(d, "a9"), "the minimized header's tab has a label"
assert "sticky_minimized=[]" in t3_line(d, "a10") and "sticky=Some(" in t3_text(d, "a10"), t3_line(d, "a10")
# (2) a Warp gradient theme: its stops survive and draw top to bottom
grad_yaml = ("accent:\n  left: '#ff0000'\n  right: '#0000ff'\nbackground:\n  top: '#101040'\n"
             "  bottom: '#402010'\nforeground: '#e0e0e0'\ndetails: darker\nterminal_colors:\n"
             "  normal: {black: '#000000', red: '#cc0000', green: '#00cc00', yellow: '#cccc00', "
             "blue: '#0000cc', magenta: '#cc00cc', cyan: '#00cccc', white: '#cccccc'}\n"
             "  bright: {black: '#555555', red: '#ff5555', green: '#55ff55', yellow: '#ffff55', "
             "blue: '#5555ff', magenta: '#ff55ff', cyan: '#55ffff', white: '#ffffff'}\n")
notes = ("# Notes\n\n```sh\necho md-run-ok > ran.txt\n```\n\n```mermaid\ngraph TD\n  A --> B\n```\n\n"
         "```console\n$ echo AKIAABCDEFGHIJKLMNOP\nAKIAABCDEFGHIJKLMNOP\n```\n")
d = t3 / "md"
d = t3_run("md", 'theme = "grad"\nwindow_opacity = 1.0\nmask_secrets = true\n' + t3_keys, [
    f"shot {d}/b0.png", f"dump {d}/b0.txt",
    "type cat notes.md\\n", "wait 600", "key cmd+up", "key ctrl+m", "wait 300", f"dump {d}/b1.txt",
    "a11y click Open in viewer", "wait 800", f"dump {d}/b2.txt",
    "a11y click Run console block…", "wait 300", f"dump {d}/b3.txt",
    # the dialog opens on Cancel: Enter alone runs nothing
    "key enter", "wait 300", f"dump {d}/b4.txt",
    "a11y click Run sh block…", "wait 300", "key right", "key enter",
    f"wait_file {d}/work/ran.txt 8000", f"dump {d}/b5.txt",
    "key ctrl+alt+r", "wait 200", "a11y click Run sh block…", "wait 300", f"dump {d}/b6.txt"],
    files={"cfg/rift/themes/grad.yaml": grad_yaml, "work/notes.md": notes})
assert "bg_gradient=true accent_gradient=true" in t3_line(d, "b0"), t3_line(d, "b0")
png = (d / "b0.png").read_bytes()
def png_rows(data):
    import zlib, struct
    i, idat, w = 8, b"", 0
    while i < len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        if kind == b"IHDR":
            w = struct.unpack(">I", data[i + 8:i + 12])[0]
        elif kind == b"IDAT":
            idat += data[i + 8:i + 8 + n]
        i += 12 + n
    raw, stride, rows, prev, p = zlib.decompress(idat), w * 4, [], bytearray(w * 4), 0
    while p < len(raw):
        f, line = raw[p], bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for x in range(stride):
            a = line[x - 4] if x >= 4 else 0
            b, c = prev[x], (prev[x - 4] if x >= 4 else 0)
            if f == 1: line[x] = (line[x] + a) & 255
            elif f == 2: line[x] = (line[x] + b) & 255
            elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return rows
rows = png_rows(png)
scale = len(rows) // 600
top = rows[70 * scale][4 * 750 * scale:4 * 750 * scale + 3]
bottom = rows[590 * scale][4 * 750 * scale:4 * 750 * scale + 3]
# the ground runs from blue-ish (#101040) at the top to brown (#402010) below
assert top[2] > top[0] and bottom[0] > bottom[2], (tuple(top), tuple(bottom))
assert "Open in viewer" in t3_text(d, "b1"), re.search(r"^menu_items=.*$", t3_text(d, "b1"), re.M)
b2 = t3_text(d, "b2")
assert re.search(r'viewer=Some\("[^"]*/md/work/notes\.md"\)', b2), b2[:800]
assert re.search(r'viewer_runs=\["1@[\d,]+", "3@[\d,]+"\]', t3_line(d, "b2")), t3_line(d, "b2")
assert 'Button "Run sh block…"' in b2 and 'Button "Run console block…"' in b2
# the dialog shows the command masked (mask_secrets) and runs nothing yet
b3 = re.search(r"^confirm=.*$", t3_text(d, "b3"), re.M).group(0)
assert "RunCode" in b3 and '["echo ••••••••••••••••••••"]' in b3, b3
assert "confirm=None" in t3_text(d, "b4") and 'cmd="echo AKIA' not in t3_text(d, "b4")
assert (d / "work/ran.txt").read_text() == "md-run-ok\n"
# a read-only pane refuses: no dialog
b6 = t3_text(d, "b6")
assert "confirm=None" in b6 and 'toast=Some("Pane is read-only")' in b6, re.search(r"^confirm=.*$", b6, re.M)
# (3) the explorer's right-click menu
d = t3 / "files"
d = t3_run("files", "", [
    "key shift+cmd+e", "wait 600", f"dump {d}/c0.txt",
    "rclick 100 113", "wait 300", f"dump {d}/c1.txt",
    "a11y click Copy relative path", "wait 300", f"dump {d}/c2.txt",
    "rclick 100 89", "wait 300", "a11y click New file…", "wait 300", f"dump {d}/c3.txt",
    "type new.txt", "key enter", "wait 500", f"dump {d}/c4.txt",
    "rclick 100 89", "wait 300", "a11y click New file…", "wait 300", "type a.txt", "key enter",
    "wait 300", f"dump {d}/c5.txt"],
    files={"work/notes.md": "# hi\n", "work/sub/a.txt": "keep\n"})
assert re.search(r'explorer_rows=\["sub@0,77,259,24", "notes\.md@0,101,259,24"\]', t3_line(d, "c0")), t3_line(d, "c0")
assert 'menu_items=["Open", "Copy path", "Copy relative path", "New file…", "Reveal in Finder"]' in t3_text(d, "c1") \
    or 'menu_items=["Open", "Copy path", "Copy relative path", "New file…", "Show in file manager"]' in t3_text(d, "c1"), \
    re.search(r"^menu_items=.*$", t3_text(d, "c1"), re.M)
assert 'clipboard="notes.md"' in t3_text(d, "c2")
# it asks for a name only (empty input); the row names the file it creates
assert re.search(r'^palette=true mode=SavePath query=""', t3_text(d, "c3"), re.M), \
    re.search(r"^palette=.*$", t3_text(d, "c3"), re.M)
assert (d / "work/sub/new.txt").read_text() == "" and 'selected=Some("new.txt")' in t3_text(d, "c4")
# an existing file is never replaced by "New file…"
assert (d / "work/sub/a.txt").read_text() == "keep\n" and "a.txt already exists" in t3_text(d, "c5")
# a path instead of a name, `..`, or a symlinked folder leading out of the
# explorer's root creates nothing
d = t3 / "escape"
(d / "work").mkdir(parents=True, exist_ok=True)
(d / "out").mkdir(exist_ok=True)
if not (d / "work/zlink").is_symlink():
    os.symlink("../out", d / "work/zlink")
new_file = lambda row, name: [f"rclick 100 {row}", "wait 300", "a11y click New file…", "wait 300",
                              f"type {name}", "key enter", "wait 300"]
d = t3_run("escape", "", [
    "key shift+cmd+e", "wait 600",
    *new_file(89, "../up.txt"), f"dump {d}/e1.txt",
    *new_file(89, str(d / "out/abs.txt")), f"dump {d}/e2.txt",
    *new_file(89, ".."), f"dump {d}/e3.txt",
    *new_file(113, "esc.txt"), f"dump {d}/e4.txt"])
assert 'toast=Some("New file: a name only, without /")' in t3_text(d, "e1"), t3_text(d, "e1")[:600]
assert 'toast=Some("New file: a name only, without /")' in t3_text(d, "e2")
assert 'toast=Some("New file: not a file name")' in t3_text(d, "e3")
assert "leads out of the explorer" in t3_text(d, "e4"), re.search(r"^toast=.*$", t3_text(d, "e4"), re.M)
assert not (d / "work/up.txt").exists() and not (d / "up.txt").exists() and list((d / "out").iterdir()) == []
# (4) a window closed mid-preview puts the theme back; a picker opened in
# another window reverts the first one's preview before it starts
d = t3 / "windows"
d = t3_run("windows", 'theme = "nord"\n' + t3_keys, [
    "new_window", "wait 800", "window 0",
    "key ctrl+alt+t", "wait 300", "type dracula", "wait 300",
    "window 1", "key ctrl+alt+t", "wait 300", "type monokai", "wait 300", f"dump {d}/w1.txt",
    "key escape", "wait 300", f"dump {d}/w2.txt",
    "window 0", "key ctrl+alt+t", "wait 300", "type dracula", "wait 300",
    "close_window", "wait 500", f"dump {d}/w3.txt"])
assert 'theme="monokai"' in t3_text(d, "w1") and 'theme="nord"' in t3_text(d, "w2"), re.search(r"^font_size.*$", t3_text(d, "w2"), re.M)
assert 'theme="nord"' in t3_text(d, "w3") and "theme_preview=false" in t3_line(d, "w3"), re.search(r"^font_size.*$", t3_text(d, "w3"), re.M)

# config-edit: config.toml beside Settings is an editor. Typing and undo,
# a drag that selects, copy and paste; a Settings change shows in the clean
# pane without reopening it; another file in the viewer stays read-only. A
# scripted run reads no /etc/shells: generated profiles come only from a
# RIFT_SHELLS_FILE fixture
import urllib.parse
ce = scratch / "config-edit"
CE_CFG = ('shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
          'check_for_updates = false\n')
def ce_run(d, steps, extra_env=None):
    """Settings › Open config.toml in a run of its own (config `CE_CFG`), then
    `steps` ("{d}" in them: the run's directory)."""
    for sub in ("cfg/rift", "data"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "cfg/rift/config.toml").write_text(CE_CFG)
    (d / "t.rift").write_text("\n".join([
        "resize 1300 800", "wait_prompt", "key cmd+,", "wait 500", "a11y click Open config.toml",
        "wait 800", *(s.replace("{d}", str(d)) for s in steps), "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1", **(extra_env or {})),
                            stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"config edit {d.name} run failed", d / "log.txt")
    return d
ce_text = lambda d, name: (d / f"{name}.txt").read_text()
ce_line = lambda d, name: re.search(r"^config_edit=.*$", ce_text(d, name), re.M)[0]
ce_buf = lambda d, name: json.loads(re.search(r"^config_buffer=(.*)$", ce_text(d, name), re.M)[1])
def ce_at(d, name):
    """(x, y) of the first line's text and a cell's width, as `dump` reports them."""
    x, y, cw = re.search(r"text_at=\(([\d.]+), ([\d.]+), ([\d.]+)\)", ce_line(d, name)).groups()
    return float(x), float(y), float(cw)
d = ce_run(ce / "edit", [
    "focus on", "dump {d}/0.txt", "type x", "wait 300", "dump {d}/1.txt",
    "key cmd+z", "wait 300", "dump {d}/2.txt"])
assert "dirty=false" in ce_line(d, "0") and ce_buf(d, "0") == CE_CFG, ce_line(d, "0")
assert ce_buf(d, "1") == "x" + CE_CFG and "dirty=true" in ce_line(d, "1"), ce_line(d, "1")
assert ce_buf(d, "2") == CE_CFG and "dirty=false" in ce_line(d, "2"), ce_line(d, "2")
# the run's file has no generated profile: /etc/shells was not read
assert 'source = "shells"' not in (d / "cfg/rift/config.toml").read_text()
# a drag across "shell" on line 1 selects it; ⌘C then ⌘V at the end pastes it
x0, y0, cw = ce_at(d, "0")
d2 = ce_run(ce / "drag", [
    "focus on", f"drag {x0 + 1:.0f} {y0 + 10:.0f} {x0 + cw * 5 + 1:.0f} {y0 + 10:.0f}", "wait 200",
    "dump {d}/sel.txt", "key cmd+c", "key cmd+down", "key cmd+v", "wait 300", "dump {d}/pasted.txt",
    "shot {d}/pasted.png"])
assert "selection=Some(0..5)" in ce_line(d2, "sel"), ce_line(d2, "sel")
assert ce_buf(d2, "pasted") == CE_CFG + "shell", ce_buf(d2, "pasted")
# a Settings change while clean: the pane shows the new line, not reopened
d3 = ce_run(ce / "follow", [
    "a11y click Warn before closing running commands", "wait 1000", "dump {d}/after.txt"])
assert "confirm_close_running = false" in ce_buf(d3, "after"), ce_buf(d3, "after")
line = ce_line(d3, "after")
assert "dirty=false" in line and "conflict=false" in line, line
assert ce_buf(d3, "after") == (d3 / "cfg/rift/config.toml").read_text()
# another file in the viewer is read-only: typing changes nothing
(ce / "other.txt").write_text("plain\n")
d4 = ce_run(ce / "other", [
    "key escape", "wait 300", "key cmd+1", "wait 300",
    "link rift://action/open_file?path=" + urllib.parse.quote(str(ce / "other.txt")), "wait 800",
    "type zz", "wait 200", "dump {d}/other.txt"])
other = ce_text(d4, "other")
assert re.search(r'^viewer=Some\("[^"]*/config-edit/other\.txt"\)', other, re.M), other[:2000]
assert "config_edit=false" in other and (ce / "other.txt").read_text() == "plain\n", other[:2000]
# RIFT_SHELLS_FILE: its shells become generated [[profiles]] at startup
(ce / "shells").write_text("/bin/sh\n")
d5 = ce_run(ce / "shells-file", ["dump {d}/s.txt"], {"RIFT_SHELLS_FILE": str(ce / "shells")})
assert 'source = "shells"' in (d5 / "cfg/rift/config.toml").read_text(), \
    (d5 / "cfg/rift/config.toml").read_text()

# config-edit-lint: a lint error (with its did-you-mean) shuts Save and ⌘S
# writes nothing; fixed, ⌘S writes the text byte for byte and it applies;
# a removed key only warns; a syntax error points at its line; the file
# changing under unsaved edits asks Reload / Keep mine; closing or quitting
# with unsaved edits asks Save / Discard / Cancel
cl = scratch / "config-edit-lint"
d = ce_run(cl / "lint", [
    "focus on", "key cmd+down", "type font_sise = 14", "wait 400", "dump {d}/err.txt",
    "shot {d}/err.png", "key cmd+s", "wait 600", "dump {d}/err-save.txt", "sh cp {d}/cfg/rift/config.toml {d}/err-save.toml",
    "key shift+cmd+left", "type font_size = 14", "wait 400", "dump {d}/fixed.txt",
    "key cmd+s", "wait 1500", "dump {d}/saved.txt", "shot {d}/saved.png",
    "key enter", "type background_processes = true", "wait 400", "dump {d}/warn.txt",
    "shot {d}/warn.png",
    "key cmd+a", "type a = 1\\nb = 2\\nc = = 3\\n", "wait 400", "dump {d}/syntax.txt",
    "a11y click Dismiss", "wait 300", "dump {d}/dismissed.txt"])
err = ce_line(d, "err")
assert "save_enabled=false" in err and "5:Error:unknown key `font_sise`" in err, err
assert "did you mean `font_size`" in err, err
assert "save_enabled=false" in ce_line(d, "err-save"), ce_line(d, "err-save")
assert (d / "err-save.toml").read_text() == CE_CFG, "⌘S wrote a file with an error"
assert "save_enabled=true" in ce_line(d, "fixed") and "diagnostics=[]" in ce_line(d, "fixed")
saved = ce_text(d, "saved")
assert (d / "cfg/rift/config.toml").read_text() == CE_CFG + "font_size = 14", "not byte for byte"
assert re.search(r"^font_size=14 ", saved, re.M) and "dirty=false" in ce_line(d, "saved"), \
    re.search(r"^font_size.*$", saved, re.M)
warn = ce_line(d, "warn")
assert "6:Warning:" in warn and "background_processes" in warn and "save_enabled=true" in warn, warn
assert "3:Error:" in ce_line(d, "syntax"), ce_line(d, "syntax")
assert re.search(r"diagnostics=\[\"3:Error:", ce_line(d, "syntax")), ce_line(d, "syntax")
assert ce_buf(d, "dismissed") == CE_CFG + "font_size = 14" and "dirty=false" in ce_line(d, "dismissed")
# the file changes under unsaved edits: a conflict, the edits kept; Keep
# mine then ⌘S overwrites; Reload takes the file's text
d = ce_run(cl / "conflict", [
    "focus on", "type # mine\\n", "a11y click Warn before closing running commands", "wait 1000",
    "dump {d}/c1.txt", "shot {d}/conflict.png",
    "a11y click Keep mine", "wait 200", "dump {d}/c2.txt", "key cmd+s", "wait 1500",
    "dump {d}/c3.txt", "sh cp {d}/cfg/rift/config.toml {d}/c3.toml",
    "focus on", "type # again\\n", "a11y click Warn before closing running commands", "wait 1000",
    "a11y click Reload", "wait 300", "dump {d}/c4.txt"])
c1 = ce_line(d, "c1")
assert "conflict=true" in c1 and "dirty=true" in c1 and ce_buf(d, "c1") == "# mine\n" + CE_CFG, c1
assert "conflict=false" in ce_line(d, "c2") and "dirty=true" in ce_line(d, "c2")
assert (d / "c3.toml").read_text() == "# mine\n" + CE_CFG, "Keep mine + Save did not overwrite"
c4 = ce_line(d, "c4")
assert "conflict=false" in c4 and "dirty=false" in c4, c4
assert ce_buf(d, "c4") == (d / "cfg/rift/config.toml").read_text() and "# again" not in ce_buf(d, "c4")
# closing with unsaved edits asks; Save is shut while they have errors;
# Cancel keeps them, ⌘Q asks too, Discard closes the pane, the file as it was
d = ce_run(cl / "close", [
    "focus on", "type font_sise = 1\\n", "wait 400", "key escape", "wait 300", "dump {d}/q1.txt",
    "shot {d}/dialog.png", "a11y click Save, unavailable", "wait 200", "dump {d}/q2.txt",
    "a11y click Cancel", "wait 200", "dump {d}/q3.txt",
    "key cmd+q", "wait 300", "dump {d}/q4.txt", "a11y click Cancel", "wait 200",
    "key escape", "wait 300", "a11y click Discard", "wait 300", "dump {d}/q5.txt"])
q1 = ce_text(d, "q1")
assert "confirm=Some((ConfigUnsaved(Pane), [])" in q1, re.search(r"^confirm=.*$", q1, re.M)
assert 'Button "Save, unavailable"' in q1 and "Fix 1 error to save" in q1, q1[-3000:]
assert "confirm=Some((ConfigUnsaved(Pane)" in ce_text(d, "q2"), "a shut Save answered"
q3 = ce_text(d, "q3")
assert "confirm=None" in q3 and ce_buf(d, "q3") == "font_sise = 1\n" + CE_CFG, ce_line(d, "q3")
assert "confirm=Some((ConfigUnsaved(Close(App)), [])" in ce_text(d, "q4"), \
    re.search(r"^confirm=.*$", ce_text(d, "q4"), re.M)
q5 = ce_text(d, "q5")
assert "config_edit=false" in q5 and re.search(r"^viewer=None ", q5, re.M), q5[:2000]
assert (d / "cfg/rift/config.toml").read_text() == CE_CFG

# settings-profiles: Settings › Profiles edits config.toml's [[profiles]] in
# place (comments kept). Add, a name and arguments, environment and colour,
# Duplicate (a detected one's copy is the user's own), Hide, Set as default,
# Delete (a detected default one: the default goes, the shell never comes
# back), the Default profile select, ⌥↑ and a drag reorder; a duplicate name
# or an empty command is refused under the form and nothing is written.
# Relaunched: the order, the default and the hidden profile hold, the deleted
# shell stays gone, the hidden one is out of the + menu
sp = scratch / "settings-profiles"
for sub in ("cfg/rift", "data", "bin"):
    (sp / sub).mkdir(parents=True, exist_ok=True)
sp_bin = {}
for name in ("zsh", "fish", "nu"):
    sp_bin[name] = sp / "bin" / name
    sp_bin[name].write_text('#!/bin/sh\nexec /bin/sh "$@"\n')
    sp_bin[name].chmod(0o755)
(sp / "shells").write_text("".join(f"{p}\n" for p in sp_bin.values()))
SP_CFG = ('# my settings\nshell = "/bin/bash"\nrestore_session = false\n'
          'durable_sessions = false\nsync_os_theme = false\ncheck_for_updates = false\n\n'
          '# my profile\n[[profiles]]\nname = "mine"\ncommand = "/bin/bash"\nargs = ["-l"]\n')
(sp / "cfg/rift/config.toml").write_text(SP_CFG)
sp_env = dict(env, XDG_CONFIG_HOME=str(sp / "cfg"), XDG_DATA_HOME=str(sp / "data"),
              RIFT_SHELLS_FILE=str(sp / "shells"), SHELL=str(sp_bin["zsh"]))
def sp_run(name, steps):
    (sp / f"{name}.rift").write_text("\n".join(
        ["resize 1300 1500", "wait_prompt", "key cmd+,", "wait 500", "a11y click Profiles",
         "wait 300", *steps, "quit"]) + "\n")
    with (sp / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sp / f"{name}.rift"), "--headless"],
                            env=sp_env, stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"settings-profiles {name} run failed", sp / f"{name}.log")
def sp_save(step):
    """Steps that keep config.toml as it is after `step`."""
    return ["wait 300", cp(sp / "cfg/rift/config.toml", sp / f"{step}.toml"), f"dump {sp}/{step}.txt"]
sp_toml = lambda step: (sp / f"{step}.toml").read_text()
sp_dump = lambda step: (sp / f"{step}.txt").read_text()
def sp_line(step):
    return re.search(r"^settings_profiles=.*$", sp_dump(step), re.M)[0]
def sp_profiles(step):
    """config.toml after `step`, from its first [[profiles]] (the user's comment above it)."""
    text = sp_toml(step)
    return text[text.index("# my profile"):]
stub = lambda name: f'[[profiles]]\nname = "{name}"\ncommand = "{sp_bin[name]}"\nsource = "shells"\n'
MINE = '# my profile\n[[profiles]]\nname = "mine"\ncommand = "/bin/bash"\nargs = ["-l"]\n'
zsh_path = str(sp_bin["zsh"])
sp_run("one", [
    *sp_save("s0"),
    "a11y click Add a profile", *sp_save("add"),
    "a11y click Name: New profile", "key cmd+backspace", "type work", "key enter", *sp_save("name"),
    # a name already taken: refused under the form, nothing written
    "a11y click Name: work", "key cmd+backspace", "type mine", "key enter", *sp_save("taken"),
    "shot {sp}/taken.png".format(sp=sp), "key escape",
    # an empty command: refused too
    f"a11y click Command: {zsh_path}", "key cmd+backspace", "key enter", *sp_save("empty"),
    "key escape",
    "a11y click Arguments: None", "type -c 'echo hi'", "key enter", *sp_save("args"),
    "a11y click Environment: None", "type A=1 B='x y'", "key enter", *sp_save("env"),
    "a11y click Tab color: None", "wait 200", "a11y click Green", *sp_save("color"),
    "a11y click Edit zsh", "wait 200", "a11y click Duplicate zsh", *sp_save("dup"),
    "a11y click Hide zsh copy from the + menu and the palette", *sp_save("hide"),
    "a11y click Edit fish", "wait 200", "a11y click Make fish the default profile",
    *sp_save("default"),
    "a11y click Delete fish", "wait 300", f"dump {sp}/confirm.txt", f"shot {sp}/confirm.png",
    "key enter", *sp_save("delete"),
    # (the relaunches' first tab runs it: a real bash, which reports prompts)
    "a11y click Default profile: Shell path", "wait 200", "a11y click mine", *sp_save("select"),
    # ⌥↑ moves the focused profile (its Edit button) up one place
    "a11y click Edit work", "wait 200", "key alt+up", *sp_save("up"),
])
s0 = sp_profiles("s0")
assert sp_toml("s0").startswith("# my settings\n"), sp_toml("s0")
assert s0 == MINE + "\n" + stub("zsh") + "\n" + stub("fish") + "\n" + stub("nu"), s0
NEW = f'\n[[profiles]]\nname = "New profile"\ncommand = "{zsh_path}"\n'
assert sp_profiles("add") == s0 + NEW, sp_profiles("add")
assert 'profile_open=Some(4)' in sp_line("add"), sp_line("add")
work = NEW.replace('"New profile"', '"work"')
assert sp_profiles("name") == s0 + work, sp_profiles("name")
assert sp_toml("taken") == sp_toml("name"), "a taken name was written"
assert 'profile_error=Some("profile \\"mine\\": the name is used twice")' in sp_line("taken"), sp_line("taken")
assert 'Label "Not saved: profile \\"mine\\": the name is used twice"' in sp_dump("taken"), sp_dump("taken")[-3000:]
assert sp_toml("empty") == sp_toml("name"), "an empty command was written"
assert 'profile_error=Some("profile \\"work\\": command is empty")' in sp_line("empty"), sp_line("empty")
work += 'args = [\n    "-c",\n    "echo hi",\n]\n'
assert sp_profiles("args") == s0 + work, sp_profiles("args")
work_env = work + '\n[profiles.env]\nA = "1"\nB = "x y"\n'
assert sp_profiles("env").replace('color = "green"\n', "") == s0 + work_env, sp_profiles("env")
assert 'color = "green"' in sp_profiles("color"), sp_profiles("color")
dup = sp_profiles("dup")
assert f'[[profiles]]\nname = "zsh copy"\ncommand = "{zsh_path}"\n\n[[profiles]]\nname = "fish"' in dup, dup
assert dup.count('source = "shells"') == 3, "the copy kept source"
assert '"zsh", "zsh copy", "fish"' in sp_line("dup") and "profile_open=Some(2)" in sp_line("dup")
assert 'name = "zsh copy"\ncommand = "' + zsh_path + '"\nhidden = true\n' in sp_profiles("hide"), sp_profiles("hide")
assert sp_toml("default").count('default_profile = "fish"') == 1, sp_toml("default")
confirm = sp_dump("confirm")
assert "DeleteProfile { index: 3, name: \"fish\", detected: true, default: true }" in confirm, \
    re.search(r"^confirm=.*$", confirm, re.M)
assert "won't add it again" in confirm and "Hide keeps it in the file instead" in confirm, confirm[-2000:]
deleted = sp_toml("delete")
assert 'name = "fish"' not in deleted and "default_profile" not in deleted, deleted
assert str(sp_bin["fish"]) in (sp / "data/rift/generated-profiles").read_text()
assert sp_toml("select").count('default_profile = "mine"') == 1, sp_toml("select")
up = sp_line("up")
assert 'settings_profiles=["mine", "zsh", "zsh copy", "work", "nu"]' in up, up
assert sp_profiles("up").index('name = "work"') < sp_profiles("up").index('name = "nu"')
assert sp_toml("up").startswith("# my settings\n") and "# my profile\n" in sp_toml("up")
# relaunched: the probe finds the rows; a drag moves "mine" past the last
sp_run("probe", [*sp_save("relaunch")])
assert sp_toml("relaunch") == sp_toml("up"), "the relaunch's sync changed the file (fish came back?)"
rows = dict((int(i), (x, y, w, h)) for i, x, y, w, h in re.findall(
    r"\((\d+), ([\d.]+), ([\d.]+), ([\d.]+), ([\d.]+)\)", re.search(r"profile_rows=(.*)$", sp_line("relaunch"))[1]))
assert sorted(rows) == [0, 1, 2, 3, 4], rows
mx = lambda r: float(r[0]) + 200
my = lambda r: float(r[1]) + float(r[3]) / 2
last = rows[4]
sp_run("drag", [f"drag {mx(rows[0]):.0f} {my(rows[0]):.0f} {mx(last):.0f} {float(last[1]) + float(last[3]) - 4:.0f}",
                *sp_save("dragged")])
assert 'settings_profiles=["zsh", "zsh copy", "work", "nu", "mine"]' in sp_line("dragged"), sp_line("dragged")
assert "# my profile\n" in sp_toml("dragged") and sp_toml("dragged").count("[[profiles]]") == 5
# the + menu: the hidden copy is not offered; the default stays
sp_run("menu", ["key escape", "wait 300", "a11y click Open a profile", "wait 300", f"dump {sp}/menu.txt"])
menu = sp_dump("menu")
assert 'menu_items=["zsh", "work", "nu", "mine", "Settings", "Command Palette", "About Rift"]' in menu, \
    re.search(r"menu_items=.*$", menu, re.M)
assert 'default_profile=Some("mine")' in sp_line("menu"), sp_line("menu")
assert sp_toml("dragged").count('default_profile = "mine"') == 1

# settings-parity: every row added so each config.toml key has one writes
# its key: a toggle, a stepper step, a segment, a text field (⏎ saves)
import tomllib
sr = scratch / "settings-parity"
for sub in ("cfg/rift", "data"):
    (sr / sub).mkdir(parents=True, exist_ok=True)
SR_CFG = ('shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
          'sync_os_theme = false\ncheck_for_updates = false\n')
mac = sys.platform == "darwin"
def sr_run(name, page, steps, cfg=SR_CFG):
    """`steps` on Settings › `page` (a window tall enough for every row);
    the config.toml it leaves, parsed."""
    (sr / "cfg/rift/config.toml").write_text(cfg)
    (sr / f"{name}.rift").write_text("\n".join(
        ["resize 1300 3000", "wait_prompt", "key cmd+,", "wait 500", f"a11y click {page}", "wait 300",
         *steps, "wait 300", f"dump {sr}/{name}.txt", "quit"]) + "\n")
    with (sr / f"{name}.log").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(sr / f"{name}.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(sr / "cfg"), XDG_DATA_HOME=str(sr / "data")),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"settings-parity {name} run failed", sr / f"{name}.log")
    return tomllib.loads((sr / "cfg/rift/config.toml").read_text())
def text_step(label, value):
    return [f"a11y click {label}", "key cmd+backspace", f"type {value}", "key enter", "wait 200"]
got = sr_run("general", "General", [
    *text_step("Shell path: /bin/bash", "/bin/sh"),
    "a11y click Keep shells running through updates",
    *text_step("Global hotkey: Off", "alt+space"),
    "a11y click Screen edge: Left",
    "a11y click Decrease Hotkey window width, 100 %",
    *text_step("Screen: pointer", "2"),
])
want = {"shell": "/bin/sh", "durable_sessions": True, "global_hotkey": "alt+space",
        "hotkey_window_edge": "left", "hotkey_window_width": 95, "hotkey_window_screen": "2"}
assert {k: got.get(k) for k in want} == want, {k: got.get(k) for k in want}
got = sr_run("appearance", "Appearance", [
    "a11y click Increase Font weight, 400", "a11y click Decrease Bold font weight, 700",
    "a11y click Bold text in bright colors", "a11y click Increase Minimum contrast, Off",
    *text_step("Font features: None", "ss01 -calt"),
    "a11y click Ambiguous-width characters: Wide",
    "a11y click Increase Top padding, 14 px", "a11y click Decrease Bottom padding, 8 px",
    "a11y click Pad full-screen programs",
    "a11y click Open new windows: Maximized",
    "a11y click Increase Window width, 1180 px", "a11y click Decrease Window height, 760 px",
    "a11y click Keep windows on top", "a11y click Resize in whole cells",
    "a11y click Show the size while resizing", "a11y click ⌃Tab switches: Last used first",
])
want = {"font_weight": 500, "bold_font_weight": 600, "bold_is_bright": True, "minimum_contrast": 1.5,
        "font_features": ["ss01", "-calt"], "ambiguous_width": "wide", "window_padding_top": 16,
        "window_padding_bottom": 6, "alt_screen_padding": False, "launch_mode": "maximized",
        "window_width": 1200, "window_height": 740, "always_on_top": True, "snap_to_cells": True,
        "resize_overlay": True, "tab_switch_order": "mru"}
assert {k: got.get(k) for k in want} == want, {k: got.get(k) for k in want}
got = sr_run("input", "Input", [
    "a11y click Vim's unnamed register: System clipboard", "a11y click ⌃R, ⌃T and ⌥C run: Rift's",
    "a11y click Tab key: Ghost text", "a11y click Open completions as you type",
    "a11y click Show the accept-key hint", "a11y click Show the ignore button",
    *text_step("More subshell commands: None", "'^nix shell'"),
    *text_step("Never offer it for: None", "bash"),
])
want = {"vim_clipboard": "unnamed", "shell_widgets": "rift", "subshell_commands": ["^nix shell"],
        "subshell_denylist": ["bash"], "tab_behavior": "autosuggestion",
        "completions_open_while_typing": True, "autosuggestion_hint": False,
        "autosuggestion_ignore_button": True}
assert {k: got.get(k) for k in want} == want, {k: got.get(k) for k in want}
got = sr_run("terminal", "Terminal", [
    f"a11y click {'⌘' if mac else 'Ctrl'}-scroll zooms the font",
    *text_step("Word characters: None", "-./"), *text_step("Word delimiters: None", "/:"),
    "a11y click Increase Ask before pasting more than, Off", "a11y click Trim trailing blanks from pastes",
    "a11y click Scrollbar: Always", "a11y click When its program exits: Keep",
    "a11y click Mask secrets", "a11y click Show programs' progress",
])
want = {"wheel_zoom": True, "word_chars": "-./", "word_delimiters": "/:", "large_paste_warning": 16,
        "trim_paste": True, "scrollbar": "always", "close_on_exit": "never", "mask_secrets": True,
        "show_progress": False}
assert {k: got.get(k) for k in want} == want, {k: got.get(k) for k in want}
got = sr_run("notifications", "Notifications", [
    f"a11y click {'Bounce the Dock icon' if mac else 'Mark the window urgent'}"])
assert got.get("request_attention") is True, got
# a value that is not valid is refused with a toast and never written
got = sr_run("invalid", "General", [*text_step("Screen: pointer", "left"), "wait 200"])
assert "hotkey_window_screen" not in got, got
assert 'toast=Some("Screen not saved: \\"left\\" is not pointer, primary or a screen number")' in \
    (sr / "invalid.txt").read_text(), re.search(r"^confirm=.*$", (sr / "invalid.txt").read_text(), re.M)
got = sr_run("remote", "Remote access", [
    "a11y click Connection settings, collapsed", "wait 200",
    *text_step("Listen address: 127.0.0.1:0", "127.0.0.1:7999"),
    *(["a11y click Ask before keeping Rift running"] if mac else []),
], SR_CFG + 'remote_access = true\nremote_listen = "127.0.0.1:0"\n')
assert got.get("remote_listen") == "127.0.0.1:7999", got
if mac:
    assert got.get("confirm_background_quit") is False, got

# config-edit-windows: ⌘Q in another window asks "Save changes?" in the
# window whose config.toml pane holds unsaved edits, and Cancel keeps every
# window; edits in the hidden drop-down window slide it in to ask
cw = scratch / "config-edit-windows"
def cw_run(name, cfg, steps):
    d = cw / name
    for sub in ("cfg/rift", "data"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "cfg/rift/config.toml").write_text(
        cfg + 'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
        'check_for_updates = false\n')
    (d / "t.rift").write_text("\n".join([
        "resize 1300 800", "wait_prompt", *steps, "key cmd+,", "wait 500", "a11y click Open config.toml",
        "wait 800", "focus on", "type x", "wait 300", *(s.replace("{d}", str(d)) for s in steps2[name]),
        "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data")),
                            stdout=log, stderr=log, timeout=90).returncode
    assert rc == 0, died(f"config-edit-windows {name} run failed", d / "log.txt")
    return lambda step: (d / f"{step}.txt").read_text()
steps2 = {
    # the pane in window 0; ⌘Q from window 1
    "window": ["new_window", "wait 800", "key cmd+q", "wait 500", "window 0", "wait 300", "dump {d}/q.txt",
               "a11y click Cancel", "wait 300", "dump {d}/c.txt"],
    # the pane in the drop-down window, hidden again; ⌘Q from window 0
    "dropdown": ["hotkey", "wait 800", "window 0", "wait 300", "key cmd+q", "wait 500", "dump {d}/q.txt",
                 "a11y click Cancel", "wait 300", "dump {d}/c.txt"],
}
t = cw_run("window", "", [])
assert "confirm=Some((ConfigUnsaved(Close(App)), [])" in t("q"), re.search(r"^confirm=.*$", t("q"), re.M)
assert re.search(r"^windows=2 ", t("c"), re.M) and "confirm=None" in t("c") and "dirty=true" in t("c")
t = cw_run("dropdown", "hotkey_window = true\nhotkey_window_height = 100\n", ["hotkey", "wait 800", "wait_prompt"])
q = t("q")
assert re.search(r"^windows=2 window=1 focused_window=Some\(1\)", q, re.M), q[:400]
assert re.search(r"^window 1: .*kind=dropdown shown=true", q, re.M), q[:400]
assert "confirm=Some((ConfigUnsaved(Close(App)), [])" in q, re.search(r"^confirm=.*$", q, re.M)
assert re.search(r"^windows=2 ", t("c"), re.M) and "confirm=None" in t("c") and "dirty=true" in t("c")


# config-edge: a pane Save then a Settings click at once keeps the saved
# text (a symlinked config.toml, seen by the watcher a second late); a file
# that does not load at startup is never rewritten, Add is refused, and a
# fix saved in the pane applies and brings the installed shells in; a file
# that is not UTF-8 opens read-only saying why, then editable once fixed;
# a profile form open in one window follows its profile when another
# window deletes the one above it
cx = scratch / "config-edge"
CX_BASE = 'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\ncheck_for_updates = false\n'
def cx_run(name, steps, cfg=None, extra_env=None, link=False):
    d = cx / name
    for sub in ("cfg/rift", "data", "dot"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    target = d / "dot/config.toml" if link else d / "cfg/rift/config.toml"
    target.write_bytes(cfg if isinstance(cfg, bytes) else (cfg or CX_BASE).encode())
    if link:
        (d / "cfg/rift/config.toml").symlink_to(target)
    (d / "t.rift").write_text("\n".join(
        ["resize 1300 1200", "wait_prompt", *(s.replace("{d}", str(d)) for s in steps), "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data"),
                                     **(extra_env or {})),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"config-edge {name} run failed", d / "log.txt")
    return d
cx_dump = lambda d, name: (d / f"{name}.txt").read_text()
cx_edit = lambda d, name: re.search(r"^config_edit=.*$", cx_dump(d, name), re.M)[0]
OPEN = ["key cmd+,", "wait 500", "a11y click Open config.toml", "wait 800", "focus on"]
d = cx_run("race", [*OPEN, "type font_size = 20\\n", "wait 300", "key cmd+s",
                    "a11y click Warn before closing running commands", "wait 2500", "dump {d}/a.txt"],
           link=True)
saved = (d / "dot/config.toml").read_text()
assert saved.startswith("font_size = 20\n") and "confirm_close_running = false" in saved, saved
assert (d / "cfg/rift/config.toml").is_symlink(), "the link was replaced"
assert re.search(r"^font_size=20 ", cx_dump(d, "a"), re.M), re.search(r"^font_size.*$", cx_dump(d, "a"), re.M)
(cx / "shells").write_text("/bin/sh\n")
BROKEN = "# broken\n" + CX_BASE + "font_size = [\n"
d = cx_run("broken", [
    "key cmd+,", "wait 500", "a11y click Profiles", "wait 300", "a11y click Add a profile", "wait 300",
    "dump {d}/a.txt", "sh cp {d}/cfg/rift/config.toml {d}/a.toml",
    "a11y click Open config.toml", "wait 800", "focus on", "key cmd+down", "key shift+cmd+up",
    "type " + ("# fixed\n" + CX_BASE + "font_size = 15\n").replace("\n", "\\n"), "wait 400",
    "key cmd+s", "wait 1000", "dump {d}/b.txt"], BROKEN, {"RIFT_SHELLS_FILE": str(cx / "shells")})
assert (d / "a.toml").read_text() == BROKEN, "a file that does not load was rewritten"
assert "settings_profiles=[] " in cx_dump(d, "a") and "fix config.toml first" in cx_dump(d, "a")
fixed = (d / "cfg/rift/config.toml").read_text()
assert fixed.startswith("# fixed\n") and 'command = "/bin/sh"\nsource = "shells"' in fixed, fixed
assert "config_error=None" in cx_dump(d, "b") and "dirty=false" in cx_edit(d, "b"), cx_edit(d, "b")
(cx / "latin1").mkdir(parents=True, exist_ok=True)
(cx / "latin1/good.toml").write_text("# cafe\n" + CX_BASE)
d = cx_run("latin1", [*OPEN, "type zz", "wait 300", "dump {d}/a.txt", "shot {d}/a.png",
                      "sh cp {d}/good.toml {d}/cfg/rift/config.toml", "wait 2500", "dump {d}/b.txt"],
           b"# caf\xe9\n" + CX_BASE.encode())
assert 'read_only=Some("Read-only: it is not UTF-8 text' in cx_edit(d, "a"), cx_edit(d, "a")
assert "config_edit=true dirty=false" in cx_edit(d, "b"), cx_edit(d, "b")
PROFS = CX_BASE + "discover_shells = false\n" + "".join(
    f'\n[[profiles]]\nname = "{n}"\ncommand = "/bin/sh"\n' for n in "abc")
d = cx_run("windows", [
    "key cmd+,", "wait 500", "a11y click Profiles", "wait 300", "a11y click Edit b", "wait 300",
    "new_window", "wait 800", "key cmd+,", "wait 500", "a11y click Profiles", "wait 300",
    "a11y click Edit a", "wait 300", "a11y click Delete a", "wait 300", "key enter", "wait 500",
    "window 0", "wait 500", "a11y click Name: b", "key cmd+backspace", "type bee", "key enter",
    "wait 500", "dump {d}/a.txt"], PROFS)
assert 'settings_profiles=["bee", "c"] default_profile=None profile_open=Some(0)' in cx_dump(d, "a"), \
    re.search(r"^settings_profiles=.*$", cx_dump(d, "a"), re.M)


# modal-pointer: a modal dialog covers the window, so nothing under it
# reacts to the pointer: the Prompt chips preview beside Edit prompt (and
# the Toolbar buttons preview beside Edit toolbar) draws no accent hover
# border and shows no tooltip (it does both without the dialog), and a link
# beside What's New gives no tooltip
mp = scratch / "modal-pointer"
MP_BASE = 'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\ncheck_for_updates = false\n'
def mp_run(name, steps):
    d = mp / name
    for sub in ("cfg/rift", "data"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "cfg/rift/config.toml").write_text(MP_BASE)
    (d / "t.rift").write_text("\n".join(
        ["resize 1300 900", "wait_prompt", *(s.replace("{d}", str(d)) for s in steps), "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data")),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"modal-pointer {name} run failed", d / "log.txt")
    return (d / "a.txt").read_text()
def mp_pixel(path, x, y, logical_w=1300):
    # the RGB at logical (x, y) of a shot, read from the PNG's own rows
    import zlib, struct
    data, i, idat, w = path.read_bytes(), 8, b"", 0
    while i < len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        if kind == b"IHDR":
            w = struct.unpack(">I", data[i + 8:i + 12])[0]
        elif kind == b"IDAT":
            idat += data[i + 8:i + 8 + n]
        i += 12 + n
    raw, stride, prev, p, scale = zlib.decompress(idat), w * 4, bytearray(w * 4), 0, w // logical_w
    for row in range(y * scale + 1):
        f, line = raw[p], bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for k in range(stride):
            a = line[k - 4] if k >= 4 else 0
            b, c = prev[k], (prev[k - 4] if k >= 4 else 0)
            if f == 1: line[k] = (line[k] + a) & 255
            elif f == 2: line[k] = (line[k] + b) & 255
            elif f == 3: line[k] = (line[k] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[k] = (line[k] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        prev = line
    return tuple(prev[4 * x * scale:4 * x * scale + 3])
MP_SETTINGS = ["key cmd+,", "wait 500", "type chips", "wait 300"]
found_rect = re.search(r"Edit prompt@(\d+),(\d+),(\d+),(\d+)", mp_run("find", [*MP_SETTINGS, "dump {d}/a.txt"]))
px, py, pw, ph = map(int, found_rect.groups())
# a spot on the preview right of the 600 px Edit prompt card
sx, sy = px + pw - 30, py + ph // 2
assert sx > 1300 // 2 + 300, (px, pw)
bare = mp_run("bare", [*MP_SETTINGS, f"move {sx} {sy}", "wait 900", "dump {d}/a.txt", "shot {d}/a.png"])
under = mp_run("under", [*MP_SETTINGS, f"click {px + pw // 2} {sy}", "wait 400", f"move {sx} {sy}",
                         "wait 900", "dump {d}/a.txt", "shot {d}/a.png"])
assert 'tooltip=Some("Edit prompt")' in bare, re.search(r"tooltip=\S*", bare)
assert "prompt_editor=Some(" in under and "tooltip=None" in under, re.search(r"tooltip=\S*", under)
lit, dim = mp_pixel(mp / "bare/a.png", sx, py), mp_pixel(mp / "under/a.png", sx, py)
# the hovered border is the accent (blue well above red); under the dialog
# it stays the plain hairline, dimmed by the backdrop
assert lit[2] > lit[0] + 80, lit
assert abs(dim[2] - dim[0]) < 20, dim
# the same beside Edit toolbar, over the Toolbar buttons preview
MP_TOOLBAR = ["key cmd+,", "wait 500", "type toolbar", "wait 300"]
tb_rect = re.search(r"Edit toolbar@(\d+),(\d+),(\d+),(\d+)", mp_run("tb-find", [*MP_TOOLBAR, "dump {d}/a.txt"]))
tx, ty, tw, th = map(int, tb_rect.groups())
tsx, tsy = tx + tw - 30, ty + th // 2
assert tsx > 1300 // 2 + 300, (tx, tw)
tb_bare = mp_run("tb-bare", [*MP_TOOLBAR, f"move {tsx} {tsy}", "wait 900", "dump {d}/a.txt", "shot {d}/a.png"])
tb_under = mp_run("tb-under", [*MP_TOOLBAR, f"click {tx + tw // 2} {tsy}", "wait 400", f"move {tsx} {tsy}",
                               "wait 900", "dump {d}/a.txt", "shot {d}/a.png"])
assert 'tooltip=Some("Edit toolbar")' in tb_bare, re.search(r"tooltip=\S*", tb_bare)
assert "toolbar_editor=Some(" in tb_under and "tooltip=None" in tb_under, re.search(r"tooltip=\S*", tb_under)
lit, dim = mp_pixel(mp / "tb-bare/a.png", tsx, ty), mp_pixel(mp / "tb-under/a.png", tsx, ty)
assert lit[2] > lit[0] + 80, lit
assert abs(dim[2] - dim[0]) < 20, dim
# a link printed in the pane, left of the What's New card: its tooltip
# shows without the dialog and not beside it
MP_LINK = "https://example.com/rift-modal"
link = mp_run("link", [f"type echo {MP_LINK}\\n", "wait 800", "dump {d}/a.txt"])
lx, ly, lw, lh = map(int, re.search(re.escape(MP_LINK) + r"@(\d+),(\d+),(\d+),(\d+)", link).groups())
assert lx + lw < 1300 // 2 - 300, (lx, lw)
spot = f"move {lx + lw // 2} {ly + lh // 2}"
shown = mp_run("link-bare", [f"type echo {MP_LINK}\\n", "wait 800", spot, "wait 900", "dump {d}/a.txt"])
assert f'tooltip=Some("{MP_LINK}' in shown, re.search(r"tooltip=\S*", shown)
news = mp_run("whatsnew", [f"type echo {MP_LINK}\\n", "wait 800", "key cmd+p", "wait 300",
                           "type What's New", "wait 300", "key enter", "wait 800", spot, "wait 900",
                           "dump {d}/a.txt"])
assert "whats_new=true" in news and "tooltip=None" in news, re.search(r"tooltip=\S*", news)


# settings-review: the Settings tab has no repository, so the code review
# button there is a dimmed ± with no counts that does nothing, and the
# panel (open on the shell tab) hides; back on the shell tab it shows again
# with its changes
sr = scratch / "settings-review"
(sr / "cfg/rift").mkdir(parents=True, exist_ok=True)
(sr / "data").mkdir(parents=True, exist_ok=True)
sr_repo = sr / "repo"
sr_repo.mkdir(parents=True, exist_ok=True)
for args in (["init", "-q"], ["config", "user.email", "t@t"], ["config", "user.name", "t"]):
    subprocess.run(["git", *args], cwd=sr_repo, check=True)
(sr_repo / "f.txt").write_text("a\n")
subprocess.run(["git", "add", "f.txt"], cwd=sr_repo, check=True)
subprocess.run(["git", "commit", "-qm", "init"], cwd=sr_repo, check=True)
(sr_repo / "f.txt").write_text("a\nb\nc\n")
(sr / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\ncheck_for_updates = false\n'
    + f"working_directory = {json.dumps(str(sr_repo))}\n")
(sr / "t.rift").write_text("\n".join([
    "resize 1300 900", "wait_prompt", "key shift+cmd+g", "wait 2000", f"dump {sr}/a.txt",
    "key cmd+,", "wait 600", "move 1277 17", "wait 900", f"dump {sr}/b.txt",
    "a11y click No repository on the Settings tab", "wait 300", f"dump {sr}/c.txt",
    "key cmd+,", "wait 800", f"dump {sr}/d.txt", "quit"]) + "\n")
with (sr / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(sr / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(sr / "cfg"), XDG_DATA_HOME=str(sr / "data")),
                        stdout=log, stderr=log, timeout=120).returncode
assert rc == 0, died("settings-review run failed", sr / "log.txt")
sr_dump = lambda name: (sr / f"{name}.txt").read_text()
sr_review = lambda name: re.search(r"^review=.*$", sr_dump(name), re.M)[0]
# the shell tab: counts widen the button, the panel is on screen
assert re.search(r'"Review changes@\d+,4,(\d+),26"', sr_dump("a"))[1] != "26", sr_dump("a")[:600]
assert sr_review("a").startswith("review=true load=ready") and " shown=true " in sr_review("a"), sr_review("a")
# the Settings tab: a 26 px ± that explains itself, the panel hidden
b = sr_dump("b")
assert re.search(r'"No repository on the Settings tab@\d+,4,26,26"', b) and "Review changes@" not in b, b[:600]
assert 'tooltip=Some("No repository on the Settings tab")' in b, re.search(r"tooltip=\S*", b)
assert re.search(r"^settings=true ", b, re.M) and " shown=false " in sr_review("b"), sr_review("b")
# a click does nothing: still Settings, the panel still open for later
assert re.search(r"^settings=true ", sr_dump("c"), re.M) and sr_review("c").startswith("review=true "), sr_review("c")
d = sr_dump("d")
assert re.search(r"^settings=false ", d, re.M) and '"Review changes@' in d, d[:600]
assert " shown=true " in sr_review("d") and 'unstaged=Some(["Mf.txt"])' in sr_review("d"), sr_review("d")


# tab-status: a tab shows its focused pane's folder (two on folders of the
# same name add the parent; a renamed tab keeps its name) and one status,
# the most urgent of its panes: failed and done from commands that ended
# out of view, working, and an agent in the other pane of a split tab
# that needs you (not the focused pane's: a ring). Focusing that pane
# makes it the tab's own. Holding ⌘ numbers the tabs ⌘1… in the ×'s place;
# releasing it takes the numbers away
ts = scratch / "tab-status"
TS_BASE = ('shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
           'check_for_updates = false\n')
def ts_run(name, style, steps):
    d = ts / name
    for sub in ("cfg/rift", "data", "bin", "w/a/src", "w/b/src", "w/run", "w/flow", "w/atlas"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "cfg/rift/config.toml").write_text(
        TS_BASE + f'tab_style = "{style}"\n[keybindings]\nrename_tab = "ctrl+alt+r"\n')
    # a coding agent (by its command's name) that asks for permission
    (d / "bin/claude").write_text("#!/bin/sh\nprintf '\\033]9;Claude needs your permission\\007'\nsleep 60\n")
    (d / "bin/claude").chmod(0o755)
    (d / "t.rift").write_text("\n".join(
        ["resize 1240 600", "focus on", "wait_prompt",
         *(s.replace("{d}", str(d)) for s in steps), "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data")),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"tab-status {name} run failed", d / "log.txt")
    return lambda dump: (d / f"{dump}.txt").read_text()
def ts_strip(text):
    # the strip's tabs as {title: (status, hint, status_pane)}
    line = re.search(r"^strip style=(\w+) h=(\d+) hints=(\w+) .* tabs=(\[.*\])$", text, re.M)
    tabs = {}
    for t in json.loads(line[4]):
        m = re.match(r'"(.*?)" status=(\S+) hint=(\S+) idle_line=".*?" status_pane="(.*?)"', t)
        tabs[m[1]] = (m[2], m[3], m[4])
    return line[1], int(line[2]), line[3] == "true", tabs
TS_PATH = "export PATH={d}/bin:$PATH"
TS_TABS = [
    # failed and done: commands that end while their tab is out of view
    f"type {TS_PATH}; cd {{d}}/w/a/src; sleep 2; false\\n", "key cmd+t", "wait_prompt",
    "type cd {d}/w/b/src; sleep 2; true\\n", "key cmd+t", "wait_prompt",
    "type cd {d}/w/run\\n", "wait 300", "type sleep 60\\n", "key cmd+t", "wait_prompt",
    # a renamed tab on the same folder as b/src
    "type cd {d}/w/b/src\\n", "wait 300", "key ctrl+alt+r", "type build\\n", "key cmd+t", "wait_prompt",
    # a split tab: the agent in the second pane, the first focused
    f"type {TS_PATH}; cd {{d}}/w/flow\\n", "wait 400", "key cmd+d", "wait_prompt",
    f"type {TS_PATH}; cd {{d}}/w/atlas\\n", "wait 400", "type claude\\n", "wait 900",
    "key alt+cmd+left", "wait 4000",
]
ts_dot = ts_run("dot", "dot", [
    *TS_TABS, "dump {d}/a.txt", "shot {d}/a.png",
    "mods none", "mods cmd", "wait 150", "dump {d}/early.txt", "wait 400", "dump {d}/b.txt",
    "mods none", "wait 200", "dump {d}/c.txt",
    "key alt+cmd+right", "wait 400", "dump {d}/d.txt"])
style, h, hints, tabs = ts_strip(ts_dot("a"))
assert (style, h, hints) == ("Dot", 34, False), (style, h, hints)
assert tabs["a/src"][0] == "failed/own" and tabs["b/src"][0] == "done/own", tabs
assert tabs["run"][0] == "working/own" and tabs["build"][0] == "-", tabs
assert tabs["flow"] == ("needs_you/other", "-", "atlas"), tabs
assert 'Tab "flow, needs you in another pane' in ts_dot("a"), ts_dot("a")[-3000:]
# ⌘ held: nothing for the first moment, then every tab's number
assert ts_strip(ts_dot("early"))[2] is False, ts_strip(ts_dot("early"))
_, _, hints, tabs = ts_strip(ts_dot("b"))
assert hints and [t[1] for t in tabs.values()] == ["⌘1", "⌘2", "⌘3", "⌘4", "⌘5"], tabs
_, _, hints, tabs = ts_strip(ts_dot("c"))
assert not hints and {t[1] for t in tabs.values()} == {"-"}, tabs
# the agent's pane focused: the tab is named for it, the status its own
assert ts_strip(ts_dot("d"))[3]["atlas"][0] == "needs_you/own", ts_strip(ts_dot("d"))
# the underline style: the failed tab's title is underlined in the error
# colour; the detailed style: a 44 px strip whose second line says it
def ts_tab_rect(text, title):
    return tuple(map(int, re.search(re.escape(title) + r"@(\d+),(\d+),(\d+),(\d+)", text).groups()))
TS_SHORT = [f"type cd {{d}}/w/a/src; sleep 2; false\\n", "key cmd+t", "wait_prompt",
            "type cd {d}/w/flow\\n", "wait 3500", "dump {d}/a.txt", "shot {d}/a.png"]
for name in ("underline", "detailed"):
    dump = ts_run(name, name, TS_SHORT)("a")
    style, h, _, tabs = ts_strip(dump)
    assert style.lower() == name and tabs["src"][0] == "failed/own", (style, tabs)
    assert h == (44 if name == "detailed" else 34), h
    x, y, w, th = ts_tab_rect(dump, "src")
    assert th == h, (th, h)
    # a red mark under the title (the underline) / on the second line
    row = y + th // 2 + (10 if name == "underline" else 9)
    marks = [mp_pixel(ts / name / "a.png", px, row, 1240) for px in range(x + 20, x + w - 20)]
    assert any(r > b + 60 and r > 150 for r, g, b in marks), (name, row, marks[::12])
# new output: a background tab's late output counts once it ran a command;
# fresh tabs left before their first prompt (startup only) stay quiet
quiet = ts_run("quiet", "dot", [
    "type cd {d}/w/run; (sleep 3; echo late) &\\n", "wait 500",
    "key cmd+t", "key cmd+t", "key cmd+t", "wait 4000", "dump {d}/a.txt"])
quiet_states = re.findall(r'status=(\S+)', re.search(r"^strip .*$", quiet("a"), re.M)[0])
assert quiet_states == ["activity/own", "-", "-", "-"], quiet_states

# remote-page: off, Settings › Remote access is one card (the switch) and
# the paired phones with Revoke only: no connection tiles, pairing, check
# box or empty headings (none of the phones list either without phones).
# On, the card says it is ready, Pair a phone shows the code inline with
# Copy link and Cancel, and Cancel withdraws the code (pairing.json loses
# it); the check box makes the next code view only
rp = scratch / "remote-page"
rp_base = ('shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n'
           'durable_sessions = false\nremote_listen = "127.0.0.1:0"\ncontrol_api = "off"\n')
rp_devices = [
    {"id": "a1b2", "name": "Test iPhone", "platform": "ios",
     "key": "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "permission": "operate",
     "paired_at": "2026-10-19T07:00:00Z", "last_seen": "2026-10-19T07:09:00Z"},
    {"id": "c3d4", "name": "Test Tablet", "platform": "android",
     "key": "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB", "permission": "view",
     "paired_at": "2026-10-12T07:00:00Z", "last_seen": "2026-10-18T13:40:00Z"}]

def rp_run(name, cfg, steps, devices=True):
    d = rp / name
    for sub in ("cfg/rift", "data/rift/remote"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    os.chmod(d / "data/rift/remote", 0o700)
    if devices:
        (d / "data/rift/remote/devices.json").write_text(json.dumps(rp_devices))
    (d / "cfg/rift/config.toml").write_text(cfg)
    (d / "off.toml").write_text(rp_base)
    (d / "t.rift").write_text("\n".join(
        ["resize 1180 1400", "wait_prompt", *(s.replace("{d}", str(d)) for s in steps), "quit"]) + "\n")
    with (d / "log.txt").open("w") as log:
        rc = subprocess.run([str(binary), "--script", str(d / "t.rift"), "--headless"],
                            env=dict(env, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data"),
                                     BASH_SILENCE_DEPRECATION_WARNING="1"),
                            stdout=log, stderr=log, timeout=120).returncode
    assert rc == 0, died(f"remote-page {name} run failed", d / "log.txt")
    return d

off = (rp_run("off", rp_base, [*settings_remote, "wait 600", "dump {d}/off.txt"]) / "off.txt").read_text()
assert 'Label "Remote access is off. Turn it on to watch' in off, off[-3000:]
assert 'Switch "Allow remote access" toggled=False' in off, off[-3000:]
assert 'Label "They can\'t connect while remote access is off.' in off, off[-3000:]
assert 'Button "Revoke Test iPhone"' in off and 'Button "Revoke Test Tablet"' in off, off[-3000:]
for gone in ("On this Wi-Fi", "Pair a phone", "The new phone can only watch", "access: ",
             "Connection settings", "Recent connections", "Ask before keeping"):
    assert gone not in off, (gone, off[-3000:])
alone = (rp_run("off-alone", rp_base, [*settings_remote, "wait 600", "dump {d}/off.txt"],
                devices=False) / "off.txt").read_text()
assert 'Switch "Allow remote access" toggled=False' in alone, alone[-3000:]
assert "They can" not in alone and "Revoke" not in alone and "No phones yet" not in alone, \
    alone[-3000:]

on = rp_run("on", rp_base + "remote_access = true\n", [
    "wait_file {d}/data/rift/remote/status.json 20000", "wait 1500", *settings_remote,
    "wait 600", "dump {d}/on.txt",
    "a11y click Pair a phone", "wait 800", "dump {d}/pair.txt",
    "sh cp {d}/data/rift/remote/pairing.json {d}/pairing-shown.json",
    "a11y click Cancel pairing: the code stops working", "wait 300", "dump {d}/cancel.txt",
    "sh cp {d}/data/rift/remote/pairing.json {d}/pairing-cancelled.json",
    "a11y click The new phone can only watch", "a11y click Pair a phone", "wait 800",
    "dump {d}/view.txt",
    "sh cp {d}/off.toml {d}/cfg/rift/config.toml", "wait 3500"])
text = (on / "on.txt").read_text()
assert 'Label "Ready for your phone. Paired phones can see your tabs' in text, text[-3000:]
assert 'Label "On this Wi-Fi (127.0.0.1:' in text, text[-3000:]
assert 'CheckBox "The new phone can only watch" toggled=False' in text, text[-3000:]
assert 'RadioButton "Test iPhone access: Can run commands, selected"' in text, text[-3000:]
assert 'RadioButton "Test Tablet access: View only, selected"' in text, text[-3000:]
pair = (on / "pair.txt").read_text()
assert 'pairing=Some("rift-remote://pair?' in pair and "&p=operate" in pair, remote_line(pair)
assert re.search(r'Label "Expires in [12]:\d\d"', pair), pair[-3000:]
for row in ('Button "Copy pairing link"', 'Button "Cancel pairing: the code stops working"',
            'Label "Pairing QR code"', "It will be able to run commands."):
    assert row in pair, (row, pair[-3000:])
assert len(json.loads((on / "pairing-shown.json").read_text())) == 1
assert json.loads((on / "pairing-cancelled.json").read_text()) == [], \
    (on / "pairing-cancelled.json").read_text()
assert "pairing=None pairing_wait=false" in (on / "cancel.txt").read_text()
view = (on / "view.txt").read_text()
assert "view_only=true" in remote_line(view) and "&p=view" in remote_line(view), remote_line(view)
assert 'CheckBox "The new phone can only watch" toggled=True' in view, view[-3000:]
# Revoke works while off (it asks first), and a phone's access changes
# with its segment while on: both write devices.json
gone = rp_run("revoke", rp_base, [*settings_remote, "wait 600",
                                  "a11y click Revoke Test iPhone", "wait 300", "key enter",
                                  "wait 600", "dump {d}/a.txt"])
left = json.loads((gone / "data/rift/remote/devices.json").read_text())
assert [x["id"] for x in left] == ["c3d4"], left
assert 'Button "Revoke Test iPhone"' not in (gone / "a.txt").read_text()
access = rp_run("access", rp_base + "remote_access = true\n", [
    "wait_file {d}/data/rift/remote/status.json 20000", "wait 1500", *settings_remote,
    # raising a phone to run commands asks first; lowering does not
    "wait 600", "a11y click Test Tablet access: Can run commands", "wait 300", "key enter",
    "wait 300", "a11y click Test iPhone access: View only", "wait 600",
    "sh cp {d}/off.toml {d}/cfg/rift/config.toml", "wait 3500"])
perms = {x["id"]: x["permission"]
         for x in json.loads((access / "data/rift/remote/devices.json").read_text())}
assert perms == {"a1b2": "view", "c3d4": "operate"}, perms

# themes-grid: Settings › Themes is a grid of cards, one per theme, under
# All / Dark / Light filters with their counts; a click on a card applies
# it; ← → ↑ ↓ walk the cards and Space applies the focused one; "Open
# themes folder" makes the folder (headless: never opened); the settings
# search finds a theme by name, and its row applies it
tg = scratch / "themes-grid"
for sub in ("cfg/rift", "data"):
    (tg / sub).mkdir(parents=True, exist_ok=True)
(tg / "cfg/rift/config.toml").write_text(
    'shell = "/bin/bash"\nrestore_session = false\nsync_os_theme = false\n')
(tg / "t.rift").write_text("\n".join([
    "resize 1180 900", "wait_prompt", "key cmd+,", "wait 300", "key end", "wait 600",
    f"dump {tg}/all.txt",
    "a11y click Show: Light  4", "wait 300", f"dump {tg}/light.txt",
    "a11y click Show: All  13", "wait 300",
    "a11y click Dracula", "wait 600", f"dump {tg}/dracula.txt",
    "focus on", *["key tab"] * 4, "wait 200", f"dump {tg}/first.txt",
    "key right", "key down", "wait 200", f"dump {tg}/walked.txt",
    "key space", "wait 600", f"sh cp {tg}/cfg/rift/config.toml {tg}/walked.toml",
    "key escape", "type nord", "wait 400", f"dump {tg}/search.txt",
    "a11y click Nord theme", "wait 600", f"sh cp {tg}/cfg/rift/config.toml {tg}/searched.toml",
    "key escape", "wait 200", "key escape", "wait 200", "key cmd+,", "wait 300", "key end",
    "wait 300", "move 700 400", "scroll -40", "wait 300",
    "a11y click Open themes folder: drop a .toml or a Warp .yaml theme into it", "wait 300",
    f"dump {tg}/opened.txt", "quit"]) + "\n")
with (tg / "log.txt").open("w") as log:
    rc = subprocess.run([str(binary), "--script", str(tg / "t.rift"), "--headless"],
                        env=dict(env, XDG_CONFIG_HOME=str(tg / "cfg"), XDG_DATA_HOME=str(tg / "data"),
                                 BASH_SILENCE_DEPRECATION_WARNING="1"),
                        stdout=log, stderr=log, timeout=90).returncode
assert rc == 0, died("themes-grid run failed", tg / "log.txt")
grid = (tg / "all.txt").read_text()
for row in ('RadioButton "Show: All  13, selected"', 'RadioButton "Show: Dark  9"',
            'RadioButton "Show: Light  4"', 'RadioButton "Dark Default, active"',
            'RadioButton "Solarized Light"'):
    assert row in grid, (row, grid[-3000:])
light = (tg / "light.txt").read_text()
assert 'RadioButton "Light Default"' in light and 'RadioButton "Dracula"' not in light, light[-3000:]
assert 'RadioButton "Dracula, active"' in (tg / "dracula.txt").read_text()
assert 'settings_focus=Some(SettingsTheme(0))' in (tg / "first.txt").read_text()
# 0 1 2 / 3 4 5 …: → from 0 is 1, ↓ from 1 is 4 (Catppuccin Latte)
assert 'settings_focus=Some(SettingsTheme(4))' in (tg / "walked.txt").read_text()
assert tomllib.loads((tg / "walked.toml").read_text())["theme"] == "catppuccin-latte"
assert 'Nord theme"' in (tg / "search.txt").read_text(), (tg / "search.txt").read_text()[-2000:]
assert tomllib.loads((tg / "searched.toml").read_text())["theme"] == "nord"
assert (tg / "cfg/rift/themes").is_dir(), "Open themes folder made no folder"
assert f'toast=Some("Headless: not opening {tg / "cfg/rift/themes"}' in (tg / "opened.txt").read_text(), \
    re.search(r"toast=.*", (tg / "opened.txt").read_text())

for client in clients:
    client.close()
stalled.close()
print(f"UI regressions passed ({time.monotonic() - started:.1f}s); {scratch}")
