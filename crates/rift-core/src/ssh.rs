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
    let mut out = Vec::new();
    collect(path, 0, &mut out);
    let mut seen = std::collections::HashSet::new();
    out.retain(|h| seen.insert(h.clone()));
    out
}

fn collect(path: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
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
                for file in glob_simple(&p) {
                    collect(&file, depth + 1, out);
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

impl SshProfile {
    /// The command line typed into a new tab to connect.
    pub fn command(&self) -> String {
        let mut parts = vec!["ssh".to_string(), "-o".into(), "SendEnv=LC_TERMINAL".into()];
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
