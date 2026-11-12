//! Completions on the remote machine of an ssh pane, run over the pane's
//! shared ssh connection (`ControlMaster`): no new login, tens of
//! milliseconds.
//!
//! A pane whose working directory is on another host ([`Target`] parsed from
//! the `ssh` command it runs) completes there when the connection's control
//! socket is alive ([`control_socket`]). One `ssh -S <socket> -o
//! BatchMode=yes` exec per request runs a fixed script ([`SCRIPT`]); the
//! request travels on its stdin, never in the command. It lists the
//! directory being completed, `$PATH` commands (once per host), git refs,
//! and the remote shell's own completions through the `rift-complete`
//! function the remote hook installer adds ([`crate::ssh::installer_script`]).
//!
//! Replies are cached per host and served stale while a refresh runs
//! ([`complete`]). Remote output is untrusted: it is read up to
//! [`MAX_REPLY`] bytes, each line at most [`crate::shell_complete::MAX_FIELD`]
//! bytes, each section at most [`MAX_SECTION`] lines, and control
//! characters are stripped; nothing from it is ever put into a command.

use crate::shell_complete::{self, Item};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Time a request waits for the remote answer; a later one fills the cache.
pub const BUDGET: Duration = Duration::from_millis(400);
/// Bytes read from one remote reply.
pub const MAX_REPLY: usize = 1 << 20;
/// Lines kept per reply section.
pub const MAX_SECTION: usize = 5000;
/// A remote request still running after this is stopped.
const HARD_LIMIT: Duration = Duration::from_secs(5);
/// A cached reply younger than this is served without a refresh.
const FRESH: Duration = Duration::from_secs(2);
/// How long a host's command list is used before it is listed again.
const COMMANDS_TTL: Duration = Duration::from_secs(600);
/// How long an `ssh -G` resolution is reused.
const RESOLVE_TTL: Duration = Duration::from_secs(60);
/// Cached replies per host.
const ENTRIES: usize = 64;

/// The fixed script the remote login shell runs as `sh -c '<SCRIPT>'`. It
/// holds no single quote and no backslash before a quote or a backslash, so
/// sh, bash, zsh and fish all pass it to `sh` unchanged. It reads the
/// request from stdin: cwd, the directory to list (as typed, `~/` allowed),
/// flags (`p` paths, `c` commands, `g` git refs, `s` the shell's own
/// completions), then the shell request (line, n, n+1 words). `ls -q`
/// prints a name's control characters as `?`, so a file name can never
/// forge a section marker.
pub const SCRIPT: &str = r#"IFS= read -r cwd || exit 0
IFS= read -r dir
IFS= read -r flags
IFS= read -r line
IFS= read -r n
case $n in ""|*[!0-9]*) exit 0 ;; esac
words=
i=0
while [ "$i" -le "$n" ]; do
  IFS= read -r w || break
  words="$words$w
"
  i=$((i + 1))
done
cd -P -- "$cwd" 2>/dev/null || exit 0
case $flags in *p*)
  printf "\036paths\n"
  d=$dir
  case $d in "~") d=$HOME ;; "~/"*) d="$HOME/${d#??}" ;; esac
  [ -n "$d" ] || d=.
  (cd -P -- "$d" 2>/dev/null && ls -1Apq 2>/dev/null | head -n 5000) ;;
esac
case $flags in *c*)
  printf "\036cmds\n"
  (IFS=:; for p in $PATH; do [ -d "$p" ] && ls -1q "$p" 2>/dev/null; done) | sort -u | head -n 5000 ;;
esac
case $flags in *g*)
  printf "\036git\n"
  git for-each-ref --format="%(refname)" refs/heads refs/remotes refs/tags 2>/dev/null | head -n 5000 ;;
esac
case $flags in *s*)
  printf "\036shell\n"
  printf "1\n%s\n%s\n%s\n%s" "$PWD" "$line" "$n" "$words" | LC_TERMINAL=Rift "${SHELL:-sh}" -i -c rift-complete 2>/dev/null | head -n 2000 ;;
esac
printf "\036done\n"
"#;

/// The ssh destination of a pane: the options that pick the host, user,
/// port, config and control socket, and the host.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub args: Vec<String>,
    pub host: String,
}

/// ssh options that take an argument (ssh(1)).
const ARG_OPTS: &str = "BbcDEeFIiJLlmOoPpQRSWw";
/// `-o` keys kept: the ones that choose the destination and its socket.
const KEEP_KEYS: [&str; 7] = [
    "port",
    "user",
    "hostname",
    "controlpath",
    "proxyjump",
    "identityfile",
    "sendenv",
];

/// The [`Target`] of an `ssh` command line (`ssh -p 2222 me@host`), or
/// `None` when it is not one. Only the options that choose the destination
/// and its control socket are kept (`-p -l -F -J -i -S -4 -6` and those `-o`
/// keys); forwardings, `-t`, `-N`, `-M` and commands such as `LocalCommand`
/// are dropped, so running it again never does more than connect.
pub fn parse_ssh_command(cmd: &str) -> Option<Target> {
    let cmd = cmd.trim();
    if cmd.contains('\n') {
        return None;
    }
    let words = crate::completion_specs::tokenize(cmd);
    let (first, rest) = words.split_first()?;
    if first.rsplit('/').next() != Some("ssh") {
        return None;
    }
    let mut args = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let w = &rest[i];
        i += 1;
        if w == "--" {
            break;
        }
        let Some(flags) = w.strip_prefix('-') else {
            return host_target(args, w);
        };
        for (at, c) in flags.char_indices() {
            if ARG_OPTS.contains(c) {
                let inline = &flags[at + c.len_utf8()..];
                let value = if !inline.is_empty() {
                    inline.to_string()
                } else if i < rest.len() {
                    i += 1;
                    rest[i - 1].clone()
                } else {
                    return None;
                };
                match c {
                    'p' | 'l' | 'F' | 'J' | 'i' | 'S' => {
                        args.push(format!("-{c}"));
                        args.push(value);
                    }
                    'o' => {
                        let key = value
                            .split(['=', ' '])
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        if KEEP_KEYS.contains(&key.as_str()) {
                            args.push("-o".into());
                            args.push(value);
                        }
                    }
                    _ => {}
                }
                break;
            }
            if c == '4' || c == '6' {
                args.push(format!("-{c}"));
            }
        }
    }
    host_target(args, rest.get(i)?)
}

/// [`parse_ssh_command`] for a command read back from the screen, which
/// the remote side can forge (OSC 133): only the destination is kept —
/// `-p`, `-l`, `-4`, `-6` and the `Port`, `User`, `HostName` options with
/// plain values, and a plain host (`[user@]name`). A config file (`-F`),
/// control socket (`-S`, `ControlPath`), jump host, identity or anything
/// else is dropped, so `ssh -G` reads only the user's own config.
pub fn parse_screen_ssh_command(cmd: &str) -> Option<Target> {
    let t = parse_ssh_command(cmd)?;
    let plain = |v: &str| {
        !v.is_empty()
            && !v.starts_with('-')
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
    };
    let host_ok = match t.host.split_once('@') {
        Some((user, host)) => plain(user) && plain(host),
        None => plain(&t.host),
    };
    if !host_ok {
        return None;
    }
    let mut args = Vec::new();
    let mut it = t.args.into_iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "-4" | "-6" => args.push(flag),
            "-p" | "-l" => {
                let v = it.next()?;
                if plain(&v) {
                    args.extend([flag, v]);
                }
            }
            "-o" => {
                let v = it.next()?;
                let keep = v.split_once(['=', ' ']).is_some_and(|(key, value)| {
                    ["port", "user", "hostname"].contains(&key.to_ascii_lowercase().as_str())
                        && plain(value.trim())
                });
                if keep {
                    args.extend([flag, v]);
                }
            }
            // -F -J -i -S take a value
            _ => {
                it.next();
            }
        }
    }
    Some(Target { args, host: t.host })
}

fn host_target(args: Vec<String>, host: &str) -> Option<Target> {
    (!host.is_empty() && !host.starts_with('-') && !host.chars().any(char::is_control)).then(|| {
        Target {
            args,
            host: host.to_string(),
        }
    })
}

/// Run `cmd`, returning its stdout when it exits within `limit` (killed
/// otherwise).
fn run_bounded(mut cmd: Command, limit: Duration) -> Option<String> {
    crate::sys::own_group(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = out.take(MAX_REPLY as u64).read_to_string(&mut s);
        let _ = tx.send(s);
    });
    let got = rx.recv_timeout(limit).ok();
    if got.is_none() {
        stop(&mut child);
    } else {
        let _ = child.wait();
    }
    got
}

fn stop(child: &mut Child) {
    crate::sys::kill_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

/// The control socket `ssh -G` resolves for `target` (its `ControlPath`,
/// tokens expanded), `None` when it has none or cannot be resolved.
pub fn resolve_control_path(target: &Target) -> Option<PathBuf> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-G").args(&target.args).arg("--").arg(&target.host);
    let out = run_bounded(cmd, Duration::from_secs(2))?;
    control_path_of(&out)
}

/// The `controlpath` of `ssh -G` output, when it names a file.
pub fn control_path_of(ssh_g: &str) -> Option<PathBuf> {
    let path = ssh_g
        .lines()
        .find_map(|l| l.strip_prefix("controlpath "))?
        .trim();
    (!path.is_empty() && path != "none" && !path.contains('%') && path.starts_with('/'))
        .then(|| PathBuf::from(path))
}

/// Whether a master connection listens on `path`.
pub fn socket_alive(path: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }
    // Windows OpenSSH has no connection sharing (ControlMaster)
    #[cfg(windows)]
    {
        let _ = path;
        false
    }
}

/// When a target's control path was resolved, and to what.
type Resolution = (Instant, Option<PathBuf>);

static RESOLVED: LazyLock<Mutex<HashMap<Target, Resolution>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The live control socket for `target`, if any: its `ControlPath`
/// (resolved once a minute) with a master listening on it.
pub fn control_socket(target: &Target) -> Option<PathBuf> {
    let now = Instant::now();
    let cached = RESOLVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(target)
        .filter(|(at, _)| now.duration_since(*at) < RESOLVE_TTL)
        .map(|(_, p)| p.clone());
    let path = match cached {
        Some(p) => p,
        None => {
            let p = resolve_control_path(target);
            RESOLVED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(target.clone(), (now, p.clone()));
            p
        }
    };
    path.filter(|p| socket_alive(p))
}

/// What one request asks the remote machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ask {
    pub cwd: String,
    /// The directory to list (`p`), as typed (unescaped), `""` = cwd.
    pub dir: String,
    pub paths: bool,
    pub commands: bool,
    pub git: bool,
    /// The remote shell's own completions for this request.
    pub shell: Option<shell_complete::Query>,
}

/// The remote answer, bounded and cleaned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    /// Entry names of `dir`; directories end in `/`.
    pub paths: Vec<String>,
    pub commands: Vec<String>,
    /// (ref, kind): kind is "branch", "remote" or "tag".
    pub git: Vec<(String, String)>,
    pub shell: Vec<Item>,
}

/// The request text the script reads.
pub fn encode(ask: &Ask) -> Option<String> {
    let mut flags = String::new();
    for (on, f) in [
        (ask.paths, 'p'),
        (ask.commands, 'c'),
        (ask.git, 'g'),
        (ask.shell.is_some(), 's'),
    ] {
        if on {
            flags.push(f);
        }
    }
    let fields = [&ask.cwd, &ask.dir];
    if fields.iter().any(|f| f.chars().any(char::is_control)) {
        return None;
    }
    let mut s = format!("{}\n{}\n{flags}\n", ask.cwd, ask.dir);
    match &ask.shell {
        Some(q) => {
            // the shell request without its id and cwd (the script adds them)
            let body = shell_complete::encode(0, q);
            let body = body.split_once('\n')?.1.split_once('\n')?.1;
            s.push_str(body);
        }
        None => s.push_str("\n0\n\n"),
    }
    Some(s)
}

/// Parse the script's output (see [`SCRIPT`]): sections after their
/// `\x1e<name>` lines, each bounded and cleaned.
pub fn parse_reply(out: &str) -> Reply {
    let mut r = Reply::default();
    let mut section = "";
    let mut shell_lines: Vec<&str> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in out.lines().take(4 * MAX_SECTION + 16) {
        // a marker may follow output that had no newline of its own
        if let Some(name) = line.rfind('\x1e').map(|at| &line[at + 1..]) {
            // the shell server's own markers (ready, begin, end) are not
            // sections
            match name {
                "paths" | "cmds" | "git" | "shell" | "done" => {
                    section = match name {
                        "paths" => "paths",
                        "cmds" => "cmds",
                        "git" => "git",
                        "shell" => "shell",
                        _ => "",
                    };
                }
                _ => {}
            }
            continue;
        }
        let n = counts.entry(section).or_insert(0);
        if *n >= MAX_SECTION {
            continue;
        }
        *n += 1;
        match section {
            "shell" => shell_lines.push(line),
            "paths" | "cmds" | "git" => {
                let v = shell_complete::clean(line);
                if v.is_empty() || v == "./" || v == "../" {
                    continue;
                }
                match section {
                    "paths" => r.paths.push(v),
                    "cmds" => r.commands.push(v),
                    _ => {
                        let (name, kind) = if let Some(b) = v.strip_prefix("refs/heads/") {
                            (b, "branch")
                        } else if let Some(t) = v.strip_prefix("refs/tags/") {
                            (t, "tag")
                        } else if let Some(m) = v.strip_prefix("refs/remotes/") {
                            (m, "remote")
                        } else {
                            continue;
                        };
                        if !name.ends_with("/HEAD") {
                            r.git.push((name.to_string(), kind.to_string()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    r.shell = shell_complete::parse_items(shell_lines);
    r
}

/// Run one request over the master on `socket`, at most 5 seconds.
pub fn fetch(socket: &Path, target: &Target, ask: &Ask) -> Option<Reply> {
    let input = encode(ask)?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-S")
        .arg(socket)
        .args([
            "-o",
            "ControlMaster=no",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=2",
            // the master only: when the socket is gone, ssh would connect
            // anew; this proxy fails at once instead
            "-o",
            "ProxyCommand=false",
            "-T",
        ])
        .args(&target.args)
        .arg("--")
        .arg(&target.host)
        // one fixed word: the remote shell runs `sh -c '<SCRIPT>'`
        .arg(format!("sh -c '{SCRIPT}'"));
    crate::sys::own_group(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let out = child.stdout.take()?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = out.take(MAX_REPLY as u64).read_to_end(&mut buf);
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);
    let got = rx.recv_timeout(HARD_LIMIT).ok();
    if got.is_none() {
        stop(&mut child);
    } else {
        let _ = child.wait();
    }
    // a reply cut short (the connection died) is still bounded and clean
    got.map(|s| parse_reply(&s))
}

#[derive(Default)]
struct HostCache {
    commands: Option<(Instant, Arc<Vec<String>>)>,
    commands_loading: bool,
    entries: HashMap<Ask, (Instant, Reply)>,
    loading: std::collections::HashSet<Ask>,
}

static HOSTS: LazyLock<Mutex<HashMap<PathBuf, HostCache>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn hosts() -> std::sync::MutexGuard<'static, HashMap<PathBuf, HostCache>> {
    HOSTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Start `ask` on a thread; its reply is stored in the host cache and sent
/// on the returned channel.
fn spawn_fetch(socket: &Path, target: &Target, ask: Ask) -> Receiver<Option<Reply>> {
    let (tx, rx) = channel();
    let (socket, target) = (socket.to_path_buf(), target.clone());
    let (socket_key, ask_key) = (socket.clone(), ask.clone());
    hosts()
        .entry(socket.clone())
        .or_default()
        .loading
        .insert(ask.clone());
    let spawned = std::thread::Builder::new()
        .name("remote-completion".into())
        .spawn(move || {
            let reply = fetch(&socket, &target, &ask);
            let mut all = hosts();
            let host = all.entry(socket).or_default();
            host.loading.remove(&ask);
            if let Some(r) = &reply {
                let now = Instant::now();
                if !r.commands.is_empty() {
                    host.commands = Some((now, Arc::new(r.commands.clone())));
                }
                if ask.commands {
                    host.commands_loading = false;
                }
                if host.entries.len() >= ENTRIES
                    && !host.entries.contains_key(&ask)
                    && let Some(old) = host
                        .entries
                        .iter()
                        .min_by_key(|(_, (at, _))| *at)
                        .map(|(k, _)| k.clone())
                {
                    host.entries.remove(&old);
                }
                host.entries.insert(ask, (now, r.clone()));
            } else if ask.commands {
                host.commands_loading = false;
            }
            drop(all);
            let _ = tx.send(reply);
        });
    if spawned.is_err() {
        // `rx` then reports a disconnect: no answer
        log::warn!("remote completion thread did not start");
        let mut all = hosts();
        if let Some(host) = all.get_mut(&socket_key) {
            host.loading.remove(&ask_key);
            if ask_key.commands {
                host.commands_loading = false;
            }
        }
    }
    rx
}

/// The remote answer for `ask` through the master on `socket`, and the
/// host's `$PATH` commands when `ask.commands` (listed once, then cached).
/// A cached reply is returned at once (refreshed in the background when
/// older than 2 s); otherwise the request waits at most `budget` and a later
/// answer only fills the cache.
pub fn complete(socket: &Path, target: &Target, mut ask: Ask, budget: Duration) -> Reply {
    let now = Instant::now();
    let want_commands = ask.commands;
    let mut cached_cmds: Option<Arc<Vec<String>>> = None;
    {
        let mut all = hosts();
        let host = all.entry(socket.to_path_buf()).or_default();
        if want_commands {
            match &host.commands {
                Some((at, c)) => {
                    cached_cmds = Some(c.clone());
                    if now.duration_since(*at) < COMMANDS_TTL || host.commands_loading {
                        ask.commands = false;
                    } else {
                        host.commands_loading = true;
                    }
                }
                None if host.commands_loading => ask.commands = false,
                None => host.commands_loading = true,
            }
        }
    }
    let with_commands = |mut r: Reply| {
        if want_commands && r.commands.is_empty() {
            r.commands = cached_cmds.as_deref().cloned().unwrap_or_default();
        }
        r
    };
    let nothing_asked = !ask.paths && !ask.commands && !ask.git && ask.shell.is_none();
    if nothing_asked {
        return with_commands(Reply::default());
    }
    let (cached, loading) = {
        let all = hosts();
        let host = all.get(socket);
        (
            host.and_then(|h| h.entries.get(&ask).cloned()),
            host.is_some_and(|h| h.loading.contains(&ask)),
        )
    };
    if let Some((at, reply)) = cached {
        if now.duration_since(at) >= FRESH && !loading {
            drop(spawn_fetch(socket, target, ask));
        }
        return with_commands(reply);
    }
    if loading {
        return with_commands(Reply::default());
    }
    let rx = spawn_fetch(socket, target, ask);
    match rx.recv_timeout(budget) {
        Ok(Some(r)) => with_commands(r),
        Ok(None) | Err(RecvTimeoutError::Disconnected) | Err(RecvTimeoutError::Timeout) => {
            with_commands(Reply::default())
        }
    }
}

/// Merge the remote machine's completions for the word ending at `end` in
/// `buffer` into `g` (Rift's own candidates for the pane, which has no local
/// directory): command names for the first word, the listing of the typed
/// directory for a path, git refs where a spec wants them, and the remote
/// shell's own completions for any other argument (the remote directory's
/// entries when it has none). `cwd` is the pane's remote directory.
pub fn complete_into(
    g: &mut crate::completion::Gathered,
    socket: &Path,
    target: &Target,
    buffer: &str,
    end: usize,
    cwd: &str,
    budget: Duration,
) {
    use crate::completion_specs::ArgKind;
    let path_like = g.path_like();
    let first = g.first_word();
    let typed = g.typed().to_string();
    let dir = match (path_like, typed.rfind('/')) {
        (true, Some(i)) => typed[..=i].to_string(),
        _ => String::new(),
    };
    let arg = g.arg_kind();
    // the remote shell is not known here: refuse every shell's
    // substitution syntax (fish's `(` included)
    let shell = (!first && !path_like)
        .then(|| shell_complete::query(crate::shell_env::ShellKind::Fish, buffer, end, cwd))
        .flatten();
    let ask = Ask {
        cwd: cwd.to_string(),
        dir,
        paths: path_like || !first,
        commands: first && !path_like,
        git: matches!(arg, ArgKind::GitRef | ArgKind::GitBranch),
        shell,
    };
    let r = complete(socket, target, ask, budget);
    g.add_commands(&r.commands);
    g.add(&r.shell, "shell");
    if path_like || (!first && r.shell.is_empty()) {
        g.add_paths(&r.paths);
    }
    let refs: Vec<(String, String)> = r
        .git
        .into_iter()
        .filter(|(_, kind)| arg != ArgKind::GitBranch || kind == "branch")
        .collect();
    g.add_matching(
        &refs,
        |kind| match kind {
            "branch" => "branch",
            "tag" => "tag",
            _ => "remote-branch",
        },
        50,
    );
}

/// Forget every cached reply and resolution (tests).
pub fn clear_cache() {
    hosts().clear();
    RESOLVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(args: &[&str], host: &str) -> Option<Target> {
        Some(Target {
            args: args.iter().map(|s| s.to_string()).collect(),
            host: host.into(),
        })
    }

    #[test]
    fn ssh_commands_parse_to_their_destination() {
        assert_eq!(parse_ssh_command("ssh prod"), t(&[], "prod"));
        assert_eq!(
            parse_ssh_command(
                "ssh -o SendEnv=LC_TERMINAL -o ControlMaster=auto -o 'ControlPath=/d d/%C' -o ControlPersist=60 -p 2222 -- me@host"
            ),
            t(
                &[
                    "-o",
                    "SendEnv=LC_TERMINAL",
                    "-o",
                    "ControlPath=/d d/%C",
                    "-p",
                    "2222"
                ],
                "me@host"
            )
        );
        assert_eq!(
            parse_ssh_command(
                "/usr/bin/ssh -tt -A -L 8080:localhost:80 -p2222 -i ~/k -4 box uptime"
            ),
            t(&["-p", "2222", "-i", "~/k", "-4"], "box")
        );
        // commands ssh would run are never kept
        assert_eq!(
            parse_ssh_command(
                "ssh -o LocalCommand=touch\\ x -o PermitLocalCommand=yes -o ProxyCommand=nc h"
            ),
            t(&[], "h")
        );
        assert_eq!(parse_ssh_command("ssh-keygen -t ed25519"), None);
        assert_eq!(parse_ssh_command("ssh"), None);
        assert_eq!(parse_ssh_command("ssh -p"), None);
        assert_eq!(parse_ssh_command("ls -la"), None);
    }

    #[test]
    fn screen_ssh_commands_keep_only_the_destination() {
        // what a remote program could print as a command: a config file
        // with `Match exec`, a control socket of its choosing
        let forged = "ssh -F /tmp/evil.conf -S /tmp/sock -o ControlPath=/tmp/x \
                      -o ProxyJump=j -J j2 -i /tmp/k -o Port=2222 -p 22 -l me -4 box";
        assert_eq!(
            parse_screen_ssh_command(forged),
            t(&["-o", "Port=2222", "-p", "22", "-l", "me", "-4"], "box")
        );
        // control: typed in the editor, the same command keeps them
        let typed = parse_ssh_command(forged).unwrap();
        assert!(typed.args.contains(&"-F".to_string()), "{typed:?}");
        assert!(typed.args.contains(&"-S".to_string()), "{typed:?}");
        assert_eq!(
            parse_screen_ssh_command("ssh me@box.example"),
            t(&[], "me@box.example")
        );
        for bad in [
            "ssh 'box;touch x'",
            "ssh me@'b x'",
            "ssh $(id)",
            "ssh -l 'me x' box",
        ] {
            let t = parse_screen_ssh_command(bad);
            assert!(
                t.as_ref()
                    .is_none_or(|t| !t.args.iter().any(|a| a.contains(' '))
                        && t.host
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || ".-_@".contains(c))),
                "{bad}: {t:?}"
            );
        }
    }

    #[test]
    fn control_path_comes_from_ssh_g() {
        let out = "user me\ncontrolmaster auto\ncontrolpath /tmp/x/abc\n";
        assert_eq!(control_path_of(out), Some(PathBuf::from("/tmp/x/abc")));
        assert_eq!(control_path_of("controlpath none\n"), None);
        assert_eq!(
            control_path_of("controlpath ~/.ssh/%C\n"),
            None,
            "unexpanded"
        );
        assert_eq!(control_path_of("user me\n"), None);
    }

    #[test]
    #[cfg(unix)]
    fn the_script_survives_every_remote_shells_single_quotes() {
        assert!(!SCRIPT.contains('\''));
        assert!(!SCRIPT.contains("\\\\"));
        // and runs under sh against a request, listing a directory
        let dir = std::env::temp_dir().join(format!("rift-remote-script-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file a"), "").unwrap();
        // a name that would forge the shell section without `ls -q`
        std::fs::write(dir.join("x\x1eshell\n\x1fforged"), "").unwrap();
        let ask = Ask {
            cwd: dir.to_string_lossy().into_owned(),
            dir: String::new(),
            paths: true,
            commands: false,
            git: false,
            shell: None,
        };
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(SCRIPT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(encode(&ask).unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let r = parse_reply(&String::from_utf8_lossy(&out.stdout));
        assert_eq!(r.paths, vec!["file a", "sub/", "x?shell??forged"]);
        assert!(r.shell.is_empty(), "{:?}", r.shell);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn remote_output_is_bounded_and_cleaned() {
        let mut out = String::from("motd\n\x1epaths\n");
        out.push_str("ok\n\x1b]0;evil\x07name\x1b[31m\n");
        for i in 0..MAX_SECTION + 50 {
            out.push_str(&format!("f{i}\n"));
        }
        out.push_str("\x1egit\nrefs/heads/main\nrefs/remotes/origin/HEAD\nrefs/tags/v1\nweird\n");
        out.push_str("\x1eshell\n\x1eready\n\x1ebegin 1\n\x1fdeploy\x1fship\nnoise\n\x1eend 1\n");
        out.push_str("\x1edone\n\x1fafter\x1fthe end\n");
        let r = parse_reply(&out);
        assert_eq!(r.paths[0], "ok");
        assert_eq!(r.paths[1], "name", "escape sequences dropped");
        assert_eq!(r.paths.len(), MAX_SECTION);
        assert_eq!(
            r.git,
            vec![
                ("main".to_string(), "branch".to_string()),
                ("v1".to_string(), "tag".to_string())
            ]
        );
        assert_eq!(r.shell, vec![("deploy".to_string(), "ship".to_string())]);
        // a newline in a request field would desynchronise the script
        let bad = Ask {
            cwd: "/a\nb".into(),
            dir: String::new(),
            paths: true,
            commands: false,
            git: false,
            shell: None,
        };
        assert_eq!(encode(&bad), None);
    }
}
