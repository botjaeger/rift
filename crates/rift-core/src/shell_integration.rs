use std::io::Write;
use std::path::PathBuf;

pub const RIFT_ZSH: &str = include_str!("../../../shell/rift.zsh");
pub const RIFT_BASH: &str = include_str!("../../../shell/rift.bash");
pub const RIFT_FISH: &str = include_str!("../../../shell/rift.fish");
pub const RIFT_PS1: &str = include_str!("../../../shell/rift.ps1");
pub const RIFT_CMD: &str = include_str!("../../../shell/rift.cmd");
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
    // PowerShell: dot-sourced by the `-Command` `shell_args` passes
    write_if_changed(&base.join("rift.ps1"), RIFT_PS1)?;
    // Command Prompt: run by the `/k call` `shell_args` passes (CRLF: cmd
    // misreads labels in a batch file with bare LF)
    write_if_changed(&base.join("rift.cmd"), &batch_text(RIFT_CMD))?;
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
    let name = crate::profiles::basename(program);
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
    // PowerShell runs the user's profile, then the integration (which wraps
    // the prompt the profile set), and stays interactive. The integration
    // runs as text, not as a script file: Windows' default execution policy
    // (Restricted on Windows PowerShell) refuses to load a .ps1 file.
    if (name.eq_ignore_ascii_case("pwsh") || name.eq_ignore_ascii_case("powershell"))
        && let Some(base) = data_dir()
    {
        return vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            powershell_load(&base.join("rift.ps1")),
        ];
    }
    // the Command Prompt runs the integration from its PROMPT, which `call`
    // sets before the first prompt (the user's AutoRun and PROMPT first)
    if name.eq_ignore_ascii_case("cmd")
        && let Some(base) = data_dir()
    {
        return cmd_args(&base.join("rift.cmd"));
    }
    Vec::new()
}

/// `cmd.exe`'s arguments that stay interactive (`/k`) after running the
/// integration script. `call` leads so the quoted path is never taken for
/// the whole command line (cmd strips quotes around a lone program path).
fn cmd_args(script: &std::path::Path) -> Vec<String> {
    vec![
        "/k".into(),
        "call".into(),
        script.to_string_lossy().into_owned(),
    ]
}

/// A batch file's text with CRLF line ends, whatever the source has.
fn batch_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The PowerShell command that runs `script` in the session's scope from its
/// text (read as UTF-8), which no execution policy blocks; quotes in the path
/// doubled.
fn powershell_load(script: &std::path::Path) -> String {
    let path = script.to_string_lossy().replace('\'', "''");
    format!(". ([ScriptBlock]::Create([IO.File]::ReadAllText('{path}')))")
}

pub(crate) fn is_executable(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && crate::sys::is_executable(path, &m))
}

/// The shell Rift will spawn: config override, else `$SHELL`, else zsh.
pub fn resolve_shell(configured: Option<&str>) -> String {
    // an executable regular file: spawning anything else aborts the app
    let exists = |s: &String| !s.trim().is_empty() && is_executable(std::path::Path::new(s.trim()));
    if let Some(c) = configured {
        // a path as is, `~/…` and a bare name (`fish`) as Settings saves them
        let path = std::env::var_os("PATH");
        let home = crate::sys::home_os().map(PathBuf::from);
        if let Some(found) = crate::profiles::find_program(c, path.as_deref(), home.as_deref()) {
            return found;
        }
        log::warn!("shell {c:?} is not an executable file, falling back");
    }
    // Windows: PowerShell 7, else Windows PowerShell, else the command
    // prompt (%ComSpec%)
    #[cfg(windows)]
    {
        let path = std::env::var_os("PATH");
        let host = crate::profiles::RealHost::new(path.as_deref());
        // pwsh on PATH, else installed under %ProgramFiles%, else Windows
        // PowerShell (the first profile `profiles::discover` lists)
        if let Some(found) = crate::profiles::find_program("pwsh.exe", path.as_deref(), None)
            .or_else(|| crate::profiles::windows_pwsh7(&host))
            .or_else(|| crate::profiles::find_program("powershell.exe", path.as_deref(), None))
        {
            return found;
        }
        std::env::var("ComSpec")
            .ok()
            .filter(exists)
            .unwrap_or_else(|| r"C:\Windows\System32\cmd.exe".into())
    }
    // $SHELL, else the first installed of the usual shells: a spawn with a
    // missing binary would abort the app (e.g. minimal Linux without zsh)
    #[cfg(unix)]
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
    crate::profiles::basename(&shell).to_string()
}

/// `$XDG_DATA_HOME/rift`, else `~/.local/share/rift`.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::sys::home_os().map(|h| PathBuf::from(h).join(".local/share")))
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
        // session_state's tests point XDG_DATA_HOME elsewhere meanwhile
        let _env = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        // PowerShell runs the integration's text (no execution policy blocks
        // it), quotes in the path doubled
        let ps = shell_args("/usr/local/bin/pwsh");
        let script = data_dir().unwrap().join("rift.ps1");
        assert_eq!(
            ps,
            [
                "-NoLogo".to_string(),
                "-NoExit".into(),
                "-Command".into(),
                format!(
                    ". ([ScriptBlock]::Create([IO.File]::ReadAllText('{}')))",
                    script.to_string_lossy().replace('\'', "''")
                ),
            ]
        );
        assert_eq!(
            powershell_load(std::path::Path::new(r"C:\Users\o'neil\rift.ps1")),
            r". ([ScriptBlock]::Create([IO.File]::ReadAllText('C:\Users\o''neil\rift.ps1')))"
        );
    }

    /// The Command Prompt runs the integration with `/k call <script>`; the
    /// script is batch text (CRLF); its PROMPT, as cmd expands it, makes
    /// Rift's scanner see D, the cwd, A, the user's prompt, then B.
    #[test]
    fn command_prompt_carries_the_block_markers() {
        use crate::osc::{Marker, OscScanner};
        let _env = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(base) = data_dir() {
            assert_eq!(
                shell_args("/x/CMD"),
                ["/k", "call", &*base.join("rift.cmd").to_string_lossy()]
            );
        }
        assert!(
            cmd_args(std::path::Path::new(r"C:\a b\rift.cmd"))
                .ends_with(&[r"C:\a b\rift.cmd".to_string()])
        );
        assert_eq!(batch_text("a\nb\r\nc\n"), "a\r\nb\r\nc\r\n");

        // the PROMPT line: the user's text sits between A and B; unset, cmd's
        // default `$P$G`
        let line = RIFT_CMD
            .lines()
            .find(|l| l.starts_with("set \"PROMPT="))
            .expect("a PROMPT line");
        assert!(RIFT_CMD.contains("if not defined RIFT_PROMPT set \"RIFT_PROMPT=$P$G\""));
        let prompt = line
            .strip_prefix("set \"PROMPT=")
            .and_then(|l| l.strip_suffix('"'))
            .unwrap()
            .replace("%RIFT_PROMPT%", "$P$G");
        // cmd's expansion: $e is ESC, $P the path, $G ">"
        let shown = prompt
            .replace("$e", "\x1b")
            .replace("$P", r"C:\Users\ann\my dir")
            .replace("$G", ">");
        let mut scanner = OscScanner::new();
        let found: Vec<Marker> = scanner
            .scan(shown.as_bytes())
            .into_iter()
            .filter_map(|s| s.marker)
            .collect();
        assert_eq!(
            found,
            [
                Marker::Done { exit: None },
                Marker::Cwd {
                    path: r"C:\Users\ann\my dir".into(),
                    host: String::new()
                },
                Marker::PromptStart,
                Marker::CommandStart,
            ]
        );
        // a user's own prompt text stays whole between A and B
        assert!(line.find("]133;A").unwrap() < line.find("%RIFT_PROMPT%").unwrap());
        assert!(line.find("%RIFT_PROMPT%").unwrap() < line.find("]133;B").unwrap());
        // `rem` lines hold nothing cmd could take for a redirection or pipe
        for l in RIFT_CMD.lines().filter(|l| l.starts_with("rem")) {
            assert!(!l.contains(['<', '>', '|', '&']), "{l}");
        }
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

    /// PowerShell's prompt, wrapped by rift.ps1: no D before the first
    /// prompt, then D with the failed native command's code, the cwd as a
    /// percent-encoded file URL, and A/B around the user's own prompt.
    /// Reported and skipped where PowerShell is not installed.
    #[test]
    fn powershell_prompt_carries_the_block_markers() {
        let dir = std::env::temp_dir().join(format!("rift-ps1 a%b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hook = dir.join("rift.ps1");
        std::fs::write(&hook, RIFT_PS1).unwrap();
        let script = format!(
            "function prompt {{ 'MINE> ' }}; . '{}'; Set-Location -LiteralPath '{}'; \
             $first = prompt; $global:LASTEXITCODE = 3; \
             Write-Error x -ErrorAction SilentlyContinue; $second = prompt; \
             [Console]::Out.Write($first + '|' + $second)",
            hook.to_string_lossy().replace('\'', "''"),
            dir.to_string_lossy().replace('\'', "''"),
        );
        let run = |sh: &str, command: &str| {
            std::process::Command::new(sh)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    command,
                ])
                .output()
                .ok()
        };
        // a real PowerShell names its version (Wine's powershell.exe is a stub
        // that prints nothing)
        let real = |sh: &&&str| {
            run(sh, "$PSVersionTable.PSVersion.Major").is_some_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u32>()
                    .is_ok_and(|v| v >= 5)
            })
        };
        let out = ["pwsh", "powershell"]
            .iter()
            .find(real)
            .and_then(|sh| run(sh, &script));
        let _ = std::fs::remove_dir_all(&dir);
        let Some(out) = out else {
            eprintln!("BLOCKED: PowerShell is not installed");
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let (first, second) = text.split_once('|').unwrap_or_else(|| panic!("{text:?}"));
        let (a, b) = ("\x1b]133;A\x07", "\x1b]133;B\x07");
        assert!(!first.contains("133;D"), "{first:?}");
        assert!(first.contains(&format!("{a}MINE> {b}")), "{first:?}");
        assert!(second.starts_with("\x1b]133;D;3\x07"), "{second:?}");
        // the folder's last part, `%` and the blank encoded
        let leaf = dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('%', "%25")
            .replace(' ', "%20");
        let osc7 = second.split("\x1b]7;").nth(1).unwrap_or_default();
        assert!(osc7.starts_with("file://"), "{second:?}");
        assert!(osc7.contains(&format!("/{leaf}\x07")), "{second:?}");
        assert!(second.ends_with(&format!("{a}MINE> {b}")), "{second:?}");
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

    /// The system rc sets HISTFILE from ZDOTDIR (macOS /etc/zshrc), which is
    /// the shim in a Rift pane: the history still goes to the user's file,
    /// lines an earlier version left in the shim's file move there once, and
    /// an rc that names its own HISTFILE keeps it.
    #[test]
    fn zsh_history_goes_to_the_users_file_not_the_shim() {
        if std::process::Command::new("zsh")
            .arg("-fc")
            .arg("true")
            .status()
            .is_err()
        {
            eprintln!("BLOCKED: zsh is not installed");
            return;
        }
        let root = std::env::temp_dir().join(format!("rift-histfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (shim, home) = (root.join("shim"), root.join("home"));
        std::fs::create_dir_all(&shim).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(shim.join(".zshenv"), ZSHENV).unwrap();
        std::fs::write(shim.join(".zshrc"), ZSHRC).unwrap();
        // as the system rc does, from the user's .zshenv (sourced while
        // ZDOTDIR is still the shim) so it holds where /etc/zshrc does not
        std::fs::write(home.join(".zshenv"), "HISTFILE=${ZDOTDIR}/.zsh_history\n").unwrap();
        std::fs::write(shim.join(".zsh_history"), "stranded cmd\n").unwrap();
        std::fs::write(home.join(".zsh_history"), "earlier cmd\n").unwrap();
        let histfile = |rc: &str| {
            std::fs::write(home.join(".zshrc"), rc).unwrap();
            let out = std::process::Command::new("zsh")
                .args(["-i", "-c", "print -r -- $HISTFILE"])
                .env("ZDOTDIR", &shim)
                .env("HOME", &home)
                .env_remove("RIFT_ORIG_ZDOTDIR")
                .env_remove("RIFT_INTEGRATION_ZSH")
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let users = home.join(".zsh_history");
        assert_eq!(histfile(""), users.to_string_lossy());
        assert_eq!(
            std::fs::read_to_string(&users).unwrap(),
            "earlier cmd\nstranded cmd\n"
        );
        assert!(!shim.join(".zsh_history").exists(), "moved once");
        // the user's own choice wins
        assert_eq!(
            histfile("HISTFILE=$HOME/mine\n"),
            home.join("mine").to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Windows: PowerShell 7, Windows PowerShell or cmd by default, named
    /// without the folder or `.exe`; a configured bare name runs from PATH.
    #[test]
    #[cfg(windows)]
    fn windows_shells_resolve_through_path_and_pathext() {
        let shell = resolve_shell(None);
        assert!(shell.to_ascii_lowercase().ends_with(".exe"), "{shell}");
        assert!(
            ["pwsh", "powershell", "cmd"].contains(&shell_name(None).to_ascii_lowercase().as_str()),
            "{shell}"
        );
        let cmd = resolve_shell(Some("cmd"));
        assert!(cmd.to_ascii_lowercase().ends_with(r"\cmd.exe"), "{cmd}");
        assert_eq!(shell_name(Some("cmd")).to_ascii_lowercase(), "cmd");
    }

    #[test]
    #[cfg(unix)]
    fn shell_name_prefers_config() {
        assert_eq!(shell_name(Some("/bin/sh")), "sh");
        assert_eq!(resolve_shell(Some("/bin/sh")), "/bin/sh");
        assert!(!shell_name(None).is_empty());
        // a bare name, as a hand-written config.toml has it, runs from PATH
        let sh = resolve_shell(Some("sh"));
        assert!(sh.starts_with('/') && sh.ends_with("/sh"), "{sh}");
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
