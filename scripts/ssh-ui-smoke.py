#!/usr/bin/env python3
"""Real desktop palette → disposable SSH server → remote integration/completion.

Usage: python3 scripts/ssh-ui-smoke.py target/release/rift-terminal
Needs Docker and OpenSSH clients. Only the fixture container is modified.
Evidence stays in target/ssh-ui-*; private keys and control sockets are removed.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent


def run(args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, timeout=120, **kwargs)
    if result.returncode:
        raise RuntimeError(f"{args!r}: {result.returncode}\n{result.stdout}\n{result.stderr}")
    return result.stdout.strip()


def main():
    binary = Path(sys.argv[1]).resolve()
    assert binary.is_file(), binary
    # Short paths: SSH's Unix control sockets must fit sockaddr_un.
    scratch = Path(tempfile.mkdtemp(prefix="rsu-", dir="/tmp"))
    evidence = ROOT / "target" / f"ssh-ui-{scratch.name}"
    evidence.mkdir(parents=True)
    container = None
    env = {key: value for key, value in os.environ.items() if not key.startswith("RIFT_")}
    for key in ("SSH_AUTH_SOCK", "SSH_AGENT_PID", "ZDOTDIR", "BASH_ENV", "ENV",
                "ZSH_ENV"):
        env.pop(key, None)
    env.update(HOME=str(scratch / "home"), XDG_CONFIG_HOME=str(scratch / "cfg"),
               XDG_DATA_HOME=str(scratch / "data"), RIFT_NO_NOTIFY="1",
               HISTFILE=str(scratch / "home/history"), SHELL="/bin/bash")
    for directory in (scratch / "home", scratch / "cfg/rift", scratch / "data"):
        directory.mkdir(parents=True)
    config = scratch / "ssh-config"
    key = scratch / "id"

    def remote(command):
        return run(["docker", "exec", "--user", "rift", "-e", "HOME=/home/rift", container,
                    "bash", "--noprofile", "--norc", "-c", command])

    def ui(name, steps):
        script = evidence / f"{name}.rift"
        script.write_text("\n".join(["wait_prompt", *steps, "quit"]) + "\n")
        with (evidence / f"{name}.log").open("w") as log:
            result = subprocess.run([str(binary), "--script", str(script), "--headless"],
                                    env=env, stdout=log, stderr=log, timeout=120)
        assert result.returncode == 0, (name, (evidence / f"{name}.log").read_text()[-4000:])
        assert "wait_prompt timed out" not in (evidence / f"{name}.log").read_text(), name

    def palette(query):
        return ["key cmd+p", f"type {query}", "wait 250", "key enter", "wait 250"]

    try:
        run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)])
        run(["docker", "build", "-q", "-t", "rift-sshd-fixture", "-f",
             str(ROOT / "packaging/sshd-fixture.Dockerfile"), str(ROOT / "packaging")])
        container = run(["docker", "run", "-d", "--rm", "-p", "127.0.0.1::22",
                         "-v", f"{key}.pub:/keys/rift.pub:ro", "rift-sshd-fixture"])
        (evidence / "container-id.txt").write_text(container + "\n")
        port = run(["docker", "port", container, "22"]).splitlines()[0].rsplit(":", 1)[1]
        config.write_text(
            f"Host fixture\n HostName 127.0.0.1\n Port {port}\n User rift\n"
            f" IdentityFile {key}\n IdentitiesOnly yes\n IdentityAgent none\n"
            f" StrictHostKeyChecking no\n UserKnownHostsFile {scratch / 'known_hosts'}\n"
            " LogLevel ERROR\n BatchMode yes\n ConnectTimeout 2\n")
        deadline = time.monotonic() + 20
        while True:
            ready = subprocess.run(["ssh", "-F", str(config), "fixture", "true"],
                                   capture_output=True, timeout=5, env=env)
            if ready.returncode == 0:
                break
            assert time.monotonic() < deadline, ready.stderr.decode(errors="replace")
            time.sleep(0.2)
        (scratch / "cfg/rift/config.toml").write_text(
            'shell = "/bin/bash"\nrestore_session = false\ndurable_sessions = false\n'
            'discover_shells = false\nsync_os_theme = false\ncheck_for_updates = false\n'
            'shell_completions = true\n[[ssh_profiles]]\nname = "fixture"\n'
            f'host = "fixture"\nargs = ["-F", {json.dumps(str(config))}]\n')
        installer = "Install Rift shell integration on fixture"
        ui("cancel", [*palette(installer), f"dump {evidence}/confirm.txt",
                      "key escape", f"dump {evidence}/cancel.txt"])
        assert 'Install shell integration?' in (evidence / "confirm.txt").read_text()
        assert "confirm=None" in (evidence / "cancel.txt").read_text()
        assert remote('test ! -e "$HOME/.local/share/rift/rift.bash"; echo $?') == "0"
        ui("install", [*palette(installer), "a11y click Install", "wait 2000", "wait_prompt",
                       f"dump {evidence}/installed.txt", f"shot {evidence}/installed.png"])
        installed = (evidence / "installed.txt").read_text()
        assert "Rift shell integration installed on" in installed, installed[-5000:]
        assert 'exit=Some(0)' in installed, installed[-5000:]
        assert remote('test -s "$HOME/.local/share/rift/rift.bash" && '
                      'test -s "$HOME/.local/share/rift/complete.bash" && echo INSTALLED') == "INSTALLED"
        # A hung fixture fails through the normal SSH profile, then the same
        # profile succeeds once its disposable server resumes.
        run(["docker", "pause", container])
        ui("remote", [*palette("SSH: fixture"), "wait 4000", "wait_prompt",
            f"dump {evidence}/offline.txt", f"sh docker unpause {container}",
            *palette("SSH: fixture"), "wait 1500", "wait_prompt",
            "type cd ~/proj; echo GUI_REMOTE_OK > gui-result.txt; pwd; cat gui-result.txt; false\\n",
            "wait 400", "wait_prompt", f"dump {evidence}/remote.txt", f"shot {evidence}/remote.png",
            "type cat remote-", "key ctrl+space", "wait 1800", "key ctrl+space", "wait 500",
            f"dump {evidence}/paths.txt", f"shot {evidence}/paths.png", "key escape",
            "key tab", "wait 250", f"dump {evidence}/accepted.txt", "key escape", "key enter",
            "wait 300", "wait_prompt", f"dump {evidence}/completed.txt", "key ctrl+u",
            "type mycmd de", "key ctrl+space", "wait 1800", "key ctrl+space", "wait 500",
            f"dump {evidence}/shell.txt", f"shot {evidence}/shell.png", "key escape", "key ctrl+u",
            "type exit\\n", "wait 500", "wait_prompt", f"dump {evidence}/returned.txt"])
        offline = (evidence / "offline.txt").read_text()
        assert 'exit=Some(255)' in offline and 'timed out' in offline.lower(), offline[-5000:]
        state = (evidence / "remote.txt").read_text()
        assert "cwd_remote=true" in state and '/home/rift/proj' in state, state[-5000:]
        assert 'exit=Some(1)' in state and "GUI_REMOTE_OK" in state, state[-5000:]
        assert remote('cat "$HOME/proj/gui-result.txt"') == "GUI_REMOTE_OK"
        paths = (evidence / "paths.txt").read_text()
        assert 'remote-only.txt' in paths and 'Cargo.toml' not in paths.split('completion_items=', 1)[1].splitlines()[0], paths[-5000:]
        assert re.search(r'editor="cat remote-only.txt\s*"', (evidence / "accepted.txt").read_text())
        assert re.search(r'cmd="cat remote-only.txt\s*" exit=Some\(0\)',
                         (evidence / "completed.txt").read_text())
        shell = (evidence / "shell.txt").read_text()
        assert '"deploy"(shell)' in shell and '"destroy"(shell)' in shell, shell[-5000:]
        returned = (evidence / "returned.txt").read_text()
        assert "editor_active=true" in returned, returned[-3000:]
        print(f"SSH UI pass: cancel, install, timeout/retry, actual profile/PTY/output/exit, remote path/shell completion; {evidence}")
    finally:
        # Only this run's control masters; never discover or signal user sessions.
        control = scratch / "data/rift/ssh"
        if config.exists() and control.exists():
            for socket in control.iterdir():
                try:
                    subprocess.run(["ssh", "-F", str(config), "-S", str(socket), "-O", "exit", "fixture"],
                                   capture_output=True, timeout=10, env=env)
                except subprocess.TimeoutExpired:
                    print(f"own control master cleanup timed out: {socket}", file=sys.stderr)
        try:
            if container:
                run(["docker", "rm", "-f", container])
                remaining = run(["docker", "ps", "-aq", "--filter", f"id={container}"])
                assert not remaining, f"fixture container still exists: {remaining}"
        finally:
            shutil.rmtree(scratch)


if __name__ == "__main__":
    main()
