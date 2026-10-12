//! The window side of an update (shift-core's `update.rs` holds the check
//! and the install): how this process was launched, the other windows that
//! restart with it, and reopening them afterwards.
//!
//! An update restarts every window: the updating window asks each other
//! window over its control socket to save its layout to
//! `<data dir>/restart/<pid>.toml` (`main-<pid>.toml` for the window that
//! owns `session.toml`) and quit (`PrepareRestart`), saves its own, and
//! quits leaving durable
//! shells running. A detached helper starts the new binary with
//! `SHIFT_RESTORE_SESSION` (its own file) and `SHIFT_RESTART_OTHERS`; that
//! process reopens one window per remaining file.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::control::{self, TabInfo};

/// Restore this window from that session file (then delete it).
pub const RESTORE_ENV: &str = "SHIFT_RESTORE_SESSION";
/// Reopen the other windows' files in `restart/` after starting.
pub const OTHERS_ENV: &str = "SHIFT_RESTART_OTHERS";
/// Started by the update helper: finish (or report) the update.
pub const RELAUNCH_ENV: &str = "SHIFT_UPDATE_RELAUNCH";
/// Test hook: the relaunched process runs this script (`--script`).
pub const RELAUNCH_SCRIPT_ENV: &str = "SHIFT_RELAUNCH_SCRIPT";
/// Test hook: reopened windows run this script, headless.
pub const RELAUNCH_WINDOW_SCRIPT_ENV: &str = "SHIFT_RELAUNCH_WINDOW_SCRIPT";
/// Set on windows opened by another window (⌘N, reopened windows): they do
/// not own `session.toml`.
pub const SECONDARY_ENV: &str = "SHIFT_SECONDARY_WINDOW";

/// How this process was started (read once, then removed from the
/// environment so shells and later windows do not inherit it).
#[derive(Debug, Default, Clone)]
pub struct Launch {
    pub restore: Option<PathBuf>,
    pub restart_others: bool,
    pub relaunched: bool,
    /// Opened by another window (`SHIFT_SECONDARY_WINDOW`).
    pub secondary: bool,
    /// Command-line arguments (without the program).
    pub args: Vec<String>,
}

static LAUNCH: OnceLock<Launch> = OnceLock::new();

/// Read and clear the restart variables. Call first thing in `main`,
/// before any thread starts.
pub fn take_env() {
    let restart_dir = shift_core::session_state::restart_dir();
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
        secondary: std::env::var_os(SECONDARY_ENV).is_some(),
        args: std::env::args().skip(1).collect(),
    };
    for k in [RESTORE_ENV, OTHERS_ENV, RELAUNCH_ENV, SECONDARY_ENV] {
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
    let mut silent: Vec<u32> = shift_pty::service::list()
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
        _ => shift_core::update::pid_alive(pid),
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
    let dir = shift_core::session_state::restart_dir()?;
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

fn owns_session(file: &Path) -> bool {
    file.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("main-"))
}

/// Which files reopen as the session-owning window: the first `main-` file,
/// unless this window already owns the session (`own_is_main`).
fn plan_reopen(
    own: Option<&Path>,
    own_is_main: bool,
    mut files: Vec<PathBuf>,
) -> Vec<(PathBuf, bool)> {
    files.retain(|f| Some(f.as_path()) != own);
    files.sort();
    let mut main_taken = own_is_main || own.is_some_and(owns_session);
    files
        .into_iter()
        .map(|f| {
            let main = !main_taken && owns_session(&f);
            main_taken |= main;
            (f, main)
        })
        .collect()
}

/// Reopen one window per restart file other than `own`: a `main-` file as
/// the session-owning window (one at most, none when `own_is_main`), the
/// rest as secondary windows. Each reopened window deletes its file once it
/// is up. A headless run reopens them only under the test hook (as
/// headless scripted runs). Returns how many were started.
pub fn reopen_windows(own: Option<&Path>, own_is_main: bool, headless: bool) -> usize {
    let Some(dir) = shift_core::session_state::restart_dir() else {
        return 0;
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let script = std::env::var(RELAUNCH_WINDOW_SCRIPT_ENV).ok();
    if headless && script.is_none() {
        log::info!("headless: not reopening windows from {}", dir.display());
        return 0;
    }
    let Ok(exe) = std::env::current_exe() else {
        return 0;
    };
    let files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_restart_file(p, &dir))
        .collect();
    let mut started = 0;
    for (file, main) in plan_reopen(own, own_is_main, files) {
        let mut cmd = std::process::Command::new(&exe);
        cmd.env(RESTORE_ENV, &file)
            .stdin(std::process::Stdio::null());
        if main {
            cmd.env_remove(SECONDARY_ENV);
        } else {
            cmd.env(SECONDARY_ENV, "1");
        }
        if let Some(s) = &script {
            cmd.args(["--script", s, "--headless"]);
        }
        match cmd.spawn() {
            // reaped by `alive` when an update waits on it; otherwise a
            // zombie until this window quits, like ⌘N's windows
            Ok(_) => started += 1,
            Err(e) => log::warn!("reopen window {}: {e}", file.display()),
        }
    }
    started
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
        }
    }

    #[test]
    fn survey_counts_tabs_and_shells() {
        let tab = |panes| TabInfo {
            index: 0,
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
    fn one_reopened_window_owns_the_session() {
        let p = |n: &str| PathBuf::from(format!("/r/{n}"));
        assert_eq!(restart_file_name(true, 7), "main-7.toml");
        assert_eq!(restart_file_name(false, 7), "7.toml");
        // a secondary window updated: the main window's file reopens as main
        assert_eq!(
            plan_reopen(
                Some(&p("9.toml")),
                false,
                vec![p("9.toml"), p("3.toml"), p("main-5.toml")]
            ),
            vec![(p("3.toml"), false), (p("main-5.toml"), true)]
        );
        // the main window updated: everything else is secondary, even a
        // second session owner (two plain launches)
        assert_eq!(
            plan_reopen(
                Some(&p("main-1.toml")),
                false,
                vec![p("main-1.toml"), p("main-2.toml")]
            ),
            vec![(p("main-2.toml"), false)]
        );
        assert_eq!(
            plan_reopen(None, false, vec![p("main-2.toml"), p("main-1.toml")]),
            vec![(p("main-1.toml"), true), (p("main-2.toml"), false)]
        );
        // a plain launch of the main window reopens a crashed update's
        // leftovers: it owns the session itself
        assert_eq!(
            plan_reopen(None, true, vec![p("main-2.toml"), p("4.toml")]),
            vec![(p("4.toml"), false), (p("main-2.toml"), false)]
        );
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
