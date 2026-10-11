use std::io::Write;
use std::path::PathBuf;

pub const SHIFT_ZSH: &str = include_str!("../../../shell/shift.zsh");
pub const SHIFT_BASH: &str = include_str!("../../../shell/shift.bash");
pub const SHIFT_FISH: &str = include_str!("../../../shell/shift.fish");
const BASHRC: &str = include_str!("../../../shell/shift-bashrc");
const BASHRC_CHILD: &str = include_str!("../../../shell/shift-bashrc-child");
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

    let integration = base.join("shift.zsh");
    write_if_changed(&integration, SHIFT_ZSH)?;
    // bash: no ZDOTDIR equivalent — the shell is spawned with
    // `--rcfile <bashrc shim>` (see `bash_rcfile`), which replays the user's
    // own startup files and then sources shift.bash
    let bash_integration = base.join("shift.bash");
    write_if_changed(&bash_integration, SHIFT_BASH)?;
    write_if_changed(&base.join("bashrc"), BASHRC)?;
    write_if_changed(&base.join("bashrc-child"), BASHRC_CHILD)?;
    // fish auto-loads vendor_conf.d from XDG_DATA_DIRS (appended at spawn)
    let fish_dir = base.join("fish/vendor_conf.d");
    std::fs::create_dir_all(&fish_dir)?;
    write_if_changed(&fish_dir.join("shift.fish"), SHIFT_FISH)?;
    write_if_changed(&zdotdir.join(".zshenv"), ZSHENV)?;
    write_if_changed(&zdotdir.join(".zprofile"), ZPROFILE)?;
    write_if_changed(&zdotdir.join(".zshrc"), ZSHRC)?;
    write_if_changed(&zdotdir.join(".zlogin"), ZLOGIN)?;

    let mut env = vec![
        (
            "SHIFT_INTEGRATION_ZSH".into(),
            integration.to_string_lossy().into_owned(),
        ),
        (
            "SHIFT_INTEGRATION_BASH".into(),
            bash_integration.to_string_lossy().into_owned(),
        ),
        ("ZDOTDIR".into(), zdotdir.to_string_lossy().into_owned()),
    ];
    if let Ok(orig) = std::env::var("ZDOTDIR") {
        env.push(("SHIFT_ORIG_ZDOTDIR".into(), orig));
    }
    // fish vendor_conf.d injection + terminal identification
    let xdg =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    env.push((
        "XDG_DATA_DIRS".into(),
        format!("{xdg}:{}", base.to_string_lossy()),
    ));
    env.push(("TERM_PROGRAM".into(), "shift".into()));
    // ours, not the launching terminal's (a Shift started from Warp passed
    // Warp's version on, and tools pair it with TERM_PROGRAM)
    env.push((
        "TERM_PROGRAM_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    ));
    // survives ssh (SendEnv LC_*): remote hooks installed by Shift load
    // only when this arrives (see ssh.rs)
    env.push(("LC_TERMINAL".into(), "Shift".into()));
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

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    path.is_file()
}

/// The shell Shift will spawn: config override, else `$SHELL`, else zsh.
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

/// `$XDG_DATA_HOME/shift`, else `~/.local/share/shift`.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|root| {
            let current = root.join("shift");
            let legacy = root.join("runway");
            if !current.exists() && legacy.exists() {
                legacy
            } else {
                current
            }
        })
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
