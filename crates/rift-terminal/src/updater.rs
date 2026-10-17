//! The window side of an update (rift-core's `update.rs` holds the check
//! and the install): how this process was launched, the other Rift
//! processes that restart with it, and reopening their windows afterwards.
//!
//! One process holds every window it opened (⌘N), and its restart file
//! holds all of them. An update restarts every Rift process: the updating
//! one asks each other one (a separate launch, or a window of a version
//! that ran one process per window) over its control socket to save its
//! layout to `<data dir>/restart/<pid>.toml` (`main-<pid>.toml` for one
//! that owns `session.toml`) and quit (`PrepareRestart`), saves its own,
//! and quits leaving durable shells running. A detached helper starts the
//! new binary with `RIFT_RESTORE_SESSION` (its own file) and
//! `RIFT_RESTART_OTHERS`; that process reopens every window of every
//! remaining file, in-process.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::control::{self, TabInfo};

/// Restore this window from that session file (then delete it).
pub const RESTORE_ENV: &str = "RIFT_RESTORE_SESSION";
/// Reopen the other windows' files in `restart/` after starting.
pub const OTHERS_ENV: &str = "RIFT_RESTART_OTHERS";
/// Started by the update helper: finish (or report) the update.
pub const RELAUNCH_ENV: &str = "RIFT_UPDATE_RELAUNCH";
/// Test hook: the relaunched process runs this script (`--script`).
pub const RELAUNCH_SCRIPT_ENV: &str = "RIFT_RELAUNCH_SCRIPT";

/// How this process was started (read once, then removed from the
/// environment so shells and later windows do not inherit it).
#[derive(Debug, Default, Clone)]
pub struct Launch {
    pub restore: Option<PathBuf>,
    pub restart_others: bool,
    pub relaunched: bool,
    /// Command-line arguments (without the program).
    pub args: Vec<String>,
}

static LAUNCH: OnceLock<Launch> = OnceLock::new();

/// Read and clear the restart variables. Call first thing in `main`,
/// before any thread starts.
pub fn take_env() {
    let restart_dir = rift_core::session_state::restart_dir();
    let launch = Launch {
        // only a restart file (it is read, then deleted)
        restore: std::env::var_os(RESTORE_ENV)
            .map(PathBuf::from)
            .filter(|f| {
                restart_dir
                    .as_deref()
                    .is_some_and(|d| is_restart_file(f, d))
            }),
        restart_others: std::env::var_os(OTHERS_ENV).is_some(),
        relaunched: std::env::var_os(RELAUNCH_ENV).is_some(),
        args: std::env::args().skip(1).collect(),
    };
    for k in [RESTORE_ENV, OTHERS_ENV, RELAUNCH_ENV] {
        // SAFETY: called at the top of main, before any other thread exists
        unsafe { std::env::remove_var(k) };
    }
    let _ = LAUNCH.set(launch);
}

pub fn launch() -> &'static Launch {
    LAUNCH.get_or_init(Launch::default)
}

/// Other windows found for an update.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Survey {
    /// Reachable windows: (pid, control socket, tabs).
    pub windows: Vec<(u32, PathBuf, Vec<TabInfo>)>,
    /// Windows seen only through the session service (their control socket
    /// is off or did not answer): they cannot be restarted.
    pub unreachable: usize,
}

impl Survey {
    pub fn tabs(&self) -> usize {
        self.windows.iter().map(|w| w.2.len()).sum()
    }

    pub fn shells(&self) -> usize {
        self.windows
            .iter()
            .flat_map(|w| &w.2)
            .map(|t| t.panes.len())
            .sum()
    }

    pub fn durable_shells(&self) -> usize {
        self.windows
            .iter()
            .flat_map(|w| &w.2)
            .flat_map(|t| &t.panes)
            .filter(|p| p.durable)
            .count()
    }
}

/// Every other window that answers `list` within `deadline`, and how many
/// more hold shells in the session service (blocking: run off the UI
/// thread).
pub fn survey(deadline: Duration) -> Survey {
    let me = std::process::id();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut asked = 0;
    for path in control::sockets() {
        let Some(pid) = pid_of(&path).filter(|p| *p != me) else {
            continue;
        };
        asked += 1;
        let tx = tx.clone();
        std::thread::spawn(move || {
            let r = control::request(&path, &control::Request::List).ok();
            let _ = tx.send((pid, path, r));
        });
    }
    drop(tx);
    let end = Instant::now() + deadline;
    let mut out = Survey::default();
    for _ in 0..asked {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((pid, path, Some(r))) if r.ok => out.windows.push((pid, path, r.tabs)),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    // windows attached to the session service that did not answer
    let reached: Vec<u32> = out.windows.iter().map(|w| w.0).chain([me]).collect();
    let mut silent: Vec<u32> = rift_pty::service::list()
        .into_iter()
        .flat_map(|s| s.clients)
        .filter(|pid| !reached.contains(pid))
        .collect();
    silent.sort_unstable();
    silent.dedup();
    out.unreachable = silent.len();
    out.windows.sort_by_key(|w| w.0);
    out
}

fn pid_of(socket: &Path) -> Option<u32> {
    socket.file_stem()?.to_str()?.parse().ok()
}

/// `pid` still runs. A window this process started (⌘N, a reopened
/// window) stays a zombie until reaped: reap it here, and count it gone.
fn alive(pid: u32) -> bool {
    let Ok(p) = i32::try_from(pid) else {
        return false;
    };
    let mut status = 0;
    match unsafe { libc::waitpid(p, &mut status, libc::WNOHANG) } {
        r if r == p => false, // our child, exited: reaped now
        0 => true,            // our child, running
        _ => rift_core::update::pid_alive(pid),
    }
}

/// Ask each window to save its layout and quit (with this update's
/// `token`), then wait (up to `deadline`) for them to exit. Returns how
/// many confirmed.
pub fn restart_others(windows: &[(u32, PathBuf)], token: &str, deadline: Duration) -> usize {
    let handles: Vec<_> = windows
        .iter()
        .cloned()
        .map(|(pid, path)| {
            let request = control::Request::PrepareRestart {
                token: token.to_string(),
            };
            std::thread::spawn(move || {
                control::request(&path, &request)
                    .is_ok_and(|r| r.ok)
                    .then_some(pid)
            })
        })
        .collect();
    let confirmed: Vec<u32> = handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect();
    let end = Instant::now() + deadline;
    while confirmed.iter().any(|p| alive(*p)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    confirmed.len()
}

/// The restart file of this window: `main-<pid>.toml` for a window that
/// owns `session.toml`, else `<pid>.toml`.
pub fn own_restart_file(owns_session: bool) -> Option<PathBuf> {
    let dir = rift_core::session_state::restart_dir()?;
    Some(dir.join(restart_file_name(owns_session, std::process::id())))
}

fn restart_file_name(owns_session: bool, pid: u32) -> String {
    if owns_session {
        format!("main-{pid}.toml")
    } else {
        format!("{pid}.toml")
    }
}

/// `file` is a restart file directly in `dir`: `<pid>.toml` or
/// `main-<pid>.toml` (paths from the environment are trusted no further).
pub fn is_restart_file(file: &Path, dir: &Path) -> bool {
    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let pid = name
        .strip_suffix(".toml")
        .map(|s| s.strip_prefix("main-").unwrap_or(s));
    file.parent() == Some(dir)
        && pid.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The windows the restart files in `dir` hold, other than `own` (this
/// process's own file, restored by `RIFT_RESTORE_SESSION`): every window
/// of each file, oldest file first. Each file is deleted once read; one
/// that does not parse is left for a look.
fn take_restart_files(
    dir: &Path,
    own: Option<&Path>,
) -> Vec<rift_core::session_state::SavedWindowTabs> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_restart_file(p, dir) && Some(p.as_path()) != own)
        .collect();
    files.sort();
    let mut windows = Vec::new();
    for file in files {
        match rift_core::session_state::load_from(&file) {
            Some(saved) => {
                windows.extend(saved.windows().into_iter().filter(|w| !w.tabs.is_empty()));
                let _ = std::fs::remove_file(&file);
            }
            None => log::warn!("restart file {} does not parse", file.display()),
        }
    }
    windows
}

/// The windows to reopen in this process after a restart: what every
/// restart file other than `own` holds (`take_restart_files`).
pub fn restart_windows(own: Option<&Path>) -> Vec<rift_core::session_state::SavedWindowTabs> {
    match rift_core::session_state::restart_dir() {
        Some(dir) => take_restart_files(&dir, own),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::PaneInfo;

    fn pane(durable: bool) -> PaneInfo {
        PaneInfo {
            id: 1,
            active: true,
            cwd: None,
            job: None,
            last_command: None,
            last_exit: None,
            durable,
            git: None,
            chips: None,
            waiting: None,
            running: None,
        }
    }

    #[test]
    fn survey_counts_tabs_and_shells() {
        let tab = |panes| TabInfo {
            index: 0,
            window: 0,
            title: String::new(),
            active: false,
            panes,
        };
        let s = Survey {
            windows: vec![
                (1, "a".into(), vec![tab(vec![pane(true), pane(false)])]),
                (
                    2,
                    "b".into(),
                    vec![tab(vec![pane(true)]), tab(vec![pane(true)])],
                ),
            ],
            unreachable: 1,
        };
        assert_eq!((s.tabs(), s.shells(), s.durable_shells()), (3, 4, 3));
        assert_eq!(pid_of(Path::new("/x/123.sock")), Some(123));
        assert_eq!(pid_of(Path::new("/x/abc.sock")), None);
    }

    #[test]
    fn restart_files_reopen_every_window_here() {
        use rift_core::session_state::{SavedSession, SavedTab, SavedWindowTabs};
        assert_eq!(restart_file_name(true, 7), "main-7.toml");
        assert_eq!(restart_file_name(false, 7), "7.toml");
        let dir = std::env::temp_dir().join(format!("rift-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let window = |title: &str| SavedWindowTabs {
            tabs: vec![SavedTab {
                title: Some(title.into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        // a version-3 file of a process per window, and a version-4 file
        // with two windows, both other processes'
        std::fs::write(dir.join("12.toml"), "[[tabs]]\ntitle = \"old\"\n").unwrap();
        let two = SavedSession::of_windows(vec![window("a"), window("b")]);
        rift_core::session_state::save_to(&dir.join("main-5.toml"), &two).unwrap();
        // this process's own file stays (RIFT_RESTORE_SESSION restores it)
        let own = dir.join("9.toml");
        rift_core::session_state::save_to(&own, &SavedSession::of_windows(vec![window("own")]))
            .unwrap();
        std::fs::write(dir.join("notes.txt"), "not a restart file").unwrap();
        std::fs::write(dir.join("13.toml"), "tabs = [").unwrap();
        let titles: Vec<String> = take_restart_files(&dir, Some(&own))
            .iter()
            .flat_map(|w| w.tabs.iter().filter_map(|t| t.title.clone()))
            .collect();
        assert_eq!(titles, ["old", "a", "b"]);
        assert!(own.exists() && dir.join("notes.txt").exists());
        // read files go; one that does not parse stays for a look
        assert!(!dir.join("12.toml").exists() && !dir.join("main-5.toml").exists());
        assert!(dir.join("13.toml").exists());
        assert!(take_restart_files(&dir, Some(&own)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_restart_files_are_trusted() {
        let dir = Path::new("/d/restart");
        assert!(is_restart_file(Path::new("/d/restart/12.toml"), dir));
        assert!(is_restart_file(Path::new("/d/restart/main-12.toml"), dir));
        for bad in [
            "/d/restart/x.toml",
            "/d/restart/main-.toml",
            "/d/restart/12.txt",
            "/d/session.toml",
            "/d/restart/sub/12.toml",
            "/etc/12.toml",
        ] {
            assert!(!is_restart_file(Path::new(bad), dir), "{bad}");
        }
    }

    #[test]
    fn exited_children_are_not_waited_on() {
        // a window this process started and that exited is a zombie until
        // reaped: `alive` reaps it instead of waiting out the deadline
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!alive(pid));
        // already reaped by `alive`
        assert!(child.wait().is_err());
        let mut running = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        assert!(alive(running.id()));
        let _ = running.kill();
        let _ = running.wait();
    }
}
