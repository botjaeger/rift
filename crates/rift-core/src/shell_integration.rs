use std::io::Write;
use std::path::PathBuf;

pub const RIFT_ZSH: &str = include_str!("../../../shell/rift.zsh");
pub const RIFT_BASH: &str = include_str!("../../../shell/rift.bash");
pub const RIFT_FISH: &str = include_str!("../../../shell/rift.fish");
const BASHRC: &str = include_str!("../../../shell/rift-bashrc");
const BASHRC_CHILD: &str = include_str!("../../../shell/rift-bashrc-child");
const ZSHENV: &str = include_str!("../../../shell/zdotdir/.zshenv");
const ZPROFILE: &str = include_str!("../../../shell/zdotdir/.zprofile");
const ZSHRC: &str = include_str!("../../../shell/zdotdir/.zshrc");
const ZLOGIN: &str = include_str!("../../../shell/zdotdir/.zlogin");

/// Materialize the shell-integration files under the user's data dir and
/// return env vars to set on the spawned shell. Returns an empty list if the
/// files can't be written (integration silently off, terminal still works).
pub fn setup() -> Vec<(String, String)> {
    match write_files() {
        Ok(env) => env,
        Err(e) => {
            log::warn!("shell integration disabled: {e}");
            Vec::new()
        }
    }
}

fn write_files() -> std::io::Result<Vec<(String, String)>> {
    let base = data_dir().ok_or_else(|| std::io::Error::other("no data dir"))?;
    let zdotdir = base.join("zdotdir");
    std::fs::create_dir_all(&zdotdir)?;

    let integration = base.join("rift.zsh");
    write_if_changed(&integration, RIFT_ZSH)?;
    // bash: no ZDOTDIR equivalent — the shell is spawned with
    // `--rcfile <bashrc shim>` (see `bash_rcfile`), which replays the user's
    // own startup files and then sources rift.bash
    let bash_integration = base.join("rift.bash");
    write_if_changed(&bash_integration, RIFT_BASH)?;
    write_if_changed(&base.join("bashrc"), BASHRC)?;
    write_if_changed(&base.join("bashrc-child"), BASHRC_CHILD)?;
    // fish auto-loads vendor_conf.d from XDG_DATA_DIRS (appended at spawn)
    let fish_dir = base.join("fish/vendor_conf.d");
    std::fs::create_dir_all(&fish_dir)?;
    write_if_changed(&fish_dir.join("rift.fish"), RIFT_FISH)?;
    write_if_changed(&zdotdir.join(".zshenv"), ZSHENV)?;
    write_if_changed(&zdotdir.join(".zprofile"), ZPROFILE)?;
    write_if_changed(&zdotdir.join(".zshrc"), ZSHRC)?;
    write_if_changed(&zdotdir.join(".zlogin"), ZLOGIN)?;
    // hooks written under the app's earlier name, in a data dir kept from
    // then: fish would auto-load the old file next to rift.fish
    for stale in ["shift.zsh", "shift.bash", "fish/vendor_conf.d/shift.fish"] {
        match std::fs::remove_file(base.join(stale)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                log::warn!("could not remove {stale}: {e}")
            }
            _ => {}
        }
    }

    let mut env = vec![
        (
            "RIFT_INTEGRATION_ZSH".into(),
            integration.to_string_lossy().into_owned(),
        ),
        (
            "RIFT_INTEGRATION_BASH".into(),
            bash_integration.to_string_lossy().into_owned(),
        ),
        ("ZDOTDIR".into(), zdotdir.to_string_lossy().into_owned()),
    ];
    if let Ok(orig) = std::env::var("ZDOTDIR") {
        env.push(("RIFT_ORIG_ZDOTDIR".into(), orig));
    }
    // fish vendor_conf.d injection + terminal identification
    let xdg =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    env.push((
        "XDG_DATA_DIRS".into(),
        format!("{xdg}:{}", base.to_string_lossy()),
    ));
    env.push(("TERM_PROGRAM".into(), "rift".into()));
    // ours, not the launching terminal's (a Rift started from Warp passed
    // Warp's version on, and tools pair it with TERM_PROGRAM)
    env.push((
        "TERM_PROGRAM_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    ));
    // survives ssh (SendEnv LC_*): remote hooks installed by Rift load
    // only when this arrives (see ssh.rs)
    env.push(("LC_TERMINAL".into(), "Rift".into()));
    env.push((
        "LC_TERMINAL_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    ));
    Ok(env)
}

/// Extra argv for `program` so its integration loads automatically: bash
/// gets `--rcfile <shim>` (the shim emulates a macOS login shell); zsh
/// (ZDOTDIR) and fish (vendor_conf.d) need none for integration, but on macOS
/// they get `-l`: terminals there run login shells, and users keep PATH setup
/// (`brew shellenv`, pyenv) in `.zprofile` / `status is-login` blocks.
pub fn shell_args(program: &str) -> Vec<String> {
    let name = program.rsplit('/').next().unwrap_or(program);
    if name == "bash"
        && let Some(base) = data_dir()
    {
        return vec![
            "--rcfile".into(),
            base.join("bashrc").to_string_lossy().into_owned(),
        ];
    }
    if cfg!(target_os = "macos") && matches!(name, "zsh" | "fish") {
        return vec!["-l".into()];
    }
    Vec::new()
}

pub(crate) fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    path.is_file()
}

/// The shell Rift will spawn: config override, else `$SHELL`, else zsh.
pub fn resolve_shell(configured: Option<&str>) -> String {
    // an executable regular file: spawning anything else aborts the app
    let exists = |s: &String| !s.trim().is_empty() && is_executable(std::path::Path::new(s.trim()));
    if let Some(c) = configured.map(str::to_string) {
        if exists(&c) {
            return c.trim().to_string();
        }
        log::warn!("shell {c:?} is not an executable file, falling back");
    }
    // $SHELL, else the first installed of the usual shells: a spawn with a
    // missing binary would abort the app (e.g. minimal Linux without zsh)
    std::env::var("SHELL")
        .ok()
        .filter(exists)
        .or_else(|| {
            ["/bin/zsh", "/usr/bin/zsh", "/bin/bash", "/usr/bin/bash"]
                .iter()
                .map(|s| s.to_string())
                .find(exists)
        })
        .unwrap_or_else(|| "/bin/sh".into())
}

/// Basename of the shell (`zsh`, `bash`, `fish`) for labels.
pub fn shell_name(configured: Option<&str>) -> String {
    let shell = resolve_shell(configured);
    shell.rsplit('/').next().unwrap_or(&shell).to_string()
}

/// `$XDG_DATA_HOME/rift`, else `~/.local/share/rift`.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|root| crate::config::app_dir(&root))
}

fn write_if_changed(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path)
        .map(|c| c == content)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_gets_rcfile_zsh_fish_login_on_macos() {
        if data_dir().is_none() {
            return;
        }
        let args = shell_args("/bin/bash");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "--rcfile");
        assert_eq!(
            std::path::Path::new(&args[1]),
            data_dir().unwrap().join("bashrc")
        );
        let login: Vec<String> = if cfg!(target_os = "macos") {
            vec!["-l".into()]
        } else {
            Vec::new()
        };
        assert_eq!(shell_args("/bin/zsh"), login);
        assert_eq!(shell_args("/opt/homebrew/bin/fish"), login);
        assert!(shell_args("/bin/sh").is_empty());
    }

    /// Runs `script` in `shell` with the hook at `$1`; (stdout, stderr), or
    /// `None` when that shell is not installed.
    fn run_hook(shell: &str, args: &[&str], hook: &str, script: &str) -> Option<(String, String)> {
        let dir =
            std::env::temp_dir().join(format!("rift-hook-{}-{}", std::process::id(), hook.len()));
        std::fs::create_dir_all(&dir).ok()?;
        let file = dir.join("hook");
        std::fs::write(&file, hook).ok()?;
        let out = std::process::Command::new(shell)
            .args(args)
            .arg(script)
            .arg(shell)
            .arg(&file)
            .env_remove("RIFT_INTEGRATION_BASH")
            .env_remove("RIFT_INTEGRATION_ZSH")
            .env_remove("VIRTUAL_ENV")
            .output();
        let _ = std::fs::remove_dir_all(&dir);
        let out = out.ok()?;
        Some((
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }

    /// A user config with `set -u` / `setopt nounset` must not break the
    /// hooks: an unset variable aborts the prompt hook, and with it the A
    /// and D markers (the editor never takes over).
    #[test]
    fn hooks_survive_nounset() {
        let bash = "set -u; PS1='x'; source \"$1\"; __rift_prompt_command; __rift_prompt_end; \
                    __rift_cmd_ran=1; false; __rift_prompt_command; __rift_prompt_end; echo END";
        for shell in ["/bin/bash", "bash"] {
            let Some((out, err)) =
                run_hook(shell, &["--noprofile", "--norc", "-c"], RIFT_BASH, bash)
            else {
                continue;
            };
            assert!(err.is_empty(), "{shell}: {err}");
            assert!(out.contains("\x1b]133;D;1\x07"), "{shell}: {out:?}");
            assert!(out.ends_with("END\n"), "{shell}: {out:?}");
        }
        let zsh = "setopt nounset; PS1='x'; source \"$1\"; __rift_precmd; __rift_precmd_end; \
                   __rift_preexec x; false; __rift_precmd; echo END";
        if let Some((out, err)) = run_hook("zsh", &["-f", "-c"], RIFT_ZSH, zsh) {
            assert!(err.is_empty(), "zsh: {err}");
            assert!(out.contains("\x1b]133;D;1\x07"), "zsh: {out:?}");
            assert!(out.ends_with("END\n"), "zsh: {out:?}");
        }
    }

    #[test]
    fn shell_name_prefers_config() {
        assert_eq!(shell_name(Some("/bin/sh")), "sh");
        assert_eq!(resolve_shell(Some("/bin/sh")), "/bin/sh");
        assert!(!shell_name(None).is_empty());
    }

    #[test]
    fn missing_shell_falls_back_to_an_installed_one() {
        let got = resolve_shell(Some("/definitely/not/a/shell"));
        assert_ne!(got, "/definitely/not/a/shell");
        assert!(std::path::Path::new(&got).exists());
        // existing but not executable (a file, a directory): same fallback
        for bad in ["/etc/hosts", "/tmp"] {
            let got = resolve_shell(Some(bad));
            assert_ne!(got, bad);
            assert!(is_executable(std::path::Path::new(&got)), "{got}");
        }
    }
}
