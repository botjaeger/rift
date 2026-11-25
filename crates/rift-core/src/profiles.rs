//! Reusable launch profiles (`[[profiles]]` in config.toml) and the shells
//! installed on this machine (`/etc/shells`), which `generated.rs` writes
//! into config.toml as profiles named after their executable (`zsh`,
//! `bash`, `fish`, …).
//!
//! A pane's launch resolves in layers: the global settings (`shell`,
//! `working_directory`), then the pane's profile (or `default_profile`),
//! then what the pane itself names (a launch configuration's per-pane
//! `shell` and `args`, or its `directory`). Executables and arguments stay
//! structured (program + argv); nothing here builds a shell command string.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The shells file read for discovered profiles.
pub const SHELLS_FILE: &str = "/etc/shells";

/// The tab colours a profile may name (the tab menu's colours).
pub const TAB_COLOR_NAMES: [&str; 6] = crate::tab_color::NAMES;

/// One `[[profiles]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Shown in the palette and the + menu; unique.
    pub name: String,
    /// The program: an absolute path, `~/…`, or a name looked up on `PATH`.
    /// Absent: the global `shell`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments after Rift's own shell-integration arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Start directory (`~` expands); a pane's own directory wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Environment for this profile's panes only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Tab title of a tab this profile opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tab colour of a tab this profile opens (one of [`TAB_COLOR_NAMES`]
    /// or `#rrggbb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The glyph its menu row shows, by name (`python`, `docker`, …; see
    /// the app's `glyphs::by_name`); unset or unknown: a terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The colour scheme its panes' terminal content renders in (a theme
    /// name, as `theme`); the window's chrome keeps the window theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// What wrote the entry: [`SHELLS_SOURCE`] for a stub Rift generated
    /// from the shells file (`generated.rs`); absent: the user's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Kept in the file but offered nowhere (the palette, the + menu); a
    /// hidden profile still resolves by name (`default_profile`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// A generated profile whose shell is no longer installed (see
    /// `generated::mark_orphans`): kept in the file, offered nowhere.
    #[serde(skip)]
    pub orphaned: bool,
}

/// `source` of a profile generated from the shells file.
pub const SHELLS_SOURCE: &str = "shells";

impl Profile {
    /// A profile named `name` with every other field unset.
    pub fn named(name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            title: None,
            color: None,
            icon: None,
            theme: None,
            source: None,
            hidden: false,
            orphaned: false,
        }
    }

    fn shell(name: &str, path: &Path) -> Profile {
        Profile {
            command: Some(path.to_string_lossy().into_owned()),
            source: Some(SHELLS_SOURCE.to_string()),
            ..Profile::named(name)
        }
    }

    /// Generated from the shells file (and not since claimed by the user).
    pub fn is_generated(&self) -> bool {
        self.source.as_deref() == Some(SHELLS_SOURCE)
    }
}

/// `[[profiles]]` problems that reject config.toml like a parse error: an
/// empty or repeated name, an unknown colour, an empty command, or a value a
/// process cannot carry (NUL bytes, `=` in an environment name).
pub fn validate(profiles: &[Profile]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for (i, p) in profiles.iter().enumerate() {
        let name = p.name.trim();
        let label = if name.is_empty() {
            format!("profiles[{i}]")
        } else {
            format!("profile {name:?}")
        };
        if name.is_empty() {
            return Err(format!("{label}: name is empty"));
        }
        if !seen.insert(name.to_string()) {
            return Err(format!("{label}: the name is used twice"));
        }
        if p.command.as_deref().is_some_and(|c| c.trim().is_empty()) {
            return Err(format!("{label}: command is empty"));
        }
        if let Some(c) = &p.color
            && crate::tab_color::TabColor::parse(c).is_none()
        {
            return Err(format!(
                "{label}: color {c:?} is not one of {} or #rrggbb",
                TAB_COLOR_NAMES.join(", ")
            ));
        }
        let nul = |s: &str| s.contains('\0');
        let mut strings = p
            .command
            .iter()
            .chain(p.cwd.iter())
            .chain(p.args.iter())
            .chain(p.env.values());
        if strings.any(|s| nul(s)) {
            return Err(format!("{label}: a value holds a NUL byte"));
        }
        if let Some(k) = p
            .env
            .keys()
            .find(|k| k.is_empty() || k.contains('=') || nul(k))
        {
            return Err(format!("{label}: environment name {k:?} is invalid"));
        }
    }
    Ok(())
}

/// The executable shells listed in `shells_file`, one per basename: the one
/// `PATH` finds when several share a name, else the first listed. Each is a
/// profile named after its basename.
pub fn discover(shells_file: &Path, path_var: Option<&OsStr>) -> Vec<Profile> {
    // Windows has no shells file: the known shells and WSL distributions
    if cfg!(windows) {
        return windows_discover(&RealHost::new(path_var));
    }
    let Ok(text) = std::fs::read_to_string(shells_file) else {
        return Vec::new();
    };
    // basename → candidate paths, in first-listed order
    let mut groups: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for line in text.lines().take(512) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.starts_with('/') {
            continue;
        }
        let path = PathBuf::from(line);
        if !crate::shell_integration::is_executable(&path) {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, paths)) => {
                if !paths.contains(&path) {
                    paths.push(path)
                }
            }
            None => groups.push((name, vec![path])),
        }
    }
    groups
        .into_iter()
        .map(|(name, paths)| {
            let on_path = on_path(&name, path_var).and_then(|p| crate::sys::canonicalize(p).ok());
            let chosen = on_path
                .and_then(|want| {
                    paths
                        .iter()
                        .find(|p| crate::sys::canonicalize(p).ok().as_ref() == Some(&want))
                })
                .unwrap_or(&paths[0]);
            Profile::shell(&name, chosen)
        })
        .collect()
}

/// What discovery asks of a Windows machine, so the rules run against a
/// fake one in tests (and on any host).
pub trait WindowsHost {
    /// An environment variable (`ProgramFiles`, `SystemRoot`, `ComSpec`).
    fn var(&self, name: &str) -> Option<String>;
    /// Whether the file exists.
    fn exists(&self, path: &str) -> bool;
    /// `exe` (`pwsh.exe`) found through `PATH`.
    fn find_on_path(&self, exe: &str) -> Option<String>;
    /// The names of the directories in `dir`.
    fn subdirs(&self, dir: &str) -> Vec<String>;
    /// The raw output of `<wsl> -l -q`; `None` when it cannot run or fails.
    fn wsl_list(&self, wsl: &str) -> Option<Vec<u8>>;
}

/// This machine, for [`WindowsHost`].
pub struct RealHost<'a> {
    path_var: Option<&'a OsStr>,
}

impl<'a> RealHost<'a> {
    pub fn new(path_var: Option<&'a OsStr>) -> RealHost<'a> {
        RealHost { path_var }
    }
}

impl WindowsHost for RealHost<'_> {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.trim().is_empty())
    }

    fn exists(&self, path: &str) -> bool {
        // an app execution alias is a reparse point `metadata` may refuse
        let p = Path::new(path);
        p.exists() || std::fs::symlink_metadata(p).is_ok()
    }

    fn find_on_path(&self, exe: &str) -> Option<String> {
        std::env::split_paths(self.path_var?)
            .filter(|d| d.is_absolute())
            .map(|d| d.join(exe))
            .find(|p| self.exists(&p.to_string_lossy()))
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn subdirs(&self, dir: &str) -> Vec<String> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect()
    }

    fn wsl_list(&self, wsl: &str) -> Option<Vec<u8>> {
        // `wsl -l` does not start the VM, but a broken install can hang: a
        // thread and a deadline keep the start-up from waiting on it
        let wsl = wsl.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut cmd = std::process::Command::new(wsl);
            cmd.args(["-l", "-q"])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let _ = tx.send(cmd.output());
        });
        let out = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .ok()?
            .ok()?;
        out.status.success().then_some(out.stdout)
    }
}

fn win_join(dir: &str, name: &str) -> String {
    format!(r"{}\{name}", dir.trim_end_matches('\\'))
}

fn win_parent(path: &str) -> Option<&str> {
    path.rfind('\\')
        .map(|i| &path[..i])
        .filter(|p| !p.is_empty())
}

fn win_root(host: &dyn WindowsHost) -> Option<String> {
    host.var("SystemRoot").or_else(|| host.var("windir"))
}

/// PowerShell 7: `pwsh.exe` on `PATH` (a Store app-execution alias only when
/// no installed copy exists: the alias is not a file Rift can start), else
/// the newest `%ProgramFiles%\PowerShell\<version>\pwsh.exe`.
pub fn windows_pwsh7(host: &dyn WindowsHost) -> Option<String> {
    let on_path = host.find_on_path("pwsh.exe");
    let alias = |p: &str| p.to_ascii_lowercase().contains(r"\windowsapps\");
    if let Some(p) = on_path.as_deref().filter(|p| !alias(p)) {
        return Some(p.to_string());
    }
    let installed = host.var("ProgramFiles").and_then(|pf| {
        let root = win_join(&pf, "PowerShell");
        // a bigger number before a smaller one, `7` before `7-preview`
        let key = |d: &str| {
            let major: u32 = d
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            (major, !d.contains('-'))
        };
        let mut dirs = host.subdirs(&root);
        dirs.sort_by_key(|d| std::cmp::Reverse(key(d)));
        dirs.into_iter()
            .map(|d| win_join(&win_join(&root, &d), "pwsh.exe"))
            .find(|p| host.exists(p))
    });
    installed.or(on_path)
}

/// Windows PowerShell 5.1.
pub fn windows_ps5(host: &dyn WindowsHost) -> Option<String> {
    let p = win_join(
        &win_root(host)?,
        r"System32\WindowsPowerShell\v1.0\powershell.exe",
    );
    host.exists(&p).then_some(p)
}

/// The Command Prompt: `%ComSpec%`, else `%SystemRoot%\System32\cmd.exe`.
pub fn windows_cmd(host: &dyn WindowsHost) -> Option<String> {
    let comspec = host.var("ComSpec").filter(|c| host.exists(c));
    comspec.or_else(|| {
        let p = win_join(&win_root(host)?, r"System32\cmd.exe");
        host.exists(&p).then_some(p)
    })
}

/// Git for Windows' bash: `%ProgramFiles%\Git\bin\bash.exe`, else the
/// `..\bin\bash.exe` of the `git.exe` on `PATH` (`Git\cmd\git.exe`).
pub fn windows_git_bash(host: &dyn WindowsHost) -> Option<String> {
    let fixed = host
        .var("ProgramFiles")
        .map(|pf| win_join(&pf, r"Git\bin\bash.exe"))
        .filter(|p| host.exists(p));
    fixed.or_else(|| {
        let git = host.find_on_path("git.exe")?;
        let root = win_parent(win_parent(&git)?)?;
        let p = win_join(root, r"bin\bash.exe");
        host.exists(&p).then_some(p)
    })
}

/// The distribution names in the output of `wsl.exe -l -q`: UTF-16LE with a
/// possible byte order mark and CRLF line ends (UTF-8 when `WSL_UTF8` is
/// set), stray NULs tolerated. Skips Docker Desktop's own distributions, a
/// repeated name, and any line with whitespace in it (a distribution name
/// has none: such a line is a message, not a name).
pub fn parse_wsl_list(raw: &[u8]) -> Vec<String> {
    let raw = raw.strip_prefix(&[0xFF, 0xFE]).unwrap_or(raw);
    let text = if raw.contains(&0) {
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().take(256) {
        let name: String = line
            .chars()
            .filter(|&c| c != '\0' && c != '\u{feff}')
            .collect();
        let name = name.trim();
        let name = name.strip_suffix("(Default)").map_or(name, str::trim_end);
        if name.is_empty()
            || name.len() > 64
            || name.chars().any(|c| c.is_whitespace() || c.is_control())
            || name.to_ascii_lowercase().starts_with("docker-desktop")
            || out.iter().any(|n| n == name)
        {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

/// A generated profile running `program` with `args`.
fn windows_shell(name: &str, program: String, args: &[&str]) -> Profile {
    Profile {
        args: args.iter().map(|a| a.to_string()).collect(),
        ..Profile::shell(name, Path::new(&program))
    }
}

/// The shells of a Windows machine, the default first: PowerShell 7,
/// Windows PowerShell, Command Prompt, Git Bash, then each WSL
/// distribution. A profile's `command` is the program's path (its icon is
/// that file's).
pub fn windows_discover(host: &dyn WindowsHost) -> Vec<Profile> {
    let mut out = Vec::new();
    if let Some(p) = windows_pwsh7(host) {
        out.push(windows_shell("PowerShell 7", p, &[]));
    }
    if let Some(p) = windows_ps5(host) {
        out.push(windows_shell("Windows PowerShell", p, &[]));
    }
    if let Some(p) = windows_cmd(host) {
        out.push(windows_shell("Command Prompt", p, &[]));
    }
    if let Some(p) = windows_git_bash(host) {
        out.push(windows_shell("Git Bash", p, &[]));
    }
    let wsl = host.find_on_path("wsl.exe").or_else(|| {
        let p = win_join(&win_root(host)?, r"System32\wsl.exe");
        host.exists(&p).then_some(p)
    });
    if let Some(wsl) = wsl
        && let Some(raw) = host.wsl_list(&wsl)
    {
        for distro in parse_wsl_list(&raw) {
            out.push(windows_shell(
                &format!("{distro} (WSL)"),
                wsl.clone(),
                &["-d", &distro],
            ));
        }
    }
    out
}

/// The program a new pane runs when nothing names one: PowerShell 7, else
/// Windows PowerShell, else the Command Prompt.
pub fn windows_default(host: &dyn WindowsHost) -> Option<String> {
    windows_pwsh7(host)
        .or_else(|| windows_ps5(host))
        .or_else(|| windows_cmd(host))
}

/// The profiles to offer: config.toml's, minus the `hidden` and the
/// orphaned ones. Installed shells reach this list only as entries written
/// into the file (`generated::sync`).
pub fn effective(profiles: &[Profile]) -> Vec<Profile> {
    profiles
        .iter()
        .filter(|p| !p.hidden && !p.orphaned)
        .cloned()
        .collect()
}

/// What a pane was opened with, as a launch configuration, duplicate or the
/// session file records it: the profile's name (never its values, so editing
/// a profile never changes a running pane) and the pane's own overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneLaunch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The pane's own program (Warp Tab Config per-pane `shell`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// The pane's own arguments (replace the profile's).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// The pane's own colour scheme (a theme name; wins over the profile's).
    /// [`DEFAULT_PANE_THEME`] keeps the window theme even when the profile
    /// names one ("Pane theme: default").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

/// A pane's `theme` that means the window theme, over the profile's.
pub const DEFAULT_PANE_THEME: &str = "default";

impl PaneLaunch {
    pub fn is_default(&self) -> bool {
        *self == PaneLaunch::default()
    }

    pub fn profile(name: &str) -> PaneLaunch {
        PaneLaunch {
            profile: Some(name.to_string()),
            ..Default::default()
        }
    }
}

/// A pane's launch with every layer applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// The profile used (named, or `default_profile`).
    pub profile: Option<String>,
    /// The program's path; `None`: the global `shell` as before profiles.
    pub program: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// The profile's directory, `~` expanded (the pane's own one wins).
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    pub color: Option<String>,
    /// The pane's colour scheme (its own `theme`, else the profile's);
    /// `None`: the window theme.
    pub theme: Option<String>,
}

impl Resolved {
    /// What the pane records: the profile actually used and `launch`'s own
    /// overrides.
    pub fn identity(&self, launch: &PaneLaunch) -> PaneLaunch {
        PaneLaunch {
            profile: self.profile.clone(),
            shell: launch.shell.clone(),
            args: launch.args.clone(),
            theme: launch.theme.clone(),
        }
    }
}

/// Why a pane cannot open; nothing runs instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    UnknownProfile(String),
    /// The program is not an executable file (`command` as written).
    Missing {
        profile: Option<String>,
        command: String,
    },
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::UnknownProfile(n) => write!(f, "No profile named \"{n}\""),
            LaunchError::Missing {
                profile: Some(p),
                command,
            } => write!(f, "Profile \"{p}\": {command} is not an executable"),
            LaunchError::Missing {
                profile: None,
                command,
            } => write!(f, "Shell {command} is not an executable"),
        }
    }
}

/// Resolve `launch` against `profiles`. `default_profile` applies to a pane
/// that names neither a profile nor a shell.
pub fn resolve(
    profiles: &[Profile],
    default_profile: Option<&str>,
    launch: &PaneLaunch,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
) -> Result<Resolved, LaunchError> {
    let default = default_profile
        .map(str::trim)
        .filter(|n| !n.is_empty() && launch.shell.is_none());
    let name = launch
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .or(default);
    let profile = match name {
        Some(n) => Some(
            profiles
                .iter()
                .find(|p| p.name.trim() == n)
                .ok_or_else(|| LaunchError::UnknownProfile(n.to_string()))?,
        ),
        None => None,
    };
    // the pane's own program brings its own arguments; else the pane's
    // arguments replace the profile's
    let (command, args) = match &launch.shell {
        Some(s) => (Some(s.clone()), launch.args.clone()),
        None => (
            profile.and_then(|p| p.command.clone()),
            if launch.args.is_empty() {
                profile.map(|p| p.args.clone()).unwrap_or_default()
            } else {
                launch.args.clone()
            },
        ),
    };
    let program = match command {
        Some(c) => Some(
            find_program(&c, path_var, home).ok_or_else(|| LaunchError::Missing {
                profile: name.map(str::to_string),
                command: c.trim().to_string(),
            })?,
        ),
        None => None,
    };
    Ok(Resolved {
        profile: name.map(str::to_string),
        program,
        args,
        env: profile
            .map(|p| p.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default(),
        cwd: profile
            .and_then(|p| p.cwd.as_deref())
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| expand_tilde(c, home)),
        title: profile
            .and_then(|p| p.title.clone())
            .filter(|t| !t.is_empty()),
        color: profile.and_then(|p| p.color.as_deref().map(|c| c.trim().to_ascii_lowercase())),
        theme: launch
            .theme
            .as_deref()
            .or(profile.and_then(|p| p.theme.as_deref()))
            .map(str::trim)
            .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(DEFAULT_PANE_THEME))
            .map(str::to_string),
    })
}

/// `command` as an executable file: a path (absolute or `~/…`) as is, a bare
/// name through `PATH`. A relative path with a slash is never run.
pub fn find_program(
    command: &str,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
) -> Option<String> {
    let c = command.trim();
    if c.is_empty() {
        return None;
    }
    if c.starts_with('~') || c.contains('/') || (cfg!(windows) && c.contains('\\')) {
        let p = expand_tilde(c, home);
        return (p.is_absolute() && crate::shell_integration::is_executable(&p))
            .then(|| p.to_string_lossy().into_owned());
    }
    on_path(c, path_var).map(|p| p.to_string_lossy().into_owned())
}

fn on_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    // Windows: `pwsh` is `pwsh.exe` (any PATHEXT extension)
    let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| format!("{name}{}", e.to_ascii_lowercase()))
            .collect()
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(path_var?)
        .filter(|d| d.is_absolute())
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| crate::shell_integration::is_executable(p))
}

fn expand_tilde(path: &str, home: Option<&Path>) -> PathBuf {
    match home {
        Some(h) if path == "~" => h.to_path_buf(),
        Some(h) if path.starts_with("~/") => h.join(path[2..].trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// The program's basename (`zsh`, `fish`), for labels and shell kinds.
/// Windows: `C:\…\pwsh.exe` is `pwsh`.
pub fn basename(program: &str) -> &str {
    let name = program.rsplit('/').next().unwrap_or(program);
    if !cfg!(windows) {
        return name;
    }
    let name = name.rsplit('\\').next().unwrap_or(name);
    match name.len().checked_sub(4) {
        Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => &name[..i],
        _ => name,
    }
}

#[cfg(test)]
mod windows_discovery_tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A fake machine: variables, files, `PATH` hits, directories, the
    /// output of `wsl -l -q`.
    #[derive(Default)]
    struct Fake {
        vars: HashMap<&'static str, &'static str>,
        files: HashSet<&'static str>,
        path: HashMap<&'static str, &'static str>,
        dirs: Vec<(&'static str, Vec<&'static str>)>,
        wsl: Option<Vec<u8>>,
    }

    impl WindowsHost for Fake {
        fn var(&self, name: &str) -> Option<String> {
            self.vars.get(name).map(|v| v.to_string())
        }
        fn exists(&self, path: &str) -> bool {
            self.files.contains(path)
        }
        fn find_on_path(&self, exe: &str) -> Option<String> {
            self.path.get(exe).map(|v| v.to_string())
        }
        fn subdirs(&self, dir: &str) -> Vec<String> {
            self.dirs
                .iter()
                .find(|(d, _)| *d == dir)
                .map(|(_, v)| v.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default()
        }
        fn wsl_list(&self, _: &str) -> Option<Vec<u8>> {
            self.wsl.clone()
        }
    }

    const PS5: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
    const CMD: &str = r"C:\Windows\System32\cmd.exe";
    const WSL: &str = r"C:\Windows\System32\wsl.exe";

    fn win11() -> Fake {
        Fake {
            vars: HashMap::from([
                ("SystemRoot", r"C:\Windows"),
                ("ComSpec", CMD),
                ("ProgramFiles", r"C:\Program Files"),
            ]),
            files: HashSet::from([PS5, CMD]),
            ..Fake::default()
        }
    }

    fn utf16(s: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { vec![] };
        out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    fn names(p: &[Profile]) -> Vec<&str> {
        p.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn wsl_list_parses_utf16_with_bom_crlf_and_nuls() {
        let raw = utf16("Ubuntu\r\nDebian\r\n", true);
        assert_eq!(parse_wsl_list(&raw), ["Ubuntu", "Debian"]);
        // no BOM, no trailing line end
        assert_eq!(parse_wsl_list(&utf16("kali-linux", false)), ["kali-linux"]);
        // a stray NUL pair and a BOM in the middle of the text
        let raw = utf16("Ubuntu\r\n\0\u{feff}Arch\r\n", false);
        assert_eq!(parse_wsl_list(&raw), ["Ubuntu", "Arch"]);
        // UTF-8 (WSL_UTF8=1), `(Default)` of the long listing, a repeat
        assert_eq!(
            parse_wsl_list(b"Ubuntu (Default)\nDebian\nDebian\n"),
            ["Ubuntu", "Debian"]
        );
    }

    #[test]
    fn wsl_list_skips_docker_desktop_messages_and_empty_output() {
        let raw = utf16(
            "docker-desktop\r\nDocker-Desktop-data\r\nUbuntu-22.04\r\n",
            true,
        );
        assert_eq!(parse_wsl_list(&raw), ["Ubuntu-22.04"]);
        assert!(parse_wsl_list(b"").is_empty());
        assert!(parse_wsl_list(&utf16("\r\n\r\n", true)).is_empty());
        assert!(parse_wsl_list(&[0xFF, 0xFE]).is_empty());
        // a message on stdout is not a distribution
        let msg = utf16(
            "Windows Subsystem for Linux has no installed distributions.\r\n",
            true,
        );
        assert!(parse_wsl_list(&msg).is_empty());
        // an odd trailing byte does not panic
        assert_eq!(parse_wsl_list(&[b'A', 0, b'B']), ["A"]);
    }

    #[test]
    fn a_fresh_windows_11_offers_windows_powershell_and_the_command_prompt() {
        let found = windows_discover(&win11());
        assert_eq!(names(&found), ["Windows PowerShell", "Command Prompt"]);
        assert_eq!(found[0].command.as_deref(), Some(PS5));
        assert_eq!(found[1].command.as_deref(), Some(CMD));
        assert!(found.iter().all(Profile::is_generated));
        assert_eq!(windows_default(&win11()).as_deref(), Some(PS5));
    }

    #[test]
    fn a_dev_machine_lists_every_shell_with_powershell_7_first() {
        let pwsh = r"C:\Program Files\PowerShell\7\pwsh.exe";
        let bash = r"C:\Program Files\Git\bin\bash.exe";
        let mut m = win11();
        m.files.extend([pwsh, bash, WSL]);
        m.path.insert("pwsh.exe", pwsh);
        m.wsl = Some(utf16("Ubuntu\r\ndocker-desktop\r\nDebian\r\n", true));
        let found = windows_discover(&m);
        assert_eq!(
            names(&found),
            [
                "PowerShell 7",
                "Windows PowerShell",
                "Command Prompt",
                "Git Bash",
                "Ubuntu (WSL)",
                "Debian (WSL)"
            ]
        );
        assert_eq!(found[0].command.as_deref(), Some(pwsh));
        assert_eq!(found[3].command.as_deref(), Some(bash));
        assert_eq!(found[4].command.as_deref(), Some(WSL));
        assert_eq!(found[4].args, ["-d", "Ubuntu"]);
        assert_eq!(found[5].args, ["-d", "Debian"]);
        assert!(found[..4].iter().all(|p| p.args.is_empty()));
        assert_eq!(windows_default(&m).as_deref(), Some(pwsh));
    }

    #[test]
    fn powershell_7_off_path_is_the_newest_under_program_files() {
        let mut m = win11();
        m.dirs = vec![(
            r"C:\Program Files\PowerShell",
            vec!["6", "7", "7-preview", "10"],
        )];
        m.files.extend([
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files\PowerShell\6\pwsh.exe",
            r"C:\Program Files\PowerShell\7-preview\pwsh.exe",
        ]);
        // 10 has no pwsh.exe: the next newest
        assert_eq!(
            windows_pwsh7(&m).as_deref(),
            Some(r"C:\Program Files\PowerShell\7\pwsh.exe")
        );
        m.files.insert(r"C:\Program Files\PowerShell\10\pwsh.exe");
        assert_eq!(
            windows_pwsh7(&m).as_deref(),
            Some(r"C:\Program Files\PowerShell\10\pwsh.exe")
        );
    }

    #[test]
    fn a_store_alias_on_path_yields_to_an_installed_powershell_7() {
        let alias = r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\pwsh.exe";
        let real = r"C:\Program Files\PowerShell\7\pwsh.exe";
        let mut m = win11();
        m.path.insert("pwsh.exe", alias);
        m.files.insert(alias);
        // alone, the alias is still the best there is
        assert_eq!(windows_pwsh7(&m).as_deref(), Some(alias));
        m.dirs = vec![(r"C:\Program Files\PowerShell", vec!["7"])];
        m.files.insert(real);
        assert_eq!(windows_pwsh7(&m).as_deref(), Some(real));
    }

    #[test]
    fn git_bash_is_found_beside_git_on_path() {
        let mut m = win11();
        let bash = r"D:\tools\Git\bin\bash.exe";
        m.path.insert("git.exe", r"D:\tools\Git\cmd\git.exe");
        assert_eq!(windows_git_bash(&m), None);
        m.files.insert(bash);
        assert_eq!(windows_git_bash(&m).as_deref(), Some(bash));
    }

    #[test]
    fn the_command_prompt_falls_back_to_system32_without_comspec() {
        let mut m = win11();
        m.vars.remove("ComSpec");
        assert_eq!(windows_cmd(&m).as_deref(), Some(CMD));
        // a ComSpec that is not there is ignored
        m.vars.insert("ComSpec", r"Z:\nope\cmd.exe");
        assert_eq!(windows_cmd(&m).as_deref(), Some(CMD));
    }

    #[test]
    fn a_missing_or_failing_wsl_adds_nothing() {
        let mut m = win11();
        m.files.insert(WSL);
        m.wsl = None;
        assert_eq!(names(&windows_discover(&m)).len(), 2);
        // wsl.exe absent: its list is not even asked
        let mut m = win11();
        m.wsl = Some(utf16("Ubuntu\r\n", true));
        assert_eq!(names(&windows_discover(&m)).len(), 2);
    }

    #[test]
    fn no_shell_at_all_is_an_empty_list_and_no_default() {
        let m = Fake::default();
        assert!(windows_discover(&m).is_empty());
        assert_eq!(windows_default(&m), None);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    /// A bare name runs from PATH with a PATHEXT extension (`code` is
    /// `code.cmd`); a backslash path is a path; labels drop `.exe`.
    #[test]
    fn programs_resolve_with_pathext_and_backslash_paths() {
        let dir = std::env::temp_dir().join(format!("rift-pathext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tool.cmd"), "@echo off\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let path = std::env::join_paths([&dir]).unwrap();
        let found = find_program("tool", Some(&path), None).expect("tool.cmd");
        assert!(found.to_ascii_lowercase().ends_with("tool.cmd"), "{found}");
        // not a program: no PATHEXT extension
        assert_eq!(find_program("notes", Some(&path), None), None);
        let full = dir.join("tool.cmd").to_string_lossy().into_owned();
        assert_eq!(
            find_program(&full, None, None).as_deref(),
            Some(full.as_str())
        );
        assert_eq!(basename(r"C:\Program Files\PowerShell\7\pwsh.exe"), "pwsh");
        assert_eq!(basename(r"C:\Windows\System32\cmd.EXE"), "cmd");
        assert_eq!(basename("bash"), "bash");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rift-profiles-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An executable file at `dir/rel`.
    fn exe(dir: &Path, rel: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn profile(name: &str) -> Profile {
        Profile::named(name)
    }

    #[test]
    fn discovery_reads_the_shells_file_dedupes_and_prefers_path() {
        let d = scratch("discover");
        let bin_zsh = exe(&d, "bin/zsh");
        let usr_zsh = exe(&d, "usr/bin/zsh");
        let bash = exe(&d, "bin/bash");
        let not_exec = d.join("bin/csh");
        std::fs::write(&not_exec, "").unwrap();
        let shells = d.join("shells");
        std::fs::write(
            &shells,
            format!(
                "# comment\n\n{}\n{}\n{}\n{}\n{}\nrelative/fish\n/missing/fish\n",
                bin_zsh.display(),
                bash.display(),
                usr_zsh.display(),
                not_exec.display(),
                bash.display(),
            ),
        )
        .unwrap();
        // PATH finds usr/bin first: that zsh wins over the first listed
        let path = std::env::join_paths([d.join("usr/bin"), d.join("bin")]).unwrap();
        let got = discover(&shells, Some(&path));
        let names: Vec<(&str, &str)> = got
            .iter()
            .map(|p| (p.name.as_str(), p.command.as_deref().unwrap()))
            .collect();
        assert_eq!(
            names,
            [
                ("zsh", usr_zsh.to_str().unwrap()),
                ("bash", bash.to_str().unwrap())
            ]
        );
        assert!(got.iter().all(Profile::is_generated));
        // without PATH the first listed wins
        let got = discover(&shells, None);
        assert_eq!(got[0].command.as_deref(), bin_zsh.to_str());
        // no file: nothing
        assert!(discover(&d.join("absent"), None).is_empty());
    }

    #[test]
    fn effective_drops_hidden_and_orphaned_profiles() {
        let mut hidden = profile("hidden");
        hidden.hidden = true;
        let mut orphan = profile("orphan");
        orphan.orphaned = true;
        let all = [profile("a"), hidden, orphan, profile("b")];
        let names: Vec<String> = effective(&all).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["a", "b"]);
        // a hidden profile still resolves by name (default_profile)
        let r = resolve(&all, Some("hidden"), &PaneLaunch::default(), None, None);
        assert_eq!(r.unwrap().profile.as_deref(), Some("hidden"));
    }

    #[test]
    fn source_and_hidden_round_trip_and_stay_out_of_a_users_entry() {
        let src = "name = \"zsh\"\ncommand = \"/bin/zsh\"\nsource = \"shells\"\nhidden = true\n";
        let p: Profile = toml::from_str(src).unwrap();
        assert!(p.is_generated() && p.hidden && !p.orphaned);
        assert_eq!(toml::to_string(&p).unwrap(), src);
        let mine = toml::to_string(&profile("mine")).unwrap();
        assert_eq!(mine, "name = \"mine\"\n");
        assert!(validate(&[p]).is_ok());
    }

    #[test]
    fn precedence_global_then_profile_then_pane() {
        let d = scratch("precedence");
        let fish = exe(&d, "bin/fish");
        let bash = exe(&d, "bin/bash");
        let path = std::env::join_paths([d.join("bin")]).unwrap();
        let home = d.join("home");
        let mut dev = profile("dev");
        dev.command = Some("fish".into());
        dev.args = vec!["--private".into()];
        dev.cwd = Some("~/proj".into());
        dev.env.insert("X".into(), "1".into());
        dev.title = Some("Dev".into());
        dev.color = Some("Green".into());
        let ps = [dev];
        let r = |default: Option<&str>, l: PaneLaunch| {
            resolve(&ps, default, &l, Some(&path), Some(&home))
        };

        // nothing named, no default: the global settings (program None)
        assert_eq!(r(None, PaneLaunch::default()).unwrap(), Resolved::default());
        // the default profile applies to a pane naming nothing
        let got = r(Some("dev"), PaneLaunch::default()).unwrap();
        assert_eq!(got.profile.as_deref(), Some("dev"));
        assert_eq!(got.program.as_deref(), fish.to_str());
        assert_eq!(got.args, ["--private"]);
        assert_eq!(got.env, [("X".to_string(), "1".to_string())]);
        assert_eq!(got.cwd, Some(home.join("proj")));
        assert_eq!(got.title.as_deref(), Some("Dev"));
        assert_eq!(got.color.as_deref(), Some("green"));
        // pane args replace the profile's; its program stays
        let got = r(
            None,
            PaneLaunch {
                profile: Some("dev".into()),
                args: vec!["-c".into(), "a b".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got.program.as_deref(), fish.to_str());
        assert_eq!(got.args, ["-c", "a b"]);
        // a pane shell overrides the profile's program and drops its args,
        // and does not take the default profile
        let shell_only = PaneLaunch {
            shell: Some(bash.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let got = r(Some("dev"), shell_only.clone()).unwrap();
        assert_eq!(got.program.as_deref(), bash.to_str());
        assert!(got.args.is_empty() && got.env.is_empty() && got.profile.is_none());
        // the identity keeps the profile used and the pane's overrides
        let got = r(Some("dev"), PaneLaunch::default()).unwrap();
        assert_eq!(
            got.identity(&PaneLaunch::default()),
            PaneLaunch::profile("dev")
        );
    }

    #[test]
    fn a_panes_theme_wins_over_its_profiles_and_default_keeps_the_window() {
        let mut prod = profile("prod");
        prod.theme = Some("light-default".into());
        let ps = [prod, profile("plain")];
        let theme = |l: PaneLaunch| resolve(&ps, None, &l, None, None).unwrap().theme;
        assert_eq!(
            theme(PaneLaunch::profile("prod")).as_deref(),
            Some("light-default")
        );
        assert_eq!(theme(PaneLaunch::profile("plain")), None);
        let own = |profile: &str, t: &str| PaneLaunch {
            theme: Some(t.into()),
            ..PaneLaunch::profile(profile)
        };
        assert_eq!(
            theme(own("prod", "tokyo-night")).as_deref(),
            Some("tokyo-night")
        );
        assert_eq!(
            theme(own("plain", "tokyo-night")).as_deref(),
            Some("tokyo-night")
        );
        // "default" (any case) and blanks: the window theme
        assert_eq!(theme(own("prod", DEFAULT_PANE_THEME)), None);
        assert_eq!(theme(own("prod", "Default")), None);
        assert_eq!(theme(own("prod", " ")), None);
        // the pane keeps its own choice, never the profile's value
        let l = own("prod", "tokyo-night");
        let r = resolve(&ps, None, &l, None, None).unwrap();
        assert_eq!(r.identity(&l).theme.as_deref(), Some("tokyo-night"));
        let r = resolve(&ps, None, &PaneLaunch::profile("prod"), None, None).unwrap();
        assert_eq!(r.identity(&PaneLaunch::profile("prod")).theme, None);
        // it round-trips through config.toml
        let src = "name = \"prod\"\ntheme = \"light-default\"\n";
        let p: Profile = toml::from_str(src).unwrap();
        assert_eq!(p.theme.as_deref(), Some("light-default"));
    }

    #[test]
    fn unknown_profiles_and_missing_programs_are_errors_naming_both() {
        let d = scratch("errors");
        let path = std::env::join_paths([d.join("bin")]).unwrap();
        let mut gone = profile("gone");
        gone.command = Some("/nonexistent/fish".into());
        let mut rel = profile("rel");
        rel.command = Some("bin/fish".into());
        exe(&d, "bin/fish");
        let ps = [gone, rel];
        let r = |l: PaneLaunch| resolve(&ps, None, &l, Some(&path), None);
        let e = r(PaneLaunch::profile("nope")).unwrap_err();
        assert_eq!(e, LaunchError::UnknownProfile("nope".into()));
        assert_eq!(e.to_string(), "No profile named \"nope\"");
        let e = r(PaneLaunch::profile("gone")).unwrap_err();
        assert_eq!(
            e.to_string(),
            "Profile \"gone\": /nonexistent/fish is not an executable"
        );
        // a relative path is never run, even when it exists from the cwd
        assert!(r(PaneLaunch::profile("rel")).is_err());
        // an unknown default profile is an error too, never the global shell
        assert!(resolve(&ps, Some("nope"), &PaneLaunch::default(), None, None).is_err());
        let e = r(PaneLaunch {
            shell: Some("nosuchshell".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(e.to_string(), "Shell nosuchshell is not an executable");
    }

    #[test]
    fn validation_rejects_bad_entries() {
        assert!(validate(&[profile("a"), profile("b")]).is_ok());
        let bad = |p: Profile| validate(&[p]).unwrap_err();
        assert!(bad(profile(" ")).contains("name is empty"));
        assert!(
            validate(&[profile("a"), profile("a")])
                .unwrap_err()
                .contains("used twice")
        );
        let mut p = profile("c");
        p.color = Some("purple".into());
        assert!(bad(p).contains("color"));
        let mut p = profile("c");
        p.color = Some("Blue".into());
        assert!(validate(&[p]).is_ok());
        let mut p = profile("c");
        p.color = Some("#ff8800".into());
        assert!(validate(&[p]).is_ok(), "any hex colour");
        let mut p = profile("c");
        p.color = Some("#ff88".into());
        assert!(bad(p).contains("color"));
        let mut p = profile("e");
        p.command = Some("  ".into());
        assert!(bad(p).contains("command is empty"));
        let mut p = profile("n");
        p.args = vec!["a\0b".into()];
        assert!(bad(p).contains("NUL"));
        let mut p = profile("k");
        p.env.insert("A=B".into(), "x".into());
        assert!(bad(p).contains("environment name"));
    }
}
