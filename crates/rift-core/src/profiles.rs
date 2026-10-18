//! Reusable launch profiles (`[[profiles]]` in config.toml) and the shells
//! installed on this machine (`/etc/shells`), which appear as implicit
//! profiles named after their executable (`zsh`, `bash`, `fish`, …).
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
    /// Found in the shells file rather than written by the user.
    #[serde(skip)]
    pub discovered: bool,
}

impl Profile {
    fn shell(name: &str, path: &Path) -> Profile {
        Profile {
            name: name.to_string(),
            command: Some(path.to_string_lossy().into_owned()),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            title: None,
            color: None,
            icon: None,
            theme: None,
            discovered: true,
        }
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
            let on_path = on_path(&name, path_var).and_then(|p| std::fs::canonicalize(p).ok());
            let chosen = on_path
                .and_then(|want| {
                    paths
                        .iter()
                        .find(|p| std::fs::canonicalize(p).ok().as_ref() == Some(&want))
                })
                .unwrap_or(&paths[0]);
            Profile::shell(&name, chosen)
        })
        .collect()
}

/// The profiles to offer: the user's, then each discovered shell whose name
/// no user profile takes (none with `discover_shells = false`).
pub fn effective(user: &[Profile], discover_shells: bool) -> Vec<Profile> {
    let mut out = user.to_vec();
    if discover_shells {
        let path = std::env::var_os("PATH");
        for p in discover(Path::new(SHELLS_FILE), path.as_deref()) {
            if !out.iter().any(|u| u.name.trim() == p.name) {
                out.push(p);
            }
        }
    }
    out
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
    if c.starts_with('~') || c.contains('/') {
        let p = expand_tilde(c, home);
        return (p.is_absolute() && crate::shell_integration::is_executable(&p))
            .then(|| p.to_string_lossy().into_owned());
    }
    on_path(c, path_var).map(|p| p.to_string_lossy().into_owned())
}

fn on_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
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
pub fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

#[cfg(test)]
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
        Profile {
            name: name.into(),
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            title: None,
            color: None,
            icon: None,
            theme: None,
            discovered: false,
        }
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
        assert!(got.iter().all(|p| p.discovered));
        // without PATH the first listed wins
        let got = discover(&shells, None);
        assert_eq!(got[0].command.as_deref(), bin_zsh.to_str());
        // no file: nothing
        assert!(discover(&d.join("absent"), None).is_empty());
    }

    #[test]
    fn effective_lets_a_user_profile_shadow_a_discovered_one() {
        let mut mine = profile("zsh");
        mine.args = vec!["-f".into()];
        let all = effective(std::slice::from_ref(&mine), true);
        assert_eq!(all.iter().filter(|p| p.name == "zsh").count(), 1);
        assert_eq!(all[0], mine);
        assert_eq!(effective(std::slice::from_ref(&mine), false), vec![mine]);
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
