//! Installed shells written into config.toml as `[[profiles]]` entries, so
//! the file is the single source of truth for what the palette and the +
//! menu offer (the Windows Terminal model).
//!
//! At startup [`sync`] reads the shells file (`/etc/shells`) and appends a
//! stub (`name`, `command`, `source = "shells"`) for each executable shell
//! Rift has not offered before. Every shell offered is remembered in the
//! seen list (`<data dir>/generated-profiles`, one command path per line),
//! so a stub the user deletes stays deleted and an edited one keeps its
//! edits. A stub whose shell was uninstalled stays in the file but is
//! marked orphaned ([`mark_orphans`]) and offered nowhere.

use crate::Config;
use crate::profiles::{self, Profile};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The seen list's file name in the data directory.
pub const SEEN_FILE: &str = "generated-profiles";

/// The commands already offered as generated profiles. A missing file is
/// an empty list.
pub fn load_seen(path: &Path) -> std::io::Result<BTreeSet<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e),
    }
}

/// Replace the seen list atomically.
pub fn save_seen(path: &Path, seen: &BTreeSet<String>) -> std::io::Result<()> {
    let body: String = seen.iter().map(|c| format!("{c}\n")).collect();
    crate::config::write_atomic(path, &body)
}

/// `config` with a stub appended for each shell in `shells_file` not in
/// `seen` and not already run by one of its profiles; each such shell is
/// added to `seen`. On the `first_run` (no config file existed) with no
/// `default_profile`, the profile running `login_shell` becomes the
/// default. `None`: nothing changed (the file must not be written); always
/// so with `discover_shells = false`, which leaves `seen` alone too.
pub fn generate(
    config: &Config,
    shells_file: &Path,
    path_var: Option<&OsStr>,
    seen: &mut BTreeSet<String>,
    login_shell: Option<&str>,
    first_run: bool,
) -> Option<Config> {
    if !config.discover_shells {
        return None;
    }
    let mut out = config.clone();
    let mut changed = false;
    for shell in profiles::discover(shells_file, path_var) {
        let Some(command) = shell.command.clone() else {
            continue;
        };
        if !seen.insert(command.clone()) {
            continue;
        }
        if out.profiles.iter().any(|p| {
            p.command
                .as_deref()
                .is_some_and(|c| same_program(c, &command, path_var))
        }) {
            continue;
        }
        let name = free_name(&out.profiles, &shell.name);
        out.profiles.push(Profile { name, ..shell });
        changed = true;
    }
    if first_run
        && out.default_profile.is_none()
        && let Some(login) = login_shell
        && let Some(p) = out.profiles.iter().find(|p| {
            p.command
                .as_deref()
                .is_some_and(|c| same_program(c, login, path_var))
        })
    {
        out.default_profile = Some(p.name.clone());
        changed = true;
    }
    changed.then_some(out)
}

/// Mark each generated profile whose command is no longer an executable
/// listed in `shells_file` as orphaned (and clear the mark on the rest).
pub fn mark_orphans(profiles: &mut [Profile], shells_file: &Path, path_var: Option<&OsStr>) {
    let listed: Vec<PathBuf> = std::fs::read_to_string(shells_file)
        .map(|text| {
            text.lines()
                .take(512)
                .map(str::trim)
                .filter(|l| l.starts_with('/'))
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    let canonical: Vec<PathBuf> = listed
        .iter()
        .filter_map(|p| crate::sys::canonicalize(p).ok())
        .collect();
    for p in profiles.iter_mut() {
        p.orphaned = p.is_generated()
            && p.command.as_deref().is_some_and(|c| {
                let Some(found) = profiles::find_program(c, path_var, home().as_deref()) else {
                    return true;
                };
                let found = PathBuf::from(found);
                let canon = crate::sys::canonicalize(&found).ok();
                !listed.contains(&found) && !canon.is_some_and(|c| canonical.contains(&c))
            });
    }
}

/// Bring the config file at `config_path` up to date with the shells
/// installed (`generate`), writing it only when it changed, and persist the
/// seen list at `seen_path`. Returns the config written, if any. A file
/// that does not load is never touched (its error is shown elsewhere).
pub fn sync(
    config_path: &Path,
    seen_path: &Path,
    shells_file: &Path,
    path_var: Option<&OsStr>,
    login_shell: Option<&str>,
) -> std::io::Result<Option<Config>> {
    let first_run = match std::fs::symlink_metadata(config_path) {
        Ok(_) => false,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => return Err(e),
    };
    let config = match Config::try_load_from(config_path) {
        Ok(c) => c.unwrap_or_default(),
        Err(e) => {
            log::warn!("installed shells not added to config.toml: {e}");
            return Ok(None);
        }
    };
    if !config.discover_shells {
        return Ok(None);
    }
    let mut seen = load_seen(seen_path)?;
    let before = seen.clone();
    let new = generate(
        &config,
        shells_file,
        path_var,
        &mut seen,
        login_shell,
        first_run,
    );
    // the file first: should it fail, the shells are offered again next time
    if let Some(c) = &new {
        c.save_to(config_path)?;
    }
    if seen != before {
        save_seen(seen_path, &seen)?;
    }
    Ok(new)
}

/// [`sync`] for this user: config.toml, the seen list in the data
/// directory, `shells_file` (`/etc/shells` outside tests), `$PATH` and the
/// login shell `$SHELL`.
pub fn sync_default(shells_file: &Path) -> std::io::Result<Option<Config>> {
    let (Some(config), Some(data)) = (
        crate::config::config_path(),
        crate::shell_integration::data_dir(),
    ) else {
        return Ok(None);
    };
    let path = std::env::var_os("PATH");
    let login = std::env::var("SHELL").ok();
    sync(
        &config,
        &data.join(SEEN_FILE),
        shells_file,
        path.as_deref(),
        login.as_deref(),
    )
}

/// [`mark_orphans`] against `shells_file` and `$PATH`.
pub fn mark_orphans_default(profiles: &mut [Profile], shells_file: &Path) {
    let path = std::env::var_os("PATH");
    mark_orphans(profiles, shells_file, path.as_deref());
}

fn home() -> Option<PathBuf> {
    crate::sys::home_os().map(PathBuf::from)
}

/// Whether `command` (as a profile writes it) runs the executable `path`:
/// the same text, or the same file once found and symlinks are resolved.
fn same_program(command: &str, path: &str, path_var: Option<&OsStr>) -> bool {
    if command.trim() == path.trim() {
        return true;
    }
    let canon = |c: &str| {
        profiles::find_program(c, path_var, home().as_deref())
            .and_then(|p| crate::sys::canonicalize(p).ok())
    };
    matches!((canon(command), canon(path)), (Some(a), Some(b)) if a == b)
}

/// `base`, or `base (2)`, `base (3)`, … when a profile already has it.
fn free_name(profiles: &[Profile], base: &str) -> String {
    let taken = |n: &str| profiles.iter().any(|p| p.name.trim() == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !taken(n))
        .expect("a free name")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rift-generated-{tag}-{}", std::process::id()));
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

    /// A machine: a shells file listing `shells` (created as executables
    /// under `bin/`), a config path and a seen list.
    struct Fixture {
        dir: PathBuf,
        shells: PathBuf,
        config: PathBuf,
        seen: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str, shells: &[&str]) -> Fixture {
            let dir = scratch(tag);
            let f = Fixture {
                shells: dir.join("shells"),
                config: dir.join("cfg/config.toml"),
                seen: dir.join("data/generated-profiles"),
                dir,
            };
            f.install(shells);
            f
        }

        fn bin(&self, name: &str) -> String {
            self.dir
                .join("bin")
                .join(name)
                .to_string_lossy()
                .into_owned()
        }

        /// The shells file lists exactly `shells`, each installed.
        fn install(&self, shells: &[&str]) {
            let mut text = String::from("# shells\n");
            for s in shells {
                exe(&self.dir, &format!("bin/{s}"));
                text.push_str(&format!("{}\n", self.bin(s)));
            }
            std::fs::write(&self.shells, text).unwrap();
        }

        fn sync(&self) -> Option<Config> {
            let login = self.bin("zsh");
            sync(&self.config, &self.seen, &self.shells, None, Some(&login)).unwrap()
        }

        fn text(&self) -> String {
            std::fs::read_to_string(&self.config).unwrap()
        }

        fn load(&self) -> Config {
            Config::try_load_from(&self.config).unwrap().unwrap()
        }

        fn names(&self) -> Vec<String> {
            self.load().profiles.into_iter().map(|p| p.name).collect()
        }
    }

    #[test]
    fn first_run_writes_one_stub_per_shell_and_defaults_to_the_login_shell() {
        let f = Fixture::new("first", &["bash", "zsh"]);
        let c = f.sync().expect("a new file");
        assert_eq!(c.default_profile.as_deref(), Some("zsh"));
        let text = f.text();
        assert_eq!(
            text,
            format!(
                "default_profile = \"zsh\"\n\n[[profiles]]\nname = \"bash\"\ncommand = \"{}\"\nsource = \"shells\"\n\n[[profiles]]\nname = \"zsh\"\ncommand = \"{}\"\nsource = \"shells\"\n",
                f.bin("bash"),
                f.bin("zsh")
            )
        );
        assert_eq!(f.load(), c);
        // a file Rift created needs no backup
        assert!(!f.dir.join("cfg/config.toml.bak").exists());
        let seen = load_seen(&f.seen).unwrap();
        assert_eq!(seen, BTreeSet::from([f.bin("bash"), f.bin("zsh")]));
        // the second run changes nothing, not one byte
        assert!(f.sync().is_none());
        assert_eq!(f.text(), text);
    }

    #[test]
    fn a_new_shell_is_appended_and_a_deleted_stub_stays_gone() {
        let f = Fixture::new("appended", &["zsh"]);
        f.sync().unwrap();
        f.install(&["zsh", "fish"]);
        let c = f.sync().expect("fish added");
        assert_eq!(f.names(), ["zsh", "fish"]);
        // not a first run: the default stays as it was
        assert_eq!(c.default_profile.as_deref(), Some("zsh"));
        // the user deletes the fish stub: it is not brought back
        let text = f.text();
        let cut = text.rfind("[[profiles]]").unwrap();
        std::fs::write(&f.config, &text[..cut]).unwrap();
        assert!(f.sync().is_none());
        assert_eq!(f.names(), ["zsh"]);
        // a rewrite of a file that existed keeps the original once
        let bak = f.dir.join("cfg/config.toml.bak");
        assert_eq!(
            std::fs::read_to_string(&bak)
                .unwrap()
                .matches("[[profiles]]")
                .count(),
            1
        );
    }

    #[test]
    fn a_users_profile_running_the_shell_takes_its_place() {
        let f = Fixture::new("user-runs", &["zsh", "bash"]);
        std::fs::create_dir_all(f.config.parent().unwrap()).unwrap();
        // the user's own "bash" runs zsh: no zsh stub, and the bash stub
        // takes the next free name
        std::fs::write(
            &f.config,
            format!(
                "[[profiles]]\nname = \"bash\"\ncommand = \"{}\"\n",
                f.bin("zsh")
            ),
        )
        .unwrap();
        f.sync().unwrap();
        let c = f.load();
        let got: Vec<(&str, &str)> = c
            .profiles
            .iter()
            .map(|p| (p.name.as_str(), p.command.as_deref().unwrap()))
            .collect();
        assert_eq!(
            got,
            [
                ("bash", f.bin("zsh").as_str()),
                ("bash (2)", f.bin("bash").as_str())
            ]
        );
        // an existing file is no first run: no default is chosen
        assert_eq!(c.default_profile, None);
        // the shell the user's profile covers is still seen
        assert!(load_seen(&f.seen).unwrap().contains(&f.bin("zsh")));
    }

    #[test]
    fn an_edited_stub_keeps_its_edit() {
        let f = Fixture::new("edited", &["zsh"]);
        f.sync().unwrap();
        let edited = f.text().replace(
            "source = \"shells\"\n",
            "source = \"shells\"\ntheme = \"tokyo-night\" # mine\n",
        );
        std::fs::write(&f.config, &edited).unwrap();
        assert!(f.sync().is_none());
        assert_eq!(f.text(), edited);
        assert_eq!(f.load().profiles[0].theme.as_deref(), Some("tokyo-night"));
    }

    #[test]
    fn an_uninstalled_shell_is_orphaned_kept_and_not_offered() {
        let f = Fixture::new("orphan", &["zsh", "fish"]);
        f.sync().unwrap();
        // fish leaves the shells file; zsh stays listed but its file is gone
        // in the second step
        f.install(&["zsh"]);
        let mut c = f.load();
        mark_orphans(&mut c.profiles, &f.shells, None);
        let orphaned: Vec<(&str, bool)> = c
            .profiles
            .iter()
            .map(|p| (p.name.as_str(), p.orphaned))
            .collect();
        assert_eq!(orphaned, [("zsh", false), ("fish", true)]);
        let offered: Vec<String> = profiles::effective(&c.profiles)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(offered, ["zsh"]);
        // it stays in the file
        assert!(f.sync().is_none());
        assert_eq!(f.names(), ["zsh", "fish"]);
        std::fs::remove_file(f.bin("zsh")).unwrap();
        mark_orphans(&mut c.profiles, &f.shells, None);
        assert!(c.profiles[0].orphaned, "a listed shell that is gone");
        // a user's own profile is never an orphan
        let mut mine = Profile::named("mine");
        mine.command = Some("/nonexistent/sh".into());
        let mut ps = [mine];
        mark_orphans(&mut ps, &f.shells, None);
        assert!(!ps[0].orphaned);
    }

    #[test]
    fn discover_shells_off_adds_nothing_and_touches_nothing() {
        let f = Fixture::new("off", &["zsh"]);
        std::fs::create_dir_all(f.config.parent().unwrap()).unwrap();
        std::fs::write(&f.config, "discover_shells = false\n").unwrap();
        assert!(f.sync().is_none());
        assert_eq!(f.text(), "discover_shells = false\n");
        assert!(!f.seen.exists());
        // and no file is created where there was none
        let mut seen = BTreeSet::new();
        let off = Config {
            discover_shells: false,
            ..Config::default()
        };
        assert!(generate(&off, &f.shells, None, &mut seen, None, true).is_none());
        assert!(seen.is_empty());
    }

    #[test]
    fn a_file_that_does_not_load_is_never_rewritten() {
        let f = Fixture::new("invalid", &["zsh"]);
        std::fs::create_dir_all(f.config.parent().unwrap()).unwrap();
        std::fs::write(&f.config, "font_size = [\n").unwrap();
        assert!(f.sync().is_none());
        assert_eq!(f.text(), "font_size = [\n");
        assert!(!f.seen.exists());
    }

    #[test]
    fn the_backup_is_written_once() {
        let f = Fixture::new("bak", &["zsh"]);
        std::fs::create_dir_all(f.config.parent().unwrap()).unwrap();
        std::fs::write(&f.config, "# mine\nfont_size = 15.0\n").unwrap();
        f.sync().unwrap();
        let bak = f.dir.join("cfg/config.toml.bak");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "# mine\nfont_size = 15.0\n"
        );
        assert!(
            f.text().starts_with("# mine\nfont_size = 15.0\n"),
            "{}",
            f.text()
        );
        // a later rewrite leaves the first backup alone
        f.install(&["zsh", "fish"]);
        f.sync().unwrap();
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "# mine\nfont_size = 15.0\n"
        );
    }

    #[test]
    fn the_seen_list_round_trips() {
        let d = scratch("seen");
        let path = d.join("data/generated-profiles");
        assert!(load_seen(&path).unwrap().is_empty());
        let set = BTreeSet::from(["/bin/zsh".to_string(), "/bin/bash".to_string()]);
        save_seen(&path, &set).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "/bin/bash\n/bin/zsh\n"
        );
        assert_eq!(load_seen(&path).unwrap(), set);
    }
}
