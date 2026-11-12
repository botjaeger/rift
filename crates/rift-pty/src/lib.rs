pub mod abs_term;
#[cfg(unix)]
pub mod daemon;
/// Windows: no session service yet (see [`uds`]); panes run local PTYs.
#[cfg(windows)]
pub mod daemon {
    /// Same bound as the Unix service (`kept` sizes its marks by it).
    pub const MAX_RETAINED: usize = 8 << 20;

    pub fn run() -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "the session service is not available on Windows yet",
        ))
    }
}
pub mod graphics;
pub mod ipc;
pub mod kept;
pub mod launchd;
pub mod privacy;
pub mod service;
pub mod session;
pub mod uds;

pub use alacritty_terminal;
pub use session::{
    ClipboardAccess, ClipboardReply, ColorReply, EventProxy, GridSize, Observer, PaneId,
    ProgressReport, Replay, SHADOW_BLOCKED, SHADOW_EVERY, SHADOW_QUIET, ServiceFailure,
    ServiceTarget, Session, SessionEvent, ShadowDue, ShadowListener, ShadowStats, SpawnOptions,
    set_default_colors, set_pane_colors,
};

/// The exit code of child `pid` once it ended: its own code, or 128 + the
/// signal that killed it. Waits up to half a second (the PTY closes as the
/// shell exits, a moment before it can be reaped); `None` when it cannot
/// tell (still running, reaped elsewhere). Only peeks (`WNOWAIT`): the
/// child stays for its owner (the `Pty`) to reap. Reaping it here would
/// free its pid while the owner still signals that pid when dropped, so a
/// new process given the same pid would get the hang-up.
#[cfg(windows)]
pub(crate) fn exit_code_of(pid: u32) -> Option<i32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    // the Pty's exit watcher holds a handle to the shell: its pid is not
    // reused while it is open
    // SAFETY: plain Win32 calls; the handle is closed before returning
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut code = STILL_ACTIVE;
        for _ in 0..50 {
            if GetExitCodeProcess(h, &mut code) == 0 {
                break;
            }
            if code != STILL_ACTIVE {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        CloseHandle(h);
        (code != STILL_ACTIVE).then_some(code as i32)
    }
}

#[cfg(unix)]
pub(crate) fn exit_code_of(pid: u32) -> Option<i32> {
    for _ in 0..50 {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if r < 0 {
            return None;
        }
        let (child, code, status) = unsafe { (info.si_pid(), info.si_code, info.si_status()) };
        if child == pid as libc::pid_t {
            return match code {
                libc::CLD_EXITED => Some(status),
                libc::CLD_KILLED | libc::CLD_DUMPED => Some(128 + status),
                _ => None,
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    /// `exit_code_of` waits half a second for the child to end; a loaded
    /// machine (the Linux gate beside the UI run) can take longer to start
    /// `/bin/sh` at all, so ask again for up to 10 s.
    fn code_of(pid: u32) -> Option<i32> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let code = super::exit_code_of(pid);
            if code.is_some() || std::time::Instant::now() > end {
                return code;
            }
        }
    }

    #[test]
    fn exit_code_of_reads_the_code_and_the_signal_and_leaves_the_child() {
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        assert_eq!(code_of(child.id()), Some(3));
        // not reaped: its owner still gets it (and its pid is not free yet)
        assert_eq!(child.wait().unwrap().code(), Some(3));
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -9 $$"])
            .spawn()
            .unwrap();
        assert_eq!(code_of(child.id()), Some(137));
        assert!(child.wait().is_ok());
    }
}
