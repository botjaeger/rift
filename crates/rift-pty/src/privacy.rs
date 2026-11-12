//! macOS privacy probes (Settings › Permissions).
//!
//! Full Disk Access cannot be asked: it is read by opening a TCC database
//! for reading, which succeeds only with the grant and never shows a prompt.
//! The system one first: macOS 27 has no per-user database on some Macs
//! (`ENOENT`), while the system one answers `EPERM` without the grant. The answer belongs to the process that asks, so the session
//! service (the app that runs the shells) answers it for itself over the
//! IPC (`Request::Privacy`).

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The databases whose readability marks Full Disk Access, the system one
/// first.
pub fn tcc_dbs() -> Vec<PathBuf> {
    let mut dbs = vec![PathBuf::from(
        "/Library/Application Support/com.apple.TCC/TCC.db",
    )];
    let home = rift_core::runtimes::home();
    if !home.as_os_str().is_empty() {
        dbs.push(home.join("Library/Application Support/com.apple.TCC/TCC.db"));
    }
    dbs
}

/// The first answer among `dbs` (a missing one tells nothing).
pub fn first_answer(dbs: &[PathBuf]) -> Option<bool> {
    dbs.iter().find_map(|p| readable(p))
}

/// Whether `path` can be opened for reading: `Some(true)` it can,
/// `Some(false)` the system refuses (`EPERM`/`EACCES`), `None` it cannot
/// tell (no such file: not macOS).
pub fn readable(path: &Path) -> Option<bool> {
    match std::fs::File::open(path) {
        Ok(_) => Some(true),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Some(false),
        Err(_) => None,
    }
}

/// Whether this process has Full Disk Access (`None`: not macOS, or the
/// database is not there).
pub fn full_disk_access() -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    first_answer(&tcc_dbs())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_readable_file_is_granted_a_missing_one_unknown() {
        let dir = std::env::temp_dir().join(format!("rift-privacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("TCC.db");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(readable(&file), Some(true));
        assert_eq!(readable(&dir.join("absent")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_database_defers_to_the_next() {
        let dir = std::env::temp_dir().join(format!("rift-privacy-next-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("TCC.db");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(first_answer(&[dir.join("absent"), file]), Some(true));
        assert_eq!(first_answer(&[dir.join("absent")]), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_file_is_not_granted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rift-privacy-deny-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("TCC.db");
        std::fs::write(&file, b"x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        // root reads anything: the probe cannot be refused there
        let expected = if unsafe { libc::geteuid() } == 0 {
            Some(true)
        } else {
            Some(false)
        };
        assert_eq!(readable(&file), expected);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
