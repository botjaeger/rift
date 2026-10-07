//! Directory listing for the file explorer: one level at a time (the tree
//! expands lazily), directories first, case-insensitive order, `.git` and
//! git-ignored entries left out.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Cap per directory: a listing past this is cut (node_modules-sized dirs)
/// to the first `MAX_ENTRIES` in display order.
pub const MAX_ENTRIES: usize = 2000;

/// Hard bound on how many directory entries one listing reads. Past it the
/// listing is still cut to `MAX_ENTRIES`, but from the entries read so far.
pub const MAX_SCAN: usize = 100_000;

/// One directory level, with what the plain `list` hides: why it could not
/// be read and whether entries were cut.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// The directory itself could not be read (permissions, gone, not a
    /// directory). Unreadable single entries are skipped silently.
    pub error: Option<String>,
    /// More than `MAX_ENTRIES` entries: `entries` holds the first
    /// `MAX_ENTRIES` in display order.
    pub truncated: bool,
}

/// The explorer root for `cwd`: the enclosing repository's working tree, or
/// `cwd` itself outside a repository.
pub fn root_for(cwd: &Path) -> PathBuf {
    git2::Repository::discover(cwd)
        .ok()
        .and_then(|r| r.workdir().map(Path::to_path_buf))
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or_else(|| cwd.to_path_buf())
}

/// List `dir`. `repo` filters ignored entries. Unreadable directories list
/// as empty; `listing` reports the error and the truncation.
pub fn list(dir: &Path, repo: Option<&git2::Repository>) -> Vec<Entry> {
    listing(dir, repo).entries
}

/// List `dir` like `list`, reporting a read error and whether the listing
/// was cut to `MAX_ENTRIES`.
pub fn listing(dir: &Path, repo: Option<&git2::Repository>) -> Listing {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) => {
            return Listing {
                error: Some(e.to_string()),
                ..Listing::default()
            };
        }
    };
    // both sides canonical: on macOS /var is a link to /private/var, and a
    // mismatch would skip the ignore check
    let workdir = repo
        .and_then(|r| r.workdir())
        .map(|w| w.canonicalize().unwrap_or_else(|_| w.to_path_buf()));
    let base = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut scanned = 0usize;
    let mut out: Vec<Entry> = read
        .take_while(|_| {
            scanned += 1;
            scanned <= MAX_SCAN
        })
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            let path = e.path();
            // follows symlinks: a link to a directory expands like one
            let is_dir = path.is_dir();
            if let (Some(repo), Some(workdir)) = (repo, workdir.as_deref())
                && let Ok(rel) = base.join(&name).strip_prefix(workdir)
            {
                // a `dir/` pattern only matches with the trailing slash
                let rel = rel.to_string_lossy().replace('\\', "/");
                let probe = if is_dir { format!("{rel}/") } else { rel };
                if repo.is_path_ignored(&probe).unwrap_or(false) {
                    return None;
                }
            }
            Some(Entry { name, path, is_dir })
        })
        .collect();
    // sort before cutting, so a capped listing is the head of the full order
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = scanned > MAX_SCAN || out.len() > MAX_ENTRIES;
    out.truncate(MAX_ENTRIES);
    Listing {
        entries: out,
        error: None,
        truncated,
    }
}

/// `path` relative to `root`, `/`-separated (the review model's key).
pub fn rel_key(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shift-fstree-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_capped_listing_is_the_head_of_the_sorted_order() {
        let dir = temp_dir("cap");
        // created newest-name first so creation order is not the sort order
        for n in (0..=MAX_ENTRIES).rev() {
            fs::write(dir.join(format!("f{n:04}")), "").unwrap();
        }
        let l = listing(&dir, None);
        let names: Vec<String> = l.entries.iter().map(|e| e.name.clone()).collect();
        let want: Vec<String> = (0..MAX_ENTRIES).map(|n| format!("f{n:04}")).collect();
        assert_eq!(names, want);
        assert!(l.truncated);
        assert!(l.error.is_none());
        assert_eq!(list(&dir, None), l.entries);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_reports_its_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("unreadable");
        let locked = dir.join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("x"), "").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        // root reads through mode 000; only assert where the mode bites
        if fs::read_dir(&locked).is_err() {
            let l = listing(&locked, None);
            assert!(l.error.is_some());
            assert!(l.entries.is_empty() && !l.truncated);
            assert!(list(&locked, None).is_empty());
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(listing(&dir.join("missing"), None).error.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_directory_is_neither_an_error_nor_truncated() {
        let dir = temp_dir("empty");
        assert_eq!(listing(&dir, None), Listing::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dirs_first_sorted_ignored_and_git_hidden() {
        let dir = temp_dir("sort");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("target/debug")).unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::write(dir.join("A.md"), "").unwrap();
        fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(dir.join("x.log"), "").unwrap();
        let repo = git2::Repository::init(&dir).unwrap();

        let names: Vec<String> = list(&dir, Some(&repo))
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["src", ".gitignore", "A.md", "b.txt"]);
        // without the repo nothing is filtered but .git
        let all: Vec<String> = list(&dir, None).into_iter().map(|e| e.name).collect();
        assert_eq!(
            all,
            ["src", "target", ".gitignore", "A.md", "b.txt", "x.log"]
        );
        assert_eq!(root_for(&dir.join("src")), dir.canonicalize().unwrap());
        assert_eq!(
            rel_key(&dir, &dir.join("src/main.rs")).as_deref(),
            Some("src/main.rs")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
