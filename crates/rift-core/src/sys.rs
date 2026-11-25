//! Platform differences in one place: process groups, liveness, file modes
//! and executable bits. Unix is the reference behaviour; Windows gets the
//! nearest equivalent.

use std::fs::{Metadata, OpenOptions};
use std::path::Path;
use std::process::{Child, Command};

/// Windows `CREATE_NEW_PROCESS_GROUP`.
#[cfg(windows)]
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Windows `CREATE_NO_WINDOW`: a console tool started from the GUI app
/// must not flash a console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start `cmd` in its own process group, so [`kill_group`] stops whatever it
/// started too. On Windows also without a console window.
pub fn own_group(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
    }
}

/// A console tool started without a console window (Windows); unchanged
/// elsewhere.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Forcibly stop the group [`own_group`] created for `child`: the child and
/// every process it started. The child itself stays unreaped.
pub fn kill_group(child: &Child) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: plain syscall on a process group this process created
        // (process_group(0)); never the caller's own group (pid > 0).
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        // the tree under the child's pid (the pid stays pinned while
        // `child` holds its handle)
        let mut cmd = Command::new("taskkill");
        cmd.args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = no_window(&mut cmd).status();
    }
}

/// Whether a process with `pid` exists (it may belong to another user).
pub fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists
        let r = unsafe { libc::kill(pid, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        const STILL_ACTIVE: u32 = 259;
        // SAFETY: plain Win32 calls; the handle is closed before returning
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE
        }
    }
}

/// Windows: this process holds administrator rights (started as
/// administrator, or any administrator's process while UAC is off), so every
/// shell it opens can change the whole computer. Elsewhere false.
///
/// The Administrators group enabled in the process's own token is what
/// gives those rights: `TokenElevation` alone also says yes for a token a
/// basic-user `runas /trustlevel` made while UAC is off, which has the group
/// only to deny with.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::BOOL;
        use windows_sys::Win32::Security::{
            CheckTokenMembership, CreateWellKnownSid, SECURITY_MAX_SID_SIZE,
            WinBuiltinAdministratorsSid,
        };
        let mut sid = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = SECURITY_MAX_SID_SIZE;
        let mut member: BOOL = 0;
        // SAFETY: a buffer of the documented maximum SID size; a null token
        // means this thread's (impersonation) or process's own token
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size,
            ) != 0
                && CheckTokenMembership(std::ptr::null_mut(), sid.as_mut_ptr().cast(), &mut member)
                    != 0
                && member != 0
        }
    }
    #[cfg(not(windows))]
    false
}

/// The home directory as `$HOME` gives it. Windows sets no `HOME` (Git Bash
/// and MSYS do, to the same place): the user's profile folder there.
pub fn home_os() -> Option<std::ffi::OsString> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME")
    }
    #[cfg(windows)]
    {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .or_else(|| std::env::var_os("USERPROFILE"))
    }
}

/// [`home_os`] in the shape of `std::env::var("HOME")`.
pub fn home_var() -> Result<String, std::env::VarError> {
    match home_os() {
        Some(h) => h.into_string().map_err(std::env::VarError::NotUnicode),
        None => Err(std::env::VarError::NotPresent),
    }
}

/// `std::fs::rename` that replaces `to`. On Windows a file another process
/// has open without delete sharing (an editor, an antivirus scan, a
/// volume without POSIX rename semantics) refuses the replace for a
/// moment: try again for up to half a second.
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    #[cfg(windows)]
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            match std::fs::rename(from, to) {
                Err(e)
                    if e.kind() == std::io::ErrorKind::PermissionDenied
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

/// `std::fs::canonicalize`, but on Windows without the `\\?\` prefix when
/// the path does not need it (`\\?\C:\x` is `C:\x`, `\\?\UNC\srv\share` is
/// `\\srv\share`): shells, links and comparisons expect the plain form.
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<std::path::PathBuf> {
    let real = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        Ok(simplify_verbatim(real))
    }
    #[cfg(not(windows))]
    {
        Ok(real)
    }
}

/// The plain form of a verbatim (`\\?\`) path when one exists: a drive path
/// or a UNC share, short enough for the classic limit (260) and free of
/// the characters only verbatim paths allow.
#[cfg_attr(not(windows), allow(dead_code))]
fn simplify_verbatim(path: std::path::PathBuf) -> std::path::PathBuf {
    let Some(s) = path.to_str() else {
        return path;
    };
    let plain = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
            return path;
        }
        rest.to_string()
    } else {
        return path;
    };
    let reserved = plain
        .split('\\')
        .skip(1)
        .any(|c| c.ends_with(['.', ' ']) || c.contains(['<', '>', '"', '|', '?', '*']));
    if plain.len() >= 260 || reserved {
        return path;
    }
    std::path::PathBuf::from(plain)
}

/// This machine's host name; empty when the system does not say.
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most buf.len() bytes into buf
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    }
}

/// Whether `path` (with metadata `meta`, symlinks followed) is a command a
/// shell would run: an execute bit on Unix, a `PATHEXT` extension on Windows.
pub fn is_executable(path: &Path, meta: &Metadata) -> bool {
    if meta.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = path;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD;.PS1".into());
        pathext
            .split(';')
            .filter_map(|e| e.strip_prefix('.'))
            .any(|e| e.eq_ignore_ascii_case(ext))
    }
}

/// A number that changes when a file is replaced: the inode on Unix. Windows
/// has no stable equivalent in std, so 0 there and callers rely on size and
/// modification time.
pub fn file_id(meta: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    }
    #[cfg(windows)]
    {
        let _ = meta;
        0
    }
}

/// Unix permission bits for files this opens. Windows files inherit the
/// folder's ACL (the user's profile folders are private to them already).
pub trait UnixMode {
    fn unix_mode(&mut self, mode: u32) -> &mut Self;
    /// `O_NONBLOCK` on the open itself, so a FIFO swapped in for a file
    /// cannot hang the caller (Windows has no such FIFOs: unchanged).
    fn nonblocking_open(&mut self) -> &mut Self;
}

impl UnixMode for OpenOptions {
    fn nonblocking_open(&mut self) -> &mut Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            self.custom_flags(libc::O_NONBLOCK)
        }
        #[cfg(windows)]
        {
            self
        }
    }

    fn unix_mode(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            self.mode(mode)
        }
        #[cfg(windows)]
        {
            let _ = mode;
            self
        }
    }
}

/// `chmod`; a no-op on Windows (see [`UnixMode`]).
pub fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// The permission bits of `meta` (Windows: 0o600 for a writable file, 0o400
/// for a read-only one).
pub fn mode(meta: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode()
    }
    #[cfg(windows)]
    {
        if meta.permissions().readonly() {
            0o400
        } else {
            0o600
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_paths_lose_the_prefix_only_when_safe() {
        use std::path::PathBuf;
        let s = |p: &str| simplify_verbatim(PathBuf::from(p));
        assert_eq!(s(r"\\?\C:\Users\me"), PathBuf::from(r"C:\Users\me"));
        assert_eq!(s(r"\\?\UNC\srv\share\x"), PathBuf::from(r"\\srv\share\x"));
        // not a drive path, a trailing dot, too long: kept verbatim
        assert_eq!(s(r"\\?\Volume{1}\x"), PathBuf::from(r"\\?\Volume{1}\x"));
        assert_eq!(s(r"\\?\C:\a.\b"), PathBuf::from(r"\\?\C:\a.\b"));
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(s(&long), PathBuf::from(&long));
        // anything else as it is
        assert_eq!(s("/usr/bin"), PathBuf::from("/usr/bin"));
    }

    #[test]
    fn own_process_is_alive_and_a_reaped_child_is_not() {
        assert!(process_alive(std::process::id()));
        #[cfg(unix)]
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let mut child = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_alive(pid));
    }

    const ROLE: &str = "RIFT_SYS_TEST_ROLE";

    /// This test binary again, running only [`process_tree_helper`] as
    /// `role`: the same on every system (no shell needed).
    fn helper(role: &str) -> Command {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "--exact",
            "sys::tests::process_tree_helper",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, role);
        c
    }

    /// Not a test by itself: `parent` starts a `sleeper`, prints its pid and
    /// waits for it; `sleeper` sleeps.
    #[test]
    #[ignore]
    fn process_tree_helper() {
        match std::env::var(ROLE).as_deref() {
            Ok("parent") => {
                let mut sleeper = helper("sleeper")
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                // the harness prints around tests: a marked line
                println!("\nPID={}", sleeper.id());
                let _ = sleeper.wait();
            }
            Ok("sleeper") => std::thread::sleep(std::time::Duration::from_secs(30)),
            _ => {}
        }
    }

    #[test]
    fn kill_group_stops_a_grandchild() {
        let mut cmd = helper("parent");
        let mut child = own_group(&mut cmd)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let out = std::io::BufReader::new(child.stdout.take().unwrap());
        let grandchild: u32 = std::io::BufRead::lines(out)
            .map_while(Result::ok)
            .find_map(|l| l.trim().strip_prefix("PID=").map(str::to_string))
            .expect("the helper prints its child's pid")
            .parse()
            .unwrap();
        assert!(process_alive(grandchild));
        kill_group(&child);
        child.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while process_alive(grandchild) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!process_alive(grandchild));
    }
}
