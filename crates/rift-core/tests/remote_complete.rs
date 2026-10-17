//! Remote completions over an SSH profile's shared connection, against a
//! real sshd in docker (packaging/sshd-fixture.Dockerfile) reached with a
//! throwaway key and a scratch ssh config (`-F`): the user's ~/.ssh is never
//! read. Needs docker:
//!
//!   cargo test -p rift-core --test remote_complete -- --ignored --nocapture

use rift_core::completion::{Completion, gather_remote};
use rift_core::remote_complete::{self, Target};
use rift_core::ssh::SshProfile;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Sshd {
    dir: PathBuf,
    container: String,
    config: PathBuf,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // the master this test started, then the container it started
        let _ = Command::new("ssh")
            .args(["-F"])
            .arg(&self.config)
            .args(["-O", "exit", "rifttest"])
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.stderr(Stdio::inherit()).output().expect("spawn");
    assert!(out.status.success(), "{cmd:?} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn start(tag: &str) -> Sshd {
    // short: the control socket path must fit a sockaddr_un
    let dir = PathBuf::from(format!("/tmp/rrc{}{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(dir.join("id")));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging");
    run(Command::new("docker")
        .args(["build", "-q", "-t", "rift-sshd-fixture", "-f"])
        .arg(root.join("sshd-fixture.Dockerfile"))
        .arg(&root));
    let container = run(Command::new("docker")
        .args(["run", "-d", "--rm", "-p", "127.0.0.1::22", "-v"])
        .arg(format!(
            "{}:/keys/rift.pub:ro",
            dir.join("id.pub").display()
        ))
        .arg("rift-sshd-fixture"));
    let port = run(Command::new("docker").args(["port", &container, "22"]));
    let port = port
        .lines()
        .next()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .to_string();
    let config = dir.join("config");
    std::fs::write(
        &config,
        format!(
            "Host rifttest\n  HostName 127.0.0.1\n  Port {port}\n  User rift\n  IdentityFile {id}\n  \
             IdentitiesOnly yes\n  IdentityAgent none\n  StrictHostKeyChecking no\n  \
             UserKnownHostsFile {kh}\n  LogLevel ERROR\n",
            id = dir.join("id").display(),
            kh = dir.join("known_hosts").display()
        ),
    )
    .unwrap();
    let sshd = Sshd {
        dir,
        container,
        config,
    };
    // sshd takes a moment to listen
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let ok = Command::new("ssh")
            .arg("-F")
            .arg(&sshd.config)
            .args(["-o", "BatchMode=yes", "rifttest", "true"])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            break;
        }
        assert!(Instant::now() < until, "sshd never accepted the key");
        std::thread::sleep(Duration::from_millis(250));
    }
    sshd
}

/// The profile's command, connected in the background as the profile tab
/// does in the foreground (same options, plus `-N -f`).
fn connect(sshd: &Sshd) -> (String, Target) {
    let profile = SshProfile {
        name: "rifttest".into(),
        host: "rifttest".into(),
        args: vec!["-F".into(), sshd.config.to_string_lossy().into_owned()],
    };
    assert!(
        !profile.shares_connections_itself(),
        "the scratch config sets no ControlPath"
    );
    let control = rift_core::ssh::private_control_dir(&sshd.dir.join("ssh")).unwrap();
    let cmd = profile.command_shared(Some(&control));
    assert!(cmd.contains("ControlMaster=auto"), "{cmd}");
    let words = rift_core::completion_specs::tokenize(&cmd);
    let status = Command::new(&words[0])
        .args(["-N", "-f", "-o", "BatchMode=yes"])
        .args(&words[1..])
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "the profile command connects: {cmd}");
    let target = remote_complete::parse_ssh_command(&cmd).unwrap();
    (cmd, target)
}

fn complete(socket: &Path, target: &Target, line: &str, cwd: &str) -> Vec<Completion> {
    // as the app does for a remote pane: nothing from this machine
    let mut g = gather_remote(line, line.len());
    remote_complete::complete_into(
        &mut g,
        socket,
        target,
        line,
        line.len(),
        cwd,
        remote_complete::BUDGET,
    );
    g.finish(line, &[], &[])
}

/// Ask until the answer holds `want` (the first answer may come after the
/// budget and only fill the cache), at most 10 s.
fn complete_until(
    socket: &Path,
    target: &Target,
    line: &str,
    cwd: &str,
    want: &str,
) -> Vec<Completion> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let got = complete(socket, target, line, cwd);
        if got.iter().any(|c| c.insert == want) || Instant::now() > until {
            return got;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn inserts(items: &[Completion]) -> Vec<&str> {
    items.iter().map(|c| c.insert.as_str()).collect()
}

#[test]
#[ignore = "needs docker"]
fn remote_panes_complete_on_the_remote_machine() {
    remote_complete::clear_cache();
    let sshd = start("a");
    // the remote hooks (rift-complete) as the palette's installer puts them
    let installer = sshd.dir.join("install.sh");
    std::fs::write(&installer, rift_core::ssh::installer_script()).unwrap();
    run(Command::new("ssh")
        .arg("-F")
        .arg(&sshd.config)
        .args(["rifttest", "sh", "-s"])
        .stdin(std::fs::File::open(&installer).unwrap()));

    let (_cmd, target) = connect(&sshd);
    let t = Instant::now();
    let socket = remote_complete::control_socket(&target).expect("a live control socket");
    eprintln!("control socket resolved in {:?}", t.elapsed());
    let cwd = "/home/rift/proj";

    // paths: the remote directory, never the local one
    let got = complete_until(&socket, &target, "cat ", cwd, "remote-only.txt");
    let ins = inserts(&got);
    assert!(ins.contains(&"remote-only.txt"), "{ins:?}");
    assert!(ins.contains(&"src/"), "{ins:?}");
    assert!(!ins.contains(&"Cargo.toml"), "no local files: {ins:?}");
    let got = complete_until(&socket, &target, "ls src/", cwd, "src/main.rs");
    assert_eq!(inserts(&got), vec!["src/main.rs"]);

    // the remote shell's own completion function (no Rift spec for mycmd)
    let t = Instant::now();
    let fresh = remote_complete::fetch(
        &socket,
        &target,
        &remote_complete::Ask {
            cwd: cwd.into(),
            dir: String::new(),
            paths: false,
            commands: false,
            git: false,
            shell: rift_core::shell_complete::query(
                rift_core::shell_env::ShellKind::Fish,
                "mycmd de",
                8,
                cwd,
            ),
        },
    );
    eprintln!("remote shell completion, fresh: {:?}", t.elapsed());
    let words: Vec<String> = fresh.unwrap().shell.into_iter().map(|(w, _)| w).collect();
    assert_eq!(words, vec!["deploy", "destroy"]);
    let got = complete_until(&socket, &target, "mycmd de", cwd, "deploy");
    let shell: Vec<&str> = got
        .iter()
        .filter(|c| c.source == "shell")
        .map(|c| c.insert.as_str())
        .collect();
    assert_eq!(shell, vec!["deploy", "destroy"], "{got:?}");

    // git refs of the remote repository
    let got = complete_until(&socket, &target, "git checkout fea", cwd, "feature-remote");
    assert!(inserts(&got).contains(&"feature-remote"), "{got:?}");

    // commands on the remote $PATH (dpkg is not on macOS)
    let got = complete_until(&socket, &target, "dpkg-q", cwd, "dpkg-query");
    assert!(inserts(&got).contains(&"dpkg-query"), "{got:?}");

    // latency: a cached answer, and a fresh one over the shared connection
    let t = Instant::now();
    let _ = complete(&socket, &target, "cat ", cwd);
    let cached = t.elapsed();
    let t = Instant::now();
    let fresh = remote_complete::fetch(
        &socket,
        &target,
        &remote_complete::Ask {
            cwd: cwd.into(),
            dir: "src/".into(),
            paths: true,
            commands: false,
            git: false,
            shell: None,
        },
    );
    let fresh_t = t.elapsed();
    assert!(fresh.is_some_and(|r| r.paths == vec!["main.rs"]));
    eprintln!("remote completion: cached {cached:?}, fresh paths {fresh_t:?}");
    assert!(cached < Duration::from_millis(50), "{cached:?}");
    assert!(fresh_t < remote_complete::BUDGET, "{fresh_t:?}");

    // rift-complete in the other remote shells: the login shell changes and
    // the profile connects again (a master keeps the shell it logged in with)
    for (shell, tag) in [("/usr/bin/zsh", "zsh"), ("/usr/bin/fish", "fish")] {
        run(Command::new("docker").args(["exec", &sshd.container, "chsh", "-s", shell, "rift"]));
        run(Command::new("ssh")
            .arg("-F")
            .arg(&sshd.config)
            .arg("-S")
            .arg(&socket)
            .args(["-O", "exit", "rifttest"]));
        remote_complete::clear_cache();
        let (_, again) = connect(&sshd);
        assert_eq!(again, target);
        let t = Instant::now();
        let want = format!("deploy-{tag}");
        let got = complete_until(&socket, &target, "mycmd de", cwd, &want);
        let shell_rows: Vec<&str> = got
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        let first = t.elapsed();
        let t = Instant::now();
        let again = remote_complete::fetch(
            &socket,
            &target,
            &remote_complete::Ask {
                cwd: cwd.into(),
                dir: String::new(),
                paths: false,
                commands: false,
                git: false,
                shell: rift_core::shell_complete::query(
                    rift_core::shell_env::ShellKind::Fish,
                    "mycmd de",
                    8,
                    cwd,
                ),
            },
        );
        assert!(again.is_some_and(|r| !r.shell.is_empty()));
        eprintln!(
            "{shell}: remote shell completion first {first:?} (cold, retried), warm {:?}",
            t.elapsed()
        );
        assert_eq!(
            shell_rows,
            vec![want.clone(), format!("destroy-{tag}")],
            "{shell}: {got:?}"
        );
    }

    // the master goes away: no socket, no remote request, no hang
    run(Command::new("ssh")
        .arg("-F")
        .arg(&sshd.config)
        .arg("-S")
        .arg(&socket)
        .args(["-O", "exit", "rifttest"]));
    let t = Instant::now();
    assert_eq!(remote_complete::control_socket(&target), None);
    let took = t.elapsed();
    eprintln!("dead master detected in {took:?}");
    assert!(took < Duration::from_millis(100), "{took:?}");
    // a request already on its way when the master died never opens a new
    // connection: it fails at once
    let t = Instant::now();
    let late = remote_complete::fetch(
        &socket,
        &target,
        &remote_complete::Ask {
            cwd: cwd.into(),
            dir: String::new(),
            paths: true,
            commands: false,
            git: false,
            shell: None,
        },
    );
    let took = t.elapsed();
    eprintln!("request over the dead master ended in {took:?}");
    assert!(late.as_ref().is_none_or(|r| r.paths.is_empty()), "{late:?}");
    assert!(took < Duration::from_millis(500), "{took:?}");
}

#[test]
#[ignore = "needs docker"]
fn a_hand_typed_ssh_has_no_control_socket() {
    remote_complete::clear_cache();
    let sshd = start("b");
    // connected by hand, without connection sharing
    let cmd = format!("ssh -F {} rifttest", sshd.config.display());
    let target = remote_complete::parse_ssh_command(&cmd).unwrap();
    assert_eq!(remote_complete::control_socket(&target), None);
    // control: the same host through the profile does get one
    let (_, shared) = connect(&sshd);
    assert!(remote_complete::control_socket(&shared).is_some());
}
