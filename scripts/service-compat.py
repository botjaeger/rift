"""A session service from the installed release (before profiles) and a new
window: plain panes stay durable on it; a profile pane with arguments starts
locally with its arguments and says why. Also the new service with an old
window. Every process here is our own child, stopped by its pid."""
import os, shutil, signal, subprocess, sys, time
from pathlib import Path

R = Path("~/code/rift")
OLD = R / "target/Rift.app/Contents/MacOS/rift-terminal"   # today's 0.14.0 build, pre-profiles
NEW = R / "target/debug/rift-terminal"
root = Path(sys.argv[1])
shutil.rmtree(root, ignore_errors=True)


def env_for(d):
    return dict(os.environ, XDG_CONFIG_HOME=str(d / "cfg"), XDG_DATA_HOME=str(d / "data"),
                RIFT_NO_NOTIFY="1", RIFT_PERSIST_SESSION="1")


def run(case, daemon_bin, window_bin, profile):
    d = root / case
    (d / "cfg/rift").mkdir(parents=True)
    (d / "data").mkdir()
    shutil.copy(daemon_bin, d / "svc")      # a copy: not the bundle, no launchd
    cfg = 'restore_session = false\ndurable_sessions = true\nsync_os_theme = false\nshell = "/bin/zsh"\n'
    if profile:
        cfg += ('\n[[profiles]]\nname = "argy"\ncommand = "/bin/zsh"\n'
                'args = ["-c", "echo ARGS_OK; exec /bin/zsh -i"]\n')
    (d / "cfg/rift/config.toml").write_text(cfg)
    svc = subprocess.Popen([str(d / "svc"), "--session-daemon"], env=env_for(d),
                           stdout=open(d / "svc.log", "w"), stderr=subprocess.STDOUT)
    time.sleep(1.5)
    steps = ["resize 900 600", "wait_prompt", "type echo PPID=$PPID\\n", "wait 1000", f"dump {d}/one.txt"]
    if profile:
        steps += ["key ctrl+shift+1", "wait 2500", f"dump {d}/two.txt", f"shot {d}/two.png"]
    steps += ["quit"]
    (d / "t.rift").write_text("\n".join(steps) + "\n")
    rc = subprocess.run([str(window_bin), "--script", str(d / "t.rift"), "--headless"],
                        env=env_for(d), stdout=open(d / "app.log", "w"),
                        stderr=subprocess.STDOUT, timeout=120).returncode
    print("  svc pid", svc.pid)
    svc.send_signal(signal.SIGTERM)
    svc.wait(timeout=10)
    print(f"== {case} rc={rc}")
    for name in ("one", "two"):
        p = d / f"{name}.txt"
        if p.exists():
            for l in p.read_text().splitlines():
                if "launch profile" in l or l.startswith("toast") or "toasts" in l or "service=" in l:
                    print(" ", name, l.strip()[:200])
    log = (d / "app.log").read_text()
    for l in log.splitlines():
        if "session service" in l or "predates" in l:
            print("  log", l[-160:])


run("old-svc-new-window", OLD, NEW, True)
run("new-svc-old-window", NEW, OLD, False)
