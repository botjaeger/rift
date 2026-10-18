//! The palette's *Install 'rift' command*: a `rift` symlink to the running
//! binary in `~/.local/bin` (`RIFT_CLI_DIR` overrides the folder), so
//! `rift .` opens a folder in Rift from any shell or editor. A file there
//! that is not Rift's own link is never replaced.

use std::path::{Path, PathBuf};

/// Overrides the folder the command is installed in (tests).
pub const DIR_ENV: &str = "RIFT_CLI_DIR";
/// The command's name.
pub const NAME: &str = "rift";

/// Where the command goes: `RIFT_CLI_DIR`, else `~/.local/bin`.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DIR_ENV).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/bin"))
}

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The `rift` link.
    pub link: PathBuf,
    /// It was there already, pointing at this binary.
    pub unchanged: bool,
}

/// Make `dir/rift` a symlink to `binary` (the folder is created). An
/// earlier link to a Rift binary is replaced; any other file is refused.
pub fn install(binary: &Path, dir: &Path) -> Result<Installed, String> {
    let link = dir.join(NAME);
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target =
                std::fs::read_link(&link).map_err(|e| format!("{}: {e}", link.display()))?;
            if target == binary {
                return Ok(Installed {
                    link,
                    unchanged: true,
                });
            }
            if !is_rift_binary(&target) {
                return Err(format!(
                    "{} already exists and points at {}; not replaced",
                    link.display(),
                    target.display()
                ));
            }
        }
        Ok(_) => {
            return Err(format!(
                "{} already exists and is not Rift's; not replaced",
                link.display()
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", link.display())),
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // made beside it, then renamed over it: never a moment without a command
    let fresh = dir.join(format!(".{NAME}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&fresh);
    std::os::unix::fs::symlink(binary, &fresh).map_err(|e| format!("{}: {e}", dir.display()))?;
    if let Err(e) = std::fs::rename(&fresh, &link) {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("{}: {e}", link.display()));
    }
    Ok(Installed {
        link,
        unchanged: false,
    })
}

/// A link target that is a Rift binary (an older build or bundle).
fn is_rift_binary(target: &Path) -> bool {
    target.file_name().is_some_and(|n| n == "rift-terminal")
}

/// Whether `dir` is one of `path`'s entries (a `$PATH` value).
pub fn on_path(dir: &Path, path: Option<&std::ffi::OsStr>) -> bool {
    let Some(path) = path else { return false };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let want = canon(dir);
    std::env::split_paths(path).any(|p| canon(&p) == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rift-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn installs_a_link_and_replaces_only_rifts_own() {
        let d = scratch("install");
        let bin = d.join("app/rift-terminal");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "").unwrap();
        let dir = d.join("bin");
        let done = install(&bin, &dir).unwrap();
        assert_eq!(done.link, dir.join("rift"));
        assert!(!done.unchanged);
        assert_eq!(std::fs::read_link(dir.join("rift")).unwrap(), bin);
        // again: nothing to do
        assert!(install(&bin, &dir).unwrap().unchanged);
        // a link to another Rift build moves to this one
        let other = d.join("old/rift-terminal");
        std::fs::remove_file(dir.join("rift")).unwrap();
        std::os::unix::fs::symlink(&other, dir.join("rift")).unwrap();
        assert!(!install(&bin, &dir).unwrap().unchanged);
        assert_eq!(std::fs::read_link(dir.join("rift")).unwrap(), bin);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_a_file_that_is_not_rifts() {
        let d = scratch("foreign");
        let bin = d.join("rift-terminal");
        let dir = d.join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rift"), "#!/bin/sh\necho mine\n").unwrap();
        let err = install(&bin, &dir).unwrap_err();
        assert!(err.contains("not replaced"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("rift")).unwrap(),
            "#!/bin/sh\necho mine\n"
        );
        // a link to some other program is not Rift's either
        std::fs::remove_file(dir.join("rift")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/true", dir.join("rift")).unwrap();
        assert!(install(&bin, &dir).is_err());
        assert_eq!(
            std::fs::read_link(dir.join("rift")).unwrap(),
            Path::new("/usr/bin/true")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn finds_the_folder_on_path() {
        let d = scratch("path");
        std::fs::create_dir_all(&d).unwrap();
        let path = std::env::join_paths([Path::new("/usr/bin"), &d]).unwrap();
        assert!(on_path(&d, Some(&path)));
        assert!(!on_path(&d.join("x"), Some(&path)));
        assert!(!on_path(&d, None));
        let _ = std::fs::remove_dir_all(&d);
    }
}
