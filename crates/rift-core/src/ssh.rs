//! SSH profiles: named remote hosts opened with the system OpenSSH client.
//!
//! Profiles come from `[[ssh_profiles]]` in config.toml and from the concrete
//! `Host` entries of `~/.ssh/config` (and files it `Include`s). Rift never
//! handles keys, passwords or host keys itself: it runs `ssh`, so the user's
//! ssh config, agent, `known_hosts` checking and prompts all apply.
//!
//! Remote shell integration is a separate, explicitly confirmed step: an
//! installer run over ssh writes Rift's hook files to the remote
//! `~/.local/share/rift/` and adds one guarded line to the remote rc files.
//! The hooks only load when `LC_TERMINAL=Rift` arrives with the session
//! (Rift sets it and passes `-o SendEnv=LC_TERMINAL`; the server must
//! `AcceptEnv LC_*`, the Debian/Ubuntu default), so other terminals are
//! unaffected. Without it, ssh is still a normal raw terminal.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Marks the lines the installer adds, so it never adds them twice.
pub const RC_MARKER: &str = "# rift-shell-integration";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshProfile {
    /// Shown in the palette and as the tab title.
    pub name: String,
    /// `user@host` or a `Host` alias from ~/.ssh/config.
    pub host: String,
    /// Extra ssh arguments (e.g. `["-p", "2222"]`).
    #[serde(default)]
    pub args: Vec<String>,
}

/// Concrete (wildcard-free) `Host` names in an ssh config file, following
/// `Include` (relative paths resolve against `~/.ssh`), at most 8 levels.
pub fn config_hosts(path: &Path) -> Vec<String> {
    config_hosts_and_sources(path).0
}

/// [`config_hosts`] and every path it read or listed: each config file
/// tried (missing ones too) and each directory an `Include` glob listed, so
/// a cache can tell when the result may have changed.
pub fn config_hosts_and_sources(path: &Path) -> (Vec<String>, Vec<PathBuf>) {
    let mut out = Vec::new();
    let mut sources = Vec::new();
    collect(path, 0, &mut out, &mut sources);
    let mut seen = std::collections::HashSet::new();
    out.retain(|h| seen.insert(h.clone()));
    (out, sources)
}

fn collect(path: &Path, depth: usize, out: &mut Vec<String>, sources: &mut Vec<PathBuf>) {
    if depth > 8 {
        return;
    }
    sources.push(path.to_path_buf());
    let Ok(body) = std::fs::read_to_string(path) else {
        return;
    };
    let ssh_dir = dirs::home_dir().map(|h| h.join(".ssh")).unwrap_or_default();
    for line in body.lines() {
        // `Key value`, `Key=value` and `Key = value` are all valid.
        // Split the key first: `=` inside a Host alias is part of the alias.
        let line = line.trim();
        let key_end = line
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(line.len());
        let (key, tail) = line.split_at(key_end);
        let tail = tail.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
        let rest: Vec<&str> = tail
            .split_whitespace()
            .take_while(|word| !word.starts_with('#'))
            .collect();
        if key.eq_ignore_ascii_case("host") {
            for h in rest {
                if !h.contains(['*', '?', '!']) {
                    out.push(h.to_string());
                }
            }
        } else if key.eq_ignore_ascii_case("include") {
            for pattern in rest {
                let p = expand(pattern, &ssh_dir);
                if p.to_string_lossy().contains('*')
                    && let Some(dir) = p.parent()
                {
                    sources.push(dir.to_path_buf());
                }
                for file in glob_simple(&p) {
                    collect(&file, depth + 1, out, sources);
                }
            }
        }
    }
}

fn expand(p: &str, ssh_dir: &Path) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        return dirs::home_dir().unwrap_or_default().join(rest);
    }
    let path = PathBuf::from(p);
    if path.is_absolute() {
        path
    } else {
        ssh_dir.join(path)
    }
}

/// `dir/*` style globs in the last component only (what ssh configs use).
fn glob_simple(p: &Path) -> Vec<PathBuf> {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
    match (p.parent(), name) {
        (Some(dir), Some(name)) if name.contains('*') => {
            let (pre, post) = name.split_once('*').unwrap_or((&name, ""));
            let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|f| {
                            f.file_name()
                                .map(|n| n.to_string_lossy())
                                .is_some_and(|n| n.starts_with(pre) && n.ends_with(post))
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            files
        }
        _ => vec![p.to_path_buf()],
    }
}

/// Configured profiles first, then ssh config hosts not already named.
pub fn all_profiles(configured: &[SshProfile]) -> Vec<SshProfile> {
    let mut out: Vec<SshProfile> = configured.to_vec();
    if let Some(home) = dirs::home_dir() {
        for h in config_hosts(&home.join(".ssh/config")) {
            if !out.iter().any(|p| p.host == h || p.name == h) {
                out.push(SshProfile {
                    name: h.clone(),
                    host: h,
                    args: Vec::new(),
                });
            }
        }
    }
    out
}

/// POSIX single-quote when needed.
pub fn quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Where Rift keeps its shared-connection sockets: `<data dir>/ssh`.
pub fn control_dir() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("ssh"))
}

/// Longest socket path a `sockaddr_un` holds on macOS (104 with the NUL);
/// Linux allows 108.
const MAX_SOCKET_PATH: usize = 103;

/// `dir` ready for control sockets: created, mode 0700, and short enough
/// for `<dir>/<40-hex %C>`. `None` otherwise (the profile then connects
/// without sharing).
pub fn private_control_dir(dir: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if dir.as_os_str().len() + 1 + 40 > MAX_SOCKET_PATH {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if !meta.is_dir() {
        return None;
    }
    if meta.permissions().mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir.to_path_buf())
}

impl SshProfile {
    /// The command line typed into a new tab to connect (no connection
    /// sharing; see [`SshProfile::command_shared`]).
    pub fn command(&self) -> String {
        self.command_shared(None)
    }

    /// Whether the profile's own args or the user's ssh config already set
    /// up connection sharing (`ControlPath`/`ControlMaster`, `-S`), which
    /// Rift then leaves as it is: `ssh -G` reports a `ControlPath`.
    /// It runs `ssh -G` (up to 2 s): call it off the UI thread.
    pub fn shares_connections_itself(&self) -> bool {
        self.args_share_connections()
            || crate::remote_complete::resolve_control_path(&crate::remote_complete::Target {
                args: self.args.clone(),
                host: self.host.clone(),
            })
            .is_some()
    }

    /// Whether the profile's own args set up connection sharing: `-S`
    /// (case matters: `-s` asks for a subsystem), or a `ControlPath` /
    /// `ControlMaster` option.
    pub fn args_share_connections(&self) -> bool {
        self.args.iter().any(|a| {
            let lower = a.to_ascii_lowercase();
            a.starts_with("-S") || lower.contains("controlpath") || lower.contains("controlmaster")
        })
    }

    /// The command line typed into a new tab to connect. With `control`
    /// (from [`private_control_dir`]) ssh shares the connection through a
    /// socket there (`ControlMaster=auto`, `ControlPath=<control>/%C`,
    /// `ControlPersist=60`), which remote completions then run over.
    pub fn command_shared(&self, control: Option<&Path>) -> String {
        let mut parts = vec!["ssh".to_string(), "-o".into(), "SendEnv=LC_TERMINAL".into()];
        if let Some(dir) = control {
            parts.extend([
                "-o".into(),
                "ControlMaster=auto".into(),
                "-o".into(),
                quote(&format!("ControlPath={}/%C", dir.display())),
                "-o".into(),
                "ControlPersist=60".into(),
            ]);
        }
        parts.extend(self.args.iter().map(|a| quote(a)));
        // `--` so a host can never be read as an option
        parts.push("--".into());
        parts.push(quote(&self.host));
        parts.join(" ")
    }

    /// The command that runs the integration installer on the host (the
    /// script is fed on stdin, so nothing is interpolated remotely).
    pub fn install_command(&self, script: &Path) -> String {
        let mut parts = vec!["ssh".to_string()];
        parts.extend(self.args.iter().map(|a| quote(a)));
        parts.push("--".into());
        parts.push(quote(&self.host));
        parts.push("sh -s".into());
        format!("{} < {}", parts.join(" "), quote(&script.to_string_lossy()))
    }
}

/// POSIX sh installer: writes the hook files and appends one guarded
/// source line per shell (idempotent via [`RC_MARKER`]).
pub fn installer_script() -> String {
    use crate::shell_integration::{RIFT_BASH, RIFT_FISH, RIFT_ZSH};
    let heredoc =
        |path: &str, body: &str| format!("cat > \"{path}\" <<'RIFT_EOF'\n{body}\nRIFT_EOF\n");
    let mut s = String::from(
        "set -e\n\
         d=\"$HOME/.local/share/rift\"\n\
         mkdir -p \"$d\" \"$HOME/.config/fish/conf.d\"\n",
    );
    s.push_str(&heredoc("$d/rift.bash", RIFT_BASH));
    s.push_str(&heredoc("$d/rift.zsh", RIFT_ZSH));
    s.push_str(&heredoc("$d/rift.fish", RIFT_FISH));
    // the completion servers `rift-complete` runs (remote completions)
    {
        use crate::shell_complete::{SERVER_BASH, SERVER_FISH, SERVER_ZSH};
        s.push_str(&heredoc("$d/complete.bash", SERVER_BASH));
        s.push_str(&heredoc("$d/complete.zsh", SERVER_ZSH));
        s.push_str(&heredoc("$d/complete.fish", SERVER_FISH));
    }
    for (rc, file) in [(".bashrc", "rift.bash"), (".zshrc", "rift.zsh")] {
        s.push_str(&format!(
            "if ! grep -qs '{RC_MARKER}' \"$HOME/{rc}\"; then\n  \
             if [ -s \"$HOME/{rc}\" ] && [ -n \"$(tail -c1 \"$HOME/{rc}\")\" ]; then echo >> \"$HOME/{rc}\"; fi\n  \
             printf '%s\\n' '[ \"$LC_TERMINAL\" = Rift ] && [ -r \"$HOME/.local/share/rift/{file}\" ] && . \"$HOME/.local/share/rift/{file}\" {RC_MARKER}' >> \"$HOME/{rc}\"\n\
             fi\n"
        ));
    }
    s.push_str(&format!(
        "printf '%s\\n' 'if test \"$LC_TERMINAL\" = Rift; and test -r ~/.local/share/rift/rift.fish; source ~/.local/share/rift/rift.fish; end {RC_MARKER}' > \"$HOME/.config/fish/conf.d/rift.fish\"\n\
         echo \"Rift shell integration installed on $(hostname). Reconnect to use it.\"\n"
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_from_config_with_include() {
        let dir = std::env::temp_dir().join(format!("rift-ssh-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        std::fs::write(
            dir.join("config"),
            format!(
                "Host *\n  ServerAliveInterval 30\nHost prod web-1 *.corp\n  User me\n\
                 Include {}/conf.d/*\nHost=db\n",
                dir.display()
            ),
        )
        .unwrap();
        std::fs::write(dir.join("conf.d/a.conf"), "Host staging\nHost prod\n").unwrap();
        assert_eq!(
            config_hosts(&dir.join("config")),
            vec!["prod", "web-1", "staging", "db"]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_a_control_socket_option_shares_connections() {
        let p = |args: &[&str]| SshProfile {
            name: "p".into(),
            host: "h".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        };
        assert!(p(&["-S", "/tmp/sock"]).args_share_connections());
        assert!(p(&["-S/tmp/sock"]).args_share_connections());
        assert!(p(&["-o", "ControlPath=/tmp/%C"]).args_share_connections());
        assert!(p(&["-o", "controlmaster=auto"]).args_share_connections());
        // `-s` asks for a subsystem
        assert!(!p(&["-s"]).args_share_connections());
        assert!(!p(&["-p", "22"]).args_share_connections());
    }

    #[test]
    fn host_comments_are_not_profiles() {
        let dir = std::env::temp_dir().join(format!("rift-ssh-comments-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("included"), "Host child # another comment\n").unwrap();
        std::fs::write(
            dir.join("config"),
            format!(
                "Host alpha # not host names\nHost alpha#literal alpha=literal\nInclude {} # note\n",
                dir.join("included").display()
            ),
        )
        .unwrap();
        assert_eq!(
            config_hosts(&dir.join("config")),
            vec!["alpha", "alpha#literal", "alpha=literal", "child"]
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn commands_are_quoted() {
        let p = SshProfile {
            name: "x".into(),
            host: "me@host".into(),
            args: vec![
                "-p".into(),
                "2222".into(),
                "-o".into(),
                "ProxyJump=a b".into(),
            ],
        };
        assert_eq!(
            p.command(),
            "ssh -o SendEnv=LC_TERMINAL -p 2222 -o 'ProxyJump=a b' -- me@host"
        );
        let evil = SshProfile {
            name: "x".into(),
            host: "-oProxyCommand=touch /tmp/pwn".into(),
            args: vec![],
        };
        assert!(
            evil.command()
                .ends_with("-- '-oProxyCommand=touch /tmp/pwn'")
        );
        assert_eq!(
            p.install_command(Path::new("/tmp/i s.sh")),
            "ssh -p 2222 -o 'ProxyJump=a b' -- me@host sh -s < '/tmp/i s.sh'"
        );
    }

    #[test]
    fn profiles_share_their_connection_in_a_private_dir() {
        let p = SshProfile {
            name: "x".into(),
            host: "me@host".into(),
            args: vec!["-p".into(), "2222".into()],
        };
        assert_eq!(
            p.command_shared(Some(Path::new("/d/ssh"))),
            "ssh -o SendEnv=LC_TERMINAL -o ControlMaster=auto -o ControlPath=/d/ssh/%C \
             -o ControlPersist=60 -p 2222 -- me@host"
        );
        assert_eq!(
            p.command_shared(Some(Path::new("/a b/ssh"))),
            "ssh -o SendEnv=LC_TERMINAL -o ControlMaster=auto -o 'ControlPath=/a b/ssh/%C' \
             -o ControlPersist=60 -p 2222 -- me@host"
        );
        assert_eq!(p.command(), p.command_shared(None));
        // the profile's own sharing options are left alone
        let own = SshProfile {
            args: vec!["-o".into(), "ControlPath=~/.ssh/cm-%C".into()],
            ..p.clone()
        };
        assert!(own.shares_connections_itself());
        // the directory is created private, and loosened ones tightened
        use std::os::unix::fs::PermissionsExt;
        // short: macOS's $TMPDIR leaves no room for the socket name
        let base = Path::new("/tmp").join(format!("rift-ctl-{}", std::process::id()));
        let dir = base.join("ssh");
        assert_eq!(private_control_dir(&dir), Some(dir.clone()));
        let mode = |d: &Path| std::fs::metadata(d).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(private_control_dir(&dir), Some(dir.clone()));
        assert_eq!(mode(&dir), 0o700);
        // a path the socket name would not fit in is not used
        let long = base.join("x".repeat(80));
        assert_eq!(private_control_dir(&long), None);
        assert!(!long.exists());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_user_control_path_in_ssh_config_is_respected() {
        let dir = std::env::temp_dir().join(format!("rift-ssh-cm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        std::fs::write(
            &cfg,
            "Host shared\n  ControlMaster auto\n  ControlPath /tmp/cm-%C\n",
        )
        .unwrap();
        let profile = |host: &str| SshProfile {
            name: host.into(),
            host: host.into(),
            args: vec!["-F".into(), cfg.to_string_lossy().into_owned()],
        };
        if std::process::Command::new("ssh")
            .arg("-V")
            .output()
            .is_err()
        {
            eprintln!("BLOCKED: ssh is not installed");
            return;
        }
        assert!(profile("shared").shares_connections_itself());
        // control: a host the config gives no ControlPath gets Rift's
        assert!(!profile("other").shares_connections_itself());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn installer_is_guarded_and_idempotent() {
        let s = installer_script();
        assert!(s.starts_with("set -e\n"));
        assert_eq!(s.matches(RC_MARKER).count(), 5);
        assert!(s.contains("[ \"$LC_TERMINAL\" = Rift ]"));
        assert!(s.contains("__rift_precmd"), "zsh hook body embedded");
        // runs under a real sh without touching $HOME
        let home = std::env::temp_dir().join(format!("rift-ssh-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        for _ in 0..2 {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(&s)
                .env("HOME", &home)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let bashrc = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        assert_eq!(
            bashrc.matches(RC_MARKER).count(),
            1,
            "second run adds nothing"
        );
        assert!(home.join(".local/share/rift/rift.zsh").exists());
        // the servers `rift-complete` runs, byte for byte
        for (file, body) in [
            ("complete.zsh", crate::shell_complete::SERVER_ZSH),
            ("complete.bash", crate::shell_complete::SERVER_BASH),
            ("complete.fish", crate::shell_complete::SERVER_FISH),
        ] {
            let got = std::fs::read_to_string(home.join(".local/share/rift").join(file)).unwrap();
            assert_eq!(got.trim_end(), body.trim_end(), "{file}");
        }
        // an rc file without a trailing newline keeps its last line intact
        std::fs::write(home.join(".zshrc"), "export A=1").unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&s)
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(out.status.success());
        let zshrc = std::fs::read_to_string(home.join(".zshrc")).unwrap();
        assert!(zshrc.starts_with("export A=1\n["), "{zshrc:?}");
        std::fs::remove_dir_all(&home).ok();
    }
}
